//! Portable, bounded regional navigation catalogue. Not for real-world navigation.
use crate::TerrainError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const MAX_NAVDATA_BYTES: usize = 8 * 1024 * 1024;
pub const NAVDATA_ATTRIBUTION: &str =
    "Navigation data: FAA Aeronautical Data Delivery Service, public domain; not for real-world navigation";
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GeoPoint {
    #[schemars(range(min=-90,max=90))]
    pub latitude_deg: f64,
    #[schemars(range(min=-180,max=180))]
    pub longitude_deg: f64,
}
impl GeoPoint {
    pub fn validate(self) -> Result<(), TerrainError> {
        if !self.latitude_deg.is_finite()
            || self.latitude_deg.abs() > 90.0
            || !self.longitude_deg.is_finite()
            || self.longitude_deg.abs() > 180.0
        {
            return Err(TerrainError("invalid geographic point"));
        }
        Ok(())
    }
    /// Great-circle distance and initial true bearing, using the IUGG mean Earth radius.
    pub fn inverse(self, other: Self) -> (f64, f64) {
        let (a, b) = (self.latitude_deg.to_radians(), other.latitude_deg.to_radians());
        let dl = (other.longitude_deg - self.longitude_deg).to_radians();
        let h = libm::sin((b - a) * 0.5) * libm::sin((b - a) * 0.5)
            + libm::cos(a) * libm::cos(b) * libm::sin(dl * 0.5) * libm::sin(dl * 0.5);
        let d = 2.0 * 6371008.8 * libm::asin(libm::sqrt(h.clamp(0.0, 1.0)));
        let brg = libm::atan2(
            libm::sin(dl) * libm::cos(b),
            libm::cos(a) * libm::sin(b) - libm::sin(a) * libm::cos(b) * libm::cos(dl),
        )
        .to_degrees()
        .rem_euclid(360.0);
        (d, brg)
    }
    pub fn destination(self, bearing_deg: f64, distance_m: f64) -> Self {
        let (lat, lon) = (self.latitude_deg.to_radians(), self.longitude_deg.to_radians());
        let b = bearing_deg.to_radians();
        let d = distance_m / 6371008.8;
        let l = libm::asin(
            (libm::sin(lat) * libm::cos(d) + libm::cos(lat) * libm::sin(d) * libm::cos(b)).clamp(-1.0, 1.0),
        );
        let o = lon
            + libm::atan2(
                libm::sin(b) * libm::sin(d) * libm::cos(lat),
                libm::cos(d) - libm::sin(lat) * libm::sin(l),
            );
        Self {
            latitude_deg: l.to_degrees(),
            longitude_deg: (o.to_degrees() + 180.0).rem_euclid(360.0) - 180.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NavaidKind {
    Vor,
    VorDme,
    Vortac,
    Ndb,
    Dme,
    Tacan,
    Marker,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavaidComponent {
    pub id: String,
    pub source_key: String,
    pub subtype_code: u8,
    pub position: GeoPoint,
    pub frequency_hz: Option<f64>,
    pub channel: Option<String>,
    pub magnetic_variation_deg: Option<f64>,
    /// Original source record date, distinct from the dataset cycle date.
    pub effective_unix_ms: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Navaid {
    pub id: String,
    pub ident: String,
    pub name: String,
    pub kind: NavaidKind,
    pub position: GeoPoint,
    #[schemars(length(max = 32))]
    pub components: Vec<NavaidComponent>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fix {
    pub id: String,
    pub ident: String,
    pub name: String,
    pub position: GeoPoint,
    pub effective_unix_ms: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IlsComponent {
    pub id: String,
    pub ident: Option<String>,
    pub name: String,
    pub position: GeoPoint,
    pub system_id: String,
    pub type_code: u8,
    pub frequency_hz: Option<f64>,
    pub channel: Option<String>,
    pub magnetic_bearing_deg: Option<f64>,
    pub magnetic_variation_deg: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteSegment {
    pub id: String,
    pub ident: String,
    pub start_id: String,
    pub end_id: String,
    pub start_ident: Option<String>,
    pub end_ident: Option<String>,
    #[schemars(length(min = 2, max = 4096))]
    pub geometry: Vec<GeoPoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HoldingPattern {
    pub id: String,
    pub name: String,
    pub position: GeoPoint,
    pub reference_id: String,
    pub inbound_course_deg: f64,
    pub turn: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Navdata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 20000))]
    pub comms: Vec<crate::ComStation>,
    #[schemars(range(min = 1, max = 1))]
    pub version: u8,
    #[schemars(length(min = 1, max = 2048))]
    pub attribution: String,
    /// ISO cycle date, or null when the provider supplied no dataset effective date.
    pub effective_date: Option<String>,
    pub snapshot_date: String,
    #[schemars(length(max = 4096))]
    pub navaids: Vec<Navaid>,
    #[schemars(length(max = 20000))]
    pub fixes: Vec<Fix>,
    #[schemars(length(max = 4096))]
    pub ils: Vec<IlsComponent>,
    #[schemars(length(max = 20000))]
    pub routes: Vec<RouteSegment>,
    #[schemars(length(max = 4096))]
    pub holds: Vec<HoldingPattern>,
}
pub fn text_ok(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
pub fn iso_date(s: &str) -> bool {
    let parts: Vec<_> = s.split('-').collect();
    let [y, m, d] = parts.as_slice() else { return false };
    if y.len() != 4
        || m.len() != 2
        || d.len() != 2
        || ![y, m, d].iter().all(|s| s.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<u16>(), m.parse::<usize>(), d.parse::<u8>()) else {
        return false;
    };
    let leap = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    s.len() == 10
        && (1900..=2200).contains(&y)
        && m > 0
        && d > 0
        && [
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
        ]
        .get(m.saturating_sub(1))
        .is_some_and(|max| d <= *max)
}
impl Navdata {
    pub fn from_json(text: &str) -> Result<Self, TerrainError> {
        if text.len() > MAX_NAVDATA_BYTES {
            return Err(TerrainError("navdata exceeds 8 MiB"));
        }
        let v: Self = serde_json::from_str(text).map_err(|_| TerrainError("invalid navdata JSON"))?;
        v.validate()?;
        Ok(v)
    }
    pub fn validate(&self) -> Result<(), TerrainError> {
        if self.version != 1
            || !text_ok(&self.attribution, 2048)
            || !iso_date(&self.snapshot_date)
            || self.effective_date.as_deref().is_some_and(|d| !iso_date(d))
            || self.navaids.len() > 4096
            || self.fixes.len() > 20000
            || self.ils.len() > 4096
            || self.routes.len() > 20000
            || self.holds.len() > 4096
        {
            return Err(TerrainError("navdata metadata or count bounds"));
        }
        let mut station_ids = BTreeSet::new();
        if self.comms.len() > 20000 {
            return Err(TerrainError("too many communications stations"));
        }
        for station in &self.comms {
            station.validate()?;
            if !station_ids.insert(&station.id) {
                return Err(TerrainError("duplicate communications station"));
            }
        }
        let mut ids = BTreeSet::new();
        let mut check = |id: &str, name: &str, p: GeoPoint| -> Result<(), TerrainError> {
            if !text_ok(id, 128) || !text_ok(name, 256) || !ids.insert(id.to_owned()) {
                return Err(TerrainError("invalid or duplicate navigation id/name"));
            }
            p.validate()
        };
        let freq = |v: Option<f64>| v.is_none_or(|n| n.is_finite() && (10000.0..=2e9).contains(&n));
        let angle = |v: Option<f64>| v.is_none_or(|n| n.is_finite() && n.abs() <= 360.0);
        for n in &self.navaids {
            check(&n.id, &n.name, n.position)?;
            if !text_ok(&n.ident, 32) || n.components.len() > 32 {
                return Err(TerrainError("invalid navaid"));
            }
            for c in &n.components {
                check(&c.id, "component", c.position)?;
                if !text_ok(&c.source_key, 256)
                    || !freq(c.frequency_hz)
                    || !angle(c.magnetic_variation_deg)
                    || c.channel.as_deref().is_some_and(|s| !text_ok(s, 32))
                {
                    return Err(TerrainError("invalid navaid component"));
                }
            }
        }
        for f in &self.fixes {
            check(&f.id, &f.name, f.position)?;
            if !text_ok(&f.ident, 32) {
                return Err(TerrainError("invalid fix ident"));
            }
        }
        for i in &self.ils {
            check(&i.id, &i.name, i.position)?;
            if i.ident.as_deref().is_some_and(|s| !text_ok(s, 32))
                || (i.ident.is_none() && i.type_code != 0)
                || !text_ok(&i.system_id, 128)
                || !freq(i.frequency_hz)
                || !angle(i.magnetic_bearing_deg)
                || !angle(i.magnetic_variation_deg)
                || i.channel.as_deref().is_some_and(|s| !text_ok(s, 32))
            {
                return Err(TerrainError("invalid ILS"));
            }
        }
        let mut vertices = 0usize;
        for r in &self.routes {
            let first = r.geometry.first().ok_or(TerrainError("empty route"))?;
            check(&r.id, &r.ident, *first)?;
            if r.geometry.len() < 2
                || r.geometry.len() > 4096
                || !text_ok(&r.start_id, 128)
                || !text_ok(&r.end_id, 128)
                || [&r.start_ident, &r.end_ident]
                    .iter()
                    .any(|v| v.as_deref().is_some_and(|s| !text_ok(s, 32)))
            {
                return Err(TerrainError("invalid route"));
            }
            vertices = vertices.saturating_add(r.geometry.len());
            for p in &r.geometry {
                p.validate()?;
            }
        }
        if vertices > 100000 {
            return Err(TerrainError("too many route vertices"));
        }
        for h in &self.holds {
            check(&h.id, &h.name, h.position)?;
            if !text_ok(&h.reference_id, 128)
                || !matches!(h.turn.as_str(), "L" | "R")
                || !h.inbound_course_deg.is_finite()
                || !(0.0..=360.0).contains(&h.inbound_course_deg)
            {
                return Err(TerrainError("invalid hold"));
            }
        }
        Ok(())
    }
}
