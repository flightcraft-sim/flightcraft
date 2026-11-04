//! Observer for the physical run-up checks: ignition lanes, fuel pumps, controls, maximum rpm.
//!
//! It only watches simulator state (never changes it) at the checklist evaluation rate, so a
//! check is recorded identically whether the student, the virtual pilot, or a script performs it.
use super::eval::Probe;
use flightcraft_aero::checklists::{LaneId, PumpId, RunupCheck};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Seconds a lane stays off before its rpm is read (POH 9.4.13 asks for one lane at a time; the
/// Rotax procedure allows about three seconds per step).
pub const LANE_SETTLE_S: f64 = 2.5;
/// Seconds a pump runs alone before its pressure is read.
pub const PUMP_SETTLE_S: f64 = 2.0;
/// Full-throttle seconds before the maximum rpm reading counts.
pub const FULL_POWER_HOLD_S: f64 = 2.0;
/// Throttle at which the engine is considered at full power.
const FULL_THROTTLE: f64 = 0.9;
/// Time constant of the rpm filter, seconds.
const RPM_TAU_S: f64 = 0.4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LaneMeasure {
    pub lane: LaneId,
    pub baseline_rpm: f64,
    pub observed_rpm: f64,
    /// Baseline minus observed; negative when the engine sped up.
    pub drop_rpm: f64,
    pub at_s: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PumpMeasure {
    pub pump: PumpId,
    pub pressure_kpa: f64,
    pub at_s: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AxisExtent {
    pub min: f64,
    pub max: f64,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ControlsMeasure {
    pub elevator: AxisExtent,
    pub aileron: AxisExtent,
    pub rudder: AxisExtent,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaxRpmMeasure {
    pub peak_rpm: f64,
    /// `(seconds after full throttle was applied, filtered rpm)` at the evaluation rate.
    pub log: Vec<(f64, f64)>,
}
/// Everything the observer has measured since the checklist was selected.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunupReport {
    pub lanes: Vec<LaneMeasure>,
    pub pumps: Vec<PumpMeasure>,
    pub controls: ControlsMeasure,
    pub max_rpm: Option<MaxRpmMeasure>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RunupObserver {
    pub report: RunupReport,
    rpm_f: Option<f64>,
    last_t: Option<f64>,
    baseline: f64,
    lane_a: Watch,
    lane_b: Watch,
    pump_one: Watch,
    pump_two: Watch,
    low_throttle_seen: bool,
    full_since: Option<f64>,
}

/// A switch-off episode being timed: when it began and whether it has been measured.
#[derive(Clone, Copy, Debug, Default)]
struct Watch {
    since: Option<f64>,
    measured: bool,
}
impl Watch {
    /// Starts or continues timing; true once `settle_s` has passed and no reading was taken yet.
    fn due(&mut self, now: f64, settle_s: f64) -> bool {
        let since = *self.since.get_or_insert(now);
        if !self.measured && now - since >= settle_s {
            self.measured = true;
            return true;
        }
        false
    }
    fn reset(&mut self) {
        *self = Self::default();
    }
}

impl RunupObserver {
    pub fn filtered_rpm(&self) -> f64 {
        self.rpm_f.unwrap_or(0.)
    }
    pub fn observe(&mut self, p: &Probe) {
        let dt = self.last_t.map_or(0., |t| (p.time_s - t).max(0.));
        self.last_t = Some(p.time_s);
        let rpm = p.rpm;
        self.rpm_f = Some(match self.rpm_f {
            Some(f) if dt > 0. => f + (rpm - f) * (1. - (-dt / RPM_TAU_S).exp()),
            Some(f) => f,
            None => rpm,
        });
        let rpm_f = self.filtered_rpm();
        if !p.on_ground {
            return;
        }
        let axes = [
            (p.elevator, &mut self.report.controls.elevator),
            (p.aileron, &mut self.report.controls.aileron),
            (p.rudder, &mut self.report.controls.rudder),
        ];
        for (v, e) in axes {
            e.min = e.min.min(v);
            e.max = e.max.max(v);
        }
        // Ignition lanes: a lane counts as "off, other on" while the engine runs.
        if p.engine_running && p.lane_a_on && p.lane_b_on {
            self.baseline = rpm_f;
        }
        for (lane, off, other_on) in [
            (LaneId::A, !p.lane_a_on, p.lane_b_on),
            (LaneId::B, !p.lane_b_on, p.lane_a_on),
        ] {
            let watch = match lane {
                LaneId::A => &mut self.lane_a,
                LaneId::B => &mut self.lane_b,
            };
            if off && other_on {
                if watch.due(p.time_s, LANE_SETTLE_S) {
                    self.report.lanes.retain(|m| m.lane != lane);
                    self.report.lanes.push(LaneMeasure {
                        lane,
                        baseline_rpm: self.baseline,
                        observed_rpm: rpm_f,
                        drop_rpm: self.baseline - rpm_f,
                        at_s: p.time_s,
                    });
                }
            } else {
                watch.reset();
            }
        }
        // Fuel pumps: one pump running alone.
        for (pump, alone) in [
            (PumpId::One, p.pump1_on && !p.pump2_on),
            (PumpId::Two, p.pump2_on && !p.pump1_on),
        ] {
            let watch = match pump {
                PumpId::One => &mut self.pump_one,
                PumpId::Two => &mut self.pump_two,
            };
            if alone {
                if watch.due(p.time_s, PUMP_SETTLE_S) {
                    self.report.pumps.retain(|m| m.pump != pump);
                    self.report.pumps.push(PumpMeasure {
                        pump,
                        pressure_kpa: p.fuel_pressure_kpa,
                        at_s: p.time_s,
                    });
                }
            } else {
                watch.reset();
            }
        }
        // Maximum rpm: acceleration from a low throttle to full.
        if p.throttle < 0.5 {
            self.low_throttle_seen = true;
            self.full_since = None;
        } else if p.throttle >= FULL_THROTTLE && p.engine_running {
            if self.full_since.is_none() && self.low_throttle_seen {
                self.full_since = Some(p.time_s);
                self.report.max_rpm = Some(MaxRpmMeasure {
                    peak_rpm: 0.,
                    log: Vec::new(),
                });
            }
            if let (Some(since), Some(m)) = (self.full_since, self.report.max_rpm.as_mut()) {
                m.peak_rpm = m.peak_rpm.max(rpm_f);
                if m.log.len() < 600 {
                    m.log.push((p.time_s - since, rpm_f));
                }
            }
        }
    }

    pub fn held_full_s(&self, now: f64) -> f64 {
        self.full_since.map_or(0., |t| now - t)
    }

    /// Whether the recorded measurement satisfies the check, or `None` when not yet measured.
    pub fn judge(&self, check: &RunupCheck, now: f64) -> Option<bool> {
        match *check {
            RunupCheck::Lane {
                lane,
                at_rpm,
                max_drop_rpm,
                allow_rpm,
            } => {
                let m = self.report.lanes.iter().find(|m| m.lane == lane)?;
                if (m.baseline_rpm - at_rpm).abs() > at_rpm * 0.1 {
                    return None;
                }
                Some(m.drop_rpm <= max_drop_rpm + allow_rpm)
            }
            RunupCheck::Pump { pump, min_kpa } => {
                let m = self.report.pumps.iter().find(|m| m.pump == pump)?;
                Some(m.pressure_kpa >= min_kpa)
            }
            RunupCheck::Controls { min_deflection } => {
                let c = &self.report.controls;
                let moved = [c.elevator, c.aileron, c.rudder]
                    .iter()
                    .all(|e| e.min <= -min_deflection && e.max >= min_deflection);
                moved.then_some(true)
            }
            RunupCheck::MaxRpm {
                min_rpm,
                max_rpm,
                within_s,
            } => {
                let m = self.report.max_rpm.as_ref()?;
                if self.held_full_s(now) < FULL_POWER_HOLD_S {
                    return None;
                }
                let reached = m.log.iter().find(|(_, r)| *r >= min_rpm).map(|(t, _)| *t);
                Some(m.peak_rpm <= max_rpm && reached.is_some_and(|t| t <= within_s))
            }
        }
    }

    /// A human-readable measurement for the item, once recorded.
    pub fn describe(&self, check: &RunupCheck) -> Option<String> {
        match *check {
            RunupCheck::Lane { lane, .. } => self.report.lanes.iter().find(|m| m.lane == lane).map(|m| {
                format!(
                    "{:.0} rpm to {:.0} rpm, drop {:.0} rpm",
                    m.baseline_rpm, m.observed_rpm, m.drop_rpm
                )
            }),
            RunupCheck::Pump { pump, .. } => self
                .report
                .pumps
                .iter()
                .find(|m| m.pump == pump)
                .map(|m| format!("{:.0} kPa on this pump alone", m.pressure_kpa)),
            RunupCheck::Controls { .. } => {
                let c = &self.report.controls;
                Some(format!(
                    "elevator {:.1}/{:.1}, aileron {:.1}/{:.1}, rudder {:.1}/{:.1}",
                    c.elevator.min, c.elevator.max, c.aileron.min, c.aileron.max, c.rudder.min, c.rudder.max
                ))
            }
            RunupCheck::MaxRpm { min_rpm, .. } => self.report.max_rpm.as_ref().map(|m| {
                let reached = m.log.iter().find(|(_, r)| *r >= min_rpm).map(|(t, _)| *t);
                match reached {
                    Some(t) => format!("peak {:.0} rpm, {:.1} s to reach {:.0}", m.peak_rpm, t, min_rpm),
                    None => format!("peak {:.0} rpm, never reached {:.0}", m.peak_rpm, min_rpm),
                }
            }),
        }
    }
}
