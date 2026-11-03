//! Independent EPT Z-unit/height check against receipt-backed 3DEP terrain.
//! EPT's horizontal EPSG:3857 declaration alone does not specify a vertical datum.
use super::{dem::Dems, lidar::Point};
use flightcraft_world::{geoid::GeoidGrid, scenery};
use serde::Serialize;
use std::collections::BTreeSet;
#[derive(Default)]
pub(super) struct Check {
    cells: BTreeSet<(i32, i32)>,
    residuals: Vec<[f64; 3]>,
    pub clipped_points: u64,
}
#[derive(Serialize)]
pub(super) struct Audit {
    pub ept_z_agrees_with_navd88_meters: bool,
    pub samples: usize,
    pub orthometric_median_absolute_error_m: Option<f64>,
    pub orthometric_p95_absolute_error_m: Option<f64>,
    pub ellipsoidal_median_absolute_error_m: Option<f64>,
    pub us_survey_feet_median_absolute_error_m: Option<f64>,
    pub control: &'static str,
}
impl Check {
    pub fn sample(
        &mut self,
        p: Point,
        bounds: [f64; 4],
        dem: &mut Dems,
        geoid: Option<&GeoidGrid>,
    ) -> Result<(), String> {
        let [x, y, z] = p.xyz;
        let [w, s, e, n] = bounds;
        if x < w || x > e || y < s || y > n {
            return Ok(());
        }
        self.clipped_points += 1;
        if p.withheld || p.classification != 2 || self.cells.len() >= 32768 {
            return Ok(());
        }
        let [lat, lon] = scenery::inverse_mercator(x, y).map_err(|e| e.to_string())?;
        // Deterministic spatial sampling prevents dense ground returns in a small patch from
        // overwhelming the independent control. Mercator units are corrected for latitude.
        let step = 16. / libm::cos(lat.to_radians());
        let cell = (
            libm::floor((x - w) / step) as i32,
            libm::floor((y - s) / step) as i32,
        );
        if !self.cells.insert(cell) {
            return Ok(());
        }
        let terrain = dem.height(lat, lon)?;
        let ellipsoidal = if let Some(grid) = geoid {
            grid.orthometric_m(lat, lon, z).map_err(|e| e.to_string())? - terrain
        } else {
            f64::NAN
        };
        self.residuals.push([
            (z - terrain).abs(),
            ellipsoidal.abs(),
            (z * 1200. / 3937. - terrain).abs(),
        ]);
        Ok(())
    }
    pub fn finish(&self) -> Audit {
        let quantile = |axis: usize, q: f64| -> Option<f64> {
            let mut values: Vec<f64> = self
                .residuals
                .iter()
                .filter_map(|r| r.get(axis).copied())
                .filter(|v| v.is_finite())
                .collect();
            values.sort_by(f64::total_cmp);
            values
                .get((q * (values.len().saturating_sub(1)) as f64) as usize)
                .copied()
        };
        let median = quantile(0, 0.5);
        let p95 = quantile(0, 0.95);
        Audit {
            ept_z_agrees_with_navd88_meters: self.residuals.len() >= 128
                && median.is_some_and(|e| e <= 0.5)
                && p95.is_some_and(|e| e <= 1.),
            samples: self.residuals.len(),
            orthometric_median_absolute_error_m: median,
            orthometric_p95_absolute_error_m: p95,
            ellipsoidal_median_absolute_error_m: quantile(1, 0.5),
            us_survey_feet_median_absolute_error_m: quantile(2, 0.5),
            control: "Spatially sampled classified ground versus independent receipt-backed USGS NAVD88 meter DEM; agreement is empirical, not a replacement for source CRS metadata",
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_distributed_samples_and_rejects_bad_vertical_hypotheses() {
        let mut check = Check::default();
        assert!(!check.finish().ept_z_agrees_with_navd88_meters);
        check.residuals = vec![[0.1, 31., 258.]; 128];
        let audit = check.finish();
        assert!(audit.ept_z_agrees_with_navd88_meters);
        assert_eq!(audit.ellipsoidal_median_absolute_error_m, Some(31.));
        check.residuals = vec![[31., 0.1, 290.]; 128];
        assert!(!check.finish().ept_z_agrees_with_navd88_meters);
        check.residuals = vec![[0.1, 31., 258.]; 127];
        assert!(!check.finish().ept_z_agrees_with_navd88_meters);
        check.residuals = vec![[0.1, 31., 258.]; 128];
        check.residuals.extend(vec![[3., 34., 261.]; 16]);
        assert!(!check.finish().ept_z_agrees_with_navd88_meters);
    }
}
