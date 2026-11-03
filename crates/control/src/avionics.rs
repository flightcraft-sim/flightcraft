//! One tagged command surface for conventional, standby, and integrated avionics.
use crate::{Command, CommandError, GlassState, Session};
pub use flightcraft_sim::avionics::AvionicsSetting;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetAvionics {
    pub setting: AvionicsSetting,
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetAvionics {}
pub type GetAvionicsState = GetAvionics;
/// Ideal passive whiskey compass; independent of AHRS, gyro power, and instrument failures.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MagneticCompass {
    /// Magnetic heading, clockwise from magnetic north.
    pub heading_deg: f64,
    /// East-positive WMM declination at the aircraft and simulation date.
    pub declination_deg: f64,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct AvionicsState {
    #[serde(flatten)]
    pub standby: flightcraft_sim::avionics::AvionicsState,
    pub glass_suite: GlassState,
    pub magnetic_compass: MagneticCompass,
    pub procedures: flightcraft_sim::avionics::procedures::ProcedureReceiver,
    pub nav_radios: flightcraft_sim::avionics::radio::Radios,
    pub radio_signals: [flightcraft_sim::avionics::radio::RadioState; 2],
    pub autopilot: Option<flightcraft_sim::equipment::EquipmentState>,
}
impl Session {
    fn avionics_snapshot(&self) -> AvionicsState {
        AvionicsState {
            standby: self.sim().systems().avionics.clone(),
            glass_suite: self.glass_state(),
            magnetic_compass: MagneticCompass {
                heading_deg: self.sim().readout().heading_rad.to_degrees(),
                declination_deg: self.sim().heading_reference().declination_rad.to_degrees(),
            },
            procedures: self.sim().procedure_receiver().clone(),
            nav_radios: self.sim().radios().clone(),
            radio_signals: self.sim().radio_state(),
            autopilot: self.sim().installed_autopilot().cloned(),
        }
    }
}
impl Command for SetAvionics {
    const NAME: &'static str = "avionics.set";
    const DESCRIPTION: &'static str =
        "Set, step, or sync the shared heading bug, or control standby and glass equipment.";
    type Output = AvionicsState;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        match self.setting {
            AvionicsSetting::Glass { setting } => session.set_glass(setting)?,
            setting => session.sim_mut().set_avionics(setting)?,
        }
        Ok(session.avionics_snapshot())
    }
}
impl Command for GetAvionics {
    const NAME: &'static str = "avionics.state";
    const DESCRIPTION: &'static str =
        "Read standby and glass displayed values and flags without advancing time.";
    type Output = AvionicsState;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(session.avionics_snapshot())
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AutopilotOutput {
    pub autopilot: Option<flightcraft_sim::equipment::EquipmentState>,
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetAutopilot {}
impl Command for GetAutopilot {
    const NAME: &'static str = "autopilot.state";
    const DESCRIPTION: &'static str =
        "Read installed autopilot modes, armed modes, targets, servos, director commands, and failures.";
    type Output = AutopilotOutput;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(AutopilotOutput {
            autopilot: session.sim.installed_autopilot().cloned(),
        })
    }
}
