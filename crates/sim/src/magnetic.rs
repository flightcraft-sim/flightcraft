//! Aircraft heading reference. Geometry stays in the regional grid; pilot bearings are magnetic.
use crate::State;
use flightcraft_world::{
    TerrainError, World,
    geomag::{MagneticModel, SimulationDate},
};
use schemars::JsonSchema;
use serde::Serialize;
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct HeadingReference {
    /// East-positive WMM declination at the aircraft and simulation date.
    pub declination_rad: f64,
    /// True bearing of local grid north at the aircraft.
    pub grid_north_true_rad: f64,
    pub geographic: bool,
}
impl HeadingReference {
    pub fn true_to_magnetic(self, angle: f64) -> f64 {
        (angle - self.declination_rad).rem_euclid(core::f64::consts::TAU)
    }
    pub fn magnetic_to_true(self, angle: f64) -> f64 {
        (angle + self.declination_rad).rem_euclid(core::f64::consts::TAU)
    }
    pub fn grid_to_magnetic(self, angle: f64) -> f64 {
        self.true_to_magnetic(angle + self.grid_north_true_rad)
    }
    pub fn magnetic_to_grid(self, angle: f64) -> f64 {
        (self.magnetic_to_true(angle) - self.grid_north_true_rad).rem_euclid(core::f64::consts::TAU)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Magnetism {
    pub reference: HeadingReference,
    pub date: SimulationDate,
    model: MagneticModel,
}
impl Magnetism {
    pub fn new(world: &World, state: State) -> Result<Self, TerrainError> {
        let mut value = Self {
            reference: HeadingReference::default(),
            date: SimulationDate::default(),
            model: MagneticModel::wmm2025()?,
        };
        value.refresh(world, state);
        Ok(value)
    }
    pub fn refresh(&mut self, world: &World, state: State) {
        let [n, e, down] = state.position_m;
        self.reference = (|| {
            let [latitude, longitude] = world.geodetic(n, e)?;
            let position = [latitude, longitude];
            let north = world.geodetic(n + 10., e)?;
            let field = self
                .model
                .field(
                    latitude,
                    longitude,
                    (-down).clamp(-1000., 850000.),
                    self.date.decimal_year().ok()?,
                )
                .ok()?;
            Some(HeadingReference {
                declination_rad: field.declination_deg.to_radians(),
                grid_north_true_rad: crate::avionics::navigation::inverse(position, north).1,
                geographic: true,
            })
        })()
        .unwrap_or_default();
    }
}

/// Resolve a command's magnetic reference before a new simulation is constructed.
pub fn reference_at(
    world: &World,
    position_m: [f64; 3],
    date: SimulationDate,
) -> Result<HeadingReference, TerrainError> {
    date.decimal_year()?;
    let state = State {
        position_m,
        ..State::default()
    };
    let mut value = Magnetism::new(world, state)?;
    value.date = date;
    value.refresh(world, state);
    Ok(value.reference)
}
