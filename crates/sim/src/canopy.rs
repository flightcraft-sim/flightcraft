//! Deterministic canopy state and guarded cabin actions. Distances belong to the exterior binding.
use crate::{Sim, SimError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Low-speed guard is a simulator estimate, not a published Sling limitation.
pub const MAX_OPEN_GROUND_SPEED_MPS: f64 = 2.;
pub const MAX_OPEN_AIRSPEED_MPS: f64 = 5.;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanopyState {
    pub sliding: bool,
    #[schemars(range(min = 0, max = 1))]
    pub open_fraction: f64,
    pub latched: bool,
}
impl Default for CanopyState {
    fn default() -> Self {
        Self::new(false, false)
    }
}
impl CanopyState {
    pub fn new(sliding: bool, parked: bool) -> Self {
        Self {
            sliding,
            open_fraction: if sliding && parked { 1. } else { 0. },
            latched: !sliding || !parked,
        }
    }
    pub fn validate(self) -> Result<(), SimError> {
        if !self.open_fraction.is_finite()
            || !(0. ..=1.).contains(&self.open_fraction)
            || (self.latched && self.open_fraction != 0.)
            || (!self.sliding && (self.open_fraction != 0. || !self.latched))
        {
            return Err(SimError::InvalidSystems("invalid canopy state".into()));
        }
        Ok(())
    }
    pub fn secure(self) -> bool {
        !self.sliding || (self.open_fraction == 0. && self.latched)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanopyAction {
    Open,
    Close,
    Latch,
    Unlatch,
}
impl Sim {
    pub fn canopy(&self) -> CanopyState {
        self.systems.canopy
    }
    /// Validate the entire action before changing state. Closing never silently latches.
    pub fn set_canopy(
        &mut self,
        action: CanopyAction,
        fraction: Option<f64>,
    ) -> Result<CanopyState, SimError> {
        let refuse = |s: &str| SimError::InvalidSystems(s.into());
        let mut canopy = self.canopy();
        if !canopy.sliding {
            return Err(refuse("aircraft has no sliding canopy"));
        }
        if fraction.is_some_and(|v| !v.is_finite() || !(0. ..=1.).contains(&v))
            || (fraction.is_some() && action != CanopyAction::Open)
        {
            return Err(refuse(
                "fraction must be finite, between 0 and 1, and used only with open",
            ));
        }
        match action {
            CanopyAction::Open => {
                let r = self.readout();
                if !r.on_ground {
                    return Err(refuse("canopy opening refused: aircraft must be on the ground"));
                }
                if r.ground_speed_mps > MAX_OPEN_GROUND_SPEED_MPS
                    || r.true_airspeed_mps > MAX_OPEN_AIRSPEED_MPS
                {
                    return Err(refuse(
                        "canopy opening refused: ground speed must be at most 2 m/s and airspeed at most 5 m/s",
                    ));
                }
                if canopy.latched {
                    return Err(refuse("canopy opening refused: unlatch first"));
                }
                canopy.open_fraction = fraction.unwrap_or(1.);
            }
            CanopyAction::Close => canopy.open_fraction = 0.,
            CanopyAction::Latch => {
                if canopy.open_fraction != 0. {
                    return Err(refuse("canopy latching refused: close completely first"));
                }
                canopy.latched = true;
            }
            CanopyAction::Unlatch => canopy.latched = false,
        }
        self.systems.canopy = canopy;
        self.systems_changed = true;
        self.systems.failures = self.failures();
        if let Some(modern) = &mut self.systems.modern {
            modern.door_open = self.systems.failures.door_open;
        }
        Ok(canopy)
    }
}
