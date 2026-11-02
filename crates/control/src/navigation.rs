//! Offline search, opt-in actions, simulation date, pins, and navigation commands.
use crate::{Command, CommandError, Session};
use flightcraft_world::{
    GeoPoint,
    geomag::{MagneticModel, SimulationDate},
    navdata::text_ok,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn failed(s: impl Into<String>) -> CommandError {
    CommandError::Failed(s.into())
}
pub(crate) fn valid_point(p: GeoPoint) -> Result<(), CommandError> {
    p.validate().map_err(|e| failed(e.to_string()))
}
#[derive(Debug, Default)]
pub(crate) struct NavigationState {
    pub date: SimulationDate,
    pub pins: PinsFile,
    pub plan: crate::planning::FlightPlan,
    pub active_plan: Option<crate::planning::FlightPlan>,
    pub geocode: crate::geocoding::GeocodeState,
    #[cfg(not(target_arch = "wasm32"))]
    pub storage: Option<std::path::PathBuf>,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSimulationDate {
    pub date: SimulationDate,
}
impl Command for SetSimulationDate {
    const NAME: &'static str = "sim.set_date";
    const DESCRIPTION: &'static str =
        "Set the deterministic WMM simulation date (2025–2029); default 2026-01-01.";
    type Output = SimulationDate;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        self.date.decimal_year().map_err(|e| failed(e.to_string()))?;
        s.navigation.date = self.date;
        Ok(self.date)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    #[schemars(length(min = 1, max = 64))]
    pub id: String,
    #[schemars(length(min = 1, max = 256))]
    pub name: String,
    #[schemars(length(max = 4096))]
    pub note: String,
    pub position: GeoPoint,
    #[schemars(range(min=-500,max=20000))]
    pub altitude_m: Option<f64>,
}
impl Pin {
    fn validate(&self) -> Result<(), CommandError> {
        valid_point(self.position)?;
        if !text_ok(&self.id, 64)
            || !text_ok(&self.name, 256)
            || self.note.len() > 4096
            || self.note.contains('\0')
            || self
                .altitude_m
                .is_some_and(|a| !a.is_finite() || !(-500.0..=20000.0).contains(&a))
        {
            return Err(failed("invalid pin fields"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinsFile {
    #[schemars(range(min = 1, max = 1))]
    pub version: u8,
    #[schemars(length(max = 1000))]
    pub pins: Vec<Pin>,
}
impl Default for PinsFile {
    fn default() -> Self {
        Self {
            version: 1,
            pins: Vec::new(),
        }
    }
}
impl PinsFile {
    pub fn from_json(text: &str) -> Result<Self, CommandError> {
        if text.len() > 1024 * 1024 {
            return Err(failed("pins exceed 1 MiB"));
        }
        let p: Self = serde_json::from_str(text).map_err(|e| failed(e.to_string()))?;
        p.validate()?;
        Ok(p)
    }
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.version != 1 || self.pins.len() > 1000 {
            return Err(failed("invalid pins version/count"));
        }
        let mut ids = std::collections::BTreeSet::new();
        for p in &self.pins {
            p.validate()?;
            if !ids.insert(&p.id) {
                return Err(failed("duplicate pin id"));
            }
        }
        Ok(())
    }
}
impl Session {
    /// Hosts configure a local data directory. No device location is ever persisted automatically.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_navigation_storage(&mut self, directory: &std::path::Path) -> Result<(), CommandError> {
        let path = directory.join("pins.json");
        let pins = match std::fs::File::open(&path) {
            Ok(f) => {
                use std::io::Read;
                let mut text = String::new();
                f.take(1024 * 1024 + 1)
                    .read_to_string(&mut text)
                    .map_err(|e| failed(e.to_string()))?;
                PinsFile::from_json(&text)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PinsFile::default(),
            Err(e) => return Err(failed(e.to_string())),
        };
        self.navigation.pins = pins;
        self.navigation.storage = Some(directory.to_owned());
        Ok(())
    }
    pub fn simulation_date(&self) -> SimulationDate {
        self.navigation.date
    }
    fn replace_pins(&mut self, pins: PinsFile) -> Result<PinsFile, CommandError> {
        pins.validate()?;
        let text = serde_json::to_string_pretty(&pins).map_err(|e| failed(e.to_string()))?;
        if text.len() > 1024 * 1024 {
            return Err(failed("pins exceed 1 MiB"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(dir) = &self.navigation.storage {
            std::fs::create_dir_all(dir).map_err(|e| failed(e.to_string()))?;
            let tmp = dir.join("pins.json.tmp");
            std::fs::write(&tmp, &text)
                .and_then(|()| std::fs::rename(&tmp, dir.join("pins.json")))
                .map_err(|e| failed(e.to_string()))?;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let storage = web_sys::window()
                .ok_or_else(|| failed("window unavailable"))?
                .local_storage()
                .map_err(|_| failed("browser storage denied"))?
                .ok_or_else(|| failed("browser storage unavailable"))?;
            storage
                .set_item("flightcraft.pins.v1", &text)
                .map_err(|_| failed("browser storage denied or full"))?;
        }
        self.navigation.pins = pins.clone();
        Ok(pins)
    }
    #[cfg(target_arch = "wasm32")]
    pub fn load_browser_pins(&mut self) -> Result<(), CommandError> {
        let storage = web_sys::window()
            .ok_or_else(|| failed("window unavailable"))?
            .local_storage()
            .map_err(|_| failed("browser storage denied"))?;
        if let Some(storage) = storage
            && let Some(text) = storage
                .get_item("flightcraft.pins.v1")
                .map_err(|_| failed("browser storage denied"))?
        {
            self.navigation.pins = PinsFile::from_json(&text)?;
        }
        Ok(())
    }
}
/// Local app data, resolved by the host; no path is embedded in assets.
#[cfg(not(target_arch = "wasm32"))]
pub fn navigation_data_directory() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("XDG_DATA_HOME") {
        return Some(std::path::PathBuf::from(p).join("flightcraft"));
    }
    #[cfg(target_os = "windows")]
    if let Some(p) = std::env::var_os("LOCALAPPDATA") {
        return Some(std::path::PathBuf::from(p).join("FlightCraft"));
    }
    std::env::var_os("HOME").map(|p| {
        let p = std::path::PathBuf::from(p);
        if cfg!(target_os = "macos") {
            p.join("Library/Application Support/FlightCraft")
        } else {
            p.join(".local/share/flightcraft")
        }
    })
}
macro_rules! pins_command {
    ($ty:ident,$name:literal,$desc:literal,$body:expr) => {
        impl Command for $ty {
            const NAME: &'static str = $name;
            const DESCRIPTION: &'static str = $desc;
            type Output = PinsFile;
            fn run(self, s: &mut Session) -> Result<PinsFile, CommandError> {
                ($body)(self, s)
            }
        }
    };
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinsList {}
pins_command!(
    PinsList,
    "pins.list",
    "List locally saved user pins.",
    |_: PinsList, s: &mut Session| Ok(s.navigation.pins.clone())
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinsAdd {
    pub pin: Pin,
}
pins_command!(
    PinsAdd,
    "pins.add",
    "Add a unique user pin and persist locally.",
    |a: PinsAdd, s: &mut Session| {
        let mut p = s.navigation.pins.clone();
        p.pins.push(a.pin);
        s.replace_pins(p)
    }
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinsUpdate {
    pub pin: Pin,
}
pins_command!(
    PinsUpdate,
    "pins.update",
    "Replace an existing pin, including its name, note, or altitude.",
    |a: PinsUpdate, s: &mut Session| {
        let mut p = s.navigation.pins.clone();
        let v = p
            .pins
            .iter_mut()
            .find(|p| p.id == a.pin.id)
            .ok_or_else(|| failed("unknown pin"))?;
        *v = a.pin;
        s.replace_pins(p)
    }
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinsRemove {
    pub id: String,
}
pins_command!(
    PinsRemove,
    "pins.remove",
    "Remove a saved user pin.",
    |a: PinsRemove, s: &mut Session| {
        let mut p = s.navigation.pins.clone();
        let before = p.pins.len();
        p.pins.retain(|p| p.id != a.id);
        if before == p.pins.len() {
            return Err(failed("unknown pin"));
        }
        s.replace_pins(p)
    }
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinsImport {
    pub document: PinsFile,
    #[serde(default)]
    pub replace: bool,
}
pins_command!(
    PinsImport,
    "pins.import",
    "Validate and import a pins document, merging unless replace is true.",
    |a: PinsImport, s: &mut Session| {
        a.document.validate()?;
        let mut p = if a.replace {
            PinsFile::default()
        } else {
            s.navigation.pins.clone()
        };
        p.pins.extend(a.document.pins);
        s.replace_pins(p)
    }
);
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinsExport {}
pins_command!(
    PinsExport,
    "pins.export",
    "Export the complete pins document.",
    |_: PinsExport, s: &mut Session| Ok(s.navigation.pins.clone())
);

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Airport,
    Navaid,
    Fix,
    Airway,
    Pin,
    Coordinate,
    Radial,
    LocationAction,
    GeocodeAction,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchResult {
    pub kind: SearchKind,
    pub ident: String,
    pub name: String,
    pub position: Option<GeoPoint>,
    pub distance_m: Option<f64>,
    pub details: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchOutput {
    pub results: Vec<SearchResult>,
    pub effective_date: Option<String>,
    pub snapshot_date: Option<String>,
    pub notice: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchQuery {
    pub text: String,
    #[serde(default)]
    pub reference: Option<GeoPoint>,
    #[serde(default = "search_limit")]
    pub limit: usize,
}
fn search_limit() -> usize {
    20
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NearestNavdata {
    pub position: GeoPoint,
    #[serde(default = "search_limit")]
    pub limit: usize,
}
impl Command for NearestNavdata {
    const NAME: &'static str = "navdata.nearest";
    const DESCRIPTION: &'static str = "Return nearest airports, navaids, and fixes in the active region.";
    type Output = SearchOutput;
    fn run(self, s: &mut Session) -> Result<SearchOutput, CommandError> {
        valid_point(self.position)?;
        if self.limit == 0 || self.limit > 100 {
            return Err(failed("limit must be 1–100"));
        }
        let mut rows = local_results(s);
        rows.retain(|r| matches!(r.kind, SearchKind::Airport | SearchKind::Navaid | SearchKind::Fix));
        for r in &mut rows {
            r.distance_m = r.position.map(|p| p.inverse(self.position).0);
        }
        rows.sort_by(|a, b| {
            a.distance_m
                .unwrap_or(f64::MAX)
                .total_cmp(&b.distance_m.unwrap_or(f64::MAX))
                .then(a.ident.cmp(&b.ident))
        });
        rows.truncate(self.limit);
        Ok(output(s, rows))
    }
}
fn output(s: &Session, results: Vec<SearchResult>) -> SearchOutput {
    let n = s.sim().world().navdata();
    SearchOutput {
        results,
        effective_date: n.and_then(|n| n.effective_date.clone()),
        snapshot_date: n.map(|n| n.snapshot_date.clone()),
        notice: "Not for real-world navigation; dataset cycle may be unknown".into(),
    }
}
fn row(kind: SearchKind, ident: &str, name: &str, p: GeoPoint, details: String) -> SearchResult {
    SearchResult {
        kind,
        ident: ident.into(),
        name: name.into(),
        position: Some(p),
        distance_m: None,
        details,
    }
}
fn local_results(s: &Session) -> Vec<SearchResult> {
    let mut rows = Vec::new();
    let world = s.sim().world();
    if let Some(a) = world.airports() {
        for a in &a.airports {
            rows.push(row(
                SearchKind::Airport,
                &a.ident,
                &a.name,
                GeoPoint {
                    latitude_deg: a.latitude_deg,
                    longitude_deg: a.longitude_deg,
                },
                a.icao.clone().unwrap_or_default(),
            ));
        }
    }
    if let Some(n) = world.navdata() {
        for n in &n.navaids {
            let frequencies = n
                .components
                .iter()
                .filter_map(|c| c.frequency_hz.map(|f| format!("{} Hz", f)))
                .collect::<Vec<_>>()
                .join(", ");
            let channels = n
                .components
                .iter()
                .filter_map(|c| c.channel.clone())
                .collect::<Vec<_>>()
                .join(", ");
            rows.push(row(
                SearchKind::Navaid,
                &n.ident,
                &n.name,
                n.position,
                format!("{:?} · {frequencies} · {channels}", n.kind),
            ));
        }
        for f in &n.fixes {
            rows.push(row(SearchKind::Fix, &f.ident, &f.name, f.position, String::new()));
        }
        let mut airways = std::collections::BTreeSet::new();
        for r in &n.routes {
            if airways.insert(&r.ident)
                && let Some(p) = r.geometry.first()
            {
                rows.push(row(
                    SearchKind::Airway,
                    &r.ident,
                    &r.ident,
                    *p,
                    format!(
                        "{} – {}",
                        r.start_ident.as_deref().unwrap_or(&r.start_id),
                        r.end_ident.as_deref().unwrap_or(&r.end_id)
                    ),
                ));
            }
        }
    }
    for p in &s.navigation.pins.pins {
        rows.push(row(SearchKind::Pin, &p.id, &p.name, p.position, p.note.clone()));
    }
    rows
}
impl Command for SearchQuery {
    const NAME: &'static str = "search.query";
    const DESCRIPTION: &'static str = "Search local airports, navaids, fixes, airways, pins, coordinates, and magnetic radials; return network actions without running them.";
    type Output = SearchOutput;
    fn run(self, s: &mut Session) -> Result<SearchOutput, CommandError> {
        if self.text.len() > 512
            || self.text.chars().any(char::is_control)
            || !(1..=100).contains(&self.limit)
        {
            return Err(failed("query exceeds bounds"));
        }
        if let Some(p) = self.reference {
            valid_point(p)?;
        }
        let q = self.text.trim().to_uppercase();
        if q.is_empty() {
            return Ok(output(s, Vec::new()));
        }
        let reference = self.reference.or_else(|| {
            let r = s.sim().readout();
            Some(GeoPoint {
                latitude_deg: r.latitude_deg?,
                longitude_deg: r.longitude_deg?,
            })
        });
        let mut rows = local_results(s);
        let rank = |r: &SearchResult| {
            let ident = r.ident.to_uppercase();
            let name = r.name.to_uppercase();
            if ident == q || matches!(r.kind, SearchKind::Airport) && r.details.to_uppercase() == q {
                0
            } else if ident.starts_with(&q)
                || matches!(r.kind, SearchKind::Airport) && r.details.to_uppercase().starts_with(&q)
            {
                1
            } else if name.contains(&q) {
                2
            } else {
                3
            }
        };
        rows.retain(|r| rank(r) < 3);
        if let Some(p) = parse_coordinates(&self.text)? {
            rows.push(row(
                SearchKind::Coordinate,
                &self.text,
                "Coordinates",
                p,
                String::new(),
            ));
        }
        if let Some((ident, radial, distance)) = parse_radial(&q)?
            && let Some(n) = s
                .sim()
                .world()
                .navdata()
                .and_then(|d| d.navaids.iter().find(|n| n.ident == ident))
        {
            let variation = MagneticModel::wmm2025()
                .and_then(|m| {
                    m.declination(
                        n.position.latitude_deg,
                        n.position.longitude_deg,
                        s.navigation.date,
                    )
                })
                .map_err(|e| failed(e.to_string()))?;
            rows.push(row(
                SearchKind::Radial,
                &q,
                &format!("{ident} {radial}° magnetic / {distance} nm"),
                n.position.destination(radial + variation, distance * 1852.0),
                format!("WMM declination {variation:.2}°"),
            ));
        }
        for r in &mut rows {
            r.distance_m = r.position.and_then(|p| reference.map(|o| o.inverse(p).0));
        }
        rows.sort_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then(
                    a.distance_m
                        .unwrap_or(f64::MAX)
                        .total_cmp(&b.distance_m.unwrap_or(f64::MAX)),
                )
                .then(a.ident.cmp(&b.ident))
        });
        if rows.is_empty() {
            rows.push(SearchResult {
                kind: if q == "MY LOCATION" {
                    SearchKind::LocationAction
                } else {
                    SearchKind::GeocodeAction
                },
                ident: String::new(),
                name: self.text,
                position: None,
                distance_m: None,
                details: if q == "MY LOCATION" {
                    "Choose My location to request permission; position is not saved"
                } else {
                    "Confirm a Census or Nominatim lookup before sending this address"
                }
                .into(),
            });
        }
        rows.truncate(self.limit);
        Ok(output(s, rows))
    }
}
/// Accept decimal, DDM, and DMS, with hemisphere prefixes/suffixes or signed decimal pairs.
pub fn parse_coordinates(text: &str) -> Result<Option<GeoPoint>, CommandError> {
    if text.len() > 512 {
        return Err(failed("coordinate input too long"));
    }
    if !text.chars().any(|c| c.is_ascii_digit()) {
        return Ok(None);
    }
    let mut tokens = Vec::<String>::new();
    let mut num = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() || matches!(c, '.' | '+' | '-') {
            num.push(c);
        } else {
            if !num.is_empty() {
                tokens.push(std::mem::take(&mut num));
            }
            if matches!(c.to_ascii_uppercase(), 'N' | 'S' | 'E' | 'W') {
                tokens.push(c.to_ascii_uppercase().to_string());
            } else if !c.is_whitespace() && !matches!(c, ',' | ';' | '°' | '\'' | '"' | '′' | '″') {
                return Ok(None);
            }
        }
    }
    if !num.is_empty() {
        tokens.push(num);
    }
    let hemi = |s: &str| matches!(s, "N" | "S" | "E" | "W");
    let hemis: Vec<_> = tokens
        .iter()
        .enumerate()
        .filter(|(_, s)| hemi(s))
        .map(|(i, _)| i)
        .collect();
    let component = |numbers: &[String], h: Option<&str>, latitude: bool| -> Result<f64, CommandError> {
        if numbers.is_empty() || numbers.len() > 3 {
            return Err(failed("expected degrees, optional minutes and seconds"));
        }
        let values = numbers
            .iter()
            .map(|s| s.parse::<f64>().map_err(|_| failed("invalid coordinate number")))
            .collect::<Result<Vec<_>, _>>()?;
        let d = *values.first().ok_or_else(|| failed("missing degrees"))?;
        let m = values.get(1).copied().unwrap_or(0.0);
        let sec = values.get(2).copied().unwrap_or(0.0);
        if !d.is_finite()
            || !m.is_finite()
            || !sec.is_finite()
            || !(0.0..60.0).contains(&m)
            || !(0.0..60.0).contains(&sec)
            || values.len() > 1 && d.fract() != 0.0
            || values.len() > 2 && m.fract() != 0.0
        {
            return Err(failed("coordinate components outside bounds"));
        }
        if let Some(h) = h
            && (latitude && !matches!(h, "N" | "S")
                || !latitude && !matches!(h, "E" | "W")
                || d.is_sign_negative())
        {
            return Err(failed("conflicting hemisphere/sign or axis"));
        }
        let sign = if matches!(h, Some("S" | "W")) || d.is_sign_negative() {
            -1.0
        } else {
            1.0
        };
        let v = (d.abs() + m / 60.0 + sec / 3600.0) * sign;
        if v.abs() > if latitude { 90.0 } else { 180.0 } {
            return Err(failed("coordinate outside bounds"));
        }
        Ok(v)
    };
    let (latitude_deg, longitude_deg) = if hemis.is_empty() {
        let [a, b] = tokens.as_slice() else {
            return Ok(None);
        };
        (
            component(std::slice::from_ref(a), None, true)?,
            component(std::slice::from_ref(b), None, false)?,
        )
    } else {
        let [first, second] = hemis.as_slice() else {
            return Ok(None);
        };
        let a = tokens
            .get(*first)
            .map(String::as_str)
            .ok_or_else(|| failed("hemisphere"))?;
        let b = tokens
            .get(*second)
            .map(String::as_str)
            .ok_or_else(|| failed("hemisphere"))?;
        let (ns, ew) = if *first == 0 {
            if *second < 2 {
                return Err(failed("coordinate components"));
            }
            (tokens.get(1..*second), tokens.get(second + 1..))
        } else {
            if *second != tokens.len().saturating_sub(1) {
                return Err(failed("coordinate trailing data"));
            }
            (tokens.get(..*first), tokens.get(first + 1..*second))
        };
        let ns = ns.ok_or_else(|| failed("coordinate components"))?;
        let ew = ew.ok_or_else(|| failed("coordinate components"))?;
        if matches!(a, "N" | "S") {
            (component(ns, Some(a), true)?, component(ew, Some(b), false)?)
        } else {
            (component(ew, Some(b), true)?, component(ns, Some(a), false)?)
        }
    };
    Ok(Some(GeoPoint {
        latitude_deg,
        longitude_deg,
    }))
}
fn parse_radial(q: &str) -> Result<Option<(String, f64, f64)>, CommandError> {
    let parts: Vec<_> = q.split('/').collect();
    let (id, r, d) = if let [id, r, d] = parts.as_slice() {
        (id.to_string(), r.to_string(), d.to_string())
    } else if parts.len() == 1 && q.is_ascii() && q.len() >= 7 {
        let split = q.len() - 6;
        let Some((id, tail)) = q.split_at_checked(split) else {
            return Ok(None);
        };
        if !tail.bytes().all(|b| b.is_ascii_digit()) || !id.bytes().any(|b| b.is_ascii_alphabetic()) {
            return Ok(None);
        }
        let Some((r, d)) = tail.split_at_checked(3) else {
            return Ok(None);
        };
        (id.into(), r.into(), d.into())
    } else {
        return Ok(None);
    };
    if id.is_empty() || id.len() > 32 {
        return Err(failed("invalid radial navaid"));
    }
    let r = r.parse::<f64>().map_err(|_| failed("invalid radial"))?;
    let d = d.parse::<f64>().map_err(|_| failed("invalid radial distance"))?;
    if !r.is_finite() || !(0.0..=360.0).contains(&r) || !d.is_finite() || !(0.0..=999.0).contains(&d) {
        return Err(failed("radial outside bounds"));
    }
    Ok(Some((id, r, d)))
}
