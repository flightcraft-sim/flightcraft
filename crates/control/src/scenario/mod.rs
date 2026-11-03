//! Bounded, deterministic scenario scripts; no controller policy lives here.
pub mod emergency;
mod engine;
mod landing_sites;
mod radio;
mod validate;
use crate::*;
pub use emergency::*;
use flightcraft_world::{FacilityKind, GeoPoint};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub use validate::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub version: u8,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub ai_written: bool,
    pub seed: u64,
    /// Route samples carry explicit MSL altitude and clearance assumptions.
    pub route: Vec<RoutePoint>,
    #[serde(default)]
    pub procedures: Vec<String>,
    pub steps: Vec<ScenarioStep>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutePoint {
    pub reference: String,
    pub altitude_msl_m: f64,
    /// Airspace ids for which this planned flight has clearance.
    #[serde(default)]
    pub clearances: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStep {
    pub id: String,
    /// All conditions must hold. Steps execute in file order unless a branch jumps forward.
    pub when: Vec<Trigger>,
    pub actions: Vec<ScenarioAction>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Trigger {
    Distance {
        reference: String,
        within_nm: f64,
    },
    DepartingRunway {
        airport: String,
        runway: String,
        within_nm: f64,
    },
    OnFinal {
        airport: String,
        runway: String,
        within_nm: f64,
    },
    CrossingRadial {
        reference: String,
        radial_deg: f64,
    },
    EnteringAirspace {
        id: String,
    },
    HeightAgl {
        min_m: f64,
        max_m: f64,
    },
    Airspeed {
        min_mps: f64,
        max_mps: f64,
    },
    Phase {
        phase: FlightPhase,
    },
    Configuration {
        min_flaps: f64,
        max_flaps: f64,
    },
    TunedFrequency {
        channel_khz: u32,
    },
    PilotCall {
        contains: String,
    },
    Readback {
        contains: String,
        result: ReadbackResult,
        timeout_s: f64,
    },
    Checklist {
        item: String,
    },
    Time {
        seconds: f64,
        #[serde(default)]
        since_previous: bool,
    },
    RandomWindow {
        earliest_s: f64,
        latest_s: f64,
    },
    Event {
        name: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlightPhase {
    Taxi,
    Takeoff,
    Climb,
    Cruise,
    Descent,
    Approach,
    Landed,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadbackResult {
    Correct,
    Incorrect,
    Missing,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScenarioAction {
    Transmission {
        facility: String,
        channel_khz: u32,
        role: FacilityKind,
        instruction: Instruction,
    },
    FailureSet {
        failure: Failure,
    },
    FailureClear {
        target: FailureTarget,
    },
    Weather {
        conditions: flightcraft_world::Conditions,
    },
    TrafficSpawn {
        airport: String,
        runway: String,
        count: u8,
    },
    Cue {
        text: String,
    },
    /// A wait installs a condition on advancement to the next step.
    Wait {
        condition: Trigger,
    },
    Branch {
        condition: Trigger,
        then_step: String,
        else_step: String,
    },
    EmitEvent {
        name: String,
    },
}
/// Typed meaning shared by scripted and live facility transmissions. No automatic ATC rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Instruction {
    Land { airport: String, runway: String },
    GoAround { airport: String, runway: String },
    Altitude { altitude_msl_m: f64, at: String },
    AirspaceClearance { id: String },
    RequestReadback { text: String },
    Advisory { text: String },
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ScenarioEvent {
    pub time_s: f64,
    pub step: String,
    pub source: EventSource,
    pub action: ScenarioAction,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct ScenarioState {
    pub id: Option<String>,
    pub step: usize,
    pub complete: bool,
    pub waiting: bool,
    pub events: Vec<ScenarioEvent>,
    pub dropped_events: u64,
    pub emergency: Option<EmergencyDebrief>,
}
#[derive(Debug, Default)]
pub(crate) struct ScenarioRuntime {
    pub document: Option<Scenario>,
    pub state: ScenarioState,
    pub started_s: f64,
    pub previous_s: f64,
    pub wait: Option<Trigger>,
    pub event_cursor: u64,
    pub previous_position: Option<GeoPoint>,
    pub previous_airspaces: Vec<String>,
    pub signals: std::collections::BTreeSet<String>,
    pub granted_clearances: std::collections::BTreeSet<String>,
    pub checklist: std::collections::BTreeSet<String>,
    pub generation: u64,
    pub scheduled: Vec<(f64, Failure)>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioValidate {
    pub scenario: Scenario,
}
impl Command for ScenarioValidate {
    const NAME: &'static str = "scenario.validate";
    const DESCRIPTION: &'static str =
        "Validate a bounded scenario against the active region and route; return all errors.";
    type Output = ScenarioValidation;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(validate(&self.scenario, s))
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStart {
    pub scenario: Scenario,
}
impl Command for ScenarioStart {
    const NAME: &'static str = "scenario.start";
    const DESCRIPTION: &'static str =
        "Validate and start a scenario on the current flight without changing its start position.";
    type Output = ScenarioState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        ensure_valid(&self.scenario, s)?;
        s.scenario = ScenarioRuntime {
            state: ScenarioState {
                id: Some(self.scenario.id.clone()),
                ..Default::default()
            },
            document: Some(self.scenario),
            started_s: s.sim.state().time_s,
            previous_s: s.sim.state().time_s,
            generation: s.flight_generation,
            event_cursor: s.flight_record().events.last().map_or(0, |e| e.id),
            ..Default::default()
        };
        s.sequence_scenario();
        Ok(s.scenario.state.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioAmend {
    pub scenario: Scenario,
}
impl Command for ScenarioAmend {
    const NAME: &'static str = "scenario.amend";
    const DESCRIPTION: &'static str =
        "Atomically replace the unexecuted suffix of an active scenario after validating the whole document.";
    type Output = ScenarioState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        ensure_valid(&self.scenario, s)?;
        let old = s
            .scenario
            .document
            .as_ref()
            .ok_or_else(|| invalid(Self::NAME, "no active scenario"))?;
        let prefix = s.scenario.state.step;
        if old.id != self.scenario.id
            || old.seed != self.scenario.seed
            || old.ai_written != self.scenario.ai_written
            || serde_json::to_value(old.steps.iter().take(prefix).collect::<Vec<_>>()).ok()
                != serde_json::to_value(self.scenario.steps.iter().take(prefix).collect::<Vec<_>>()).ok()
        {
            return Err(invalid(
                Self::NAME,
                "id, seed, authorship, and executed steps cannot change",
            ));
        }
        s.scenario.document = Some(self.scenario);
        s.scenario.state.complete = s
            .scenario
            .document
            .as_ref()
            .is_some_and(|d| s.scenario.state.step >= d.steps.len())
            && s.scenario.wait.is_none();
        Ok(s.scenario.state.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetScenarioState {}
impl Command for GetScenarioState {
    const NAME: &'static str = "scenario.state";
    const DESCRIPTION: &'static str =
        "Read scenario progress, timestamped actions, and the emergency debrief.";
    type Output = ScenarioState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(s.scenario.state.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSignal {
    pub name: String,
}
impl Command for ScenarioSignal {
    const NAME: &'static str = "scenario.signal";
    const DESCRIPTION: &'static str =
        "Supply a named flight event for scenario triggers, including future controller events.";
    type Output = ScenarioState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if !text_ok(&self.name) || s.scenario.signals.len() >= 512 {
            return Err(invalid(Self::NAME, "invalid or too many signals"));
        }
        s.scenario.signals.insert(self.name);
        s.sequence_scenario();
        Ok(s.scenario.state.clone())
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommsAtc {
    pub facility: String,
    pub channel_khz: u32,
    pub role: FacilityKind,
    pub instruction: Instruction,
}
impl Command for CommsAtc {
    const NAME: &'static str = "comms.atc";
    const DESCRIPTION: &'static str =
        "Send one validated facility transmission tagged ai and visibly marked simulated ATC.";
    type Output = Transmission;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let errors = validate_transmission(
            s,
            &self.facility,
            self.channel_khz,
            self.role,
            &self.instruction,
            &s.scenario.granted_clearances,
        );
        if !errors.is_empty() {
            return Err(invalid(Self::NAME, errors.join("; ")));
        }
        s.scenario_transmission(
            &self.facility,
            self.channel_khz,
            self.role,
            self.instruction,
            EventSource::Ai,
        )
    }
}
fn ensure_valid(doc: &Scenario, s: &Session) -> Result<(), CommandError> {
    let report = validate(doc, s);
    if report.valid {
        Ok(())
    } else {
        Err(invalid("scenario", report.errors.join("; ")))
    }
}
fn text_ok(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control)
}
/// Content files embedded for offline desktop, browser, and MCP use.
pub fn library() -> Result<Vec<Scenario>, CommandError> {
    [
        include_str!("../../../../scenarios/engine-takeoff.json"),
        include_str!("../../../../scenarios/engine-cruise.json"),
        include_str!("../../../../scenarios/electrical-fire.json"),
        include_str!("../../../../scenarios/vacuum-imc.json"),
        include_str!("../../../../scenarios/door-takeoff.json"),
        include_str!("../../../../scenarios/three-mile-final.json"),
        include_str!("../../../../scenarios/carb-ice.json"),
    ]
    .iter()
    .map(|text| serde_json::from_str(text).map_err(|e| invalid("scenario.library", e.to_string())))
    .collect()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstructorFailure {
    pub failure: Failure,
    pub within_seconds: f64,
    pub seed: u64,
}
impl Command for InstructorFailure {
    const NAME: &'static str = "scenario.instructor_failure";
    const DESCRIPTION: &'static str =
        "Trigger a failure now or at a deterministic seeded time within the next N seconds.";
    type Output = ScenarioState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        crate::range(Self::NAME, "within_seconds", self.within_seconds, 0.0, 3600.0)?;
        match self.failure {
            Failure::PartialPower { fraction } => crate::range(Self::NAME, "fraction", fraction, 0.0, 1.0)?,
            Failure::Jam { surface, position } => {
                let mut f = flightcraft_sim::Failures::default();
                f.jam(surface, position)
                    .map_err(flightcraft_sim::SimError::from)?;
            }
            _ => (),
        }
        if s.scenario.scheduled.len() >= 64 {
            return Err(invalid(Self::NAME, "at most 64 scheduled failures"));
        }
        let value = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let unit = (value >> 11) as f64 / 9007199254740992.;
        s.scenario
            .scheduled
            .push((s.sim.state().time_s + self.within_seconds * unit, self.failure));
        s.sequence_scenario();
        Ok(s.scenario.state.clone())
    }
}

pub(super) fn contains_phrase(text: &str, phrase: &str) -> bool {
    let normalize = |t: &str| {
        t.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(" {} ", normalize(text)).contains(&format!(" {} ", normalize(phrase)))
}
