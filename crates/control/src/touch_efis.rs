//! Behaviour of the landscape touch EFIS: screen modes, page model, knob roles, key presses.
//!
//! This module is pure state transitions. It never reads the session and never draws; skins
//! cannot change any of it. Section references are to `docs/glass.md` ("Landscape touch EFIS
//! model"), which cites the manufacturer's public pilot's guide for each rule.
use flightcraft_aero::modern::{MfdPage, TouchLayout};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use flightcraft_aero::display_model::BezelControl;

/// How much of the screen the PFD and MFD share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScreenMode {
    FullPfd,
    FullMfd,
    Split,
}

impl ScreenMode {
    pub fn of(layout: TouchLayout) -> Self {
        match layout {
            TouchLayout::IntegratedPfd | TouchLayout::FullPfd | TouchLayout::Svt => Self::FullPfd,
            TouchLayout::PfdMap | TouchLayout::Split => Self::Split,
            TouchLayout::Mfd
            | TouchLayout::MapEngine
            | TouchLayout::Engine
            | TouchLayout::FlightPlan
            | TouchLayout::Nearest
            | TouchLayout::Traffic => Self::FullMfd,
        }
    }
}

/// The page the MFD (full or split pane) shows for a layout and the display's saved page.
pub fn effective_page(layout: TouchLayout, saved: MfdPage) -> MfdPage {
    match layout {
        TouchLayout::PfdMap => MfdPage::Map,
        // Legacy full-screen layouts name their page; `Mfd` and `Split` use the saved page.
        TouchLayout::Mfd | TouchLayout::Split => saved,
        other => MfdPage::of_layout(other).unwrap_or(saved),
    }
}

/// Layout for a screen mode and MFD page. The PFD layout is the display's installed one.
pub fn layout_for(mode: ScreenMode, installed_pfd: TouchLayout) -> TouchLayout {
    match mode {
        ScreenMode::FullPfd => installed_pfd,
        ScreenMode::FullMfd => TouchLayout::Mfd,
        ScreenMode::Split => TouchLayout::Split,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Knob {
    Left,
    Right,
}

/// Dual-concentric knob: the small inner ring and the large outer ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Ring {
    Inner,
    Outer,
}

/// What turning a knob ring changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnobRole {
    HeadingBug,
    AltitudeBug,
    Course,
    Baro,
    FlightDirectorBug,
    MapRange,
    MfdPage,
}

/// The "Right Side Knob Action" setting: what the alternate PFD knob does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlternateKnobs {
    /// Inner: course. Outer: baro. The default.
    #[default]
    CourseBaro,
    /// Inner: flight director bug. Outer: baro.
    FdBugBaro,
    /// Mirrors the main knob: heading and altitude.
    HeadingAltitude,
}

/// Where the MFD pane sits in split mode. The PFD uses the knob on the other side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneSide {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EfisConfig {
    pub alternate: AlternateKnobs,
    /// "Press to toggle knob action": an inner-knob push swaps main and alternate functions.
    pub press_to_toggle: bool,
    pub mfd_side: PaneSide,
}

impl Default for EfisConfig {
    fn default() -> Self {
        Self {
            alternate: AlternateKnobs::CourseBaro,
            press_to_toggle: false,
            mfd_side: PaneSide::Right,
        }
    }
}

/// Seconds after which a toggled knob returns to its normal functions.
pub const TOGGLE_REVERT_S: f64 = 10.0;

fn main_role(ring: Ring) -> KnobRole {
    match ring {
        Ring::Inner => KnobRole::HeadingBug,
        Ring::Outer => KnobRole::AltitudeBug,
    }
}

fn alternate_role(cfg: EfisConfig, ring: Ring) -> KnobRole {
    match (cfg.alternate, ring) {
        (AlternateKnobs::CourseBaro, Ring::Inner) => KnobRole::Course,
        (AlternateKnobs::FdBugBaro, Ring::Inner) => KnobRole::FlightDirectorBug,
        (AlternateKnobs::HeadingAltitude, Ring::Inner) => KnobRole::HeadingBug,
        (AlternateKnobs::HeadingAltitude, Ring::Outer) => KnobRole::AltitudeBug,
        (_, Ring::Outer) => KnobRole::Baro,
    }
}

fn mfd_role(ring: Ring) -> KnobRole {
    match ring {
        Ring::Inner => KnobRole::MapRange,
        Ring::Outer => KnobRole::MfdPage,
    }
}

/// Which side the PFD's main knob is on. Main functions are on the left by default.
fn pfd_knob_side(cfg: EfisConfig) -> Knob {
    match cfg.mfd_side {
        PaneSide::Right => Knob::Left,
        PaneSide::Left => Knob::Right,
    }
}

/// What turning `ring` of `knob` changes. `toggled` is a live press-to-toggle state.
pub fn knob_role(mode: ScreenMode, cfg: EfisConfig, knob: Knob, ring: Ring, toggled: bool) -> KnobRole {
    match mode {
        ScreenMode::FullMfd => mfd_role(ring),
        ScreenMode::FullPfd => {
            // Main knobs on the left, alternate on the right; a toggle swaps them.
            let main = (knob == Knob::Left) != toggled;
            if main {
                main_role(ring)
            } else {
                alternate_role(cfg, ring)
            }
        }
        ScreenMode::Split => {
            if knob == pfd_knob_side(cfg) {
                // Split reserves the other knob for the MFD, so the PFD knob carries both sets.
                if toggled {
                    alternate_role(cfg, ring)
                } else {
                    main_role(ring)
                }
            } else {
                mfd_role(ring)
            }
        }
    }
}

/// What pushing a knob's inner ring does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushEffect {
    /// Swap main and alternate functions for [`TOGGLE_REVERT_S`].
    ToggleFunctions,
    /// Set the heading bug to the current heading.
    SyncHeading,
    /// Set the course to the current desired track or heading.
    SyncCourse,
    /// Set the flight director bug to the current pitch.
    SyncFlightDirector,
    /// Recentre the map on the aircraft (clear panning).
    CentreMap,
    None,
}

pub fn knob_push(mode: ScreenMode, cfg: EfisConfig, knob: Knob, toggled: bool) -> PushEffect {
    let role = knob_role(mode, cfg, knob, Ring::Inner, toggled);
    if cfg.press_to_toggle && mode != ScreenMode::FullMfd && role != KnobRole::MapRange {
        return PushEffect::ToggleFunctions;
    }
    match role {
        KnobRole::HeadingBug => PushEffect::SyncHeading,
        KnobRole::Course => PushEffect::SyncCourse,
        KnobRole::FlightDirectorBug => PushEffect::SyncFlightDirector,
        KnobRole::MapRange => PushEffect::CentreMap,
        _ => PushEffect::None,
    }
}

/// Map ranges the range knob steps through, in nautical miles.
pub const MAP_RANGES_NM: [f64; 14] = [1., 2., 3., 5., 7., 10., 15., 20., 30., 50., 70., 100., 150., 200.];

/// Step the range by `detents`; clockwise (positive) zooms in, so the range gets smaller.
/// `reverse` is the "Knob Zoom Action: Reverse" setting.
pub fn step_range(current_nm: f64, detents: i32, reverse: bool) -> f64 {
    let index = MAP_RANGES_NM
        .iter()
        .position(|r| *r >= current_nm - 1e-9)
        .unwrap_or(MAP_RANGES_NM.len() - 1) as i64;
    let direction = if reverse { 1 } else { -1 };
    let next = (index + i64::from(detents) * direction).clamp(0, MAP_RANGES_NM.len() as i64 - 1);
    MAP_RANGES_NM.get(next as usize).copied().unwrap_or(200.)
}

/// Step the MFD page by `detents` through the main pages, wrapping.
pub fn step_page(current: MfdPage, detents: i32) -> MfdPage {
    let pages = MfdPage::MAIN;
    // The Nearest page is entered with its key; the knob leaves it for the first main page.
    let index = pages.iter().position(|p| *p == current).unwrap_or(0) as i64;
    let next = (index + i64::from(detents)).rem_euclid(pages.len() as i64);
    pages.get(next as usize).copied().unwrap_or(MfdPage::Map)
}

/// A keypad or menu shown over the display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Overlay {
    #[default]
    None,
    /// MENU on a full-screen PFD: navigation tools and More Options.
    PfdOptions,
    /// MENU on any other screen: options for the page.
    PageMenu,
    /// A second MENU press.
    MainMenu,
    /// Direct-to waypoint entry.
    DirectTo,
}

/// A physical key on the bezel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EfisKey {
    Nearest,
    Direct,
    Menu,
    Back,
}

/// The persistent and transient state a key press changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyState {
    pub layout: TouchLayout,
    pub installed_pfd: TouchLayout,
    pub page: MfdPage,
    pub overlay: Overlay,
    /// The page shown before the current one, for BACK.
    pub previous: Option<(TouchLayout, MfdPage)>,
}

/// Apply a key press; `hold` is a press-and-hold.
pub fn key(state: KeyState, key: EfisKey, hold: bool) -> KeyState {
    let mut next = state;
    let mode = ScreenMode::of(state.layout);
    let remember =
        |s: &mut KeyState| s.previous = Some((state.layout, effective_page(state.layout, state.page)));
    match key {
        EfisKey::Nearest => {
            // The nearest pages live on the MFD; a full PFD opens the split screen to show them.
            remember(&mut next);
            next.page = MfdPage::Nearest;
            next.layout = if mode == ScreenMode::FullMfd {
                TouchLayout::Mfd
            } else {
                TouchLayout::Split
            };
            next.overlay = Overlay::None;
        }
        EfisKey::Direct => next.overlay = Overlay::DirectTo,
        EfisKey::Menu => {
            next.overlay = match state.overlay {
                Overlay::None => {
                    if mode == ScreenMode::FullPfd {
                        Overlay::PfdOptions
                    } else {
                        Overlay::PageMenu
                    }
                }
                // A second press opens the main menu; a third clears it.
                Overlay::PfdOptions | Overlay::PageMenu | Overlay::DirectTo => Overlay::MainMenu,
                Overlay::MainMenu => Overlay::None,
            };
        }
        EfisKey::Back => {
            if hold {
                // Hold toggles full-screen and split-screen.
                remember(&mut next);
                next.overlay = Overlay::None;
                next.layout = match mode {
                    ScreenMode::Split => state.installed_pfd,
                    ScreenMode::FullPfd => TouchLayout::Split,
                    ScreenMode::FullMfd => TouchLayout::Split,
                };
            } else if state.overlay != Overlay::None {
                next.overlay = Overlay::None;
            } else if let Some((layout, page)) = state.previous {
                next.previous = Some((state.layout, effective_page(state.layout, state.page)));
                next.layout = layout;
                next.page = page;
            }
        }
    }
    next
}

/// Touching the on-screen SPLIT or FULL button.
pub fn toggle_split(state: KeyState) -> KeyState {
    key(state, EfisKey::Back, true)
}

/// Set a page directly (page bar, Select Page menu, or MFD knob). A full PFD opens the split.
pub fn show_page(state: KeyState, page: MfdPage) -> KeyState {
    let mut next = state;
    next.previous = Some((state.layout, effective_page(state.layout, state.page)));
    next.page = page;
    next.overlay = Overlay::None;
    if ScreenMode::of(state.layout) == ScreenMode::FullPfd {
        next.layout = TouchLayout::Split;
    } else if state.layout != TouchLayout::Split {
        next.layout = TouchLayout::Mfd;
    }
    next
}

/// Reversion: the surviving display shows the PFD with an engine strip.
pub fn reverted_layout(installed_pfd: TouchLayout) -> TouchLayout {
    installed_pfd
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: EfisConfig = EfisConfig {
        alternate: AlternateKnobs::CourseBaro,
        press_to_toggle: false,
        mfd_side: PaneSide::Right,
    };

    fn state(layout: TouchLayout) -> KeyState {
        KeyState {
            layout,
            installed_pfd: TouchLayout::IntegratedPfd,
            page: MfdPage::Map,
            overlay: Overlay::None,
            previous: None,
        }
    }

    #[test]
    fn full_pfd_knobs_follow_the_guides_defaults() {
        let m = ScreenMode::FullPfd;
        let r = |k, ring| knob_role(m, CFG, k, ring, false);
        assert_eq!(r(Knob::Left, Ring::Inner), KnobRole::HeadingBug);
        assert_eq!(r(Knob::Left, Ring::Outer), KnobRole::AltitudeBug);
        assert_eq!(r(Knob::Right, Ring::Inner), KnobRole::Course);
        assert_eq!(r(Knob::Right, Ring::Outer), KnobRole::Baro);
    }

    #[test]
    fn right_side_knob_action_selects_the_alternate_functions() {
        let m = ScreenMode::FullPfd;
        let fd = EfisConfig {
            alternate: AlternateKnobs::FdBugBaro,
            ..CFG
        };
        assert_eq!(
            knob_role(m, fd, Knob::Right, Ring::Inner, false),
            KnobRole::FlightDirectorBug
        );
        assert_eq!(knob_role(m, fd, Knob::Right, Ring::Outer, false), KnobRole::Baro);
        let mirror = EfisConfig {
            alternate: AlternateKnobs::HeadingAltitude,
            ..CFG
        };
        assert_eq!(
            knob_role(m, mirror, Knob::Right, Ring::Inner, false),
            KnobRole::HeadingBug
        );
        assert_eq!(
            knob_role(m, mirror, Knob::Right, Ring::Outer, false),
            KnobRole::AltitudeBug
        );
    }

    #[test]
    fn toggling_swaps_main_and_alternate_on_a_full_pfd() {
        let m = ScreenMode::FullPfd;
        assert_eq!(knob_role(m, CFG, Knob::Left, Ring::Outer, true), KnobRole::Baro);
        assert_eq!(
            knob_role(m, CFG, Knob::Right, Ring::Inner, true),
            KnobRole::HeadingBug
        );
    }

    #[test]
    fn split_screen_reserves_the_mfd_knob() {
        let m = ScreenMode::Split;
        // Default: MFD pane on the right, so the right knob drives the MFD.
        assert_eq!(
            knob_role(m, CFG, Knob::Right, Ring::Outer, false),
            KnobRole::MfdPage
        );
        assert_eq!(
            knob_role(m, CFG, Knob::Right, Ring::Inner, false),
            KnobRole::MapRange
        );
        assert_eq!(
            knob_role(m, CFG, Knob::Left, Ring::Inner, false),
            KnobRole::HeadingBug
        );
        // With the pane on the left the roles exchange.
        let left = EfisConfig {
            mfd_side: PaneSide::Left,
            ..CFG
        };
        assert_eq!(
            knob_role(m, left, Knob::Left, Ring::Outer, false),
            KnobRole::MfdPage
        );
        assert_eq!(
            knob_role(m, left, Knob::Right, Ring::Outer, false),
            KnobRole::AltitudeBug
        );
        // The PFD knob reaches baro and course only through the toggle.
        assert_eq!(knob_role(m, CFG, Knob::Left, Ring::Outer, true), KnobRole::Baro);
        assert_eq!(knob_role(m, CFG, Knob::Left, Ring::Inner, true), KnobRole::Course);
    }

    #[test]
    fn full_mfd_knobs_both_drive_the_mfd() {
        for k in [Knob::Left, Knob::Right] {
            assert_eq!(
                knob_role(ScreenMode::FullMfd, CFG, k, Ring::Outer, false),
                KnobRole::MfdPage
            );
            assert_eq!(
                knob_role(ScreenMode::FullMfd, CFG, k, Ring::Inner, false),
                KnobRole::MapRange
            );
        }
    }

    #[test]
    fn pushing_the_heading_knob_syncs_the_bug_unless_toggle_is_enabled() {
        assert_eq!(
            knob_push(ScreenMode::FullPfd, CFG, Knob::Left, false),
            PushEffect::SyncHeading
        );
        assert_eq!(
            knob_push(ScreenMode::FullPfd, CFG, Knob::Right, false),
            PushEffect::SyncCourse
        );
        let t = EfisConfig {
            press_to_toggle: true,
            ..CFG
        };
        assert_eq!(
            knob_push(ScreenMode::FullPfd, t, Knob::Left, false),
            PushEffect::ToggleFunctions
        );
        assert_eq!(
            knob_push(ScreenMode::FullMfd, t, Knob::Left, false),
            PushEffect::CentreMap
        );
        // The MFD knob in split screen never toggles.
        assert_eq!(
            knob_push(ScreenMode::Split, t, Knob::Right, false),
            PushEffect::CentreMap
        );
    }

    #[test]
    fn range_knob_steps_the_published_ladder_and_stops_at_the_ends() {
        assert_eq!(step_range(10., 1, false), 7.);
        assert_eq!(step_range(10., -1, false), 15.);
        assert_eq!(step_range(10., 1, true), 15.);
        assert_eq!(step_range(1., 5, false), 1.);
        assert_eq!(step_range(200., -5, false), 200.);
        assert_eq!(step_range(12., 1, false), 10.);
    }

    #[test]
    fn page_knob_cycles_the_main_pages_and_wraps() {
        assert_eq!(step_page(MfdPage::Map, 1), MfdPage::Waypoint);
        assert_eq!(step_page(MfdPage::Map, -1), MfdPage::Engine);
        assert_eq!(step_page(MfdPage::Engine, 1), MfdPage::Map);
        assert_eq!(step_page(MfdPage::Nearest, 1), MfdPage::Waypoint);
        let mut p = MfdPage::Map;
        for _ in 0..MfdPage::MAIN.len() {
            p = step_page(p, 1);
        }
        assert_eq!(p, MfdPage::Map);
    }

    #[test]
    fn menu_cycles_page_menu_main_menu_and_clear() {
        let mut s = state(TouchLayout::Mfd);
        s = key(s, EfisKey::Menu, false);
        assert_eq!(s.overlay, Overlay::PageMenu);
        s = key(s, EfisKey::Menu, false);
        assert_eq!(s.overlay, Overlay::MainMenu);
        s = key(s, EfisKey::Menu, false);
        assert_eq!(s.overlay, Overlay::None);
        // On a full PFD the first press is the PFD options menu.
        let p = key(state(TouchLayout::IntegratedPfd), EfisKey::Menu, false);
        assert_eq!(p.overlay, Overlay::PfdOptions);
        assert_eq!(key(p, EfisKey::Menu, false).overlay, Overlay::MainMenu);
    }

    #[test]
    fn back_closes_menus_then_returns_to_the_previous_page() {
        let mut s = state(TouchLayout::Mfd);
        s = show_page(s, MfdPage::FlightPlan);
        assert_eq!(s.page, MfdPage::FlightPlan);
        s = key(s, EfisKey::Menu, false);
        s = key(s, EfisKey::Back, false);
        assert_eq!(s.overlay, Overlay::None);
        assert_eq!(s.page, MfdPage::FlightPlan);
        s = key(s, EfisKey::Back, false);
        assert_eq!(s.page, MfdPage::Map);
    }

    #[test]
    fn back_hold_toggles_full_and_split() {
        let pfd = state(TouchLayout::IntegratedPfd);
        let split = key(pfd, EfisKey::Back, true);
        assert_eq!(ScreenMode::of(split.layout), ScreenMode::Split);
        let back = key(split, EfisKey::Back, true);
        assert_eq!(back.layout, TouchLayout::IntegratedPfd);
        assert_eq!(toggle_split(pfd).layout, TouchLayout::Split);
        let mfd = key(state(TouchLayout::Mfd), EfisKey::Back, true);
        assert_eq!(mfd.layout, TouchLayout::Split);
    }

    #[test]
    fn nearest_opens_the_nearest_page_even_from_a_full_pfd() {
        let s = key(state(TouchLayout::IntegratedPfd), EfisKey::Nearest, false);
        assert_eq!(s.layout, TouchLayout::Split);
        assert_eq!(s.page, MfdPage::Nearest);
        let m = key(state(TouchLayout::Mfd), EfisKey::Nearest, false);
        assert_eq!(m.layout, TouchLayout::Mfd);
        assert_eq!(effective_page(m.layout, m.page), MfdPage::Nearest);
    }

    #[test]
    fn direct_opens_entry_from_any_screen_and_back_cancels_it() {
        for layout in [TouchLayout::IntegratedPfd, TouchLayout::Mfd, TouchLayout::Split] {
            let s = key(state(layout), EfisKey::Direct, false);
            assert_eq!(s.overlay, Overlay::DirectTo);
            assert_eq!(key(s, EfisKey::Back, false).overlay, Overlay::None);
        }
    }

    #[test]
    fn showing_a_page_from_a_full_pfd_opens_the_split() {
        let s = show_page(state(TouchLayout::IntegratedPfd), MfdPage::Traffic);
        assert_eq!(s.layout, TouchLayout::Split);
        assert_eq!(s.page, MfdPage::Traffic);
        let f = show_page(state(TouchLayout::Mfd), MfdPage::Terrain);
        assert_eq!(f.layout, TouchLayout::Mfd);
    }

    #[test]
    fn legacy_layouts_name_their_page() {
        assert_eq!(effective_page(TouchLayout::Engine, MfdPage::Map), MfdPage::Engine);
        assert_eq!(effective_page(TouchLayout::PfdMap, MfdPage::Info), MfdPage::Map);
        assert_eq!(effective_page(TouchLayout::Split, MfdPage::Info), MfdPage::Info);
        assert_eq!(ScreenMode::of(TouchLayout::Svt), ScreenMode::FullPfd);
        assert_eq!(ScreenMode::of(TouchLayout::Traffic), ScreenMode::FullMfd);
    }
}
