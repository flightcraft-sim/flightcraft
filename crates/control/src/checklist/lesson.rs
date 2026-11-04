//! Procedure lessons: ground lessons graded against an aircraft's own normal-procedure data.
//!
//! A lesson names an aircraft, a starting state, optionally a seeded fault, and one task per
//! checklist. The same fixed-step runtime that ticks the checklist grades the tasks, with the
//! POH reference of every item that was missed or misordered in the reasons.
use super::*;
use flightcraft_aero::checklists::{LaneId, PumpId};
use flightcraft_aero::panel_fidelity::BackupCircuit;
use flightcraft_sim::modern::{Component, ModernSetting};
use flightcraft_sim::systems::SystemSetting;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureLesson {
    pub version: u8,
    pub id: String,
    pub title: String,
    pub briefing: String,
    pub setup: ProcedureSetup,
    pub tasks: Vec<ProcedureTask>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureSetup {
    /// Installed aircraft pack id.
    pub aircraft: String,
    pub start: GroundStart,
    /// Oil temperature in degrees Celsius for an idling start; omitted means warm (70).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oil_temperature_c: Option<f64>,
    /// Seeded defect the student should find. At most one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<ProcedureFault>,
    #[serde(default = "default_temperature")]
    pub temperature_c: f64,
}
fn default_temperature() -> f64 {
    15.
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroundStart {
    /// Master off, ignition off, everything as left after securing.
    ColdAndDark,
    /// Engine started and idling, brakes on, at the run-up point.
    EngineIdling,
    /// Engine off, powered up as after a normal start of the day (shutdown lesson).
    EngineRunningAfterFlight,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcedureFault {
    /// The named ignition lane delivers weak sparks and drops excessive rpm when it runs alone.
    WeakLane { lane: LaneId },
    /// The named fuel pump has failed; the pump check shows no pressure.
    FailedPump { pump: PumpId },
    /// The backup battery holds only the given fraction of its charge.
    LowBackupBattery {
        circuit: BackupCircuit,
        #[schemars(range(min = 0, max = 1))]
        fraction: f64,
    },
    /// Oil temperature is below the warm-up limit when the run-up is due.
    ColdOil { temperature_c: f64 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskExpectation {
    /// Work the checklist through correctly.
    Complete,
    /// The seeded fault is the point: stopping is correct and continuing fails the task.
    StopOnFault,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureTask {
    pub title: String,
    pub checklist: String,
    pub poh_ref: String,
    pub expect: TaskExpectation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureResult {
    #[default]
    Pending,
    Pass,
    Fail,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProcedureTaskGrade {
    pub title: String,
    pub checklist: String,
    pub poh_ref: String,
    pub expect: TaskExpectation,
    pub result: ProcedureResult,
    pub reasons: Vec<String>,
    pub finished_s: Option<f64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProcedureDebrief {
    pub lesson: Option<String>,
    pub title: Option<String>,
    pub briefing: String,
    pub complete: bool,
    pub started_s: f64,
    pub tasks: Vec<ProcedureTaskGrade>,
    pub run_up: RunupReport,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct ProcedureRuntime {
    pub document: Option<ProcedureLesson>,
    pub task: usize,
    pub debrief: ProcedureDebrief,
}

/// Original lesson documents, available identically in native and browser builds.
pub fn procedure_lesson_library() -> Result<Vec<ProcedureLesson>, CommandError> {
    [
        include_str!("../../../../lessons/sling-before-start.json"),
        include_str!("../../../../lessons/sling-runup.json"),
        include_str!("../../../../lessons/sling-runup-weak-lane.json"),
        include_str!("../../../../lessons/sling-runup-failed-pump.json"),
        include_str!("../../../../lessons/sling-runup-low-backup.json"),
        include_str!("../../../../lessons/sling-runup-cold-oil.json"),
        include_str!("../../../../lessons/sling-shutdown.json"),
    ]
    .iter()
    .map(|text| serde_json::from_str(text).map_err(|e| invalid("procedure_lesson.list", e.to_string())))
    .collect()
}

fn validate(doc: &ProcedureLesson) -> Result<(), CommandError> {
    let bad = |m: &str| invalid(ProcedureLessonStart::NAME, m);
    let text = |s: &str, max| !s.trim().is_empty() && s.len() <= max;
    if doc.version != 1
        || !text(&doc.id, 96)
        || !text(&doc.title, 256)
        || doc.briefing.len() > 2048
        || doc.tasks.is_empty()
        || doc.tasks.len() > 16
        || !text(&doc.setup.aircraft, 96)
    {
        return Err(bad("invalid procedure lesson bounds"));
    }
    range(
        ProcedureLessonStart::NAME,
        "temperature_c",
        doc.setup.temperature_c,
        -40.,
        50.,
    )?;
    if let Some(t) = doc.setup.oil_temperature_c {
        range(ProcedureLessonStart::NAME, "oil_temperature_c", t, -40., 120.)?;
    }
    match doc.setup.fault {
        Some(ProcedureFault::LowBackupBattery { fraction, .. }) => {
            range(ProcedureLessonStart::NAME, "fraction", fraction, 0., 1.)?;
        }
        Some(ProcedureFault::ColdOil { temperature_c }) => {
            range(
                ProcedureLessonStart::NAME,
                "temperature_c",
                temperature_c,
                -40.,
                49.,
            )?;
        }
        _ => {}
    }
    let faulted = doc.setup.fault.is_some();
    for t in &doc.tasks {
        if !text(&t.title, 256) || !text(&t.checklist, 64) || !text(&t.poh_ref, 256) {
            return Err(bad("invalid task"));
        }
        if t.expect == TaskExpectation::StopOnFault && !faulted {
            return Err(bad("a stop-on-fault task needs a seeded fault"));
        }
    }
    Ok(())
}

impl Session {
    pub fn procedure_debrief(&self) -> &ProcedureDebrief {
        &self.procedure.debrief
    }

    /// Called each fixed step after the checklist has evaluated.
    pub(crate) fn sequence_procedure_lesson(&mut self) {
        let Some(doc) = self.procedure.document.clone() else {
            return;
        };
        if self.procedure.debrief.complete {
            return;
        }
        let Some(task) = doc.tasks.get(self.procedure.task) else {
            return;
        };
        let now = self.sim.state().time_s;
        let rt = &self.checklist;
        let continued = rt.continued_past_gate();
        let finished = rt.is_complete()
            || rt.aborted().is_some()
            || (task.expect == TaskExpectation::StopOnFault && continued);
        if !finished {
            return;
        }
        let Some(mut grade) = self.procedure.debrief.tasks.get(self.procedure.task).cloned() else {
            return;
        };
        grade.finished_s = Some(now);
        let mut reasons = Vec::new();
        let mut fail = false;
        let refs = |w: &ChecklistWarning| format!("{} ({})", w.text, w.poh_ref);
        for w in rt.warnings() {
            match w.kind {
                WarningKind::Limit | WarningKind::Timer | WarningKind::Sequence => {
                    fail = true;
                    reasons.push(refs(w));
                }
                WarningKind::Gate => {}
            }
        }
        match task.expect {
            TaskExpectation::Complete => {
                if let Some(a) = rt.aborted() {
                    fail = true;
                    reasons.push(format!(
                        "Stopped the procedure ({}), but nothing was wrong. ({})",
                        a.reason, task.poh_ref
                    ));
                }
            }
            TaskExpectation::StopOnFault => match rt.aborted() {
                Some(a) if !continued => {
                    reasons.push(format!(
                        "Found the defect and stopped at {}. ({})",
                        a.item.as_deref().unwrap_or("the checklist"),
                        task.poh_ref
                    ));
                }
                _ => {
                    fail = true;
                    reasons.push(format!(
                        "Carried on past a failed or unmet check; the aircraft was not fit to continue. ({})",
                        task.poh_ref
                    ));
                }
            },
        }
        grade.result = if fail {
            ProcedureResult::Fail
        } else {
            ProcedureResult::Pass
        };
        grade.reasons = reasons;
        self.procedure.debrief.run_up = rt.report().clone();
        if let Some(slot) = self.procedure.debrief.tasks.get_mut(self.procedure.task) {
            *slot = grade;
        }
        self.procedure.task += 1;
        if let Some(next) = doc.tasks.get(self.procedure.task) {
            let _ = self.execute("checklist.select", serde_json::json!({"id": next.checklist}));
        } else {
            self.procedure.debrief.complete = true;
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureLessonList {}
impl Command for ProcedureLessonList {
    const NAME: &'static str = "procedure_lesson.list";
    const DESCRIPTION: &'static str = "List offline ground-procedure lessons: before start, run-up with seeded faults, shutdown and securing.";
    type Output = Vec<ProcedureLesson>;
    fn run(self, _: &mut Session) -> Result<Self::Output, CommandError> {
        procedure_lesson_library()
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureLessonStart {
    /// A library lesson id.
    #[serde(default)]
    pub id: Option<String>,
    /// Or a complete lesson document.
    #[serde(default)]
    pub lesson: Option<ProcedureLesson>,
}
impl Command for ProcedureLessonStart {
    const NAME: &'static str = "procedure_lesson.start";
    const DESCRIPTION: &'static str = "Validate and start a ground-procedure lesson: loads the aircraft, sets the starting state and any seeded fault, and opens the first checklist.";
    type Output = ProcedureDebrief;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let doc = match (self.id, self.lesson) {
            (Some(id), None) => procedure_lesson_library()?
                .into_iter()
                .find(|l| l.id == id)
                .ok_or_else(|| invalid(Self::NAME, "unknown lesson id"))?,
            (None, Some(doc)) => doc,
            _ => return Err(invalid(Self::NAME, "supply exactly one of id or lesson")),
        };
        validate(&doc)?;
        let setup = &doc.setup;
        s.execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":setup.aircraft},"reset":true}),
        )?;
        for task in &doc.tasks {
            if !s
                .procedure_set()
                .is_some_and(|p| p.checklists.iter().any(|c| c.id == task.checklist))
            {
                return Err(invalid(
                    Self::NAME,
                    format!("aircraft has no checklist {}", task.checklist),
                ));
            }
        }
        s.execute("sim.reset", serde_json::json!({"start":{"kind":"runway"}}))?;
        s.sim_mut().set_systems(SystemSetting::Environment {
            temperature_c: setup.temperature_c,
            relative_humidity: 0.5,
        })?;
        s.prepare_ground_state(setup)?;
        s.procedure = ProcedureRuntime {
            debrief: ProcedureDebrief {
                lesson: Some(doc.id.clone()),
                title: Some(doc.title.clone()),
                briefing: doc.briefing.clone(),
                complete: false,
                started_s: s.sim.state().time_s,
                tasks: doc
                    .tasks
                    .iter()
                    .map(|t| ProcedureTaskGrade {
                        title: t.title.clone(),
                        checklist: t.checklist.clone(),
                        poh_ref: t.poh_ref.clone(),
                        expect: t.expect,
                        result: ProcedureResult::Pending,
                        reasons: Vec::new(),
                        finished_s: None,
                    })
                    .collect(),
                run_up: RunupReport::default(),
            },
            task: 0,
            document: Some(doc.clone()),
        };
        s.execute(
            "checklist.select",
            serde_json::json!({"id": doc.tasks.first().map(|t| t.checklist.clone())}),
        )?;
        Ok(s.procedure.debrief.clone())
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureLessonDebrief {}
impl Command for ProcedureLessonDebrief {
    const NAME: &'static str = "procedure_lesson.debrief";
    const DESCRIPTION: &'static str =
        "Read the live grades of the running ground-procedure lesson, with POH references for every miss.";
    type Output = ProcedureDebrief;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let mut d = s.procedure.debrief.clone();
        d.run_up = s.checklist.report().clone();
        Ok(d)
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureLessonFinish {}
impl Command for ProcedureLessonFinish {
    const NAME: &'static str = "procedure_lesson.finish";
    const DESCRIPTION: &'static str = "End the lesson; tasks not finished fail as incomplete.";
    type Output = ProcedureDebrief;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let run_up = s.checklist.report().clone();
        let open = s.procedure.task;
        for (i, t) in s.procedure.debrief.tasks.iter_mut().enumerate() {
            if i >= open && t.result == ProcedureResult::Pending {
                t.result = ProcedureResult::Fail;
                t.reasons.push(format!("Not finished. ({})", t.poh_ref));
            }
        }
        s.procedure.debrief.complete = true;
        s.procedure.debrief.run_up = run_up;
        Ok(s.procedure.debrief.clone())
    }
}

impl Session {
    /// Brings the aircraft to the lesson's starting state and seeds its fault.
    fn prepare_ground_state(&mut self, setup: &ProcedureSetup) -> Result<(), CommandError> {
        let lanes = self.sim.systems().modern.as_ref().is_some_and(|m| m.electronic);
        match setup.start {
            GroundStart::ColdAndDark => {
                self.set_ground_switches(false)?;
            }
            GroundStart::EngineIdling | GroundStart::EngineRunningAfterFlight => {
                self.start_engine_for_setup(lanes)?;
                let warm = setup.oil_temperature_c.unwrap_or(70.);
                self.sim_mut()
                    .set_systems(SystemSetting::OilTemperature { value_c: warm })?;
            }
        }
        match setup.fault {
            Some(ProcedureFault::WeakLane { lane }) => {
                self.sim_mut().set_systems(SystemSetting::Equipment {
                    setting: ModernSetting::Failure {
                        component: if lane == LaneId::A {
                            Component::LaneAWeak
                        } else {
                            Component::LaneBWeak
                        },
                        failed: true,
                    },
                })?;
            }
            Some(ProcedureFault::FailedPump { pump }) => {
                self.sim_mut().set_systems(SystemSetting::Equipment {
                    setting: ModernSetting::Failure {
                        component: if pump == PumpId::One {
                            Component::Pump1
                        } else {
                            Component::Pump2
                        },
                        failed: true,
                    },
                })?;
            }
            Some(ProcedureFault::LowBackupBattery { circuit, fraction }) => {
                let capacity = self
                    .active_configuration
                    .as_ref()
                    .and_then(|c| c.installed_systems.definition.as_ref())
                    .and_then(|d| d.modern.as_ref())
                    .and_then(|m| m.backup_buses.iter().find(|b| b.circuit == circuit))
                    .map(|b| b.capacity_ah)
                    .ok_or_else(|| {
                        invalid(ProcedureLessonStart::NAME, "aircraft has no such backup battery")
                    })?;
                self.sim_mut().set_systems(SystemSetting::Equipment {
                    setting: ModernSetting::BackupCharge {
                        circuit,
                        remaining_ah: capacity * fraction,
                    },
                })?;
            }
            Some(ProcedureFault::ColdOil { temperature_c }) => {
                self.sim_mut().set_systems(SystemSetting::OilTemperature {
                    value_c: temperature_c,
                })?;
            }
            None => {}
        }
        // Let seeded state reach the instruments before the first checklist evaluation.
        self.sim_mut().step(60);
        Ok(())
    }

    fn set_ground_switches(&mut self, on: bool) -> Result<(), CommandError> {
        use flightcraft_sim::systems::SystemSwitch;
        self.sim_mut().set_systems(SystemSetting::Switch {
            switch: SystemSwitch::Master,
            on,
        })?;
        self.sim_mut().set_systems(SystemSetting::Switch {
            switch: SystemSwitch::Avionics,
            on,
        })?;
        if self.sim.systems().modern.as_ref().is_some_and(|m| m.electronic) {
            use flightcraft_sim::modern::{KeyPosition, Lane, Pump};
            for setting in [
                ModernSetting::Lane { lane: Lane::A, on },
                ModernSetting::Lane { lane: Lane::B, on },
                ModernSetting::Pump { pump: Pump::One, on },
                ModernSetting::Pump { pump: Pump::Two, on },
                ModernSetting::Key {
                    position: if on { KeyPosition::On } else { KeyPosition::Off },
                },
            ] {
                self.sim_mut().set_systems(SystemSetting::Equipment { setting })?;
            }
        } else {
            self.sim_mut().set_systems(SystemSetting::Magnetos {
                position: if on {
                    flightcraft_sim::systems::Magnetos::Both
                } else {
                    flightcraft_sim::systems::Magnetos::Off
                },
            })?;
        }
        Ok(())
    }

    /// Cranks and settles the engine at idle with the brakes on; deterministic fixed steps.
    fn start_engine_for_setup(&mut self, electronic: bool) -> Result<(), CommandError> {
        use flightcraft_sim::modern::KeyPosition;
        use flightcraft_sim::systems::{FuelSelector, SystemSwitch};
        self.set_ground_switches(true)?;
        if electronic {
            use flightcraft_aero::panel_fidelity::BackupCircuit;
            for (circuit, on) in [(BackupCircuit::Efis, true), (BackupCircuit::Ems, false)] {
                let _ = self.sim_mut().set_systems(SystemSetting::Equipment {
                    setting: ModernSetting::BackupPower { circuit, on },
                });
            }
        }
        self.sim_mut()
            .set_systems(SystemSetting::FuelSelector {
                position: FuelSelector::Both,
            })
            .or_else(|_| {
                self.sim_mut().set_systems(SystemSetting::FuelSelector {
                    position: FuelSelector::Left,
                })
            })?;
        let mut c = self.sim.controls();
        c.brake = 1.;
        c.throttle = 0.05;
        self.sim_mut().set_controls(c);
        if electronic {
            self.sim_mut().set_systems(SystemSetting::Equipment {
                setting: ModernSetting::Key {
                    position: KeyPosition::Start,
                },
            })?;
        } else {
            self.sim_mut().set_systems(SystemSetting::Switch {
                switch: SystemSwitch::Starter,
                on: true,
            })?;
        }
        for _ in 0..(10 * 120) {
            self.sim_mut().step(1);
            if self.sim.systems().engine_running {
                break;
            }
        }
        if electronic {
            self.sim_mut().set_systems(SystemSetting::Equipment {
                setting: ModernSetting::Key {
                    position: KeyPosition::On,
                },
            })?;
        } else {
            self.sim_mut().set_systems(SystemSetting::Switch {
                switch: SystemSwitch::Starter,
                on: false,
            })?;
        }
        self.sim_mut().step(5 * 120);
        if !self.sim.systems().engine_running {
            return Err(invalid(
                ProcedureLessonStart::NAME,
                "engine did not start for the lesson",
            ));
        }
        Ok(())
    }
}
