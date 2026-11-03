//! Recorded weather targets and fixed-tick transitions. No network reads during replay.
use super::{Session, runtime::LiveObservation};
use crate::CommandError;
use flightcraft_sim::Weather;
use flightcraft_world::{CloudLayer, Conditions, Coverage, Obscuration, Precipitation};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordedLiveWeather {
    pub time_s: f64,
    pub duration_s: f64,
    pub observation: LiveObservation,
    /// Grid-referenced wind and MSL conditions, resolved when accepted, for exact replay.
    pub initial_wind: Weather,
    pub target_wind: Weather,
    pub initial_conditions: Conditions,
    pub target_conditions: Conditions,
}
pub type Transition = RecordedLiveWeather;
impl RecordedLiveWeather {
    pub fn validate(&self) -> Result<(), String> {
        if !self.time_s.is_finite() || self.time_s < 0. || !matches!(self.duration_s, 0. | 60.) {
            return Err("Invalid recorded weather timing".into());
        }
        self.observation.validate()?;
        self.initial_wind.validate().map_err(|e| e.to_string())?;
        self.target_wind.validate().map_err(|e| e.to_string())?;
        self.initial_conditions.validate().map_err(|e| e.to_string())?;
        self.target_conditions.validate().map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Apply a recorded target at a simulation time; contains no clock, endpoint lookup, or GET.
    pub fn apply_at(&self, session: &mut Session, time_s: f64) -> Result<(), CommandError> {
        self.validate().map_err(CommandError::Failed)?;
        self.apply_tick(session, time_s)
    }
    fn apply_tick(&self, session: &mut Session, time_s: f64) -> Result<(), CommandError> {
        if !time_s.is_finite() {
            return Err(CommandError::Failed("Invalid replay time".into()));
        }
        if time_s < self.time_s {
            return Ok(());
        }
        let fraction = if self.duration_s == 0. {
            1.
        } else {
            ((time_s - self.time_s) / self.duration_s).clamp(0., 1.)
        };
        let mix = |a: f64, b: f64| a + (b - a) * fraction;
        let mut wind = self.target_wind;
        let angle =
            (self.target_wind.wind_from_deg - self.initial_wind.wind_from_deg + 180.).rem_euclid(360.) - 180.;
        wind.wind_from_deg = (self.initial_wind.wind_from_deg + angle * fraction).rem_euclid(360.);
        wind.wind_speed_mps = mix(self.initial_wind.wind_speed_mps, wind.wind_speed_mps);
        let mut c = self.target_conditions.clone();
        c.sea_level_pressure_pa = mix(
            self.initial_conditions.sea_level_pressure_pa,
            c.sea_level_pressure_pa,
        );
        c.temperature_c = mix(self.initial_conditions.temperature_c, c.temperature_c);
        c.dewpoint_c = mix(self.initial_conditions.dewpoint_c, c.dewpoint_c).min(c.temperature_c);
        c.visibility_sm = mix(self.initial_conditions.visibility_sm, c.visibility_sm);
        // Grow incoming cloud slabs from zero thickness, and shrink outgoing slabs to zero.
        let count = self.initial_conditions.layers.len().max(c.layers.len());
        let mut layers = Vec::new();
        for index in 0..count {
            let before = self.initial_conditions.layers.get(index);
            let after = c.layers.get(index);
            let Some(reference) = after.or(before) else {
                continue;
            };
            let a = before.copied().unwrap_or(CloudLayer {
                top_ft_msl: reference.base_ft_msl,
                ..*reference
            });
            let b = after.copied().unwrap_or(CloudLayer {
                top_ft_msl: reference.base_ft_msl,
                ..*reference
            });
            let layer = CloudLayer {
                base_ft_msl: mix(a.base_ft_msl, b.base_ft_msl),
                top_ft_msl: mix(a.top_ft_msl, b.top_ft_msl),
                coverage: reference.coverage,
            };
            if layer.top_ft_msl > layer.base_ft_msl {
                layers.push(layer);
            }
        }
        c.layers = layers;
        session.sim_mut().set_conditions(c)?;
        // Setting identical wind repeatedly restarts the gust process; only update when changed.
        if session.sim().weather() != wind {
            session.sim_mut().set_weather(wind)?;
        }
        Ok(())
    }
}
impl Session {
    pub(crate) fn accept_live(
        &mut self,
        observation: LiveObservation,
        follow: bool,
    ) -> Result<(), CommandError> {
        if self.comms.record.live_weather.len() >= 2000 {
            return Err(CommandError::Failed(
                "Flight record weather limit reached; start a new flight".into(),
            ));
        }
        let o = &observation.extracted.observation;
        let mut conditions = self.sim.conditions().clone();
        let elevation = self
            .sim
            .world()
            .airports()
            .and_then(|f| {
                f.airports.iter().find(|a| {
                    a.ident == observation.source.airport
                        || a.icao.as_deref() == Some(&observation.source.airport)
                })
            })
            .map(|a| a.elevation_m)
            .or_else(|| {
                self.stations()
                    .iter()
                    .find(|s| s.airport == observation.source.airport)
                    .map(|s| s.elevation_m)
            });
        // Converting reported AGL heights or station temperatures requires a known field elevation.
        if elevation.is_none()
            && (o.layers.as_ref().is_some_and(|v| !v.is_empty())
                || o.temperature_c.is_some()
                || o.dewpoint_c.is_some())
        {
            return Err(CommandError::Failed(
                "Load the airport's region before applying station temperatures or cloud heights".into(),
            ));
        }
        let elevation = elevation.unwrap_or(0.);
        if let Some(v) = o.temperature_c {
            conditions.temperature_c = (v + 0.0065 * elevation).clamp(-60., 60.);
        }
        if let Some(v) = o.dewpoint_c {
            conditions.dewpoint_c = (v + 0.002 * elevation).clamp(-100., conditions.temperature_c);
        }
        conditions.dewpoint_c = conditions.dewpoint_c.min(conditions.temperature_c);
        if let Some(v) = o.altimeter_inhg {
            conditions.sea_level_pressure_pa = v * 3386.389;
        }
        if let Some(v) = o.visibility_sm {
            conditions.visibility_sm = v.clamp(0.125, 100.);
        }
        if let Some(layers) = &o.layers {
            let mut result = Vec::new();
            for layer in layers.iter().take(3) {
                let Some(base) = layer.base_agl_ft else {
                    return Err(CommandError::Failed(
                        "Cloud height is unknown; cannot apply this observation automatically".into(),
                    ));
                };
                let base = (base + elevation / 0.3048).clamp(-1500., 59000.);
                result.push(CloudLayer {
                    base_ft_msl: base,
                    top_ft_msl: (base + 2000.).min(60000.),
                    coverage: match layer.coverage.as_str() {
                        "FEW" => Coverage::Few,
                        "SCT" => Coverage::Sct,
                        "BKN" => Coverage::Bkn,
                        _ => Coverage::Ovc,
                    },
                });
            }
            conditions.layers = result;
        }
        if let Some(weather) = &o.present_weather {
            conditions.obscuration = if weather.iter().any(|v| v.contains("FG")) {
                Obscuration::Fog
            } else if weather.iter().any(|v| v.contains("BR")) {
                Obscuration::Mist
            } else if weather.iter().any(|v| v.contains("HZ")) {
                Obscuration::Haze
            } else {
                Obscuration::None
            };
            conditions.precipitation = if weather.iter().any(|v| v.contains("SN")) {
                Precipitation::Snow
            } else if weather.iter().any(|v| v.contains("RA") || v.contains("DZ")) {
                Precipitation::Rain
            } else {
                Precipitation::None
            };
        }
        // Reported visibility already includes precipitation. Conditions stores a baseline that
        // the renderer attenuates for rain/snow, so normalize before applying the observation.
        if let Some(visibility) = o.visibility_sm {
            let factor = match conditions.precipitation {
                Precipitation::None => 1.,
                Precipitation::Rain => 0.75,
                Precipitation::Snow => 0.5,
            };
            conditions.visibility_sm = (visibility / factor).clamp(0.125, 100.);
        }
        let mut wind = self.sim.weather();
        if let Some(w) = &o.wind {
            if let Some(v) = w.from_true_deg {
                wind.wind_from_deg =
                    (v - self.sim.heading_reference().grid_north_true_rad.to_degrees()).rem_euclid(360.);
            }
            wind.wind_speed_mps = w.speed_kt * 0.514444;
        }
        let transition = Transition {
            time_s: self.sim.state().time_s,
            duration_s: if follow { 60. } else { 0. },
            observation: observation.clone(),
            initial_wind: self.sim.weather(),
            target_wind: wind,
            initial_conditions: self.sim.conditions().clone(),
            target_conditions: conditions,
        };
        transition.validate().map_err(CommandError::Failed)?;
        transition.apply_at(self, self.sim.state().time_s)?;
        self.comms.record.live_weather.push(transition.clone());
        self.live_weather.applied_retrieval = Some(observation.retrieved_at);
        self.live_weather.transition = if follow { Some(transition) } else { None };
        self.comms.next_weather = self.sim.state().time_s;
        Ok(())
    }
    pub(crate) fn tick_live_transition(&mut self) {
        if let Some(t) = self.live_weather.transition.take() {
            let now = self.sim.state().time_s + flightcraft_sim::DT_S;
            if let Err(e) = t.apply_tick(self, now) {
                self.live_weather.load_errors.push(e.to_string());
            } else if now < t.time_s + t.duration_s {
                self.live_weather.transition = Some(t);
            }
        }
    }
}
