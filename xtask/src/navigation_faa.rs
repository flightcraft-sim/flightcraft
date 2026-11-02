//! Bounded offline conversion of FAA ADDS navigation layers.
use flightcraft_world::{GeoPoint, Navdata, Region, geo::LocalProjection, navdata::*};
use serde_json::Value;
use std::{collections::BTreeMap, io::Read, path::Path};
fn string<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| text_ok(s, 256))
        .ok_or_else(|| format!("missing or invalid {k}"))
}
fn optional(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}
fn number(v: &Value, k: &str) -> Result<f64, String> {
    v.get(k)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("missing number {k}"))
}
fn angle(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}
fn coordinate(v: &Value) -> Result<GeoPoint, String> {
    let a = v.as_array().ok_or("coordinate must be array")?;
    if !(2..=3).contains(&a.len()) {
        return Err("coordinate size".into());
    }
    let p = GeoPoint {
        longitude_deg: a.first().and_then(Value::as_f64).ok_or("longitude")?,
        latitude_deg: a.get(1).and_then(Value::as_f64).ok_or("latitude")?,
    };
    p.validate().map_err(|e| e.to_string())?;
    Ok(p)
}
fn point(v: &Value) -> Result<GeoPoint, String> {
    let g = v.get("geometry").ok_or("geometry")?;
    if g.get("type").and_then(Value::as_str) != Some("Point") {
        return Err("expected Point".into());
    }
    coordinate(g.get("coordinates").ok_or("coordinates")?)
}
fn props(v: &Value) -> Result<&Value, String> {
    let p = v.get("properties").ok_or("properties")?;
    if !p.is_object() {
        return Err("invalid properties".into());
    }
    for key in [
        "GLOBAL_ID",
        "GFID",
        "IDENT",
        "IDENT_TXT",
        "NAME_TXT",
        "NAME",
        "CLIENTKEY_ID",
        "FREQUENCY_UOM",
        "CHANNEL_TXT",
        "CHANNEL",
        "LAT_TXT",
        "LONG_TXT",
        "LATITUDE",
        "LONGITUDE",
        "ILSSYS_ID",
        "STARTPT_ID",
        "ENDPT_ID",
        "REFPT_ID",
        "DIRTURN",
    ] {
        if p.get(key).is_some_and(|v| !v.is_null() && !v.is_string()) {
            return Err(format!("invalid string property {key}"));
        }
    }
    for key in [
        "FREQUENCY_VAL",
        "FREQUENCY",
        "MAGNETICVARIATION_VAL",
        "MAGVAR",
        "MAGBRG",
        "EFFECTIVE_DATE",
    ] {
        if p.get(key).is_some_and(|v| !v.is_null() && v.as_f64().is_none()) {
            return Err(format!("invalid numeric property {key}"));
        }
    }
    Ok(p)
}
fn features(v: &Value) -> Result<&[Value], String> {
    if v.get("type").and_then(Value::as_str) != Some("FeatureCollection")
        || v.get("exceededTransferLimit").and_then(Value::as_bool) == Some(true)
        || v.pointer("/properties/exceededTransferLimit")
            .and_then(Value::as_bool)
            == Some(true)
    {
        return Err("invalid or truncated FAA collection".into());
    }
    let f = v.get("features").and_then(Value::as_array).ok_or("features")?;
    if f.len() > 100000 {
        return Err("too many source features".into());
    }
    Ok(f)
}
fn frequency(p: &Value, key: &str, uom: &str, ndb: bool) -> Result<Option<f64>, String> {
    if p.get(key).is_none_or(Value::is_null) {
        return Ok(None);
    }
    let n = number(p, key)?;
    let multiplier = match p.get(uom).and_then(Value::as_str) {
        Some("HZ" | "Hz") => 1.0,
        Some("KHZ" | "kHz") => 1000.0,
        Some("MHZ" | "MHz") => 1e6,
        None => {
            if ndb {
                1000.0
            } else {
                1e6
            }
        }
        _ => return Err("unknown frequency unit".into()),
    };
    Ok(Some(n * multiplier))
}
fn dms(s: &str) -> Result<f64, String> {
    let suffix = s.chars().last().ok_or("DMS")?;
    let body = s.get(..s.len().saturating_sub(1)).ok_or("DMS")?;
    let vals = body
        .split('-')
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "DMS")?;
    let [d, m, se] = vals.as_slice() else {
        return Err("DMS".into());
    };
    if !d.is_finite()
        || !m.is_finite()
        || !se.is_finite()
        || *d < 0.0
        || !(0.0..60.0).contains(m)
        || !(0.0..60.0).contains(se)
        || !matches!(suffix, 'N' | 'S' | 'E' | 'W')
    {
        return Err("DMS bounds".into());
    }
    let x = d + m / 60.0 + se / 3600.0;
    if x > if matches!(suffix, 'N' | 'S') { 90.0 } else { 180.0 } {
        return Err("DMS bounds".into());
    }
    Ok(x * if matches!(suffix, 'S' | 'W') { -1.0 } else { 1.0 })
}
fn source_position(f: &Value, p: &Value, lat: &str, lon: &str) -> Result<GeoPoint, String> {
    let geometry = point(f)?;
    match (
        p.get(lat).and_then(Value::as_str),
        p.get(lon).and_then(Value::as_str),
    ) {
        (Some(a), Some(b)) => {
            let d = GeoPoint {
                latitude_deg: dms(a)?,
                longitude_deg: dms(b)?,
            };
            if d.inverse(geometry).0 > 250.0 {
                return Err("DMS and geometry disagree by more than 250 m".into());
            }
            Ok(d)
        }
        (None, None) => Ok(geometry),
        _ => Err("partial source coordinates".into()),
    }
}
struct Bounds {
    national: bool,
    projection: LocalProjection,
    south: f64,
    west: f64,
    north: f64,
    east: f64,
}
impl Bounds {
    fn new(r: &Region) -> Result<Self, String> {
        r.validate().map_err(|e| e.to_string())?;
        let [south, west] = r.southwest_m;
        let extent = f64::from(r.tile_size - 1) * f64::from(r.spacing_m);
        Ok(Self {
            national: false,
            projection: LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg)
                .map_err(|e| e.to_string())?,
            south,
            west,
            north: south + f64::from(r.tiles_north) * extent,
            east: west + f64::from(r.tiles_east) * extent,
        })
    }
    fn local(&self, p: GeoPoint) -> Result<[f64; 2], String> {
        self.projection
            .project(p.latitude_deg, p.longitude_deg)
            .map_err(|e| e.to_string())
    }
    fn contains(&self, p: GeoPoint, margin: f64) -> Result<bool, String> {
        if self.national {
            return Ok(true);
        }
        let [n, e] = self.local(p)?;
        Ok(libm::hypot(
            (self.south - n).max(0.0).max(n - self.north),
            (self.west - e).max(0.0).max(e - self.east),
        ) <= margin)
    }
    fn crosses(&self, a: GeoPoint, b: GeoPoint) -> Result<bool, String> {
        if self.national {
            return Ok(true);
        }
        let [an, ae] = self.local(a)?;
        let [bn, be] = self.local(b)?;
        let (mut low, mut high) = (0.0_f64, 1.0_f64);
        for (x, d, min, max) in [
            (an, bn - an, self.south, self.north),
            (ae, be - ae, self.west, self.east),
        ] {
            if d.abs() < 1e-12 {
                if x < min || x > max {
                    return Ok(false);
                }
            } else {
                let t1 = (min - x) / d;
                let t2 = (max - x) / d;
                low = low.max(t1.min(t2));
                high = high.min(t1.max(t2));
                if low > high {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}
pub(super) fn convert(
    r: &Region,
    layers: &[Value; 6],
    snapshot: String,
    effective: Option<String>,
) -> Result<Navdata, String> {
    convert_bounded(layers, snapshot, effective, Bounds::new(r)?, true)
}

pub(super) fn convert_national(r: &Region, layers: &[Value; 6], snapshot: String) -> Result<Navdata, String> {
    let mut bounds = Bounds::new(r)?;
    bounds.national = true;
    convert_bounded(layers, snapshot, None, bounds, false)
}

fn convert_bounded(
    layers: &[Value; 6],
    snapshot: String,
    effective: Option<String>,
    bounds: Bounds,
    validate: bool,
) -> Result<Navdata, String> {
    let [systems, components, fixes, ils, routes, holds] = layers;
    let mut data = Navdata {
        version: 1,
        attribution: NAVDATA_ATTRIBUTION.into(),
        effective_date: effective,
        snapshot_date: snapshot,
        ..Navdata::default()
    };
    let mut endpoints = BTreeMap::new();
    let mut component_records = Vec::new();
    for f in features(components)? {
        let p = props(f)?;
        let pos = source_position(f, p, "LAT_TXT", "LONG_TXT")?;
        let id = string(p, "GFID")?.to_uppercase();
        let subtype = number(p, "SUBTYPE_CODE")?;
        // Inner markers have no ident and are not components of a NAVAIDSystem.
        if p.get("IDENT_TXT").is_some_and(Value::is_null) && subtype == 0.0 {
            continue;
        }
        let ident = string(p, "IDENT_TXT")?.to_owned();
        if subtype.fract() != 0.0 || !(0.0..=255.0).contains(&subtype) {
            return Err("invalid component subtype".into());
        }
        endpoints.insert(id.clone(), ident.clone());
        let c = NavaidComponent {
            id,
            source_key: string(p, "CLIENTKEY_ID")?.into(),
            subtype_code: subtype as u8,
            position: pos,
            frequency_hz: frequency(p, "FREQUENCY_VAL", "FREQUENCY_UOM", subtype == 1.0)?,
            channel: optional(p, "CHANNEL_TXT"),
            magnetic_variation_deg: angle(p, "MAGNETICVARIATION_VAL"),
            effective_unix_ms: p.get("EFFECTIVE_DATE").and_then(Value::as_i64),
        };
        component_records.push((ident, optional(p, "CLIENTKEY_ID").unwrap_or_default(), c));
    }
    for f in features(systems)? {
        let p = props(f)?;
        let position = point(f)?;
        let id = string(p, "GLOBAL_ID")?.to_uppercase();
        let ident = string(p, "IDENT")?.to_owned();
        endpoints.insert(id.clone(), ident.clone());
        // FAA system codes: 2/3 NDB/locator, 4 VOR, 5 DME, 6 VOR/DME, 7 TACAN, 8 VORTAC.
        let system_type = number(p, "TYPE_CODE")?;
        if system_type.fract() != 0.0 || !(0.0..=255.0).contains(&system_type) {
            return Err("invalid system type".into());
        }
        let (kind, label) = match system_type as u8 {
            2 | 3 => (NavaidKind::Ndb, "NDB"),
            4 => (NavaidKind::Vor, "VOR"),
            5 => (NavaidKind::Dme, "DME"),
            6 => (NavaidKind::VorDme, "VOR/DME"),
            7 => (NavaidKind::Tacan, "TACAN"),
            8 => (NavaidKind::Vortac, "VORTAC"),
            _ => return Err("unsupported FAA navaid system type".into()),
        };
        if !bounds.contains(position, 40.0 * 1852.0)? {
            continue;
        }
        let components = component_records
            .iter()
            .filter(|(i, key, c)| {
                i == &ident
                    && key.split('_').nth(1).is_some_and(|s| {
                        s == label
                            || (label == "NDB" && s == "NDB/DME")
                            || matches!((label, s), ("VOR/DME", "VORTAC") | ("VORTAC", "VOR/DME"))
                    })
                    && position.inverse(c.position).0 < 10000.0
            })
            .map(|(_, _, c)| c.clone())
            .collect();
        data.navaids.push(Navaid {
            id,
            ident,
            name: string(p, "NAME_TXT")?.into(),
            kind,
            position,
            components,
        });
    }
    for f in features(fixes)? {
        let p = props(f)?;
        let position = source_position(f, p, "LAT_TXT", "LONG_TXT")?;
        let id = string(p, "GFID")?.to_uppercase();
        let ident = string(p, "IDENT_TXT")?.to_owned();
        endpoints.insert(id.clone(), ident.clone());
        if bounds.contains(position, 0.0)? {
            data.fixes.push(Fix {
                id,
                name: optional(p, "NAME_TXT")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| ident.clone()),
                ident,
                position,
                effective_unix_ms: p.get("EFFECTIVE_DATE").and_then(Value::as_i64),
            });
        }
    }
    for f in features(ils)? {
        let p = props(f)?;
        let position = source_position(f, p, "LATITUDE", "LONGITUDE")?;
        if !bounds.contains(position, 0.0)? {
            continue;
        }
        let ident = optional(p, "IDENT");
        let typ = number(p, "ILS_TYPE")?;
        if typ.fract() != 0.0 || !(0.0..=255.0).contains(&typ) {
            return Err("ILS type".into());
        }
        data.ils.push(IlsComponent {
            id: string(p, "GLOBAL_ID")?.to_uppercase(),
            name: optional(p, "NAME")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| ident.clone().unwrap_or_else(|| "Unidentified marker".into())),
            ident,
            position,
            system_id: string(p, "ILSSYS_ID")?.into(),
            type_code: typ as u8,
            frequency_hz: frequency(p, "FREQUENCY", "FREQUENCY_UOM", false)?,
            channel: optional(p, "CHANNEL"),
            magnetic_bearing_deg: angle(p, "MAGBRG"),
            magnetic_variation_deg: angle(p, "MAGVAR"),
        });
    }
    for f in features(routes)? {
        let p = props(f)?;
        if f.pointer("/geometry/type").and_then(Value::as_str) != Some("LineString") {
            return Err("route must be LineString".into());
        }
        let coords = f
            .pointer("/geometry/coordinates")
            .and_then(Value::as_array)
            .ok_or("route coordinates")?;
        if !(2..=4096).contains(&coords.len()) {
            return Err("route size".into());
        }
        let geometry = coords.iter().map(coordinate).collect::<Result<Vec<_>, _>>()?;
        let mut crosses = false;
        for pair in geometry.windows(2) {
            if let [a, b] = pair {
                crosses |= bounds.crosses(*a, *b)?;
            }
        }
        if !crosses {
            continue;
        }
        let start_id = string(p, "STARTPT_ID")?.to_uppercase();
        let end_id = string(p, "ENDPT_ID")?.to_uppercase();
        data.routes.push(RouteSegment {
            id: string(p, "GLOBAL_ID")?.to_uppercase(),
            ident: string(p, "IDENT")?.into(),
            start_ident: endpoints.get(&start_id).cloned(),
            end_ident: endpoints.get(&end_id).cloned(),
            start_id,
            end_id,
            geometry,
        });
    }
    for f in features(holds)? {
        let p = props(f)?;
        let position = source_position(f, p, "LATITUDE", "LONGITUDE")?;
        if bounds.contains(position, 0.0)? {
            data.holds.push(HoldingPattern {
                id: string(p, "GLOBAL_ID")?.to_uppercase(),
                name: string(p, "NAME")?.into(),
                position,
                reference_id: string(p, "REFPT_ID")?.to_uppercase(),
                inbound_course_deg: number(p, "CRSIN")?,
                turn: string(p, "DIRTURN")?.into(),
            });
        }
    }
    data.navaids.sort_by(|a, b| a.id.cmp(&b.id));
    data.fixes.sort_by(|a, b| a.id.cmp(&b.id));
    data.ils.sort_by(|a, b| a.id.cmp(&b.id));
    data.routes.sort_by(|a, b| a.id.cmp(&b.id));
    data.holds.sort_by(|a, b| a.id.cmp(&b.id));
    if validate {
        data.validate().map_err(|e| e.to_string())?;
    }
    Ok(data)
}
fn read(path: &Path, max: usize) -> Result<String, String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(max as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > max {
        return Err("input too large".into());
    }
    Ok(text)
}
pub fn run() -> Result<(), String> {
    run_args(std::env::args().skip(2))
}

pub(super) fn run_args(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut opts = BTreeMap::new();
    while let Some(k) = args.next() {
        let v = args.next().ok_or("missing option value")?;
        if !matches!(
            k.as_str(),
            "--region" | "--layers" | "--out" | "--snapshot-date" | "--effective-date"
        ) || opts.insert(k, v).is_some()
        {
            return Err("unknown or duplicate option".into());
        }
    }
    let get = |k: &str| {
        opts.get(k)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {k}"))
    };
    let region = Region::from_json(&read(Path::new(get("--region")?), 65536)?).map_err(|e| e.to_string())?;
    let mut layers = Vec::new();
    for name in [
        "NAVAIDSystem",
        "NavaidComponent",
        "DesignatedPoint",
        "ILS_Component",
        "ATS_Route",
        "HoldingPattern",
    ] {
        layers.push(
            serde_json::from_str(&read(
                &Path::new(get("--layers")?).join(format!("{name}.geojson")),
                32 * 1024 * 1024,
            )?)
            .map_err(|e| e.to_string())?,
        );
    }
    let layers: [Value; 6] = layers.try_into().map_err(|_| "six layers required")?;
    let data = convert(
        &region,
        &layers,
        get("--snapshot-date")?.into(),
        opts.get("--effective-date").cloned(),
    )?;
    let text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    if text.len() > MAX_NAVDATA_BYTES {
        return Err("output too large".into());
    }
    use std::io::Write;
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(get("--out")?)
        .map_err(|e| e.to_string())?;
    out.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    eprintln!(
        "navigation: {} navaids, {} fixes, {} ILS, {} route segments, {} holds; cycle {:?}, snapshot {}",
        data.navaids.len(),
        data.fixes.len(),
        data.ils.len(),
        data.routes.len(),
        data.holds.len(),
        data.effective_date,
        data.snapshot_date
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    #[test]
    fn handwritten_layers_join_units_dms_and_crossing_route() {
        let r = Region::from_json(include_str!("../../assets/terrain/new-york/region.json")).unwrap();
        let collection = |f: Vec<Value>| serde_json::json!({"type":"FeatureCollection","features":f});
        let pt = |p: Value| serde_json::json!({"type":"Feature","geometry":{"type":"Point","coordinates":[-75.0,41.0]},"properties":p});
        let layers = [
            collection(vec![pt(
                serde_json::json!({"GLOBAL_ID":"sys","IDENT":"ABC","NAME_TXT":"Example","TYPE_CODE":6}),
            )]),
            collection(vec![pt(
                serde_json::json!({"GFID":"comp","IDENT_TXT":"ABC","CLIENTKEY_ID":"ABC_VORTAC_EXAMPLE_3","SUBTYPE_CODE":3,"FREQUENCY_VAL":113.0,"FREQUENCY_UOM":"MHZ","LAT_TXT":"41-00-00N","LONG_TXT":"075-00-00W"}),
            )]),
            collection(vec![pt(
                serde_json::json!({"GFID":"fix","IDENT_TXT":"EXAMP","NAME_TXT":null,"LAT_TXT":"41-00-00N","LONG_TXT":"075-00-00W"}),
            )]),
            collection(vec![pt(
                serde_json::json!({"GLOBAL_ID":"ils-marker","IDENT":null,"NAME":null,"ILS_TYPE":0,"ILSSYS_ID":"ils-system"}),
            )]),
            collection(vec![
                serde_json::json!({"geometry":{"type":"LineString","coordinates":[[-75.1,41.0],[-73.0,41.0]]},"properties":{"GLOBAL_ID":"route","IDENT":"V1","STARTPT_ID":"fix","ENDPT_ID":"sys"}}),
            ]),
            collection(vec![]),
        ];
        let d = convert(&r, &layers, "2026-11-01".into(), None).unwrap();
        assert_eq!(d.navaids[0].components[0].frequency_hz, Some(113e6));
        assert_eq!(d.navaids[0].components[0].source_key, "ABC_VORTAC_EXAMPLE_3");
        assert!(d.ils[0].ident.is_none());
        assert_eq!(d.routes[0].start_ident.as_deref(), Some("EXAMP"));
        assert_eq!(d.fixes[0].name, "EXAMP");
        let mut bad = layers.clone();
        bad[0]["exceededTransferLimit"] = Value::Bool(true);
        assert!(convert(&r, &bad, "2026-11-01".into(), None).is_err());
        assert!(dms("90-01-00N").is_err());
    }
}
