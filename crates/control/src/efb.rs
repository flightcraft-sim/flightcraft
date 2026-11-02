//! Tablet broadcast commands, with platform IO isolated from the deterministic simulation.
pub mod encode;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod sender;
use crate::{Command, CommandError, Session};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EfbSettings {
    pub enabled: bool,
    /// Numeric IPv4 tablet or subnet broadcast address; no DNS or port syntax.
    pub destination: String,
    pub gdl90: bool,
    pub simulator: bool,
}
impl Default for EfbSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            destination: "255.255.255.255".into(),
            gdl90: true,
            simulator: false,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct EfbStatus {
    pub settings: EfbSettings,
    pub available: bool,
    pub gdl90_packets: u64,
    pub simulator_packets: u64,
    /// Successful datagrams in the most recently completed one-second interval.
    pub packets_per_second: u64,
    pub last_error: Option<String>,
    /// Why no aircraft packets are currently being sent.
    pub reason: Option<String>,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EfbBroadcast {
    pub enabled: bool,
    pub destination: String,
    pub gdl90: bool,
    pub simulator: bool,
}
impl Command for EfbBroadcast {
    const NAME: &'static str = "efb.broadcast";
    const DESCRIPTION: &'static str = "Configure desktop UDP broadcast to tablet apps; off by default.";
    type Output = EfbStatus;
    fn run(self, session: &mut Session) -> Result<EfbStatus, CommandError> {
        let address = self
            .destination
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| crate::invalid(Self::NAME, "destination must be a numeric IPv4 address"))?;
        if address.is_unspecified()
            || address.is_multicast()
            || self.enabled && !self.gdl90 && !self.simulator
        {
            return Err(crate::invalid(
                Self::NAME,
                "use a unicast or broadcast IPv4 address, and enable at least one protocol",
            ));
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = session;
            Err(CommandError::Failed(
                "tablet UDP broadcast is unavailable in the browser".into(),
            ))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let settings = EfbSettings {
                enabled: self.enabled,
                destination: address.to_string(),
                gdl90: self.gdl90,
                simulator: self.simulator,
            };
            session
                .efb
                .as_ref()
                .ok_or_else(|| CommandError::Failed("tablet broadcast needs the desktop app".into()))?
                .configure(settings)
                .map_err(CommandError::Failed)?;
            session.update_efb();
            Ok(session.efb_status())
        }
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EfbGetStatus {}
impl Command for EfbGetStatus {
    const NAME: &'static str = "efb.status";
    const DESCRIPTION: &'static str = "Read tablet broadcast settings, packet counts, rate, and last error.";
    type Output = EfbStatus;
    fn run(self, session: &mut Session) -> Result<EfbStatus, CommandError> {
        Ok(session.efb_status())
    }
}
impl Session {
    pub fn efb_status(&self) -> EfbStatus {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(sender) = &self.efb {
            return sender.status();
        }
        EfbStatus {
            reason: Some(
                if cfg!(target_arch = "wasm32") {
                    "unavailable in the browser"
                } else {
                    "needs the desktop app"
                }
                .into(),
            ),
            ..Default::default()
        }
    }
    /// The app supplies a locally persisted random serial, independent of aircraft identity.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn initialize_efb(&mut self, serial: u64) -> Result<(), String> {
        self.efb = Some(sender::Sender::new(serial)?);
        self.update_efb();
        Ok(())
    }
    /// Publish the latest truth once per frame. The sender never reads the simulation.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn update_efb(&self) {
        if let Some(sender) = &self.efb {
            sender.update(aircraft(self));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn aircraft(session: &Session) -> Option<encode::Aircraft> {
    let sim = session.sim();
    let r = sim.readout();
    let [north, east, _] = sim.state().position_m;
    let [lat, lon] = [r.latitude_deg?, r.longitude_deg?];
    // A geodetic displacement gives true azimuth and accounts for grid convergence.
    let bearing = |n: f64, e: f64| -> Option<f64> {
        let [lat2, lon2] = sim.world().geodetic(north + n, east + e)?;
        let phi = lat.to_radians();
        let dphi = (lat2 - lat).to_radians();
        let dl = ((lon2 - lon + 180.0).rem_euclid(360.0) - 180.0).to_radians();
        // WGS84 meridional and prime-vertical metric factors. Their common
        // denominator and semi-major axis cancel in atan2. A spherical bearing
        // would bias oblique tracks because a degree of latitude and longitude
        // have different ellipsoidal radii of curvature.
        let flattening = 1.0 / 298.257_223_563;
        let e2 = flattening * (2.0 - flattening);
        Some(
            (dl * phi.cos() * (1.0 - e2 * phi.sin().powi(2)))
                .atan2(dphi * (1.0 - e2))
                .to_degrees()
                .rem_euclid(360.0),
        )
    };
    let heading = bearing(r.heading_grid_rad.cos(), r.heading_grid_rad.sin())?;
    let [vn, ve, _] = sim.state().velocity_mps;
    let track = if r.ground_speed_mps > 0.01 {
        bearing(vn / r.ground_speed_mps, ve / r.ground_speed_mps)?
    } else {
        heading
    };
    let p = flightcraft_world::atmosphere(r.altitude_msl_m).pressure_pa;
    // Invert the same ISA lapse-rate/isothermal atmosphere used by the simulation.
    let p11 = flightcraft_world::atmosphere(11000.0).pressure_pa;
    let pressure_altitude = if p >= p11 {
        288.15 / 0.0065
            * (1.0
                - (p / 101325.0)
                    .powf(0.0065 * flightcraft_world::GAS_CONSTANT / flightcraft_world::GRAVITY_MPS2))
    } else {
        11000.0 - flightcraft_world::GAS_CONSTANT * 216.65 / flightcraft_world::GRAVITY_MPS2 * (p / p11).ln()
    };
    Some(encode::Aircraft {
        latitude_deg: lat,
        longitude_deg: lon,
        altitude_msl_m: r.altitude_msl_m,
        pressure_altitude_m: Some(pressure_altitude),
        track_deg: track,
        ground_speed_mps: r.ground_speed_mps,
        vertical_speed_mps: r.vertical_speed_mps,
        heading_deg: heading,
        pitch_deg: r.pitch_rad.to_degrees(),
        roll_deg: r.roll_rad.to_degrees(),
        airborne: !r.on_ground,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod encode_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)]
mod truth_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn truth_uses_ambient_pressure_true_bearings_and_ignores_sensor_failures() {
        let mut session = Session::new().unwrap();
        assert!(aircraft(&session).is_none());
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
        session
            .install_region(crate::read_region(&directory).unwrap())
            .unwrap();
        for height in [1000.0, 15000.0] {
            session
                .run(crate::Reset {
                    world: flightcraft_world::WorldChoice::Region("new-york".into()),
                    start: crate::ResetStart::Air {
                        height_m: height,
                        airspeed_mps: 40.0,
                        heading_rad: 0.0,
                    },
                    weather: Some(flightcraft_sim::Weather {
                        wind_from_deg: 270.0,
                        wind_speed_mps: 10.0,
                        ..Default::default()
                    }),
                    paused: None,
                })
                .unwrap();
            let a = aircraft(&session).unwrap();
            assert!((a.pressure_altitude_m.unwrap() - height).abs() < 1e-8);
            assert!((a.altitude_msl_m - height).abs() < 1e-8);
            let true_heading = session
                .sim()
                .heading_reference()
                .magnetic_to_true(0.0)
                .to_degrees();
            let angle_error = |a: f64, b: f64| (a - b + 180.0).rem_euclid(360.0) - 180.0;
            assert!(angle_error(a.heading_deg, true_heading).abs() < 1e-5);
            let true_track = true_heading + (10.0_f64 / 40.0).atan().to_degrees();
            assert!(angle_error(a.track_deg, true_track).abs() < 1e-5);
            assert!((a.ground_speed_mps - 1700.0_f64.sqrt()).abs() < 1e-8);
            session
                .execute("failure.set", json!({"failure":{"kind":"static_blocked"}}))
                .unwrap();
            session
                .execute("failure.set", json!({"failure":{"kind":"pitot_blocked"}}))
                .unwrap();
            let failed = aircraft(&session).unwrap();
            assert_eq!(a.pressure_altitude_m, failed.pressure_altitude_m);
            assert_eq!(a.pitch_deg, failed.pitch_deg);
            assert_eq!(a.roll_deg, failed.roll_deg);
        }
    }
}
