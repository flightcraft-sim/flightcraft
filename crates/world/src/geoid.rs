//! A shipped, bounded subset of the public-domain NGS GEOID18 model.
//! Simulation heights are orthometric MSL/NAVD88; georeferenced GLB positions are ellipsoidal.
use crate::TerrainError;
use serde::{Deserialize, Serialize};

pub const MAX_GEOID_SIDE: usize = 512;
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GeoidModel {
    NgsGeoid18,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeoidGrid {
    pub model: GeoidModel,
    pub source_sha256: String,
    pub south_deg: f64,
    pub west_deg: f64,
    /// South-to-north and west-to-east angular spacing, in degrees.
    pub step_deg: [f64; 2],
    pub rows: u32,
    pub cols: u32,
    /// South row first; west column first. Separation N, with h = H + N.
    pub undulation_m: Vec<f32>,
}
impl GeoidGrid {
    fn layout(&self) -> Result<(usize, usize), TerrainError> {
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        if !(2..=MAX_GEOID_SIDE).contains(&rows)
            || !(2..=MAX_GEOID_SIDE).contains(&cols)
            || rows.checked_mul(cols) != Some(self.undulation_m.len())
            || !self.south_deg.is_finite()
            || !self.west_deg.is_finite()
            || self
                .step_deg
                .iter()
                .any(|v| !v.is_finite() || (*v - 1. / 60.).abs() > 1e-10)
            || self.south_deg < -90.
            || self.west_deg < -180.
            || self.south_deg + (rows - 1) as f64 * self.step_deg[0] > 90.
            || self.west_deg + (cols - 1) as f64 * self.step_deg[1] > 180.
        {
            return Err(TerrainError("invalid GEOID18 subset dimensions or registration"));
        }
        Ok((rows, cols))
    }
    pub fn validate(&self) -> Result<(), TerrainError> {
        self.layout()?;
        if self.source_sha256.len() != 64
            || !self.source_sha256.bytes().all(|v| v.is_ascii_hexdigit())
            || self.undulation_m.iter().any(|v| !v.is_finite() || v.abs() > 120.)
        {
            return Err(TerrainError("invalid GEOID18 source hash or separation samples"));
        }
        Ok(())
    }
    pub fn separation_m(&self, lat: f64, lon: f64) -> Result<f64, TerrainError> {
        let (rows, cols) = self.layout()?;
        if !lat.is_finite() || !lon.is_finite() {
            return Err(TerrainError("invalid geoid coordinate"));
        }
        let y = (lat - self.south_deg) / self.step_deg[0];
        let x = (lon - self.west_deg) / self.step_deg[1];
        let (last_y, last_x) = ((rows - 1) as f64, (cols - 1) as f64);
        if x < -1e-9 || y < -1e-9 || x > last_x + 1e-9 || y > last_y + 1e-9 {
            return Err(TerrainError("GEOID18 subset does not cover coordinate"));
        }
        let x = x.clamp(0., last_x);
        let y = y.clamp(0., last_y);
        let col = (libm::floor(x) as usize).min(cols - 2);
        let row = (libm::floor(y) as usize).min(rows - 2);
        let at = |r: usize, c: usize| -> Result<f64, TerrainError> {
            let v = f64::from(
                *self
                    .undulation_m
                    .get(r * cols + c)
                    .ok_or(TerrainError("geoid sample outside grid"))?,
            );
            if !v.is_finite() || v.abs() > 120. {
                return Err(TerrainError("invalid geoid separation"));
            }
            Ok(v)
        };
        let (a, b, c, d) = (
            at(row, col)?,
            at(row, col + 1)?,
            at(row + 1, col)?,
            at(row + 1, col + 1)?,
        );
        let (u, v) = (x - col as f64, y - row as f64);
        Ok((a * (1. - u) + b * u) * (1. - v) + (c * (1. - u) + d * u) * v)
    }
    pub fn ellipsoidal_m(&self, lat: f64, lon: f64, orthometric_m: f64) -> Result<f64, TerrainError> {
        if !orthometric_m.is_finite() || !(-20_000.0..=100_000.).contains(&orthometric_m) {
            return Err(TerrainError("invalid orthometric height"));
        }
        Ok(orthometric_m + self.separation_m(lat, lon)?)
    }
    pub fn orthometric_m(&self, lat: f64, lon: f64, ellipsoidal_m: f64) -> Result<f64, TerrainError> {
        if !ellipsoidal_m.is_finite() || !(-20_000.0..=100_000.).contains(&ellipsoidal_m) {
            return Err(TerrainError("invalid ellipsoidal height"));
        }
        Ok(ellipsoidal_m - self.separation_m(lat, lon)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn grid() -> GeoidGrid {
        GeoidGrid {
            model: GeoidModel::NgsGeoid18,
            source_sha256: "a".repeat(64),
            south_deg: 40.,
            west_deg: -75.,
            step_deg: [1. / 60.; 2],
            rows: 2,
            cols: 2,
            undulation_m: vec![-34., -32., -30., -28.],
        }
    }
    #[test]
    fn bilinear_separation_and_height_sign_have_independent_expected_values() {
        let g = grid();
        g.validate().unwrap();
        let (lat, lon) = (40. + 1. / 120., -75. + 1. / 120.);
        assert!((g.separation_m(lat, lon).unwrap() + 31.).abs() < 1e-9);
        assert!((g.ellipsoidal_m(lat, lon, 113.0808).unwrap() - 82.0808).abs() < 1e-9);
        assert!((g.orthometric_m(lat, lon, 82.0808).unwrap() - 113.0808).abs() < 1e-9);
        assert_eq!(g.separation_m(40., -75.).unwrap(), -34.);
        assert!((g.separation_m(40. + 1. / 60., -75. + 1. / 60.).unwrap() + 28.).abs() < 1e-9);
    }
    #[test]
    fn hostile_grids_and_missing_coverage_fail_closed() {
        let g = grid();
        for (lat, lon) in [
            (f64::NAN, -75.),
            (40., f64::INFINITY),
            (39.99, -75.),
            (40., -74.99 + 1.),
        ] {
            assert!(g.separation_m(lat, lon).is_err());
        }
        for n in [0, 1, u32::MAX] {
            let mut v = g.clone();
            v.rows = n;
            assert!(v.validate().is_err());
            assert!(v.separation_m(40., -75.).is_err());
        }
        let mut v = g.clone();
        v.undulation_m[1] = f32::NAN;
        assert!(v.validate().is_err());
        assert!(v.separation_m(40., -75.).is_err());
        let mut v = g.clone();
        v.undulation_m.pop();
        assert!(v.validate().is_err());
        let mut v = g.clone();
        v.step_deg[0] = 0.;
        assert!(v.validate().is_err());
        assert!(g.ellipsoidal_m(40., -75., f64::NAN).is_err());
    }
}
