//! Offline FAA ADDS GeoJSON import. No network access or simulator-derived data.
use flightcraft_world::{
    Airport, AirportRunway, Airports, HeightTile, Region, RunwayEnd, RunwaySurface, World,
    geo::LocalProjection,
};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
pub const ATTRIBUTION: &str = "Airports and runways: FAA Aeronautical Data Delivery Service, public domain";
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string {key}"))
}
fn number(v: &Value, key: &str) -> Result<f64, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("missing finite number {key}"))
}
pub(super) fn dms(text: &str) -> Result<f64, String> {
    let suffix = text.chars().last().ok_or("empty coordinate")?;
    let components = text.get(..text.len().saturating_sub(1)).ok_or("invalid DMS")?;
    let values = components
        .split('-')
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid DMS")?;
    let [d, m, s] = values.as_slice() else {
        return Err("expected degrees-minutes-seconds".into());
    };
    let max = if matches!(suffix, 'N' | 'S') {
        90.0
    } else if matches!(suffix, 'E' | 'W') {
        180.0
    } else {
        return Err("invalid hemisphere".into());
    };
    if !d.is_finite()
        || !m.is_finite()
        || !s.is_finite()
        || *d < 0.0
        || *d > max
        || !(0.0..60.0).contains(m)
        || !(0.0..60.0).contains(s)
    {
        return Err("DMS outside bounds".into());
    }
    let value = d + m / 60.0 + s / 3600.0;
    if value > max {
        return Err("DMS outside bounds".into());
    }
    Ok(value * if matches!(suffix, 'S' | 'W') { -1.0 } else { 1.0 })
}
fn features(v: &Value) -> Result<&[Value], String> {
    if v.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
        return Err("expected GeoJSON FeatureCollection".into());
    }
    if v.get("exceededTransferLimit").and_then(Value::as_bool) == Some(true) {
        return Err("truncated FAA layer: fetch all pages before import".into());
    }
    let features = v
        .get("features")
        .and_then(Value::as_array)
        .ok_or("missing features")?;
    if features.len() > 100_000 {
        return Err("too many features".into());
    }
    Ok(features)
}
pub(super) fn centreline(feature: &Value, projection: &LocalProjection) -> Result<[[f64; 2]; 2], String> {
    let geometry = feature.get("geometry").ok_or("missing runway geometry")?;
    if geometry.get("type").and_then(Value::as_str) != Some("Polygon") {
        return Err("runway must be a Polygon".into());
    }
    let rings = geometry
        .get("coordinates")
        .and_then(Value::as_array)
        .ok_or("missing polygon rings")?;
    if rings.len() != 1 {
        return Err("runway polygon must have one ring".into());
    }
    let ring = rings
        .first()
        .and_then(Value::as_array)
        .ok_or("missing runway ring")?;
    if ring.len() != 5 || ring.first() != ring.last() {
        return Err("runway outline must be a closed quadrilateral".into());
    }
    let mut points = Vec::new();
    for coord in ring.iter().take(4) {
        let coords = coord.as_array().ok_or("invalid polygon point")?;
        let lon = coords
            .first()
            .and_then(Value::as_f64)
            .ok_or("missing longitude")?;
        let lat = coords.get(1).and_then(Value::as_f64).ok_or("missing latitude")?;
        points.push(projection.project(lat, lon).map_err(|e| e.to_string())?);
    }
    let [a, b, c, d] = points.as_slice() else {
        return Err("invalid quadrilateral".into());
    };
    let distance = |a: &[f64; 2], b: &[f64; 2]| {
        let [an, ae] = *a;
        let [bn, be] = *b;
        libm::hypot(bn - an, be - ae)
    };
    let mid = |a: &[f64; 2], b: &[f64; 2]| {
        let [an, ae] = *a;
        let [bn, be] = *b;
        [(an + bn) * 0.5, (ae + be) * 0.5]
    };
    let ends = if distance(a, b) + distance(c, d) < distance(b, c) + distance(d, a) {
        [mid(a, b), mid(c, d)]
    } else {
        [mid(b, c), mid(d, a)]
    };
    // Reject collapsed, skewed, and self-intersecting outlines rather than guessing a centreline.
    let [[an, ae], [bn, be]] = ends;
    let length = libm::hypot(bn - an, be - ae);
    if length < 50.0 {
        return Err("degenerate runway polygon".into());
    }
    for pair in [(a, b), (b, c), (c, d), (d, a)] {
        let edge = distance(pair.0, pair.1);
        if edge < 1.0 {
            return Err("degenerate runway edge".into());
        }
    }
    Ok(ends)
}
/// Azimuth of a projected line measured against geodetic north at its threshold.
pub(super) fn true_heading(p: &LocalProjection, a: [f64; 2], b: [f64; 2]) -> Result<f64, String> {
    let [an, ae] = a;
    let [bn, be] = b;
    let length = libm::hypot(bn - an, be - ae);
    let [lat, lon] = p.unproject(an, ae).map_err(|e| e.to_string())?;
    let [lat1, lon1] = p
        .unproject(an + (bn - an) / length, ae + (be - ae) / length)
        .map_err(|e| e.to_string())?;
    let phi = lat.to_radians();
    let e2 = (1.0 / 298.257223563) * (2.0 - 1.0 / 298.257223563);
    let ratio = (1.0 - e2) / (1.0 - e2 * libm::sin(phi) * libm::sin(phi));
    let dlon = (lon1 - lon + 180.0).rem_euclid(360.0) - 180.0;
    Ok(libm::atan2(dlon * libm::cos(phi), (lat1 - lat) * ratio)
        .to_degrees()
        .rem_euclid(360.0))
}
pub(super) fn import(
    region: &Region,
    terrain: &World,
    airport_layer: &Value,
    runway_layer: &Value,
) -> Result<Airports, String> {
    let p = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
        .map_err(|e| e.to_string())?;
    let mut airports = BTreeMap::new();
    for f in features(airport_layer)? {
        let props = f.get("properties").ok_or("missing airport properties")?;
        if props.get("TYPE_CODE").and_then(Value::as_str) != Some("AD")
            || props.get("PRIVATEUSE").and_then(Value::as_i64) != Some(0)
        {
            continue;
        }
        let lat = dms(string(props, "LATITUDE")?)?;
        let lon = dms(string(props, "LONGITUDE")?)?;
        let Ok(local) = p.project(lat, lon) else {
            continue;
        };
        if flightcraft_world::inside(region, local).is_err() {
            continue;
        }
        let global = string(props, "GLOBAL_ID")?.to_owned();
        let airport = Airport {
            ident: string(props, "IDENT")?.into(),
            icao: props
                .get("ICAO_ID")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            name: string(props, "NAME")?.into(),
            elevation_m: number(props, "ELEVATION")? * 0.3048,
            latitude_deg: lat,
            longitude_deg: lon,
            runways: Vec::new(),
        };
        if airports.insert(global, airport).is_some() {
            return Err("duplicate airport GLOBAL_ID".into());
        }
    }
    for f in features(runway_layer)? {
        let props = f.get("properties").ok_or("missing runway properties")?;
        let Some(airport) = airports.get_mut(string(props, "AIRPORT_ID")?) else {
            continue;
        };
        let designator = string(props, "DESIGNATOR")?;
        let Some((first, second)) = designator.split_once('/') else {
            continue;
        };
        if !flightcraft_world::end_ident(first) || !flightcraft_world::end_ident(second) {
            continue;
        }
        let surface = match string(props, "COMP_CODE")? {
            "ASPH" | "ASP+GRS" | "ASP+TRTD" => RunwaySurface::Asphalt,
            "CONC" | "CONC+ASPH" | "CONC+GRS" => RunwaySurface::Concrete,
            "TURF" | "GRASS" | "TURF+DIRT" => RunwaySurface::Turf,
            "GRVL" => RunwaySurface::Gravel,
            "WATER" | "WATR" => continue,
            _ => RunwaySurface::Other,
        };
        if string(props, "DIM_UOM")? != "FT" {
            return Err("FAA runway dimensions must use FT".into());
        }
        let length = number(props, "LENGTH")? * 0.3048;
        let width = number(props, "WIDTH")? * 0.3048;
        let mut ends = centreline(f, &p).map_err(|e| format!("{} {designator}: {e}", airport.ident))?;
        let [[an, ae], [bn, be]] = ends;
        let measured = libm::hypot(bn - an, be - ae);
        if (measured / length - 1.0).abs() > 0.05 {
            return Err(format!(
                "{} {designator}: polygon length {measured:.1} m differs from LENGTH {length:.1} m by more than 5%",
                airport.ident
            ));
        }
        let number = first
            .get(..2)
            .ok_or("invalid runway ident")?
            .parse::<f64>()
            .map_err(|e| e.to_string())?;
        let heading = true_heading(&p, ends[0], ends[1])?;
        let delta = (heading - number * 10.0 + 180.0).rem_euclid(360.0) - 180.0;
        // Designators are magnetic and may be old; the opposite direction is unambiguous.
        if delta.abs() > 90.0 {
            ends.swap(0, 1);
        }
        let make_end = |ident: &str, a: [f64; 2], b: [f64; 2]| -> Result<RunwayEnd, String> {
            let [n, e] = a;
            flightcraft_world::inside(region, a).map_err(|e| e.to_string())?;
            let [lat, lon] = p.unproject(n, e).map_err(|e| e.to_string())?;
            Ok(RunwayEnd {
                ident: ident.into(),
                lighting: None,
                latitude_deg: lat,
                longitude_deg: lon,
                elevation_m: terrain
                    .terrain_ground(n, e)
                    .map_or(airport.elevation_m, |g| g.height_msl_m),
                heading_deg: true_heading(&p, a, b)?,
            })
        };
        let [a, b] = ends;
        airport.runways.push(AirportRunway {
            designator: designator.into(),
            length_m: length,
            width_m: width,
            surface,
            ends: [make_end(first, a, b)?, make_end(second, b, a)?],
        });
    }
    let mut airports: Vec<_> = airports.into_values().filter(|a| !a.runways.is_empty()).collect();
    airports.sort_by(|a, b| a.ident.cmp(&b.ident));
    for a in &mut airports {
        a.runways.sort_by(|a, b| a.designator.cmp(&b.designator));
    }
    let output = Airports {
        attribution: ATTRIBUTION.into(),
        airports,
    };
    output.validate(region).map_err(|e| e.to_string())?;
    Ok(output)
}
pub fn run() -> Result<(), String> {
    run_args(std::env::args().skip(2))
}

pub(super) fn run_args(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let (mut region, mut airports, mut runways, mut out) = (None, None, None, None);
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {"--region"=>&mut region,"--airports"=>&mut airports,"--runways"=>&mut runways,"--out"=>&mut out,_=>return Err("usage: navdata-faa --region <region.json> --airports <US_Airport.geojson> --runways <Runways.geojson> --out <airports.json>".into())};
        if slot.is_some() {
            return Err(format!("duplicate {arg}"));
        }
        *slot = Some(args.next().ok_or("missing path")?);
    }
    let region_path = region.ok_or("missing --region")?;
    let region_path = Path::new(&region_path);
    let directory = region_path.parent().ok_or("region needs parent directory")?;
    let bytes = super::terrain_srtm::bounded(region_path, flightcraft_world::MAX_REGION_BYTES)?;
    let mut region = Region::from_json(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let tiles = region
        .tiles
        .iter()
        .map(|name| {
            let bytes = super::terrain_srtm::bounded(
                &directory.join(name),
                flightcraft_world::MAX_REGION_TILE_BYTES,
            )?;
            HeightTile::read(&bytes).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let terrain = World::from_region(region.clone(), tiles).map_err(|e| e.to_string())?;
    let layer = |path: String| -> Result<Value, String> {
        let bytes = super::terrain_srtm::bounded(Path::new(&path), 64 * 1024 * 1024)?;
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    };
    let airports = import(
        &region,
        &terrain,
        &layer(airports.ok_or("missing --airports")?)?,
        &layer(runways.ok_or("missing --runways")?)?,
    )?;
    // An origin already at a catalogue threshold uses the sampled end elevation and FAA dimensions.
    let projection = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
        .map_err(|e| e.to_string())?;
    'origin: for airport in &airports.airports {
        for runway in &airport.runways {
            for end in &runway.ends {
                let [n, e] = projection
                    .project(end.latitude_deg, end.longitude_deg)
                    .map_err(|e| e.to_string())?;
                if libm::hypot(n, e) < 0.001 {
                    region.origin.field_elevation_m = end.elevation_m;
                    region.origin.runway_heading_deg = end.heading_deg;
                    region.origin.runway_length_m = runway.length_m;
                    region.origin.runway_width_m = runway.width_m;
                    break 'origin;
                }
            }
        }
    }
    // Prepare the render grid from the same union of runway planes used by collision queries.
    // All end elevations have already been sampled from the original elevation tiles.
    let prepared = terrain
        .clone()
        .with_airports(airports.clone())
        .map_err(|e| e.to_string())?;
    let mut flattened = Vec::new();
    for tile in terrain.tiles() {
        let [on, oe] = tile.origin_m();
        let mut samples = Vec::new();
        let mut surfaces = Vec::new();
        for j in 0..tile.size() {
            for i in 0..tile.size() {
                let n = on + f64::from(i) * tile.spacing_m();
                let e = oe + f64::from(j) * tile.spacing_m();
                samples.push(prepared.prepared_airport_ground(n, e).height_msl_m as f32);
            }
        }
        for j in 0..tile.size() - 1 {
            for i in 0..tile.size() - 1 {
                let n = on + (f64::from(i) + 0.5) * tile.spacing_m();
                let e = oe + (f64::from(j) + 0.5) * tile.spacing_m();
                // Surface rectangles and markings are rendered separately at their exact widths.
                surfaces.push(
                    terrain
                        .terrain_ground(n, e)
                        .map_or(flightcraft_world::Surface::Grass, |g| g.surface),
                );
            }
        }
        flattened.push(
            HeightTile::new(
                tile.origin_m(),
                tile.spacing_m() as f32,
                tile.size(),
                samples,
                surfaces,
            )
            .map_err(|e| e.to_string())?,
        );
    }
    let out = out.ok_or("missing --out")?;
    let out = Path::new(&out);
    if out != directory.join("airports.json") {
        return Err("output must be airports.json beside region.json".into());
    }
    if out.exists() {
        return Err("refusing to overwrite airports.json".into());
    }
    let bytes = serde_json::to_vec_pretty(&airports).map_err(|e| e.to_string())?;
    Airports::from_json(&bytes, &region).map_err(|e| e.to_string())?;
    for (name, tile) in region.tiles.iter().zip(&flattened) {
        std::fs::write(directory.join(name), tile.write()).map_err(|e| e.to_string())?;
    }
    std::fs::write(out, bytes).map_err(|e| e.to_string())?;
    region.airports = Some("airports.json".into());
    // Publish manifest last, using a same-directory rename.
    let temp = directory.join("region.json.pending");
    let publish = || -> Result<(), String> {
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(&region).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(&temp, region_path).map_err(|e| e.to_string())
    };
    if let Err(e) = publish() {
        let _ = std::fs::remove_file(out);
        let _ = std::fs::remove_file(temp);
        return Err(e);
    }
    eprintln!(
        "xtask: imported {} airports, {} runways",
        airports.airports.len(),
        airports.airports.iter().map(|a| a.runways.len()).sum::<usize>()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn region() -> Region {
        Region::from_json(
            &json!({"id":"fixture","name":"Fixture", "origin":{
            "latitude_deg":0.5,"longitude_deg":0.5,"field_elevation_m":100.0,
            "runway_heading_deg":90.0,"runway_length_m":1000.0,"runway_width_m":20.0},
            "southwest_m":[-2000.0,-2000.0],"tiles_north":2,"tiles_east":2,
            "spacing_m":31.25,"tile_size":65,"runway_blend_m":200.0})
            .to_string(),
        )
        .unwrap()
    }
    #[test]
    fn handwritten_geojson_filters_projects_orients_and_samples_unflattened_terrain() {
        let airports: Value =
            serde_json::from_str(include_str!("../tests/fixtures/faa/airports.geojson")).unwrap();
        let runways: Value =
            serde_json::from_str(include_str!("../tests/fixtures/faa/runways.geojson")).unwrap();
        let r = region();
        let terrain = World::from_tiles(vec![
            HeightTile::new(
                [-2000.0, -2000.0],
                100.0,
                41,
                (0..41)
                    .flat_map(|j| (0..41).map(move |_| 98.0 + j as f32 * 0.1))
                    .collect(),
                vec![flightcraft_world::Surface::Grass; 40 * 40],
            )
            .unwrap(),
        ])
        .unwrap();
        let output = import(&r, &terrain, &airports, &runways).unwrap();
        assert_eq!(output.airports.len(), 1);
        let a = &output.airports[0];
        assert_eq!(a.ident, "T01");
        assert_eq!(a.runways.len(), 1);
        let runway = &a.runways[0];
        assert_eq!(runway.designator, "09/27");
        assert!((runway.length_m - 1001.88).abs() < 0.1);
        let [west, east] = &runway.ends;
        assert!(west.longitude_deg < east.longitude_deg);
        assert!((west.heading_deg - 90.0).abs() < 0.01);
        assert!((east.heading_deg - 270.0).abs() < 0.01);
        assert!((west.elevation_m - 100.0).abs() < 0.01);
        assert!((east.elevation_m - 101.0).abs() < 0.01);
        let mut bad = runways.clone();
        bad["features"][0]["properties"]["LENGTH"] = json!(2000);
        assert!(import(&r, &terrain, &airports, &bad).unwrap_err().contains("5%"));
        bad = runways.clone();
        bad["features"][0]["geometry"]["coordinates"][0] = json!([]);
        assert!(import(&r, &terrain, &airports, &bad).is_err());
        let fallback = import(&r, &World::default(), &airports, &runways).unwrap();
        assert_eq!(
            fallback.airports[0].runways[0].ends[0].elevation_m,
            500.0 * 0.3048
        );
    }
    #[test]
    fn hostile_dms_and_features() {
        assert!((dms("074-59-51.0146W").unwrap() + 74.9975040556).abs() < 1e-9);
        for text in ["", "N", "1-60-0N", "90-0-1N", "1-2-NaNN", "1-2-3X", "é"] {
            assert!(dms(text).is_err(), "{text}");
        }
        assert!(features(&json!({"type":"Point","features":[]})).is_err());
    }
}
