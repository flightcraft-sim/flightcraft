//! Geographic navigation catalogue, independent of terrain installation and local NED frames.
use super::{
    download::{self, Cache, MAX_DOWNLOAD, Transport, read, sha256, write_json},
    region::{self, Request, Sources},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const SHARD_BYTES: usize = 8 * 1024 * 1024;

fn normalized(
    index: &mut Index,
    output: &Path,
    inputs: &Path,
    counts: &mut BTreeMap<String, u64>,
) -> Result<(), String> {
    let presets: Vec<region::Bundled> = download::json(&super::root()?.join("datasets/regions.json"))?;
    let template = &presets.first().ok_or("missing grid template")?.grid;
    let layers: Vec<Value> = region::LAYERS
        .iter()
        .skip(3)
        .map(|name| download::json(&inputs.join(format!("{name}.geojson"))))
        .collect::<Result<_, _>>()?;
    let layers: [Value; 6] = layers.try_into().map_err(|_| "six navigation layers")?;
    let snapshot = download::now()?.get(..10).ok_or("snapshot date")?.to_owned();
    let data = super::navigation_faa::convert_national(template, &layers, snapshot)?;
    let value = serde_json::to_value(data).map_err(|e| e.to_string())?;
    for kind in ["navaids", "fixes", "ils", "routes", "holds"] {
        let records = value[kind].as_array().ok_or("navigation records")?;
        counts.insert(kind.into(), records.len() as u64);
        for (part, records) in records.chunks(512).enumerate() {
            let mut shard = json!({"version":1,"attribution":value["attribution"],"snapshot_date":value["snapshot_date"],"effective_date":null,"navaids":[],"fixes":[],"ils":[],"routes":[],"holds":[]});
            shard[kind] = json!(records);
            let text = serde_json::to_string(&shard).map_err(|e| e.to_string())?;
            flightcraft_world::Navdata::from_json(&text).map_err(|e| e.to_string())?;
            add(
                index,
                output,
                (kind, "US"),
                &format!("navigation/{kind}-{part:04}.json"),
                records.len(),
                None,
                &shard,
            )?;
        }
    }
    let airports: Value = download::json(&inputs.join("US_Airport.geojson"))?;
    let runways: Value = download::json(&inputs.join("Runways.geojson"))?;
    let mut by_airport = BTreeMap::<String, Vec<Value>>::new();
    for runway in runways["features"].as_array().ok_or("runway features")? {
        if let Some(id) = runway["properties"]["AIRPORT_ID"].as_str() {
            by_airport.entry(id.into()).or_default().push(runway.clone());
        }
    }
    let mut states = BTreeMap::<String, Vec<flightcraft_world::Airport>>::new();
    for airport in airports["features"].as_array().ok_or("airport features")? {
        let p = &airport["properties"];
        if p["TYPE_CODE"] != "AD" || p["PRIVATEUSE"] != 0 {
            continue;
        }
        let lat = super::navdata_faa::dms(p["LATITUDE"].as_str().ok_or("airport latitude")?)?;
        let lon = super::navdata_faa::dms(p["LONGITUDE"].as_str().ok_or("airport longitude")?)?;
        let mut region = template.clone();
        region.origin.latitude_deg = lat;
        region.origin.longitude_deg = lon;
        region.origin.field_elevation_m = p["ELEVATION"].as_f64().ok_or("airport elevation")? * 0.3048;
        region.southwest_m = [-10240., -10240.];
        region.spacing_m = 80.;
        region.tiles_north = 1;
        region.tiles_east = 1;
        let id = p["GLOBAL_ID"].as_str().ok_or("airport GLOBAL_ID")?;
        let runways = by_airport.get(id).cloned().unwrap_or_default();
        let data = super::navdata_faa::import(
            &region,
            &flightcraft_world::World::default(),
            &json!({"type":"FeatureCollection","features":[airport]}),
            &json!({"type":"FeatureCollection","features":runways}),
        )?;
        states.entry(state(airport)).or_default().extend(data.airports);
    }
    let mut count = 0;
    for (state, mut airports) in states {
        airports.sort_by(|a, b| a.ident.cmp(&b.ident));
        count += airports.len() as u64;
        for (part, records) in airports.chunks(128).enumerate() {
            add(
                index,
                output,
                ("airports", &state),
                &format!("airports/{state}-{part:04}.json"),
                records.len(),
                None,
                &json!({"attribution":super::navdata_faa::ATTRIBUTION,"airports":records}),
            )?;
        }
    }
    counts.insert("public_airports".into(), count);
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shard {
    pub kind: String,
    pub state: String,
    pub path: String,
    pub bounds: Option<[f64; 4]>,
    pub count: u64,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub version: u8,
    pub coordinates: String,
    pub cycle: String,
    pub effective_date: String,
    pub shards: Vec<Shard>,
}

pub fn feature_bounds(feature: &Value) -> Option<[f64; 4]> {
    fn coordinates(v: &Value, bounds: &mut [f64; 4], found: &mut bool) {
        let Some(a) = v.as_array() else {
            return;
        };
        if let (Some(lon), Some(lat)) = (
            a.first().and_then(Value::as_f64),
            a.get(1).and_then(Value::as_f64),
        ) {
            if lon.is_finite()
                && lat.is_finite()
                && (-180.0..=180.0).contains(&lon)
                && (-90.0..=90.0).contains(&lat)
            {
                bounds[0] = bounds[0].min(lon);
                bounds[1] = bounds[1].min(lat);
                bounds[2] = bounds[2].max(lon);
                bounds[3] = bounds[3].max(lat);
                *found = true;
            }
        } else {
            for value in a {
                coordinates(value, bounds, found);
            }
        }
    }
    let mut bounds = [180., 90., -180., -90.];
    let mut found = false;
    coordinates(&feature["geometry"]["coordinates"], &mut bounds, &mut found);
    found.then_some(bounds)
}
fn state(feature: &Value) -> String {
    for key in ["STATE", "STATE_CODE", "STATE_CODE_TXT"] {
        if let Some(s) = feature["properties"][key]
            .as_str()
            .filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_uppercase()))
        {
            return s.into();
        }
    }
    "US".into()
}
fn add(
    index: &mut Index,
    directory: &Path,
    category: (&str, &str),
    name: &str,
    count: usize,
    bounds: Option<[f64; 4]>,
    value: &impl Serialize,
) -> Result<(), String> {
    let (kind, state) = category;
    write_json(&directory.join(name), value)?;
    let bytes = read(&directory.join(name), MAX_DOWNLOAD)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("navigation shard exceeds 16 MiB".into());
    }
    index.shards.push(Shard {
        kind: kind.into(),
        state: state.into(),
        path: name.into(),
        bounds,
        count: count as u64,
        bytes: bytes.len() as u64,
        sha256: sha256(&bytes),
    });
    Ok(())
}
pub fn build<T: Transport>(request: &Request, inputs: &Path, cache: &mut Cache<T>) -> Result<(), String> {
    region::acquire_layers(request, inputs, cache, None)?;
    let (cycle, effective_date) = region::acquire_procedures(request, inputs, cache)?;
    std::fs::create_dir(&request.out).map_err(|e| e.to_string())?;
    let mut index = Index {
        version: 1,
        coordinates: "WGS84 longitude/latitude degrees, elevations MSL metres".into(),
        cycle,
        effective_date,
        shards: Vec::new(),
    };
    let mut counts = BTreeMap::new();
    let mut airport_states = BTreeMap::<String, BTreeSet<String>>::new();
    for kind in region::LAYERS {
        let layer: Value = download::json(&inputs.join(format!("{kind}.geojson")))?;
        let features = layer["features"]
            .as_array()
            .ok_or("national layer needs features")?;
        counts.insert((*kind).into(), features.len() as u64);
        let mut groups = BTreeMap::<String, Vec<Value>>::new();
        for feature in features {
            if feature_bounds(feature).is_none() {
                return Err(format!("{kind} contains missing or invalid geographic geometry"));
            }
            let state = state(feature);
            if *kind == "US_Airport" {
                let ident = feature["properties"]["ICAO_ID"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| feature["properties"]["IDENT"].as_str());
                if let Some(ident) = ident {
                    airport_states
                        .entry(state.clone())
                        .or_default()
                        .insert(ident.into());
                }
            }
            groups.entry(state).or_default().push(feature.clone());
        }
        for (state, mut features) in groups {
            features.sort_by_key(|f| {
                f["properties"]["GLOBAL_ID"]
                    .as_str()
                    .or_else(|| f["properties"]["GFID"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| f["id"].to_string())
            });
            let mut shard = Vec::new();
            let mut size = 0;
            let mut number = 0;
            for feature in features {
                let bytes = serde_json::to_vec(&feature).map_err(|e| e.to_string())?.len();
                if bytes > SHARD_BYTES / 2 {
                    return Err("single FAA feature exceeds shard budget".into());
                }
                if size + bytes > SHARD_BYTES / 2 && !shard.is_empty() {
                    raw_shard(&mut index, &request.out, kind, &state, number, &shard)?;
                    shard.clear();
                    size = 0;
                    number += 1;
                }
                size += bytes;
                shard.push(feature);
            }
            if !shard.is_empty() {
                raw_shard(&mut index, &request.out, kind, &state, number, &shard)?;
            }
        }
    }
    normalized(&mut index, &request.out, inputs, &mut counts)?;
    let cifp =
        String::from_utf8(read(&inputs.join("FAACIFP18"), 128 * 1024 * 1024)?).map_err(|e| e.to_string())?;
    let dtpp =
        String::from_utf8(read(&inputs.join("dtpp.xml"), 32 * 1024 * 1024)?).map_err(|e| e.to_string())?;
    let mut failures = Vec::new();
    let mut procedures = 0;
    // Airport batches keep terminal and supporting enroute points inside the existing limits.
    for (state, ids) in airport_states {
        let ids: Vec<_> = ids.into_iter().collect();
        for (batch, ids) in ids.chunks(32).enumerate() {
            match super::procedures_faa::import(&cifp, &ids.iter().cloned().collect()).and_then(|mut p| {
                super::procedures_faa::attach_plates(&mut p, &dtpp)?;
                Ok(p)
            }) {
                Ok(data) => {
                    procedures += data.procedures.len() as u64;
                    add(
                        &mut index,
                        &request.out,
                        ("procedures", &state),
                        &format!("procedures/{state}-{batch:04}.json"),
                        data.procedures.len(),
                        None,
                        &data,
                    )?;
                }
                Err(error) => failures.push(format!("{state} batch {batch}: {error}")),
            }
        }
    }
    counts.insert("procedures".into(), procedures);
    index.shards.sort_by(|a, b| a.path.cmp(&b.path));
    let bytes = index.shards.iter().map(|s| s.bytes).sum();
    write_json(&request.out.join("index.json"), &index)?;
    write_json(
        &request.out.join("sources.json"),
        &Sources {
            version: 1,
            sources: cache.sources.clone(),
        },
    )?;
    let report=super::validation::Report{version:1,dataset:"national-navigation".into(),counts,bytes,checks:Vec::new(),procedure_parse_failures:failures.clone(),warnings:vec!["FAA geographic source layers retain public, private, military, and territorial records. Regional import policies still select supported public-use land airports. Runtime catalogue discovery is a separate integration hook: index.json identifies bounded geographic shards, without requiring terrain.".into()]};
    super::validation::write(&request.out, &report)?;
    if !failures.is_empty() {
        write_json(&request.out.with_extension("failed.json"), &report)?;
        return Err(format!(
            "national CIFP parse failed in {} batches; see the failure report",
            failures.len()
        ));
    }
    Ok(())
}
fn raw_shard(
    index: &mut Index,
    directory: &Path,
    kind: &str,
    state: &str,
    number: usize,
    features: &[Value],
) -> Result<(), String> {
    let mut bounds = [180_f64, 90_f64, -180_f64, -90_f64];
    for feature in features {
        if let Some([w, s, e, n]) = feature_bounds(feature) {
            bounds = [
                bounds[0].min(w),
                bounds[1].min(s),
                bounds[2].max(e),
                bounds[3].max(n),
            ];
        }
    }
    add(
        index,
        directory,
        (kind, state),
        &format!("layers/{kind}-{state}-{number:04}.geojson"),
        features.len(),
        Some(bounds),
        &json!({"type":"FeatureCollection","features":features}),
    )
}

pub fn procedure_view(
    national: &Path,
    airports: &BTreeSet<String>,
) -> Result<flightcraft_world::Procedures, String> {
    let index: Index = download::json(&national.join("index.json"))?;
    let mut result = flightcraft_world::Procedures {
        version: 1,
        cycle: index.cycle,
        effective_date: index.effective_date,
        attribution: flightcraft_world::procedures::PROCEDURES_ATTRIBUTION.into(),
        points: Vec::new(),
        localizers: Vec::new(),
        sbas: Vec::new(),
        procedures: Vec::new(),
    };
    let mut points = BTreeMap::new();
    let mut used = BTreeSet::new();
    for shard in index.shards.iter().filter(|s| s.kind == "procedures") {
        if !super::data::safe_path(&shard.path) || shard.bytes > 16 * 1024 * 1024 {
            return Err("invalid procedure shard".into());
        }
        let bytes = read(&national.join(&shard.path), 16 * 1024 * 1024)?;
        if bytes.len() as u64 != shard.bytes || sha256(&bytes) != shard.sha256 {
            return Err("procedure shard checksum mismatch".into());
        }
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let mut data = flightcraft_world::Procedures::from_json(text).map_err(|e| e.to_string())?;
        if data.cycle != result.cycle || data.effective_date != result.effective_date {
            return Err("national procedure shard edition mismatch".into());
        }
        data.procedures.retain(|p| airports.contains(&p.airport));
        for procedure in &data.procedures {
            for leg in procedure.routes.iter().flat_map(|r| &r.legs) {
                used.extend(
                    [&leg.fix, &leg.recommended_navaid, &leg.center_fix]
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
            }
        }
        result.procedures.extend(data.procedures);
        result.localizers.extend(
            data.localizers
                .into_iter()
                .filter(|p| airports.contains(&p.airport)),
        );
        result
            .sbas
            .extend(data.sbas.into_iter().filter(|p| airports.contains(&p.airport)));
        for point in data.points {
            if let Some(previous) = points.insert(point.id.clone(), point.clone())
                && serde_json::to_value(previous).map_err(|e| e.to_string())?
                    != serde_json::to_value(point).map_err(|e| e.to_string())?
            {
                return Err("inconsistent national procedure point".into());
            }
        }
    }
    result.points = points
        .into_values()
        .filter(|p| p.airport.as_ref().is_some_and(|a| airports.contains(a)) || used.contains(&p.id))
        .collect();
    result
        .procedures
        .sort_by(|a, b| (&a.airport, &a.ident).cmp(&(&b.airport, &b.ident)));
    let text = serde_json::to_string(&result).map_err(|e| e.to_string())?;
    flightcraft_world::Procedures::from_json(&text).map_err(|e| e.to_string())
}

pub fn views(national: &Path, output: &Path, bounds: [f64; 4]) -> Result<(), String> {
    let index: Index = download::json(&national.join("index.json"))?;
    if index.version != 1 || !index.coordinates.starts_with("WGS84") {
        return Err("unsupported national index".into());
    }
    for kind in region::LAYERS {
        let mut features = Vec::new();
        for shard in index.shards.iter().filter(|s| s.kind == *kind) {
            if !super::data::safe_path(&shard.path) || shard.bytes > 16 * 1024 * 1024 {
                return Err("invalid national shard".into());
            }
            // A 40 NM geographic margin includes navaid components joined by the regional importer.
            let [w, s, e, n] = bounds;
            let latitude = s.abs().max(n.abs()).min(85.);
            let longitude_margin = 74080. / (111000. * libm::cos(latitude.to_radians())) + 0.2;
            let margin = [
                (w - longitude_margin).max(-180.),
                (s - 1.).max(-90.),
                (e + longitude_margin).min(180.),
                (n + 1.).min(90.),
            ];
            if shard.bounds.is_some_and(|b| !region::intersects(b, margin)) {
                continue;
            }
            let bytes = read(&national.join(&shard.path), 16 * 1024 * 1024)?;
            if bytes.len() as u64 != shard.bytes || sha256(&bytes) != shard.sha256 {
                return Err("national shard checksum mismatch".into());
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            for feature in value["features"].as_array().ok_or("national shard features")? {
                if feature_bounds(feature).is_some_and(|b| region::intersects(b, margin)) {
                    features.push(feature.clone());
                }
            }
        }
        write_json(
            &output.join(format!("{kind}.geojson")),
            &json!({"type":"FeatureCollection","features":features}),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn geographic_shards_filter_crossing_features_and_verify_checksums() {
        let root = std::env::temp_dir().join(format!("flightcraft-national-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut index = Index {
            version: 1,
            coordinates: "WGS84".into(),
            cycle: "2611".into(),
            effective_date: "2026-10-29".into(),
            shards: Vec::new(),
        };
        let crossing = json!({"type":"Feature","geometry":{"type":"LineString","coordinates":[[-78.,41.],[-72.,41.]]},"properties":{"STATE":"US"}});
        let island = json!({"type":"Feature","geometry":{"type":"Point","coordinates":[144.8,13.5]},"properties":{"STATE":"GU"}});
        raw_shard(&mut index, &root, "ATS_Route", "US", 0, &[crossing]).unwrap();
        raw_shard(&mut index, &root, "US_Airport", "GU", 0, &[island]).unwrap();
        write_json(&root.join("index.json"), &index).unwrap();
        views(&root, &root.join("view"), [-76., 40., -74., 42.]).unwrap();
        let routes: Value = download::json(&root.join("view/ATS_Route.geojson")).unwrap();
        assert_eq!(routes["features"].as_array().unwrap().len(), 1);
        let airports: Value = download::json(&root.join("view/US_Airport.geojson")).unwrap();
        assert!(airports["features"].as_array().unwrap().is_empty());
        let path = root.join(&index.shards.first().unwrap().path);
        std::fs::write(path, b"corrupt").unwrap();
        assert!(views(&root, &root.join("broken"), [-76., 40., -74., 42.]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
