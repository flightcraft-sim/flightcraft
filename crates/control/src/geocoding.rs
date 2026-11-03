//! Explicit, bounded address lookup. Local search never invokes these transports.
use crate::{
    Command, CommandError, Session,
    navigation::{failed, valid_point},
};
use flightcraft_world::GeoPoint;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
const MAX_RESPONSE: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GeocodeProvider {
    Census,
    Nominatim,
}
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeocodeLookup {
    pub address: String,
    pub provider: GeocodeProvider,
    #[serde(default)]
    pub allow_network: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct GeocodeResult {
    pub name: String,
    pub position: GeoPoint,
    pub attribution: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct GeocodeOutput {
    pub provider: GeocodeProvider,
    pub results: Vec<GeocodeResult>,
    pub cached: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct GeocodeState {
    cache: BTreeMap<String, GeocodeOutput>,
}
/// Providers own URL construction, documented response decoding, and mandatory result attribution.
pub trait Geocoder {
    fn url(&self, address: &str) -> String;
    fn parse(&self, response: &str) -> Result<Vec<GeocodeResult>, CommandError>;
}
pub struct CensusGeocoder;
pub struct NominatimGeocoder;
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
fn parse_json(s: &str) -> Result<serde_json::Value, CommandError> {
    if s.len() > MAX_RESPONSE {
        return Err(failed("geocoder response exceeds 1 MiB"));
    }
    serde_json::from_str(s).map_err(|e| failed(e.to_string()))
}
fn result(
    name: Option<&str>,
    lat: Option<f64>,
    lon: Option<f64>,
    attribution: &str,
) -> Result<GeocodeResult, CommandError> {
    let r = GeocodeResult {
        name: name
            .filter(|s| flightcraft_world::navdata::text_ok(s, 2048))
            .ok_or_else(|| failed("invalid geocoder name"))?
            .into(),
        position: GeoPoint {
            latitude_deg: lat.ok_or_else(|| failed("missing latitude"))?,
            longitude_deg: lon.ok_or_else(|| failed("missing longitude"))?,
        },
        attribution: attribution.into(),
    };
    valid_point(r.position)?;
    Ok(r)
}
impl Geocoder for CensusGeocoder {
    fn url(&self, a: &str) -> String {
        format!(
            "https://geocoding.geo.census.gov/geocoder/locations/onelineaddress?address={}&benchmark=Public_AR_Current&format=json",
            encode(a)
        )
    }
    fn parse(&self, s: &str) -> Result<Vec<GeocodeResult>, CommandError> {
        let v = parse_json(s)?;
        let rows = v
            .pointer("/result/addressMatches")
            .and_then(|v| v.as_array())
            .ok_or_else(|| failed("invalid Census addressMatches"))?;
        if rows.len() > 20 {
            return Err(failed("too many geocoder results"));
        }
        rows.iter()
            .map(|v| {
                result(
                    v.get("matchedAddress").and_then(|v| v.as_str()),
                    v.pointer("/coordinates/y").and_then(|v| v.as_f64()),
                    v.pointer("/coordinates/x").and_then(|v| v.as_f64()),
                    "US Census Bureau, public domain",
                )
            })
            .collect()
    }
}
impl Geocoder for NominatimGeocoder {
    fn url(&self, a: &str) -> String {
        format!(
            "https://nominatim.openstreetmap.org/search?q={}&format=jsonv2&limit=10",
            encode(a)
        )
    }
    fn parse(&self, s: &str) -> Result<Vec<GeocodeResult>, CommandError> {
        let v = parse_json(s)?;
        let rows = v.as_array().ok_or_else(|| failed("invalid Nominatim response"))?;
        if rows.len() > 20 {
            return Err(failed("too many geocoder results"));
        }
        rows.iter()
            .map(|v| {
                result(
                    v.get("display_name").and_then(|v| v.as_str()),
                    v.get("lat").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()),
                    v.get("lon").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()),
                    "© OpenStreetMap contributors",
                )
            })
            .collect()
    }
}
fn provider(p: GeocodeProvider) -> Box<dyn Geocoder> {
    match p {
        GeocodeProvider::Census => Box::new(CensusGeocoder),
        GeocodeProvider::Nominatim => Box::new(NominatimGeocoder),
    }
}
fn key(a: &GeocodeLookup) -> String {
    format!("{:?}:{}", a.provider, a.address.trim().to_lowercase())
}
impl GeocodeLookup {
    fn validate(&self) -> Result<(), CommandError> {
        if !self.allow_network {
            return Err(failed(
                "geocode.lookup requires explicit allow_network: true; confirm the provider and address first",
            ));
        }
        if !flightcraft_world::navdata::text_ok(self.address.trim(), 512) {
            return Err(failed("address must contain 1–512 printable bytes"));
        }
        Ok(())
    }
}
impl Session {
    pub fn cache_geocode_result(
        &mut self,
        request: &GeocodeLookup,
        output: GeocodeOutput,
    ) -> Result<(), CommandError> {
        request.validate()?;
        self.load_geocode_cache()?;
        if output.provider != request.provider || output.results.len() > 20 {
            return Err(failed("invalid geocode cache entry"));
        }
        for r in &output.results {
            valid_point(r.position)?;
        }
        let mut cache = self.navigation.geocode.cache.clone();
        if cache.len() >= 128
            && !cache.contains_key(&key(request))
            && let Some(first) = cache.keys().next().cloned()
        {
            cache.remove(&first);
        }
        cache.insert(key(request), output);
        let text = serde_json::to_string(&cache).map_err(|e| failed(e.to_string()))?;
        if text.len() > MAX_RESPONSE {
            return Err(failed("geocode cache exceeds 1 MiB"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(dir) = &self.navigation.storage {
            std::fs::create_dir_all(dir).map_err(|e| failed(e.to_string()))?;
            let tmp = dir.join("geocode-cache.json.tmp");
            std::fs::write(&tmp, &text)
                .and_then(|()| std::fs::rename(&tmp, dir.join("geocode-cache.json")))
                .map_err(|e| failed(e.to_string()))?;
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(storage) = web_sys::window()
                .ok_or_else(|| failed("window unavailable"))?
                .local_storage()
                .map_err(|_| failed("browser storage denied"))?
            {
                storage
                    .set_item("flightcraft.geocode.v1", &text)
                    .map_err(|_| failed("browser cache storage denied or full"))?;
            }
        }
        self.navigation.geocode.cache = cache;
        Ok(())
    }
    fn load_geocode_cache(&mut self) -> Result<(), CommandError> {
        #[cfg(not(target_arch = "wasm32"))]
        let text = if let Some(dir) = &self.navigation.storage {
            match std::fs::File::open(dir.join("geocode-cache.json")) {
                Ok(f) => {
                    use std::io::Read;
                    let mut t = String::new();
                    f.take(MAX_RESPONSE as u64 + 1)
                        .read_to_string(&mut t)
                        .map_err(|e| failed(e.to_string()))?;
                    Some(t)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(failed(e.to_string())),
            }
        } else {
            None
        };
        #[cfg(target_arch = "wasm32")]
        let text = match web_sys::window()
            .ok_or_else(|| failed("window unavailable"))?
            .local_storage()
            .map_err(|_| failed("browser storage denied"))?
        {
            Some(s) => s
                .get_item("flightcraft.geocode.v1")
                .map_err(|_| failed("browser cache storage denied"))?,
            None => None,
        };
        if let Some(text) = text {
            if text.len() > MAX_RESPONSE {
                return Err(failed("geocode cache too large"));
            }
            let cache: BTreeMap<String, GeocodeOutput> =
                serde_json::from_str(&text).map_err(|e| failed(e.to_string()))?;
            if cache.len() > 128 {
                return Err(failed("geocode cache count"));
            }
            for (k, v) in &cache {
                if k.len() > 550 || v.results.len() > 20 {
                    return Err(failed("geocode cache entry bounds"));
                }
                for r in &v.results {
                    valid_point(r.position)?;
                    if !flightcraft_world::navdata::text_ok(&r.name, 2048)
                        || r.attribution
                            != match v.provider {
                                GeocodeProvider::Census => "US Census Bureau, public domain",
                                GeocodeProvider::Nominatim => "© OpenStreetMap contributors",
                            }
                    {
                        return Err(failed("invalid geocode cache attribution/name"));
                    }
                }
            }
            self.navigation.geocode.cache = cache;
        }
        Ok(())
    }
}
impl Command for GeocodeLookup {
    const NAME: &'static str = "geocode.lookup";
    const DESCRIPTION: &'static str = "Look up one address with Census or Nominatim only with explicit allow_network:true; cache locally and attribute every result.";
    type Output = GeocodeOutput;
    fn run(self, s: &mut Session) -> Result<GeocodeOutput, CommandError> {
        self.validate()?;
        s.load_geocode_cache()?;
        if let Some(v) = s.navigation.geocode.cache.get(&key(&self)) {
            let mut v = v.clone();
            v.cached = true;
            return Ok(v);
        }
        if s.camera.is_some() {
            return Err(CommandError::AppPending(crate::AppRequest::Geocode(self)));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let output = lookup_native(&self)?;
            s.cache_geocode_result(&self, output.clone())?;
            Ok(output)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = s;
            Err(CommandError::AppPending(crate::AppRequest::Geocode(self)))
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn lookup_native(a: &GeocodeLookup) -> Result<GeocodeOutput, CommandError> {
    use std::{
        io::Read,
        sync::Mutex,
        time::{Duration, Instant},
    };
    // One gate for every session, held through completion: no concurrent or bulk Nominatim requests.
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut gate = if a.provider == GeocodeProvider::Nominatim {
        let guard = LAST
            .lock()
            .map_err(|_| failed("geocoder rate gate unavailable"))?;
        if guard.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return Err(failed(
                "Nominatim allows at most one request per second; retry later",
            ));
        }
        Some(guard)
    } else {
        None
    };
    let client: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .max_redirects(0)
        .build()
        .into();
    if let Some(gate) = &mut gate {
        **gate = Some(Instant::now());
    }
    let p = provider(a.provider);
    let mut response = client
        .get(p.url(&a.address))
        .header(
            "User-Agent",
            "FlightCraft/0.1 (https://github.com/flightcraft-sim/flightcraft)",
        )
        .call()
        .map_err(|e| failed(format!("geocoder request failed: {e}")))?;
    let mut text = String::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_RESPONSE as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| failed(e.to_string()))?;
    let results = p.parse(&text)?;
    if let Some(gate) = &mut gate {
        **gate = Some(Instant::now());
    }
    Ok(GeocodeOutput {
        provider: a.provider,
        results,
        cached: false,
    })
}
#[cfg(target_arch = "wasm32")]
pub async fn lookup_browser(a: GeocodeLookup) -> Result<GeocodeOutput, CommandError> {
    use wasm_bindgen::{JsCast, JsValue};
    thread_local! {static LAST:std::cell::Cell<f64>=const {std::cell::Cell::new(f64::NEG_INFINITY)};static BUSY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
    struct Gate(bool);
    impl Drop for Gate {
        fn drop(&mut self) {
            if self.0 {
                LAST.with(|v| v.set(js_sys::Date::now()));
                BUSY.with(|b| b.set(false));
            }
        }
    }
    a.validate()?;
    let limited = a.provider == GeocodeProvider::Nominatim;
    if limited {
        if BUSY.with(|b| b.get()) || LAST.with(|t| js_sys::Date::now() - t.get() < 1000.0) {
            return Err(failed(
                "Nominatim allows at most one request per second; retry later",
            ));
        }
        BUSY.with(|b| b.set(true));
        LAST.with(|t| t.set(js_sys::Date::now()));
    }
    let _gate = Gate(limited);
    let window = web_sys::window().ok_or_else(|| failed("window unavailable"))?;
    let p = provider(a.provider);
    let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&p.url(&a.address)))
        .await
        .map_err(|_| failed("geocoder fetch failed"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| failed("invalid fetch response"))?;
    if !response.ok() {
        return Err(failed(format!("geocoder HTTP {}", response.status())));
    }
    let stream = response.body().ok_or_else(|| failed("empty response"))?;
    let reader = stream
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
        .map_err(|_| failed("response stream unavailable"))?;
    let mut bytes = Vec::new();
    loop {
        let chunk = wasm_bindgen_futures::JsFuture::from(reader.read())
            .await
            .map_err(|_| failed("response stream failed"))?;
        let done = js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
            .map_err(|_| failed("stream status"))?
            .as_bool()
            .unwrap_or(false);
        if done {
            break;
        }
        let data =
            js_sys::Reflect::get(&chunk, &JsValue::from_str("value")).map_err(|_| failed("stream data"))?;
        let data = js_sys::Uint8Array::new(&data);
        if bytes.len().saturating_add(data.length() as usize) > MAX_RESPONSE {
            let _ = reader.cancel();
            return Err(failed("geocoder response too large"));
        }
        bytes.extend(data.to_vec());
    }
    let text = String::from_utf8(bytes).map_err(|_| failed("geocoder UTF-8"))?;
    Ok(GeocodeOutput {
        provider: a.provider,
        results: p.parse(&text)?,
        cached: false,
    })
}
/// Run platform IO away from the desktop frame loop, through the same registered commands.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_navigation_request(request: crate::AppRequest) -> Result<serde_json::Value, CommandError> {
    let mut session = Session::new()?;
    if let Some(dir) = crate::navigation_data_directory() {
        session.load_navigation_storage(&dir)?;
    }
    match request {
        crate::AppRequest::Geocode(args) => {
            let output = session.run(args)?;
            serde_json::to_value(output).map_err(|e| failed(e.to_string()))
        }
        crate::AppRequest::Location => {
            let output = session.run(crate::CurrentLocation {
                request_permission: true,
            })?;
            serde_json::to_value(output).map_err(|e| failed(e.to_string()))
        }
        _ => Err(failed("not a navigation IO request")),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    #[test]
    fn handwritten_provider_responses() {
        let c=CensusGeocoder.parse(r#"{"result":{"addressMatches":[{"matchedAddress":"100 Example St","coordinates":{"x":-75.0,"y":41.0}}]}}"#).unwrap();
        assert_eq!(c[0].position.latitude_deg, 41.0);
        let n = NominatimGeocoder
            .parse(r#"[{"lat":"41.0","lon":"-75.0","display_name":"Example place"}]"#)
            .unwrap();
        assert_eq!(n[0].attribution, "© OpenStreetMap contributors");
        assert!(
            NominatimGeocoder
                .parse(r#"[{"lat":"NaN","lon":"0","display_name":"Bad"}]"#)
                .is_err()
        );
        assert!(CensusGeocoder.url("a&b").contains("a%26b"));
    }
    #[test]
    fn cache_reuses_attributed_results_without_network_and_still_requires_consent() {
        let mut s = Session::new().unwrap();
        let a = GeocodeLookup {
            address: "Example".into(),
            provider: GeocodeProvider::Nominatim,
            allow_network: true,
        };
        let output = GeocodeOutput {
            provider: a.provider,
            results: NominatimGeocoder
                .parse(r#"[{"lat":"41","lon":"-75","display_name":"Example"}]"#)
                .unwrap(),
            cached: false,
        };
        s.cache_geocode_result(&a, output).unwrap();
        let found = s.run(a.clone()).unwrap();
        assert!(found.cached);
        assert_eq!(found.results[0].attribution, "© OpenStreetMap contributors");
        assert!(
            s.run(GeocodeLookup {
                allow_network: false,
                ..a
            })
            .is_err()
        );
    }
    #[test]
    fn denied_network_does_not_send() {
        let mut s = Session::new().unwrap();
        assert!(
            s.execute(
                "geocode.lookup",
                serde_json::json!({"address":"Example","provider":"census"})
            )
            .is_err()
        );
    }
}
