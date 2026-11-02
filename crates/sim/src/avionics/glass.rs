//! Integrated suite settings and installation wiring; sensors are shared with standby units.
use super::sensor::{AirDataComputer, ComputerInput};
use crate::{Readout, systems::SystemsState};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GlassPage {
    #[default]
    Map,
    Nearest,
    FlightPlan,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MapOrientation {
    #[default]
    NorthUp,
    TrackUp,
    HeadingUp,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransponderMode {
    Off,
    Standby,
    On,
    #[default]
    Alt,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BearingPointer {
    #[default]
    Off,
    Gps,
}
/// A resolved GPS waypoint; independent of a region's airport catalogue.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GpsWaypoint {
    #[schemars(length(min = 1, max = 64))]
    pub ident: String,
    pub kind: WaypointKind,
    pub position: flightcraft_world::GeoPoint,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WaypointKind {
    Airport,
    Navaid,
    Fix,
    Pin,
    Coordinate,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GlassSetting {
    SyntheticVision {
        enabled: bool,
    },
    HeadingBug {
        /// Selected magnetic heading, radians.
        #[schemars(range(min=0,max=core::f64::consts::TAU))]
        heading_rad: f64,
    },
    Course {
        /// Magnetic CRS/OBS, radians.
        #[schemars(range(min=0,max=core::f64::consts::TAU))]
        course_rad: f64,
    },
    SelectedAltitude {
        #[schemars(range(min=-500,max=15000))]
        altitude_m: f64,
    },
    Baro {
        #[schemars(range(min = 80000, max = 110000))]
        pressure_pa: f64,
    },
    PressureUnits {
        hpa: bool,
    },
    BearingPointer {
        source: BearingPointer,
    },
    Transponder {
        #[schemars(length(min=4,max=4),extend("pattern"="^[0-7]{4}$"))]
        code: String,
        mode: TransponderMode,
    },
    MapRange {
        #[schemars(range(min = 1, max = 200))]
        range_nm: f64,
    },
    Orientation {
        orientation: MapOrientation,
    },
    Page {
        page: GlassPage,
    },
    DirectTo {
        airport: String,
    },
    FlightPlan {
        #[schemars(length(max = 64))]
        airports: Vec<String>,
    },
    FlightPlanWaypoints {
        #[schemars(length(max = 64))]
        waypoints: Vec<GpsWaypoint>,
    },
    DirectToWaypoint {
        waypoint: GpsWaypoint,
    },
    ActiveLeg {
        index: usize,
    },
    Failure {
        target: GlassFailure,
        failed: bool,
    },
}
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GlassFailure {
    Ahrs,
    Adc,
    Pfd,
    Mfd,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlassComputers {
    pub primary: AirDataComputer,
    pub standby: AirDataComputer,
    pub ahrs_failed: bool,
    pub adc_failed: bool,
    pub pfd_failed: bool,
    pub mfd_failed: bool,
}
impl GlassComputers {
    pub(crate) fn hash_words(&self) -> Vec<u64> {
        let mut words = vec![
            u64::from(self.ahrs_failed),
            u64::from(self.adc_failed),
            u64::from(self.pfd_failed),
            u64::from(self.mfd_failed),
        ];
        words.extend(self.primary.hash_words());
        words.extend(self.standby.hash_words());
        words
    }
    pub(crate) fn step(
        &mut self,
        systems: &SystemsState,
        state: crate::State,
        truth: Readout,
        advance: bool,
    ) {
        let powered = |id: &str| systems.electrical.loads.iter().any(|l| l.id == id && l.powered);
        self.primary.step(
            systems,
            state,
            truth,
            ComputerInput {
                powered: powered("glass_ahrs"),
                ahrs_failed: self.ahrs_failed,
                adc_failed: self.adc_failed,
                advance,
                ..Default::default()
            },
        );
        self.standby.step(
            systems,
            state,
            truth,
            ComputerInput {
                powered: powered("glass_standby"),
                advance,
                ..Default::default()
            },
        );
    }
}
