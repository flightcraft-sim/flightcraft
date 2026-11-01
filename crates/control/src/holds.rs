//! Hold commands and bounded, condition-driven stepping.

use crate::{Command, CommandError, MAX_STEPS, Session, invalid, provided, range, without_default};
use flightcraft_sim::{
    Readout, State,
    autopilot::{AirspeedMode, Targets, wrap},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A boolean that accepts only `false`, used to disengage a numeric hold.
#[derive(Debug, Deserialize)]
#[serde(try_from = "bool")]
pub struct Disengage;

impl TryFrom<bool> for Disengage {
    type Error = &'static str;
    fn try_from(value: bool) -> Result<Self, Self::Error> {
        if value {
            Err("use a numeric target to engage a hold")
        } else {
            Ok(Self)
        }
    }
}

impl JsonSchema for Disengage {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Disengage".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type": "boolean", "const": false})
    }
}

/// Numeric target, or `null`/`false` to disengage. Omitted fields keep their current setting.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum HoldTarget {
    Target(f64),
    Off(Disengage),
    Null(()),
}

impl HoldTarget {
    fn target(self) -> Option<f64> {
        match self {
            Self::Target(v) => Some(v),
            Self::Off(_) | Self::Null(()) => None,
        }
    }
}

// Range constraints also apply to a referenced union: JSON Schema ignores numeric bounds
// for false and null. Schemars' numeric validation attributes only transform numeric schemas.
fn hold_range(schema: &mut schemars::Schema, min: f64, max: f64) {
    without_default(schema);
    schema.insert("minimum".into(), serde_json::json!(min));
    schema.insert("maximum".into(), serde_json::json!(max));
}

fn pitch_schema(schema: &mut schemars::Schema) {
    hold_range(schema, -0.25, 0.35);
}
fn altitude_schema(schema: &mut schemars::Schema) {
    hold_range(schema, 2.0, 10_000.0);
}
fn vertical_speed_schema(schema: &mut schemars::Schema) {
    hold_range(schema, -8.0, 8.0);
}
fn heading_schema(schema: &mut schemars::Schema) {
    hold_range(schema, 0.0, core::f64::consts::TAU);
}
fn airspeed_schema(schema: &mut schemars::Schema) {
    hold_range(schema, 25.0, 80.0);
}

/// Patch any subset of holds. Conflicting vertical or lateral holds must be disengaged in the
/// same call. Bounds: pitch -0.25..0.35 rad, altitude 2..10000 m, climb -8..8 m/s, heading 0..2π,
/// airspeed 25..80 m/s, and bank limit 0.05..0.6 rad.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAutopilot {
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "HoldTarget", transform = pitch_schema)]
    pub pitch_rad: Option<HoldTarget>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "HoldTarget", transform = altitude_schema)]
    pub altitude_m: Option<HoldTarget>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "HoldTarget", transform = vertical_speed_schema)]
    pub vertical_speed_mps: Option<HoldTarget>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "HoldTarget", transform = heading_schema)]
    pub heading_rad: Option<HoldTarget>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "HoldTarget", transform = airspeed_schema)]
    pub airspeed_mps: Option<HoldTarget>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "Option<bool>", transform = without_default)]
    pub wings_level: Option<Option<bool>>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "f64", range(min = 0.05, max = 0.6), transform = without_default)]
    pub bank_limit_rad: Option<f64>,
    #[serde(default, deserialize_with = "provided")]
    #[schemars(with = "AirspeedMode", transform = without_default)]
    pub airspeed_mode: Option<AirspeedMode>,
}

impl Command for SetAutopilot {
    const NAME: &'static str = "autopilot.set";
    const DESCRIPTION: &'static str =
        "Patch SI hold targets; null or false disengages a hold. Pitch airspeed mode uses full power.";
    type Output = Targets;
    fn run(self, session: &mut Session) -> Result<Targets, CommandError> {
        let mut t = session.sim.readout().autopilot;
        for (value, target) in [
            (self.pitch_rad, &mut t.pitch_rad),
            (self.altitude_m, &mut t.altitude_m),
            (self.vertical_speed_mps, &mut t.vertical_speed_mps),
            (self.heading_rad, &mut t.heading_rad),
            (self.airspeed_mps, &mut t.airspeed_mps),
        ] {
            if let Some(value) = value {
                *target = value.target();
            }
        }
        if let Some(value) = self.wings_level {
            t.wings_level = value.unwrap_or(false);
        }
        if let Some(value) = self.bank_limit_rad {
            t.bank_limit_rad = value;
        }
        if let Some(value) = self.airspeed_mode {
            t.airspeed_mode = value;
        }
        session
            .sim
            .set_autopilot(t)
            .map_err(|e| invalid(Self::NAME, e.to_string()))?;
        Ok(t)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutopilotOff {}

impl Command for AutopilotOff {
    const NAME: &'static str = "autopilot.off";
    const DESCRIPTION: &'static str = "Disengage all holds, leaving the last controls in place.";
    type Output = Targets;
    fn run(self, session: &mut Session) -> Result<Targets, CommandError> {
        session.sim.autopilot_off();
        Ok(session.sim.readout().autopilot)
    }
}

/// Run stops at the first matching condition in input order. Above/below comparisons are
/// inclusive. Elapsed time is relative to the start of this call, including when paused.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunCondition {
    AltitudeAbove {
        #[schemars(range(min = 0, max = 20000))]
        altitude_m: f64,
    },
    AltitudeBelow {
        #[schemars(range(min = 0, max = 20000))]
        altitude_m: f64,
    },
    AirspeedAbove {
        #[schemars(range(min = 0, max = 150))]
        airspeed_mps: f64,
    },
    AirspeedBelow {
        #[schemars(range(min = 0, max = 150))]
        airspeed_mps: f64,
    },
    HeadingWithin {
        #[schemars(range(min = 0, max = core::f64::consts::TAU))]
        heading_rad: f64,
        #[schemars(range(min = 0.0001, max = core::f64::consts::PI))]
        tolerance_rad: f64,
    },
    OnGround {},
    TimeElapsed {
        #[schemars(range(min = 0, max = 600))]
        seconds: f64,
    },
}

impl RunCondition {
    fn validate(&self) -> Result<(), CommandError> {
        match *self {
            Self::AltitudeAbove { altitude_m } | Self::AltitudeBelow { altitude_m } => {
                range(RunUntil::NAME, "altitude_m", altitude_m, 0.0, 20_000.0)
            }
            Self::AirspeedAbove { airspeed_mps } | Self::AirspeedBelow { airspeed_mps } => {
                range(RunUntil::NAME, "airspeed_mps", airspeed_mps, 0.0, 150.0)
            }
            Self::HeadingWithin {
                heading_rad,
                tolerance_rad,
            } => {
                range(
                    RunUntil::NAME,
                    "heading_rad",
                    heading_rad,
                    0.0,
                    core::f64::consts::TAU,
                )?;
                range(
                    RunUntil::NAME,
                    "tolerance_rad",
                    tolerance_rad,
                    0.0001,
                    core::f64::consts::PI,
                )
            }
            Self::OnGround {} => Ok(()),
            Self::TimeElapsed { seconds } => range(RunUntil::NAME, "seconds", seconds, 0.0, 600.0),
        }
    }

    fn matches(&self, r: &Readout, elapsed: f64) -> bool {
        match *self {
            Self::AltitudeAbove { altitude_m } => r.altitude_msl_m >= altitude_m,
            Self::AltitudeBelow { altitude_m } => r.altitude_msl_m <= altitude_m,
            Self::AirspeedAbove { airspeed_mps } => r.true_airspeed_mps >= airspeed_mps,
            Self::AirspeedBelow { airspeed_mps } => r.true_airspeed_mps <= airspeed_mps,
            Self::HeadingWithin {
                heading_rad,
                tolerance_rad,
            } => wrap(heading_rad - r.heading_rad).abs() <= tolerance_rad,
            Self::OnGround {} => r.on_ground,
            Self::TimeElapsed { seconds } => elapsed + 1e-9 >= seconds,
        }
    }
}

fn default_max_steps() -> u32 {
    MAX_STEPS
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunUntil {
    /// Stop when any condition matches; at most 16. An empty list runs to the step cap.
    #[schemars(length(max = 16))]
    pub conditions: Vec<RunCondition>,
    #[serde(default = "default_max_steps")]
    #[schemars(range(min = 0, max = MAX_STEPS))]
    pub max_steps: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunStop {
    Condition { index: usize, condition: RunCondition },
    StepCap,
    Crashed,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunOutput {
    pub readout: Readout,
    pub state: State,
    pub steps: u32,
    pub fired: RunStop,
}

impl Command for RunUntil {
    const NAME: &'static str = "sim.run_until";
    const DESCRIPTION: &'static str = "Step until any condition matches, a crash occurs, or the bounded step cap is reached; works while paused.";
    type Output = RunOutput;
    fn run(self, session: &mut Session) -> Result<RunOutput, CommandError> {
        if self.max_steps > MAX_STEPS || self.conditions.len() > 16 {
            return Err(invalid(
                Self::NAME,
                "at most 72000 steps and 16 conditions allowed",
            ));
        }
        for condition in &self.conditions {
            condition.validate()?;
        }
        let start_time = session.sim.state().time_s;
        let mut steps = 0;
        loop {
            let readout = session.sim.readout();
            let fired = if readout.crashed {
                Some(RunStop::Crashed)
            } else if let Some((index, condition)) = self
                .conditions
                .iter()
                .enumerate()
                .find(|(_, c)| c.matches(&readout, readout.time_s - start_time))
            {
                Some(RunStop::Condition {
                    index,
                    condition: condition.clone(),
                })
            } else if steps == self.max_steps {
                Some(RunStop::StepCap)
            } else {
                None
            };
            if let Some(fired) = fired {
                return Ok(RunOutput {
                    readout,
                    state: *session.sim.state(),
                    steps,
                    fired,
                });
            }
            session.sim.step(1);
            steps += 1;
        }
    }
}
