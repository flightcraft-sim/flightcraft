use super::{fetch, source::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, mpsc};

pub const STALE_SECONDS: i64 = 90 * 60;
pub fn now() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64)
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.) as i64
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LiveObservation {
    pub source: WeatherSource,
    pub retrieved_at: i64,
    /// Unix UTC seconds; None when the source provides no observation time.
    pub observed_at: Option<i64>,
    pub extracted: Extracted,
}
impl LiveObservation {
    pub fn validate(&self) -> Result<(), String> {
        self.source.validate()?;
        self.extracted.observation.validate()?;
        if self.retrieved_at < 0
            || self.observed_at.is_some_and(|t| t < 0)
            || self.extracted.matches.len() > 16
            || self
                .extracted
                .matches
                .iter()
                .any(|(k, v)| k.len() > 64 || v.len() > 16 * 1024)
        {
            return Err("Recorded observation exceeds size or time bounds".into());
        }
        Ok(())
    }

    pub fn age_seconds(&self, at: i64) -> i64 {
        at.saturating_sub(self.observed_at.unwrap_or(self.retrieved_at))
            .max(0)
    }
    pub fn stale(&self, at: i64) -> bool {
        self.age_seconds(at) > STALE_SECONDS
    }
    pub fn from_body(source: WeatherSource, body: &[u8], at: i64) -> Result<Self, String> {
        let extracted = source.extract(body)?;
        let observed_at = observation_time(&extracted, at)?;
        Ok(Self {
            source,
            retrieved_at: at,
            observed_at,
            extracted,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SourceStatus {
    pub source: WeatherSource,
    pub last_good: Option<LiveObservation>,
    pub error: Option<String>,
    pub fetching: bool,
    pub next_attempt_at: i64,
    pub failures: u32,
    pub age_seconds: Option<i64>,
    pub stale: bool,
}
impl SourceStatus {
    pub(crate) fn new(source: WeatherSource) -> Self {
        Self {
            source,
            last_good: None,
            error: None,
            fetching: false,
            next_attempt_at: 0,
            failures: 0,
            age_seconds: None,
            stale: false,
        }
    }
    pub(crate) fn finish(&mut self, result: Result<LiveObservation, String>, at: i64) {
        self.fetching = false;
        match result {
            Ok(good) => {
                self.last_good = Some(good);
                self.error = None;
                self.failures = 0;
            }
            Err(e) => {
                self.error = Some(e);
                self.failures = self.failures.saturating_add(1);
            }
        }
        let base = i64::from(self.source.refresh_seconds);
        let delay = base
            .saturating_mul(1i64 << self.failures.min(6))
            .min(base.max(3600));
        self.next_attempt_at = at.saturating_add(delay);
        self.refresh_age(at);
    }
    fn refresh_age(&mut self, at: i64) {
        self.age_seconds = self.last_good.as_ref().map(|o| o.age_seconds(at));
        self.stale = self.last_good.as_ref().is_some_and(|o| o.stale(at));
    }
}
#[derive(Debug)]
pub(crate) enum Completion {
    Observation(String, Box<Result<LiveObservation, String>>),
    Nearby {
        airport: String,
        result: Result<Vec<super::nearby::NearbyStation>, String>,
    },
}
#[derive(Clone, Debug)]
pub struct Runtime {
    pub entries: BTreeMap<String, SourceStatus>,
    pub saved_sources: BTreeMap<String, WeatherSource>,
    pub loaded: bool,
    pub load_errors: Vec<String>,
    pub(crate) sender: mpsc::Sender<Completion>,
    pub nearby: BTreeMap<String, super::nearby::NearbyStatus>,
    receiver: Arc<Mutex<mpsc::Receiver<Completion>>>,
    pub following: Option<String>,
    pub applied_retrieval: Option<i64>,
    pub transition: Option<super::apply::Transition>,
    pub generation: u64,
}
impl Default for Runtime {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            nearby: BTreeMap::new(),
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
            entries: BTreeMap::new(),
            saved_sources: BTreeMap::new(),
            loaded: false,
            load_errors: Vec::new(),
            following: None,
            applied_retrieval: None,
            transition: None,
            generation: 0,
        }
    }
}
impl Runtime {
    pub fn insert(&mut self, source: WeatherSource) -> Result<(), String> {
        source.validate()?;
        if let Some(old) = self.entries.get(&source.id) {
            if old.source == source {
                return Ok(());
            }
            if old.fetching || old.next_attempt_at > now() {
                return Err("Wait for the source refresh interval before changing its settings".into());
            }
        }
        if self.entries.len() >= 128 && !self.entries.contains_key(&source.id) {
            return Err("At most 128 weather sources can be loaded".into());
        }
        self.entries.insert(source.id.clone(), SourceStatus::new(source));
        Ok(())
    }
    pub fn poll(&mut self, at: i64) {
        let mut completed = Vec::new();
        if let Ok(receiver) = self.receiver.lock() {
            while let Ok(result) = receiver.try_recv() {
                completed.push(result);
            }
        }
        for result in completed {
            match result {
                Completion::Observation(id, result) => {
                    if let Some(entry) = self.entries.get_mut(&id) {
                        entry.finish(*result, at);
                    }
                }
                Completion::Nearby { airport, result } => self.finish_nearby(airport, result, at),
            }
        }
        for entry in self.entries.values_mut() {
            entry.refresh_age(at);
        }
    }
    pub fn request(&mut self, id: &str, at: i64) -> Result<(), String> {
        let active = self.entries.values().filter(|s| s.fetching).count()
            + self.nearby.values().filter(|s| s.fetching).count();
        let entry = self.entries.get_mut(id).ok_or("Choose a weather source first")?;
        if entry.fetching || at < entry.next_attempt_at {
            return Ok(());
        }
        if active >= 4 {
            return Err("Four weather requests are already running; try again shortly".into());
        }
        entry.fetching = true;
        entry.next_attempt_at = at.saturating_add(i64::from(entry.source.refresh_seconds));
        let source = entry.source.clone();
        let sender = self.sender.clone();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let id = source.id.clone();
            if let Err(e) = std::thread::Builder::new()
                .name("weather-fetch".into())
                .spawn(move || {
                    let result = fetch::fetch(&source)
                        .and_then(|b| LiveObservation::from_body(source.clone(), &b, now()));
                    let _ = sender.send(Completion::Observation(source.id, Box::new(result)));
                })
                && let Some(entry) = self.entries.get_mut(&id)
            {
                entry.finish(Err(format!("Cannot start weather request: {e}")), at);
            }
        }
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch::fetch_browser(&source)
                .await
                .and_then(|b| LiveObservation::from_body(source.clone(), &b, now()));
            let _ = sender.send(Completion::Observation(source.id, Box::new(result)));
        });
        Ok(())
    }
}
/// Resolve DDHHMMZ against adjacent calendar months, never treating an old report as newly issued.
fn observation_time(extracted: &Extracted, at: i64) -> Result<Option<i64>, String> {
    use time::{Date, Month, OffsetDateTime};
    let timestamp = |date: Date, hour: u8, minute: u8| {
        date.with_hms(hour, minute, 0)
            .ok()
            .map(|t| t.assume_utc().unix_timestamp())
    };
    let mut field_time = None;
    if let (Some(d), Some(t)) = (extracted.matches.get("date"), extracted.matches.get("time")) {
        let date = d
            .split('-')
            .filter_map(|v| v.parse::<i32>().ok())
            .collect::<Vec<_>>();
        let time = t
            .split([':', ' '])
            .filter_map(|v| v.parse::<u8>().ok())
            .collect::<Vec<_>>();
        if let (Some(&year), Some(&month), Some(&day), Some(&hour), Some(&minute)) =
            (date.first(), date.get(1), date.get(2), time.first(), time.get(1))
        {
            let value = u8::try_from(month)
                .ok()
                .and_then(|m| Month::try_from(m).ok())
                .zip(u8::try_from(day).ok())
                .and_then(|(m, d)| Date::from_calendar_date(year, m, d).ok())
                .and_then(|d| timestamp(d, hour, minute));
            if let Some(value) = value {
                if value > at.saturating_add(600) {
                    return Err("Observation timestamp is in the future".into());
                }
                field_time = Some(value);
            }
        }
        if field_time.is_none() {
            return Err("Invalid observation date or time".into());
        }
    }
    let Some(t) = &extracted.observation.time else {
        return Ok(field_time);
    };
    // A page's current clock must not make an old METAR look fresh. The page date anchors
    // the calendar month/year; DDHHMMZ remains the observation's authoritative time.
    let anchor = field_time.unwrap_or(at);
    let day = t.get(..2).and_then(|s| s.parse::<u8>().ok());
    let hour = t.get(2..4).and_then(|s| s.parse::<u8>().ok());
    let minute = t.get(4..6).and_then(|s| s.parse::<u8>().ok());
    let now = OffsetDateTime::from_unix_timestamp(anchor).map_err(|_| "Invalid observation date")?;
    let mut candidates = Vec::new();
    if let (Some(day), Some(hour), Some(minute)) = (day, hour, minute) {
        let month_index = now.year() * 12 + i32::from(now.month() as u8) - 1;
        for shift in -1..=1 {
            let index = month_index + shift;
            let month = Month::try_from((index.rem_euclid(12) + 1) as u8).map_err(|_| "Invalid month")?;
            if let Some(value) = Date::from_calendar_date(index.div_euclid(12), month, day)
                .ok()
                .and_then(|d| timestamp(d, hour, minute))
            {
                candidates.push(value);
            }
        }
    }
    let value = candidates
        .into_iter()
        .min_by_key(|v| v.abs_diff(anchor))
        .ok_or("Invalid METAR observation time")?;
    if value > at.saturating_add(600) {
        return Err("Observation timestamp is in the future".into());
    }
    Ok(Some(value))
}

pub fn utc_label(timestamp: i64) -> Option<String> {
    let t = time::OffsetDateTime::from_unix_timestamp(timestamp).ok()?;
    Some(format!("{} {:02}:{:02} UTC", t.date(), t.hour(), t.minute()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    fn server(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/weather", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(Instant::now() < deadline, "fake weather server timed out");
                    std::thread::sleep(Duration::from_millis(2));
                };
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 1024];
                while !bytes.windows(4).any(|s| s == b"\r\n\r\n") {
                    let n = stream.read(&mut buffer).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 8192);
                }
                requests.push(String::from_utf8(bytes).unwrap());
                let _ = stream.write_all(response.as_bytes());
            }
            requests
        });
        (url, thread)
    }
    fn response(code: &str, body: &str, headers: &str) -> String {
        format!(
            "HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
            body.len()
        )
    }
    fn source(url: &str) -> WeatherSource {
        WeatherSource::preset("KZZZ", url, Extraction::MetarText)
    }
    #[test]
    fn native_get_user_agent_no_cookies_and_5xx_backoff() {
        let text = "METAR KZZZ 031400Z 27010KT 10SM CLR 15/05 A3000";
        let (url, thread) = server(vec![
            response("200 OK", text, "Set-Cookie: tracking=1\r\n"),
            response("503 Service Unavailable", "retry later", ""),
        ]);
        let source = source(&url);
        let bytes = fetch::fetch(&source).unwrap();
        let observed = LiveObservation::from_body(source.clone(), &bytes, 1_793_714_400).unwrap();
        let mut state = SourceStatus::new(source.clone());
        state.finish(Ok(observed.clone()), 10);
        assert_eq!(state.next_attempt_at, 70);
        state.finish(
            fetch::fetch(&source).and_then(|b| LiveObservation::from_body(source, &b, now())),
            70,
        );
        assert!(state.error.is_some());
        assert_eq!(state.last_good, Some(observed.clone()));
        assert_eq!(state.next_attempt_at, 190);
        state.finish(Err("HTTP 504".into()), 190);
        assert_eq!(state.next_attempt_at, 430);
        state.refresh_age(observed.observed_at.unwrap() + 5401);
        assert!(state.stale);
        let requests = thread.join().unwrap();
        for request in requests {
            assert!(request.starts_with("GET /weather HTTP/1.1"));
            assert!(request.contains(concat!(
                "FlightCraft/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/flightcraft-sim/flightcraft)"
            )));
            assert!(!request.to_ascii_lowercase().contains("cookie:"));
        }
    }
    #[test]
    fn fetch_caps_binary_and_redirects() {
        let oversized = "x".repeat(MAX_BODY + 1);
        let (url, thread) = server(vec![response("200 OK", &oversized, "")]);
        assert!(fetch::fetch(&source(&url)).is_err());
        thread.join().unwrap();
        let (url, thread) = server(vec![response("200 OK", "\0binary", "")]);
        let source = source(&url);
        let bytes = fetch::fetch(&source).unwrap();
        assert!(source.extract(&bytes).is_err());
        thread.join().unwrap();
        let redirects = (0..4)
            .map(|_| response("302 Found", "", "Location: /weather\r\n"))
            .collect();
        let (url, thread) = server(redirects);
        assert!(fetch::fetch(&super::tests::source(&url)).is_err());
        assert_eq!(thread.join().unwrap().len(), 4);
    }
    #[test]
    fn failure_backoff_last_good_stale_and_success_reset_without_network() {
        let mut source = source("http://127.0.0.1/weather");
        source.extraction = Extraction::Fields {
            fields: Fields {
                altimeter: Some(Selector::Body),
                ..Default::default()
            },
        };
        let observation = LiveObservation::from_body(source.clone(), b"30.31 inHg", 1000).unwrap();
        let mut state = SourceStatus::new(source.clone());
        state.finish(Ok(observation.clone()), 1000);
        assert_eq!(state.next_attempt_at, 1060);
        state.finish(Err("HTTP 504".into()), 1060);
        assert_eq!(state.next_attempt_at, 1180);
        state.finish(Err("HTTP 503".into()), 1180);
        assert_eq!(state.next_attempt_at, 1420);
        assert_eq!(state.last_good, Some(observation.clone()));
        state.refresh_age(6400);
        assert!(!state.stale);
        state.refresh_age(6401);
        assert!(state.stale);
        for _ in 0..10 {
            state.finish(Err("HTTP 504".into()), 7000);
        }
        assert_eq!(state.next_attempt_at, 10600);
        state.finish(Ok(observation), 11000);
        assert_eq!(state.next_attempt_at, 11060);
        assert_eq!(state.failures, 0);
        assert!(state.error.is_none());
        let mut runtime = Runtime::default();
        runtime.insert(source.clone()).unwrap();
        runtime.entries.insert(source.id.clone(), state);
        runtime.request(&source.id, 11059).unwrap();
        assert!(!runtime.entries[&source.id].fetching);
    }
    #[test]
    fn background_request_enforces_refresh_and_retries() {
        let (url, thread) = server(vec![response("504 Gateway Timeout", "retry", "")]);
        let source = source(&url);
        let id = source.id.clone();
        let mut runtime = Runtime::default();
        runtime.insert(source).unwrap();
        runtime.request(&id, 100).unwrap();
        // Completion is injected back into the same queue after waiting only for the fake server.
        let completion = runtime
            .receiver
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        runtime.sender.send(completion).unwrap();
        runtime.poll(100);
        assert!(!runtime.entries[&id].fetching);
        assert!(runtime.entries[&id].error.is_some());
        assert_eq!(runtime.entries[&id].next_attempt_at, 220);
        runtime.request(&id, 219).unwrap();
        assert!(!runtime.entries[&id].fetching);
        assert_eq!(thread.join().unwrap().len(), 1);
        runtime.finish_nearby("1N7".into(), Err("HTTP 504".into()), 100);
        assert_eq!(runtime.nearby["1N7"].next_attempt_at, 220);
        runtime.finish_nearby("1N7".into(), Err("HTTP 503".into()), 220);
        assert_eq!(runtime.nearby["1N7"].next_attempt_at, 460);
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Catalog {
    #[serde(default)]
    pub saved_sources: Vec<WeatherSource>,
    pub sources: BTreeMap<String, SourceStatus>,
    pub nearby: BTreeMap<String, super::nearby::NearbyStatus>,
    pub following: Option<String>,
}

impl Catalog {
    pub fn validate(&self) -> Result<(), String> {
        if self.saved_sources.len() > 128
            || self.sources.len() > 128
            || self.nearby.len() > 128
            || self.nearby.values().any(|n| n.stations.len() > 4)
        {
            return Err("Weather catalog exceeds source limits".into());
        }
        for source in &self.saved_sources {
            source.validate()?;
        }
        for status in self.sources.values() {
            status.source.validate()?;
            if let Some(o) = &status.last_good {
                o.validate()?;
            }
        }
        for nearby in self.nearby.values() {
            for station in &nearby.stations {
                station.observation.validate()?;
                if !station.distance_nm.is_finite() || !(0. ..=150.).contains(&station.distance_nm) {
                    return Err("Invalid nearby reporting distance".into());
                }
            }
        }
        Ok(())
    }
}
