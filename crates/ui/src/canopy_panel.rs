//! When the cockpit canopy controls are worth drawing over the outside view.
//!
//! The panel is a ground and securing aid. In flight with the canopy latched nothing is drawn;
//! it appears briefly after the pilot hovers or clicks the inside latch, or after any canopy
//! change, and fades. An unlatched canopy in flight gets a small amber annunciation instead.

/// Seconds the panel stays after a hover, click or canopy change.
pub(crate) const REVEAL_S: f64 = 4.;
/// Seconds over which the panel fades out at the end of its reveal.
pub(crate) const FADE_S: f64 = 1.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CanopyPanel {
    Hidden,
    /// The full controls, at this opacity (1 is solid).
    Controls(f32),
    /// Small amber "CANOPY UNLATCHED" line only.
    Unlatched,
}

/// Remembers when the panel was last asked for, and the canopy it last saw.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Reveal {
    until_s: f64,
    seen: Option<(bool, u32)>,
}

impl Reveal {
    /// Keep the panel up for the next few seconds (hover, click, drag).
    pub(crate) fn touch(&mut self, now_s: f64) {
        self.until_s = now_s + REVEAL_S;
    }
    /// Notice a canopy change (from the panel, a key, a script or a failure). The first
    /// observation only records the canopy, so opening a flight does not flash the panel.
    pub(crate) fn observe(&mut self, now_s: f64, latched: bool, open_fraction: f64) {
        let now = (latched, (open_fraction.clamp(0., 1.) * 100.).round() as u32);
        if self.seen.is_some_and(|before| before != now) {
            self.touch(now_s);
        }
        self.seen = Some(now);
    }
    pub(crate) fn remaining_s(&self, now_s: f64) -> f64 {
        (self.until_s - now_s).max(0.)
    }
}

pub(crate) fn visibility(on_ground: bool, latched: bool, reveal_remaining_s: f64) -> CanopyPanel {
    if on_ground {
        CanopyPanel::Controls(1.)
    } else if reveal_remaining_s > 0. {
        CanopyPanel::Controls((reveal_remaining_s / FADE_S).clamp(0., 1.) as f32)
    } else if !latched {
        CanopyPanel::Unlatched
    } else {
        CanopyPanel::Hidden
    }
}

/// Plain wording for how far the canopy is open.
pub(crate) fn position_text(open_fraction: f64) -> String {
    let percent = (open_fraction.clamp(0., 1.) * 100.).round();
    if percent < 1. {
        "Closed".into()
    } else {
        format!("Open {percent:.0}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_the_ground_the_controls_are_always_solid() {
        assert_eq!(visibility(true, true, 0.), CanopyPanel::Controls(1.));
        assert_eq!(visibility(true, false, 0.), CanopyPanel::Controls(1.));
    }

    #[test]
    fn in_flight_latched_draws_nothing() {
        assert_eq!(visibility(false, true, 0.), CanopyPanel::Hidden);
    }

    #[test]
    fn in_flight_unlatched_shows_only_the_annunciation() {
        assert_eq!(visibility(false, false, 0.), CanopyPanel::Unlatched);
    }

    #[test]
    fn hover_or_click_reveals_the_controls_then_fades_away() {
        let mut reveal = Reveal::default();
        reveal.touch(100.);
        assert_eq!(
            visibility(false, true, reveal.remaining_s(101.)),
            CanopyPanel::Controls(1.)
        );
        let CanopyPanel::Controls(a) = visibility(false, true, reveal.remaining_s(103.5)) else {
            panic!("still fading");
        };
        assert!((a - 0.5).abs() < 1e-6);
        assert_eq!(
            visibility(false, true, reveal.remaining_s(104.5)),
            CanopyPanel::Hidden
        );
        // Unlatched in flight: controls while revealed, annunciation afterwards.
        assert_eq!(
            visibility(false, false, reveal.remaining_s(104.5)),
            CanopyPanel::Unlatched
        );
    }

    #[test]
    fn a_canopy_change_reveals_but_the_first_look_does_not() {
        let mut reveal = Reveal::default();
        reveal.observe(10., true, 0.);
        assert_eq!(reveal.remaining_s(10.), 0.);
        reveal.observe(11., true, 0.);
        assert_eq!(reveal.remaining_s(11.), 0.);
        reveal.observe(12., false, 0.);
        assert!(reveal.remaining_s(12.) > 3.9);
        reveal.observe(20., false, 0.);
        assert_eq!(reveal.remaining_s(20.), 0.);
        reveal.observe(21., false, 0.4);
        assert!(reveal.remaining_s(21.) > 3.9);
    }

    #[test]
    fn position_uses_plain_words() {
        assert_eq!(position_text(0.), "Closed");
        assert_eq!(position_text(0.003), "Closed");
        assert_eq!(position_text(0.4), "Open 40%");
        assert_eq!(position_text(1.), "Open 100%");
    }
}
