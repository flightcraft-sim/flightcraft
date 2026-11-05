//! Behaviour of the landscape touch EFIS: screen modes, page model, knob roles, key presses.
//!
//! This module is pure state transitions. It never reads the session and never draws; skins
//! cannot change any of it. Section references are to `docs/glass.md` ("Landscape touch EFIS
//! model"), which cites the manufacturer's public pilot's guide for each rule.
use crate::touch_entry::EntryTarget;
use flightcraft_aero::modern::{MfdPage, NearestPage, TouchLayout};
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

const FEET_PER_NM: f64 = 6_076.115_49;

/// The 23 map ranges the range knob and the zoom buttons step through, in nautical miles. The
/// pilot's guide states that there are 23 ranges and that the distances depend on configuration;
/// it prints only part of the progression (2, 3, 5, 8, 12, 20, 30, 50, 80, 120 in its map setup
/// lists). The rest follow the same 1-1.5-2-3-5-8 progression: six steps in feet below 1 NM and
/// larger steps above 120 NM.
pub const MAP_RANGES_NM: [f64; 23] = [
    500. / FEET_PER_NM,
    800. / FEET_PER_NM,
    1200. / FEET_PER_NM,
    2000. / FEET_PER_NM,
    3000. / FEET_PER_NM,
    5000. / FEET_PER_NM,
    1.2,
    2.,
    3.,
    5.,
    8.,
    12.,
    20.,
    30.,
    50.,
    80.,
    120.,
    200.,
    300.,
    500.,
    800.,
    1000.,
    1200.,
];

/// The smallest and largest selectable range, in nautical miles.
pub const MAP_RANGE_MIN_NM: f64 = MAP_RANGES_NM[0];
pub const MAP_RANGE_MAX_NM: f64 = MAP_RANGES_NM[22];

/// Step the range by `detents`; clockwise (positive) zooms in, so the range gets smaller.
/// `reverse` is the "Knob Zoom Action: Reverse" setting. A range between steps snaps to the next
/// larger step first.
pub fn step_range(current_nm: f64, detents: i32, reverse: bool) -> f64 {
    let index = MAP_RANGES_NM
        .iter()
        .position(|r| *r >= current_nm * (1. - 1e-9))
        .unwrap_or(MAP_RANGES_NM.len() - 1) as i64;
    let direction = if reverse { 1 } else { -1 };
    let next = (index + i64::from(detents) * direction).clamp(0, MAP_RANGES_NM.len() as i64 - 1);
    MAP_RANGES_NM
        .get(next as usize)
        .copied()
        .unwrap_or(MAP_RANGE_MAX_NM)
}

/// The label printed at the map's lower right: feet below 1 NM, else nautical miles.
pub fn range_label(range_nm: f64) -> String {
    if range_nm < 1. {
        let feet = libm::round(range_nm * FEET_PER_NM / 100.) * 100.;
        format!("{feet:.0}ft")
    } else if (range_nm - libm::round(range_nm)).abs() < 1e-9 {
        format!("{range_nm:.0}nm")
    } else {
        format!("{range_nm:.1}nm")
    }
}

/// Step the nearest page by `detents` through the modelled nearest pages, wrapping.
pub fn step_nearest(current: NearestPage, detents: i32) -> NearestPage {
    let pages = NearestPage::ALL;
    let index = pages.iter().position(|p| *p == current).unwrap_or(0) as i64;
    let next = (index + i64::from(detents)).rem_euclid(pages.len() as i64);
    pages.get(next as usize).copied().unwrap_or_default()
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
    /// Direct-to waypoint entry (the identifier keypad).
    DirectTo,
    /// A touch keypad for a PFD box: baro, heading, altitude, or course.
    Keypad(EntryTarget),
    /// The COM1 standby window: frequencies, transfer, and the frequency keypad.
    ComWindow,
    /// The NAV1 standby window.
    NavWindow,
    /// The transponder window: code keypad and mode keys.
    TransponderWindow,
    /// The Select Page menu, opened from the page bar.
    SelectPage,
}

impl Overlay {
    /// The keypad target this overlay is entering a value for, if it is an entry overlay.
    pub fn entry_target(self) -> Option<EntryTarget> {
        match self {
            Self::DirectTo => Some(EntryTarget::DirectTo),
            Self::Keypad(t) => Some(t),
            Self::ComWindow => Some(EntryTarget::ComStandby),
            Self::NavWindow => Some(EntryTarget::NavStandby),
            Self::TransponderWindow => Some(EntryTarget::Transponder),
            _ => None,
        }
    }
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
                Overlay::MainMenu => Overlay::None,
                _ => Overlay::MainMenu,
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

/// Reversion: the surviving display presents the PFD and the engine page in split-screen.
pub fn reverted_layout(_installed_pfd: TouchLayout) -> TouchLayout {
    TouchLayout::Split
}

/// The page the MFD pane shows in reversion.
pub const REVERSION_PAGE: MfdPage = MfdPage::Engine;

/// A page the Select Page menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageChoice {
    Main(MfdPage),
    Nearest(NearestPage),
}

/// The Select Page menu for the current page: the nearest pages while the nearest group is
/// showing, else the main pages the engine models.
pub fn select_page_entries(current: MfdPage) -> Vec<PageChoice> {
    if current == MfdPage::Nearest {
        NearestPage::ALL
            .iter()
            .copied()
            .map(PageChoice::Nearest)
            .collect()
    } else {
        MfdPage::MAIN
            .iter()
            .copied()
            .chain([MfdPage::Cas])
            .map(PageChoice::Main)
            .collect()
    }
}

/// Labels for the status bar's knob hints: what the inner and outer ring of a knob do.
pub fn knob_hint(
    mode: ScreenMode,
    cfg: EfisConfig,
    knob: Knob,
    toggled: bool,
) -> (&'static str, &'static str) {
    (
        knob_role(mode, cfg, knob, Ring::Inner, toggled).label(),
        knob_role(mode, cfg, knob, Ring::Outer, toggled).label(),
    )
}

impl KnobRole {
    /// Short name of the function, as the status bar prints it.
    pub fn label(self) -> &'static str {
        match self {
            Self::HeadingBug => "Heading",
            Self::AltitudeBug => "Altitude",
            Self::Course => "Course",
            Self::Baro => "Baro",
            Self::FlightDirectorBug => "FD Bug",
            Self::MapRange => "Zoom Map",
            Self::MfdPage => "Select Page",
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
    fn range_ladder_has_the_guides_23_steps_in_increasing_order() {
        assert_eq!(MAP_RANGES_NM.len(), 23);
        assert!(MAP_RANGES_NM.windows(2).all(|w| w[0] < w[1]));
        // The steps the guide prints in its map setup lists are all on the ladder.
        for nm in [2., 3., 5., 8., 12., 20., 30., 50., 80., 120.] {
            assert!(MAP_RANGES_NM.contains(&nm), "{nm}");
        }
        // Six steps in feet, from 500 ft up to 5,000 ft, then 1.2 NM.
        let feet: Vec<f64> = MAP_RANGES_NM[..6]
            .iter()
            .map(|nm| (nm * 6076.11549).round())
            .collect();
        assert_eq!(feet, [500., 800., 1200., 2000., 3000., 5000.]);
        assert_eq!(MAP_RANGES_NM[6], 1.2);
        assert_eq!(MAP_RANGE_MIN_NM, MAP_RANGES_NM[0]);
        assert_eq!(MAP_RANGE_MAX_NM, 1200.);
    }

    #[test]
    fn range_knob_walks_every_step_and_stops_at_the_ends() {
        // Clockwise zooms in through all 22 steps, one detent each.
        let mut r = MAP_RANGE_MAX_NM;
        let mut seen = vec![r];
        for _ in 0..22 {
            r = step_range(r, 1, false);
            seen.push(r);
        }
        assert_eq!(seen.len(), 23);
        assert!(seen.windows(2).all(|w| w[1] < w[0]));
        assert_eq!(seen[22], MAP_RANGE_MIN_NM);
        assert_eq!(step_range(MAP_RANGE_MIN_NM, 5, false), MAP_RANGE_MIN_NM);
        assert_eq!(step_range(MAP_RANGE_MAX_NM, -5, false), MAP_RANGE_MAX_NM);
        assert_eq!(step_range(12., 1, false), 8.);
        assert_eq!(step_range(12., -1, false), 20.);
        assert_eq!(step_range(12., 1, true), 20.);
        // Several detents at once, and a range between steps snaps up first.
        assert_eq!(step_range(120., 3, false), 30.);
        assert_eq!(step_range(40., 1, false), 30.);
        assert_eq!(step_range(0.01, -1, false), MAP_RANGES_NM[1]);
    }

    #[test]
    fn range_labels_use_feet_below_one_mile() {
        assert_eq!(range_label(MAP_RANGES_NM[0]), "500ft");
        assert_eq!(range_label(MAP_RANGES_NM[5]), "5000ft");
        assert_eq!(range_label(1.2), "1.2nm");
        assert_eq!(range_label(20.), "20nm");
        assert_eq!(range_label(1200.), "1200nm");
    }

    #[test]
    fn nearest_pages_step_through_the_modelled_set_and_wrap() {
        assert_eq!(step_nearest(NearestPage::Airports, 1), NearestPage::Vors);
        assert_eq!(step_nearest(NearestPage::Airports, -1), NearestPage::Airspace);
        assert_eq!(step_nearest(NearestPage::Airspace, 1), NearestPage::Airports);
        assert_eq!(step_nearest(NearestPage::Vors, 3), NearestPage::Airspace);
    }

    #[test]
    fn select_page_menu_lists_main_pages_or_nearest_pages() {
        let main = select_page_entries(MfdPage::Map);
        assert_eq!(main.len(), MfdPage::MAIN.len() + 1);
        assert_eq!(main[0], PageChoice::Main(MfdPage::Map));
        assert_eq!(
            main.last(),
            Some(&PageChoice::Main(MfdPage::Cas)),
            "the CAS list follows the main pages"
        );
        let nearest = select_page_entries(MfdPage::Nearest);
        assert_eq!(nearest.len(), 5);
        assert_eq!(nearest[1], PageChoice::Nearest(NearestPage::Vors));
        assert!(!nearest.iter().any(|c| matches!(c, PageChoice::Main(_))));
    }

    #[test]
    fn knob_hints_name_the_functions_in_each_mode() {
        let h = |mode, k, t| knob_hint(mode, CFG, k, t);
        assert_eq!(h(ScreenMode::FullPfd, Knob::Left, false), ("Heading", "Altitude"));
        assert_eq!(h(ScreenMode::FullPfd, Knob::Right, false), ("Course", "Baro"));
        assert_eq!(
            h(ScreenMode::Split, Knob::Right, false),
            ("Zoom Map", "Select Page")
        );
        assert_eq!(h(ScreenMode::FullPfd, Knob::Left, true), ("Course", "Baro"));
        let fd = EfisConfig {
            alternate: AlternateKnobs::FdBugBaro,
            ..CFG
        };
        assert_eq!(
            knob_hint(ScreenMode::FullPfd, fd, Knob::Right, false),
            ("FD Bug", "Baro")
        );
    }

    #[test]
    fn entry_overlays_name_their_keypad_and_menu_leaves_them() {
        use crate::touch_entry::EntryTarget as T;
        assert_eq!(Overlay::Keypad(T::Baro).entry_target(), Some(T::Baro));
        assert_eq!(Overlay::DirectTo.entry_target(), Some(T::DirectTo));
        assert_eq!(Overlay::ComWindow.entry_target(), Some(T::ComStandby));
        assert_eq!(Overlay::NavWindow.entry_target(), Some(T::NavStandby));
        assert_eq!(Overlay::TransponderWindow.entry_target(), Some(T::Transponder));
        assert_eq!(Overlay::MainMenu.entry_target(), None);
        assert_eq!(Overlay::SelectPage.entry_target(), None);
        // BACK closes a keypad; MENU swaps it for the main menu.
        let mut s = state(TouchLayout::IntegratedPfd);
        s.overlay = Overlay::Keypad(T::Heading);
        assert_eq!(key(s, EfisKey::Back, false).overlay, Overlay::None);
        assert_eq!(key(s, EfisKey::Menu, false).overlay, Overlay::MainMenu);
        s.overlay = Overlay::SelectPage;
        assert_eq!(key(s, EfisKey::Back, false).overlay, Overlay::None);
        let json = serde_json::to_string(&Overlay::Keypad(T::Altitude)).unwrap();
        assert_eq!(json, r#"{"keypad":"altitude"}"#);
    }

    #[test]
    fn reversion_is_the_split_screen_with_the_engine_page() {
        assert_eq!(reverted_layout(TouchLayout::IntegratedPfd), TouchLayout::Split);
        assert_eq!(
            ScreenMode::of(reverted_layout(TouchLayout::FullPfd)),
            ScreenMode::Split
        );
        assert_eq!(REVERSION_PAGE, MfdPage::Engine);
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
