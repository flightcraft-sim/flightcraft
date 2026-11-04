//! Busy-airspace traffic: simulated aircraft flown by a bounded kinematic model, controlled by the
//! tower and approach rules on the real frequencies, heard only where the user's radios can hear.
//! See `docs/traffic.md`. Nothing here enters the physics state or its golden hash.
use super::*;
mod advisory;
mod control;
mod craft;
mod field;
pub use advisory::RadioDiscipline;
pub(crate) use advisory::{AdvisoryBook, clock_of};
pub(crate) use control::Fleet;
use control::{Stage, Tx};
use field::Field;

/// Silent warm-up so a newly enabled frequency is already busy.
const WARMUP_S: f64 = 600.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrafficDensity {
    #[default]
    Off,
    /// 4 aircraft.
    Light,
    /// 8 aircraft.
    Moderate,
    /// 14 aircraft.
    Busy,
}
impl TrafficDensity {
    pub fn target(self) -> usize {
        match self {
            Self::Off => 0,
            Self::Light => 4,
            Self::Moderate => 8,
            Self::Busy => 14,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FleetRole {
    Controller,
    Pilot,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FleetCallKind {
    CheckIn,
    Instruction,
    Readback,
    /// The controller corrects a wrong readback.
    Correction,
    Information,
    Terminate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FleetFlight {
    Arrival,
    Departure,
    FlightFollowing,
    Pattern,
}
/// The landing number the controller gave, and the aircraft it follows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FleetSequence {
    pub number: u8,
    pub follows: String,
}
/// Tags a simulated-traffic transmission for voices, captions, and display correlation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FleetCall {
    /// Calls that belong together (instruction, readback, correction) share this number.
    pub exchange: u32,
    pub aircraft: u32,
    /// The aircraft's full spoken callsign.
    pub callsign: String,
    pub role: FleetRole,
    pub kind: FleetCallKind,
    pub instructions: Vec<AtcInstruction>,
    pub facility: String,
    pub frequency_khz: u32,
    /// Stable voice index 0-7: the aircraft's for pilots, the facility's for controllers.
    pub voice: u8,
    /// Traffic never addresses the user; the user answers only calls with their own callsign.
    pub for_user: bool,
    pub sequence: Option<FleetSequence>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FleetIntent {
    pub kind: FleetFlight,
    pub phase: String,
    pub airport: String,
    pub runway: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FleetAircraft {
    pub id: u32,
    /// Fully spoken, such as `Cessna Four Two Seven Kilo Papa`.
    pub callsign: String,
    pub registration: String,
    pub aircraft_type: String,
    pub type_designator: String,
    pub latitude_deg: Option<f64>,
    pub longitude_deg: Option<f64>,
    pub altitude_msl_ft: f64,
    pub position_m: [f64; 3],
    pub track_true_deg: f64,
    pub track_magnetic_deg: f64,
    pub ground_speed_kt: f64,
    pub vertical_rate_fpm: f64,
    pub on_ground: bool,
    pub squawk: String,
    pub frequency_khz: u32,
    /// Station id of the controlling facility, and its spoken name.
    pub facility_id: String,
    pub facility: String,
    pub intent: FleetIntent,
    pub assigned_altitude_ft: Option<u32>,
    pub assigned_heading_deg: Option<u16>,
    pub assigned_speed_kt: Option<u16>,
    pub cleared_to_land_or_approach: bool,
    /// Airspace class containing the aircraft, when the region carries airspace.
    pub airspace_class: Option<String>,
    pub voice: u8,
    /// From the user: distance, true bearing, and altitude difference (traffic minus user).
    pub range_nm: f64,
    pub bearing_true_deg: f64,
    pub relative_altitude_ft: f64,
    /// The user's tuned COM frequency matches this aircraft's controller.
    pub on_user_frequency: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct FleetStats {
    pub spawned: u32,
    pub completed: u32,
    pub transmissions: u64,
    pub readbacks_accepted: u32,
    pub readbacks_corrected: u32,
    /// Times the physics guard had to slow an aircraft; the controllers aim to keep this at zero.
    pub safety_slowdowns: u64,
    /// Calls the user keyed over, which were therefore sent again.
    pub stepped_on: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FleetFrequency {
    pub khz: u32,
    pub facility_id: String,
    pub facility: String,
    pub aircraft: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FleetState {
    pub density: TrafficDensity,
    pub airport: Option<String>,
    pub runway: Option<String>,
    pub seed: u64,
    pub aircraft: Vec<FleetAircraft>,
    pub frequencies: Vec<FleetFrequency>,
    pub stats: FleetStats,
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetTrafficDensity {
    pub density: TrafficDensity,
    /// A towered airport with imported tower and approach frequencies. Omission uses the configured
    /// ATC airport, else the nearest towered airport.
    #[serde(default)]
    pub airport: Option<String>,
}
impl Command for SetTrafficDensity {
    const NAME: &'static str = "traffic.density";
    const DESCRIPTION: &'static str = "Set busy-airspace traffic (off, light 4, moderate 8, busy 14 aircraft) controlled by the tower and approach rules on the real frequencies; seeded from the session weather seed.";
    type Output = FleetState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        // A new traffic setup starts a new radio-discipline tally.
        s.comms.advisories = Default::default();
        if self.density == TrafficDensity::Off {
            s.comms.fleet = None;
            return Ok(s.fleet_state());
        }
        let airport = match self.airport.or_else(|| s.comms.atc.airport.clone()) {
            Some(a) => a,
            None => s
                .nearest_towered()
                .ok_or_else(|| invalid(Self::NAME, "no towered airport with imported frequencies"))?,
        };
        let field = Field::build(s, &airport)?;
        let call = s.pilot_callsign();
        let seed = s.sim.weather().seed ^ fnv(&field.airport);
        let now = s.sim.state().time_s;
        let reserved = vec![call.full, call.abbreviated, call.current];
        s.comms.fleet = Some(Fleet::new(field, self.density, seed, now, reserved));
        Ok(s.fleet_state())
    }
}
#[derive(Debug, Default, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetTrafficState {}
impl Command for GetTrafficState {
    const NAME: &'static str = "traffic.state";
    const DESCRIPTION: &'static str = "List every busy-airspace aircraft: callsign, position, track, speed, vertical rate, squawk, frequency, controlling facility, and intent.";
    type Output = FleetState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(s.fleet_state())
    }
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl Session {
    /// Training setup: start the traffic and put the user in radar contact with the approach-low
    /// controller of the airport, COM1 tuned to it, as an airborne continuation would be.
    pub(crate) fn start_lesson_traffic(
        &mut self,
        airport: &str,
        density: TrafficDensity,
    ) -> Result<(), CommandError> {
        self.execute(
            "traffic.density",
            serde_json::json!({"density":density,"airport":airport}),
        )?;
        let station = self
            .comms
            .fleet
            .as_ref()
            .map(|f| f.field.st(f.field.lo).clone())
            .ok_or_else(|| invalid("lesson.start", "traffic needs a towered airport"))?;
        self.execute(
            "comms.tune",
            serde_json::json!({"radio":1,"channel_khz":station.channel_khz}),
        )?;
        self.comms.atc.radar_facility = Some(super::atc::radar_facility(&station));
        self.comms.atc.current = Some(station);
        Ok(())
    }

    fn nearest_towered(&self) -> Option<String> {
        let [pn, pe, _] = self.sim.state().position_m;
        self.stations()
            .into_iter()
            .filter(|st| st.kind == FacilityKind::Tower)
            .filter_map(|st| {
                let [n, e] = self.radio_project(st.position.latitude_deg, st.position.longitude_deg)?;
                Some((libm::hypot(n - pn, e - pe), st.airport))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
            .map(|v| v.1)
    }

    pub fn fleet_state(&self) -> FleetState {
        let Some(fleet) = &self.comms.fleet else {
            return FleetState {
                density: TrafficDensity::Off,
                airport: None,
                runway: None,
                seed: 0,
                aircraft: Vec::new(),
                frequencies: Vec::new(),
                stats: FleetStats::default(),
            };
        };
        let f = &fleet.field;
        let [pn, pe, pd] = self.sim.state().position_m;
        let tuned: Vec<u32> = self
            .comms
            .radios
            .iter()
            .filter_map(|r| carrier_thirds_hz(r.active_khz))
            .collect();
        let aircraft: Vec<FleetAircraft> = fleet
            .craft
            .iter()
            .filter(|a| !a.gone)
            .map(|a| {
                let st = f.st(a.station);
                let geo = self.sim.world().geodetic(a.body.pos[0], a.body.pos[1]);
                let grid_true = a.body.hdg + self.sim.heading_reference().grid_north_true_rad;
                let ft = a.body.alt_m / 0.3048;
                let class = self
                    .sim
                    .world()
                    .airspace_at(a.body.pos[0], a.body.pos[1], a.body.alt_m)
                    .ok()
                    .and_then(|q| {
                        q.containing
                            .iter()
                            .filter_map(|v| v.class)
                            .map(|c| format!("{c:?}"))
                            .min()
                    });
                FleetAircraft {
                    id: a.id,
                    callsign: a.ident.full.clone(),
                    registration: a.ident.registration.clone(),
                    aircraft_type: a.ident.spec().spoken.into(),
                    type_designator: a.ident.spec().designator.into(),
                    latitude_deg: geo.map(|g| g[0]),
                    longitude_deg: geo.map(|g| g[1]),
                    altitude_msl_ft: ft,
                    position_m: [a.body.pos[0], a.body.pos[1], -a.body.alt_m],
                    track_true_deg: grid_true.to_degrees().rem_euclid(360.),
                    track_magnetic_deg: f.mag.grid_to_magnetic(a.body.hdg).to_degrees().rem_euclid(360.),
                    ground_speed_kt: a.body.spd / 0.514_444,
                    vertical_rate_fpm: a.body.vs / 0.3048 * 60.,
                    on_ground: a.body.on_ground,
                    squawk: a.squawk.clone(),
                    frequency_khz: a.freq,
                    facility_id: st.id.clone(),
                    facility: phraseology::facility(st),
                    intent: FleetIntent {
                        kind: a.kind,
                        phase: a.stage.label().into(),
                        airport: f.airport.clone(),
                        runway: f.runway.clone(),
                    },
                    assigned_altitude_ft: a.alt_ft,
                    assigned_heading_deg: a.hdg_deg,
                    assigned_speed_kt: a.speed_kt,
                    cleared_to_land_or_approach: a.cleared,
                    airspace_class: class,
                    voice: a.ident.voice,
                    range_nm: libm::hypot(a.body.pos[0] - pn, a.body.pos[1] - pe) / 1852.,
                    bearing_true_deg: (libm::atan2(a.body.pos[1] - pe, a.body.pos[0] - pn)
                        + self.sim.heading_reference().grid_north_true_rad)
                        .to_degrees()
                        .rem_euclid(360.),
                    relative_altitude_ft: (a.body.alt_m + pd) / 0.3048,
                    on_user_frequency: carrier_thirds_hz(a.freq).is_some_and(|c| tuned.contains(&c)),
                }
            })
            .collect();
        let mut frequencies: Vec<FleetFrequency> = Vec::new();
        for a in &aircraft {
            match frequencies.iter_mut().find(|q| q.khz == a.frequency_khz) {
                Some(q) => q.aircraft += 1,
                None => frequencies.push(FleetFrequency {
                    khz: a.frequency_khz,
                    facility_id: a.facility_id.clone(),
                    facility: a.facility.clone(),
                    aircraft: 1,
                }),
            }
        }
        frequencies.sort_by_key(|q| q.khz);
        FleetState {
            density: fleet.density,
            airport: Some(f.airport.clone()),
            runway: Some(f.runway.clone()),
            seed: fleet.seed,
            aircraft,
            frequencies,
            stats: fleet.stats.clone(),
        }
    }

    /// Circuit-style targets for the user's tower rules (`follow the Cherokee on left base`).
    pub(super) fn fleet_targets(&self) -> Vec<TrafficAircraft> {
        let Some(fleet) = &self.comms.fleet else {
            return Vec::new();
        };
        let f = &fleet.field;
        fleet
            .craft
            .iter()
            .filter(|a| !a.gone)
            .filter_map(|a| {
                let leg = match (a.kind, a.stage) {
                    (FleetFlight::Pattern, Stage::PatFly) => {
                        if a.body.path.len() <= 1 {
                            PatternLeg::Final
                        } else {
                            PatternLeg::Base
                        }
                    }
                    (FleetFlight::Pattern, Stage::PatCall | Stage::AwaitPat) => PatternLeg::Downwind,
                    (FleetFlight::Arrival, s)
                        if s >= Stage::TowerCall
                            && s < Stage::Rollout
                            && a.body.path_remaining() < 6. * 1852. =>
                    {
                        PatternLeg::Final
                    }
                    _ => return None,
                };
                let geo = self.sim.world().geodetic(a.body.pos[0], a.body.pos[1]);
                let _ = f;
                Some(TrafficAircraft {
                    address: 0xf20000 + a.id,
                    callsign: a.ident.full.clone(),
                    registration: a.ident.registration.clone(),
                    aircraft_type: a.ident.spec().spoken.into(),
                    position_m: [a.body.pos[0], a.body.pos[1], -a.body.alt_m],
                    latitude_deg: geo.map(|g| g[0]),
                    longitude_deg: geo.map(|g| g[1]),
                    altitude_msl_m: a.body.alt_m,
                    heading_grid_rad: a.body.hdg,
                    speed_mps: a.body.spd,
                    vertical_speed_mps: a.body.vs,
                    leg,
                    sequence: 0,
                    following_pilot: false,
                })
            })
            .collect()
    }

    pub(super) fn sequence_fleet(&mut self, dt: f64) {
        let Some(mut fleet) = self.comms.fleet.take() else {
            return;
        };
        let now = self.sim.state().time_s;
        if now >= fleet.info_at {
            fleet.info = self
                .atis_report(&fleet.field.airport)
                .map_or_else(|| "alpha".into(), |r| r.information);
            fleet.info_at = now + 60.0;
        }
        // A call the user keyed over was blocked: the fleet sends it again once the channel clears.
        let blocked: Vec<u32> = fleet
            .chans
            .iter()
            .filter_map(|(khz, c)| {
                let (end, _) = c.inflight.as_ref()?;
                let id = c.event?;
                (*end <= now + 1e-9
                    && self
                        .comms
                        .record
                        .events
                        .iter()
                        .rev()
                        .any(|e| e.id == id && e.reception == Reception::Blocked))
                .then_some(*khz)
            })
            .collect();
        fleet.repeat_blocked(&blocked);
        fleet.advance(now, dt);
        self.comms.fleet = Some(fleet);
        // The controller's call to the user takes the carrier ahead of queued traffic calls.
        self.sequence_advisories();
        let Some(mut fleet) = self.comms.fleet.take() else {
            return;
        };
        let busy = self.busy_channels(&fleet);
        let transmissions = fleet.start_tx(now, &busy);
        for tx in transmissions {
            self.emit_fleet(&mut fleet, tx);
        }
        self.comms.fleet = Some(fleet);
    }

    /// Yield to anything already on a channel (the user, or a controller reply that is queued).
    pub(super) fn busy_channels(&self, fleet: &Fleet) -> Vec<u32> {
        let now = self.sim.state().time_s;
        fleet
            .chans
            .keys()
            .copied()
            .filter(|khz| {
                let carrier = carrier_thirds_hz(*khz);
                self.comms
                    .pending
                    .iter()
                    .any(|p| p.carrier == carrier && p.end_s + 0.5 > now)
                    || self
                        .comms
                        .atc
                        .replies
                        .iter()
                        .any(|r| carrier_thirds_hz(r.station.channel_khz) == carrier)
                    || self
                        .comms
                        .traffic_calls
                        .iter()
                        .any(|c| carrier_thirds_hz(c.khz) == carrier)
            })
            .collect()
    }

    fn emit_fleet(&mut self, fleet: &mut Fleet, tx: Tx) {
        let station = fleet.field.st(tx.station).clone();
        let (origin, range, speaker) = match tx.role {
            FleetRole::Controller => {
                let Some(origin) = self.station_origin(&station) else {
                    return;
                };
                (
                    origin,
                    station.range_nm,
                    Speaker::Facility {
                        id: station.id.clone(),
                        role: station.kind,
                    },
                )
            }
            FleetRole::Pilot => {
                let Some(origin) = tx.origin else { return };
                (
                    origin,
                    40.0,
                    Speaker::Traffic {
                        callsign: tx.callsign,
                    },
                )
            }
        };
        let Some(carrier) = carrier_thirds_hz(tx.freq) else {
            return;
        };
        // Only transmissions the user's radios can hear become events; the rest still happen.
        if !self.listening(carrier, origin, range) {
            return;
        }
        let freq = tx.freq;
        let id = self.start_com(
            tx.freq,
            speaker,
            tx.text,
            Meaning::Fleet { call: tx.call },
            EventSource::Rules,
            origin,
            range,
        );
        if let Some(c) = fleet.chans.get_mut(&freq) {
            c.event = Some(id);
        }
    }
}

#[cfg(test)]
mod tests;
