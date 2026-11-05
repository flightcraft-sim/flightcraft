//! Ground hazards a pre-flight can leave behind, and how they act on the failure and engine models.
//!
//! A pitot cover is the existing blocked pitot, missing flap hardware is the existing flap
//! asymmetry, and missing aileron hardware jams the aileron in the existing control-jam path once
//! airborne. Chocks and tie-down ropes only hold the aircraft still. The remaining findings act
//! through physical models:
//!
//! - **Low oil** leaves the tank below the dipstick MIN mark. The engine's oil system
//!   (`systems.rs`) then loses pressure and runs hotter, and starvation wears the engine until it
//!   seizes.
//! - **Damaged tyre** rolls harder and drags its side of the aircraft off the centreline. It
//!   fatigues with rolling distance and touchdown energy until it bursts (see [`TyreWear`]).
//! - **Loose cowl fastener** shakes the airframe under power and, flown fast, lets the cowl go.
//! - **Cracked wheel fairing** adds a little parasite drag.
use flightcraft_aero::{Failures, Wheel};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Height above ground below which a restraint still holds the aircraft, in metres.
pub const RESTRAINT_HEIGHT_M: f64 = 0.3;
/// Aileron position, normalised, at which loose or missing hinge hardware leaves the surface.
pub const AILERON_HARDWARE_JAM: f64 = 0.35;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroundHazards {
    /// A pitot cover is still fitted: the pitot is blocked.
    #[serde(default)]
    pub pitot_cover: bool,
    /// Wheel chocks are still in place: the aircraft cannot roll.
    #[serde(default)]
    pub chocks: bool,
    /// A tie-down rope is still attached: the aircraft can neither roll nor lift off.
    #[serde(default)]
    pub tie_downs: bool,
    /// Flap hinge hardware is missing: flaps become asymmetric when used.
    #[serde(default)]
    pub flap_hardware: bool,
    /// Aileron hinge hardware is missing: the aileron sticks off-centre in flight.
    #[serde(default)]
    pub aileron_hardware: bool,
    /// The oil is below the dipstick MIN mark.
    #[serde(default)]
    pub low_oil: bool,
    /// An engine cowl fastener is loose.
    #[serde(default)]
    pub loose_cowl: bool,
    /// A tyre is cut or worn through to the cord: which wheel it is.
    #[serde(default)]
    pub damaged_tyre: Option<Wheel>,
    /// A wheel fairing is cracked.
    #[serde(default)]
    pub fairing_crack: bool,
}

/// Fatigue of a damaged tyre. It fails when its integrity reaches zero.
///
/// An original heuristic, not measured data: the tyre loses `1 / ROLL_LIFE_M` per metre rolled
/// under load and `TOUCHDOWN_ENERGY * sink^2` at each touchdown, so a taxi, a take-off roll, and a
/// normal landing leave it nearly spent and the landing roll or a firm landing bursts it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TyreWear {
    /// 1 when sound, 0 when it bursts.
    pub integrity: f64,
    /// The tyre has burst.
    pub flat: bool,
}

impl Default for TyreWear {
    fn default() -> Self {
        Self {
            integrity: 1.0,
            flat: false,
        }
    }
}

/// Height above ground, metres, below which a wheel carries load for tyre wear.
pub const TYRE_LOADED_HEIGHT_M: f64 = 0.05;
/// Metres rolled under load that use up a damaged tyre.
pub const TYRE_ROLL_LIFE_M: f64 = 1_000.0;
/// Integrity lost per (m/s)^2 of sink rate at touchdown.
pub const TYRE_TOUCHDOWN_ENERGY: f64 = 0.6;
/// Indicated airspeed, m/s, above which a loose cowl starts to lift.
pub const COWL_LIFT_IAS_MPS: f64 = 40.0;
/// Stress, in seconds of lift at the reference excess speed, at which the cowl comes off.
pub const COWL_FAIL_STRESS_S: f64 = 30.0;
/// Excess speed over [`COWL_LIFT_IAS_MPS`], m/s, that adds one second of stress per second.
pub const COWL_REFERENCE_EXCESS_MPS: f64 = 20.0;

impl TyreWear {
    /// Advance one step on the ground at `ground_speed_mps`. `touchdown_sink_mps` is the sink rate
    /// on the step the wheels arrive, if they did.
    pub fn step(&mut self, dt_s: f64, ground_speed_mps: f64, touchdown_sink_mps: Option<f64>) {
        if self.flat {
            return;
        }
        self.integrity -= ground_speed_mps.abs() * dt_s / TYRE_ROLL_LIFE_M;
        if let Some(sink) = touchdown_sink_mps {
            self.integrity -= TYRE_TOUCHDOWN_ENERGY * sink.max(0.0) * sink.max(0.0);
        }
        if self.integrity <= 0.0 {
            self.integrity = 0.0;
            self.flat = true;
        }
    }
}

/// Stress on a loose cowl at `ias_mps` over `dt_s`, in seconds-equivalent.
#[must_use]
pub fn cowl_stress_rate(ias_mps: f64) -> f64 {
    ((ias_mps - COWL_LIFT_IAS_MPS) / COWL_REFERENCE_EXCESS_MPS).clamp(0.0, 2.0)
}

impl GroundHazards {
    #[must_use]
    pub fn restrained(self) -> bool {
        self.chocks || self.tie_downs
    }

    /// Overlay the hazards on a failure record. Aileron hardware only matters once airborne.
    pub fn apply(self, failures: &mut Failures, airborne: bool) {
        failures.pitot_blocked |= self.pitot_cover;
        failures.flap_asymmetry |= self.flap_hardware;
        if self.aileron_hardware && airborne && failures.aileron.is_none() {
            failures.aileron = Some(AILERON_HARDWARE_JAM);
        }
        failures.cowl_loose |= self.loose_cowl;
        failures.fairing_crack |= self.fairing_crack;
        if failures.damaged_tyre.is_none() {
            failures.damaged_tyre = self.damaged_tyre;
        }
    }

    /// Whether any of the findings added after the first five is present.
    #[must_use]
    pub fn has_physical_findings(self) -> bool {
        self.low_oil || self.loose_cowl || self.damaged_tyre.is_some() || self.fairing_crack
    }

    pub(crate) fn hash_words(self) -> Vec<u64> {
        let mut words = vec![
            u64::from(self.pitot_cover),
            u64::from(self.chocks),
            u64::from(self.tie_downs),
            u64::from(self.flap_hardware),
            u64::from(self.aileron_hardware),
        ];
        // The later findings extend the stream only when present, so older hashes are kept.
        if self.has_physical_findings() {
            words.extend([
                u64::from(self.low_oil),
                u64::from(self.loose_cowl),
                match self.damaged_tyre {
                    None => 0,
                    Some(Wheel::Left) => 1,
                    Some(Wheel::Right) => 2,
                    Some(Wheel::Nose) => 3,
                },
                u64::from(self.fairing_crack),
            ]);
        }
        words
    }
}
