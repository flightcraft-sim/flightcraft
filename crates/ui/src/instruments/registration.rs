//! The aircraft's identification placard: the full registration, as a real panel carries.
use super::{DrawList, Ink, Instrument, InstrumentInput};

/// Shows the current registration (the pilot's override, else the pack's), so a custom tail
/// number appears on the panel as soon as it is set.
struct RegistrationPlacard;

impl Instrument for RegistrationPlacard {
    fn update(&mut self, input: InstrumentInput<'_>) -> DrawList {
        let text = input
            .registration
            .or_else(|| input.aircraft.map(|a| a.variant.identity.registration.as_str()))
            .unwrap_or_default();
        let mut d = DrawList {
            size: [320., 22.],
            items: Vec::new(),
            indicated: 0.,
            valid: !text.is_empty(),
        };
        if !text.is_empty() {
            // Fits the strip for up to the longest allowed registration.
            d.label([160., 11.], text, 18., Ink::Text);
        }
        d
    }
}

pub(super) fn create(_: &str) -> Result<Box<dyn Instrument>, String> {
    Ok(Box::new(RegistrationPlacard))
}
