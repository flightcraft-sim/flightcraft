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

/// Which arrangement of the primary flight display is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PfdMode {
    /// The primary flight display fills the screen.
    Full,
    /// The primary flight display shares the screen with one multi-function pane.
    Split,
}

/// An element of the primary flight display, for placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PfdElement {
    /// The whole top strip: radio boxes, data fields, and the split button.
    DataBar,
    /// Active and standby communication frequencies at the left of the data bar.
    ComBox,
    /// Transponder code and mode box in the data bar (split mode only).
    TransponderBox,
    /// The SPLIT or FULL touch button at the right end of the data bar.
    SplitButton,
    /// Autopilot, yaw damper, and flight director mode annunciations, top centre.
    AfcsBox,
    AirspeedTape,
    TrueAirspeedBox,
    GroundSpeedBox,
    /// The black box on the airspeed tape that shows the airspeed.
    AirspeedPointer,
    AltitudeTape,
    SelectedAltitudeBox,
    BaroBox,
    /// The barometric minimums box under the baro box: touch it to set the minimums altitude.
    MinimumsBox,
    /// The vertical speed scale beside the altitude tape.
    VerticalSpeedScale,
    /// The black box on the altitude tape that shows the altitude.
    AltitudePointer,
    /// Ball under the aircraft symbol.
    SlipBox,
    /// Standard-rate turn scale under the slip ball.
    TurnRateScale,
    /// Current heading or track, top of the compass.
    HeadingBox,
    SelectedHeadingBox,
    SelectedCourseBox,
    WindBox,
    /// Crew alerting (CAS) messages, upper left of the attitude indicator.
    CasWindow,
    /// Flap position indicator: a vertical scale left of the attitude symbol.
    FlapIndicator,
    /// Elevator trim indicator, beside the flap indicator.
    TrimIndicator,
    /// The G meter (a PFD option): load factor now, and the lowest and highest since reset.
    GMeter,
    /// Knob function hints and messages along the bottom edge.
    StatusBar,
}

/// A field of the top data bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataField {
    /// Next waypoint identifier.
    Waypoint,
    /// Magnetic bearing to the waypoint.
    Bearing,
    /// Distance to the waypoint.
    Distance,
    /// Time en route to the waypoint.
    TimeEnRoute,
    GroundSpeed,
    /// Ground track.
    Track,
    /// Minimum safe altitude.
    SafeAltitude,
    /// Vertical speed required to meet the target altitude.
    VerticalSpeedRequired,
}

impl DataField {
    /// Short label printed above the value.
    pub fn label(self) -> &'static str {
        match self {
            Self::Waypoint => "WPT",
            Self::Bearing => "BRG",
            Self::Distance => "DIST",
            Self::TimeEnRoute => "ETE",
            Self::GroundSpeed => "GS",
            Self::Track => "TRK",
            Self::SafeAltitude => "MSA",
            Self::VerticalSpeedRequired => "VSR",
        }
    }
}

/// Placement of the primary flight display's elements, in native screen pixels of the model
/// (1280 x 768). `panel` is the rectangle the primary flight display occupies; element rectangles
/// are relative to the panel's top-left, except the data bar and status bar, which span the screen.
///
/// The arrangement is read from the figures of the modelled unit's pilot's guide (flight
/// instruments overview, full-screen and split-screen figures); every number is a measurement
/// of a figure and good to a few percent, not a drawing dimension.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PfdGeometry {
    pub mode: PfdMode,
    /// `[left, top, width, height]` of the panel on the screen.
    pub panel: [f64; 4],
    /// `[left, top, width, height]` of each element.
    pub elements: Vec<(PfdElement, [f64; 4])>,
    /// Aircraft symbol and roll scale centre, within the panel.
    pub aircraft_symbol: [f64; 2],
    /// Radius of the roll scale arc.
    pub roll_arc_radius: f64,
    /// Compass rose centre within the panel, and its radius.
    pub hsi_center: [f64; 2],
    pub hsi_radius: f64,
    /// Data fields and the screen pixel x of each centre.
    pub data_fields: Vec<(DataField, f64)>,
    /// Width of the engine column left of the panel in split mode, else zero.
    pub engine_column_px: f64,
}

impl PfdGeometry {
    pub fn rect(&self, element: PfdElement) -> [f64; 4] {
        self.elements
            .iter()
            .find(|(e, _)| *e == element)
            .map_or([0.; 4], |(_, r)| *r)
    }

    /// An element's rectangle on the screen rather than in the panel.
    pub fn screen_rect(&self, element: PfdElement) -> [f64; 4] {
        let [x, y, w, h] = self.rect(element);
        match element {
            PfdElement::DataBar
            | PfdElement::ComBox
            | PfdElement::TransponderBox
            | PfdElement::SplitButton
            | PfdElement::StatusBar => [x, y, w, h],
            _ => [self.panel[0] + x, self.panel[1] + y, w, h],
        }
    }
}

/// Fraction of the screen width the primary flight display (with its engine column) takes in split
/// mode. Measured from the split-screen figure.
pub const SPLIT_PANEL_FRACTION: f64 = 0.563;
/// Fraction of the screen width of the split-mode engine column.
pub const SPLIT_ENGINE_FRACTION: f64 = 0.12;
/// Fraction of the screen height of the bottom status bar.
pub const STATUS_BAR_FRACTION: f64 = 24. / 768.;
/// Fraction of the screen height of the top data bar.
pub const DATA_BAR_FRACTION: f64 = 60. / 768.;

impl DisplayModel {
    /// Where the primary flight display's elements sit in `mode`.
    pub fn pfd_geometry(self, mode: PfdMode) -> PfdGeometry {
        use PfdElement as E;
        let [sw, sh] = self.resolution_px().map(f64::from);
        let status = (
            E::StatusBar,
            [0., sh * (1. - STATUS_BAR_FRACTION), sw, sh * STATUS_BAR_FRACTION],
        );
        match mode {
            PfdMode::Full => PfdGeometry {
                mode,
                panel: [0., 0., sw, sh],
                elements: vec![
                    (E::DataBar, [0., 0., sw, 60.]),
                    (E::ComBox, [0., 0., 238., 60.]),
                    (E::SplitButton, [1212., 8., 56., 43.]),
                    (E::AfcsBox, [448., 65., 368., 33.]),
                    (E::AirspeedTape, [345., 127., 86., 269.]),
                    (E::TrueAirspeedBox, [345., 98., 86., 29.]),
                    (E::GroundSpeedBox, [345., 400., 86., 27.]),
                    (E::AirspeedPointer, [351., 242., 80., 37.]),
                    (E::AltitudeTape, [824., 135., 111., 253.]),
                    (E::SelectedAltitudeBox, [840., 98., 95., 29.]),
                    (E::BaroBox, [840., 399., 95., 32.]),
                    (E::MinimumsBox, [840., 433., 95., 24.]),
                    (E::VerticalSpeedScale, [935., 135., 39., 253.]),
                    (E::AltitudePointer, [868., 242., 140., 37.]),
                    (E::SlipBox, [578., 361., 143., 28.]),
                    (E::TurnRateScale, [545., 407., 190., 24.]),
                    (E::HeadingBox, [608., 423., 64., 34.]),
                    (E::SelectedHeadingBox, [483., 435., 84., 27.]),
                    (E::SelectedCourseBox, [708., 435., 92., 27.]),
                    (E::WindBox, [393., 469., 90., 47.]),
                    (E::CasWindow, [436., 102., 108., 54.]),
                    (E::FlapIndicator, [440., 282., 26., 104.]),
                    (E::TrimIndicator, [472., 282., 26., 104.]),
                    (E::GMeter, [436., 166., 108., 76.]),
                    status,
                ],
                aircraft_symbol: [640., 261.],
                roll_arc_radius: 135.,
                hsi_center: [637., 603.],
                hsi_radius: 131.,
                data_fields: vec![
                    (DataField::Waypoint, 317.),
                    (DataField::Bearing, 431.),
                    (DataField::Distance, 539.),
                    (DataField::TimeEnRoute, 649.),
                    (DataField::GroundSpeed, 771.),
                    (DataField::Track, 887.),
                    (DataField::SafeAltitude, 1006.),
                    (DataField::VerticalSpeedRequired, 1125.),
                ],
                engine_column_px: 0.,
            },
            PfdMode::Split => {
                let left = (sw * SPLIT_ENGINE_FRACTION).round();
                let width = (sw * SPLIT_PANEL_FRACTION).round() - left;
                PfdGeometry {
                    mode,
                    panel: [left, 0., width, sh],
                    elements: vec![
                        (E::DataBar, [0., 0., sw, 60.]),
                        (E::ComBox, [0., 0., 238., 60.]),
                        (E::TransponderBox, [342., 0., 214., 60.]),
                        (E::SplitButton, [1212., 8., 56., 43.]),
                        (E::AfcsBox, [133., 82., 300., 28.]),
                        (E::AirspeedTape, [16., 120., 86., 264.]),
                        (E::TrueAirspeedBox, [16., 92., 86., 28.]),
                        (E::GroundSpeedBox, [16., 414., 86., 24.]),
                        (E::AirspeedPointer, [22., 244., 80., 37.]),
                        (E::AltitudeTape, [434., 144., 95., 264.]),
                        (E::SelectedAltitudeBox, [434., 114., 95., 28.]),
                        (E::BaroBox, [434., 414., 95., 28.]),
                        (E::MinimumsBox, [446., 444., 83., 24.]),
                        (E::VerticalSpeedScale, [529., 144., 34., 264.]),
                        (E::AltitudePointer, [458., 244., 105., 37.]),
                        (E::SlipBox, [215., 384., 136., 28.]),
                        (E::TurnRateScale, [190., 428., 190., 24.]),
                        (E::HeadingBox, [251., 436., 64., 34.]),
                        (E::SelectedHeadingBox, [129., 440., 84., 27.]),
                        (E::SelectedCourseBox, [347., 440., 84., 27.]),
                        (E::WindBox, [37., 480., 90., 42.]),
                        (E::CasWindow, [108., 122., 100., 52.]),
                        (E::FlapIndicator, [110., 284., 24., 100.]),
                        (E::TrimIndicator, [140., 284., 24., 100.]),
                        (E::GMeter, [108., 182., 100., 72.]),
                        status,
                    ],
                    aircraft_symbol: [283., 274.],
                    roll_arc_radius: 135.,
                    hsi_center: [283., 592.],
                    hsi_radius: 118.,
                    data_fields: vec![(DataField::Waypoint, 761.), (DataField::Bearing, 997.)],
                    engine_column_px: left,
                }
            }
        }
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

    // Placement tests follow the arrangement of the full-screen and split-screen figures in the
    // modelled unit's pilot's guide (flight instruments overview and split mode figures).
    use PfdElement as E;
    fn geometry(mode: PfdMode) -> PfdGeometry {
        DisplayModel::LandscapeTouchEfis.pfd_geometry(mode)
    }
    fn right(r: [f64; 4]) -> f64 {
        r[0] + r[2]
    }
    fn bottom(r: [f64; 4]) -> f64 {
        r[1] + r[3]
    }
    fn mid_x(r: [f64; 4]) -> f64 {
        r[0] + r[2] / 2.
    }

    #[test]
    fn full_pfd_puts_the_tapes_either_side_of_a_centred_attitude_and_compass() {
        let g = geometry(PfdMode::Full);
        let [px, py, pw, ph] = g.panel;
        assert_eq!([px, py, pw, ph], [0., 0., 1280., 768.]);
        let speed = g.rect(E::AirspeedTape);
        let altitude = g.rect(E::AltitudeTape);
        // Airspeed left of centre, altitude right of centre, neither beside the compass rose.
        assert!(right(speed) < g.hsi_center[0] - g.hsi_radius);
        assert!(altitude[0] > g.hsi_center[0] + g.hsi_radius);
        // Both tapes start at about the same height and are about as tall.
        assert!((speed[1] - altitude[1]).abs() < 20.);
        assert!((speed[3] - altitude[3]).abs() < 30.);
        // The aircraft symbol sits on the tapes' common centre line.
        let tape_mid = speed[1] + speed[3] / 2.;
        assert!((g.aircraft_symbol[1] - tape_mid).abs() < 10.);
        assert!((g.aircraft_symbol[0] - pw / 2.).abs() < 6.);
        // The compass is centred and sits in the lower third without leaving the panel.
        assert!((g.hsi_center[0] - pw / 2.).abs() < 8.);
        assert!(g.hsi_center[1] > ph * 0.7);
        assert!(g.hsi_center[1] + g.hsi_radius < ph);
        assert!((g.hsi_radius / ph - 0.17).abs() < 0.01);
    }

    #[test]
    fn full_pfd_boxes_sit_where_the_figure_puts_them() {
        let g = geometry(PfdMode::Full);
        let speed = g.rect(E::AirspeedTape);
        let altitude = g.rect(E::AltitudeTape);
        // True airspeed above and ground speed below the airspeed tape, in its column.
        for e in [E::TrueAirspeedBox, E::GroundSpeedBox] {
            assert!((g.rect(e)[0] - speed[0]).abs() < 1.);
        }
        assert!(bottom(g.rect(E::TrueAirspeedBox)) <= speed[1] + 1.);
        assert!(g.rect(E::GroundSpeedBox)[1] >= bottom(speed));
        // Selected altitude above and baro below the altitude tape.
        assert!(bottom(g.rect(E::SelectedAltitudeBox)) <= altitude[1] + 1.);
        assert!(g.rect(E::BaroBox)[1] >= bottom(altitude));
        assert!(g.rect(E::SelectedAltitudeBox)[0] >= altitude[0]);
        // The vertical speed scale adjoins the altitude tape on its right.
        let vsi = g.rect(E::VerticalSpeedScale);
        assert!((vsi[0] - right(altitude)).abs() < 1.);
        assert!((vsi[1] - altitude[1]).abs() < 1. && (vsi[3] - altitude[3]).abs() < 1.);
        // The altitude pointer overlaps the tape and the scale at the tapes' centre line.
        let pointer = g.rect(E::AltitudePointer);
        assert!(pointer[0] < right(altitude) && right(pointer) > vsi[0]);
        // Heading, current heading, and course boxes share a row above the compass.
        let (hdg, cur, crs) = (
            g.rect(E::SelectedHeadingBox),
            g.rect(E::HeadingBox),
            g.rect(E::SelectedCourseBox),
        );
        assert!(right(hdg) < cur[0] && right(cur) < crs[0]);
        assert!(bottom(cur) <= g.hsi_center[1] - g.hsi_radius + 40.);
        assert!((mid_x(cur) - g.hsi_center[0]).abs() < 6.);
        assert!((hdg[1] - crs[1]).abs() < 1.);
        // Wind sits left of the compass, under the airspeed tape.
        let wind = g.rect(E::WindBox);
        assert!(right(wind) < g.hsi_center[0] - g.hsi_radius + 10. && wind[1] > bottom(speed));
    }

    #[test]
    fn afcs_box_is_top_centre_directly_under_the_data_bar() {
        for mode in [PfdMode::Full, PfdMode::Split] {
            let g = geometry(mode);
            let afcs = g.screen_rect(E::AfcsBox);
            let bar = g.rect(E::DataBar);
            assert!(afcs[1] >= bottom(bar));
            assert!(afcs[1] - bottom(bar) < 40.);
            let panel_mid = g.panel[0] + g.panel[2] / 2.;
            assert!((mid_x(afcs) - panel_mid).abs() < 30., "{mode:?}");
            assert!(bottom(afcs) < g.panel[1] + g.aircraft_symbol[1] - g.roll_arc_radius * 0.4);
        }
    }

    #[test]
    fn cas_window_and_the_flap_and_trim_indicators_sit_left_of_the_attitude_symbol_clear_of_the_tapes() {
        // Pilot's guide 3.7 (CAS window) and the trim and flap position figure (page 81): the
        // indicators are on the attitude indicator, the flap scale left of the trim scale.
        for mode in [PfdMode::Full, PfdMode::Split] {
            let g = geometry(mode);
            let (cas, flap, trim) = (
                g.rect(E::CasWindow),
                g.rect(E::FlapIndicator),
                g.rect(E::TrimIndicator),
            );
            let speed = g.rect(E::AirspeedTape);
            let [sx, sy] = g.aircraft_symbol;
            for (name, r) in [("cas", cas), ("flap", flap), ("trim", trim)] {
                assert!(
                    r[2] > 0. && r[0] > right(speed),
                    "{mode:?} {name} clear of the airspeed tape"
                );
                assert!(
                    right(r) < sx - g.roll_arc_radius * 0.5,
                    "{mode:?} {name} left of the symbol"
                );
            }
            assert!(right(flap) <= trim[0] + 0.5, "{mode:?} flap left of trim");
            assert!((flap[1] - trim[1]).abs() < 1. && (flap[3] - trim[3]).abs() < 1.);
            // The CAS window is above the gauges, which sit below the symbol's level.
            assert!(bottom(cas) < flap[1] && flap[1] > sy);
            // Nothing overlaps the wind box, slip box, or selected heading box.
            let overlap = |a: [f64; 4], b: [f64; 4]| {
                a[0] < right(b) && b[0] < right(a) && a[1] < bottom(b) && b[1] < bottom(a)
            };
            for r in [cas, flap, trim] {
                for e in [
                    E::WindBox,
                    E::SlipBox,
                    E::SelectedHeadingBox,
                    E::AfcsBox,
                    E::TrueAirspeedBox,
                ] {
                    assert!(!overlap(r, g.rect(e)), "{mode:?} {e:?}");
                }
            }
        }
    }

    #[test]
    fn data_bar_runs_com_then_eight_fields_then_the_split_button() {
        let g = geometry(PfdMode::Full);
        let bar = g.rect(E::DataBar);
        assert_eq!(bar[2], 1280.);
        assert!((bar[3] / 768. - 0.078).abs() < 0.002);
        let com = g.rect(E::ComBox);
        let labels: Vec<_> = g.data_fields.iter().map(|(f, _)| f.label()).collect();
        assert_eq!(labels, ["WPT", "BRG", "DIST", "ETE", "GS", "TRK", "MSA", "VSR"]);
        assert!(g.data_fields.windows(2).all(|w| w[0].1 < w[1].1));
        assert!(g.data_fields[0].1 > right(com));
        assert!(g.data_fields[7].1 < g.rect(E::SplitButton)[0]);
        let gaps: Vec<f64> = g.data_fields.windows(2).map(|w| w[1].1 - w[0].1).collect();
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        assert!(gaps.iter().all(|x| (x - mean).abs() < mean * 0.2));
    }

    #[test]
    fn split_pfd_takes_a_little_over_half_with_the_engine_column_at_its_left() {
        let g = geometry(PfdMode::Split);
        let [left, _, width, height] = g.panel;
        assert!((left / 1280. - SPLIT_ENGINE_FRACTION).abs() < 0.005);
        assert!(((left + width) / 1280. - SPLIT_PANEL_FRACTION).abs() < 0.005);
        assert_eq!(g.engine_column_px, left);
        assert_eq!(height, 768.);
        // Tapes are pinned to the pane edges; the compass is centred in the pane.
        let speed = g.rect(E::AirspeedTape);
        let vsi = g.rect(E::VerticalSpeedScale);
        assert!(speed[0] < 30. && right(vsi) <= width + 1. && right(vsi) > width - 12.);
        assert!((g.hsi_center[0] - width / 2.).abs() < 6.);
        for e in [
            E::AirspeedTape,
            E::AltitudeTape,
            E::VerticalSpeedScale,
            E::SelectedAltitudeBox,
            E::BaroBox,
            E::WindBox,
            E::SlipBox,
            E::AfcsBox,
        ] {
            let r = g.rect(e);
            assert!(r[0] >= 0. && right(r) <= width + 1., "{e:?}");
        }
        assert!(right(speed) < g.hsi_center[0] - g.hsi_radius);
        assert!(g.rect(E::AltitudeTape)[0] > g.hsi_center[0] + g.hsi_radius);
        assert!(g.rect(E::TransponderBox)[2] > 0.);
        assert_eq!(g.data_fields.len(), 2);
    }

    #[test]
    fn every_element_lies_on_the_screen_in_both_modes() {
        for mode in [PfdMode::Full, PfdMode::Split] {
            let g = geometry(mode);
            for (e, _) in &g.elements {
                let r = g.screen_rect(*e);
                assert!(r[0] >= 0. && r[1] >= 0., "{e:?}");
                assert!(right(r) <= 1280.5 && bottom(r) <= 768.5, "{mode:?} {e:?}");
            }
            assert!(g.rect(E::StatusBar)[1] > 735.);
        }
    }
}
