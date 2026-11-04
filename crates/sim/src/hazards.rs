//! Ground hazards a pre-flight can leave behind, and how they act on the existing failure models.
//!
//! Nothing here is a new failure model: a pitot cover is the existing blocked pitot, missing flap
//! hardware is the existing flap asymmetry, and missing aileron hardware jams the aileron in the
//! existing control-jam path once airborne. Chocks and tie-down ropes only hold the aircraft still.
use flightcraft_aero::Failures;
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
    }

    pub(crate) fn hash_words(self) -> [u64; 5] {
        [
            u64::from(self.pitot_cover),
            u64::from(self.chocks),
            u64::from(self.tie_downs),
            u64::from(self.flap_hardware),
            u64::from(self.aileron_hardware),
        ]
    }
}
