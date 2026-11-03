//! Human facility labels and transcript presentation; machine records remain structured.
use super::*;

pub fn facility_role(role: FacilityKind) -> &'static str {
    match role {
        FacilityKind::Tower => "Tower",
        FacilityKind::Ground => "Ground",
        FacilityKind::Approach => "Approach",
        FacilityKind::Departure => "Departure",
        FacilityKind::Clearance => "Clearance",
        FacilityKind::Ctaf => "Traffic",
        FacilityKind::Unicom => "Unicom",
        FacilityKind::Awos => "AWOS",
        FacilityKind::Asos => "ASOS",
        FacilityKind::Atis => "ATIS",
    }
}
pub fn facility_radio_name(station: &ComStation) -> String {
    let name = station
        .metadata
        .as_ref()
        .map(|m| m.callsign.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&station.name);
    let name = name
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or(String::new(), |c| {
                format!("{}{}", c.to_uppercase(), chars.as_str().to_lowercase())
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    let role = facility_role(station.kind);
    if name.to_lowercase().ends_with(&role.to_lowercase()) {
        name
    } else {
        format!("{name} {role}")
    }
}
pub fn speaker_label(speaker: &Speaker, stations: &[ComStation]) -> String {
    match speaker {
        Speaker::Pilot => "Pilot".into(),
        Speaker::Instructor => "Instructor".into(),
        Speaker::Facility { id, role } => stations
            .iter()
            .find(|s| s.id == *id)
            .map_or_else(|| facility_role(*role).into(), facility_radio_name),
        Speaker::Traffic { callsign } => callsign.clone(),
        Speaker::Nav { ident } => format!("Navigation {ident}"),
    }
}
impl Transmission {
    /// State changes remain in the machine record, but are never speech or transcript lines.
    pub fn is_spoken(&self) -> bool {
        !matches!(self.meaning, Meaning::ScenarioAction { .. })
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
        let reception = match self.reception {
            Reception::Heard => "heard",
            Reception::Pending => "pending",
            Reception::Blocked => "blocked",
            Reception::NotReceived => "not received",
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
            speaker_label(&self.speaker, stations),
            self.text
        ))
    }
}
