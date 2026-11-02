//! Display preferences convert at the panel boundary; simulation and commands retain SI.
use serde::{Deserialize, Serialize};

pub const PA_PER_INHG: f64 = 3386.389;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureUnit {
    #[default]
    InHg,
    Hpa,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineUnits {
    #[default]
    Us,
    Si,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayOptions {
    pub pressure: PressureUnit,
    pub engine: EngineUnits,
}
impl DisplayOptions {
    pub fn adjust_pressure(self, pressure_pa: f64, steps: f64) -> f64 {
        if !pressure_pa.is_finite() || !steps.is_finite() {
            return pressure_pa;
        }
        match self.pressure {
            PressureUnit::InHg => {
                ((libm::round(pressure_pa / PA_PER_INHG * 100.) + steps).clamp(2810., 3100.) / 100.)
                    * PA_PER_INHG
            }
            PressureUnit::Hpa => (libm::round(pressure_pa / 100.) + steps).clamp(950., 1050.) * 100.,
        }
    }
    pub(super) fn pressure_value(self, pressure_pa: f64) -> String {
        match self.pressure {
            PressureUnit::InHg => format!("{:.2}", pressure_pa / PA_PER_INHG),
            PressureUnit::Hpa => format!("{:.0}", pressure_pa / 100.),
        }
    }
    pub(super) fn pressure_label(self) -> &'static str {
        match self.pressure {
            PressureUnit::InHg => "inHg",
            PressureUnit::Hpa => "hPa",
        }
    }
}
