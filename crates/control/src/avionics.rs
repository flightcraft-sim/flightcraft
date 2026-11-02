//! Typed command surface shared by the panel, shortcuts, scripts, and MCP.
use crate::{Command, CommandError, Session};
use flightcraft_sim::avionics::{AvionicsSetting, AvionicsState};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAvionics {
    pub setting: AvionicsSetting,
}
impl Command for SetAvionics {
    const NAME: &'static str = "avionics.set";
    const DESCRIPTION: &'static str = "Set a standby unit's heading bug, course, baro, altitude, direct-to airport, power, reversion, or sensor failure.";
    type Output = AvionicsState;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        session.sim_mut().set_avionics(self.setting)?;
        Ok(session.sim().systems().avionics.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetAvionics {}
impl Command for GetAvionics {
    const NAME: &'static str = "avionics.state";
    const DESCRIPTION: &'static str = "Read the displayed standby values, power, alignment, validity flags, GPS guidance, and reversion state without advancing time.";
    type Output = AvionicsState;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(session.sim().systems().avionics.clone())
    }
}
