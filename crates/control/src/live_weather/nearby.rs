//! Nearby reports retain their own station identity and are never relabeled as field weather.
use super::{LiveObservation, SourceStatus, WeatherSource, runtime, source::MAX_BODY};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct NearbyStation {
    pub station: String,
    pub distance_nm: f64,
    pub direction: String,
    pub observation: LiveObservation,
}
impl NearbyStation {
    pub fn label(&self) -> String {
        format!(
            "{} · nearest reporting station, {:.0} nm {}",
            self.station, self.distance_nm, self.direction
        )
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct NearbyStatus {
    pub stations: Vec<NearbyStation>,
    pub fetching: bool,
    pub error: Option<String>,
    pub next_attempt_at: i64,
    pub failures: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CrossCheck {
    pub official_altimeter_mean_inhg: Option<f64>,
    pub altimeter_difference_inhg: Option<f64>,
    pub altimeter_disagreement: bool,
    pub older_than_neighbors_seconds: Option<i64>,
    pub time_disagreement: bool,
}
pub fn compare(local: &LiveObservation, stations: &[NearbyStation], at: i64) -> CrossCheck {
    let official = stations
        .iter()
        .filter(|s| !s.observation.stale(at))
        .collect::<Vec<_>>();
    let altimeters = official
        .iter()
        .filter_map(|s| s.observation.extracted.observation.altimeter_inhg)
        .collect::<Vec<_>>();
    let mean = (!altimeters.is_empty()).then(|| altimeters.iter().sum::<f64>() / altimeters.len() as f64);
    let difference = local
        .extracted
        .observation
        .altimeter_inhg
        .zip(mean)
        .map(|(local, mean)| local - mean);
    let newest = official.iter().filter_map(|s| s.observation.observed_at).max();
    let older = local
        .observed_at
        .zip(newest)
        .map(|(local, newest)| newest.saturating_sub(local));
    CrossCheck {
        official_altimeter_mean_inhg: mean,
        altimeter_difference_inhg: difference,
        altimeter_disagreement: difference.is_some_and(|d| d.abs() > 0.05 + 1e-9),
        older_than_neighbors_seconds: older,
        time_disagreement: older.is_some_and(|d| d > 3600),
    }
}
pub fn discovery_source(airport: &str, position: [f64; 2]) -> Result<WeatherSource, String> {
    let [lat, lon] = position;
    if !lat.is_finite()
        || !lon.is_finite()
        || !(-90. ..=90.).contains(&lat)
        || !(-180. ..=180.).contains(&lon)
    {
        return Err("Invalid airport location".into());
    }
    // Enclose a 150 nm search circle, including at high latitudes and the date line.
    let lat_margin = 2.6;
    let lon_margin = (lat_margin / libm::cos(lat.to_radians()).abs().max(0.01)).min(180.);
    let (west, east) = if lat.abs() + lat_margin >= 90. || lon - lon_margin < -180. || lon + lon_margin > 180.
    {
        (-180., 180.)
    } else {
        (lon - lon_margin, lon + lon_margin)
    };
    let mut source = WeatherSource::aviationweather(airport);
    source.url = format!(
        "https://aviationweather.gov/api/data/metar?bbox={:.4},{:.4},{:.4},{:.4}&format=json&hours=2",
        (lat - lat_margin).max(-90.),
        west,
        (lat + lat_margin).min(90.),
        east
    );
    source.validate()?;
    Ok(source)
}
pub fn parse_reports(
    bytes: &[u8],
    position: [f64; 2],
    airport: &str,
    at: i64,
) -> Result<Vec<NearbyStation>, String> {
    if bytes.len() > MAX_BODY {
        return Err("Nearby weather response exceeds 1 MB".into());
    }
    let reports: Vec<serde_json::Value> = serde_json::from_slice(bytes)
        .map_err(|_| "No nearby official METAR reports were returned".to_string())?;
    let mut stations = Vec::new();
    for row in reports.into_iter().take(4096) {
        let Some(ident) = row.get("icaoId").and_then(|v| v.as_str()) else {
            continue;
        };
        if ident == airport
            || ident.len() != 4
            || !ident
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            continue;
        }
        let Some(lat) = row
            .get("lat")
            .and_then(|v| v.as_f64())
            .filter(|v| (-90. ..=90.).contains(v))
        else {
            continue;
        };
        let Some(lon) = row
            .get("lon")
            .and_then(|v| v.as_f64())
            .filter(|v| (-180. ..=180.).contains(v))
        else {
            continue;
        };
        let mut source = WeatherSource::aviationweather(ident);
        source.name = format!("{ident} · Aviation Weather Center");
        let body = serde_json::to_vec(&vec![row.clone()]).map_err(|e| e.to_string())?;
        let Ok(mut observation) = LiveObservation::from_body(source, &body, at) else {
            continue;
        };
        if let Some(time) = row
            .get("obsTime")
            .and_then(|v| v.as_i64())
            .filter(|v| *v <= at.saturating_add(600))
        {
            observation.observed_at = Some(time);
        }
        let (distance, angle) = flightcraft_sim::avionics::navigation::inverse_nm(position, [lat, lon]);
        if distance > 150. {
            continue;
        }
        let octant = ((angle.rem_euclid(360.) + 22.5) / 45.).floor() as usize % 8;
        let direction = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"]
            .get(octant)
            .copied()
            .unwrap_or("N")
            .into();
        stations.push(NearbyStation {
            station: ident.into(),
            distance_nm: distance,
            direction,
            observation,
        });
    }
    // The API can return multiple reports per station. Keep its newest timestamp before
    // ranking stations by distance, independently of response order or coordinate revisions.
    stations.sort_by(|a, b| {
        a.station
            .cmp(&b.station)
            .then_with(|| b.observation.observed_at.cmp(&a.observation.observed_at))
    });
    stations.dedup_by(|a, b| a.station == b.station);
    stations.sort_by(|a, b| {
        a.distance_nm
            .total_cmp(&b.distance_nm)
            .then(a.station.cmp(&b.station))
    });
    stations.truncate(4);
    if stations.is_empty() {
        return Err(
            "No nearby official reporting stations were returned; local sources may still be available"
                .into(),
        );
    }
    Ok(stations)
}
impl super::runtime::Runtime {
    pub fn request_nearby(&mut self, airport: &str, position: [f64; 2], at: i64) -> Result<(), String> {
        if self.nearby.len() >= 128 && !self.nearby.contains_key(airport) {
            return Err("Nearby weather search limit reached".into());
        }
        let active = self.entries.values().filter(|s| s.fetching).count()
            + self.nearby.values().filter(|s| s.fetching).count();
        let entry = self.nearby.entry(airport.into()).or_default();
        if entry.fetching || at < entry.next_attempt_at {
            return Ok(());
        }
        if active >= 4 {
            return Err("Four weather requests are already running; try again shortly".into());
        }
        let source = discovery_source(airport, position)?;
        entry.fetching = true;
        entry.next_attempt_at = at.saturating_add(60);
        let sender = self.sender.clone();
        let airport = airport.to_string();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let error_airport = airport.clone();
            if let Err(e) = std::thread::Builder::new()
                .name("nearby-weather".into())
                .spawn(move || {
                    let result = super::fetch::fetch(&source)
                        .and_then(|b| parse_reports(&b, position, &airport, runtime::now()));
                    let _ = sender.send(runtime::Completion::Nearby { airport, result });
                })
            {
                self.finish_nearby(
                    error_airport,
                    Err(format!("Cannot start nearby weather request: {e}")),
                    at,
                );
            }
        }
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let result = super::fetch::fetch_browser(&source)
                .await
                .and_then(|b| parse_reports(&b, position, &airport, runtime::now()));
            let _ = sender.send(runtime::Completion::Nearby { airport, result });
        });
        Ok(())
    }
    pub(crate) fn finish_nearby(
        &mut self,
        airport: String,
        result: Result<Vec<NearbyStation>, String>,
        at: i64,
    ) {
        let entry = self.nearby.entry(airport).or_default();
        entry.fetching = false;
        match result {
            Ok(stations) => {
                for station in &stations {
                    let observation = &station.observation;
                    if self.entries.len() < 128 || self.entries.contains_key(&observation.source.id) {
                        let mut status = self
                            .entries
                            .remove(&observation.source.id)
                            .unwrap_or_else(|| SourceStatus::new(observation.source.clone()));
                        if !status.fetching {
                            status.finish(Ok(observation.clone()), at);
                        }
                        self.entries.insert(observation.source.id.clone(), status);
                    }
                }
                entry.stations = stations;
                entry.error = None;
                entry.failures = 0;
            }
            Err(e) => {
                entry.error = Some(e);
                entry.failures = entry.failures.saturating_add(1);
            }
        }
        entry.next_attempt_at = at.saturating_add((60i64 << entry.failures.min(6)).min(3600));
    }
}
