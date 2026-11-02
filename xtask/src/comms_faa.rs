//! Bounded, offline FAA frequency CSV adapter. Unsupported use labels fail explicitly.
use flightcraft_world::{Airports, ComStation, FacilityKind, GeoPoint, Navdata};
use std::{collections::BTreeMap, io::Read, path::Path};
fn read(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 16 * 1024 * 1024 {
        return Err("frequency input exceeds 16 MiB".into());
    }
    Ok(text)
}
/// RFC-style quoted fields, escaped quotes, CRLF, and quoted commas; bounded before parsing.
pub(super) fn csv(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut records = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut closed = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' if field.is_empty() && !closed => quoted = true,
                ',' | '\n' => {
                    row.push(std::mem::take(&mut field));
                    closed = false;
                    if c == '\n' {
                        if row.iter().any(|f| !f.is_empty()) {
                            records.push(std::mem::take(&mut row));
                        } else {
                            row.clear();
                        }
                    }
                }
                '\r' => {}
                _ if closed => return Err("unexpected text after closing CSV quote".into()),
                '"' => return Err("quote inside unquoted CSV field".into()),
                _ => field.push(c),
            }
        }
        if field.len() > 4096 || row.len() > 256 || records.len() > 100000 {
            return Err("CSV field/row/count budget".into());
        }
    }
    if quoted {
        return Err("unterminated CSV quote".into());
    }
    row.push(field);
    if row.iter().any(|f| !f.is_empty()) {
        records.push(row);
    }
    Ok(records)
}
pub fn convert(text: &str, airports: &Airports, source: &str) -> Result<Vec<ComStation>, String> {
    if text.len() > 16 * 1024 * 1024 || source.is_empty() || source.len() > 1800 {
        return Err("invalid input/source bounds".into());
    }
    let records = csv(text)?;
    let header = records.first().ok_or("missing frequency CSV header")?;
    let index = header
        .iter()
        .enumerate()
        .map(|(i, k)| (k.trim_start_matches('\u{feff}').trim().to_uppercase(), i))
        .collect::<BTreeMap<_, _>>();
    for key in ["ARPT_ID", "FREQ", "FREQ_USE"] {
        if !index.contains_key(key) {
            return Err(format!("missing CSV header {key}"));
        }
    }
    let mut stations = Vec::new();
    for (row_number, row) in records.iter().enumerate().skip(1) {
        if row.len() != header.len() {
            return Err(format!("CSV row {row_number}: column count"));
        }
        let get = |key: &str| {
            index
                .get(key)
                .and_then(|i| row.get(*i))
                .map(|s| s.trim())
                .unwrap_or("")
        };
        let airport_id = get("ARPT_ID");
        let airport = airports
            .airports
            .iter()
            .find(|a| a.ident == airport_id || a.icao.as_deref() == Some(airport_id));
        // National records may carry coordinates directly, without terrain or an airport shard.
        let position = if !get("LAT_DECIMAL").is_empty() && !get("LONG_DECIMAL").is_empty() {
            GeoPoint {
                latitude_deg: get("LAT_DECIMAL").parse().map_err(|_| "invalid latitude")?,
                longitude_deg: get("LONG_DECIMAL").parse().map_err(|_| "invalid longitude")?,
            }
        } else if let Some(a) = airport {
            GeoPoint {
                latitude_deg: a.latitude_deg,
                longitude_deg: a.longitude_deg,
            }
        } else {
            continue;
        };
        let mhz = get("FREQ")
            .parse::<f64>()
            .map_err(|_| format!("invalid frequency in row {row_number}"))?;
        let khz = mhz * 1000.0;
        if !khz.is_finite() || khz < 0.0 || (khz - khz.round()).abs() > 1e-5 {
            return Err("frequency needs a MHz channel designator".into());
        }
        let kind = match get("FREQ_USE").to_uppercase().as_str() {
            "CTAF" => FacilityKind::Ctaf,
            "UNICOM" => FacilityKind::Unicom,
            "AWOS" | "AWOS-3" | "AWOS-III" => FacilityKind::Awos,
            "ASOS" => FacilityKind::Asos,
            "ATIS" | "D-ATIS" => FacilityKind::Atis,
            "TWR" | "TOWER" | "LCL" => FacilityKind::Tower,
            "GND" | "GROUND" => FacilityKind::Ground,
            "APP" | "APPROACH" => FacilityKind::Approach,
            "DEP" | "DEPARTURE" => FacilityKind::Departure,
            "CLNC DEL" | "CLEARANCE" => FacilityKind::Clearance,
            other => {
                return Err(format!(
                    "unsupported FAA frequency use {other:?} in row {row_number}; normalize explicitly"
                ));
            }
        };
        let name = if !get("CALL_SIGN").is_empty() {
            get("CALL_SIGN").to_owned()
        } else {
            airport
                .map(|a| a.name.clone())
                .ok_or("national record needs CALL_SIGN")?
        };
        let elevation_m = if !get("ELEVATION_M").is_empty() {
            get("ELEVATION_M").parse().map_err(|_| "invalid elevation")?
        } else {
            airport
                .map(|a| a.elevation_m)
                .ok_or("national record needs ELEVATION_M")?
        };
        let range_nm = if matches!(kind, FacilityKind::Approach | FacilityKind::Departure) {
            80.0
        } else {
            25.0
        };
        let station = ComStation {
            metadata: None,
            id: format!("faa-com-{airport_id}-{row_number}"),
            airport: airport_id.into(),
            name,
            kind,
            channel_khz: khz.round() as u32,
            position,
            elevation_m,
            antenna_agl_m: 10.0,
            range_nm,
            source: format!("{source}#row={}", row_number + 1),
        };
        station.validate().map_err(|e| e.to_string())?;
        stations.push(station);
        if stations.len() > 20000 {
            return Err("station count exceeds 20000".into());
        }
    }
    Ok(stations)
}
pub fn run() -> Result<(), String> {
    let args = std::env::args().skip(2).collect::<Vec<_>>();
    let mut options = BTreeMap::new();
    for pair in args.chunks(2) {
        let [key, value] = pair else {
            return Err("frequency options need values".into());
        };
        if !["--input", "--airports", "--source", "--navdata", "--out"].contains(&key.as_str())
            || options.insert(key.as_str(), value.as_str()).is_some()
        {
            return Err("unknown or duplicate frequency option".into());
        }
    }
    let required = |key| options.get(key).copied().ok_or_else(|| format!("missing {key}"));
    let airports = if let Some(path) = options.get("--airports") {
        serde_json::from_str::<Airports>(&read(Path::new(path))?).map_err(|e| e.to_string())?
    } else {
        Airports {
            attribution: "FAA frequency coordinates".into(),
            airports: Vec::new(),
        }
    };
    let stations = convert(
        &read(Path::new(required("--input")?))?,
        &airports,
        required("--source")?,
    )?;
    let value = if let Some(path) = options.get("--navdata") {
        let mut data = Navdata::from_json(&read(Path::new(path))?).map_err(|e| e.to_string())?;
        data.comms = stations;
        data.validate().map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?
    } else {
        serde_json::to_string_pretty(&serde_json::json!({"stations":stations})).map_err(|e| e.to_string())?
    };
    use std::io::Write;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(required("--out")?)
        .map_err(|e| e.to_string())?;
    output.write_all(value.as_bytes()).map_err(|e| e.to_string())
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    #[test]
    fn national_import_needs_no_terrain_and_retains_source() {
        let airports = Airports {
            attribution: "Fixture".into(),
            airports: Vec::new(),
        };
        let csv = "ARPT_ID,FREQ,FREQ_USE,CALL_SIGN,LAT_DECIMAL,LONG_DECIMAL,ELEVATION_M\r\nTEST,123.000,AWOS,\"Fixture, Weather\",40,-75,100\r\n";
        let s = convert(csv, &airports, "FAA fixture").unwrap();
        assert_eq!(s[0].channel_khz, 123000);
        assert_eq!(s[0].kind, FacilityKind::Awos);
        assert!(s[0].source.ends_with("row=2"));
        assert!(convert(&csv.replace("123.000", "123.020"), &airports, "FAA fixture").is_err());
        assert!(convert(&csv.replace("AWOS", "UNRECOGNIZED"), &airports, "FAA fixture").is_err());
        assert!(super::csv("x,y\n\"unterminated,y").is_err());
        assert!(super::csv("x,y\n\"closed\"junk,y").is_err());
    }
}
