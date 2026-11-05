//! Barometric minimums: the pilot's minimums altitude, the PFD annunciation it drives, and the
//! once-per-descent callout. Pure functions; nothing reads a session or a clock.
//!
//! The altitude compared is the indicated (barometric) altitude, as the altimeter shows it. The
//! annunciation arms 100 ft above the minimums and stays while the aircraft is at or below them.

/// Distance above the minimums altitude at which the annunciation appears, in feet.
pub const APPROACH_FT: f64 = 100.0;
const FEET_PER_METRE: f64 = 1.0 / 0.3048;

/// What the PFD shows for the current altitude against the minimums.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimumsState {
    /// No minimums altitude is set, or no valid altitude.
    Off,
    /// More than 100 ft above the minimums: the bug only.
    Clear,
    /// Within 100 ft above the minimums: amber MINIMUMS.
    Approaching,
    /// At or below the minimums: amber MINIMUMS, drawn solid so it differs from the approach.
    Reached,
}

impl MinimumsState {
    /// The annunciation text, if one is shown.
    pub fn text(self) -> Option<&'static str> {
        matches!(self, Self::Approaching | Self::Reached).then_some("MINIMUMS")
    }
}

/// The state for an indicated altitude and a minimums altitude, both metres MSL.
pub fn state(altitude_m: f64, minimums_m: Option<f64>) -> MinimumsState {
    let Some(minimums) = minimums_m.filter(|m| m.is_finite()) else {
        return MinimumsState::Off;
    };
    if !altitude_m.is_finite() {
        return MinimumsState::Off;
    }
    let above_ft = (altitude_m - minimums) * FEET_PER_METRE;
    if above_ft <= 1e-6 {
        MinimumsState::Reached
    } else if above_ft <= APPROACH_FT + 1e-6 {
        MinimumsState::Approaching
    } else {
        MinimumsState::Clear
    }
}

/// Advance the callout watch by one step. `above` is whether the aircraft was above the minimums
/// at the previous step (`None` when not yet watching). Returns whether to sound the callout now,
/// and the value to keep. The callout sounds only on a step that finds the aircraft at or below
/// the minimums after a step that found it above: once per descent, never at the moment the
/// minimums are set below the aircraft's altitude, and again only after a climb back above.
pub fn callout(above: Option<bool>, now: MinimumsState) -> (bool, Option<bool>) {
    match now {
        MinimumsState::Off => (false, None),
        MinimumsState::Clear | MinimumsState::Approaching => (false, Some(true)),
        MinimumsState::Reached => (above == Some(true), Some(false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ft(feet: f64) -> f64 {
        feet * 0.3048
    }

    #[test]
    fn the_annunciation_arms_within_100_ft_above_and_holds_below() {
        let min = Some(ft(1000.));
        assert_eq!(state(ft(2000.), None), MinimumsState::Off);
        assert_eq!(state(f64::NAN, min), MinimumsState::Off);
        assert_eq!(state(ft(1101.), min), MinimumsState::Clear);
        assert_eq!(state(ft(1100.), min), MinimumsState::Approaching);
        assert_eq!(state(ft(1001.), min), MinimumsState::Approaching);
        assert_eq!(state(ft(1000.), min), MinimumsState::Reached);
        assert_eq!(state(ft(400.), min), MinimumsState::Reached);
        assert_eq!(state(ft(1050.), min).text(), Some("MINIMUMS"));
        assert_eq!(state(ft(1500.), min).text(), None);
    }

    #[test]
    fn the_callout_sounds_once_per_descent() {
        let min = Some(ft(1000.));
        let mut above = None;
        let mut calls = 0;
        // Descend through the minimums, level off below, climb out, and descend again.
        let path = [
            1500., 1200., 1100., 1050., 1001., 999., 950., 900., 900., 1200., 1300., 1090., 990.,
        ];
        for altitude in path {
            let (call, next) = callout(above, state(ft(altitude), min));
            calls += u32::from(call);
            above = next;
        }
        assert_eq!(calls, 2);
    }

    #[test]
    fn minimums_set_below_the_aircraft_do_not_call_and_clearing_stops_the_watch() {
        let (call, above) = callout(None, state(ft(500.), Some(ft(1000.))));
        assert!(!call);
        let (call, above) = callout(above, state(ft(450.), Some(ft(1000.))));
        assert!(!call);
        assert_eq!(callout(above, state(ft(450.), None)), (false, None));
    }
}
