//! Deterministic visual weather. Heights are MSL feet; visibility is statute miles.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Coverage {
    Few,
    Sct,
    Bkn,
    Ovc,
}
impl Coverage {
    pub fn fraction(self) -> f64 {
        match self {
            Self::Few => 0.2,
            Self::Sct => 0.45,
            Self::Bkn => 0.8,
            Self::Ovc => 1.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloudLayer {
    #[schemars(range(min = -1500, max = 60000))]
    pub base_ft_msl: f64,
    #[schemars(range(min = -1500, max = 60000))]
    pub top_ft_msl: f64,
    pub coverage: Coverage,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Obscuration {
    #[default]
    None,
    Mist,
    Fog,
    Haze,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Precipitation {
    #[default]
    None,
    Rain,
    Snow,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimeOfDay {
    #[default]
    Day,
    Dusk,
    Night,
    Dawn,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeatherPreset {
    Vfr,
    Mvfr,
    Ifr,
    Lifr,
    Night,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    /// Sea-level reference observations; station temperature uses a standard lapse rate.
    #[serde(default = "default_temperature")]
    pub temperature_c: f64,
    #[serde(default = "default_dewpoint")]
    pub dewpoint_c: f64,
    #[serde(default = "default_pressure")]
    pub sea_level_pressure_pa: f64,
    #[schemars(length(max = 3))]
    pub layers: Vec<CloudLayer>,
    #[schemars(range(min = 0.125, max = 100))]
    pub visibility_sm: f64,
    pub obscuration: Obscuration,
    pub precipitation: Precipitation,
    pub time_of_day: TimeOfDay,
}
fn default_temperature() -> f64 {
    15.0
}
fn default_dewpoint() -> f64 {
    5.0
}
fn default_pressure() -> f64 {
    101325.0
}
impl Default for Conditions {
    fn default() -> Self {
        Self {
            temperature_c: default_temperature(),
            dewpoint_c: default_dewpoint(),
            sea_level_pressure_pa: default_pressure(),
            layers: Vec::new(),
            visibility_sm: 10.,
            obscuration: Obscuration::None,
            precipitation: Precipitation::None,
            time_of_day: TimeOfDay::Day,
        }
    }
}
impl Conditions {
    pub fn validate(&self) -> Result<(), crate::WeatherError> {
        if !self.temperature_c.is_finite()
            || !(-60.0..=60.0).contains(&self.temperature_c)
            || !self.dewpoint_c.is_finite()
            || !(-100.0..=self.temperature_c).contains(&self.dewpoint_c)
            || !self.sea_level_pressure_pa.is_finite()
            || !(80000.0..=110000.0).contains(&self.sea_level_pressure_pa)
            || !self.visibility_sm.is_finite()
            || !(0.125..=100.).contains(&self.visibility_sm)
            || self.layers.len() > 3
            || self.layers.iter().any(|l| {
                !l.base_ft_msl.is_finite()
                    || !l.top_ft_msl.is_finite()
                    || !(-1500. ..=60000.).contains(&l.base_ft_msl)
                    || !(-1500. ..=60000.).contains(&l.top_ft_msl)
                    || l.base_ft_msl >= l.top_ft_msl
            })
        {
            return Err(crate::WeatherError::Conditions);
        }
        Ok(())
    }
    pub fn preset(preset: WeatherPreset) -> Self {
        let mut result = Self::default();
        let (base, visibility, obscuration) = match preset {
            WeatherPreset::Vfr => return result,
            WeatherPreset::Night => {
                result.time_of_day = TimeOfDay::Night;
                return result;
            }
            WeatherPreset::Mvfr => (1800., 4., Obscuration::Haze),
            WeatherPreset::Ifr => (600., 2., Obscuration::Mist),
            WeatherPreset::Lifr => (200., 0.5, Obscuration::Fog),
        };
        result.layers.push(CloudLayer {
            base_ft_msl: base,
            top_ft_msl: base + 3000.,
            coverage: Coverage::Ovc,
        });
        result.visibility_sm = visibility;
        result.obscuration = obscuration;
        result
    }
    pub fn effective_visibility_m(&self) -> f64 {
        self.visibility_sm
            * 1609.344
            * match self.precipitation {
                Precipitation::None => 1.,
                Precipitation::Rain => 0.75,
                Precipitation::Snow => 0.5,
            }
    }
    /// Stable spatial cloud holes shared with the shader; no random generator or clock.
    pub fn cloud_density(north_m: f64, east_m: f64) -> f64 {
        0.5 + 0.25 * libm::sin(north_m * 0.0017) + 0.25 * libm::sin(east_m * 0.0023)
    }
    pub fn in_cloud(&self, [n, e, d]: [f64; 3]) -> bool {
        let altitude = -d;
        self.layers.iter().any(|l| {
            altitude >= l.base_ft_msl * 0.3048
                && altitude <= l.top_ft_msl * 0.3048
                && Self::cloud_density(n, e) <= l.coverage.fraction()
        })
    }
    pub fn imc(&self, position: [f64; 3]) -> bool {
        self.in_cloud(position) || self.effective_visibility_m() < 3. * 1609.344
    }
    /// Stable words for session hashing, emitted only for nondefault visual weather.
    pub fn hash_words(&self) -> Vec<u64> {
        let mut words = vec![
            self.temperature_c.to_bits(),
            self.dewpoint_c.to_bits(),
            self.sea_level_pressure_pa.to_bits(),
            self.visibility_sm.to_bits(),
            self.obscuration as u64,
            self.precipitation as u64,
            self.time_of_day as u64,
            self.layers.len() as u64,
        ];
        for l in &self.layers {
            words.extend([l.base_ft_msl.to_bits(), l.top_ft_msl.to_bits(), l.coverage as u64]);
        }
        words
    }
}
