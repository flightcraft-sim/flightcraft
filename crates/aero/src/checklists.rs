//! Normal-procedure checklists, carried in the systems layer of an aircraft pack.
//!
//! Emergency procedures live in [`crate::emergencies`]; this is the matching normal-procedure
//! data (before start, start, warm-up, run-up, taxi, before take-off, shutdown, securing). An
//! item may carry a [`Verify`] binding to simulator state, so a checklist can tick itself and be
//! graded, a `control` naming the panel element to highlight, and a timer for POH time limits.
//! The data is original wording with references to the pack author's POH, never a replacement.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The only procedure-data version this build reads. Unknown versions are rejected.
pub const PROCEDURES_VERSION: u8 = 1;
const MAX_CHECKLISTS: usize = 32;
const MAX_ITEMS: usize = 64;
const MAX_LIMITS: usize = 16;
const MAX_DEPTH: usize = 4;

/// Which part of the flight a checklist covers; drives the ground layout and lesson selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChecklistPhase {
    BeforeStart,
    EngineStart,
    WarmUp,
    RunUp,
    Taxi,
    BeforeTakeoff,
    Shutdown,
    Securing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LaneId {
    A,
    B,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PumpId {
    One,
    Two,
}

/// A run-up check recorded by the simulator's observer while the pilot performs it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "check", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunupCheck {
    /// One ignition lane switched off at `at_rpm`; the other must carry the engine. The recorded
    /// drop must not exceed `max_drop_rpm` plus the measurement allowance `allow_rpm`.
    Lane {
        lane: LaneId,
        at_rpm: f64,
        max_drop_rpm: f64,
        allow_rpm: f64,
    },
    /// One electric fuel pump alone keeps fuel pressure at or above `min_kpa`.
    Pump { pump: PumpId, min_kpa: f64 },
    /// Every primary control moved to at least `min_deflection` both ways, and the flaps cycled.
    Controls { min_deflection: f64 },
    /// Full throttle reaches `min_rpm` without exceeding `max_rpm`, accelerating from idle within
    /// `within_s` seconds.
    MaxRpm {
        min_rpm: f64,
        max_rpm: f64,
        within_s: f64,
    },
}

/// A condition on simulator state. Used for item completion, timers, and limit warnings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Verify {
    Master {
        on: bool,
    },
    Avionics {
        on: bool,
    },
    /// Ignition switches: both magnetos, or both electronic lanes with the key on.
    Ignition {
        on: bool,
    },
    Lane {
        lane: LaneId,
        on: bool,
    },
    Pump {
        pump: PumpId,
        on: bool,
    },
    /// Backup battery switch for a named circuit, optionally with a minimum voltage.
    BackupBattery {
        circuit: crate::panel_fidelity::BackupCircuit,
        on: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_voltage: Option<f64>,
    },
    /// Any of the listed selector positions (`left`, `right`, `both`, `off`).
    FuelSelector {
        positions: Vec<String>,
    },
    /// Any panel control by element id, in the control's own 0..1 range.
    Panel {
        control: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    /// Crankshaft rpm.
    Rpm {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    OilTemp {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_c: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_c: Option<f64>,
    },
    OilPressure {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_bar: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_bar: Option<f64>,
    },
    FuelPressure {
        min_kpa: f64,
    },
    /// Main bus voltage.
    BusVoltage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_v: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_v: Option<f64>,
    },
    EngineRunning {
        running: bool,
    },
    Starter {
        engaged: bool,
    },
    /// Wheel brakes at or above half pressure; the simulator has no separate park-brake valve.
    Brake {
        set: bool,
    },
    /// Flap position as a fraction of full travel.
    Flaps {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    /// Pitch trim within `tolerance` of neutral (normalised, 0..0.3).
    TrimNeutral {
        tolerance: f64,
    },
    Throttle {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    /// Sliding canopy closed and latched (always true on aircraft without one).
    CanopyLatched {
        latched: bool,
    },
    /// A run-up check the simulator has observed and found within limits.
    Check(RunupCheck),
    /// Every listed condition holds.
    All {
        of: Vec<Verify>,
    },
}

/// How a POH time limit applies to an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimerKind {
    /// Counts while `start_when` holds; exceeding `seconds` warns (starter duty limit).
    Max,
    /// Counts from `start_when`; the item's `verify` must hold within `seconds` or the timer
    /// expires with a warning (oil pressure within 10 s of start).
    Within,
    /// The item's `verify` must hold continuously for `seconds` to complete it (warm-up).
    Hold,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemTimer {
    pub label: String,
    pub kind: TimerKind,
    pub seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_when: Option<Verify>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecklistItem {
    pub id: String,
    /// The challenge, as read from the POH.
    pub text: String,
    /// The response ("on", "clear of people and obstructions").
    pub response: String,
    /// Section of the pilot's operating handbook, e.g. "POH 4.5.2 item 7".
    pub poh_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<Verify>,
    /// Panel element id to highlight on the dash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer: Option<ItemTimer>,
    /// Extra caution or note shown beneath the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Set when a number comes from outside the pilot's operating handbook and has not been
    /// verified against that source ("per Rotax 912 iS operator's manual, not verified").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unverified: Option<String>,
    /// A go/no-go gate: if the verify binding is unmet or fails, the pilot must stop (abort)
    /// rather than carry on to later items.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stop_on_failure: bool,
}

/// A condition that must not be allowed to persist; shown as a warning while it holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecklistLimit {
    pub id: String,
    pub message: String,
    pub poh_ref: String,
    pub when: Verify,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Checklist {
    pub id: String,
    pub title: String,
    pub phase: ChecklistPhase,
    pub poh_ref: String,
    pub items: Vec<ChecklistItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<ChecklistLimit>,
}

/// All normal-procedure checklists of an aircraft, with an explicit data version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureSet {
    #[schemars(range(min = 1, max = 1))]
    pub version: u8,
    /// Document and revision the lists were transcribed from.
    pub source: String,
    pub checklists: Vec<Checklist>,
}

fn text_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn range_ok(v: Option<f64>, lo: f64, hi: f64) -> bool {
    v.is_none_or(|v| v.is_finite() && (lo..=hi).contains(&v))
}
fn bad(what: &str) -> crate::AircraftError {
    crate::aircraft::error("procedures", what)
}

impl RunupCheck {
    fn validate(&self) -> Result<(), crate::AircraftError> {
        let ok = match *self {
            Self::Lane {
                at_rpm,
                max_drop_rpm,
                allow_rpm,
                ..
            } => {
                (1000. ..=7000.).contains(&at_rpm)
                    && (0. ..=1000.).contains(&max_drop_rpm)
                    && (0. ..=200.).contains(&allow_rpm)
            }
            Self::Pump { min_kpa, .. } => min_kpa.is_finite() && (0. ..=2000.).contains(&min_kpa),
            Self::Controls { min_deflection } => {
                min_deflection.is_finite() && (0.1..=1.).contains(&min_deflection)
            }
            Self::MaxRpm {
                min_rpm,
                max_rpm,
                within_s,
            } => {
                (1000. ..=8000.).contains(&min_rpm)
                    && (min_rpm..=8000.).contains(&max_rpm)
                    && within_s.is_finite()
                    && (0.5..=60.).contains(&within_s)
            }
        };
        if ok {
            Ok(())
        } else {
            Err(bad("run-up check out of range"))
        }
    }
}

impl Verify {
    fn validate(&self, depth: usize) -> Result<(), crate::AircraftError> {
        if depth > MAX_DEPTH {
            return Err(bad("verify nested too deeply"));
        }
        let ok = match self {
            Self::Master { .. }
            | Self::Avionics { .. }
            | Self::Ignition { .. }
            | Self::Lane { .. }
            | Self::Pump { .. }
            | Self::EngineRunning { .. }
            | Self::Starter { .. }
            | Self::Brake { .. }
            | Self::CanopyLatched { .. } => true,
            Self::BackupBattery { min_voltage, .. } => range_ok(*min_voltage, 0., 60.),
            Self::FuelSelector { positions } => {
                !positions.is_empty()
                    && positions.len() <= 3
                    && positions
                        .iter()
                        .all(|p| matches!(p.as_str(), "left" | "right" | "both" | "off"))
            }
            Self::Panel { control, min, max } => {
                text_ok(control, 64) && range_ok(*min, 0., 1.) && range_ok(*max, 0., 1.)
            }
            Self::Rpm { min, max } => range_ok(*min, 0., 8000.) && range_ok(*max, 0., 8000.),
            Self::OilTemp { min_c, max_c } => range_ok(*min_c, -50., 200.) && range_ok(*max_c, -50., 200.),
            Self::OilPressure { min_bar, max_bar } => {
                range_ok(*min_bar, 0., 20.) && range_ok(*max_bar, 0., 20.)
            }
            Self::FuelPressure { min_kpa } => min_kpa.is_finite() && (0. ..=2000.).contains(min_kpa),
            Self::BusVoltage { min_v, max_v } => range_ok(*min_v, 0., 60.) && range_ok(*max_v, 0., 60.),
            Self::Flaps { min, max } => range_ok(*min, 0., 1.) && range_ok(*max, 0., 1.),
            Self::TrimNeutral { tolerance } => tolerance.is_finite() && (0.001..=0.3).contains(tolerance),
            Self::Throttle { min, max } => range_ok(*min, 0., 1.) && range_ok(*max, 0., 1.),
            Self::Check(check) => return check.validate(),
            Self::All { of } => {
                if of.is_empty() || of.len() > 8 {
                    return Err(bad("all requires 1..8 conditions"));
                }
                return of.iter().try_for_each(|v| v.validate(depth + 1));
            }
        };
        let bounded = match self {
            Self::Rpm { min, max } | Self::Throttle { min, max } | Self::Flaps { min, max } => {
                min.zip(*max).is_none_or(|(a, b)| a <= b)
            }
            Self::OilTemp {
                min_c: min,
                max_c: max,
            } => min.zip(*max).is_none_or(|(a, b)| a <= b),
            Self::OilPressure {
                min_bar: min,
                max_bar: max,
            } => min.zip(*max).is_none_or(|(a, b)| a <= b),
            Self::BusVoltage {
                min_v: min,
                max_v: max,
            } => min.zip(*max).is_none_or(|(a, b)| a <= b),
            _ => true,
        };
        if ok && bounded {
            Ok(())
        } else {
            Err(bad("verify value out of range"))
        }
    }
}

impl Checklist {
    pub fn validate(&self) -> Result<(), crate::AircraftError> {
        if !text_ok(&self.id, 64)
            || !text_ok(&self.title, 256)
            || !text_ok(&self.poh_ref, 256)
            || self.items.is_empty()
            || self.items.len() > MAX_ITEMS
            || self.limits.len() > MAX_LIMITS
        {
            return Err(bad("invalid checklist bounds"));
        }
        let mut ids = std::collections::BTreeSet::new();
        for item in &self.items {
            if !text_ok(&item.id, 64)
                || !ids.insert(item.id.as_str())
                || !text_ok(&item.text, 256)
                || !text_ok(&item.response, 256)
                || !text_ok(&item.poh_ref, 256)
                || item.control.as_deref().is_some_and(|c| !text_ok(c, 64))
                || item.note.as_deref().is_some_and(|n| !text_ok(n, 1024))
                || item.unverified.as_deref().is_some_and(|n| !text_ok(n, 256))
            {
                return Err(bad("invalid checklist item"));
            }
            if let Some(v) = &item.verify {
                v.validate(0)?;
            }
            if let Some(t) = &item.timer {
                if !text_ok(&t.label, 128) || !t.seconds.is_finite() || !(1. ..=3600.).contains(&t.seconds) {
                    return Err(bad("invalid item timer"));
                }
                if let Some(v) = &t.start_when {
                    v.validate(0)?;
                }
                let needs_start = matches!(t.kind, TimerKind::Max | TimerKind::Within);
                if needs_start && t.start_when.is_none() {
                    return Err(bad("max and within timers require start_when"));
                }
                if matches!(t.kind, TimerKind::Hold | TimerKind::Within) && item.verify.is_none() {
                    return Err(bad("hold and within timers require a verify binding"));
                }
            }
            if item.stop_on_failure && item.verify.is_none() {
                return Err(bad("stop_on_failure requires a verify binding"));
            }
        }
        let mut limit_ids = std::collections::BTreeSet::new();
        for limit in &self.limits {
            if !text_ok(&limit.id, 64)
                || !limit_ids.insert(limit.id.as_str())
                || !text_ok(&limit.message, 512)
                || !text_ok(&limit.poh_ref, 256)
            {
                return Err(bad("invalid checklist limit"));
            }
            limit.when.validate(0)?;
        }
        Ok(())
    }
}

impl ProcedureSet {
    pub fn validate(&self) -> Result<(), crate::AircraftError> {
        if self.version != PROCEDURES_VERSION {
            return Err(bad("unsupported procedure data version"));
        }
        if !text_ok(&self.source, 512) || self.checklists.is_empty() || self.checklists.len() > MAX_CHECKLISTS
        {
            return Err(bad("invalid procedure set"));
        }
        let mut ids = std::collections::BTreeSet::new();
        for list in &self.checklists {
            list.validate()?;
            if !ids.insert(list.id.as_str()) {
                return Err(bad("duplicate checklist id"));
            }
        }
        Ok(())
    }
}
