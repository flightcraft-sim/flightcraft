//! Human facility labels and transcript presentation; machine records remain structured.
use super::*;

// Compatibility names for transcript clients; all wording comes from the shared renderer.
pub use super::phraseology::{
    facility as facility_radio_name, facility_kind as facility_role, speaker as speaker_label,
};
impl Transmission {
    /// State changes remain in the machine record, but are never speech or transcript lines.
    pub fn is_spoken(&self) -> bool {
        !matches!(
            self.meaning,
            Meaning::ScenarioAction { .. } | Meaning::Instrument { .. } | Meaning::Compliance { .. }
        )
    }
    pub fn is_simulated_atc(&self) -> bool {
        matches!(self.speaker, Speaker::Facility { .. })
            && matches!(
                self.source,
                EventSource::Scenario | EventSource::ScenarioAi | EventSource::Ai
            )
    }
    pub fn transcript_line(&self, stations: &[ComStation]) -> Option<String> {
        if !self.is_spoken() {
            return None;
        }
        let reception = if matches!(self.meaning, Meaning::Recognition { .. }) {
            "ungraded recognition preview"
        } else {
            match self.reception {
                Reception::Heard => "heard",
                Reception::Pending => "pending",
                Reception::Blocked => "blocked",
                Reception::NotReceived => "not received",
            }
        };
        let source = match self.source {
            EventSource::Rules => "rules",
            EventSource::Scenario => "scenario",
            EventSource::ScenarioAi => "scenario (ai-written)",
            EventSource::Ai => "ai",
            EventSource::Pilot => "pilot",
        };
        Some(format!(
            "{:7.2}s [{source}] {}{}: {} ({reception})",
            self.time_s,
            if self.is_simulated_atc() {
                "[Simulated ATC] "
            } else {
                ""
            },
            if self.speaker_name.is_empty() {
                speaker_label(&self.speaker, stations)
            } else {
                self.speaker_name.clone()
            },
            self.text
        ))
    }
}
