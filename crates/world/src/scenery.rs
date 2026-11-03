//! Portable imagery quadtree. Coordinates are EPSG:3857 metres; image rows run southward.
//! Presentation metadata does not participate in simulation identity or ground queries.
use crate::TerrainError;
use serde::{Deserialize, Serialize};

pub const MERCATOR_RADIUS_M: f64 = 6_378_137.0;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct TileKey {
    pub level: u8,
    pub x: u32,
    /// Zero is the northernmost row.
    pub y: u32,
}
impl TileKey {
    pub fn valid(self) -> bool {
        self.level <= 12 && self.x < (1_u32 << self.level) && self.y < (1_u32 << self.level)
    }
    pub fn parent(self) -> Option<Self> {
        (self.level > 0).then(|| Self {
            level: self.level.saturating_sub(1),
            x: self.x / 2,
            y: self.y / 2,
        })
    }
    pub fn name(self) -> String {
        format!("{}/{}/{}", self.level, self.x, self.y)
    }
}

pub fn mercator(latitude: f64, longitude: f64) -> Result<[f64; 2], TerrainError> {
    if !latitude.is_finite()
        || latitude.abs() > 85.
        || !longitude.is_finite()
        || !(-180.0..=180.).contains(&longitude)
    {
        return Err(TerrainError("invalid imagery coordinate"));
    }
    Ok([
        MERCATOR_RADIUS_M * longitude.to_radians(),
        MERCATOR_RADIUS_M * libm::asinh(libm::tan(latitude.to_radians())),
    ])
}
pub fn inverse_mercator(x: f64, y: f64) -> Result<[f64; 2], TerrainError> {
    if !x.is_finite() || !y.is_finite() || x.abs() > 20_037_509. || y.abs() > 20_037_509. {
        return Err(TerrainError("invalid Mercator coordinate"));
    }
    Ok([
        libm::atan(libm::sinh(y / MERCATOR_RADIUS_M)).to_degrees(),
        (x / MERCATOR_RADIUS_M).to_degrees(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_coordinates_and_keys() {
        assert!(
            !TileKey {
                level: 255,
                x: 0,
                y: 0
            }
            .valid()
        );
        for (lat, lon) in [(f64::NAN, 0.), (90., 0.), (0., f64::INFINITY)] {
            assert!(mercator(lat, lon).is_err());
        }
        for (lat, lon) in [
            (40.96905613156273, -75.00242278301268),
            (40.973226688232224, -74.99258941913907),
        ] {
            let [x, y] = mercator(lat, lon).unwrap();
            let [a, b] = inverse_mercator(x, y).unwrap();
            assert!((a - lat).abs() < 1e-10 && (b - lon).abs() < 1e-10);
        }
    }
}
