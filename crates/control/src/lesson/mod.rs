//! Instrument lessons share scenario actions and fixed-step measurements.
pub(crate) mod engine;
mod grading;
mod holding;
use crate::*;
use flightcraft_world::{Conditions, GeoPoint};
pub use grading::*;
pub use holding::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lesson {
    pub version: u8,
    pub id: String,
    pub title: String,
    pub briefing: String,
    pub setup: LessonSetup,
    pub scenario: Scenario,
    pub tasks: Vec<InstrumentTask>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonSetup {
    pub region: String,
    pub aircraft: String,
    pub airport: String,
    pub procedure: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_fix: Option<String>,
    pub mode: InstrumentApproach,
    /// Explicit, instructor-selected training minimum; CIFP does not encode plate minima.
    pub minimum_ft: f64,
    pub start_distance_nm: f64,
    #[serde(default)]
    pub cross_track_nm: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_altitude_ft: Option<f64>,
    pub airspeed_kt: f64,
    pub hood: bool,
    pub look_up: bool,
    pub conditions: Conditions,
    #[serde(default)]
    pub wind: flightcraft_world::Weather,
    #[serde(default)]
    pub failures: Vec<InstrumentFailure>,
    /// Busy-airspace traffic switched on at the start, with the user already in radar contact with
    /// the approach controller of `airport` on COM1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<LessonTraffic>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonTraffic {
    pub density: TrafficDensity,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentApproach {
    Ils,
    Lpv,
    Lnav,
    Localizer,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentTaskKind {
    Precision,
    Nonprecision,
    Holding,
    Missed,
    PartialPanel,
    UnusualAttitude,
    LookUp,
    /// Listening discipline on a busy frequency: answer traffic advisories, answer only calls that
    /// use your callsign, and do not transmit over others.
    Radio,
}
impl InstrumentTaskKind {
    pub fn acs(self) -> &'static str {
        match self {
            Self::Precision => "IR.VI.B",
            Self::Nonprecision => "IR.VI.A",
            Self::Holding => "IR.III.B",
            Self::Missed => "IR.VI.C",
            Self::PartialPanel => "IR.VII.D",
            Self::UnusualAttitude => "IR.IV.B",
            Self::LookUp => "IR.VI.E",
            Self::Radio => "IR.III.A",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstrumentTask {
    pub title: String,
    pub kind: InstrumentTaskKind,
    pub acs: String,
    pub tolerances: AcsTolerances,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentFailure {
    Vacuum,
    Ahrs,
    Display,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct LessonDebrief {
    pub title: Option<String>,
    pub briefing: String,
    pub complete: bool,
    pub started_s: f64,
    pub tasks: Vec<TaskGrade>,
    pub samples: Vec<InstrumentSample>,
    pub geometry: Vec<ProcedureGeometry>,
    pub hold: Option<HoldObservation>,
    pub notes: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProcedureGeometry {
    pub name: String,
    pub missed: bool,
    pub position: GeoPoint,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct LessonRuntime {
    pub document: Option<Lesson>,
    pub debrief: LessonDebrief,
    pub last_sample_s: Option<f64>,
    pub failure_due: Vec<(f64, InstrumentFailure)>,
    pub minimum_called: bool,
    pub missed_started_s: Option<f64>,
    pub recovery_started_s: Option<f64>,
    pub hold_monitor: holding::HoldMonitor,
}
impl Session {
    pub fn lesson_debrief(&self) -> &LessonDebrief {
        &self.lesson.debrief
    }
}
/// Original lesson documents, available identically in native and browser builds.
pub fn lesson_library() -> Result<Vec<Lesson>, CommandError> {
    [
        include_str!("../../../../lessons/kabe-ils-06.json"),
        include_str!("../../../../lessons/kabe-lpv-06.json"),
        include_str!("../../../../lessons/kabe-lnav-06.json"),
        include_str!("../../../../lessons/kmmu-lpv-05.json"),
        include_str!("../../../../lessons/kmmu-lnav-05.json"),
        include_str!("../../../../lessons/kabe-localizer-06.json"),
        include_str!("../../../../lessons/kabe-missed.json"),
        include_str!("../../../../lessons/kabe-localizer-missed.json"),
        include_str!("../../../../lessons/kabe-hold.json"),
        include_str!("../../../../lessons/corvid-vacuum.json"),
        include_str!("../../../../lessons/corvid-ahrs.json"),
        include_str!("../../../../lessons/corvid-display.json"),
        include_str!("../../../../lessons/unusual-attitude.json"),
        include_str!("../../../../lessons/busy-frequency.json"),
    ]
    .iter()
    .map(|text| serde_json::from_str(text).map_err(|e| invalid("lesson.list", e.to_string())))
    .collect()
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonList {}
impl Command for LessonList {
    const NAME: &'static str = "lesson.list";
    const DESCRIPTION: &'static str = "List offline IFR lessons, briefings, setups, and ACS task tolerances.";
    type Output = Vec<Lesson>;
    fn run(self, _: &mut Session) -> Result<Self::Output, CommandError> {
        lesson_library()
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonStart {
    pub lesson: Lesson,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetLessonDebrief {}
impl Command for GetLessonDebrief {
    const NAME: &'static str = "lesson.debrief";
    const DESCRIPTION: &'static str = "Read live instrument grades, measured deviations, and regional procedure geometry without advancing time.";
    type Output = LessonDebrief;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(s.lesson.debrief.clone())
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LessonFinish {}
impl Command for LessonFinish {
    const NAME: &'static str = "lesson.finish";
    const DESCRIPTION: &'static str =
        "Freeze the IFR debrief; tasks without sufficient measured evidence fail as incomplete.";
    type Output = LessonDebrief;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        s.lesson.finish();
        Ok(s.lesson.debrief.clone())
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstructorInstrumentFailure {
    pub failure: InstrumentFailure,
    pub earliest_s: f64,
    pub latest_s: f64,
    pub seed: u32,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstructorAttitude {
    pub pitch_deg: f64,
    pub bank_deg: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstrumentEvent {
    Failure { failure: InstrumentFailure },
    Attitude { pitch_deg: f64, bank_deg: f64 },
    Hood { enabled: bool },
}
