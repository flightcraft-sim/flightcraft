//! Session radio service. Communications never enter the physics state or its hash.
use crate::{Command, CommandError, Session, invalid};
use flightcraft_world::{
    ComStation, FacilityKind,
    comms::{carrier_thirds_hz, valid_channel},
    navdata::text_ok,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub(crate) mod archive;
pub use archive::*;
mod callsign;
pub use callsign::*;
mod presentation;
pub use presentation::*;
mod atc;
pub use atc::*;
pub mod phraseology;
mod speech;
pub(crate) mod weather;
pub use speech::*;
pub use weather::ReportedLayer;
mod fleet;
mod record;
mod traffic;
mod traffic_display;
mod voices;
pub use fleet::*;
pub use record::*;
use std::collections::VecDeque;
use traffic::Pattern;
pub use traffic::*;
pub use traffic_display::*;
pub use voices::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComSpacing {
    #[default]
    Khz25,
    Khz833,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComRadio {
    pub active_khz: u32,
    pub standby_khz: u32,
    pub spacing: ComSpacing,
}
impl Default for ComRadio {
    fn default() -> Self {
        Self {
            active_khz: 122800,
            standby_khz: 124000,
            spacing: ComSpacing::Khz25,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioPanel {
    pub transmit: u8,
    pub receive_com1: bool,
    pub receive_com2: bool,
    pub nav_ident: bool,
    pub intercom: bool,
    pub volume: f64,
    pub squelch: f64,
    pub speech: bool,
    pub captions: bool,
    pub ptt: bool,
    pub intercom_ptt: bool,
}
impl Default for AudioPanel {
    fn default() -> Self {
        Self {
            transmit: 1,
            receive_com1: true,
            receive_com2: false,
            nav_ident: false,
            intercom: true,
            volume: 0.7,
            squelch: 0.2,
            speech: true,
            captions: true,
            ptt: false,
            intercom_ptt: false,
        }
    }
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct CommsSnapshot {
    pub callsign: PilotCallsign,
    pub adf_khz: Option<u32>,
    pub powered: bool,
    pub com1_powered: bool,
    pub com2_powered: bool,
    pub com1: ComRadio,
    pub com2: ComRadio,
    pub panel: AudioPanel,
    pub stations: Vec<ComStation>,
    pub traffic: TrafficState,
    pub expected_call: Option<PatternLeg>,
}
#[derive(Clone, Debug)]
struct TrafficCall {
    khz: u32,
    speaker: Speaker,
    text: String,
    meaning: Meaning,
    origin: [f64; 3],
}
#[derive(Clone, Debug)]
pub(crate) struct Comms {
    speech_draft: Option<SpeechDraft>,
    pub(crate) atc: atc::Atc,
    callsign_overrides: CallsignPreferences,
    saved_callsigns: CallsignPreferences,
    abbreviate: bool,
    contacts: std::collections::BTreeSet<u32>,
    adf_khz: Option<u32>,
    generation: u64,
    pub record: FlightRecord,
    pub radios: [ComRadio; 2],
    panel: AudioPanel,
    pattern: Option<Pattern>,
    fleet: Option<fleet::Fleet>,
    /// Busy-airspace traffic at further towered airports; `fleet` is the first airport.
    more_fleets: Vec<fleet::Fleet>,
    advisories: fleet::AdvisoryBook,
    traffic_calls: VecDeque<TrafficCall>,
    pending: Vec<Pending>,
    heard: VecDeque<u64>,
    next_id: u64,
    last_tick: f64,
    pub(crate) next_weather: f64,
    next_ident: f64,
    extra_stations: Vec<ComStation>,
}
impl Default for Comms {
    fn default() -> Self {
        Self {
            speech_draft: None,
            atc: Default::default(),
            callsign_overrides: Default::default(),
            saved_callsigns: Default::default(),
            abbreviate: true,
            contacts: Default::default(),
            adf_khz: None,
            generation: 0,
            record: FlightRecord::default(),
            radios: Default::default(),
            panel: Default::default(),
            pattern: None,
            fleet: None,
            more_fleets: Vec::new(),
            advisories: Default::default(),
            traffic_calls: VecDeque::new(),
            pending: Vec::new(),
            heard: VecDeque::new(),
            next_id: 1,
            last_tick: 0.0,
            next_weather: 0.0,
            next_ident: 0.0,
            extra_stations: Vec::new(),
        }
    }
}
#[derive(Clone, Debug)]
struct Pending {
    id: u64,
    end_s: f64,
    carrier: Option<u32>,
    channel: Channel,
    eligible: bool,
    origin: Option<[f64; 3]>,
    range_nm: f64,
    collided: bool,
    radio: Option<u8>,
    atc_receiver: Option<ComStation>,
}
#[derive(Debug, Default, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommsTune {
    /// Optional ADF tuning for NDB identification, 190–1750 kHz.
    #[serde(default)]
    pub adf_khz: Option<u32>,
    pub radio: u8,
    #[serde(default)]
    pub channel_khz: Option<u32>,
    #[serde(default)]
    pub standby: bool,
    #[serde(default)]
    pub swap: bool,
    #[serde(default)]
    pub spacing: Option<ComSpacing>,
}
impl Command for CommsTune {
    const NAME: &'static str = "comms.tune";
    const DESCRIPTION: &'static str =
        "Tune COM1 or COM2 using a 25 or 8.33 kHz channel designator; optionally tune standby or swap.";
    type Output = CommsSnapshot;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if self.adf_khz.is_some_and(|f| !(190..=1750).contains(&f)) {
            return Err(invalid(Self::NAME, "ADF requires 190..1750 kHz"));
        }
        let radio = radio_mut(&mut s.comms.radios, self.radio)?;
        let mut replacement = radio.clone();
        if let Some(spacing) = self.spacing {
            replacement.spacing = spacing;
        }
        if let Some(khz) = self.channel_khz {
            if self.swap {
                return Err(invalid(Self::NAME, "tune and swap are separate actions"));
            }
            if self.standby {
                replacement.standby_khz = khz;
            } else {
                replacement.active_khz = khz;
            }
        }
        if self.swap {
            std::mem::swap(&mut replacement.active_khz, &mut replacement.standby_khz);
        }
        if ![replacement.active_khz, replacement.standby_khz]
            .iter()
            .all(|f| valid_channel(*f, replacement.spacing == ComSpacing::Khz833))
        {
            return Err(invalid(Self::NAME, "invalid COM channel or incompatible spacing"));
        }
        let old_carrier = carrier_thirds_hz(radio.active_khz);
        let new_carrier = carrier_thirds_hz(replacement.active_khz);
        *radio = replacement;
        if old_carrier != new_carrier
            && let Some(carrier) = new_carrier
        {
            s.comms.contacts.remove(&carrier);
        }
        if let Some(adf) = self.adf_khz {
            s.comms.adf_khz = Some(adf);
        }
        s.comms.next_weather = s.sim.state().time_s;
        Ok(s.comms_state())
    }
}
pub(super) fn radio_mut(radios: &mut [ComRadio; 2], radio: u8) -> Result<&mut ComRadio, CommandError> {
    radios
        .get_mut(usize::from(
            radio
                .checked_sub(1)
                .ok_or_else(|| invalid("comms.tune", "radio is 1 or 2"))?,
        ))
        .ok_or_else(|| invalid("comms.tune", "radio is 1 or 2"))
}
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommsAudioPanel {
    #[serde(default)]
    pub transmit: Option<u8>,
    #[serde(default)]
    pub receive_com1: Option<bool>,
    #[serde(default)]
    pub receive_com2: Option<bool>,
    #[serde(default)]
    pub nav_ident: Option<bool>,
    #[serde(default)]
    pub intercom: Option<bool>,
    #[serde(default)]
    pub volume: Option<f64>,
    #[serde(default)]
    pub squelch: Option<f64>,
    #[serde(default)]
    pub speech: Option<bool>,
    #[serde(default)]
    pub captions: Option<bool>,
    #[serde(default)]
    pub ptt: Option<bool>,
    #[serde(default)]
    pub intercom_ptt: Option<bool>,
}
impl Command for CommsAudioPanel {
    const NAME: &'static str = "comms.audio_panel";
    const DESCRIPTION: &'static str = "Read or change transmit/receive selects, NAV ident, intercom, volume, squelch, PTT, speech, and captions.";
    type Output = CommsSnapshot;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if self.transmit.is_some_and(|v| v != 1 && v != 2) {
            return Err(invalid(Self::NAME, "transmit is 1 or 2"));
        }
        for v in [self.volume, self.squelch].into_iter().flatten() {
            crate::range(Self::NAME, "level", v, 0.0, 1.0)?;
        }
        if let Some(transmit) = self.transmit.filter(|t| *t != s.comms.panel.transmit) {
            let carrier = |radio: u8| {
                s.comms
                    .radios
                    .get(usize::from(radio.saturating_sub(1)))
                    .and_then(|r| carrier_thirds_hz(r.active_khz))
            };
            let next = carrier(transmit);
            if next != carrier(s.comms.panel.transmit)
                && let Some(next) = next
            {
                s.comms.contacts.remove(&next);
            }
        }
        let p = &mut s.comms.panel;
        macro_rules! patch { ($($field:ident),*) => { $(if let Some(value) = self.$field { p.$field = value; })* } }
        patch!(
            transmit,
            receive_com1,
            receive_com2,
            nav_ident,
            intercom,
            volume,
            squelch,
            speech,
            captions,
            ptt,
            intercom_ptt
        );
        Ok(s.comms_state())
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommsTransmit {
    #[schemars(length(min = 1, max = 1024))]
    pub text: String,
    #[serde(default)]
    pub intercom: bool,
}
impl Command for CommsTransmit {
    const NAME: &'static str = "comms.transmit";
    const DESCRIPTION: &'static str = "Send a typed pilot call on selected COM or intercom; grade airport, runway, phase, reception, and overlap.";
    type Output = Transmission;
    fn run(mut self, s: &mut Session) -> Result<Self::Output, CommandError> {
        self.text = self.text.replace("{callsign}", &s.pilot_callsign().current);
        if !text_ok(&self.text, 1024) {
            return Err(invalid(Self::NAME, "text requires 1..1024 printable bytes"));
        }
        let now = s.sim.state().time_s;
        if s.comms.pending.iter().any(|p| {
            p.end_s > now
                && s.comms
                    .record
                    .events
                    .iter()
                    .any(|e| e.id == p.id && e.speaker == Speaker::Pilot)
        }) {
            return Err(invalid(Self::NAME, "pilot is still transmitting"));
        }
        if !self.intercom && s.sim.failures().radio_failure {
            return Err(invalid(Self::NAME, "COM radio failure"));
        }
        if !s.comms_power().0 {
            return Err(invalid(Self::NAME, "audio panel has no electrical power"));
        }
        let leg = PatternLeg::parse(&self.text);
        let meaning = s.comms.pattern.as_ref().zip(leg).map_or_else(
            || Meaning::PilotCall {
                message: parse_pilot(&self.text, &s.pilot_callsign()),
            },
            |(p, leg)| Meaning::Position {
                airport: p.airport.clone(),
                runway: p.runway.clone(),
                leg,
            },
        );
        let id = if self.intercom {
            if !s.comms.panel.intercom {
                return Err(invalid(Self::NAME, "intercom is off"));
            }
            s.start_local(
                Channel::Intercom,
                Speaker::Pilot,
                self.text,
                meaning,
                EventSource::Pilot,
            )
        } else {
            let selected = s.comms.panel.transmit;
            let (_, one, two) = s.comms_power();
            if !(if selected == 1 { one } else { two }) {
                return Err(invalid(Self::NAME, "selected COM has no electrical power"));
            }
            let khz = s
                .comms
                .radios
                .get(usize::from(selected - 1))
                .map_or(122800, |r| r.active_khz);
            let id = s.start_com(
                khz,
                Speaker::Pilot,
                self.text,
                meaning,
                EventSource::Pilot,
                s.sim.state().position_m,
                25.0,
            );
            s.note_user_keyed(id, khz);
            id
        };
        let grade = if !self.intercom {
            s.grade_call(id, leg)
        } else {
            None
        };
        let event = s
            .comms
            .record
            .events
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| invalid(Self::NAME, "event record is full"))?;
        event.grade = grade;
        Ok(event.clone())
    }
}
#[derive(Debug, Default, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommsListen {}
#[derive(Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct HeardTransmission {
    pub transmission: Option<Transmission>,
    pub pending: usize,
}
impl Command for CommsListen {
    const NAME: &'static str = "comms.listen";
    const DESCRIPTION: &'static str =
        "Consume the next completed heard transmission, with typed meaning; step or wait when empty.";
    type Output = HeardTransmission;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let id = s.comms.heard.pop_front();
        Ok(HeardTransmission {
            transmission: id.and_then(|id| s.comms.record.events.iter().find(|e| e.id == id).cloned()),
            pending: s.comms.pending.len(),
        })
    }
}
#[derive(Debug, Default, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommsFacilities {
    /// Optional bounded FAA-normalized catalogue; omission only lists the active world's stations.
    #[serde(default)]
    #[schemars(length(max = 20000))]
    pub stations: Option<Vec<ComStation>>,
}
impl Command for CommsFacilities {
    const NAME: &'static str = "comms.facilities";
    const DESCRIPTION: &'static str =
        "List region FAA communications, or install a validated offline station catalogue for this session.";
    type Output = CommsSnapshot;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if let Some(stations) = self.stations {
            if stations.len() > 20000 {
                return Err(invalid(Self::NAME, "too many stations"));
            }
            let mut ids = std::collections::BTreeSet::new();
            for st in &stations {
                st.validate().map_err(|e| invalid(Self::NAME, e.to_string()))?;
                if !ids.insert(&st.id) {
                    return Err(invalid(Self::NAME, "duplicate station id"));
                }
            }
            s.comms.extra_stations = stations;
        }
        Ok(s.comms_state())
    }
}
impl Session {
    pub(crate) fn record_lesson_event(&mut self, event: crate::InstrumentEvent) {
        self.record_state_change(
            Meaning::Instrument { event },
            EventSource::Rules,
            "Instrument training state event",
        );
    }
    pub(crate) fn record_scenario_action(
        &mut self,
        action: crate::scenario::ScenarioAction,
        source: EventSource,
    ) {
        self.record_state_change(Meaning::ScenarioAction { action }, source, "Scenario state event");
    }
    fn record_state_change(&mut self, meaning: Meaning, source: EventSource, label: &str) {
        if self.comms.record.events.len() >= MAX_EVENTS {
            self.comms.record.dropped_events += 1;
            return;
        }
        let id = self.comms.next_id;
        self.comms.next_id += 1;
        self.comms.record.events.push(Transmission {
            id,
            time_s: self.sim.state().time_s,
            duration_s: 0.,
            channel: Channel::Intercom,
            speaker: Speaker::Instructor,
            speaker_name: "Instructor".into(),
            frequency_hz: None,
            text: label.into(),
            meaning,
            source,
            reception: Reception::NotReceived,
            reason: Some("Recorded state change; not a spoken transmission".into()),
            grade: None,
        });
    }
    pub(crate) fn selected_com_frequency(&self) -> u32 {
        if self.comms.panel.transmit == 1 {
            self.comms.radios.first().map_or(121500, |r| r.active_khz)
        } else {
            self.comms.radios.get(1).map_or(121500, |r| r.active_khz)
        }
    }
    pub fn flight_record(&self) -> &FlightRecord {
        &self.comms.record
    }
    pub fn comms_state(&self) -> CommsSnapshot {
        if let Some(state) = &self.presentation_comms {
            return state.clone();
        }
        let (powered, com1_powered, com2_powered) = self.comms_power();
        let [com1, com2] = self.comms.radios.clone();
        CommsSnapshot {
            callsign: self.pilot_callsign(),
            adf_khz: self.comms.adf_khz,
            powered,
            com1_powered,
            com2_powered,
            com1,
            com2,
            panel: self.comms.panel.clone(),
            stations: self.stations(),
            traffic: self.traffic_state(),
            expected_call: self.comms.pattern.as_ref().and_then(|p| p.pilot_leg(self)),
        }
    }
    pub(super) fn radio_project(&self, lat: f64, lon: f64) -> Option<[f64; 2]> {
        let r = self.sim.world().region()?;
        flightcraft_world::geo::LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg)
            .ok()?
            .project(lat, lon)
            .ok()
    }
    fn comms_power(&self) -> (bool, bool, bool) {
        let radio_healthy = !self.sim.failures().radio_failure;
        let installation = self
            .aircraft_configuration()
            .map(|c| c.panel.audio_panel.clone())
            .unwrap_or_default();
        let loads = &self.sim.systems().electrical.loads;
        if loads.is_empty() {
            let on = self.sim.systems().controls.master && self.sim.systems().controls.avionics;
            return (on, on && radio_healthy, on && radio_healthy);
        }
        let power = |id: &str| {
            self.sim.systems().controls.master
                && self.sim.systems().controls.avionics
                && loads
                    .iter()
                    .any(|l| l.id == id && l.powered && l.breaker_closed && !l.tripped)
        };
        (
            power(&installation.power_load),
            power(&installation.com1_load) && radio_healthy,
            power(&installation.com2_load) && radio_healthy,
        )
    }
    pub(super) fn stations(&self) -> Vec<ComStation> {
        let mut stations = self
            .sim
            .world()
            .navdata()
            .map(|d| d.comms.clone())
            .unwrap_or_default();
        for st in &self.comms.extra_stations {
            if !stations.iter().any(|s| s.id == st.id) {
                stations.push(st.clone());
            }
        }
        stations
    }
    pub(crate) fn crew_transmit(&mut self, text: String) -> Result<(), CommandError> {
        let carrier = self
            .comms
            .radios
            .get(usize::from(self.comms.panel.transmit.saturating_sub(1)))
            .and_then(|r| carrier_thirds_hz(r.active_khz));
        if self
            .comms
            .pending
            .iter()
            .any(|p| p.channel == Channel::Com && p.carrier == carrier)
        {
            return Err(invalid(
                CommsTransmit::NAME,
                "COM carrier is busy; listen before transmitting",
            ));
        }
        let event = CommsTransmit {
            text,
            intercom: false,
        }
        .run(self)?;
        if let Some(record) = self.comms.record.events.iter_mut().find(|e| e.id == event.id) {
            record.source = EventSource::Rules;
        }
        Ok(())
    }
    pub(crate) fn sync_comms_generation(&mut self) {
        self.comms.generation = self.flight_generation;
    }
    pub(crate) fn sequence_comms(&mut self) {
        if self.comms.generation != self.flight_generation {
            self.lesson = crate::lesson::LessonRuntime::default();
            self.scenario = crate::scenario::ScenarioRuntime {
                generation: self.flight_generation,
                ..Default::default()
            };
            let extra_stations = std::mem::take(&mut self.comms.extra_stations);
            let callsign_overrides = std::mem::take(&mut self.comms.callsign_overrides);
            let saved_callsigns = std::mem::take(&mut self.comms.saved_callsigns);
            let abbreviate = self.comms.abbreviate;
            self.comms = Comms {
                generation: self.flight_generation,
                callsign_overrides,
                saved_callsigns,
                abbreviate,
                extra_stations,
                ..Default::default()
            };
        }
        self.poll_live_weather();
        self.comms
            .record
            .input(self.sim.state().time_s, self.sim.controls());
        self.sync_radio_failure();
        self.finish_transmissions();
        self.comms.record.emergency = self.scenario.state.emergency.clone();
    }
    pub(crate) fn step_with_comms(&mut self, steps: u32) {
        for _ in 0..steps {
            self.sequence_crew();
            self.comms
                .record
                .input(self.sim.state().time_s, self.sim.controls());
            self.tick_live_transition();
            self.sim.step(1);
            self.apply_display_loss_rules();
            let now = self.sim.state().time_s;
            self.finish_transmissions();
            self.sequence_scenario();
            self.sequence_lesson();
            self.sequence_preflight();
            self.sequence_checklist();
            self.sequence_inset();
            self.sequence_procedure_lesson();
            if now - self.comms.last_tick >= 0.25 - 1e-8 {
                let dt = now - self.comms.last_tick;
                self.comms.last_tick = now;
                self.sequence_atc();
                self.update_pattern(dt);
                self.sequence_fleet(dt);
                self.sequence_traffic_calls();
                self.broadcast_weather();
                self.broadcast_ident();
            }
        }
    }
    fn listening(&self, carrier: u32, origin: [f64; 3], range_nm: f64) -> bool {
        let (panel, one, two) = self.comms_power();
        if !panel || self.comms.panel.volume <= 0.0 {
            return false;
        }
        let tuned = self
            .comms
            .radios
            .iter()
            .zip([
                self.comms.panel.receive_com1 && one,
                self.comms.panel.receive_com2 && two,
            ])
            .any(|(r, on)| on && carrier_thirds_hz(r.active_khz) == Some(carrier));
        tuned
            && reception(
                self.sim.world(),
                self.sim.state().position_m,
                origin,
                range_nm,
                self.comms.panel.squelch,
            )
    }
    pub(crate) fn start_local(
        &mut self,
        channel: Channel,
        speaker: Speaker,
        text: String,
        meaning: Meaning,
        source: EventSource,
    ) -> u64 {
        self.start_event(channel, None, speaker, text, meaning, source, None, 0.0)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_com(
        &mut self,
        khz: u32,
        speaker: Speaker,
        text: String,
        meaning: Meaning,
        source: EventSource,
        origin: [f64; 3],
        range_nm: f64,
    ) -> u64 {
        self.start_event(
            Channel::Com,
            carrier_thirds_hz(khz),
            speaker,
            text,
            meaning,
            source,
            Some(origin),
            range_nm,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn start_event(
        &mut self,
        channel: Channel,
        carrier: Option<u32>,
        speaker: Speaker,
        text: String,
        meaning: Meaning,
        source: EventSource,
        origin: Option<[f64; 3]>,
        range_nm: f64,
    ) -> u64 {
        let id = self.comms.next_id;
        self.comms.next_id += 1;
        if self.comms.record.events.len() >= record::MAX_EVENTS {
            self.comms.record.dropped_events += 1;
            return id;
        }
        let now = self.sim.state().time_s;
        let duration = if let Meaning::Ident { morse, .. } = &meaning {
            morse_duration(morse)
        } else if let Meaning::Fleet { call } = &meaning {
            fleet_call_duration(&text, call)
        } else if let Meaning::TrafficAdvisory { voice, .. } | Meaning::TrafficFollowUp { voice, .. } =
            &meaning
        {
            // The same duration the fleet queue held the carrier for.
            controller_voice(*voice).duration_s(&text)
        } else {
            (text.split_whitespace().count() as f64 / 2.7).clamp(1.0, 40.0)
        };
        let eligible = match channel {
            Channel::Com => {
                carrier
                    .zip(origin)
                    .is_some_and(|(f, o)| self.listening(f, o, range_nm))
                    || speaker == Speaker::Pilot
            }
            Channel::Intercom => {
                self.comms_power().0 && self.comms.panel.intercom && self.comms.panel.volume > 0.0
            }
            Channel::Nav => {
                self.comms_power().0 && self.comms.panel.nav_ident && self.comms.panel.volume > 0.0
            }
        };
        let radio =
            (speaker == Speaker::Pilot && channel == Channel::Com).then_some(self.comms.panel.transmit);
        let mut collided = false;
        if let Some(frequency) = carrier {
            for pending in &mut self.comms.pending {
                if pending.carrier == Some(frequency) && pending.end_s > now && (eligible && pending.eligible)
                {
                    pending.collided = true;
                    collided = true;
                }
            }
        }
        let frequency_hz = if channel == Channel::Nav {
            if let Meaning::Ident { ident, .. } = &meaning {
                self.sim
                    .radio_state()
                    .iter()
                    .zip(&self.sim.radios().receivers)
                    .find_map(|(r, tuning)| {
                        (r.ident.as_ref() == Some(ident)).then_some(f64::from(tuning.active_hz))
                    })
            } else {
                None
            }
        } else {
            carrier.map(|v| f64::from(v) / 3.0)
        };
        let local_crew = speaker == Speaker::Crew && channel == Channel::Intercom;
        self.comms.record.events.push(Transmission {
            id,
            time_s: now,
            duration_s: duration,
            channel,
            frequency_hz,
            speaker_name: phraseology::speaker(&speaker, &self.stations()),
            speaker,
            text,
            meaning,
            source,
            reception: if local_crew {
                if eligible {
                    Reception::Heard
                } else {
                    Reception::NotReceived
                }
            } else {
                Reception::Pending
            },
            reason: None,
            grade: None,
        });
        if !local_crew {
            let atc_receiver = if speaker_is_pilot(&self.comms.record.events, id) && channel == Channel::Com {
                let khz = self
                    .comms
                    .radios
                    .get(usize::from(self.comms.panel.transmit.saturating_sub(1)))
                    .map_or(122800, |r| r.active_khz);
                let message = self
                    .comms
                    .record
                    .events
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| parse_pilot(&e.text, &self.pilot_callsign()))
                    .unwrap_or_default();
                self.atc_receiver(&message, khz)
            } else {
                None
            };
            self.comms.pending.push(Pending {
                atc_receiver,
                id,
                end_s: now + duration,
                carrier,
                channel,
                eligible,
                origin,
                range_nm,
                collided,
                radio,
            });
        }
        id
    }
    fn finish_transmissions(&mut self) {
        let now = self.sim.state().time_s;
        // A transmission must remain audible for its entire duration; retuning cannot recover words.
        let mut pending = std::mem::take(&mut self.comms.pending);
        for p in &mut pending {
            let audible = match p.channel {
                Channel::Com => {
                    if let Some(radio) = p.radio {
                        let (panel, one, two) = self.comms_power();
                        panel
                            && (if radio == 1 { one } else { two })
                            && self
                                .comms
                                .radios
                                .get(usize::from(radio - 1))
                                .and_then(|r| carrier_thirds_hz(r.active_khz))
                                == p.carrier
                    } else {
                        p.carrier
                            .zip(p.origin)
                            .is_some_and(|(f, o)| self.listening(f, o, p.range_nm))
                    }
                }
                Channel::Intercom => self.comms_power().0 && self.comms.panel.intercom,
                Channel::Nav => self.comms_power().0 && self.comms.panel.nav_ident,
            };
            let reaches_controller = p.atc_receiver.as_ref().is_none_or(|st| {
                self.station_origin(st).is_some_and(|o| {
                    reception(self.sim.world(), self.sim.state().position_m, o, st.range_nm, 0.)
                })
            });
            p.eligible &= audible && reaches_controller;
        }
        let mut completed = Vec::new();
        for p in pending {
            if p.end_s > now + 1e-8 {
                self.comms.pending.push(p);
                continue;
            }
            if let Some(event) = self.comms.record.events.iter_mut().rev().find(|e| e.id == p.id) {
                event.reception = if !p.eligible {
                    Reception::NotReceived
                } else if p.collided {
                    Reception::Blocked
                } else {
                    Reception::Heard
                };
                event.reason = match event.reception {
                    Reception::Blocked => Some("simultaneous transmissions on the same carrier".into()),
                    Reception::NotReceived => Some(
                        "not tuned, unpowered, muted, out of range, terrain blocked, or retuned during call"
                            .into(),
                    ),
                    _ => None,
                };
                if let Some(grade) = &mut event.grade {
                    grade.received = event.reception == Reception::Heard;
                    grade.passed = grade.airport_correct
                        && grade.runway_correct
                        && grade.frequency_correct
                        && grade.timely
                        && grade.received;
                    if grade.passed {
                        grade.feedback = "Call correct, timely, and received on CTAF.".into();
                    }
                    if !grade.received {
                        grade.feedback = event.reason.clone().unwrap_or_default();
                    }
                }
                completed.push((event.clone(), p.atc_receiver));
                if event.reception == Reception::Heard {
                    if self.comms.heard.len() >= 512 {
                        self.comms.heard.pop_front();
                    }
                    self.comms.heard.push_back(event.id);
                    if event.speaker == Speaker::Pilot
                        && event.grade.as_ref().is_some_and(|g| g.passed)
                        && let Some(pattern) = &mut self.comms.pattern
                        && event
                            .grade
                            .as_ref()
                            .is_some_and(|g| g.expected_leg == pattern.expected)
                    {
                        pattern.phase_called = true;
                    }
                }
            }
        }
        for (event, receiver) in completed {
            // AIM 4-2-4: only a heard controller abbreviation permits the pilot to shorten.
            let callsign = self.pilot_callsign();
            if event.reception == Reception::Heard
                && event.channel == Channel::Com
                && matches!(
                    event.speaker,
                    Speaker::Facility {
                        role: FacilityKind::Tower
                            | FacilityKind::Ground
                            | FacilityKind::Approach
                            | FacilityKind::Departure
                            | FacilityKind::Clearance,
                        ..
                    }
                )
                && words(&event.text).contains(&words(&callsign.abbreviated))
                && !words(&event.text).contains(&words(&callsign.full))
                && let Some(carrier) = event.frequency_hz.map(|hz| (hz * 3.).round() as u32)
            {
                self.comms.contacts.insert(carrier);
            }
            self.completed_atc(&event, receiver);
        }
    }
    fn grade_call(&self, id: u64, leg: Option<PatternLeg>) -> Option<CallGrade> {
        let p = self.comms.pattern.as_ref()?;
        if self
            .stations()
            .iter()
            .any(|st| st.airport == p.airport && st.kind == FacilityKind::Tower)
        {
            return None;
        }
        let event = self.comms.record.events.iter().find(|e| e.id == id)?;
        let expected = p.pilot_leg(self);
        let normalized = words(&event.text);
        let padded = format!(" {normalized} ");
        let airport_correct = padded.contains(&format!(" {} ", words(&p.name)))
            || normalized
                .split_whitespace()
                .any(|t| t == p.airport.to_lowercase());
        let runway_correct = padded.contains(&format!(
            " runway {} ",
            p.runway.trim_start_matches('0').to_lowercase()
        )) || padded.contains(&format!(" runway {} ", p.runway.to_lowercase()))
            || padded.contains(&format!(" runway {} ", runway_words(&p.runway)))
            || padded.contains(&format!(
                " runway {} ",
                runway_words(&p.runway).replace("niner", "nine")
            ));
        let timely = leg.is_some() && leg == expected;
        let selected = self.comms.radios.get(usize::from(self.comms.panel.transmit - 1));
        let frequency_correct =
            selected.and_then(|r| carrier_thirds_hz(r.active_khz)) == carrier_thirds_hz(p.frequency_khz);
        Some(CallGrade {
            airport_correct,
            runway_correct,
            frequency_correct,
            timely,
            received: false,
            expected_leg: expected,
            passed: false,
            feedback: if !frequency_correct {
                "Tune the airport CTAF before calling."
            } else if !airport_correct {
                "Include the correct airport name."
            } else if !runway_correct {
                "Include the correct runway."
            } else if !timely {
                "Call does not match the aircraft's current pattern leg."
            } else {
                "Call queued; grade is final when transmission ends."
            }
            .into(),
        })
    }
    fn sequence_traffic_calls(&mut self) {
        let now = self.sim.state().time_s;
        let Some(call) = self.comms.traffic_calls.front() else {
            return;
        };
        let carrier = carrier_thirds_hz(call.khz);
        if self
            .comms
            .pending
            .iter()
            .any(|p| p.carrier == carrier && p.end_s + 0.5 > now)
        {
            return;
        }
        if let Some(call) = self.comms.traffic_calls.pop_front() {
            self.start_com(
                call.khz,
                call.speaker,
                call.text,
                call.meaning,
                EventSource::Rules,
                call.origin,
                25.0,
            );
        }
    }
    fn broadcast_weather(&mut self) {
        let now = self.sim.state().time_s;
        if now < self.comms.next_weather {
            return;
        }
        self.comms.next_weather = now + 60.0;
        for station in self.stations().into_iter().filter(|s| {
            matches!(
                s.kind,
                FacilityKind::Awos | FacilityKind::Asos | FacilityKind::Atis
            )
        }) {
            let Some([n, e]) =
                self.radio_project(station.position.latitude_deg, station.position.longitude_deg)
            else {
                continue;
            };
            let origin = [n, e, -(station.elevation_m + station.antenna_agl_m)];
            if !carrier_thirds_hz(station.channel_khz)
                .is_some_and(|f| self.listening(f, origin, station.range_nm))
            {
                continue;
            }
            // ATIS and ASOS may share a carrier; the controlled information broadcast takes priority.
            if station.kind != FacilityKind::Atis
                && self
                    .stations()
                    .iter()
                    .any(|s| s.kind == FacilityKind::Atis && s.channel_khz == station.channel_khz)
            {
                continue;
            }
            let (text, meaning) = if station.kind == FacilityKind::Atis {
                if let Some(report) = self.atis_report(&station.airport) {
                    (report.text.clone(), Meaning::Atis { report })
                } else {
                    self.station_weather_report(&station)
                }
            } else {
                self.station_weather_report(&station)
            };
            self.start_com(
                station.channel_khz,
                Speaker::Facility {
                    id: station.id,
                    role: station.kind,
                },
                text,
                meaning,
                EventSource::Rules,
                origin,
                station.range_nm,
            );
        }
    }
    fn broadcast_ident(&mut self) {
        let now = self.sim.state().time_s;
        if now < self.comms.next_ident || !self.comms.panel.nav_ident {
            return;
        }
        self.comms.next_ident = now + 15.0;
        for radio in self.sim.radio_state() {
            if radio.signal_valid
                && let Some(ident) = radio.ident
            {
                let code = morse(&ident);
                self.start_local(
                    Channel::Nav,
                    Speaker::Nav { ident: ident.clone() },
                    format!("NAV ident {ident}: {code}"),
                    Meaning::Ident { ident, morse: code },
                    EventSource::Rules,
                );
            }
        }
        if let Some(khz) = self.comms.adf_khz {
            let station = self
                .sim
                .world()
                .navdata()
                .and_then(|data| {
                    data.navaids.iter().find(|n| {
                        matches!(n.kind, flightcraft_world::navdata::NavaidKind::Ndb)
                            && n.components.iter().any(|c| {
                                c.frequency_hz
                                    .is_some_and(|f| (f - f64::from(khz) * 1000.0).abs() < 1.0)
                            })
                    })
                })
                .cloned();
            if let Some(station) = station
                && let Some([n, e]) =
                    self.radio_project(station.position.latitude_deg, station.position.longitude_deg)
            {
                let altitude = self.sim.world().ground(n, e).height_msl_m + 10.0;
                if reception(
                    self.sim.world(),
                    self.sim.state().position_m,
                    [n, e, -altitude],
                    50.0,
                    self.comms.panel.squelch,
                ) {
                    let code = morse(&station.ident);
                    let id = self.start_local(
                        Channel::Nav,
                        Speaker::Nav {
                            ident: station.ident.clone(),
                        },
                        format!("ADF ident {}: {}", station.ident, code),
                        Meaning::Ident {
                            ident: station.ident,
                            morse: code,
                        },
                        EventSource::Rules,
                    );
                    if let Some(event) = self.comms.record.events.iter_mut().find(|e| e.id == id) {
                        event.frequency_hz = Some(f64::from(khz) * 1000.0);
                    }
                }
            }
        }
    }
}
/// VHF line of sight: 100 m terrain samples, a 4/3-Earth curvature bulge, and a bounded range.
/// Missing geographic terrain is unavailable reception, never transparent terrain.
pub fn reception(
    world: &flightcraft_world::World,
    receiver: [f64; 3],
    transmitter: [f64; 3],
    range_nm: f64,
    squelch: f64,
) -> bool {
    if !receiver.iter().chain(&transmitter).all(|v| v.is_finite())
        || !range_nm.is_finite()
        || range_nm <= 0.0
        || !squelch.is_finite()
    {
        return false;
    }
    let [rn, re, rd] = receiver;
    let [tn, te, td] = transmitter;
    let distance = libm::hypot(tn - rn, te - re);
    if distance > range_nm * 1852.0 * (1.0 - squelch.clamp(0.0, 1.0) * 0.25) {
        return false;
    }
    let steps = (distance / 100.0).ceil().clamp(1.0, 6000.0) as u32;
    for i in 1..steps {
        let f = f64::from(i) / f64::from(steps);
        let n = rn + (tn - rn) * f;
        let e = re + (te - re) * f;
        let height = -rd + (rd - td) * f;
        let earth = distance * distance * f * (1.0 - f) / (2.0 * 8_495_000.0);
        let terrain = if world.region().is_some() {
            let Some(g) = world.terrain_ground(n, e) else {
                return false;
            };
            g.height_msl_m
        } else {
            world.ground(n, e).height_msl_m
        };
        if terrain + earth > height {
            return false;
        }
    }
    true
}
fn words(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn runway_words(s: &str) -> String {
    phraseology::runway(s)
}

pub fn morse(ident: &str) -> String {
    ident
        .to_uppercase()
        .chars()
        .filter_map(|c| match c {
            'A' => Some(".-"),
            'B' => Some("-..."),
            'C' => Some("-.-."),
            'D' => Some("-.."),
            'E' => Some("."),
            'F' => Some("..-."),
            'G' => Some("--."),
            'H' => Some("...."),
            'I' => Some(".."),
            'J' => Some(".---"),
            'K' => Some("-.-"),
            'L' => Some(".-.."),
            'M' => Some("--"),
            'N' => Some("-."),
            'O' => Some("---"),
            'P' => Some(".--."),
            'Q' => Some("--.-"),
            'R' => Some(".-."),
            'S' => Some("..."),
            'T' => Some("-"),
            'U' => Some("..-"),
            'V' => Some("...-"),
            'W' => Some(".--"),
            'X' => Some("-..-"),
            'Y' => Some("-.--"),
            'Z' => Some("--.."),
            '0' => Some("-----"),
            '1' => Some(".----"),
            '2' => Some("..---"),
            '3' => Some("...--"),
            '4' => Some("....-"),
            '5' => Some("....."),
            '6' => Some("-...."),
            '7' => Some("--..."),
            '8' => Some("---.."),
            '9' => Some("----."),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn morse_duration(code: &str) -> f64 {
    code.chars()
        .map(|c| match c {
            '.' => 2.0,
            '-' => 4.0,
            ' ' => 2.0,
            _ => 0.0,
        })
        .sum::<f64>()
        * 0.1
}
#[cfg(test)]
mod tests;

fn speaker_is_pilot(events: &[Transmission], id: u64) -> bool {
    events
        .iter()
        .rev()
        .find(|e| e.id == id)
        .is_some_and(|e| e.speaker == Speaker::Pilot)
}
