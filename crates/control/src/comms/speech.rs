//! Constrained recognition post-pass. Numeric mistakes are preserved for readback grading.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SpeechDraft {
    pub id: u64,
    pub recognized: String,
    pub normalized: String,
    pub message: PilotMessage,
    pub retry: bool,
    pub prompt: String,
    pub intercom: bool,
    pub radio: u8,
    pub channel_khz: u32,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommsRecognize {
    /// Optional client correlation ID for asynchronous browser/worker transports.
    #[serde(default)]
    pub request_id: Option<u64>,
    #[schemars(length(max = 1024))]
    pub text: String,
    pub confidence: f32,
    #[serde(default)]
    pub intercom: bool,
}
impl Command for CommsRecognize {
    const NAME: &'static str = "comms.recognize";
    const DESCRIPTION: &'static str = "Normalize recognized speech and record an ungraded preview; low confidence asks the student to say again.";
    type Output = SpeechDraft;
    fn run(self, s: &mut Session) -> Result<SpeechDraft, CommandError> {
        if self.text.len() > 1024
            || self.text.chars().any(char::is_control)
            || !self.confidence.is_finite()
            || !(0.0..=1.0).contains(&self.confidence)
        {
            return Err(invalid(
                Self::NAME,
                "recognition requires printable text and confidence from zero to one",
            ));
        }
        if s.comms.record.events.len() >= MAX_EVENTS {
            return Err(invalid(Self::NAME, "flight event record is full"));
        }
        let state = s.comms_state();
        let callsign = state.callsign.clone();
        let (normalized, grammar) = normalize_speech(&self.text, self.intercom, s);
        let message = parse_pilot(&normalized, &callsign);
        let retry = self.confidence < 0.65 || !grammar;
        let id = s.comms.next_id;
        s.comms.next_id += 1;
        let radio = state.panel.transmit;
        let channel_khz = if radio == 1 {
            state.com1.active_khz
        } else {
            state.com2.active_khz
        };
        let draft = SpeechDraft {
            id: self.request_id.unwrap_or(id),
            recognized: self.text.clone(),
            normalized: normalized.clone(),
            message,
            retry,
            prompt: if retry {
                "Say again. Hold push-to-talk and retry."
            } else {
                "Check what you said, then transmit."
            }
            .into(),
            intercom: self.intercom,
            radio,
            channel_khz,
        };
        s.comms.record.events.push(Transmission {
            id,
            time_s: s.sim.state().time_s,
            duration_s: 0.0,
            channel: if self.intercom {
                Channel::Intercom
            } else {
                Channel::Com
            },
            frequency_hz: (!self.intercom).then_some(f64::from(channel_khz) * 1000.0),
            speaker: Speaker::Pilot,
            speaker_name: "You · recognized, awaiting review".into(),
            text: if self.text.is_empty() {
                "No speech recognized".into()
            } else {
                self.text
            },
            meaning: Meaning::Recognition { normalized, retry },
            source: EventSource::Pilot,
            reception: Reception::Heard,
            reason: Some(draft.prompt.clone()),
            grade: None,
        });
        s.comms.speech_draft = Some(draft.clone());
        Ok(draft)
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommsConfirmSpeech {
    pub id: u64,
}
impl Command for CommsConfirmSpeech {
    const NAME: &'static str = "comms.confirm_speech";
    const DESCRIPTION: &'static str = "Transmit a reviewed speech draft through comms.transmit; refuse stale routes and low-confidence recognition.";
    type Output = Transmission;
    fn run(self, s: &mut Session) -> Result<Transmission, CommandError> {
        let draft = s
            .comms
            .speech_draft
            .as_ref()
            .filter(|d| d.id == self.id)
            .cloned()
            .ok_or_else(|| invalid(Self::NAME, "speech preview has expired; say again"))?;
        if draft.retry {
            return Err(invalid(Self::NAME, "say again before transmitting"));
        }
        let state = s.comms_state();
        let khz = if state.panel.transmit == 1 {
            state.com1.active_khz
        } else {
            state.com2.active_khz
        };
        if !draft.intercom && (draft.radio != state.panel.transmit || draft.channel_khz != khz) {
            return Err(invalid(
                Self::NAME,
                "transmit selection changed; say again on the selected COM",
            ));
        }
        let mut event = CommsTransmit {
            text: draft.normalized.clone(),
            intercom: draft.intercom,
        }
        .run(s)?;
        s.comms.speech_draft = None;
        if draft.intercom && intercom_command(s, &draft.normalized).is_err() {
            let reason = "Unable to accept that crew command. Check the crew plan and control exchange.";
            event.reason = Some(reason.into());
            if let Some(record) = s.comms.record.events.iter_mut().find(|e| e.id == event.id) {
                record.reason = event.reason.clone();
            }
            s.start_local(
                Channel::Intercom,
                Speaker::Crew,
                reason.into(),
                Meaning::Crew { critical: false },
                EventSource::Rules,
            );
        }
        Ok(event)
    }
}
fn intercom_command(s: &mut Session, text: &str) -> Result<(), CommandError> {
    use crate::{ControlOwner, CrewHandover, CrewIntent, PilotIntent};
    if text == "go around" {
        let result = CrewIntent {
            intent: PilotIntent::GoAround {},
        }
        .run(s)?;
        if let Some(reason) = result.refusal {
            return Err(invalid("crew.intent", reason));
        }
    } else {
        let to = if text == "your controls" {
            ControlOwner::Crew
        } else {
            ControlOwner::Pilot
        };
        let state = s.crew_state();
        // A pilot utterance never impersonates the crew's acknowledgement.
        let confirm = state.pending_handover == Some(to) && to == ControlOwner::Pilot;
        CrewHandover {
            to,
            actor: ControlOwner::Pilot,
            confirm,
        }
        .run(s)?;
    }
    Ok(())
}
/// All vocabulary comes from a small grammar, aircraft identity, NASR calls, and current readbacks.
/// Fuzzy correction uses unique proper-name or heard-readback tokens at edit distance one.
/// Numeric words are never corrected to an expected answer; missing words are never inserted.
pub fn normalize_speech(text: &str, intercom: bool, s: &Session) -> (String, bool) {
    let canonical = |v: &str| {
        words(v)
            .split_whitespace()
            .map(|w| match w {
                "niner" => "nine",
                "tree" => "three",
                "fife" => "five",
                "fower" => "four",
                "juliett" => "juliet",
                "alfa" => "alpha",
                w => w,
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    // Keep decimal punctuation for the shared numeric parser.
    let mut decimal_text = String::new();
    let mut chars = text.chars().peekable();
    let mut previous_digit = false;
    while let Some(c) = chars.next() {
        if c == '.' && previous_digit && chars.peek().is_some_and(char::is_ascii_digit) {
            decimal_text.push_str(" point ");
        } else {
            decimal_text.push(c);
        }
        previous_digit = c.is_ascii_digit();
    }
    let mut text = canonical(&decimal_text).replace("clear of the runway", "clear of runway");
    for (split, joined) in [
        ("cross wind", "crosswind"),
        ("down wind", "downwind"),
        ("take off", "takeoff"),
    ] {
        text = format!(" {text} ")
            .replace(&format!(" {split} "), &format!(" {joined} "))
            .trim()
            .to_owned();
    }
    if intercom {
        let valid = matches!(
            text.as_str(),
            "your controls" | "my controls" | "i have the controls" | "go around"
        );
        return (text, valid);
    }
    let identity = s.comms_state().callsign;
    let mut proper = std::collections::BTreeSet::<String>::new();
    let mut add = |v: &str| {
        proper.extend(canonical(v).split_whitespace().map(str::to_owned));
    };
    for v in [
        &identity.full,
        &identity.abbreviated,
        &identity.registration,
        &identity.spoken_type,
        &identity.registration.trim_start_matches('N').to_owned(),
    ] {
        add(v);
    }
    for st in s.comms_state().stations {
        add(&phraseology::facility(&st));
        add(&st.airport);
    }
    // Join an ASR space only when the exact joined token exists in pack/NASR
    // names. This changes spelling, not the identity or any numeric value.
    let tokens: Vec<_> = text.split_whitespace().collect();
    let mut joined = Vec::new();
    let mut at = 0;
    while let Some(word) = tokens.get(at) {
        if let Some(next) = tokens.get(at + 1) {
            let combined = format!("{word}{next}");
            if combined.len() >= 5
                && proper.contains(&combined)
                && combined.bytes().all(|b| b.is_ascii_alphabetic())
            {
                joined.push(combined);
                at += 2;
                continue;
            }
        }
        joined.push((*word).to_owned());
        at += 1;
    }
    text = joined.join(" ");
    let grammar = "november zero oh one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty thirty forty fifty sixty seventy eighty ninety hundred thousand point decimal alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike oscar papa quebec romeo sierra tango uniform victor whiskey xray x ray yankee zulu with information request requesting taxi taxiing to the runway hold holding short of cleared for takeoff land landing departing departure ready vfr ifr clearance delivery inbound outbound straight in left right center downwind crosswind base final traffic closed pattern enter entering report reporting number following extending extend will call your turn turning going go around clear contact tower ground approach control maintain climb climbing descend descending altitude leaving level heading fly squawk squawking as filed expect minutes after frequency airport sight visual ils rnav vor loc localizer ndb flight following bravo transition remain remaining outside into class at and this is on parking north south east west miles from negative unable roger wilco good morning afternoon evening";
    let mut allowed: std::collections::BTreeSet<String> =
        grammar.split_whitespace().map(str::to_owned).collect();
    let mut expected = std::collections::BTreeSet::<String>::new();
    for p in s.atc_state().pending_instructions.into_iter().filter(|p| p.heard) {
        expected.extend(
            canonical(&s.atc_readback(&p.instruction))
                .split_whitespace()
                .map(str::to_owned),
        );
    }
    if s.atc_state().closed_traffic {
        // An accepted closed-traffic clearance makes pattern position reports
        // part of the current call set. Numeric slots remain excluded below.
        expected.extend(["crosswind", "downwind", "base", "final"].map(str::to_owned));
    }
    allowed.extend(expected.iter().cloned());
    text = pattern_boundaries(&text, &expected, &proper);
    let numeric = "zero oh one two three four five six seven eight nine niner tree fife ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty thirty forty fifty sixty seventy eighty ninety hundred thousand left right center negative unable not";
    let mut valid = !text.is_empty() && text.len() <= 1024;
    let mut result = Vec::new();
    for word in text.split_whitespace().take(160) {
        if (word.len() == 1 && word.bytes().all(|b| b.is_ascii_alphabetic()))
            || allowed.contains(word)
            || proper.contains(word)
            || word.bytes().all(|b| b.is_ascii_digit())
            || {
                let runway = word.trim_end_matches(['l', 'r', 'c']);
                runway.len() == 2 && runway.parse::<u8>().is_ok_and(|v| (1..=36).contains(&v))
            }
        {
            result.push(word.to_owned());
        } else {
            let candidates: std::collections::BTreeSet<_> = proper
                .iter()
                .chain(expected.iter())
                .filter(|v| {
                    v.len() >= 5
                        && word.len() >= 5
                        && v.bytes().all(|b| b.is_ascii_alphabetic())
                        && !numeric.split_whitespace().any(|n| n == v.as_str())
                        && result.last().is_none_or(|last| last != "information")
                        && edit_one(word, v)
                })
                .collect();
            if candidates.len() == 1
                && let Some(candidate) = candidates.first()
            {
                result.push((*candidate).clone());
            } else {
                valid = false;
                result.push(word.to_owned());
            }
        }
    }
    let mut normalized = result.join(" ");
    // Match the pack's renderer (which spells 9 Niner) without changing numeric slot values.
    for form in [&identity.full, &identity.abbreviated] {
        normalized = normalized.replace(&canonical(form), &words(form));
    }
    let parsed = parse_pilot(&normalized, &identity);
    let has_call = parsed.request.is_some()
        || parsed.runway.is_some()
        || parsed.heading_deg.is_some()
        || parsed.altitude_ft.is_some()
        || parsed.squawk.is_some()
        || parsed.channel_khz.is_some()
        || normalized.contains("extending downwind")
        || normalized.contains("taxi to parking")
        || normalized.contains("remaining outside");
    // Reject malformed numeric slots instead of treating them as open dictation.
    for (key, present) in [
        ("runway", parsed.runway.is_some()),
        ("heading", parsed.heading_deg.is_some()),
        ("squawk", parsed.squawk.is_some()),
        ("maintain", parsed.altitude_ft.is_some()),
        ("information", parsed.information.is_some()),
        ("point", parsed.channel_khz.is_some()),
    ] {
        if normalized.split_whitespace().any(|w| w == key)
            && !present
            && !(key == "runway" && parsed.request == Some(PilotRequest::ClearRunway))
        {
            valid = false;
        }
    }
    (normalized, valid && has_call && parsed.callsign_present)
}
/// Repair one spelling edit across an ASR token boundary only for an expected
/// pattern leg. The spoken direction must be present verbatim; slots, omitted
/// directions, and negation are never inferred or changed.
fn pattern_boundaries(
    text: &str,
    expected: &std::collections::BTreeSet<String>,
    proper: &std::collections::BTreeSet<String>,
) -> String {
    let tokens: Vec<_> = text.split_whitespace().collect();
    let mut result = Vec::new();
    let mut at = 0;
    while let Some(word) = tokens.get(at) {
        let mut candidates = std::collections::BTreeSet::new();
        if !proper.contains(*word) && word.bytes().all(|b| b.is_ascii_alphabetic()) {
            for direction in ["left", "right"] {
                if !word.starts_with(direction) || word.len() <= direction.len() {
                    continue;
                }
                for leg in ["crosswind", "downwind", "base", "final"] {
                    if !expected.contains(leg) {
                        continue;
                    }
                    let joined = format!("{direction}{leg}");
                    if *word == joined || edit_one(word, &joined) {
                        candidates.insert((1, format!("{direction} {leg}")));
                    }
                    if let Some(next) = tokens.get(at + 1)
                        && next.bytes().all(|b| b.is_ascii_alphabetic())
                    {
                        let heard = format!("{word}{next}");
                        if heard == joined || edit_one(&heard, &joined) {
                            candidates.insert((2, format!("{direction} {leg}")));
                        }
                    }
                }
            }
        }
        if candidates.len() == 1
            && let Some((consumed, phrase)) = candidates.first()
        {
            result.push(phrase.clone());
            at += *consumed;
        } else {
            result.push((*word).to_owned());
            at += 1;
        }
    }
    result.join(" ")
}

fn edit_one(a: &str, b: &str) -> bool {
    let mut a = a.bytes().peekable();
    let mut b = b.bytes().peekable();
    let mut edits = 0;
    while let (Some(x), Some(y)) = (a.peek(), b.peek()) {
        if x == y {
            a.next();
            b.next();
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        match a.len().cmp(&b.len()) {
            std::cmp::Ordering::Greater => {
                a.next();
            }
            std::cmp::Ordering::Less => {
                b.next();
            }
            std::cmp::Ordering::Equal => {
                a.next();
                b.next();
            }
        }
    }
    edits + a.count() + b.count() == 1
}

#[cfg(test)]
mod tests;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommsDiscardSpeech {
    pub id: u64,
}
impl Command for CommsDiscardSpeech {
    const NAME: &'static str = "comms.discard_speech";
    const DESCRIPTION: &'static str = "Discard a recognition draft without transmitting or grading; retain its ungraded transcript evidence.";
    type Output = bool;
    fn run(self, s: &mut Session) -> Result<bool, CommandError> {
        let discarded = s.comms.speech_draft.as_ref().is_some_and(|d| d.id == self.id);
        if discarded {
            s.comms.speech_draft = None;
        }
        Ok(discarded)
    }
}
