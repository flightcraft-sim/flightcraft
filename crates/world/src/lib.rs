//! Deterministic atmosphere, height tiles, and the original Corvid Field.
#![deny(clippy::disallowed_methods)]

pub mod airspace;
pub use airspace::*;
mod airports;
pub mod geo;
pub use airports::*;
mod region;
pub use region::*;
pub mod srtm;
mod terrain;
pub use terrain::*;
mod conditions;
pub use conditions::*;
mod weather;
pub use weather::*;

/// Standard gravitational acceleration, m/s².
pub const GRAVITY_MPS2: f64 = 9.80665;
/// Specific gas constant of dry air, J/(kg K).
pub const GAS_CONSTANT: f64 = 287.05287;
pub const SEA_LEVEL_DENSITY: f64 = 1.225;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atmosphere {
    pub temperature_k: f64,
    pub pressure_pa: f64,
    pub density_kg_m3: f64,
    pub speed_of_sound_mps: f64,
}

/// ISO 2533 / US Standard Atmosphere 1976 lapse-rate equations. Input is geopotential
/// altitude; clamped to 0–20 km. Non-finite input selects sea level.
#[must_use]
pub fn atmosphere(height_m: f64) -> Atmosphere {
    let h = if height_m.is_finite() {
        height_m.clamp(0.0, 20_000.0)
    } else {
        0.0
    };
    let t11 = 216.65;
    let p11 = 101_325.0 * libm::pow(t11 / 288.15, GRAVITY_MPS2 / (0.0065 * GAS_CONSTANT));
    let (t, p) = if h <= 11_000.0 {
        let t = 288.15 - 0.0065 * h;
        (
            t,
            101_325.0 * libm::pow(t / 288.15, GRAVITY_MPS2 / (0.0065 * GAS_CONSTANT)),
        )
    } else {
        (
            t11,
            p11 * libm::exp(-GRAVITY_MPS2 * (h - 11_000.0) / (GAS_CONSTANT * t11)),
        )
    };
    Atmosphere {
        temperature_k: t,
        pressure_pa: p,
        density_kg_m3: p / (GAS_CONSTANT * t),
        speed_of_sound_mps: libm::sqrt(1.4 * GAS_CONSTANT * t),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Grass,
    Pavement,
    Water,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Runway {
    pub length_m: f64,
    pub width_m: f64,
    pub heading_deg: f64,
    pub threshold_ned_m: [f64; 3],
}

pub const RUNWAY: Runway = Runway {
    length_m: 900.0,
    width_m: 23.0,
    heading_deg: 360.0,
    threshold_ned_m: [0.0; 3],
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Airfield {
    pub name: &'static str,
    pub runway: Runway,
}
pub const AIRFIELD: Airfield = Airfield {
    name: "Corvid Field",
    runway: RUNWAY,
};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, schemars::JsonSchema)]
pub struct Ground {
    pub height_msl_m: f64,
    /// Upward unit normal in the local NED frame.
    pub normal_ned: [f64; 3],
    pub surface: Surface,
}

/// Flat terrain; runway 36 extends north (+x) from the NED origin.
#[must_use]
pub fn ground(north_m: f64, east_m: f64) -> Ground {
    let paved = north_m.is_finite()
        && east_m.is_finite()
        && (0.0..=RUNWAY.length_m).contains(&north_m)
        && east_m.abs() <= RUNWAY.width_m * 0.5;
    Ground {
        height_msl_m: 0.0,
        normal_ned: [0.0, 0.0, -1.0],
        surface: if paved { Surface::Pavement } else { Surface::Grass },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isa_reference_table() {
        // US Standard Atmosphere 1976, geopotential heights; rounded table values.
        for (h, t, p, rho, a) in [
            (0.0, 288.15, 101325.0, 1.225, 340.294),
            (5000.0, 255.65, 54019.9, 0.736116, 320.529),
            (11000.0, 216.65, 22632.1, 0.363918, 295.069),
            (15000.0, 216.65, 12044.6, 0.193674, 295.069),
            (20000.0, 216.65, 5474.89, 0.0880349, 295.069),
        ] {
            let v = atmosphere(h);
            assert!((v.temperature_k - t).abs() < 0.001);
            assert!((v.pressure_pa - p).abs() / p < 0.00001);
            assert!((v.density_kg_m3 - rho).abs() / rho < 0.00001);
            assert!((v.speed_of_sound_mps - a).abs() < 0.001);
        }
    }
    #[test]
    fn runway_and_hostile_input() {
        assert_eq!(ground(450.0, 11.5).surface, Surface::Pavement);
        assert_eq!(ground(-0.01, 0.0).surface, Surface::Grass);
        assert_eq!(ground(900.01, 0.0).surface, Surface::Grass);
        assert_eq!(ground(10.0, 11.51).surface, Surface::Grass);
        assert_eq!(atmosphere(f64::NAN), atmosphere(0.0));
        assert_eq!(atmosphere(1e300), atmosphere(20000.0));
    }
}

pub mod geomag;

pub mod navdata;
pub use navdata::{GeoPoint, MAX_NAVDATA_BYTES, Navdata};

pub mod procedures;
pub use procedures::{MAX_PROCEDURES_BYTES, Procedures};
