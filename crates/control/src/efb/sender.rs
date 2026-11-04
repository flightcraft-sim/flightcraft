//! A bounded latest-value mailbox and a joinable, independently paced UDP worker.
use super::{EfbSettings, EfbStatus, encode};
use std::{
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
struct Shared {
    status: EfbStatus,
    aircraft: Option<(encode::Aircraft, Instant)>,
    traffic: Vec<encode::Traffic>,
}
#[derive(Debug)]
pub(crate) struct Sender {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Sender {
    pub fn new(serial: u64) -> Result<Self, String> {
        Self::with_ports(serial, 4000, 49002)
    }
    fn with_ports(serial: u64, gdl_port: u16, simulator_port: u16) -> Result<Self, String> {
        let shared = Arc::new(Mutex::new(Shared {
            status: EfbStatus {
                available: true,
                ..Default::default()
            },
            aircraft: None,
            traffic: Vec::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_shared = shared.clone();
        let worker_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("tablet-broadcast".into())
            .spawn(move || worker(worker_shared, worker_stop, serial, gdl_port, simulator_port))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            stop,
            thread: Some(thread),
        })
    }
    pub fn configure(&self, settings: EfbSettings) -> Result<(), String> {
        self.shared
            .lock()
            .map_err(|_| "tablet sender state unavailable")?
            .status
            .settings = settings;
        Ok(())
    }
    pub fn update(&self, aircraft: Option<encode::Aircraft>) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.aircraft = aircraft.map(|a| (a, Instant::now()));
            shared.status.reason = reason(&shared, Instant::now());
        }
    }
    pub fn update_traffic(&self, traffic: Vec<encode::Traffic>) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.traffic = traffic;
        }
    }
    pub fn status(&self) -> EfbStatus {
        self.shared
            .lock()
            .map(|s| s.status.clone())
            .unwrap_or_else(|_| EfbStatus {
                last_error: Some("tablet sender state unavailable".into()),
                ..Default::default()
            })
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn reason(shared: &Shared, now: Instant) -> Option<String> {
    if !shared.status.settings.enabled {
        Some("disabled".into())
    } else if shared.aircraft.is_none() {
        Some("active world has no geodetic position; select a geographic region".into())
    } else if shared
        .aircraft
        .is_some_and(|(_, at)| now.duration_since(at) > Duration::from_secs(2))
    {
        Some("aircraft truth is stale; waiting for the app".into())
    } else {
        None
    }
}

/// Deadlines skip missed periods; a stalled worker never produces a catch-up burst.
#[derive(Debug)]
struct Schedule {
    second: Instant,
    attitude: Instant,
}
impl Schedule {
    fn new(now: Instant) -> Self {
        Self {
            second: now,
            attitude: now,
        }
    }
    fn due(&mut self, now: Instant) -> (bool, bool) {
        let second = now >= self.second;
        let attitude = now >= self.attitude;
        if second {
            self.second = now + Duration::from_secs(1);
        }
        if attitude {
            self.attitude = now + Duration::from_millis(200);
        }
        (second, attitude)
    }
}
fn worker(
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    serial: u64,
    gdl_port: u16,
    simulator_port: u16,
) {
    let mut socket = None;
    let mut schedule = Schedule::new(Instant::now());
    let mut interval = Instant::now();
    let mut previous_count = 0;
    while !stop.load(Ordering::Relaxed) {
        if let Ok(mut shared) = shared.lock() {
            let now = Instant::now();
            let total = shared
                .status
                .gdl90_packets
                .saturating_add(shared.status.simulator_packets);
            if now.duration_since(interval) >= Duration::from_secs(1) {
                shared.status.packets_per_second = total.saturating_sub(previous_count);
                previous_count = total;
                interval = now;
            }
            shared.status.reason = reason(&shared, now);
            if shared.status.reason.is_none() {
                let result = (|| -> Result<(), String> {
                    if socket.is_none() {
                        let udp = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
                        udp.set_broadcast(true).map_err(|e| e.to_string())?;
                        udp.set_nonblocking(true).map_err(|e| e.to_string())?;
                        socket = Some(udp);
                    }
                    if let (Some(udp), Some((aircraft, _))) = (&socket, shared.aircraft) {
                        let (second, attitude) = schedule.due(now);
                        let utc = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_err(|e| e.to_string())?
                            .as_secs()
                            % 86400;
                        let destination = shared
                            .status
                            .settings
                            .destination
                            .parse::<Ipv4Addr>()
                            .map_err(|e| e.to_string())?;
                        let mut outgoing = packets(
                            &shared.status.settings,
                            &aircraft,
                            serial,
                            utc as u32,
                            second,
                            attitude,
                        );
                        if second && shared.status.settings.gdl90 {
                            outgoing.extend(
                                shared
                                    .traffic
                                    .iter()
                                    .map(|t| (true, encode::frame(&encode::traffic_report(t)))),
                            );
                        }
                        if second && shared.status.settings.simulator {
                            outgoing.extend(
                                shared
                                    .traffic
                                    .iter()
                                    .map(|t| (false, encode::xtraffic(&t.aircraft, t.address).into_bytes())),
                            );
                        }
                        for (gdl, bytes) in outgoing {
                            let port = if gdl { gdl_port } else { simulator_port };
                            if let Err(error) = udp.send_to(&bytes, SocketAddrV4::new(destination, port)) {
                                shared.status.last_error = Some(error.to_string());
                                continue;
                            }
                            let count = if gdl {
                                &mut shared.status.gdl90_packets
                            } else {
                                &mut shared.status.simulator_packets
                            };
                            *count = count.saturating_add(1);
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    shared.status.last_error = Some(error);
                }
            } else {
                schedule = Schedule::new(now);
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
}
fn packets(
    settings: &EfbSettings,
    aircraft: &encode::Aircraft,
    serial: u64,
    utc: u32,
    second: bool,
    attitude: bool,
) -> Vec<(bool, Vec<u8>)> {
    let mut result = Vec::new();
    if settings.gdl90 {
        if second {
            for payload in [
                encode::heartbeat(utc),
                encode::report(aircraft, true, (serial as u32) & 0xffffff, "FLIGHTCR"),
                encode::geometric_altitude(aircraft.altitude_msl_m),
                encode::identification(serial),
            ] {
                result.push((true, encode::frame(&payload)));
            }
        }
        if attitude {
            result.push((true, encode::frame(&encode::ahrs(aircraft))));
        }
    }
    if settings.simulator {
        if second {
            result.push((false, encode::xgps(aircraft).into_bytes()));
        }
        if attitude {
            result.push((false, encode::xatt(aircraft).into_bytes()));
        }
    }
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn schedule_skips_missed_ticks_and_obeys_five_hz() {
        let now = Instant::now();
        let mut schedule = Schedule::new(now);
        assert_eq!(schedule.due(now), (true, true));
        assert_eq!(schedule.due(now + Duration::from_millis(199)), (false, false));
        assert_eq!(schedule.due(now + Duration::from_millis(200)), (false, true));
        assert_eq!(schedule.due(now + Duration::from_secs(10)), (true, true));
        assert_eq!(schedule.due(now + Duration::from_secs(10)), (false, false));
    }
    #[test]
    fn protocol_toggles_and_packet_rates() {
        let a = super::super::encode_tests::truth();
        let mut settings = EfbSettings::default();
        assert_eq!(packets(&settings, &a, 1, 0, true, true).len(), 5);
        assert_eq!(packets(&settings, &a, 1, 0, false, true).len(), 1);
        settings.gdl90 = false;
        settings.simulator = true;
        let text = packets(&settings, &a, 1, 0, true, true);
        assert_eq!(text.len(), 2);
        assert!(text.iter().all(|(gdl, _)| !gdl));
        settings.gdl90 = true;
        assert_eq!(packets(&settings, &a, 1, 0, true, true).len(), 7);
        assert!(packets(&settings, &a, 1, 0, false, false).is_empty());
    }
    #[test]
    fn disabled_unreferenced_and_stale_truth_suppress_all_packets() {
        let now = Instant::now();
        let mut shared = Shared {
            status: EfbStatus::default(),
            aircraft: None,
            traffic: Vec::new(),
        };
        assert_eq!(reason(&shared, now).as_deref(), Some("disabled"));
        shared.status.settings.enabled = true;
        assert!(reason(&shared, now).is_some_and(|r| r.contains("geodetic")));
        shared.aircraft = Some((super::super::encode_tests::truth(), now));
        assert!(reason(&shared, now).is_none());
        assert!(reason(&shared, now + Duration::from_secs(3)).is_some_and(|r| r.contains("stale")));
    }
    #[test]
    fn worker_broadcasts_traffic_reports_with_stable_addresses() {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let sender = Sender::with_ports(42, socket.local_addr().unwrap().port(), 49002).unwrap();
        let truth = super::super::encode_tests::truth();
        sender.update(Some(truth));
        let traffic = |address, callsign: &str| encode::Traffic {
            aircraft: truth,
            address,
            callsign: callsign.into(),
            emitter: 1,
            alert: false,
        };
        sender.update_traffic(vec![traffic(0xf10001, "TRAINER1"), traffic(0xf10002, "TRAINER2")]);
        sender
            .configure(EfbSettings {
                enabled: true,
                destination: "127.0.0.1".into(),
                gdl90: true,
                simulator: false,
            })
            .unwrap();
        let mut found = Vec::new();
        for _ in 0..10 {
            let mut bytes = [0; 256];
            let size = socket.recv(&mut bytes).unwrap();
            if bytes.get(1) == Some(&0x14) {
                assert!(
                    bytes
                        .get(..size)
                        .is_some_and(|b| b.windows(7).any(|w| w == b"TRAINER"))
                );
                found.push(bytes.get(3..6).unwrap().to_vec());
            }
            if found.len() == 2 {
                break;
            }
        }
        assert_eq!(found, vec![vec![0xf1, 0x00, 0x01], vec![0xf1, 0x00, 0x02]]);
    }
    #[test]
    fn worker_sends_both_protocols_on_loopback_and_stops_when_disabled() {
        let gdl = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let simulator = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        gdl.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        simulator.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let sender = Sender::with_ports(
            42,
            gdl.local_addr().unwrap().port(),
            simulator.local_addr().unwrap().port(),
        )
        .unwrap();
        sender.update(Some(super::super::encode_tests::truth()));
        sender
            .configure(EfbSettings {
                enabled: true,
                destination: "127.0.0.1".into(),
                gdl90: true,
                simulator: true,
            })
            .unwrap();
        let mut bytes = [0; 128];
        let size = gdl.recv(&mut bytes).unwrap();
        assert_eq!(bytes.first(), Some(&0x7e));
        assert_eq!(bytes.get(size - 1), Some(&0x7e));
        assert_eq!(bytes.get(1), Some(&0)); // Heartbeat is first.
        let size = simulator.recv(&mut bytes).unwrap();
        assert_eq!(
            bytes.get(..size),
            Some(encode::xgps(&super::super::encode_tests::truth()).as_bytes())
        );
        sender.configure(EfbSettings::default()).unwrap();
        let before = sender.status();
        assert!(before.gdl90_packets >= 5);
        assert!(before.simulator_packets >= 2);
        thread::sleep(Duration::from_millis(240));
        let after = sender.status();
        assert_eq!(before.gdl90_packets, after.gdl90_packets);
        assert_eq!(before.simulator_packets, after.simulator_packets);
        assert_eq!(after.reason.as_deref(), Some("disabled"));
        assert!(after.last_error.is_none());
    }
}
