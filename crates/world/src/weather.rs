//! Steady wind, a surface-layer power law, and seeded linear Dryden gusts.
//!
//! Transfer functions independently implemented from MIL-F-8785C (5 November 1980),
//! §3.7.2, and MIL-HDBK-1797 (19 December 1997), atmospheric turbulence model:
//! Hu(s) ∝ 1/(1+τs), Hv,w(s) ∝ (1+√3 τs)/(1+τs)², τ=L/V.
//! We use the isotropic 1,750 ft (533.4 m) scale-length approximation throughout
//! the flight envelope; intensity is an explicit per-axis RMS, not an exceedance
//! probability or a low-altitude wind-derived intensity. Angular gusts are omitted.
//! For unit two-sided white-noise intensity and a=V/L, the normalised functions
//! are Hu(s)=σ√(2a)/(s+a) and Hv,w(s)=σ√a(√3 s+a)/(s+a)².

use serde::{Deserialize, Serialize};

/// Original scenario presets: RMS gust velocity on each NED axis, m/s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurbulenceLevel {
    #[default]
    Off,
    Light,
    Moderate,
    Severe,
}

impl TurbulenceLevel {
    #[must_use]
    pub fn intensity_mps(self) -> f64 {
        match self {
            Self::Off => 0.0,
            Self::Light => 1.0,
            Self::Moderate => 2.0,
            Self::Severe => 4.0,
        }
    }
}

/// Wind speed is specified at 10 m AGL. Direction is meteorological (blows FROM).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Weather {
    #[schemars(range(min = 0, max = 360))]
    pub wind_from_deg: f64,
    #[schemars(range(min = 0, max = 75))]
    pub wind_speed_mps: f64,
    pub shear: bool,
    pub turbulence: TurbulenceLevel,
    pub seed: u64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WeatherError {
    #[error(
        "weather requires at most three layers with finite base below top (-1500..60000 ft MSL) and visibility 0.125..100 SM"
    )]
    Conditions,
    #[error("weather requires finite wind_from_deg in 0..=360 and wind_speed_mps in 0..=75")]
    Settings,
    #[error("gust step requires finite dt in 0..=1 s and airspeed in 0..=1000 m/s")]
    Step,
}

impl Weather {
    pub fn validate(self) -> Result<(), WeatherError> {
        if !self.wind_from_deg.is_finite()
            || !(0.0..=360.0).contains(&self.wind_from_deg)
            || !self.wind_speed_mps.is_finite()
            || !(0.0..=75.0).contains(&self.wind_speed_mps)
        {
            return Err(WeatherError::Settings);
        }
        Ok(())
    }

    /// Wind in NED, with a 1/7 power law from 0 to 1,000 m AGL, constant above.
    /// Time enters the total wind through the fixed-step Dryden process.
    #[must_use]
    pub fn mean_wind_mps(self, position_ned_m: [f64; 3]) -> [f64; 3] {
        let [n, e, d] = position_ned_m;
        let height = (-d - crate::ground(n, e).height_msl_m).clamp(0.0, 1000.0);
        let factor = if self.shear {
            libm::pow(height / 10.0, 1.0 / 7.0)
        } else {
            1.0
        };
        let direction = self.wind_from_deg * (core::f64::consts::PI / 180.0);
        let speed = self.wind_speed_mps * factor;
        [-speed * libm::cos(direction), -speed * libm::sin(direction), 0.0]
    }
}

/// Exact discretisation of the unit-variance Dryden shaping filters. The two-pole
/// state has stationary covariance [[1, 1/2], [1/2, 1/2]]. Keeping its correlated
/// innovations preserves the spectrum and RMS independently of the fixed step.
#[derive(Clone, Debug)]
pub struct Wind {
    weather: Weather,
    rng: u64,
    heading_rad: f64,
    longitudinal: f64,
    lateral: [f64; 2],
    vertical: [f64; 2],
}

impl Wind {
    pub fn new(weather: Weather) -> Result<Self, WeatherError> {
        weather.validate()?;
        Ok(Self {
            weather,
            rng: weather.seed,
            heading_rad: 0.0,
            longitudinal: 0.0,
            lateral: [0.0; 2],
            vertical: [0.0; 2],
        })
    }

    #[must_use]
    pub fn weather(&self) -> Weather {
        self.weather
    }

    /// Current wind without advancing any memory. Safe for RK4 substages and readouts.
    #[must_use]
    pub fn velocity_mps(&self, position_ned_m: [f64; 3]) -> [f64; 3] {
        let [n, e, d] = self.weather.mean_wind_mps(position_ned_m);
        let [u, v, w] = self.gust_mps();
        [n + u, e + v, d + w]
    }

    /// Gust in NED; the longitudinal/lateral filters follow the horizontal air-relative
    /// flight path sampled at the start of each step. Vertical remains down.
    #[must_use]
    pub fn gust_mps(&self) -> [f64; 3] {
        let transverse =
            |[x, y]: [f64; 2]| libm::sqrt(1.5) * x + (1.0 - libm::sqrt(3.0)) / libm::sqrt(2.0) * y;
        let sigma = self.weather.turbulence.intensity_mps();
        let u = sigma * self.longitudinal;
        let v = sigma * transverse(self.lateral);
        let (sin, cos) = (libm::sin(self.heading_rad), libm::cos(self.heading_rad));
        [
            u * cos - v * sin,
            u * sin + v * cos,
            sigma * transverse(self.vertical),
        ]
    }

    /// Advance once per fixed simulation step, never per force evaluation. Invalid
    /// step/speed inputs are errors and leave the process untouched. At rest a 1 m/s
    /// convection floor prevents a frozen gust. Off consumes no random numbers.
    pub fn advance(&mut self, dt_s: f64, air_velocity_ned_mps: [f64; 3]) -> Result<(), WeatherError> {
        let [n, e, d] = air_velocity_ned_mps;
        let airspeed_mps = libm::sqrt(n * n + e * e + d * d);
        if !dt_s.is_finite()
            || !(0.0..=1.0).contains(&dt_s)
            || !airspeed_mps.is_finite()
            || !(0.0..=1000.0).contains(&airspeed_mps)
        {
            return Err(WeatherError::Step);
        }
        if self.weather.turbulence == TurbulenceLevel::Off || dt_s == 0.0 {
            return Ok(());
        }
        if n * n + e * e > 0.01 {
            self.heading_rad = libm::atan2(e, n);
        }
        let d = airspeed_mps.max(1.0) * dt_s / 533.4;
        if d == 0.0 {
            return Ok(());
        }
        let decay = libm::exp(-d);
        let q11 = -libm::expm1(-2.0 * d);
        // Series avoid cancellation of the O(d²) and O(d³) covariance entries.
        let (q12, q22) = if d < 0.01 {
            (
                d * d * (1.0 + d * (-4.0 / 3.0 + d * (1.0 + d * (-8.0 / 15.0 + d * 2.0 / 9.0)))),
                d * d * d * (2.0 / 3.0 + d * (-1.0 + d * (4.0 / 5.0 + d * (-4.0 / 9.0 + d * 4.0 / 21.0)))),
            )
        } else {
            let e2 = decay * decay;
            ((q11 - 2.0 * d * e2) * 0.5, (q11 - 2.0 * d * (1.0 + d) * e2) * 0.5)
        };
        let root = libm::sqrt(q11);
        let shared = q12 / root;
        let independent = libm::sqrt((q22 - shared * shared).max(0.0));
        let [a, b] = self.gaussian_pair();
        let [c, e] = self.gaussian_pair();
        let [f, _] = self.gaussian_pair();
        self.longitudinal = decay * self.longitudinal + root * a;
        let advance = |[x, y]: [f64; 2], n1: f64, n2: f64| {
            [
                decay * x + root * n1,
                decay * (y + d * x) + shared * n1 + independent * n2,
            ]
        };
        self.lateral = advance(self.lateral, b, c);
        self.vertical = advance(self.vertical, e, f);
        Ok(())
    }

    // SplitMix64, written here from its integer mixing definition. Six open-interval
    // uniforms per step feed Box–Muller. No platform RNG, clock, or cached Gaussian.
    fn uniform(&mut self) -> f64 {
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        ((z >> 12) as f64 + 0.5) / 4_503_599_627_370_496.0
    }

    fn gaussian_pair(&mut self) -> [f64; 2] {
        let radius = libm::sqrt(-2.0 * libm::log(self.uniform()));
        let angle = core::f64::consts::TAU * self.uniform();
        [radius * libm::cos(angle), radius * libm::sin(angle)]
    }

    /// Process memory for deterministic trajectory hashing, in stable field order.
    pub fn hash_words(&self) -> impl Iterator<Item = u64> + '_ {
        core::iter::once(self.rng).chain(
            core::iter::once(self.heading_rad)
                .chain(core::iter::once(self.longitudinal))
                .chain(self.lateral)
                .chain(self.vertical)
                .map(f64::to_bits),
        )
    }
}
