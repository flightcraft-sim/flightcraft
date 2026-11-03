use super::*;
/// FAA-S-ACS-8 task tolerances. Timing/leg margins are instructor criteria, not ACS quotations.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcsTolerances {
    pub altitude_ft: f64,
    pub heading_deg: f64,
    pub airspeed_kt: f64,
    pub cdi_scale: f64,
}
impl Default for AcsTolerances {
    fn default() -> Self {
        Self {
            altitude_ft: 100.,
            heading_deg: 10.,
            airspeed_kt: 10.,
            cdi_scale: 0.75,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskResult {
    #[default]
    Pending,
    Pass,
    Fail,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TaskGrade {
    pub title: String,
    pub kind: InstrumentTaskKind,
    pub acs: String,
    pub tolerances: AcsTolerances,
    pub result: TaskResult,
    pub reasons: Vec<String>,
    pub observed_s: f64,
    pub completed: bool,
}
impl TaskGrade {
    pub(super) fn fail(&mut self, reason: &str) {
        self.result = TaskResult::Fail;
        if self.reasons.len() < 32 && !self.reasons.iter().any(|r| r == reason) {
            self.reasons.push(reason.into());
        }
    }
    pub(super) fn done(&mut self) {
        self.completed = true;
        if self.result != TaskResult::Fail {
            self.result = TaskResult::Pass;
        }
    }
    /// Pure measured-sample evaluator, shared by live grading and synthetic-flight tests.
    pub fn observe(&mut self, s: &InstrumentSample, minimum_ft: f64, precision: bool) {
        if !s.grading {
            return;
        }
        self.observed_s += s.interval_s;
        let t = self.tolerances;
        if !s.signal_valid {
            self.fail("Navigation guidance lost during the graded segment.");
        }
        if s.cdi_scale.abs() > t.cdi_scale {
            self.fail("Lateral deviation exceeded ¾-scale CDI.");
        }
        if precision && s.vertical_scale.is_none_or(|v| v.abs() > t.cdi_scale) {
            self.fail("Vertical deviation exceeded ¾-scale glidepath indication or guidance was lost.");
        }
        if s.airspeed_error_kt.abs() > t.airspeed_kt {
            self.fail("Airspeed exceeded ±10 kt.");
        }
        if !precision && s.altitude_ft < minimum_ft - 0.5 && !s.visual {
            self.fail("Descended below MDA without visual reference.");
        }
        if s.level_segment && s.altitude_error_ft.abs() > t.altitude_ft {
            self.fail("Altitude exceeded ±100 ft.");
        }
        if s.at_minimum && !s.visual && !s.missed {
            self.fail("Continued below minimums without visual reference or a missed approach.");
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct InstrumentSample {
    pub time_s: f64,
    pub interval_s: f64,
    pub position: Option<GeoPoint>,
    pub altitude_ft: f64,
    pub altitude_error_ft: f64,
    pub airspeed_error_kt: f64,
    pub heading_error_deg: f64,
    /// Signed fraction of full-scale indication; independent of receiver dot count.
    pub cdi_scale: f64,
    pub vertical_scale: Option<f64>,
    pub signal_valid: bool,
    pub grading: bool,
    pub level_segment: bool,
    pub at_minimum: bool,
    pub visual: bool,
    pub missed: bool,
    pub hood: bool,
}
impl LessonRuntime {
    pub(super) fn finish(&mut self) {
        if self.debrief.complete {
            return;
        }
        for grade in &mut self.debrief.tasks {
            if !grade.completed {
                grade.fail("Incomplete: required task outcome was not observed.");
            }
        }
        self.debrief.complete = true;
    }
}
impl LessonDebrief {
    /// Archive/display data is bounded and observational; importing it never starts a lesson.
    pub fn validate(&self) -> Result<(), CommandError> {
        let bad = || invalid("lesson.debrief", "invalid or oversized instrument debrief");
        if self.title.as_ref().is_some_and(|t| t.len() > 256)
            || self.briefing.len() > 8192
            || self.tasks.len() > 16
            || self.samples.len() > 14400
            || self.geometry.len() > 256
            || self.notes.len() > 64
            || !self.started_s.is_finite()
        {
            return Err(bad());
        }
        let mut previous = self.started_s;
        for s in &self.samples {
            if !s.time_s.is_finite()
                || s.time_s < previous
                || !s.interval_s.is_finite()
                || !(0. ..=60.).contains(&s.interval_s)
                || [
                    s.altitude_ft,
                    s.altitude_error_ft,
                    s.airspeed_error_kt,
                    s.heading_error_deg,
                    s.cdi_scale,
                ]
                .iter()
                .any(|n| !n.is_finite())
                || s.vertical_scale.is_some_and(|n| !n.is_finite())
            {
                return Err(bad());
            }
            if let Some(p) = s.position {
                p.validate().map_err(|_| bad())?;
            }
            previous = s.time_s;
        }
        for g in &self.geometry {
            if g.name.len() > 64 {
                return Err(bad());
            }
            g.position.validate().map_err(|_| bad())?;
        }
        for t in &self.tasks {
            if t.title.len() > 256
                || t.acs != t.kind.acs()
                || t.reasons.len() > 32
                || t.reasons.iter().any(|r| r.len() > 1024)
                || !t.observed_s.is_finite()
                || t.observed_s < 0.
            {
                return Err(bad());
            }
        }
        Ok(())
    }
}
