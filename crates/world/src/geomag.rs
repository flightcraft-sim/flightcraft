//! WMM2025 degree-12 internal field, independently evaluated from NOAA's harmonic equations.
//! Coefficients use Schmidt quasi-normalisation without the Condon–Shortley phase.
use crate::TerrainError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SimulationDate {
    #[schemars(range(min = 2025, max = 2029))]
    pub year: u16,
    #[schemars(range(min = 1, max = 12))]
    pub month: u8,
    #[schemars(range(min = 1, max = 31))]
    pub day: u8,
}
impl Default for SimulationDate {
    fn default() -> Self {
        Self {
            year: 2026,
            month: 1,
            day: 1,
        }
    }
}
impl SimulationDate {
    pub fn decimal_year(self) -> Result<f64, TerrainError> {
        let leap =
            self.year.is_multiple_of(4) && (!self.year.is_multiple_of(100) || self.year.is_multiple_of(400));
        let lengths = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        let days = lengths
            .get(usize::from(self.month.saturating_sub(1)))
            .copied()
            .unwrap_or(0);
        if !(2025..=2029).contains(&self.year) || self.month == 0 || self.day == 0 || self.day > days {
            return Err(TerrainError(
                "date must be valid and within WMM2025 lifetime (2025–2029)",
            ));
        }
        let elapsed: u16 = lengths
            .iter()
            .take(usize::from(self.month - 1))
            .map(|v| u16::from(*v))
            .sum();
        Ok(f64::from(self.year)
            + f64::from(elapsed + u16::from(self.day) - 1) / if leap { 366.0 } else { 365.0 })
    }
}

#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
pub struct MagneticField {
    pub north_nt: f64,
    pub east_nt: f64,
    pub down_nt: f64,
    pub declination_deg: f64,
    pub inclination_deg: f64,
    pub horizontal_nt: f64,
    pub total_nt: f64,
}
#[derive(Clone, Debug)]
pub struct MagneticModel {
    coefficients: Vec<Coefficient>,
}
#[derive(Clone, Debug)]
struct Coefficient {
    n: usize,
    m: usize,
    g: f64,
    h: f64,
    dg: f64,
    dh: f64,
}
// Checked access keeps the recurrence total even if its implementation changes.
#[derive(Default)]
struct Triangle([[f64; 13]; 13]);
impl Triangle {
    fn get(&self, n: usize, m: usize) -> f64 {
        self.0.get(n).and_then(|r| r.get(m)).copied().unwrap_or(0.0)
    }
    fn set(&mut self, n: usize, m: usize, v: f64) {
        if let Some(x) = self.0.get_mut(n).and_then(|r| r.get_mut(m)) {
            *x = v;
        }
    }
}
impl MagneticModel {
    pub fn wmm2025() -> Result<Self, TerrainError> {
        let mut coefficients = Vec::new();
        for line in include_str!("../../../assets/geomag/WMM2025.COF").lines().skip(1) {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.first().is_some_and(|s| s.starts_with("9999")) {
                break;
            }
            let [n, m, g, h, dg, dh] = fields.as_slice() else {
                return Err(TerrainError("invalid bundled WMM row"));
            };
            let n = n
                .parse::<usize>()
                .map_err(|_| TerrainError("invalid WMM degree"))?;
            let m = m
                .parse::<usize>()
                .map_err(|_| TerrainError("invalid WMM order"))?;
            if !(1..=12).contains(&n) || m > n {
                return Err(TerrainError("invalid WMM degree/order"));
            }
            let number = |s: &str| {
                s.parse::<f64>()
                    .map_err(|_| TerrainError("invalid WMM coefficient"))
            };
            coefficients.push(Coefficient {
                n,
                m,
                g: number(g)?,
                h: number(h)?,
                dg: number(dg)?,
                dh: number(dh)?,
            });
        }
        if coefficients.len() != 90 {
            return Err(TerrainError("incomplete WMM coefficients"));
        }
        Ok(Self { coefficients })
    }
    /// WGS84 ellipsoidal altitude in metres, decimal year, east-positive declination.
    pub fn field(
        &self,
        latitude_deg: f64,
        longitude_deg: f64,
        altitude_m: f64,
        year: f64,
    ) -> Result<MagneticField, TerrainError> {
        if !latitude_deg.is_finite()
            || latitude_deg.abs() > 90.0
            || !longitude_deg.is_finite()
            || longitude_deg.abs() > 180.0
            || !altitude_m.is_finite()
            || !(-1000.0..=850000.0).contains(&altitude_m)
            || !year.is_finite()
            || !(2025.0..2030.0).contains(&year)
        {
            return Err(TerrainError("WMM position or date outside model bounds"));
        }
        // Evaluate the limit at poles; the meridian is given by the supplied longitude.
        let lat = latitude_deg.clamp(-89.9999999, 89.9999999).to_radians();
        let lon = longitude_deg.to_radians();
        let (sl, cl) = libm::sincos(lat);
        let e2 = 1.0 - (6356.7523142_f64 / 6378.137) * (6356.7523142 / 6378.137);
        let nu = 6378.137 / libm::sqrt(1.0 - e2 * sl * sl);
        let rho = (nu + altitude_m / 1000.0) * cl;
        let z = (nu * (1.0 - e2) + altitude_m / 1000.0) * sl;
        let r = libm::hypot(rho, z);
        let geocentric = libm::atan2(z, rho);
        let theta = core::f64::consts::FRAC_PI_2 - geocentric;
        let (st, ct) = libm::sincos(theta);
        let mut p = Triangle::default();
        let mut dp = Triangle::default();
        p.set(0, 0, 1.0);
        for n in 1..=12 {
            for m in 0..=n {
                let (v, d) = if n == m {
                    let k = (2 * n - 1) as f64;
                    (
                        k * st * p.get(n - 1, m - 1),
                        k * (ct * p.get(n - 1, m - 1) + st * dp.get(n - 1, m - 1)),
                    )
                } else {
                    let k = (2 * n - 1) as f64;
                    let j = (n + m - 1) as f64;
                    let den = (n - m) as f64;
                    let prev2 = if n >= 2 { p.get(n - 2, m) } else { 0.0 };
                    let dprev2 = if n >= 2 { dp.get(n - 2, m) } else { 0.0 };
                    (
                        (k * ct * p.get(n - 1, m) - j * prev2) / den,
                        (k * (-st * p.get(n - 1, m) + ct * dp.get(n - 1, m)) - j * dprev2) / den,
                    )
                };
                p.set(n, m, v);
                dp.set(n, m, d);
            }
        }
        let (mut br, mut bt, mut bp) = (0.0, 0.0, 0.0);
        for c in &self.coefficients {
            let mut norm = if c.m == 0 { 1.0 } else { 2.0 };
            for k in (c.n - c.m + 1)..=(c.n + c.m) {
                norm /= k as f64;
            }
            norm = libm::sqrt(norm);
            let g = c.g + (year - 2025.0) * c.dg;
            let h = c.h + (year - 2025.0) * c.dh;
            let (s, co) = libm::sincos(c.m as f64 * lon);
            let scale = libm::pow(6371.2 / r, (c.n + 2) as f64) * norm;
            br += (c.n + 1) as f64 * scale * (g * co + h * s) * p.get(c.n, c.m);
            bt -= scale * (g * co + h * s) * dp.get(c.n, c.m);
            bp += scale * c.m as f64 * (g * s - h * co) * p.get(c.n, c.m) / st;
        }
        let (sd, cd) = libm::sincos(lat - geocentric);
        let north_nt = -bt * cd - br * sd;
        let down_nt = bt * sd - br * cd;
        let horizontal_nt = libm::hypot(north_nt, bp);
        Ok(MagneticField {
            north_nt,
            east_nt: bp,
            down_nt,
            horizontal_nt,
            total_nt: libm::hypot(horizontal_nt, down_nt),
            declination_deg: libm::atan2(bp, north_nt).to_degrees(),
            inclination_deg: libm::atan2(down_nt, horizontal_nt).to_degrees(),
        })
    }
    pub fn declination(&self, lat: f64, lon: f64, date: SimulationDate) -> Result<f64, TerrainError> {
        Ok(self.field(lat, lon, 0.0, date.decimal_year()?)?.declination_deg)
    }
}
