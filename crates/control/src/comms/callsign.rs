//! Civil GA callsigns: manufacturer/type plus registration, with channel-local first contact.
use super::*;
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PilotCallsign {
    pub overridden: bool,
    pub registration: String,
    pub spoken_type: String,
    #[serde(default)]
    pub not_us: bool,
    pub full: String,
    pub abbreviated: String,
    pub current: String,
}

#[cfg(test)]
#[path = "callsign_tests.rs"]
mod tests;

pub use super::phraseology::spoken_registration;

/// A typed, pilot-readable reason for rejecting a callsign. Failed changes are atomic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum CallsignError {
    #[error("A US registration must start with N.")]
    UsPrefix,
    #[error("An N-number requires 1 to 5 characters after N.")]
    UsLength,
    #[error("can't start with zero")]
    LeadingZero,
    #[error("the letters I and O aren't used")]
    ExcludedLetters,
    #[error("letters only at the end")]
    LetterPosition,
    #[error("Use no more than two letters at the end.")]
    TooManyLetters,
    #[error("Use only letters A–Z and digits 0–9 in an N-number.")]
    UsCharacters,
    #[error(
        "Use a prefix and suffix separated by one hyphen, such as C-GABC or G-ABCD (up to 12 characters)."
    )]
    ForeignFormat,
    #[error("Spoken type requires 1 to 64 letters, digits, spaces, or hyphens.")]
    SpokenType,
    #[error("Saved callsign preferences exceed their size limit or contain an invalid variant.")]
    Preferences,
}

/// Normalize and validate the FAA N-number or explicitly selected foreign format.
pub fn validate_registration(input: &str, not_us: bool) -> Result<String, CallsignError> {
    let value = input.to_ascii_uppercase();
    if not_us {
        let Some((prefix, suffix)) = value.split_once('-') else {
            return Err(CallsignError::ForeignFormat);
        };
        if value.len() > 12
            || prefix.is_empty()
            || suffix.is_empty()
            || !prefix
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            || !prefix.bytes().any(|c| c.is_ascii_uppercase())
            || !suffix
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(CallsignError::ForeignFormat);
        }
        return Ok(value);
    }
    let suffix = value.strip_prefix('N').ok_or(CallsignError::UsPrefix)?;
    if !(1..=5).contains(&suffix.len()) {
        return Err(CallsignError::UsLength);
    }
    if suffix.starts_with('0') {
        return Err(CallsignError::LeadingZero);
    }
    if suffix.contains(['I', 'O']) {
        return Err(CallsignError::ExcludedLetters);
    }
    if !suffix
        .bytes()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Err(CallsignError::UsCharacters);
    }
    if !suffix.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(CallsignError::LetterPosition);
    }
    let letters = suffix.trim_start_matches(|c: char| c.is_ascii_digit());
    if letters.bytes().any(|c| c.is_ascii_digit()) {
        return Err(CallsignError::LetterPosition);
    }
    if letters.len() > 2 {
        return Err(CallsignError::TooManyLetters);
    }
    Ok(value)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallsignIdentity {
    pub registration: String,
    pub spoken_type: String,
    #[serde(default)]
    pub not_us: bool,
}
impl CallsignIdentity {
    fn validate(&mut self) -> Result<(), CallsignError> {
        self.registration = validate_registration(&self.registration, self.not_us)?;
        if !text_ok(&self.spoken_type, 64)
            || !self
                .spoken_type
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
        {
            return Err(CallsignError::SpokenType);
        }
        Ok(())
    }
}
/// Stored by eframe on desktop and in browser storage; keyed by resolved variant id.
pub type CallsignPreferences = std::collections::BTreeMap<String, CallsignIdentity>;

#[derive(Clone, Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommsCallsign {
    #[serde(default)]
    pub registration: Option<String>,
    #[serde(default)]
    pub spoken_type: Option<String>,
    /// Explicitly allow ICAO prefix-dash registrations outside the US.
    #[serde(default)]
    pub not_us: Option<bool>,
    /// Save this variant's choice in the application's preferences store.
    #[serde(default = "persist_default")]
    pub persist: bool,
    /// Disable to retain full identification; ATC abbreviations require controller initiation.
    #[serde(default)]
    pub abbreviate_after_contact: Option<bool>,
    #[serde(default)]
    pub reset_to_pack: bool,
}
fn persist_default() -> bool {
    true
}
impl Default for CommsCallsign {
    fn default() -> Self {
        Self {
            registration: None,
            spoken_type: None,
            not_us: None,
            persist: true,
            abbreviate_after_contact: None,
            reset_to_pack: false,
        }
    }
}
impl Command for CommsCallsign {
    const NAME: &'static str = "comms.callsign";
    const DESCRIPTION: &'static str = "Read or validate aircraft registration/type, save per variant, restore pack identity, or control ATC-initiated abbreviation.";
    type Output = PilotCallsign;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if self.registration.is_none()
            && self.spoken_type.is_none()
            && self.not_us.is_none()
            && self.abbreviate_after_contact.is_none()
            && !self.reset_to_pack
        {
            return Ok(s.pilot_callsign());
        }
        let variant = s.callsign_variant().to_owned();
        if self.reset_to_pack {
            s.comms.callsign_overrides.remove(&variant);
            s.comms.saved_callsigns.remove(&variant);
        } else if self.registration.is_some() || self.spoken_type.is_some() || self.not_us.is_some() {
            let current = s.pilot_callsign();
            let mut identity = CallsignIdentity {
                registration: self.registration.unwrap_or(current.registration),
                spoken_type: self.spoken_type.unwrap_or(current.spoken_type),
                not_us: self.not_us.unwrap_or(current.not_us),
            };
            identity.validate()?;
            if self.persist {
                s.comms.saved_callsigns.insert(variant.clone(), identity.clone());
            }
            s.comms.callsign_overrides.insert(variant, identity);
        }
        if let Some(v) = self.abbreviate_after_contact {
            s.comms.abbreviate = v;
        }
        s.comms.contacts.clear();
        let presentation = s.presentation_comms.take();
        let output = s.pilot_callsign();
        if let Some(mut snapshot) = presentation {
            snapshot.callsign = output.clone();
            s.presentation_comms = Some(snapshot);
        }
        Ok(output)
    }
}
impl Session {
    pub(crate) fn callsign_variant(&self) -> &str {
        self.active_configuration
            .as_ref()
            .map_or(self.sim().aircraft().identity.id.as_str(), |c| {
                c.variant.identity.id.as_str()
            })
    }
    pub fn callsign_preferences(&self) -> &CallsignPreferences {
        &self.comms.saved_callsigns
    }
    /// A browser display also carries session-only changes, without saving them as preferences.
    pub(crate) fn restore_presented_callsign(
        &mut self,
        callsign: &PilotCallsign,
    ) -> Result<(), CallsignError> {
        if callsign.overridden {
            let mut identity = CallsignIdentity {
                registration: callsign.registration.clone(),
                spoken_type: callsign.spoken_type.clone(),
                not_us: callsign.not_us,
            };
            identity.validate()?;
            self.comms
                .callsign_overrides
                .insert(self.callsign_variant().to_owned(), identity);
        }
        Ok(())
    }
    /// Validate the complete file before replacing any current preference.
    pub fn restore_callsign_preferences(
        &mut self,
        mut values: CallsignPreferences,
    ) -> Result<(), CallsignError> {
        if values.len() > 1024
            || values.keys().any(|k| {
                k.is_empty()
                    || k.len() > 128
                    || !k
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
            })
        {
            return Err(CallsignError::Preferences);
        }
        for value in values.values_mut() {
            value.validate()?;
        }
        self.comms.callsign_overrides = values.clone();
        self.comms.saved_callsigns = values;
        self.comms.contacts.clear();
        Ok(())
    }
    pub fn pilot_callsign(&self) -> PilotCallsign {
        if let Some(snapshot) = &self.presentation_comms {
            return snapshot.callsign.clone();
        }
        let identity = &self.sim().aircraft().identity;
        let overridden = self.comms.callsign_overrides.get(self.callsign_variant());
        let not_us = overridden.map_or_else(|| identity.registration.contains('-'), |c| c.not_us);
        let (registration, kind) = overridden
            .map(|c| (c.registration.clone(), c.spoken_type.clone()))
            .unwrap_or_else(|| {
                (
                    if identity.registration.is_empty() {
                        "N371FC".into()
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
        let (full, abbreviated) = phraseology::callsign_forms(&registration, &kind);
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
            overridden: overridden.is_some(),
            not_us,
            registration,
            spoken_type: kind,
            full,
            abbreviated,
            current,
        }
    }
}

/// Exact hybrid form produced by recognizers: “1234 Bravo” retains each registration value.
pub fn spoken_registration_groups(registration: &str) -> String {
    let mut result = String::new();
    let mut was_digit = false;
    for c in registration.chars() {
        if c.is_ascii_digit() {
            if !result.is_empty() && !was_digit {
                result.push(' ');
            }
            result.push(c);
            was_digit = true;
        } else {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&spoken_registration(&c.to_string()));
            was_digit = false;
        }
    }
    result
}
