//! Normal-procedure checklists: live verification against simulator state, POH timers, limit
//! warnings, run-up grading, and the "show me" demonstration by the virtual pilot.
//!
//! The lists are pack data (`flightcraft_aero::checklists`). Nothing here changes the flight
//! except `checklist.show_me`, which performs one item the way a pilot would.
mod eval;
mod lesson;
mod runup;
mod show;
pub use lesson::*;
pub use runup::{AxisExtent, ControlsMeasure, LaneMeasure, MaxRpmMeasure, PumpMeasure, RunupReport};

use crate::*;
use eval::{Probe, eval, failed_check};
use flightcraft_aero::checklists::{Checklist, ChecklistPhase, TimerKind, Verify};
use runup::RunupObserver;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whether a binding records a deliberate pilot action (a switch, a performed check). Readings that
/// pass through a range on the way elsewhere (rpm, voltage, temperature) are never out of sequence.
fn is_action(v: &Verify) -> bool {
    match v {
        Verify::Rpm { .. }
        | Verify::OilTemp { .. }
        | Verify::OilPressure { .. }
        | Verify::FuelPressure { .. }
        | Verify::BusVoltage { .. }
        | Verify::Throttle { .. }
        | Verify::Flaps { .. }
        | Verify::EngineRunning { .. }
        | Verify::Starter { .. } => false,
        Verify::All { of } => of.iter().all(is_action),
        _ => true,
    }
}

/// Simulated seconds between evaluations.
const EVAL_INTERVAL_S: f64 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Pending,
    Current,
    Done,
    /// A recorded measurement is outside its limit.
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoneBy {
    /// The simulator state satisfied the item's `verify` binding.
    Verified,
    /// Acknowledged by the pilot (items with no binding).
    Manual,
    /// Acknowledged although the binding was not satisfied.
    Forced,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// An action was performed ahead of the current item.
    Sequence,
    /// A checklist limit was exceeded.
    Limit,
    /// A POH timer ran out.
    Timer,
    /// The pilot went on past an unmet or failed go/no-go item.
    Gate,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChecklistWarning {
    pub time_s: f64,
    pub kind: WarningKind,
    pub item: Option<String>,
    pub text: String,
    pub poh_ref: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TimerView {
    pub label: String,
    pub kind: TimerKind,
    pub seconds: f64,
    pub elapsed_s: f64,
    pub running: bool,
    pub expired: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ItemView {
    pub id: String,
    pub text: String,
    pub response: String,
    pub poh_ref: String,
    pub status: ItemStatus,
    pub done_by: Option<DoneBy>,
    /// Whether the simulator can verify this item; otherwise the pilot ticks it.
    pub verifiable: bool,
    pub control: Option<String>,
    pub note: Option<String>,
    pub unverified: Option<String>,
    pub gate: bool,
    pub timer: Option<TimerView>,
    /// A recorded measurement, for example the lane rpm drop.
    pub detail: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChecklistSummary {
    pub id: String,
    pub title: String,
    pub phase: ChecklistPhase,
    pub poh_ref: String,
    pub items: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AbortRecord {
    pub time_s: f64,
    pub item: Option<String>,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ChecklistView {
    /// Lists the aircraft carries; empty when it has no normal-procedure data.
    pub available: Vec<ChecklistSummary>,
    pub active: Option<String>,
    pub title: Option<String>,
    pub phase: Option<ChecklistPhase>,
    pub poh_ref: Option<String>,
    pub current: Option<usize>,
    pub items: Vec<ItemView>,
    pub complete: bool,
    pub warnings: Vec<ChecklistWarning>,
    /// Limit warnings whose condition holds right now.
    pub active_limits: Vec<String>,
    pub aborted: Option<AbortRecord>,
    /// The pilot went on past an unmet or failed go/no-go item.
    pub continued_past_gate: bool,
    pub runup: RunupReport,
    /// The item the virtual pilot is demonstrating, if any.
    pub showing: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct TimerRuntime {
    elapsed_s: f64,
    running: bool,
    expired: bool,
}
#[derive(Clone, Debug, Default)]
struct ItemRuntime {
    done: Option<(DoneBy, f64)>,
    timer: TimerRuntime,
    /// `Verify` held at the previous evaluation, to spot an action as it happens.
    was_true: bool,
    out_of_order_warned: bool,
    failed_detail: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct ChecklistRuntime {
    active: Option<String>,
    items: Vec<ItemRuntime>,
    limit_was: Vec<bool>,
    warnings: Vec<ChecklistWarning>,
    active_limits: Vec<String>,
    observer: RunupObserver,
    last_eval_s: Option<f64>,
    aborted: Option<AbortRecord>,
    continued_past_gate: bool,
    gate_warned: bool,
    show: Option<show::ShowMe>,
    completed_at_s: Option<f64>,
}

impl Session {
    fn procedure_set(&self) -> Option<&flightcraft_aero::checklists::ProcedureSet> {
        self.active_configuration
            .as_ref()
            .and_then(|c| c.installed_systems.procedures.as_ref())
    }
    fn active_checklist(&self) -> Option<Checklist> {
        let id = self.checklist.active.as_ref()?;
        self.procedure_set()?
            .checklists
            .iter()
            .find(|c| &c.id == id)
            .cloned()
    }

    /// Called every fixed step; evaluates at 10 Hz of simulated time.
    pub(crate) fn sequence_checklist(&mut self) {
        if self.checklist.active.is_none() {
            return;
        }
        let now = self.sim.state().time_s;
        if self
            .checklist
            .last_eval_s
            .is_some_and(|t| now - t < EVAL_INTERVAL_S - 1e-9 && now >= t)
        {
            return;
        }
        let Some(list) = self.active_checklist() else {
            self.checklist = ChecklistRuntime::default();
            return;
        };
        let mut rt = std::mem::take(&mut self.checklist);
        let dt = rt.last_eval_s.map_or(0., |t| (now - t).max(0.));
        rt.last_eval_s = Some(now);
        let probe = Probe::read(self);
        rt.step(&list, &probe, dt);
        if let Some(show) = rt.show.take() {
            rt.show = show.tick(self, &probe, &rt.observer);
        }
        self.checklist = rt;
    }

    pub(crate) fn checklist_view(&self) -> ChecklistView {
        let mut view = ChecklistView {
            available: self
                .procedure_set()
                .map(|p| {
                    p.checklists
                        .iter()
                        .map(|c| ChecklistSummary {
                            id: c.id.clone(),
                            title: c.title.clone(),
                            phase: c.phase,
                            poh_ref: c.poh_ref.clone(),
                            items: c.items.len(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ..Default::default()
        };
        let Some(list) = self.active_checklist() else {
            return view;
        };
        let rt = &self.checklist;
        let probe = Probe::read(self);
        let current = rt.current_index();
        view.active = Some(list.id.clone());
        view.title = Some(list.title.clone());
        view.phase = Some(list.phase);
        view.poh_ref = Some(list.poh_ref.clone());
        view.current = current;
        view.complete = current.is_none();
        view.warnings = rt.warnings.clone();
        view.active_limits = rt.active_limits.clone();
        view.aborted = rt.aborted.clone();
        view.continued_past_gate = rt.continued_past_gate;
        view.runup = rt.observer.report.clone();
        view.showing = rt.show.as_ref().map(|s| s.item.clone());
        view.items = list
            .items
            .iter()
            .zip(&rt.items)
            .enumerate()
            .map(|(i, (item, st))| {
                let failed = item
                    .verify
                    .as_ref()
                    .and_then(|v| failed_check(v, &probe, &rt.observer))
                    .or_else(|| st.failed_detail.clone());
                let status = if st.done.is_some() {
                    ItemStatus::Done
                } else if failed.is_some() {
                    ItemStatus::Failed
                } else if Some(i) == current {
                    ItemStatus::Current
                } else {
                    ItemStatus::Pending
                };
                let detail = item.verify.as_ref().and_then(|v| match v {
                    Verify::Check(c) => rt.observer.describe(c),
                    _ => None,
                });
                ItemView {
                    id: item.id.clone(),
                    text: item.text.clone(),
                    response: item.response.clone(),
                    poh_ref: item.poh_ref.clone(),
                    status,
                    done_by: st.done.map(|d| d.0),
                    verifiable: item.verify.is_some(),
                    control: item.control.clone(),
                    note: item.note.clone(),
                    unverified: item.unverified.clone(),
                    gate: item.stop_on_failure,
                    timer: item.timer.as_ref().map(|t| TimerView {
                        label: t.label.clone(),
                        kind: t.kind,
                        seconds: t.seconds,
                        elapsed_s: st.timer.elapsed_s.min(t.seconds * 4.),
                        running: st.timer.running,
                        expired: st.timer.expired,
                    }),
                    detail,
                }
            })
            .collect();
        view
    }
}

impl ChecklistRuntime {
    fn new(list: &Checklist) -> Self {
        Self {
            active: Some(list.id.clone()),
            items: vec![ItemRuntime::default(); list.items.len()],
            limit_was: vec![false; list.limits.len()],
            ..Default::default()
        }
    }
    pub(crate) fn current_index(&self) -> Option<usize> {
        self.items.iter().position(|i| i.done.is_none())
    }
    pub(crate) fn is_open(&self) -> bool {
        self.active.is_some()
    }
    pub(crate) fn is_complete(&self) -> bool {
        self.active.is_some() && self.current_index().is_none()
    }
    pub(crate) fn aborted(&self) -> Option<&AbortRecord> {
        self.aborted.as_ref()
    }
    pub(crate) fn continued_past_gate(&self) -> bool {
        self.continued_past_gate
    }
    pub(crate) fn warnings(&self) -> &[ChecklistWarning] {
        &self.warnings
    }
    pub(crate) fn report(&self) -> &RunupReport {
        &self.observer.report
    }

    fn warn(&mut self, now: f64, kind: WarningKind, item: Option<&str>, text: String, poh_ref: &str) {
        if self.warnings.len() < 256 {
            self.warnings.push(ChecklistWarning {
                time_s: now,
                kind,
                item: item.map(str::to_owned),
                text,
                poh_ref: poh_ref.to_owned(),
            });
        }
    }

    fn step(&mut self, list: &Checklist, p: &Probe, dt: f64) {
        self.observer.observe(p);
        let now = p.time_s;
        // Timers.
        let mut expired: Vec<(String, String, String)> = Vec::new();
        let current = self.current_index();
        for (i, item) in list.items.iter().enumerate() {
            let Some(timer) = &item.timer else { continue };
            let Some(st) = self.items.get_mut(i) else { continue };
            let started = timer
                .start_when
                .as_ref()
                .is_some_and(|v| eval(v, p, &self.observer));
            match timer.kind {
                TimerKind::Max => {
                    if started {
                        st.timer.running = true;
                        st.timer.elapsed_s += dt;
                        if st.timer.elapsed_s > timer.seconds && !st.timer.expired {
                            st.timer.expired = true;
                            let text = format!("{} exceeded {:.0} s.", timer.label, timer.seconds);
                            expired.push((item.id.clone(), text, item.poh_ref.clone()));
                        }
                    } else {
                        st.timer.running = false;
                        st.timer.elapsed_s = 0.;
                    }
                }
                TimerKind::Within => {
                    if st.done.is_some() {
                        st.timer.running = false;
                    } else if started {
                        if st.timer.expired {
                            continue;
                        }
                        let holds = item.verify.as_ref().is_some_and(|v| eval(v, p, &self.observer));
                        if holds {
                            st.timer.running = false;
                        } else {
                            st.timer.running = true;
                            st.timer.elapsed_s += dt;
                            if st.timer.elapsed_s > timer.seconds {
                                st.timer.expired = true;
                                st.timer.running = false;
                                let text =
                                    format!("{}: not reached within {:.0} s.", timer.label, timer.seconds);
                                expired.push((item.id.clone(), text, item.poh_ref.clone()));
                            }
                        }
                    } else {
                        st.timer.running = false;
                        st.timer.elapsed_s = 0.;
                    }
                }
                TimerKind::Hold => {
                    if st.done.is_some() {
                        st.timer.running = false;
                    } else if Some(i) == current {
                        let holds = item.verify.as_ref().is_some_and(|v| eval(v, p, &self.observer));
                        if holds {
                            st.timer.running = true;
                            st.timer.elapsed_s += dt;
                        } else {
                            st.timer.running = false;
                            st.timer.elapsed_s = 0.;
                        }
                    }
                }
            }
        }
        for (id, text, poh_ref) in expired {
            self.warn(now, WarningKind::Timer, Some(&id), text, &poh_ref);
        }
        // Items: ticks, and actions performed out of sequence.
        let current = self.current_index();
        for (i, item) in list.items.iter().enumerate() {
            let Some(st) = self.items.get_mut(i) else { continue };
            if st.done.is_some() {
                continue;
            }
            let Some(v) = &item.verify else { continue };
            let holds = eval(v, p, &self.observer);
            let edge = holds && !st.was_true;
            st.was_true = holds;
            if edge && is_action(v) && current.is_some_and(|c| i > c) && !st.out_of_order_warned {
                st.out_of_order_warned = true;
                if item.timer.as_ref().is_none_or(|t| t.kind != TimerKind::Hold) {
                    let next = current
                        .and_then(|c| list.items.get(c))
                        .map_or("", |n| n.text.as_str());
                    let text = format!("Out of sequence: \"{}\" before \"{}\".", item.text, next);
                    self.warn(now, WarningKind::Sequence, Some(&item.id), text, &item.poh_ref);
                    // Pressing on with later actions while an earlier go/no-go item is unmet.
                    if list.items.iter().take(i).enumerate().any(|(g, it)| {
                        it.stop_on_failure && self.items.get(g).is_some_and(|x| x.done.is_none())
                    }) {
                        self.continued_past_gate = true;
                    }
                }
            }
        }
        // Advance through items already satisfied, stopping at the first that is not.
        while let Some(c) = self.current_index() {
            let Some(item) = list.items.get(c) else { break };
            let Some(v) = &item.verify else { break };
            let Some(st) = self.items.get(c) else { break };
            let timer_ok = match item.timer.as_ref().map(|t| (t.kind, t.seconds)) {
                Some((TimerKind::Hold, secs)) => st.timer.elapsed_s >= secs,
                _ => true,
            };
            if eval(v, p, &self.observer) && timer_ok {
                if let Some(st) = self.items.get_mut(c) {
                    st.done = Some((DoneBy::Verified, now));
                }
            } else {
                break;
            }
        }
        // Gates: a later item done while an earlier gate is unmet.
        if !self.continued_past_gate {
            let gate_open =
                list.items
                    .iter()
                    .zip(&self.items)
                    .enumerate()
                    .any(|(i, (item, st))| {
                        item.stop_on_failure
                            && st.done.is_none()
                            && self.items.iter().skip(i + 1).any(|later| later.done.is_some())
                    })
                    || list.items.iter().zip(&self.items).any(|(item, st)| {
                        item.stop_on_failure && matches!(st.done, Some((DoneBy::Forced, _)))
                    });
            if gate_open {
                self.continued_past_gate = true;
            }
            if self.continued_past_gate && !self.gate_warned {
                self.gate_warned = true;
                self.warn(
                    now,
                    WarningKind::Gate,
                    None,
                    "Continued past a go/no-go item that was not satisfied; stop and investigate.".into(),
                    &list.poh_ref,
                );
            }
        }
        if !p.on_ground && self.items.iter().any(|i| i.done.is_none()) && !self.continued_past_gate {
            // Airborne with the checklist unfinished: the defect, if any, was flown with.
            let gate_unmet = list
                .items
                .iter()
                .zip(&self.items)
                .any(|(it, st)| it.stop_on_failure && st.done.is_none());
            if gate_unmet {
                self.continued_past_gate = true;
            }
        }
        // Limits.
        let mut active = Vec::new();
        for (i, limit) in list.limits.iter().enumerate() {
            let holds = eval(&limit.when, p, &self.observer);
            if holds {
                active.push(limit.message.clone());
                if !self.limit_was.get(i).copied().unwrap_or(false) {
                    self.warn(
                        now,
                        WarningKind::Limit,
                        None,
                        limit.message.clone(),
                        &limit.poh_ref,
                    );
                }
            }
            if let Some(was) = self.limit_was.get_mut(i) {
                *was = holds;
            }
        }
        self.active_limits = active;
        if self.completed_at_s.is_none() && self.is_complete() {
            self.completed_at_s = Some(now);
        }
        for st in &mut self.items {
            if st.done.is_none() {
                st.failed_detail = None;
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecklistListCommand {}
impl Command for ChecklistListCommand {
    const NAME: &'static str = "checklist.list";
    const DESCRIPTION: &'static str = "List the aircraft's normal-procedure checklists (before start through securing) with POH references.";
    type Output = Vec<ChecklistSummary>;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(s.checklist_view().available)
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecklistStateCommand {}
impl Command for ChecklistStateCommand {
    const NAME: &'static str = "checklist.state";
    const DESCRIPTION: &'static str = "Read the active checklist: items, which are done and by what, timers, limit and sequence warnings, and run-up measurements.";
    type Output = ChecklistView;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(s.checklist_view())
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecklistSelect {
    /// A checklist id, or omitted to close the checklist.
    #[serde(default)]
    pub id: Option<String>,
}
impl Command for ChecklistSelect {
    const NAME: &'static str = "checklist.select";
    const DESCRIPTION: &'static str = "Open a checklist from the aircraft pack, or close it. Opening resets progress and run-up measurements.";
    type Output = ChecklistView;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        match self.id {
            None => s.checklist = ChecklistRuntime::default(),
            Some(id) => {
                let list = s
                    .procedure_set()
                    .and_then(|p| p.checklists.iter().find(|c| c.id == id))
                    .cloned()
                    .ok_or_else(|| invalid(Self::NAME, "unknown checklist for this aircraft"))?;
                s.checklist = ChecklistRuntime::new(&list);
                s.checklist.last_eval_s = None;
                // Evaluate once so already-satisfied items start ticked and are not "new actions".
                let probe = Probe::read(s);
                let mut rt = std::mem::take(&mut s.checklist);
                rt.last_eval_s = Some(probe.time_s);
                rt.step(&list, &probe, 0.);
                rt.warnings.clear();
                rt.gate_warned = false;
                rt.continued_past_gate = false;
                rt.limit_was = list
                    .limits
                    .iter()
                    .map(|l| eval(&l.when, &probe, &rt.observer))
                    .collect();
                s.checklist = rt;
            }
        }
        Ok(s.checklist_view())
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecklistComplete {
    pub item: String,
    /// Acknowledge although the binding is not satisfied (recorded as forced).
    #[serde(default)]
    pub force: bool,
}
impl Command for ChecklistComplete {
    const NAME: &'static str = "checklist.complete";
    const DESCRIPTION: &'static str = "Tick a checklist item by hand. Items the simulator can verify refuse unless forced; a forced go/no-go item counts as continuing past the defect.";
    type Output = ChecklistView;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let list = s
            .active_checklist()
            .ok_or_else(|| invalid(Self::NAME, "no checklist is open"))?;
        let i = list
            .items
            .iter()
            .position(|x| x.id == self.item)
            .ok_or_else(|| invalid(Self::NAME, "unknown item"))?;
        let probe = Probe::read(s);
        let rt = &mut s.checklist;
        if rt.items.get(i).is_none_or(|st| st.done.is_some()) {
            return Err(invalid(Self::NAME, "item already done"));
        }
        let item = list
            .items
            .get(i)
            .ok_or_else(|| invalid(Self::NAME, "unknown item"))?;
        let how = match &item.verify {
            None => DoneBy::Manual,
            Some(v) if eval(v, &probe, &rt.observer) => DoneBy::Verified,
            Some(_) if self.force => DoneBy::Forced,
            Some(_) => {
                return Err(invalid(
                    Self::NAME,
                    "the simulator does not show this item done; do it, or force to override",
                ));
            }
        };
        if rt.current_index().is_some_and(|c| i > c) {
            let text = format!("Out of sequence: \"{}\" ticked ahead of the list.", item.text);
            rt.warn(
                probe.time_s,
                WarningKind::Sequence,
                Some(&item.id),
                text,
                &item.poh_ref,
            );
        }
        if let Some(st) = rt.items.get_mut(i) {
            st.done = Some((how, probe.time_s));
        }
        rt.step(&list, &probe, 0.);
        Ok(s.checklist_view())
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecklistAbort {
    pub reason: String,
}
impl Command for ChecklistAbort {
    const NAME: &'static str = "checklist.abort";
    const DESCRIPTION: &'static str = "Stop the procedure and squawk a defect or an unmet limit: the correct response to a failed run-up check. Recorded for grading.";
    type Output = ChecklistView;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if self.reason.trim().is_empty() || self.reason.len() > 512 {
            return Err(invalid(Self::NAME, "reason must be 1..512 bytes"));
        }
        let list = s
            .active_checklist()
            .ok_or_else(|| invalid(Self::NAME, "no checklist is open"))?;
        let now = s.sim.state().time_s;
        let item = s
            .checklist
            .current_index()
            .and_then(|i| list.items.get(i))
            .map(|i| i.id.clone());
        s.checklist.aborted = Some(AbortRecord {
            time_s: now,
            item,
            reason: self.reason,
        });
        s.checklist.show = None;
        Ok(s.checklist_view())
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecklistShowMe {
    pub item: String,
}
impl Command for ChecklistShowMe {
    const NAME: &'static str = "checklist.show_me";
    const DESCRIPTION: &'static str = "The virtual pilot performs one checklist item and explains it on the intercom. Run-up checks are performed with the proper holds.";
    type Output = ChecklistView;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let list = s
            .active_checklist()
            .ok_or_else(|| invalid(Self::NAME, "no checklist is open"))?;
        let item = list
            .items
            .iter()
            .find(|x| x.id == self.item)
            .ok_or_else(|| invalid(Self::NAME, "unknown item"))?;
        let show = show::ShowMe::plan(s, item).ok_or_else(|| {
            invalid(
                Self::NAME,
                "nothing to demonstrate: this item has no simulator binding",
            )
        })?;
        s.crew_say(show.explanation(item));
        s.checklist.show = Some(show);
        Ok(s.checklist_view())
    }
}
