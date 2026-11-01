//! Deterministic fixed-step holds for the Corvid. All controller memory belongs to `Sim`.
//!
//! A vertical hold owns elevator; throttle-mode airspeed owns throttle. Pitch-mode airspeed
//! owns elevator and sets full throttle. Heading and wings level own aileron and rudder.
//! Brakes and flaps always remain with the pilot. Disengaging leaves the last controls in place.

use crate::{Controls, DT_S, Readout, SimError};
use flightcraft_aero as aero;
use flightcraft_world as world;
use serde::{Deserialize, Serialize};

/// How airspeed is controlled. Pitch mode trades excess speed for climb at full power.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AirspeedMode {
    #[default]
    Throttle,
    Pitch,
}

/// Optional holds. Only one vertical hold and one lateral hold may be engaged at a time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Targets {
    pub pitch_rad: Option<f64>,
    pub altitude_m: Option<f64>,
    pub vertical_speed_mps: Option<f64>,
    pub heading_rad: Option<f64>,
    pub airspeed_mps: Option<f64>,
    pub wings_level: bool,
    /// Maximum commanded bank magnitude, radians (0.05 to 0.6).
    pub bank_limit_rad: f64,
    pub airspeed_mode: AirspeedMode,
}

impl Default for Targets {
    fn default() -> Self {
        Self {
            pitch_rad: None,
            altitude_m: None,
            vertical_speed_mps: None,
            heading_rad: None,
            airspeed_mps: None,
            wings_level: false,
            bank_limit_rad: 0.4,
            airspeed_mode: AirspeedMode::Throttle,
        }
    }
}

impl Targets {
    /// Validate finite, bounded targets and unambiguous control ownership.
    pub fn validate(&self) -> Result<(), SimError> {
        for (name, value, lo, hi) in [
            ("pitch_rad", self.pitch_rad, -0.25, 0.35),
            ("altitude_m", self.altitude_m, 2.0, 10_000.0),
            ("vertical_speed_mps", self.vertical_speed_mps, -8.0, 8.0),
            ("heading_rad", self.heading_rad, 0.0, core::f64::consts::TAU),
            ("airspeed_mps", self.airspeed_mps, 25.0, 80.0),
            ("bank_limit_rad", Some(self.bank_limit_rad), 0.05, 0.6),
        ] {
            if value.is_some_and(|v| !v.is_finite() || !(lo..=hi).contains(&v)) {
                return Err(SimError::InvalidAutopilot(format!(
                    "{name} must be finite and between {lo} and {hi}"
                )));
            }
        }
        let vertical_count = usize::from(self.pitch_rad.is_some())
            + usize::from(self.altitude_m.is_some())
            + usize::from(self.vertical_speed_mps.is_some())
            + usize::from(self.airspeed_mps.is_some() && self.airspeed_mode == AirspeedMode::Pitch);
        if vertical_count > 1 || (self.heading_rad.is_some() && self.wings_level) {
            return Err(SimError::InvalidAutopilot(
                "engage only one vertical hold and one lateral hold".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn engaged(self) -> bool {
        self.pitch_rad.is_some()
            || self.altitude_m.is_some()
            || self.vertical_speed_mps.is_some()
            || self.heading_rad.is_some()
            || self.airspeed_mps.is_some()
            || self.wings_level
    }
}

/// PI feedback plus a measured rate for derivative damping. Conditional integration avoids
/// windup: a saturated output only integrates errors that move it back toward its allowed range.
#[derive(Clone, Debug, Default)]
struct Pid {
    integral: f64,
}

impl Pid {
    fn output(&mut self, error: f64, base: f64, kp: f64, ki: f64, lo: f64, hi: f64) -> f64 {
        let candidate = self.integral + error * ki * DT_S;
        let output = base + kp * error + candidate;
        if (lo..=hi).contains(&output) || (output > hi && error < 0.0) || (output < lo && error > 0.0) {
            // Integral trim must have both signs, including for throttle (output 0..1).
            // This lets it remove feed-forward bias as well as add missing control effort.
            self.integral = candidate.clamp(lo - hi, hi - lo);
        }
        (base + kp * error + self.integral).clamp(lo, hi)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Autopilot {
    targets: Targets,
    pitch: Pid,
    speed: Pid,
    climb: Pid,
    heading: Pid,
    roll: Pid,
}

impl Autopilot {
    pub(crate) fn targets(&self) -> Targets {
        self.targets
    }

    pub(crate) fn set(&mut self, targets: Targets) {
        // Reset only affected loops, so changing heading does not disturb vertical trim.
        if targets.pitch_rad != self.targets.pitch_rad
            || targets.altitude_m != self.targets.altitude_m
            || targets.vertical_speed_mps != self.targets.vertical_speed_mps
            || targets.airspeed_mode != self.targets.airspeed_mode
        {
            self.pitch = Pid::default();
            self.climb = Pid::default();
        }
        if targets.airspeed_mps != self.targets.airspeed_mps
            || targets.airspeed_mode != self.targets.airspeed_mode
        {
            self.speed = Pid::default();
        }
        if targets.heading_rad != self.targets.heading_rad || targets.wings_level != self.targets.wings_level
        {
            self.heading = Pid::default();
            self.roll = Pid::default();
        }
        self.targets = targets;
    }

    pub(crate) fn update(&mut self, r: Readout, rates: [f64; 3]) -> Controls {
        let t = self.targets;
        let [p, q, yaw_rate] = rates;
        let speed = r.true_airspeed_mps;
        let mut c = r.controls;
        let bank = t
            .heading_rad
            .map(|heading| {
                self.heading.output(
                    wrap(heading - r.heading_rad),
                    0.0,
                    1.3,
                    0.015,
                    -t.bank_limit_rad,
                    t.bank_limit_rad,
                )
            })
            .or(if t.wings_level { Some(0.0) } else { None });
        if let Some(bank) = bank {
            c.aileron = self
                .roll
                .output(bank - r.roll_rad, -0.35 * p, 1.4, 0.06, -1.0, 1.0);
            c.rudder = (0.8 * (world::GRAVITY_MPS2 * libm::tan(r.roll_rad) / speed.max(15.0) - yaw_rate)
                - 0.8 * r.beta_rad)
                .clamp(-1.0, 1.0);
            if r.on_ground {
                c.rudder = t.heading_rad.map_or(0.0, |h| {
                    (wrap(h - r.heading_rad) * 1.5 - yaw_rate * 0.4).clamp(-1.0, 1.0)
                });
            }
        }
        let mut climb = t.vertical_speed_mps.or_else(|| {
            t.altitude_m
                .map(|h| ((h - r.altitude_msl_m) * 0.12).clamp(-3.0, 4.0))
        });
        if t.airspeed_mode == AirspeedMode::Pitch
            && let Some(target) = t.airspeed_mps
        {
            c.throttle = 1.0;
            // Speed above target commands a steeper climb; the integral learns available
            // excess power. Reduced power permits a steeper descent to hold glide speed.
            // At saturation, the aircraft is allowed to accelerate or descend.
            climb = Some(self.speed.output(
                speed - target,
                0.0,
                0.9,
                0.10,
                if r.failures.engine_power_fraction < 1.0 {
                    -8.0
                } else {
                    -3.0
                },
                6.0,
            ));
        }
        let rho = world::atmosphere(r.altitude_msl_m).density_kg_m3;
        let target_pitch = t.pitch_rad.or_else(|| {
            climb.map(|desired| {
                let cl = 2.0 * aero::MASS_KG * world::GRAVITY_MPS2
                    / (rho
                        * speed.max(26.0)
                        * speed.max(26.0)
                        * aero::WING_AREA_M2
                        * libm::cos(bank.unwrap_or(r.roll_rad)));
                let alpha = ((cl - 0.25) / 5.0).clamp(-0.05, 0.23);
                self.climb.output(
                    desired - r.vertical_speed_mps,
                    alpha + desired / speed.max(25.0),
                    0.015,
                    0.004,
                    -0.15,
                    0.30,
                )
            })
        });
        if let Some(pitch) = target_pitch {
            c.elevator = self.pitch.output(
                pitch - r.pitch_rad,
                -aero::coefficients(r.alpha_rad, c.flaps).pitch / aero::DERIVATIVES.cm_elevator - 0.8 * q,
                2.2,
                0.15,
                -1.0,
                1.0,
            );
        }
        if t.airspeed_mode == AirspeedMode::Throttle
            && let Some(target) = t.airspeed_mps
        {
            let cl = 2.0 * aero::MASS_KG * world::GRAVITY_MPS2 / (rho * target * target * aero::WING_AREA_M2);
            let alpha = ((cl - 0.25) / 5.0).clamp(-0.05, 0.23);
            let drag =
                0.5 * rho * target * target * aero::WING_AREA_M2 * aero::coefficients(alpha, c.flaps).drag;
            let needed =
                drag + aero::MASS_KG * world::GRAVITY_MPS2 * climb.unwrap_or(r.vertical_speed_mps) / target;
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..24 {
                let mid = (lo + hi) * 0.5;
                if aero::propulsion(target, mid, rho).0 > needed {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            c.throttle = self
                .speed
                .output(target - speed, (lo + hi) * 0.5, 0.07, 0.012, 0.0, 1.0);
        }
        c.clamped()
    }
}

/// Signed shortest heading difference, including across north.
#[must_use]
pub fn wrap(angle: f64) -> f64 {
    (angle + core::f64::consts::PI).rem_euclid(core::f64::consts::TAU) - core::f64::consts::PI
}
