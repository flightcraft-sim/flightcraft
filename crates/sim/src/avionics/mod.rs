//! Independent electronic instruments, stepped only on the fixed simulation clock.
pub mod electronic_attitude;
pub mod electronic_hsi;
mod sensor;
use crate::{Readout, SimError, State, systems::SystemsState};
use flightcraft_world::World;
use schemars::JsonSchema;
pub use sensor::Power;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    ElectronicAttitude,
    ElectronicHsi,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AvionicsSetting {
    HeadingBug {
        #[schemars(range(min = 0, max = 360))]
        degrees: f64,
    },
    Course {
        #[schemars(range(min = 0, max = 360))]
        degrees: f64,
    },
    Baro {
        #[schemars(range(min = 95000, max = 105000))]
        pressure_pa: f64,
    },
    SelectedAltitude {
        #[schemars(range(min = -1000, max = 30000))]
        feet: f64,
    },
    DirectTo {
        #[schemars(length(min = 1, max = 8))]
        ident: String,
    },
    Power {
        unit: Unit,
        on: bool,
    },
    AhrsFailure {
        unit: Unit,
        failed: bool,
    },
    AirDataFailure {
        failed: bool,
    },
    Reversion {
        enabled: bool,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct AvionicsState {
    pub installed: bool,
    pub electronic_attitude: electronic_attitude::ElectronicAttitude,
    pub electronic_hsi: electronic_hsi::ElectronicHsi,
}
impl AvionicsState {
    pub(crate) fn hash_words(&self) -> Vec<u64> {
        let mut words = self.electronic_attitude.hash_words();
        words.extend(self.electronic_hsi.hash_words());
        words.push(u64::from(self.installed));
        words
    }
    pub(crate) fn configure(configuration: &flightcraft_aero::ResolvedAircraft) -> Self {
        let has = |id| configuration.panel.instruments.iter().any(|i| i.module == id);
        Self {
            installed: has("electronic_attitude") && has("electronic_hsi"),
            ..Self::default()
        }
    }
    pub(crate) fn update(
        &mut self,
        state: State,
        truth: Readout,
        systems: &SystemsState,
        world: &World,
        advance: bool,
    ) {
        if !self.installed {
            return;
        }
        self.electronic_attitude
            .update("electronic_attitude", state, truth, systems, advance);
        self.electronic_hsi.update(
            state,
            truth,
            systems,
            world,
            !self.electronic_attitude.attitude_valid
                && (!self.electronic_attitude.aligning
                    || self.electronic_hsi.reversion_attitude.attitude_valid),
            advance,
        );
    }
    pub(crate) fn set(
        &mut self,
        setting: AvionicsSetting,
        world: &World,
        state: State,
    ) -> Result<(), SimError> {
        if !self.installed {
            return Err(invalid("standby pair is not installed"));
        }
        let a = &mut self.electronic_attitude;
        let h = &mut self.electronic_hsi;
        match setting {
            AvionicsSetting::HeadingBug { degrees } => {
                bounded(degrees, 0., 360.)?;
                h.heading_bug_deg = degrees.rem_euclid(360.);
            }
            AvionicsSetting::Course { degrees } => {
                bounded(degrees, 0., 360.)?;
                h.course_deg = degrees.rem_euclid(360.);
            }
            AvionicsSetting::Baro { pressure_pa } => {
                bounded(pressure_pa, 95000., 105000.)?;
                a.baro_inhg = pressure_pa / 3386.389;
                h.reversion_attitude.baro_inhg = a.baro_inhg;
            }
            AvionicsSetting::SelectedAltitude { feet } => {
                bounded(feet, -1000., 30000.)?;
                a.selected_altitude_ft = feet;
                h.reversion_attitude.selected_altitude_ft = feet;
            }
            AvionicsSetting::DirectTo { ident } => h.direct_to(&ident, world, state)?,
            AvionicsSetting::Power { unit, on } => match unit {
                Unit::ElectronicAttitude => a.power.switch_on = on,
                Unit::ElectronicHsi => h.reversion_attitude.power.switch_on = on,
            },
            AvionicsSetting::AhrsFailure { unit, failed } => match unit {
                Unit::ElectronicAttitude => a.sensor.failed = failed,
                Unit::ElectronicHsi => h.reversion_attitude.sensor.failed = failed,
            },
            AvionicsSetting::AirDataFailure { failed } => {
                a.air_data_failed = failed;
                h.reversion_attitude.air_data_failed = failed;
            }
            AvionicsSetting::Reversion { enabled } => h.manual_reversion = enabled,
        }
        Ok(())
    }
}
fn invalid(message: &str) -> SimError {
    SimError::InvalidSystems(message.into())
}
fn bounded(v: f64, lo: f64, hi: f64) -> Result<(), SimError> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(invalid("avionics setting outside range"))
    }
}
