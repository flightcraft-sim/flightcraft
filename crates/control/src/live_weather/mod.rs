//! Opt-in live weather: endpoint descriptions, tolerant observations, and recorded application.
pub mod apply;
pub mod fetch;
pub mod metar;
pub mod nearby;
pub mod runtime;
pub mod source;
mod storage;
#[cfg(test)]
mod tests;
use crate::{Command, CommandError, Session};
pub use apply::RecordedLiveWeather;
pub use runtime::{LiveObservation, SourceStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub use source::{Extraction, Fields, Selector, WeatherSource};

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeatherSources {
    pub airport: Option<String>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct WeatherSourcesOutput {
    pub sources: Vec<SourceStatus>,
    pub following: Option<String>,
    pub errors: Vec<String>,
    pub nearby: Option<nearby::NearbyStatus>,
}
impl Command for WeatherSources {
    const NAME: &'static str = "weather.sources";
    const DESCRIPTION: &'static str = "List built-in and user weather sources, last-good observations, ages, and fetch errors without fetching.";
    type Output = WeatherSourcesOutput;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        s.load_weather_sources();
        let requested_airport = self.airport.clone();
        let mut aliases = self.airport.clone().into_iter().collect::<Vec<_>>();
        if let Some(airport) = self.airport {
            if let Some(a) = s.sim.world().airports().and_then(|f| {
                f.airports
                    .iter()
                    .find(|a| a.ident == airport || a.icao.as_deref() == Some(&airport))
            }) {
                aliases.push(a.ident.clone());
                if let Some(icao) = &a.icao {
                    aliases.push(icao.clone());
                }
            }
            // ICAO identifiers are the default AWC lookup, with no assumed K-prefix for local ids.
            if let Some(icao) = aliases
                .iter()
                .find(|v| v.len() == 4 && v.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
            {
                let source = WeatherSource::aviationweather(icao);
                s.live_weather.insert(source).map_err(CommandError::Failed)?;
            }
        }
        s.poll_live_weather();
        let nearby_ids = requested_airport
            .as_ref()
            .and_then(|a| s.live_weather.nearby.get(a))
            .map(|n| {
                n.stations
                    .iter()
                    .map(|v| v.observation.source.id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(WeatherSourcesOutput {
            sources: s
                .live_weather
                .entries
                .values()
                .filter(|e| {
                    aliases.is_empty()
                        || aliases.contains(&e.source.airport)
                        || nearby_ids.contains(&e.source.id)
                })
                .cloned()
                .collect(),
            nearby: requested_airport.and_then(|a| s.live_weather.nearby.get(&a).cloned()),
            following: s.live_weather.following.clone(),
            errors: s.live_weather.load_errors.clone(),
        })
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeatherSourceTest {
    /// Supply an existing id, or a complete source description to test before saving.
    pub source_id: Option<String>,
    pub source: Option<WeatherSource>,
}
impl Command for WeatherSourceTest {
    const NAME: &'static str = "weather.source_test";
    const DESCRIPTION: &'static str = "Start a bounded background GET and parse one source; poll weather.sources for exact matches and parsed fields. Refresh/backoff applies to tests too.";
    type Output = SourceStatus;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        s.load_weather_sources();
        let id = match (self.source_id, self.source) {
            (Some(id), None) => id,
            (None, Some(source)) => {
                let id = source.id.clone();
                s.live_weather.insert(source).map_err(CommandError::Failed)?;
                id
            }
            _ => {
                return Err(crate::invalid(
                    Self::NAME,
                    "Supply one source id or one complete source",
                ));
            }
        };
        s.live_weather.poll(runtime::now());
        if s.presentation_commands.is_none() {
            s.live_weather
                .request(&id, runtime::now())
                .map_err(CommandError::Failed)?;
        }
        s.live_weather
            .entries
            .get(&id)
            .cloned()
            .ok_or_else(|| CommandError::Failed("Weather source unavailable".into()))
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeatherSourceSave {
    pub source: WeatherSource,
}
impl Command for WeatherSourceSave {
    const NAME: &'static str = "weather.source_save";
    const DESCRIPTION: &'static str = "Save a successfully tested source to the user weather-sources configuration directory; never opens a dialog.";
    type Output = SourceStatus;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        self.source.validate().map_err(CommandError::Failed)?;
        let entry = s
            .live_weather
            .entries
            .get(&self.source.id)
            .filter(|e| {
                e.source == self.source
                    && e.error.is_none()
                    && e.last_good.as_ref().is_some_and(|v| v.source == self.source)
            })
            .ok_or_else(|| {
                CommandError::Failed("Test these exact source settings successfully before saving".into())
            })?;
        let output = entry.clone();
        if s.presentation_commands.is_none() {
            if s.live_weather.saved_sources.contains_key(&self.source.id)
                || s.live_weather.saved_sources.len() >= 128
            {
                return Err(CommandError::Failed("Source already saved, or the saved-source limit was reached. Choose a different name for a new source.".into()));
            }
            storage::save(&self.source).map_err(CommandError::Failed)?;
            s.live_weather
                .saved_sources
                .insert(self.source.id.clone(), self.source);
        }
        Ok(output)
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum WeatherLive {
    UseNow {
        source_id: String,
    },
    Follow {
        source_id: String,
    },
    Stop,
    /// Replay a saved application, including its resolved wind reference and transition.
    Replay {
        record: Box<RecordedLiveWeather>,
    },
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct WeatherLiveOutput {
    pub following: Option<String>,
    pub applied: Option<LiveObservation>,
}
impl Command for WeatherLive {
    const NAME: &'static str = "weather.live";
    const DESCRIPTION: &'static str = "Use a fresh tested observation now, follow with one-minute fixed-step transitions, stop following, or replay a recorded application without networking.";
    type Output = WeatherLiveOutput;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if let Self::Replay { record } = self {
            record.validate().map_err(CommandError::Failed)?;
            s.live_weather.following = None;
            record.apply_at(s, s.sim.state().time_s)?;
            s.live_weather.transition =
                (s.sim.state().time_s < record.time_s + record.duration_s).then_some(*record.clone());
            return Ok(WeatherLiveOutput {
                following: None,
                applied: Some(record.observation),
            });
        }
        if matches!(self, Self::Stop) {
            s.live_weather.following = None;
            if let Some(transition) = s.live_weather.transition.take() {
                if s.comms.record.live_weather.len() >= 2000 {
                    s.live_weather.transition = Some(transition);
                    return Err(CommandError::Failed("Following stopped. The accepted blend will finish because the flight record is full.".into()));
                }
                let wind = s.sim.weather();
                let conditions = s.sim.conditions().clone();
                let freeze = RecordedLiveWeather {
                    time_s: s.sim.state().time_s,
                    duration_s: 0.,
                    observation: transition.observation,
                    initial_wind: wind,
                    target_wind: wind,
                    initial_conditions: conditions.clone(),
                    target_conditions: conditions,
                };
                freeze.apply_at(s, s.sim.state().time_s)?;
                s.comms.record.live_weather.push(freeze);
            }
            return Ok(WeatherLiveOutput {
                following: None,
                applied: None,
            });
        }
        let (source_id, follow) = match self {
            Self::UseNow { source_id } => (source_id, false),
            Self::Follow { source_id } => (source_id, true),
            _ => return Err(CommandError::Failed("Invalid live weather action".into())),
        };
        s.live_weather.poll(runtime::now());
        let observation = s
            .live_weather
            .entries
            .get(&source_id)
            .and_then(|e| e.last_good.clone())
            .ok_or_else(|| {
                CommandError::Failed("Test the source and wait for an observation first".into())
            })?;
        if observation.stale(runtime::now()) {
            return Err(CommandError::Failed(
                "Observation is older than 90 minutes; wait for fresh data".into(),
            ));
        }
        s.accept_live(observation.clone(), follow)?;
        s.live_weather.following = follow.then_some(source_id);
        Ok(WeatherLiveOutput {
            following: s.live_weather.following.clone(),
            applied: Some(observation),
        })
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeatherPhone {
    pub airport: String,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct WeatherPhoneOutput {
    pub text: String,
    pub observation: Option<LiveObservation>,
}
impl Command for WeatherPhone {
    const NAME: &'static str = "weather.phone";
    const DESCRIPTION: &'static str = "Read the airport weather-by-phone transcript from its last-good live observation; does not make a phone call.";
    type Output = WeatherPhoneOutput;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let observation = s.live_for_airport(&self.airport).cloned();
        let text = if let Some(o) = &observation {
            let station = s.stations().into_iter().find(|v| {
                v.airport == self.airport
                    && matches!(
                        v.kind,
                        flightcraft_world::FacilityKind::Awos | flightcraft_world::FacilityKind::Asos
                    )
            });
            s.live_weather_words(o, station.as_ref())
        } else if let Some(station) = s.stations().iter().find(|v| {
            v.airport == self.airport
                && matches!(
                    v.kind,
                    flightcraft_world::FacilityKind::Awos | flightcraft_world::FacilityKind::Asos
                )
        }) {
            s.station_weather_report(station).0
        } else {
            "No weather observation is available for this airport. Test a source in Weather → Live.".into()
        };
        Ok(WeatherPhoneOutput { text, observation })
    }
}
impl Session {
    /// Accepted user definitions, excluding transient source tests.
    pub fn saved_weather_sources(&self) -> Vec<WeatherSource> {
        self.live_weather.saved_sources.values().cloned().collect()
    }
    pub(crate) fn load_weather_sources(&mut self) {
        if self.live_weather.loaded {
            return;
        }
        self.live_weather.loaded = true;
        match WeatherSource::from_json(include_str!(
            "../../../../examples/weather-sources/1n7-digiwx.json"
        )) {
            Ok(source) => {
                if let Err(e) = self.live_weather.insert(source) {
                    self.live_weather.load_errors.push(e);
                }
            }
            Err(e) => self.live_weather.load_errors.push(e),
        }
        let (sources, errors) = storage::load();
        self.live_weather.load_errors.extend(errors);
        for source in sources {
            self.live_weather
                .saved_sources
                .insert(source.id.clone(), source.clone());
            if let Err(e) = self.live_weather.insert(source) {
                self.live_weather.load_errors.push(e);
            }
        }
    }
    pub(crate) fn poll_live_weather(&mut self) {
        if self.presentation_commands.is_some() {
            return;
        }
        if self.live_weather.generation != self.flight_generation {
            self.live_weather.generation = self.flight_generation;
            self.live_weather.following = None;
            self.live_weather.transition = None;
            self.live_weather.applied_retrieval = None;
        }
        let now = runtime::now();
        self.live_weather.poll(now);
        if let Some(id) = self.live_weather.following.clone() {
            if let Some(o) = self
                .live_weather
                .entries
                .get(&id)
                .and_then(|s| s.last_good.clone())
                && !o.stale(now)
                && self.live_weather.applied_retrieval != Some(o.retrieved_at)
                && let Err(e) = self.accept_live(o, true)
            {
                self.live_weather.following = None;
                self.live_weather.load_errors.push(e.to_string());
            }
            if let Err(e) = self.live_weather.request(&id, now) {
                self.live_weather.load_errors.push(e);
            }
            let airport = self
                .live_weather
                .entries
                .get(&id)
                .map(|s| s.source.airport.clone());
            if let Some(airport) = airport
                && self.live_weather.nearby.contains_key(&airport)
                && let Some(a) = self.sim.world().airports().and_then(|f| {
                    f.airports
                        .iter()
                        .find(|a| a.ident == airport || a.icao.as_deref() == Some(&airport))
                })
            {
                let _ = self
                    .live_weather
                    .request_nearby(&airport, [a.latitude_deg, a.longitude_deg], now);
            }
        }
    }
    pub(crate) fn live_for_airport(&self, airport: &str) -> Option<&LiveObservation> {
        let icao = self
            .sim
            .world()
            .airports()
            .and_then(|f| f.airports.iter().find(|a| a.ident == airport))
            .and_then(|a| a.icao.as_deref());
        self.live_weather
            .entries
            .values()
            .filter(|v| v.source.airport == airport || Some(v.source.airport.as_str()) == icao)
            .filter_map(|v| v.last_good.as_ref())
            .max_by_key(|v| {
                (
                    self.live_weather.following.as_ref() == Some(&v.source.id),
                    v.observed_at.unwrap_or(v.retrieved_at),
                )
            })
    }
    pub(crate) fn live_weather_words(
        &self,
        o: &LiveObservation,
        station: Option<&flightcraft_world::ComStation>,
    ) -> String {
        let location = station
            .map(|s| (s.position.latitude_deg, s.position.longitude_deg, s.elevation_m))
            .or_else(|| {
                self.sim
                    .world()
                    .airports()
                    .and_then(|f| {
                        f.airports.iter().find(|a| {
                            a.ident == o.source.airport || a.icao.as_deref() == Some(&o.source.airport)
                        })
                    })
                    .map(|a| (a.latitude_deg, a.longitude_deg, a.elevation_m))
            });
        let declination = location
            .and_then(|(lat, lon, height)| {
                flightcraft_world::geomag::MagneticModel::wmm2025()
                    .ok()?
                    .field(lat, lon, height, self.simulation_date().decimal_year().ok()?)
                    .ok()
            })
            .map(|f| f.declination_deg);
        crate::comms::phraseology::live_weather(
            o,
            &self.weather_observation_name(&o.source.airport, station),
            declination,
            runtime::now(),
        )
    }
    pub(crate) fn weather_observation_name(
        &self,
        airport: &str,
        station: Option<&flightcraft_world::ComStation>,
    ) -> String {
        self.sim
            .world()
            .airports()
            .and_then(|f| {
                f.airports
                    .iter()
                    .find(|a| a.ident == airport || a.icao.as_deref() == Some(airport))
            })
            .map(|a| crate::comms::phraseology::normal_case(&a.name))
            .or_else(|| station.map(crate::comms::phraseology::weather_station_name))
            .unwrap_or_else(|| airport.into())
    }
    pub(crate) fn station_weather_report(
        &self,
        station: &flightcraft_world::ComStation,
    ) -> (String, crate::Meaning) {
        if let Some(o) = self.live_for_airport(&station.airport) {
            (
                self.live_weather_words(o, Some(station)),
                crate::Meaning::LiveWeather {
                    observation: Box::new(o.clone()),
                },
            )
        } else {
            crate::comms::weather::weather_report(
                station,
                &self.weather_observation_name(&station.airport, Some(station)),
                self.sim.conditions(),
                self.pilot_weather(),
                self.sim.state().time_s,
            )
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeatherNearby {
    pub airport: String,
}
impl Command for WeatherNearby {
    const NAME: &'static str = "weather.nearby";
    const DESCRIPTION: &'static str = "Find up to four nearest official METAR stations within 150 nm of a loaded airport; results keep their station identity, distance, and direction. Uses background GET with backoff.";
    type Output = nearby::NearbyStatus;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let airport = s
            .sim
            .world()
            .airports()
            .and_then(|f| {
                f.airports
                    .iter()
                    .find(|a| a.ident == self.airport || a.icao.as_deref() == Some(&self.airport))
            })
            .ok_or_else(|| {
                CommandError::Failed("Load this airport's region to find nearby reporting stations".into())
            })?;
        let position = [airport.latitude_deg, airport.longitude_deg];
        if s.presentation_commands.is_some() {
            return Ok(nearby::NearbyStatus {
                fetching: true,
                ..Default::default()
            });
        }
        s.live_weather
            .request_nearby(&self.airport, position, runtime::now())
            .map_err(CommandError::Failed)?;
        s.live_weather
            .nearby
            .get(&self.airport)
            .cloned()
            .ok_or_else(|| CommandError::Failed("Nearby search unavailable".into()))
    }
}

/// Saved configuration for a new browser session owner; test-only drafts are excluded.
pub(crate) fn user_sources_for_seed(s: &Session) -> Vec<WeatherSource> {
    if s.live_weather.loaded {
        s.live_weather.saved_sources.values().cloned().collect()
    } else {
        storage::load().0
    }
}
/// Commit the SharedWorker's accepted source settings in its connected browser window.
#[cfg(target_arch = "wasm32")]
pub fn persist_browser_weather_sources(sources: &[WeatherSource]) -> Result<(), String> {
    storage::replace_browser_sources(sources)
}
