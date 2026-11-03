//! Civil GA callsigns: manufacturer/type plus registration, with channel-local first contact.
use super::*;
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PilotCallsign {
    pub overridden: bool,
    pub registration: String,
    pub spoken_type: String,
    pub full: String,
    pub abbreviated: String,
    pub current: String,
}
pub fn spoken_registration(registration: &str) -> String {
    registration
        .chars()
        .filter_map(|c| {
            Some(match c {
                '0' => "Zero",
                '1' => "One",
                '2' => "Two",
                '3' => "Three",
                '4' => "Four",
                '5' => "Five",
                '6' => "Six",
                '7' => "Seven",
                '8' => "Eight",
                '9' => "Niner",
                'A' => "Alpha",
                'B' => "Bravo",
                'C' => "Charlie",
                'D' => "Delta",
                'E' => "Echo",
                'F' => "Foxtrot",
                'G' => "Golf",
                'H' => "Hotel",
                'I' => "India",
                'J' => "Juliett",
                'K' => "Kilo",
                'L' => "Lima",
                'M' => "Mike",
                'N' => "November",
                'O' => "Oscar",
                'P' => "Papa",
                'Q' => "Quebec",
                'R' => "Romeo",
                'S' => "Sierra",
                'T' => "Tango",
                'U' => "Uniform",
                'V' => "Victor",
                'W' => "Whiskey",
                'X' => "X-ray",
                'Y' => "Yankee",
                'Z' => "Zulu",
                _ => return None,
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn forms(registration: &str, kind: &str) -> (String, String) {
    // US pilots may replace November with the aircraft type. Other nationality prefixes remain.
    let suffix = registration
        .strip_prefix('N')
        .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(registration);
    let short = suffix
        .chars()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    (
        format!("{kind} {}", spoken_registration(suffix)),
        format!("{kind} {}", spoken_registration(&short)),
    )
}
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommsCallsign {
    #[serde(default)]
    pub registration: Option<String>,
    #[serde(default)]
    pub spoken_type: Option<String>,
    /// Disable to retain full identification; ATC abbreviations require controller initiation.
    #[serde(default)]
    pub abbreviate_after_contact: Option<bool>,
    #[serde(default)]
    pub reset_to_pack: bool,
}
impl Command for CommsCallsign {
    const NAME: &'static str = "comms.callsign";
    const DESCRIPTION: &'static str = "Read aircraft radio identity, override registration/type, or opt into abbreviation after first contact.";
    type Output = PilotCallsign;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if self.registration.is_none()
            && self.spoken_type.is_none()
            && self.abbreviate_after_contact.is_none()
            && !self.reset_to_pack
        {
            return Ok(s.pilot_callsign());
        }
        let current = s.pilot_callsign();
        let registration = self.registration.unwrap_or(current.registration);
        let kind = self.spoken_type.unwrap_or(current.spoken_type);
        if !(2..=12).contains(&registration.len())
            || !registration
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
            || !text_ok(&kind, 64)
            || !kind
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
        {
            return Err(invalid(
                Self::NAME,
                "registration requires 2..12 uppercase characters; spoken type requires 1..64 simple characters",
            ));
        }
        s.comms.callsign_override = if self.reset_to_pack {
            None
        } else {
            Some((registration, kind))
        };
        if let Some(v) = self.abbreviate_after_contact {
            s.comms.abbreviate = v;
        }
        s.comms.contacts.clear();
        Ok(s.pilot_callsign())
    }
}
impl Session {
    pub fn pilot_callsign(&self) -> PilotCallsign {
        let identity = &self.sim().aircraft().identity;
        let (registration, kind) = self.comms.callsign_override.clone().unwrap_or_else(|| {
            (
                if identity.registration.is_empty() {
                    "N1234B".into()
                } else {
                    identity.registration.clone()
                },
                if identity.spoken_type.is_empty() {
                    identity
                        .display_name
                        .split_whitespace()
                        .next()
                        .unwrap_or("Aircraft")
                        .to_owned()
                } else {
                    identity.spoken_type.clone()
                },
            )
        });
        let (full, abbreviated) = forms(&registration, &kind);
        let carrier = self
            .comms
            .radios
            .get(usize::from(self.comms.panel.transmit.saturating_sub(1)))
            .and_then(|r| carrier_thirds_hz(r.active_khz));
        let contacted = carrier.is_some_and(|c| self.comms.contacts.contains(&c));
        let current = if self.comms.abbreviate && contacted {
            abbreviated.clone()
        } else {
            full.clone()
        };
        PilotCallsign {
            overridden: self.comms.callsign_override.is_some(),
            registration,
            spoken_type: kind,
            full,
            abbreviated,
            current,
        }
    }
}
