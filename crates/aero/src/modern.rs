//! Declarative, fictional equipment; behaviour and public artwork are separate layers.
use crate::aircraft::{positive, require};
use crate::{AircraftError, Table};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EngineControl {
    #[default]
    Magnetos,
    ElectronicDualLane {
        lane_a_load: String,
        lane_b_load: String,
        pump_1_load: String,
        pump_2_load: String,
        single_lane_power_fraction: f64,
        /// Power fraction with only lane A running; defaults to `single_lane_power_fraction`.
        /// The Sling 2 POH allows no drop with lane B off, so its lane A carries the engine fully.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lane_a_only_power_fraction: Option<f64>,
        /// Power fraction with only lane B running; defaults to `single_lane_power_fraction`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lane_b_only_power_fraction: Option<f64>,
        fuel_pressure_pa: f64,
        /// Degrees Celsius against shaft-power fraction.
        coolant_temperature_c: Box<Table>,
        coolant_lag_s: f64,
        /// Absolute inlet pressure / ambient pressure against throttle demand.
        manifold_pressure_fraction: Box<Table>,
    },
}
impl EngineControl {
    pub fn validate(&self) -> Result<(), AircraftError> {
        if let Self::ElectronicDualLane {
            lane_a_load,
            lane_b_load,
            pump_1_load,
            pump_2_load,
            single_lane_power_fraction,
            lane_a_only_power_fraction,
            lane_b_only_power_fraction,
            fuel_pressure_pa,
            coolant_temperature_c,
            coolant_lag_s,
            manifold_pressure_fraction,
        } = self
        {
            let mut ids = BTreeSet::new();
            for load in [lane_a_load, lane_b_load, pump_1_load, pump_2_load] {
                crate::layers::id(load, "engine.control.load")?;
                require(
                    ids.insert(load),
                    "engine.control",
                    "four independent loads required",
                )?;
            }
            require(
                (0.5..1.0).contains(single_lane_power_fraction),
                "single_lane_power_fraction",
                "requires 0.5..1",
            )?;
            for fraction in [lane_a_only_power_fraction, lane_b_only_power_fraction]
                .into_iter()
                .flatten()
            {
                require(
                    (0.5..=1.0).contains(fraction),
                    "lane_only_power_fraction",
                    "requires 0.5..=1",
                )?;
            }
            require(
                (10000.0..=1_000_000.0).contains(fuel_pressure_pa),
                "fuel_pressure_pa",
                "requires 10..1000 kPa",
            )?;
            positive(*coolant_lag_s, "coolant_lag_s")?;
            for (table, lo, hi) in [
                (coolant_temperature_c, -60., 200.),
                (manifold_pressure_fraction, 0.1, 1.),
            ] {
                table.validate("engine.control.table")?;
                require(
                    table.points.first().map(|p| p.0) == Some(0.)
                        && table.points.last().map(|p| p.0) == Some(1.)
                        && table.points.iter().all(|(_, y)| (lo..=hi).contains(y)),
                    "engine.control.table",
                    "requires 0..1 domain and bounded ordinates",
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnnunciationSource {
    LowVoltage,
    LowFuelPressure,
    LaneFault,
    HighCoolantTemperature,
    DoorOpen,
    ParachuteArmed,
    EngineShutdown,
    /// Oil pressure below the threshold (pascals) with the engine running.
    LowOilPressure,
    /// Oil temperature above the threshold (degrees Celsius).
    HighOilTemperature,
    /// A cowl fastener is loose and the engine is making power: airframe vibration.
    CowlVibration,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Caution,
    Warning,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnnunciationDefinition {
    pub source: AnnunciationSource,
    pub severity: Severity,
    pub label: String,
    /// Volts, pascals, or degrees Celsius, for threshold sources; zero otherwise.
    pub threshold: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParachuteDefinition {
    pub hold_s: f64,
    pub deployment_delay_s: f64,
    pub inflation_s: f64,
    pub descent_rate_mps: f64,
    pub hanging_pitch_rad: f64,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModernSystems {
    #[serde(default)]
    #[schemars(length(max = 2))]
    pub backup_buses: Vec<crate::panel_fidelity::BackupBus>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wigwag: Option<crate::panel_fidelity::Wigwag>,
    #[serde(default)]
    pub selector_both: bool,
    #[serde(default)]
    #[schemars(length(max = 32))]
    pub annunciations: Vec<AnnunciationDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parachute: Option<ParachuteDefinition>,
}
impl ModernSystems {
    pub fn validate(&self) -> Result<(), AircraftError> {
        require(self.backup_buses.len() <= 2, "backup_buses", "at most two buses")?;
        let mut circuits = BTreeSet::new();
        let mut loads = BTreeSet::new();
        for bus in &self.backup_buses {
            require(
                circuits.insert(bus.circuit as u8)
                    && (0.01..=100.).contains(&bus.capacity_ah)
                    && (9. ..=32.).contains(&bus.voltage)
                    && (1..=64).contains(&bus.loads.len()),
                "backup_bus",
                "invalid circuit, capacity, voltage, or load count",
            )?;
            for id in &bus.loads {
                crate::layers::id(id, "backup_bus.load")?;
                require(
                    loads.insert(id),
                    "backup_bus.load",
                    "load belongs to only one backup bus",
                )?;
            }
        }
        if let Some(w) = &self.wigwag {
            crate::layers::id(&w.left_load, "wigwag.left_load")?;
            crate::layers::id(&w.right_load, "wigwag.right_load")?;
            require(
                w.left_load != w.right_load && (0.1..=5.).contains(&w.half_period_s),
                "wigwag",
                "requires distinct lights and 0.1..5 second half period",
            )?;
        }
        require(
            self.annunciations.len() <= 32,
            "annunciations",
            "at most 32 lamps",
        )?;
        let mut sources = BTreeSet::new();
        for a in &self.annunciations {
            require(
                sources.insert(a.source as u8),
                "annunciations",
                "duplicate source",
            )?;
            require(
                !a.label.is_empty()
                    && a.label.len() <= 64
                    && !a.label.chars().any(char::is_control)
                    && a.threshold.is_finite()
                    && (0.0..=1_000_000.).contains(&a.threshold),
                "annunciations",
                "invalid label or threshold",
            )?;
        }
        if let Some(p) = &self.parachute {
            for value in [p.hold_s, p.deployment_delay_s, p.inflation_s] {
                require(
                    value.is_finite() && (0.1..=60.).contains(&value),
                    "parachute.time",
                    "requires 0.1..60 seconds",
                )?;
            }
            require(
                (1.0..=20.).contains(&p.descent_rate_mps) && (-1.2..=-0.05).contains(&p.hanging_pitch_rad),
                "parachute",
                "descent 1..20 m/s and nose-down pitch required",
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TouchLayout {
    IntegratedPfd,
    FullPfd,
    #[default]
    PfdMap,
    /// Landscape split screen: the PFD beside one MFD pane showing the display's `MfdPage`.
    Split,
    MapEngine,
    /// Primary flight display with synthetic vision always on.
    Svt,
    /// Large moving map with a configurable data area ([`DataArea`]) beside it.
    Mfd,
    /// Full-screen engine page.
    Engine,
    /// Full-screen flight-plan list.
    FlightPlan,
    /// Full-screen nearest-airports list.
    Nearest,
    /// Full-screen traffic list.
    Traffic,
}
impl TouchLayout {
    /// Pages that show the primary flight display (attitude and tapes).
    pub fn is_pfd(self) -> bool {
        matches!(
            self,
            Self::IntegratedPfd | Self::FullPfd | Self::PfdMap | Self::Split | Self::Svt
        )
    }
}
/// The pages of a landscape touch EFIS multi-function display, in page-bar order.
///
/// `Map`, `Waypoint`, `FlightPlan`, `Terrain`, `Traffic`, `Info`, and `Engine` are the main pages;
/// `Nearest` is reached with the NEAREST key and `Cas` from the CAS window or Select Page; neither is
/// part of the knob cycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MfdPage {
    #[default]
    Map,
    Waypoint,
    FlightPlan,
    Terrain,
    Traffic,
    Info,
    Engine,
    Nearest,
    /// The crew alerting message list, opened from the CAS window's triangle or Select Page.
    Cas,
}
impl MfdPage {
    /// The main pages the MFD knob cycles through.
    pub const MAIN: [Self; 7] = [
        Self::Map,
        Self::Waypoint,
        Self::FlightPlan,
        Self::Terrain,
        Self::Traffic,
        Self::Info,
        Self::Engine,
    ];
    /// Abbreviated name shown in the page bar.
    pub fn abbreviation(self) -> &'static str {
        match self {
            Self::Map => "Map",
            Self::Waypoint => "Wpt",
            Self::FlightPlan => "FPL",
            Self::Terrain => "Ter",
            Self::Traffic => "Trf",
            Self::Info => "Info",
            Self::Engine => "Eng",
            Self::Nearest => "Nrst",
            Self::Cas => "CAS",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Map => "Map",
            Self::Waypoint => "Waypoint",
            Self::FlightPlan => "Active flight plan",
            Self::Terrain => "Terrain",
            Self::Traffic => "Traffic",
            Self::Info => "Info",
            Self::Engine => "Engine",
            Self::Nearest => "Nearest",
            Self::Cas => "CAS messages",
        }
    }
    /// The page a legacy full-screen layout shows, if it is one.
    pub fn of_layout(layout: TouchLayout) -> Option<Self> {
        match layout {
            TouchLayout::Mfd | TouchLayout::MapEngine => Some(Self::Map),
            TouchLayout::Engine => Some(Self::Engine),
            TouchLayout::FlightPlan => Some(Self::FlightPlan),
            TouchLayout::Nearest => Some(Self::Nearest),
            TouchLayout::Traffic => Some(Self::Traffic),
            _ => None,
        }
    }
}
/// A page of the nearest group, chosen on the page navigation bar once NEAREST is pressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NearestPage {
    #[default]
    Airports,
    Vors,
    Ndbs,
    Intersections,
    Airspace,
}
impl NearestPage {
    /// The modelled nearest pages, in the order of the unit's list. Weather, user waypoints, cities,
    /// control centres, flight service stations, and visual reporting points are not modelled.
    pub const ALL: [Self; 5] = [
        Self::Airports,
        Self::Vors,
        Self::Ndbs,
        Self::Intersections,
        Self::Airspace,
    ];
    pub fn abbreviation(self) -> &'static str {
        match self {
            Self::Airports => "Apt",
            Self::Vors => "VOR",
            Self::Ndbs => "NDB",
            Self::Intersections => "INT",
            Self::Airspace => "ASPC",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Airports => "Nearest airports",
            Self::Vors => "Nearest VORs",
            Self::Ndbs => "Nearest NDBs",
            Self::Intersections => "Nearest intersections",
            Self::Airspace => "Nearest airspace",
        }
    }
    /// Text shown when the list is empty.
    pub fn empty_text(self) -> &'static str {
        match self {
            Self::Airspace => "No Immediate Airspace",
            _ => "None Within 200 NM",
        }
    }
}
/// What the MFD page's data column shows beside the map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataArea {
    #[default]
    Engine,
    FlightPlan,
    Nearest,
    Traffic,
}
/// Where the pilot sits. A right-seat pilot gets the primary flight display on the right screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PilotSeat {
    #[default]
    Left,
    Right,
}
impl TouchLayout {
    /// The pages a pilot can choose between, in menu order. The PFD entry is the display's installed
    /// layout (`installed`), so it keeps the composition the pack designed.
    pub fn pages(installed: Self) -> [Self; 10] {
        [
            installed,
            Self::Svt,
            Self::PfdMap,
            Self::Split,
            Self::Mfd,
            Self::MapEngine,
            Self::Engine,
            Self::FlightPlan,
            Self::Nearest,
            Self::Traffic,
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::IntegratedPfd | Self::FullPfd => "PFD",
            Self::PfdMap => "PFD + map",
            Self::Split => "Split screen (PFD + MFD page)",
            Self::MapEngine => "Map + engine strip",
            Self::Svt => "SVT (synthetic vision PFD)",
            Self::Mfd => "MFD (map + data)",
            Self::Engine => "Engine",
            Self::FlightPlan => "Flight plan",
            Self::Nearest => "Nearest",
            Self::Traffic => "Traffic",
        }
    }
}
impl DataArea {
    pub const ALL: [Self; 4] = [Self::Engine, Self::FlightPlan, Self::Nearest, Self::Traffic];
    pub fn label(self) -> &'static str {
        match self {
            Self::Engine => "Engine",
            Self::FlightPlan => "Flight plan",
            Self::Nearest => "Nearest",
            Self::Traffic => "Traffic",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanelColor {
    Panel,
    Raised,
    Dial,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanelShape {
    Rectangle,
    Rounded,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Rocker {
    Beacon,
    Nav,
    Strobe,
    Taxi,
    Landing,
    Master,
    Avionics,
    Alternator,
    Lights,
    PitotHeat,
    LaneA,
    LaneB,
    Pump1,
    Pump2,
    EmsBackup,
    EfisBackup,
    LandingWigwag,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanelInk {
    #[default]
    Text,
    Muted,
    Amber,
    Red,
    White,
    Accent,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outline {
    Polygon { points: Vec<[f64; 2]> },
    Cubic { curves: Vec<[[f64; 2]; 4]> },
}
impl Outline {
    fn validate(&self, [w, h]: [f64; 2]) -> Result<(), AircraftError> {
        let points: Vec<_> = match self {
            Self::Polygon { points } => {
                require(
                    (3..=128).contains(&points.len()),
                    "outline",
                    "requires 3..128 polygon vertices",
                )?;
                points.clone()
            }
            Self::Cubic { curves } => {
                require(
                    (1..=32).contains(&curves.len()),
                    "outline",
                    "requires 1..32 cubic curves",
                )?;
                for pair in curves.windows(2) {
                    if let [a, b] = pair {
                        let [_, _, _, end] = a;
                        let [start, _, _, _] = b;
                        require(end == start, "outline", "curves must join")?;
                    }
                }
                if let (Some([start, _, _, _]), Some([_, _, _, end])) = (curves.first(), curves.last()) {
                    require(start == end, "outline", "outline must close")?;
                }
                curves.iter().flatten().copied().collect()
            }
        };
        require(
            points.iter().all(|[x, y]| {
                x.is_finite() && y.is_finite() && (0. ..=w).contains(x) && (0. ..=h).contains(y)
            }),
            "outline",
            "control points must fit panel",
        )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelShell {
    #[serde(default)]
    pub palette: crate::panel_fidelity::PanelPalette,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub centre_console: Option<Outline>,
    /// The cabin floor under the centre console, drawn behind it in the console window: the
    /// footwells that the control sticks stand in.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<Outline>,
    pub outline: Outline,
    pub fill: PanelColor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glare_shield: Option<Outline>,
    #[serde(default)]
    pub screws: Vec<[f64; 2]>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EngineStripStyle {
    #[default]
    Bars,
    DialsAndBars,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DisplayInset {
    Map,
    Terrain,
    SyntheticVision,
    Engine,
    Navigation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PfdCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PfdInset {
    pub corner: PfdCorner,
    pub content: DisplayInset,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisplayComposition {
    #[serde(default)]
    pub top_data_bar: bool,
    #[serde(default = "strip_fraction")]
    pub engine_strip_fraction: f64,
    #[serde(default)]
    pub engine_style: EngineStripStyle,
    #[serde(default = "with_hsi")]
    pub hsi: bool,
    #[serde(default)]
    pub insets: Vec<PfdInset>,
}
fn strip_fraction() -> f64 {
    0.18
}
fn with_hsi() -> bool {
    true
}
impl Default for DisplayComposition {
    fn default() -> Self {
        Self {
            top_data_bar: false,
            engine_strip_fraction: strip_fraction(),
            engine_style: Default::default(),
            hsi: true,
            insets: Vec::new(),
        }
    }
}

/// Token in placard text that is replaced at draw time by the aircraft's current registration.
pub const REGISTRATION_TOKEN: &str = "{registration}";
/// Longest registration a placard can be asked to show. It is shorter than the token, so
/// expansion never lengthens placard text and the 128-character limit holds for the result.
pub const MAX_REGISTRATION_LEN: usize = 12;
const _: () = assert!(MAX_REGISTRATION_LEN <= REGISTRATION_TOKEN.len());

/// Placard text with `{registration}` replaced by `registration`.
pub fn expand_placard_text(text: &str, registration: &str) -> String {
    let registration = registration
        .get(..registration.len().min(MAX_REGISTRATION_LEN))
        .unwrap_or(registration);
    text.replace(REGISTRATION_TOKEN, registration)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelElement {
    AudioPanel,
    Label {
        text: String,
    },
    Placard {
        text: String,
        font_size_mm: f64,
        color: PanelInk,
    },
    Toggle {
        label: String,
        #[serde(default)]
        guarded: bool,
        #[serde(default)]
        lamp: bool,
    },
    Dimmer {
        label: String,
    },
    PushPull {
        label: String,
    },
    Vent,
    Elt {
        label: String,
    },
    PitchWheel {
        label: String,
    },
    Lamp {
        label: String,
        color: PanelInk,
    },
    Bracket {
        label: String,
    },
    GuardBar,
    TouchDisplay {
        load: String,
        /// The installed primary-flight-display layout; also what the PFD soft key selects.
        layout: TouchLayout,
        /// The page shown at power-up. Omission shows `layout`, as packs always have. A two-display
        /// installation declares `mfd` here on the screen that is the multi-function display.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_page: Option<TouchLayout>,
        engine_strip: bool,
        #[serde(default)]
        composition: DisplayComposition,
    },
    Navigator {
        load: String,
        transponder: bool,
        /// Host this aircraft panel's shared `audio_panel` installation on an Audio page.
        #[serde(default)]
        audio_panel: bool,
    },
    Standby,
    Autopilot,
    Breaker {
        load: String,
        label: String,
    },
    Rocker {
        switch: Rocker,
        label: String,
        #[serde(default)]
        backlit: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sublabel: Option<String>,
    },
    MasterLight {
        severity: Severity,
    },
    Key,
    FuelSelector,
    Parachute,
    /// Throttle lever on the cabin centre console. Forward (up) is more power; it sets the pilot's
    /// throttle, as the keyboard, joystick, and `sim.set_controls` do.
    Throttle {
        label: String,
    },
    /// Hand brake lever on the centre console: held, it applies the wheel brakes.
    BrakeLever {
        label: String,
    },
    /// Brake intercept valve handle, the parking brake: closed, it keeps the brakes applied.
    ParkBrake {
        label: String,
    },
    /// Rotary flap selector with one detent at each of the aircraft's flap stages.
    FlapSelector {
        label: String,
    },
    /// The pilot's control-stick grip seen from above, nose up the page, with the POH button
    /// allocation: 1 trim down (forward), 3 trim up (aft), 2 autopilot, 4 not allocated, 5 push to
    /// talk. The trim buttons are live; the sticks themselves are flown with the flight controls.
    ControlStick {
        label: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EquipmentInstallation {
    #[serde(default)]
    pub appearance: crate::panel_fidelity::EquipmentAppearance,
    pub id: String,
    pub position: [f64; 2],
    pub size: [f64; 2],
    pub element: PanelElement,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EquipmentPanel {
    /// Optional shell artwork. All coordinates and dimensions are millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<PanelShell>,
    pub size: [f64; 2],
    /// Optional essential rectangle [left, top, width, height], in panel millimetres.
    /// Short viewports anchor cropping around this area without distorting the panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub essential: Option<[f64; 4]>,
    pub color: PanelColor,
    pub shape: PanelShape,
    #[schemars(length(min = 1, max = 128))]
    pub elements: Vec<EquipmentInstallation>,
}
impl EquipmentPanel {
    pub fn validate(&self) -> Result<(), AircraftError> {
        require(
            self.size
                .iter()
                .all(|v| v.is_finite() && (1.0..=10000.).contains(v)),
            "panel.size",
            "requires 1..10000",
        )?;
        if let Some([x, y, w, h]) = self.essential {
            let [pw, ph] = self.size;
            require(
                [x, y, w, h].iter().all(|v| v.is_finite())
                    && x >= 0.
                    && y >= 0.
                    && w > 0.
                    && h > 0.
                    && x + w <= pw
                    && y + h <= ph,
                "panel.essential",
                "requires a positive rectangle within the panel",
            )?;
        }
        require(
            (1..=128).contains(&self.elements.len()),
            "panel.elements",
            "requires 1..128 elements",
        )?;
        if let Some(shell) = &self.shell {
            shell.outline.validate(self.size)?;
            shell.palette.validate()?;
            if let Some(console) = &shell.centre_console {
                console.validate(self.size)?;
            }
            if let Some(floor) = &shell.floor {
                floor.validate(self.size)?;
            }
            if let Some(glare) = &shell.glare_shield {
                glare.validate(self.size)?;
            }
            require(shell.screws.len() <= 128, "shell.screws", "at most 128 screws")?;
            let [w, h] = self.size;
            require(
                shell.screws.iter().all(|[x, y]| {
                    x.is_finite() && y.is_finite() && (0. ..=w).contains(x) && (0. ..=h).contains(y)
                }),
                "shell.screws",
                "must fit panel",
            )?;
        }
        let mut ids = BTreeSet::new();
        let [width, height] = self.size;
        let mut navigators = 0;
        for e in &self.elements {
            crate::layers::id(&e.id, "panel.elements.id")?;
            e.appearance.validate(e.size)?;
            if let Some(shell) = &self.shell {
                use crate::panel_fidelity::SrgbHex;
                let ink = shell
                    .palette
                    .placard_text
                    .clone()
                    .unwrap_or(SrgbHex("#e4edf2".into()));
                let face = shell.palette.face.clone().unwrap_or(SrgbHex(
                    match shell.fill {
                        PanelColor::Panel => "#131d28",
                        PanelColor::Raised => "#1d2b39",
                        PanelColor::Dial => "#0a121a",
                    }
                    .into(),
                ));
                require(
                    ink.contrast(&face).is_some_and(|v| v >= 4.5),
                    "panel.palette",
                    "placards require contrast of at least 4.5:1",
                )?;
                if let Some(b) = &e.appearance.bezel {
                    let bg = b
                        .color
                        .as_ref()
                        .or(shell.palette.bezel.as_ref())
                        .cloned()
                        .unwrap_or(SrgbHex("#0a121a".into()));
                    require(
                        ink.contrast(&bg).is_some_and(|v| v >= 4.5),
                        "display.bezel.color",
                        "bezel labels require contrast of at least 4.5:1",
                    )?;
                }
            }
            if let Some(b) = &e.appearance.bezel
                && self.shell.is_none()
                && let Some(bg) = &b.color
            {
                require(
                    crate::panel_fidelity::SrgbHex("#e4edf2".into())
                        .contrast(bg)
                        .is_some_and(|v| v >= 4.5),
                    "display.bezel.color",
                    "bezel labels require contrast of at least 4.5:1",
                )?;
            }
            if let Some(layout) = &e.appearance.controller {
                let background = self
                    .shell
                    .as_ref()
                    .and_then(|s| s.palette.bezel.clone())
                    .unwrap_or(crate::panel_fidelity::SrgbHex("#0a121a".into()));
                for control in &layout.controls {
                    if let Some(color) = &control.lit_color {
                        require(
                            color.contrast(&background).is_some_and(|v| v >= 4.5),
                            "controller.lit_color",
                            "lit labels require contrast of at least 4.5:1",
                        )?;
                    }
                }
            }
            use crate::panel_fidelity::{ControlStyle as S, NavigatorLayout};
            let a = &e.appearance;
            let compatible_style = match a.style {
                None => true,
                Some(S::BlackRocker) => matches!(e.element, PanelElement::Rocker { .. }),
                Some(S::PushPullBreaker) => matches!(e.element, PanelElement::Breaker { .. }),
                Some(S::RoundedPullHandle) => matches!(e.element, PanelElement::Parachute),
                Some(S::SilverKey) => matches!(e.element, PanelElement::Key),
                Some(S::LouvredEyeball) => matches!(e.element, PanelElement::Vent),
                Some(S::EltPlate) => matches!(e.element, PanelElement::Elt { .. }),
                Some(S::RedTHandle) => matches!(e.element, PanelElement::FuelSelector),
                Some(S::ArcDimmer) => matches!(e.element, PanelElement::Dimmer { .. }),
                Some(S::CabinHeat) => matches!(e.element, PanelElement::PushPull { .. }),
            };
            require(
                compatible_style
                    && (a.binding.is_none() || matches!(e.element, PanelElement::Toggle { .. }))
                    && (a.bezel.is_none() || matches!(e.element, PanelElement::TouchDisplay { .. }))
                    && (a.controller.is_none() || matches!(e.element, PanelElement::Autopilot))
                    && (a.navigator_layout == NavigatorLayout::Classic
                        || matches!(e.element, PanelElement::Navigator { .. }))
                    && (!a.synthetic_vision || matches!(e.element, PanelElement::TouchDisplay { .. })),
                "equipment.appearance",
                "option incompatible with element kind",
            )?;
            require(ids.insert(&e.id), "panel.elements.id", "duplicate installation")?;
            let [x, y] = e.position;
            let [w, h] = e.size;
            require(
                [x, y, w, h].iter().all(|v| v.is_finite())
                    && x >= 0.
                    && y >= 0.
                    && w > 0.
                    && h > 0.
                    && x + w <= width
                    && y + h <= height,
                "panel.elements.bounds",
                "equipment must fit panel",
            )?;
            let valid_text = |t: &str| !t.is_empty() && t.len() <= 128 && !t.chars().any(char::is_control);
            match &e.element {
                PanelElement::Placard {
                    text, font_size_mm, ..
                } => require(
                    valid_text(text) && font_size_mm.is_finite() && (0.5..=30.).contains(font_size_mm),
                    "placard",
                    "invalid text or font size",
                )?,
                PanelElement::Label { text } => require(valid_text(text), "label", "invalid text")?,
                PanelElement::Dimmer { label }
                | PanelElement::PushPull { label }
                | PanelElement::Elt { label }
                | PanelElement::PitchWheel { label }
                | PanelElement::Throttle { label }
                | PanelElement::BrakeLever { label }
                | PanelElement::ParkBrake { label }
                | PanelElement::FlapSelector { label }
                | PanelElement::ControlStick { label }
                | PanelElement::Lamp { label, .. }
                | PanelElement::Bracket { label }
                | PanelElement::Toggle { label, .. } => {
                    require(valid_text(label), "panel.label", "invalid label")?
                }
                PanelElement::TouchDisplay { composition, .. } => require(
                    composition.insets.len() <= 4
                        && composition
                            .insets
                            .iter()
                            .map(|i| i.corner as u8)
                            .collect::<BTreeSet<_>>()
                            .len()
                            == composition.insets.len()
                        && composition.engine_strip_fraction.is_finite()
                        && (0.1..=0.3).contains(&composition.engine_strip_fraction),
                    "display.composition",
                    "engine strip fraction requires 0.1..0.3",
                )?,
                _ => {}
            }
            match &e.element {
                PanelElement::TouchDisplay { load, .. }
                | PanelElement::Navigator { load, .. }
                | PanelElement::Breaker { load, .. } => crate::layers::id(load, "panel.element.load")?,
                _ => {}
            }
            if matches!(e.element, PanelElement::Navigator { .. }) {
                navigators += 1;
            }
            if let PanelElement::Rocker {
                sublabel: Some(label),
                ..
            } = &e.element
            {
                require(valid_text(label), "rocker.sublabel", "invalid text")?;
            }
            if let PanelElement::Rocker { label, .. } | PanelElement::Breaker { label, .. } = &e.element {
                require(
                    !label.is_empty() && label.len() <= 32 && !label.chars().any(char::is_control),
                    "panel.label",
                    "requires 1..32 printable bytes",
                )?;
            }
        }
        require(
            navigators <= 1,
            "panel.navigator",
            "one shared navigator supported",
        )
    }
}

/// Validate cross-layer equipment references, including public native compositions.
pub fn validate_installation(
    e: &crate::Engine,
    systems: Option<&crate::SystemsDefinition>,
    p: &crate::Panel,
) -> Result<(), AircraftError> {
    use crate::aircraft::error;
    if let Some(panel) = &p.equipment {
        panel.validate()?;
        if panel.elements.iter().any(|i| i.appearance.binding.is_some()) {
            require(
                matches!(e.control, EngineControl::ElectronicDualLane { .. }),
                "panel.binding",
                "lane binding requires electronic engine",
            )?;
        }
    }
    if let EngineControl::ElectronicDualLane {
        lane_a_load,
        lane_b_load,
        pump_1_load,
        pump_2_load,
        ..
    } = &e.control
    {
        let d = systems.ok_or_else(|| error("engine.control", "operational systems required"))?;
        for load in [lane_a_load, lane_b_load, pump_1_load, pump_2_load] {
            require(
                d.electrical
                    .loads
                    .iter()
                    .any(|l| &l.id == load && l.switch == crate::LoadSwitch::Always),
                "engine.control.load",
                "requires independent always-connected loads controlled by lane and pump switches",
            )?;
        }
        for load in [lane_a_load, lane_b_load] {
            require(
                d.electrical
                    .loads
                    .iter()
                    .any(|l| &l.id == load && l.backup_capacity_ah > 0.),
                "engine.control.backup",
                "each lane needs backup power",
            )?;
        }
    }
    if let Some(panel) = &p.equipment {
        let d = systems.ok_or_else(|| error("panel.equipment", "operational systems required"))?;
        for element in &panel.elements {
            if matches!(
                element.element,
                PanelElement::Navigator {
                    audio_panel: true,
                    ..
                }
            ) {
                for load in [
                    &p.audio_panel.power_load,
                    &p.audio_panel.com1_load,
                    &p.audio_panel.com2_load,
                ] {
                    require(
                        d.electrical.loads.iter().any(|l| &l.id == load),
                        "panel.navigator.audio_panel",
                        "hosted audio and COM loads must exist",
                    )?;
                }
            }
            match &element.element {
                PanelElement::TouchDisplay { load, .. }
                | PanelElement::Navigator { load, .. }
                | PanelElement::Breaker { load, .. } => require(
                    d.electrical.loads.iter().any(|l| &l.id == load),
                    "panel.load",
                    "unknown load",
                )?,
                PanelElement::Rocker {
                    switch: Rocker::EmsBackup | Rocker::EfisBackup,
                    ..
                } => {
                    let wanted = if matches!(
                        element.element,
                        PanelElement::Rocker {
                            switch: Rocker::EmsBackup,
                            ..
                        }
                    ) {
                        crate::panel_fidelity::BackupCircuit::Ems
                    } else {
                        crate::panel_fidelity::BackupCircuit::Efis
                    };
                    require(
                        d.modern
                            .as_ref()
                            .is_some_and(|m| m.backup_buses.iter().any(|b| b.circuit == wanted)),
                        "panel.backup_switch",
                        "backup bus not installed",
                    )?;
                }
                PanelElement::Rocker {
                    switch: Rocker::LandingWigwag,
                    ..
                } => require(
                    d.modern.as_ref().is_some_and(|m| m.wigwag.is_some()),
                    "panel.wigwag",
                    "alternating lights not installed",
                )?,
                PanelElement::Parachute => require(
                    d.modern.as_ref().is_some_and(|m| m.parachute.is_some()),
                    "panel.parachute",
                    "not installed in systems",
                )?,
                PanelElement::Autopilot => {
                    require(p.autopilot.is_some(), "panel.autopilot", "not installed")?
                }
                _ => {}
            }
        }
    }
    Ok(())
}
