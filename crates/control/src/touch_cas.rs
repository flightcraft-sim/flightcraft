//! The landscape touch EFIS's crew alerting (CAS) window and its trim and flap position
//! indicators, as pure functions over the simulation's existing annunciations and controls.
//!
//! Pilot's guide 190-01754-00 Rev. Y: CAS messages are grouped by criticality (warning red, caution
//! yellow, advisory white), shown in a window on the PFD, and acknowledged by touching the window
//! or pressing BACK (3.7, printed pages 106 and 107; appendix 10.5, pages 376 and 377). The trim
//! and flap position indicators are optional PFD indications beside the EIS page (page 81).
//! Nothing here stores state: the messages are rebuilt from the annunciations each frame.
use flightcraft_aero::modern::Severity;
use flightcraft_sim::equipment::EquipmentFailure;
use flightcraft_sim::modern::ModernState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How many messages the PFD's window has room for; more show the down-pointing triangle.
pub const CAS_WINDOW_LINES: usize = 3;

/// A message's urgency, in the guide's colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CasSeverity {
    /// Red: requires immediate attention.
    Warning,
    /// Yellow: pilot awareness and possible corrective action.
    Caution,
    /// White: general information.
    Advisory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CasMessage {
    pub severity: CasSeverity,
    pub text: String,
    /// The pilot has acknowledged this message; it stays listed until its condition clears.
    #[serde(default)]
    pub acknowledged: bool,
    /// Activation order: a later message has a larger number. Autopilot failures are 0.
    #[serde(default)]
    pub sequence: u64,
    /// Position in the aircraft's annunciation list, or none for an autopilot failure.
    #[serde(default)]
    pub annunciation: Option<usize>,
}

/// What the CAS window shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CasWindow {
    /// Every active message, warnings first, then cautions, then advisories, each group in
    /// declaration order.
    pub messages: Vec<CasMessage>,
    /// A warning or caution has not been acknowledged: the window's text flashes.
    pub unacknowledged: bool,
}

impl CasWindow {
    /// The messages the window has room for.
    pub fn visible(&self) -> &[CasMessage] {
        self.messages.get(..CAS_WINDOW_LINES).unwrap_or(&self.messages)
    }

    /// The message list page's order: warnings, then cautions, then advisories, newest first
    /// within each. `cas_acknowledge` indices refer to this order.
    pub fn list(&self) -> Vec<CasMessage> {
        let mut list = self.messages.clone();
        list.sort_by(|a, b| a.severity.cmp(&b.severity).then(b.sequence.cmp(&a.sequence)));
        list
    }

    /// More messages than lines: the guide's upside-down triangle appears.
    pub fn overflow(&self) -> bool {
        self.messages.len() > CAS_WINDOW_LINES
    }
}

fn failure_text(f: EquipmentFailure) -> &'static str {
    match f {
        EquipmentFailure::RollServo => "ROLL SERVO",
        EquipmentFailure::PitchServo => "PITCH SERVO",
        EquipmentFailure::Trim => "PITCH TRIM",
        EquipmentFailure::RollSensor
        | EquipmentFailure::PressureSensor
        | EquipmentFailure::AttitudeSensor => "AP SENSOR",
    }
}

/// The CAS window for the aircraft's active annunciations (the master caution and warning
/// lamps' sources) and the autopilot's failures. Autopilot failures are cautions.
pub fn window(modern: Option<&ModernState>, autopilot_failures: &[EquipmentFailure]) -> CasWindow {
    let mut messages = Vec::new();
    if let Some(m) = modern {
        for (i, a) in m.annunciations.iter().enumerate().filter(|(_, a)| a.active) {
            messages.push(CasMessage {
                acknowledged: a.acknowledged,
                sequence: a.sequence,
                annunciation: Some(i),
                severity: match a.severity {
                    Severity::Warning => CasSeverity::Warning,
                    Severity::Caution => CasSeverity::Caution,
                },
                text: a.label.clone(),
            });
        }
    }
    for f in autopilot_failures {
        let text = failure_text(*f);
        if !messages.iter().any(|m| m.text == text) {
            messages.push(CasMessage {
                severity: CasSeverity::Caution,
                text: text.to_owned(),
                acknowledged: true,
                sequence: 0,
                annunciation: None,
            });
        }
    }
    // A stable sort keeps declaration order within a group.
    messages.sort_by_key(|m| m.severity);
    let messages_acknowledged = messages.iter().all(|m| m.acknowledged);
    CasWindow {
        messages,
        unacknowledged: !messages_acknowledged
            || modern.is_some_and(|m| m.master_caution || m.master_warning),
    }
}

/// The severities whose master lamp is lit, for acknowledging from the window or BACK.
pub fn unacknowledged_severities(modern: Option<&ModernState>) -> Vec<Severity> {
    let mut out = Vec::new();
    if let Some(m) = modern {
        if m.master_warning {
            out.push(Severity::Warning);
        }
        if m.master_caution {
            out.push(Severity::Caution);
        }
    }
    out
}

/// The flap position indicator: a vertical scale, 0 at the top, the last detent's deflection at
/// the bottom, and a pointer at the present position.
#[derive(Clone, Debug, PartialEq)]
pub struct FlapGauge {
    /// 0 (retracted, top) to 1 (fully extended, bottom).
    pub fraction: f64,
    /// Scale label at the bottom: the full deflection in degrees when the variant declares it.
    pub full_label: String,
    /// The pointer's degrees, when the variant declares the deflection.
    pub degrees: Option<f64>,
}

/// The elevator trim indicator: DN at the top, UP at the bottom, as in the guide's figure.
#[derive(Clone, Debug, PartialEq)]
pub struct TrimGauge {
    /// -1 (full nose down, top) to 1 (full nose up, bottom).
    pub nose_up: f64,
    /// The pointer is off the neutral mark.
    pub off_neutral: bool,
}

/// Trim travel of the engine's trim control, in elevator-equivalent units.
pub const TRIM_LIMIT: f64 = 0.3;

pub fn flap_gauge(position: f64, full_deflection_deg: Option<f64>) -> FlapGauge {
    let fraction = if position.is_finite() {
        position.clamp(0., 1.)
    } else {
        0.
    };
    FlapGauge {
        fraction,
        full_label: full_deflection_deg.map_or_else(|| "F".to_owned(), |d| format!("{d:.0}")),
        degrees: full_deflection_deg.map(|d| d * fraction),
    }
}

pub fn trim_gauge(trim: f64) -> TrimGauge {
    let nose_up = if trim.is_finite() {
        (trim / TRIM_LIMIT).clamp(-1., 1.)
    } else {
        0.
    };
    TrimGauge {
        nose_up,
        off_neutral: nose_up.abs() > 0.02,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn no_annunciations_make_an_empty_window() {
        let w = window(None, &[]);
        assert!(w.messages.is_empty() && !w.unacknowledged && !w.overflow());
    }

    #[test]
    fn autopilot_failures_are_cautions_without_duplicates() {
        let w = window(
            None,
            &[
                EquipmentFailure::RollSensor,
                EquipmentFailure::PressureSensor,
                EquipmentFailure::Trim,
            ],
        );
        let texts: Vec<_> = w.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["AP SENSOR", "PITCH TRIM"]);
        assert!(w.messages.iter().all(|m| m.severity == CasSeverity::Caution));
    }

    #[test]
    fn gauges_clamp_and_label() {
        assert_eq!(flap_gauge(2., Some(32.)).fraction, 1.);
        assert_eq!(flap_gauge(0.5, Some(32.)).degrees, Some(16.));
        assert_eq!(flap_gauge(0.5, None).full_label, "F");
        assert_eq!(flap_gauge(1., Some(32.)).full_label, "32");
        assert_eq!(trim_gauge(0.3).nose_up, 1.);
        assert_eq!(trim_gauge(-9.).nose_up, -1.);
        assert!(!trim_gauge(0.).off_neutral && trim_gauge(0.1).off_neutral);
    }
}
