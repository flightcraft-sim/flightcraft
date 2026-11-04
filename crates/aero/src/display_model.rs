//! Display models: the physical geometry and control set of a class of panel display.
//!
//! A model is brand-neutral. Real unit names live in packs and skin packs, never here. The first
//! model is the 10.6-inch landscape touch EFIS: a 5:3 touch screen above one row of physical
//! controls (two knobs, NEAREST and DIRECT keys, a data-card slot, and MENU and BACK keys).
//!
//! Sources for the figures are in `docs/glass.md` ("Landscape touch EFIS model"). Figures marked
//! `estimate` could not be read from public text and are the engine's own proportions.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A display class the engine knows how to draw and operate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DisplayModel {
    /// 10.6-inch landscape touch EFIS with a bottom row of physical controls.
    LandscapeTouchEfis,
}

/// One physical control on the bezel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BezelControl {
    /// Dual-concentric knob on the left; by default the PFD "main" knob.
    LeftKnob,
    /// Opens the nearest pages.
    NearestKey,
    /// Opens direct-to entry.
    DirectKey,
    /// Data-card slot; not operable.
    CardSlot,
    /// Page menu, then main menu.
    MenuKey,
    /// Back, with press-and-hold behaviour.
    BackKey,
    /// Dual-concentric knob on the right; by default the PFD "alternate" knob or the MFD knob.
    RightKnob,
}

impl BezelControl {
    pub const ALL: [Self; 7] = [
        Self::LeftKnob,
        Self::NearestKey,
        Self::DirectKey,
        Self::CardSlot,
        Self::MenuKey,
        Self::BackKey,
        Self::RightKnob,
    ];
    pub fn is_knob(self) -> bool {
        matches!(self, Self::LeftKnob | Self::RightKnob)
    }
    /// Short functional label printed beside the control.
    pub fn label(self) -> &'static str {
        match self {
            Self::LeftKnob | Self::RightKnob => "",
            Self::NearestKey => "NRST",
            Self::DirectKey => "DIRECT",
            Self::CardSlot => "CARD",
            Self::MenuKey => "MENU",
            Self::BackKey => "BACK",
        }
    }
}

/// A control's placement as fractions of the whole bezel (0..1, origin top-left).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ControlPlacement {
    pub control: BezelControl,
    pub center: [f64; 2],
    /// Width and height as fractions of the bezel; knobs are circular on the shorter side.
    pub size: [f64; 2],
}

/// Complete physical description of a display model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DisplayGeometry {
    /// Native touch-screen resolution in pixels.
    pub resolution_px: [u32; 2],
    pub diagonal_in: f64,
    /// Visible screen in mm, derived from resolution and diagonal.
    pub screen_mm: [f64; 2],
    /// Panel cutout in mm.
    pub cutout_mm: [f64; 2],
    /// Outer bezel in mm (estimate).
    pub bezel_mm: [f64; 2],
    /// Screen rectangle within the bezel, as fractions: left, top, width, height.
    pub screen_rect: [f64; 4],
    pub controls: Vec<ControlPlacement>,
}

const MM_PER_IN: f64 = 25.4;

impl DisplayModel {
    pub const ALL: [Self; 1] = [Self::LandscapeTouchEfis];

    /// Native resolution, from the manufacturer's public specification (see docs).
    pub fn resolution_px(self) -> [u32; 2] {
        match self {
            Self::LandscapeTouchEfis => [1280, 768],
        }
    }

    /// Screen aspect, width over height.
    pub fn screen_aspect(self) -> f64 {
        let [w, h] = self.resolution_px();
        f64::from(w) / f64::from(h)
    }

    /// Aspect of the whole bezel, width over height.
    pub fn bezel_aspect(self) -> f64 {
        let g = self.geometry();
        g.bezel_mm[0] / g.bezel_mm[1]
    }

    pub fn geometry(self) -> DisplayGeometry {
        match self {
            Self::LandscapeTouchEfis => {
                let resolution_px = self.resolution_px();
                let diagonal_in = 10.6;
                let [w, h] = resolution_px.map(f64::from);
                let diag_px = libm::hypot(w, h);
                let screen_mm = [
                    diagonal_in * MM_PER_IN * w / diag_px,
                    diagonal_in * MM_PER_IN * h / diag_px,
                ];
                // Installation manual outline: cutout 262.00 x 185.10 mm. Outer bezel is an estimate.
                let cutout_mm = [262.0, 185.1];
                let bezel_mm = [272.0, 195.0];
                let left = (bezel_mm[0] - screen_mm[0]) / 2.0 / bezel_mm[0];
                let top = 12.0 / bezel_mm[1];
                let screen_rect = [left, top, screen_mm[0] / bezel_mm[0], screen_mm[1] / bezel_mm[1]];
                let row_y = (top * bezel_mm[1] + screen_mm[1] + (bezel_mm[1] - 12.0 - screen_mm[1]) / 2.0)
                    / bezel_mm[1];
                let c = |control, x: f64, w: f64, h: f64| ControlPlacement {
                    control,
                    center: [x, row_y],
                    size: [w, h],
                };
                // Left to right, in the order the pilot's guide's bezel figure labels them.
                let controls = vec![
                    c(BezelControl::LeftKnob, 0.075, 0.085, 0.145),
                    c(BezelControl::NearestKey, 0.205, 0.085, 0.07),
                    c(BezelControl::DirectKey, 0.300, 0.085, 0.07),
                    c(BezelControl::CardSlot, 0.500, 0.095, 0.03),
                    c(BezelControl::MenuKey, 0.700, 0.085, 0.07),
                    c(BezelControl::BackKey, 0.795, 0.085, 0.07),
                    c(BezelControl::RightKnob, 0.925, 0.085, 0.145),
                ];
                DisplayGeometry {
                    resolution_px,
                    diagonal_in,
                    screen_mm,
                    cutout_mm,
                    bezel_mm,
                    screen_rect,
                    controls,
                }
            }
        }
    }

    /// Placement of one control.
    pub fn placement(self, control: BezelControl) -> Option<ControlPlacement> {
        self.geometry()
            .controls
            .into_iter()
            .find(|p| p.control == control)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn landscape_screen_is_five_by_three_at_the_published_resolution() {
        let m = DisplayModel::LandscapeTouchEfis;
        assert_eq!(m.resolution_px(), [1280, 768]);
        assert!((m.screen_aspect() - 5. / 3.).abs() < 1e-9);
        let g = m.geometry();
        assert!((g.screen_mm[0] / g.screen_mm[1] - 5. / 3.).abs() < 1e-9);
        let diag = libm::hypot(g.screen_mm[0], g.screen_mm[1]) / 25.4;
        assert!((diag - 10.6).abs() < 1e-9);
    }

    #[test]
    fn screen_and_controls_fit_inside_the_bezel_without_overlap() {
        let g = DisplayModel::LandscapeTouchEfis.geometry();
        let [l, t, w, h] = g.screen_rect;
        assert!(l > 0. && t > 0. && l + w < 1. && t + h < 1.);
        assert!(g.screen_mm[0] < g.cutout_mm[0] && g.cutout_mm[0] < g.bezel_mm[0]);
        assert!(g.screen_mm[1] < g.cutout_mm[1] && g.cutout_mm[1] < g.bezel_mm[1]);
        assert_eq!(g.controls.len(), BezelControl::ALL.len());
        for p in &g.controls {
            let [cx, cy] = p.center;
            let [pw, ph] = p.size;
            assert!(cx - pw / 2. >= 0. && cx + pw / 2. <= 1.);
            // Controls sit below the screen.
            assert!(cy - ph / 2. >= t + h, "{:?} overlaps the screen", p.control);
            assert!(cy + ph / 2. <= 1.);
        }
        for (i, a) in g.controls.iter().enumerate() {
            for b in g.controls.iter().skip(i + 1) {
                assert!(
                    (a.center[0] - b.center[0]).abs() >= (a.size[0] + b.size[0]) / 2.,
                    "{:?} overlaps {:?}",
                    a.control,
                    b.control
                );
            }
        }
    }

    #[test]
    fn controls_run_left_to_right_in_the_documented_order() {
        let g = DisplayModel::LandscapeTouchEfis.geometry();
        let order: Vec<_> = g.controls.iter().map(|p| p.control).collect();
        assert_eq!(order, BezelControl::ALL);
        assert!(g.controls.windows(2).all(|w| w[0].center[0] < w[1].center[0]));
    }
}
