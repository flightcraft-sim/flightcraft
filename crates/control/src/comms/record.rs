use super::*;
use flightcraft_sim::Controls;

pub const MAX_EVENTS: usize = 20000;
pub const MAX_INPUTS: usize = 100000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Com,
    Intercom,
    Nav,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Speaker {
    Pilot,
    Instructor,
    Crew,
    Facility { id: String, role: FacilityKind },
    Traffic { callsign: String },
    Nav { ident: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    Rules,
    Scenario,
    #[serde(rename = "scenario (ai-written)")]
    ScenarioAi,
    Ai,
    Pilot,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Meaning {
    Instrument {
        event: crate::InstrumentEvent,
    },
    /// Local recognition evidence, shown before the student approves a transmission. Never graded.
    Recognition {
        normalized: String,
        retry: bool,
    },
    Crew {
        critical: bool,
    },
    ScenarioAction {
        action: crate::scenario::ScenarioAction,
    },
    Scenario {
        instruction: crate::scenario::Instruction,
    },
    Atis {
        report: AtisReport,
    },
    Atc {
        instruction: AtcInstruction,
    },
    PilotCall {
        message: PilotMessage,
    },
    Compliance {
        grade: ComplianceGrade,
    },
    InstructorFeedback {
        message: String,
    },
    LightGun {
        signal: LightGunState,
    },
    RadarContact {
        station_id: String,
    },
    TrafficAdvisory {
        callsign: String,
        clock: u8,
        miles: f64,
        altitude_ft: u32,
    },
    Position {
        airport: String,
        runway: String,
        leg: PatternLeg,
    },
    LiveWeather {
        observation: Box<crate::LiveObservation>,
    },
    Weather {
        airport: String,
        wind_from_deg: f64,
        wind_speed_mps: f64,
        visibility_m: Option<f64>,
        ceiling_agl_m: Option<f64>,
        altimeter_pa: f64,
        #[serde(default)]
        temperature_c: f64,
        #[serde(default)]
        dewpoint_c: f64,
        #[serde(default)]
        layers: Vec<ReportedLayer>,
        #[serde(default)]
        obscuration: flightcraft_world::Obscuration,
        #[serde(default)]
        precipitation: flightcraft_world::Precipitation,
    },
    Ident {
        ident: String,
        morse: String,
    },
    Cue {
        airport: String,
        leg: PatternLeg,
        message: String,
    },
    Text,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Reception {
    Pending,
    Heard,
    Blocked,
    NotReceived,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallGrade {
    pub airport_correct: bool,
    pub runway_correct: bool,
    pub frequency_correct: bool,
    pub timely: bool,
    pub received: bool,
    pub expected_leg: Option<PatternLeg>,
    pub passed: bool,
    pub feedback: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transmission {
    pub id: u64,
    pub time_s: f64,
    pub duration_s: f64,
    pub channel: Channel,
    pub frequency_hz: Option<f64>,
    pub speaker: Speaker,
    /// Rendered at emission so saved debriefs retain the imported radio call.
    #[serde(default)]
    pub speaker_name: String,
    pub text: String,
    pub meaning: Meaning,
    pub source: EventSource,
    pub reception: Reception,
    pub reason: Option<String>,
    pub grade: Option<CallGrade>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordedInput {
    pub time_s: f64,
    pub controls: Controls,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlightRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lesson: Option<crate::LessonDebrief>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emergency: Option<crate::scenario::EmergencyDebrief>,
    #[schemars(range(min = 1, max = 1))]
    pub version: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 2000))]
    pub live_weather: Vec<crate::RecordedLiveWeather>,
    pub events: Vec<Transmission>,
    pub inputs: Vec<RecordedInput>,
    pub dropped_events: u64,
    pub dropped_inputs: u64,
}
impl Default for FlightRecord {
    fn default() -> Self {
        Self {
            emergency: None,
            lesson: None,
            version: 1,
            live_weather: Vec::new(),
            events: Vec::new(),
            inputs: Vec::new(),
            dropped_events: 0,
            dropped_inputs: 0,
        }
    }
}
impl FlightRecord {
    /// Bounded import for debrief tools; this does not execute events or touch physics.
    pub fn from_json(text: &str) -> Result<Self, CommandError> {
        if text.len() > 128 * 1024 * 1024 {
            return Err(invalid("flight.events", "record exceeds 128 MiB"));
        }
        let record: Self = serde_json::from_str(text).map_err(|e| invalid("flight.events", e.to_string()))?;
        if record.version != 1 || record.events.len() > MAX_EVENTS || record.inputs.len() > MAX_INPUTS {
            return Err(invalid("flight.events", "record count exceeds bounds"));
        }
        if let Some(debrief) = &record.lesson {
            debrief.validate()?;
        }
        if record.live_weather.len() > 2000 {
            return Err(invalid("flight.events", "too many recorded weather applications"));
        }
        let mut previous_time = 0.;
        for weather in &record.live_weather {
            weather.validate().map_err(CommandError::Failed)?;
            if weather.time_s < previous_time {
                return Err(invalid(
                    "flight.events",
                    "weather applications are not in time order",
                ));
            }
            previous_time = weather.time_s;
        }
        if let Some(debrief) = &record.emergency {
            debrief.validate()?;
        }
        let mut previous = None;
        let mut time = 0.0;
        for event in &record.events {
            if !event.time_s.is_finite()
                || event.time_s < time
                || previous.is_some_and(|id| event.id <= id)
                || !event.duration_s.is_finite()
                || !(0.0..=120.0).contains(&event.duration_s)
                || !text_ok(
                    &event.text,
                    if matches!(event.meaning, Meaning::LiveWeather { .. }) {
                        8192
                    } else {
                        1024
                    },
                )
                || (!event.speaker_name.is_empty() && !text_ok(&event.speaker_name, 128))
                || event
                    .frequency_hz
                    .is_some_and(|f| !f.is_finite() || !(10000.0..=140e6).contains(&f))
            {
                return Err(invalid(
                    "flight.events",
                    "invalid event ordering, time, text, or frequency",
                ));
            }
            if let Meaning::LiveWeather { observation } = &event.meaning {
                observation.validate().map_err(CommandError::Failed)?;
            }
            // Meaning contains numeric observations; serde_json rejects nonfinite numbers.
            previous = Some(event.id);
            time = event.time_s;
        }
        time = 0.0;
        for input in &record.inputs {
            let c = input.controls;
            if !input.time_s.is_finite()
                || input.time_s < time
                || [c.elevator, c.aileron, c.rudder]
                    .iter()
                    .any(|v| !v.is_finite() || !(-1.0..=1.0).contains(v))
                || [c.throttle, c.flaps, c.brake]
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return Err(invalid("flight.events", "invalid recorded input"));
            }
            time = input.time_s;
        }
        Ok(record)
    }
    pub(super) fn input(&mut self, time_s: f64, controls: Controls) {
        if self.inputs.last().is_some_and(|i| i.controls == controls) {
            return;
        }
        if self.inputs.len() >= MAX_INPUTS {
            self.dropped_inputs += 1;
            return;
        }
        self.inputs.push(RecordedInput { time_s, controls });
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct FlightEvents {
    #[serde(default)]
    pub after: u64,
    #[serde(default = "page_size")]
    #[schemars(range(min = 1, max = 1000))]
    pub limit: usize,
    #[serde(default)]
    pub input_offset: usize,
}
impl Default for FlightEvents {
    fn default() -> Self {
        Self {
            after: 0,
            limit: 100,
            input_offset: 0,
        }
    }
}
fn page_size() -> usize {
    100
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct EventPage {
    pub events: Vec<Transmission>,
    pub inputs: Vec<RecordedInput>,
    pub next: u64,
    pub next_input_offset: usize,
    pub pending: usize,
    pub dropped_events: u64,
    pub dropped_inputs: u64,
}
impl Command for FlightEvents {
    const NAME: &'static str = "flight.events";
    const DESCRIPTION: &'static str =
        "Page typed flight transmissions and recorded control changes; cursors are flight-local.";
    type Output = EventPage;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if !(1..=1000).contains(&self.limit) {
            return Err(invalid(Self::NAME, "limit is 1..1000"));
        }
        let r = &s.comms.record;
        let events: Vec<_> = r
            .events
            .iter()
            .filter(|e| e.id > self.after)
            .take(self.limit)
            .cloned()
            .collect();
        let inputs: Vec<_> = r
            .inputs
            .iter()
            .skip(self.input_offset)
            .take(self.limit)
            .cloned()
            .collect();
        Ok(EventPage {
            next: events.last().map_or(self.after, |e| e.id),
            next_input_offset: self.input_offset.saturating_add(inputs.len()),
            events,
            inputs,
            pending: s.comms.pending.len(),
            dropped_events: r.dropped_events,
            dropped_inputs: r.dropped_inputs,
        })
    }
}
