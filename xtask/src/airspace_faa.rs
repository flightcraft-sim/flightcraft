//! Offline FAA GeoJSON conversion; source files are never fetched by this tool.
use flightcraft_world::{
    AirspaceAltitude, AirspaceClass, AirspaceFile, AirspaceKind, AirspaceVolume, AltitudeReference,
    FAA_AIRSPACE_ATTRIBUTION, GeographicPolygon, ProjectedAirspaceVolume, Region, segment_distance,
    srtm::LocalProjection,
};
use serde_json::Value;
use std::{io::Read, path::PathBuf};

pub fn run() -> Result<(), String> {
    run_args(std::env::args().skip(2))
}

pub(super) fn run_args(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let (mut region, mut input, mut output) = (None, None, None);
    let (mut keep_a, mut keep_e5) = (false, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--region" => region = args.next().map(PathBuf::from),
            "--airspace" => input = args.next().map(PathBuf::from),
            "--out" => output = args.next().map(PathBuf::from),
            "--keep-a" => keep_a = true,
            "--keep-e5" => keep_e5 = true,
            _ => return Err(format!("unknown airspace-faa argument: {arg}")),
        }
    }
    let usage = "airspace-faa --region <region.json> --airspace <Class_Airspace.geojson> --out <airspace.json> [--keep-a] [--keep-e5]";
    let mut text = String::new();
    std::fs::File::open(region.ok_or(usage)?)
        .map_err(|e| e.to_string())?
        .take(65537)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 65536 {
        return Err("region too large".into());
    }
    let region: Region = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    text.clear();
    const MAX_INPUT: usize = 256 * 1024 * 1024;
    std::fs::File::open(input.ok_or(usage)?)
        .map_err(|e| e.to_string())?
        .take(MAX_INPUT as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > MAX_INPUT {
        return Err("FAA input too large".into());
    }
    let (file, before, after) = convert(&region, &text, keep_a, keep_e5)?;
    let output = output.ok_or(usage)?;
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    AirspaceFile::from_json(&text).map_err(|e| e.to_string())?;
    // Never replace existing output or create a partial document on a conversion error.
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    target
        .write_all(format!("{text}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    eprintln!(
        "xtask: {} volumes; vertices {before} -> {after} (15 m Douglas–Peucker)",
        file.volumes.len()
    );
    Ok(())
}

pub fn convert(
    region: &Region,
    text: &str,
    keep_a: bool,
    keep_e5: bool,
) -> Result<(AirspaceFile, usize, usize), String> {
    let projection = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
        .map_err(|e| e.to_string())?;
    let span = f64::from(region.tile_size.saturating_sub(1)) * f64::from(region.spacing_m);
    let [sn, se] = region.southwest_m;
    let high = [
        sn + span * f64::from(region.tiles_north),
        se + span * f64::from(region.tiles_east),
    ];
    if !span.is_finite()
        || span <= 0.0
        || region.tiles_north == 0
        || region.tiles_east == 0
        || [sn, se, high[0], high[1]]
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 250000.0)
    {
        return Err("invalid airspace region grid (maximum ±250 km)".into());
    }
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if root["type"] != "FeatureCollection" {
        return Err("expected GeoJSON FeatureCollection".into());
    }
    let features = root["features"].as_array().ok_or("missing features")?;
    if features.len() > 100000 {
        return Err("too many FAA features".into());
    }
    let mut file = AirspaceFile {
        attribution: FAA_AIRSPACE_ATTRIBUTION.into(),
        volumes: Vec::new(),
    };
    let (mut before, mut after) = (0, 0);
    for (index, feature) in features.iter().enumerate() {
        if feature["type"] != "Feature" {
            return Err("expected GeoJSON Feature".into());
        }
        let p = &feature["properties"];
        let local = p["LOCAL_TYPE"].as_str().ok_or("missing LOCAL_TYPE")?;
        let class = match p["CLASS"].as_str() {
            Some("A") if keep_a => Some(AirspaceClass::A),
            Some("B") => Some(AirspaceClass::B),
            Some("C") => Some(AirspaceClass::C),
            Some("D") => Some(AirspaceClass::D),
            Some("E")
                if matches!(local, "CLASS_E2" | "CLASS_E3" | "CLASS_E4")
                    || (keep_e5 && local == "CLASS_E5") =>
            {
                Some(AirspaceClass::E)
            }
            None if p["CLASS"].is_null() && matches!(local, "MODE C" | "MODE-C") => None,
            Some("A" | "E") => continue,
            _ => return Err("unsupported FAA class or local type".into()),
        };
        let kind = if class.is_none() {
            AirspaceKind::ModeC
        } else {
            AirspaceKind::Class
        };
        let floor = altitude(p, "LOWER", false)?;
        let ceiling = altitude(p, "UPPER", true)?;
        let geometry = &feature["geometry"];
        let coordinates = geometry["coordinates"]
            .as_array()
            .ok_or("missing geometry coordinates")?;
        let polygons: Vec<&Value> = match geometry["type"].as_str() {
            Some("Polygon") => vec![&geometry["coordinates"]],
            Some("MultiPolygon") => coordinates.iter().collect(),
            _ => return Err("FAA geometry requires Polygon or MultiPolygon".into()),
        };
        let mut converted = Vec::new();
        for polygon in polygons {
            let mut rings = GeographicPolygon::new();
            for ring in polygon.as_array().ok_or("invalid polygon")? {
                let coordinates = ring.as_array().ok_or("invalid ring")?;
                before += coordinates.len();
                if before > 2_000_000 {
                    return Err("FAA source vertex budget exceeded".into());
                }
                let mut geographic = Vec::new();
                for coordinate in coordinates {
                    let pair = coordinate.as_array().ok_or("invalid coordinate")?;
                    if pair.len() != 2 {
                        return Err("FAA requires two-dimensional coordinates".into());
                    }
                    let lat = pair.get(1).and_then(Value::as_f64).ok_or("invalid latitude")?;
                    let lon = pair.first().and_then(Value::as_f64).ok_or("invalid longitude")?;
                    projection.project(lat, lon).map_err(|e| e.to_string())?;
                    if geographic.last() != Some(&[lat, lon]) {
                        geographic.push([lat, lon]);
                    }
                }
                if geographic.first() != geographic.last()
                    && let Some(first) = geographic.first().copied()
                {
                    geographic.push(first);
                }
                rings.push(simplify(&geographic, &projection)?);
            }
            converted.push(rings);
        }
        let id = p
            .get("OBJECTID")
            .or_else(|| feature.get("id"))
            .filter(|v| v.is_number() || v.is_string())
            .map_or_else(|| index.to_string(), |v| v.to_string().replace('"', ""));
        let volume = AirspaceVolume {
            id: format!("faa-{id}"),
            name: p["NAME"].as_str().ok_or("missing FAA name")?.into(),
            class,
            kind,
            sector: match &p["SECTOR"] {
                Value::Null => None,
                Value::String(s) if !s.is_empty() => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => return Err("invalid FAA sector".into()),
            },
            floor,
            ceiling,
            polygons: converted,
        };
        let checked = AirspaceFile {
            attribution: FAA_AIRSPACE_ATTRIBUTION.into(),
            volumes: vec![volume],
        }
        .project(projection)
        .map_err(|e| e.to_string())?;
        let Some(projected) = checked.volumes.into_iter().next() else {
            return Err("missing converted volume".into());
        };
        if intersects(&projected, [sn, se], high) {
            after += projected
                .info
                .polygons
                .iter()
                .flatten()
                .map(Vec::len)
                .sum::<usize>();
            file.volumes.push(projected.info);
        }
    }
    file.validate().map_err(|e| e.to_string())?;
    Ok((file, before, after))
}
fn altitude(p: &Value, prefix: &str, upper: bool) -> Result<AirspaceAltitude, String> {
    let value = &p[format!("{prefix}_VAL")];
    let value = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .ok_or("invalid FAA altitude")?;
    if upper && (-9999.0..=-9990.0).contains(&value) {
        return Ok(AirspaceAltitude {
            value_ft: 18000.0,
            reference: AltitudeReference::STD,
        });
    }
    let code = p[format!("{prefix}_CODE")]
        .as_str()
        .ok_or("missing FAA altitude reference")?;
    let unit = p[format!("{prefix}_UOM")]
        .as_str()
        .ok_or("missing FAA altitude unit")?;
    let reference = match code {
        "MSL" => AltitudeReference::MSL,
        "SFC" => AltitudeReference::SFC,
        "STD" => AltitudeReference::STD,
        _ => return Err("unsupported FAA altitude reference".into()),
    };
    let value_ft = match unit {
        "FT" => value,
        "FL" if reference == AltitudeReference::STD => value * 100.0,
        _ => return Err("unsupported FAA altitude unit".into()),
    };
    Ok(AirspaceAltitude { value_ft, reference })
}
/// Iterative Douglas–Peucker on two arcs of a closed ring avoids recursive stack growth.
fn simplify(ring: &[[f64; 2]], projection: &LocalProjection) -> Result<Vec<[f64; 2]>, String> {
    if ring.len() < 4 {
        return Err("FAA ring needs three distinct vertices".into());
    }
    let points = ring
        .iter()
        .map(|[a, b]| projection.project(*a, *b).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let first = *points.first().ok_or("empty ring")?;
    let split = points
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            segment_distance(**a, first, first).total_cmp(&segment_distance(**b, first, first))
        })
        .map(|(i, _)| i)
        .ok_or("empty ring")?;
    let end = points.len() - 1;
    let mut keep = vec![false; points.len()];
    let mut stack = vec![(0, split), (split, end)];
    while let Some((a, b)) = stack.pop() {
        if let Some(v) = keep.get_mut(a) {
            *v = true;
        }
        if let Some(v) = keep.get_mut(b) {
            *v = true;
        }
        let pa = *points.get(a).ok_or("ring start")?;
        let pb = *points.get(b).ok_or("ring end")?;
        let mut farthest = (15.0, None);
        for (i, p) in points.iter().enumerate().take(b).skip(a + 1) {
            let distance = segment_distance(*p, pa, pb);
            if distance > farthest.0 {
                farthest = (distance, Some(i));
            }
        }
        if let Some(i) = farthest.1 {
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    let simplified: Vec<_> = ring
        .iter()
        .zip(keep)
        .filter_map(|(p, keep)| keep.then_some(*p))
        .collect();
    // Small polygons remain intact rather than disappearing under the tolerance.
    Ok(if simplified.len() < 4 {
        ring.to_vec()
    } else {
        simplified
    })
}
fn intersects(v: &ProjectedAirspaceVolume, low: [f64; 2], high: [f64; 2]) -> bool {
    let corners = [low, [low[0], high[1]], high, [high[0], low[1]], low];
    corners.iter().any(|p| v.contains_lateral(*p))
        || v.polygons.iter().flatten().any(|ring| {
            ring.windows(2).any(|w| {
                let [a, b] = w else {
                    return false;
                };
                if (low[0]..=high[0]).contains(&a[0]) && (low[1]..=high[1]).contains(&a[1]) {
                    return true;
                }
                // Liang–Barsky segment clipping against the grid rectangle.
                let (mut enter, mut exit) = (0.0_f64, 1.0_f64);
                for axis in 0..2 {
                    let delta = b[axis] - a[axis];
                    if delta == 0.0 {
                        if a[axis] < low[axis] || a[axis] > high[axis] {
                            return false;
                        }
                    } else {
                        let x = (low[axis] - a[axis]) / delta;
                        let y = (high[axis] - a[axis]) / delta;
                        enter = enter.max(x.min(y));
                        exit = exit.min(x.max(y));
                    }
                }
                enter <= exit
            })
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn handwritten_faa_geometry_altitudes_filtering_and_simplification() {
        let region: Region = serde_json::from_str(r#"{"id":"test", "name":"Test", "origin":{"latitude_deg":0,"longitude_deg":0,"field_elevation_m":0,"runway_heading_deg":0,"runway_length_m":900,"runway_width_m":23},"southwest_m":[-6000,-6000],"tiles_north":6,"tiles_east":6,"spacing_m":31.25,"tile_size":65,"runway_blend_m":200}"#).unwrap();
        let text = include_str!("../tests/fixtures/airspace.geojson");
        let (file, before, after) = convert(&region, text, false, false).unwrap();
        assert_eq!(file.volumes.len(), 4);
        assert!(after < before);
        assert_eq!(file.volumes[0].sector.as_deref(), Some("AREA A"));
        assert_eq!(file.volumes[0].polygons[0][0].len(), 5);
        assert_eq!(file.volumes[0].polygons[0].len(), 2);
        assert_eq!(file.volumes[1].polygons.len(), 2);
        assert_eq!(file.volumes[2].ceiling.value_ft, 18000.0);
        assert_eq!(file.volumes[2].ceiling.reference, AltitudeReference::STD);
        assert_eq!(file.volumes[3].kind, AirspaceKind::ModeC);
        let all = convert(&region, text, true, true).unwrap().0;
        assert_eq!(all.volumes.len(), 6);
        assert_eq!(all.volumes[4].ceiling.value_ft, 60000.0);
        let mut malformed: Value = serde_json::from_str(text).unwrap();
        malformed["features"][0]["properties"]["LOWER_UOM"] = "M".into();
        assert!(convert(&region, &malformed.to_string(), false, false).is_err());
        malformed["features"][0]["properties"]["LOWER_UOM"] = "FT".into();
        malformed["features"][0]["geometry"]["coordinates"][0][0] = serde_json::json!([181, 0]);
        assert!(convert(&region, &malformed.to_string(), false, false).is_err());
        assert!(convert(&region, "{}", false, false).is_err());
    }
}
