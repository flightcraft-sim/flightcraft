//! User-authored source descriptions and bounded text extraction, without executing page code.
use super::metar::{self, Observation};
use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_BODY: usize = 1024 * 1024;
pub const DISCLAIMER: &str = "Advisory only. Not for flight planning.";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = source_schema)]
pub struct WeatherSource {
    #[schemars(range(min = 1, max = 1))]
    pub format_version: u32,
    #[schemars(length(min = 1, max = 64), regex(pattern = "^[a-z0-9][a-z0-9_-]*$"))]
    pub id: String,
    #[schemars(length(min = 1, max = 128))]
    pub name: String,
    #[schemars(length(min = 3, max = 8), regex(pattern = "^[A-Z0-9]+$"))]
    pub airport: String,
    #[schemars(length(min = 1, max = 2048), regex(pattern = "^https?://"))]
    pub url: String,
    pub allow_http: bool,
    #[schemars(range(min = 60, max = 86400))]
    pub refresh_seconds: u32,
    #[schemars(range(min = 1, max = 120))]
    pub timeout_seconds: u32,
    pub advisory_only: bool,
    #[schemars(length(max = 1024))]
    pub disclaimer: String,
    /// Browser opt-in; the server must also send Access-Control-Allow-Origin.
    #[serde(default)]
    pub cors: bool,
    pub extraction: Extraction,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Extraction {
    Digiwx,
    MetarText,
    Aviationweather,
    Metar {
        selector: Selector,
        #[serde(default)]
        fields: Fields,
    },
    Fields {
        fields: Fields,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    ElementId {
        element_id: String,
    },
    /// #id, .class, #id.class, or .class.other; no combinators or executable expressions.
    CssLike {
        css_like: String,
    },
    Regex {
        regex: String,
    },
    Body,
    JsonPointer {
        json_pointer: String,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Fields {
    pub wind: Option<Selector>,
    pub gust: Option<Selector>,
    pub altimeter: Option<Selector>,
    pub visibility: Option<Selector>,
    pub layers: Option<Selector>,
    pub ceiling: Option<Selector>,
    pub temperature: Option<Selector>,
    pub dew_point: Option<Selector>,
    pub runway_in_use: Option<Selector>,
    pub date: Option<Selector>,
    pub time: Option<Selector>,
}
impl Fields {
    fn selectors(&self) -> Vec<(&str, &Option<Selector>)> {
        vec![
            ("wind", &self.wind),
            ("gust", &self.gust),
            ("altimeter", &self.altimeter),
            ("visibility", &self.visibility),
            ("layers", &self.layers),
            ("ceiling", &self.ceiling),
            ("temperature", &self.temperature),
            ("dew_point", &self.dew_point),
            ("runway_in_use", &self.runway_in_use),
            ("date", &self.date),
            ("time", &self.time),
        ]
    }
    pub fn digiwx() -> Self {
        let id = |v: &str| Some(Selector::ElementId { element_id: v.into() });
        Self {
            wind: id("lblWind"),
            gust: id("lblGusts"),
            altimeter: id("lblAltimeter"),
            visibility: id("lblVIZ"),
            layers: id("lblCeilometer"),
            temperature: id("lblTemperature"),
            dew_point: id("lblDewPoint"),
            runway_in_use: id("lblIndicatedRunway"),
            date: id("lblDate"),
            time: id("lblTime"),
            ceiling: None,
        }
    }
}
fn error(message: impl Into<String>) -> String {
    message.into()
}
fn bounded_regex(pattern: &str) -> Result<Regex, String> {
    if pattern.len() > 2048 {
        return Err(error("Regex exceeds 2,048 bytes"));
    }
    RegexBuilder::new(pattern)
        .size_limit(256 * 1024)
        .dfa_size_limit(512 * 1024)
        .nest_limit(32)
        .build()
        .map_err(|e| format!("Invalid or oversized regex: {e}"))
}
fn valid_atom(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 128
        && v.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b':'))
}
impl Selector {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Regex { regex } => {
                if bounded_regex(regex)?.captures_len() != 2 {
                    return Err(error("Regex must contain exactly one capture group"));
                }
            }
            Self::ElementId { element_id } if !valid_atom(element_id) => {
                return Err(error("Invalid element id"));
            }
            Self::CssLike { css_like } => {
                css_parts(css_like)?;
            }
            Self::JsonPointer { json_pointer }
                if json_pointer.len() > 1024
                    || !json_pointer.is_empty() && !json_pointer.starts_with('/') =>
            {
                return Err(error("Invalid JSON pointer"));
            }
            _ => {}
        }
        Ok(())
    }
    pub fn extract(&self, body: &str) -> Result<Option<String>, String> {
        self.validate()?;
        if body.len() > MAX_BODY {
            return Err(error("Weather response exceeds 1 MB"));
        }
        let result = match self {
            Self::Body => Some(body.trim().into()),
            Self::Regex { regex } => bounded_regex(regex)?
                .captures(body)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().trim().into()),
            Self::JsonPointer { json_pointer } => {
                let json: serde_json::Value =
                    serde_json::from_str(body).map_err(|_| error("Response is not valid JSON"))?;
                json.pointer(json_pointer).and_then(|v| {
                    v.as_str()
                        .map(String::from)
                        .or_else(|| v.as_f64().map(|n| n.to_string()))
                })
            }
            Self::ElementId { element_id } => html_match(body, Some(element_id), &[])?,
            Self::CssLike { css_like } => {
                let (id, classes) = css_parts(css_like)?;
                html_match(body, id, &classes)?
            }
        };
        if result.as_ref().is_some_and(|v| v.len() > 16 * 1024) {
            return Err(error("Matched weather text exceeds 16 KiB"));
        }
        Ok(result)
    }
}
fn css_parts(v: &str) -> Result<(Option<&str>, Vec<&str>), String> {
    let mut parts = v.split('.');
    let first = parts.next().unwrap_or("");
    let id = first.strip_prefix('#');
    let classes = parts.collect::<Vec<_>>();
    if v.len() > 512
        || !(first.is_empty() || id.is_some_and(valid_atom))
        || classes.iter().any(|v| !valid_atom(v))
        || id.is_none() && classes.is_empty()
    {
        return Err(error("Use only #id and .class selectors"));
    }
    Ok((id, classes))
}
fn html_match(body: &str, id: Option<&str>, classes: &[&str]) -> Result<Option<String>, String> {
    // Tokenize tags, then balance the selected element. Never run scripts, load resources, or
    // parse the ASP.NET form state. Attribute quoting and nested inline tags are supported.
    let tags = bounded_regex(r#"(?s)<!--.*?-->|<(?:[^'">]|"[^"]*"|'[^']*')*>"#)?;
    let attrs = bounded_regex(r#"([\w:-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#)?;
    let tag_name = |tag: &str| {
        tag.trim_start_matches('<')
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let mut selected: Option<(String, usize, usize)> = None;
    for tag in tags.find_iter(body) {
        let t = tag.as_str();
        if t.starts_with("<!") || t.starts_with("<?") {
            continue;
        }
        let name = tag_name(t);
        let close = t.starts_with("</");
        if let Some((wanted, start, depth)) = selected.as_mut() {
            if &name == wanted {
                if close {
                    *depth = depth.saturating_sub(1);
                } else if !t.ends_with("/>") {
                    *depth += 1;
                }
                if *depth == 0 {
                    let content = body.get(*start..tag.start()).unwrap_or("");
                    if content.len() > 16 * 1024 {
                        return Err("Matched weather text exceeds 16 KiB".into());
                    }
                    return Ok(Some(decode_entities(&tags.replace_all(content, " "))));
                }
            }
            continue;
        }
        if close || matches!(name.as_str(), "script" | "style") {
            continue;
        }
        let mut found_id = None;
        let mut found_classes = None;
        for a in attrs.captures_iter(t) {
            let key = a.get(1).map(|v| v.as_str()).unwrap_or("");
            let value = a
                .get(2)
                .or_else(|| a.get(3))
                .or_else(|| a.get(4))
                .map(|v| v.as_str());
            if key.eq_ignore_ascii_case("id") {
                found_id = value;
            }
            if key.eq_ignore_ascii_case("class") {
                found_classes = value;
            }
        }
        if id.is_none_or(|v| found_id == Some(v))
            && classes
                .iter()
                .all(|v| found_classes.is_some_and(|c| c.split_whitespace().any(|s| s == *v)))
        {
            selected = Some((name, tag.end(), 1));
        }
    }
    Ok(None)
}
fn decode_entities(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('&') {
        out.push_str(before);
        if let Some(end) = after
            .char_indices()
            .take(13)
            .find_map(|(i, c)| (c == ';').then_some(i))
        {
            let entity = after.get(..end).unwrap_or("");
            let tail = after.get(end + 1..).unwrap_or("");
            let decoded = match entity {
                "nbsp" => Some(' '),
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "deg" => Some('°'),
                v => v
                    .strip_prefix("#x")
                    .and_then(|s| u32::from_str_radix(s, 16).ok())
                    .or_else(|| v.strip_prefix('#').and_then(|s| s.parse().ok()))
                    .and_then(char::from_u32),
            };
            if let Some(c) = decoded {
                out.push(c);
            } else {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
            rest = tail;
        } else {
            out.push('&');
            rest = after;
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
impl WeatherSource {
    pub fn preset(airport: &str, url: &str, kind: Extraction) -> Self {
        Self {
            format_version: 1,
            id: format!("{}-local", airport.to_ascii_lowercase()),
            name: format!("{airport} local weather"),
            airport: airport.into(),
            url: url.into(),
            allow_http: url.starts_with("http://"),
            refresh_seconds: 60,
            timeout_seconds: 15,
            advisory_only: true,
            disclaimer: DISCLAIMER.into(),
            cors: false,
            extraction: kind,
        }
    }
    pub fn aviationweather(icao: &str) -> Self {
        let mut s = Self::preset(
            icao,
            &format!("https://aviationweather.gov/api/data/metar?ids={icao}&format=json"),
            Extraction::Aviationweather,
        );
        s.id = format!("{}-aviationweather", icao.to_ascii_lowercase());
        s.name = format!("{icao} · Aviation Weather Center");
        s.cors = true;
        s.advisory_only = false;
        s.disclaimer.clear();
        s
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != 1
            || !valid_atom(&self.id)
            || self.id.len() > 64
            || !self.id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
            || self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
            || !(3..=8).contains(&self.airport.len())
            || !self
                .airport
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            || !(60..=86400).contains(&self.refresh_seconds)
            || !(1..=120).contains(&self.timeout_seconds)
            || self.disclaimer.len() > 1024
            || self
                .disclaimer
                .chars()
                .any(|c| c.is_control() && !c.is_whitespace())
            || self.advisory_only && self.disclaimer.trim().is_empty()
            || matches!(self.extraction, Extraction::Digiwx) && !self.advisory_only
        {
            return Err(error(
                "Invalid weather source version, identity, refresh, timeout, or advisory disclaimer",
            ));
        }
        let url = url::Url::parse(&self.url).map_err(|_| error("Enter a complete HTTP or HTTPS URL"))?;
        if self.url.len() > 2048
            || !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.scheme() == "http" && !self.allow_http
        {
            return Err(error(
                "HTTP needs allow_http; URLs must have a host and no credentials or fragment",
            ));
        }
        if let Extraction::Metar { selector, .. } = &self.extraction {
            selector.validate()?;
        }
        let fields = match &self.extraction {
            Extraction::Fields { fields } | Extraction::Metar { fields, .. } => Some(fields),
            _ => None,
        };
        if let Some(f) = fields {
            for (_, v) in f.selectors() {
                if let Some(v) = v {
                    v.validate()?;
                }
            }
        }
        Ok(())
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        if text.len() > 64 * 1024 {
            return Err(error("Source file exceeds 64 KiB"));
        }
        let source: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        source.validate()?;
        Ok(source)
    }
    pub fn extract(&self, bytes: &[u8]) -> Result<Extracted, String> {
        self.validate()?;
        if bytes.len() > MAX_BODY {
            return Err(error("Weather response exceeds 1 MB"));
        }
        let body =
            std::str::from_utf8(bytes).map_err(|_| error("Weather response is binary or invalid UTF-8"))?;
        if body
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(error("Weather response contains binary control characters"));
        }
        let (selector, fields) = match &self.extraction {
            Extraction::Digiwx => (
                Some(Selector::ElementId {
                    element_id: "MetarLine".into(),
                }),
                Fields::digiwx(),
            ),
            Extraction::MetarText => (Some(Selector::Body), Fields::default()),
            Extraction::Aviationweather => (
                Some(Selector::JsonPointer {
                    json_pointer: "/0/rawOb".into(),
                }),
                Fields::default(),
            ),
            Extraction::Metar { selector, fields } => (Some(selector.clone()), fields.clone()),
            Extraction::Fields { fields } => (None, fields.clone()),
        };
        let mut matches = BTreeMap::new();
        let mut o = Observation::default();
        if let Some(selector) = selector
            && let Some(raw) = selector.extract(body)?
        {
            o = metar::parse(&raw);
            matches.insert("metar".into(), raw);
        }
        for (key, selector) in fields.selectors() {
            if let Some(selector) = selector
                && let Some(value) = selector.extract(body)?
            {
                matches.insert(key.into(), value);
            }
        }
        supplement(&mut o, &matches);
        o.validate()?;
        if o.station.as_deref().is_some_and(|s| s != self.airport) {
            return Err(error("Observation station does not match this source's airport"));
        }
        if !o.usable() {
            if matches!(self.extraction, Extraction::Aviationweather) {
                return Err(
                    "No official METAR is available for this airport. Find nearby reporting stations.".into(),
                );
            }
            return Err(error(
                "No usable weather fields matched; check the endpoint and selectors",
            ));
        }
        Ok(Extracted {
            observation: o,
            matches,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Extracted {
    /// Exact selected text, before field parsing, for the Test view.
    pub matches: BTreeMap<String, String>,
    pub observation: Observation,
}
fn first_number(text: &str) -> Option<f64> {
    text.split(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-')))
        .find_map(|s| s.parse::<f64>().ok().filter(|v| v.is_finite()))
}
fn supplement(o: &mut Observation, m: &BTreeMap<String, String>) {
    if o.wind.is_none()
        && let Some(v) = m.get("wind")
    {
        let upper = v.to_ascii_uppercase();
        o.wind = metar::wind(upper.trim());
        if o.wind.is_none()
            && let Some((a, b)) = upper.split_once('@')
        {
            let variable = a.contains("VRB") || a.contains("VARIABLE");
            let speed = first_number(b).map(|n| {
                if b.contains("M/S") || b.contains("MPS") {
                    n * 1.94384449
                } else {
                    n
                }
            });
            if let Some(speed) = speed.filter(|v| (0. ..=145.).contains(v)) {
                o.wind = Some(metar::Wind {
                    from_true_deg: if variable {
                        None
                    } else {
                        first_number(a).filter(|v| (0. ..=360.).contains(v))
                    },
                    variable,
                    speed_kt: speed,
                    ..Default::default()
                });
            }
        }
    }
    if let Some(w) = &mut o.wind
        && w.gust_kt.is_none()
    {
        w.gust_kt = m
            .get("gust")
            .and_then(|s| {
                first_number(s).map(|v| {
                    if s.to_ascii_uppercase().contains("M/S") || s.to_ascii_uppercase().contains("MPS") {
                        v * 1.94384449
                    } else {
                        v
                    }
                })
            })
            .filter(|v| (0. ..=200.).contains(v));
    }
    if o.altimeter_inhg.is_none()
        && let Some(s) = m.get("altimeter")
    {
        o.altimeter_inhg = first_number(s)
            .map(|v| {
                if s.trim().starts_with('A') && s.trim().len() == 5 {
                    v / 100.
                } else if v > 100. {
                    v / 33.86389
                } else {
                    v
                }
            })
            .filter(|v| (23.63..=32.48).contains(v));
    }
    if o.visibility_sm.is_none()
        && let Some(s) = m.get("visibility")
    {
        let s = s.to_ascii_uppercase();
        o.visibility_sm = if s.contains("KM") {
            first_number(&s).map(|v| v / 1.609344)
        } else if s.trim().ends_with('M') && !s.ends_with("SM") {
            first_number(&s).map(|v| v / 1609.344)
        } else {
            metar::visibility(&s)
        }
        .filter(|v| (0. ..=100.).contains(v));
    }
    if o.layers.is_none()
        && let Some(s) = m.get("layers")
    {
        let upper = s.to_ascii_uppercase();
        if matches!(upper.trim(), "CLR" | "SKC" | "NSC" | "NCD") {
            o.layers = Some(Vec::new());
        } else {
            let layers = upper
                .split_whitespace()
                .filter_map(metar::layer)
                .collect::<Vec<_>>();
            if !layers.is_empty() {
                o.layers = Some(layers);
            }
        }
    }
    if o.layers.is_none()
        && let Some(s) = m.get("ceiling").or_else(|| m.get("layers"))
        && let Some(height) = first_number(s)
            .map(|v| {
                if s.to_ascii_uppercase().contains("MET") || s.trim().to_ascii_uppercase().ends_with('M') {
                    v / 0.3048
                } else {
                    v
                }
            })
            .filter(|v| (0. ..=60000.).contains(v))
    {
        let upper = s.to_ascii_uppercase();
        let coverage = if upper.contains("BROKEN") || upper.contains("BKN") {
            "BKN"
        } else if upper.contains("SCATTERED") || upper.contains("SCT") {
            "SCT"
        } else if upper.contains("FEW") {
            "FEW"
        } else {
            "OVC"
        };
        o.layers = Some(vec![metar::Layer {
            coverage: coverage.into(),
            base_agl_ft: Some(height),
        }]);
    }
    let temp = |key| {
        m.get(key)
            .and_then(|s| {
                let upper = s.to_ascii_uppercase();
                let number = upper
                    .split_whitespace()
                    .next()
                    .filter(|v| v.starts_with('M'))
                    .and_then(metar::temperature)
                    .or_else(|| first_number(s));
                number.map(|v| {
                    if s.to_ascii_uppercase().contains('F') && !s.to_ascii_uppercase().contains('C') {
                        (v - 32.) * 5. / 9.
                    } else {
                        v
                    }
                })
            })
            .filter(|v| (-100. ..=100.).contains(v))
    };
    if o.temperature_c.is_none() {
        o.temperature_c = temp("temperature");
    }
    if o.dewpoint_c.is_none() {
        o.dewpoint_c = temp("dew_point");
    }
    o.runway_in_use = m
        .get("runway_in_use")
        .and_then(|v| v.split_whitespace().last())
        .filter(|v| {
            v.len() <= 3
                && v.chars().any(|c| c.is_ascii_digit())
                && v.chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, 'L' | 'R' | 'C'))
        })
        .map(String::from);
}

fn source_schema(schema: &mut schemars::Schema) {
    schema.insert("allOf".into(),serde_json::json!([
        {"if":{"properties":{"url":{"pattern":"^http://"}},"required":["url"]},"then":{"properties":{"allow_http":{"const":true}}}},
        {"if":{"properties":{"advisory_only":{"const":true}},"required":["advisory_only"]},"then":{"properties":{"disclaimer":{"minLength":1,"pattern":"\\S"}}}},
        {"if":{"properties":{"extraction":{"properties":{"kind":{"const":"digiwx"}},"required":["kind"]}},"required":["extraction"]},"then":{"properties":{"advisory_only":{"const":true}}}}
    ]));
}
