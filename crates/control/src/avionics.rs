//! One tagged command surface for conventional, standby, and integrated avionics.
use crate::{Command, CommandError, GlassState, Session};
pub use flightcraft_sim::avionics::AvionicsSetting;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAvionics {
    pub setting: AvionicsSetting,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetAvionics {}
pub type GetAvionicsState = GetAvionics;
#[derive(Debug, Serialize, JsonSchema)]
pub struct AvionicsState {
    #[serde(flatten)]
    pub standby: flightcraft_sim::avionics::AvionicsState,
    pub glass_suite: GlassState,
    pub autopilot: Option<flightcraft_sim::equipment::EquipmentState>,
}
impl Session {
    fn avionics_snapshot(&self) -> AvionicsState {
        AvionicsState {
            standby: self.sim().systems().avionics.clone(),
            glass_suite: self.glass_state(),
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
#[derive(Debug, Deserialize, JsonSchema)]
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
