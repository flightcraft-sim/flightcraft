//! Original FAA ARINC 424 fixed-column reader. See docs/procedures.md for supported columns.
use flightcraft_world::{GeoPoint, Procedures, Region, procedures::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};
fn field(s: &str, a: usize, b: usize) -> &str {
    s.get(a.saturating_sub(1)..b).unwrap_or("").trim()
}
fn numeric(s: &str) -> Result<Option<f64>, String> {
    if s.is_empty() {
        return Ok(None);
    }
    let v = s
        .parse::<f64>()
        .map_err(|_| format!("invalid numeric field {s:?}"))?;
    if !v.is_finite() {
        return Err("nonfinite CIFP number".into());
    }
    Ok(Some(v))
}
fn scaled(s: &str, scale: f64) -> Result<Option<f64>, String> {
    Ok(numeric(s)?.map(|n| n * scale))
}
fn required(s: &str) -> Result<f64, String> {
    numeric(s)?.ok_or_else(|| "missing numeric field".into())
}
fn altitude(s: &str) -> Result<Option<f64>, String> {
    if let Some(fl) = s.strip_prefix("FL") {
        scaled(fl, 30.48)
    } else {
        scaled(s, 0.3048)
    }
}
fn coordinate(s: &str) -> Result<f64, String> {
    let (sign, digits, width, max) = match s.get(..1) {
        Some("N") => (1., s.get(1..).unwrap_or(""), 2, 90.),
        Some("S") => (-1., s.get(1..).unwrap_or(""), 2, 90.),
        Some("E") => (1., s.get(1..).unwrap_or(""), 3, 180.),
        Some("W") => (-1., s.get(1..).unwrap_or(""), 3, 180.),
        _ => return Err("invalid CIFP hemisphere".into()),
    };
    if !digits.bytes().all(|b| b.is_ascii_digit()) || ![width + 6, width + 8].contains(&digits.len()) {
        return Err("invalid CIFP coordinate length".into());
    }
    let degrees = required(digits.get(..width).ok_or("coordinate degrees")?)?;
    let minutes = required(digits.get(width..width + 2).ok_or("coordinate minutes")?)?;
    let seconds = required(digits.get(width + 2..).ok_or("coordinate seconds")?)?
        / if digits.len() == width + 6 { 100. } else { 10000. };
    let value = degrees + minutes / 60. + seconds / 3600.;
    if minutes >= 60. || seconds >= 60. || value > max {
        return Err("CIFP coordinate outside bounds".into());
    }
    Ok(sign * value)
}
fn position(s: &str, lat: (usize, usize), lon: (usize, usize)) -> Result<GeoPoint, String> {
    Ok(GeoPoint {
        latitude_deg: coordinate(field(s, lat.0, lat.1))?,
        longitude_deg: coordinate(field(s, lon.0, lon.1))?,
    })
}
fn variation(s: &str) -> Result<Option<f64>, String> {
    if s.is_empty() {
        return Ok(None);
    }
    let sign = match s.get(..1) {
        Some("E") => 1.,
        Some("W") => -1.,
        _ => return Err("invalid magnetic variation".into()),
    };
    scaled(s.get(1..).unwrap_or(""), sign * 0.1)
}
fn key(airport: &str, icao: &str, section: &str, ident: &str) -> String {
    format!("{airport}/{icao}/{section}/{ident}")
}
fn reference(s: &str, airport: &str, a: usize, b: usize, icao: usize, section: usize) -> Option<String> {
    let ident = field(s, a, b);
    if ident.is_empty() {
        return None;
    }
    let sec = field(s, section, section + 1);
    Some(key(
        if sec.starts_with('P') { airport } else { "" },
        field(s, icao, icao + 1),
        sec,
        ident,
    ))
}
fn parse_leg(s: &str, airport: &str) -> Result<ProcedureLeg, String> {
    let path: PathTerminator = serde_json::from_value(serde_json::Value::String(field(s, 48, 49).into()))
        .map_err(|_| format!("unknown path terminator {}", field(s, 48, 49)))?;
    let distance = field(s, 75, 78);
    let timed = distance.starts_with('T');
    let course = field(s, 71, 74);
    Ok(ProcedureLeg {
        sequence: field(s, 27, 29).parse().map_err(|_| "invalid leg sequence")?,
        fix: reference(s, airport, 30, 34, 35, 37),
        waypoint_description: s.get(39..43).unwrap_or("").into(),
        path,
        turn: match field(s, 44, 44) {
            "" => None,
            "L" => Some(Turn::L),
            "R" => Some(Turn::R),
            _ => return Err("invalid turn".into()),
        },
        recommended_navaid: reference(s, airport, 51, 54, 55, 79),
        theta_deg: scaled(field(s, 63, 66), 0.1)?,
        rho_nm: scaled(field(s, 67, 70), 0.1)?,
        course_deg: scaled(
            course.strip_suffix('T').unwrap_or(course),
            if course.ends_with('T') { 1. } else { 0.1 },
        )?,
        course_true: course.ends_with('T'),
        distance_nm: if timed { None } else { scaled(distance, 0.1)? },
        time_s: if timed {
            scaled(distance.get(1..).unwrap_or(""), 6.)?
        } else {
            None
        },
        altitude_descriptor: field(s, 83, 83).into(),
        altitude1_m: altitude(field(s, 85, 89))?,
        altitude2_m: altitude(field(s, 90, 94))?,
        speed_limit_kt: numeric(field(s, 100, 102))?,
        vertical_angle_deg: scaled(field(s, 103, 106), 0.01)?,
        arc_radius_nm: if path == PathTerminator::RF {
            scaled(field(s, 57, 62), 0.001)?
        } else {
            None
        },
        center_fix: if matches!(path, PathTerminator::RF | PathTerminator::AF) {
            reference(s, airport, 107, 111, 113, 115)
        } else {
            None
        },
        missed: false,
    })
}
fn load(path: &Path, max: usize) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("input must be a regular file".into());
    }
    let mut text = String::new();
    file.take(max as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > max {
        return Err("input exceeds size bound".into());
    }
    Ok(text)
}
fn runway(ident: &str) -> Option<String> {
    let rest = ident.get(1..)?;
    let number = rest.get(..2)?;
    if rest
        .get(2..3)
        .is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let suffix = rest
        .get(2..3)
        .filter(|s| matches!(*s, "L" | "R" | "C"))
        .unwrap_or("");
    Some(format!("RW{number}{suffix}"))
}
pub fn import(text: &str, airports: &BTreeSet<String>) -> Result<Procedures, String> {
    if text.len() > 128 * 1024 * 1024 || !text.is_ascii() {
        return Err("CIFP must be bounded ASCII".into());
    }
    if !text
        .lines()
        .take(10)
        .any(|l| l.contains("2611") && l.contains("29 OCT 2026"))
    {
        return Err("expected FAA cycle 2611, effective 2026-10-29".into());
    }
    let mut points = BTreeMap::new();
    let mut localizers = Vec::new();
    let mut sbas = Vec::new();
    let mut procedures: BTreeMap<(String, String), Procedure> = BTreeMap::new();
    for (line_no, line) in text.lines().enumerate() {
        if !line.starts_with('S') {
            continue;
        }
        if line.len() != 132 {
            return Err(format!("record {} is not 132 columns", line_no + 1));
        }
        let airport = field(line, 7, 10);
        let section = field(line, 5, 5);
        let sub = field(line, 13, 13);
        let terminal = section == "P" && airports.contains(airport);
        let enroute = section == "D" || (section == "E" && field(line, 6, 6) == "A");
        if !terminal && !enroute {
            continue;
        }
        let result = (|| -> Result<(), String> {
            if enroute || (terminal && matches!(sub, "A" | "C" | "G")) {
                // Continuation records do not contain primary coordinates.
                if !matches!(field(line, 22, 22), "0" | "1" | "") {
                    return Ok(());
                }
                let (scope, sec, ident) = if enroute {
                    (
                        "",
                        if section == "D" {
                            if field(line, 6, 6) == "B" { "DB" } else { "D" }
                        } else {
                            "EA"
                        },
                        field(line, 14, 18),
                    )
                } else {
                    (
                        airport,
                        match sub {
                            "A" => "PA",
                            "C" => "PC",
                            _ => "PG",
                        },
                        if sub == "A" { airport } else { field(line, 14, 18) },
                    )
                };
                if !matches!(field(line, 33, 33), "N" | "S") {
                    return Ok(());
                }
                let id = key(
                    scope,
                    if enroute || sec == "PC" {
                        field(line, 20, 21)
                    } else {
                        field(line, 11, 12)
                    },
                    sec,
                    ident,
                );
                let elevation_m = match sec {
                    "PA" => altitude(field(line, 57, 61))?,
                    "PG" => altitude(field(line, 67, 71))?,
                    _ => None,
                };
                let magnetic_variation_deg = match sec {
                    "PA" => variation(field(line, 52, 56))?,
                    "D" | "DB" => variation(field(line, 75, 79))?,
                    "PC" | "EA" => variation(field(line, 75, 79))?,
                    _ => None,
                };
                let p = ProcedurePoint {
                    id: id.clone(),
                    ident: ident.into(),
                    airport: if scope.is_empty() {
                        None
                    } else {
                        Some(scope.into())
                    },
                    section: sec.into(),
                    position: position(line, (33, 41), (42, 51))?,
                    elevation_m,
                    magnetic_variation_deg,
                };
                // Identical duplicated primary records are harmless; inconsistent coordinates are errors.
                if let Some(old) = points.insert(id, p.clone())
                    && old.position != p.position
                {
                    return Err("conflicting CIFP point".into());
                }
            } else if terminal && sub == "I" && field(line, 22, 22) == "0" {
                let gs = if field(line, 56, 56).is_empty() {
                    None
                } else {
                    Some(position(line, (56, 64), (65, 74))?)
                };
                localizers.push(Localizer {
                    airport: airport.into(),
                    ident: field(line, 14, 17).into(),
                    runway: field(line, 28, 32).into(),
                    frequency_hz: required(field(line, 23, 27))? * 10000.,
                    position: position(line, (33, 41), (42, 51))?,
                    course_magnetic_deg: required(field(line, 52, 55))? * 0.1,
                    magnetic_variation_deg: variation(field(line, 91, 95))?.unwrap_or(0.),
                    course_width_deg: scaled(field(line, 84, 87), 0.01)?,
                    gs_position: gs,
                    gs_angle_deg: scaled(field(line, 88, 90), 0.01)?,
                    gs_elevation_m: altitude(field(line, 98, 102))?,
                    threshold_crossing_height_m: scaled(field(line, 96, 97), 0.3048)?,
                });
                let id = key(airport, field(line, 11, 12), "PI", field(line, 14, 17));
                points.insert(
                    id.clone(),
                    ProcedurePoint {
                        id,
                        ident: field(line, 14, 17).into(),
                        airport: Some(airport.into()),
                        section: "PI".into(),
                        position: position(line, (33, 41), (42, 51))?,
                        elevation_m: None,
                        magnetic_variation_deg: variation(field(line, 91, 95))?,
                    },
                );
            } else if terminal && sub == "P" && field(line, 28, 28).is_empty() && field(line, 37, 37) == "0" {
                sbas.push(SbasPathPoint {
                    airport: airport.into(),
                    approach: field(line, 14, 19).into(),
                    runway: field(line, 20, 24).into(),
                    threshold: position(line, (38, 48), (49, 60))?,
                    flight_path_alignment: position(line, (71, 81), (82, 93))?,
                    angle_deg: required(field(line, 67, 70))? * 0.01,
                    payload: line.into(),
                });
            } else if terminal && matches!(sub, "D" | "E" | "F") {
                if field(line, 39, 39) == "2" && field(line, 40, 40) == "W" {
                    if let Some(p) = procedures.get_mut(&(airport.into(), field(line, 14, 19).into())) {
                        for (column, capability) in [
                            (41, ApproachCapability::LPV),
                            (52, ApproachCapability::LnavVnav),
                            (63, ApproachCapability::LNAV),
                        ] {
                            if field(line, column, column) == "A" && !p.capabilities.contains(&capability) {
                                p.capabilities.push(capability);
                            }
                        }
                    }
                    return Ok(());
                }
                // Primary application record (0 or 1); extension records are not legs.
                if !matches!(field(line, 39, 39), "0" | "1") {
                    return Ok(());
                }
                let ident = field(line, 14, 19);
                let route_type = field(line, 20, 20);
                let transition = field(line, 21, 25);
                let p = procedures
                    .entry((airport.into(), ident.into()))
                    .or_insert_with(|| Procedure {
                        airport: airport.into(),
                        ident: ident.into(),
                        kind: match sub {
                            "D" => ProcedureKind::Departure,
                            "E" => ProcedureKind::Arrival,
                            _ => ProcedureKind::Approach,
                        },
                        runway: if sub == "F" { runway(ident) } else { None },
                        plate: None,
                        capabilities: Vec::new(),
                        routes: Vec::new(),
                    });
                let index = p
                    .routes
                    .iter()
                    .position(|r| r.route_type == route_type && r.transition == transition)
                    .unwrap_or_else(|| {
                        p.routes.push(ProcedureRoute {
                            route_type: route_type.into(),
                            transition: transition.into(),
                            legs: Vec::new(),
                        });
                        p.routes.len().saturating_sub(1)
                    });
                let r = p.routes.get_mut(index).ok_or("route allocation")?;
                r.legs.push(parse_leg(line, airport)?);
            }
            Ok(())
        })();
        result.map_err(|e| format!("CIFP record {}: {e}", line_no + 1))?;
    }
    let mut used = BTreeSet::new();
    for p in procedures.values_mut() {
        let p_airport = p.airport.clone();
        for r in &mut p.routes {
            r.legs.sort_by_key(|l| l.sequence);
            let mut missed = false;
            for l in &mut r.legs {
                l.missed = missed || l.waypoint_description.starts_with("  M");
                // Fourth description column M identifies the MAP itself; following legs are missed.
                missed = l.missed || l.waypoint_description.ends_with('M');
                for id in [&mut l.fix, &mut l.recommended_navaid, &mut l.center_fix]
                    .into_iter()
                    .flatten()
                {
                    if !points.contains_key(id) {
                        // Recommended navaids may omit their section code. Resolve only an unambiguous ident.
                        let ident = id.rsplit('/').next().unwrap_or("");
                        let mut matches = points.values().filter(|p| {
                            p.ident == ident
                                && (p.airport.is_none() || p.airport.as_deref() == Some(p_airport.as_str()))
                        });
                        let first = matches.next();
                        if let Some(point) = first.filter(|_| matches.next().is_none()) {
                            *id = point.id.clone();
                        } else {
                            return Err(format!("unresolved procedure reference {id}"));
                        }
                    }
                    used.insert(id.clone());
                }
            }
        }
    }
    let points = points
        .into_values()
        .filter(|p| p.airport.is_some() || used.contains(&p.id))
        .collect();
    let data = Procedures {
        version: 1,
        cycle: "2611".into(),
        effective_date: "2026-10-29".into(),
        attribution: PROCEDURES_ATTRIBUTION.into(),
        points,
        localizers,
        sbas,
        procedures: procedures.into_values().collect(),
    };
    data.validate().map_err(|e| e.to_string())?;
    Ok(data)
}
fn plate_matches(p: &Procedure, name: &str) -> bool {
    let title = name.strip_prefix("COPTER ").unwrap_or(name);
    let prefix = match p.ident.get(..1) {
        Some("I") => "ILS",
        Some("L") => "LOC",
        Some("R" | "H") => "RNAV",
        Some("V" | "S") => "VOR",
        Some("N") => "NDB",
        _ => return false,
    };
    if !(title.starts_with(prefix) || prefix == "LOC" && title.starts_with("ILS") && title.contains("LOC")) {
        return false;
    }
    if p.ident
        .get(1..4)
        .is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit()))
    {
        return name.starts_with("COPTER ") && name.ends_with(p.ident.get(1..).unwrap_or(""));
    }
    let runway = p.runway.as_deref().unwrap_or("").trim_start_matches("RW");
    let shared = runway
        .strip_suffix('R')
        .is_some_and(|n| title.contains(&format!("RWY {n}L/R")));
    if !runway.is_empty() && !title.contains(&format!("RWY {runway}")) && !shared {
        return false;
    }
    p.ident
        .split('-')
        .nth(1)
        .is_none_or(|s| title.contains(&format!(" {s} ")) || title.ends_with(&format!("-{s}")))
}
fn attach_plates(data: &mut Procedures, text: &str) -> Result<(), String> {
    if text.len() > 32 * 1024 * 1024 {
        return Err("d-TPP metadata too large".into());
    }
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            nodes_limit: 2_000_000,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    if doc.root_element().attribute("cycle") != Some(data.cycle.as_str()) {
        return Err("d-TPP and CIFP cycles differ".into());
    }
    for airport in doc.descendants().filter(|n| n.has_tag_name("airport_name")) {
        for p in data.procedures.iter_mut().filter(|p| {
            Some(p.airport.as_str()) == airport.attribute("icao_ident")
                || Some(p.airport.as_str()) == airport.attribute("apt_ident")
        }) {
            let tag = |n: roxmltree::Node<'_, '_>, tag: &str| {
                n.children()
                    .find(|n| n.has_tag_name(tag))
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .to_owned()
            };
            let candidates = airport.children().filter(|n| n.has_tag_name("record"));
            for record in candidates {
                let name = tag(record, "chart_name");
                let matches = if p.kind == ProcedureKind::Approach {
                    tag(record, "chart_code") == "IAP" && plate_matches(p, &name)
                } else {
                    tag(record, "faanfd18").split('.').any(|id| id == p.ident)
                };
                if matches {
                    p.plate = Some(PlateLink {
                        name,
                        pdf: tag(record, "pdf_name"),
                    });
                    break;
                }
            }
        }
    }
    data.validate().map_err(|e| e.to_string())
}
pub fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(2);
    let mut options = BTreeMap::new();
    while let Some(k) = args.next() {
        if !["--region", "--cifp", "--out", "--dtpp"].contains(&k.as_str()) || options.contains_key(&k) {
            return Err("usage: cargo xtask procedures-faa --region <region.json> --cifp <FAACIFP18> --out <procedures.json> [--dtpp <metadata.xml>]".into());
        }
        options.insert(k, args.next().ok_or("missing option value")?);
    }
    let region_path = Path::new(options.get("--region").ok_or("missing --region")?);
    let region = Region::from_json(&load(region_path, flightcraft_world::MAX_REGION_BYTES)?)
        .map_err(|e| e.to_string())?;
    let airport_path = region_path
        .parent()
        .ok_or("region directory")?
        .join(region.airports.as_ref().ok_or("region needs airports")?);
    let airports = flightcraft_world::Airports::from_json(
        load(&airport_path, flightcraft_world::MAX_AIRPORT_BYTES)?.as_bytes(),
        &region,
    )
    .map_err(|e| e.to_string())?;
    let ids = airports
        .airports
        .iter()
        .flat_map(|a| std::iter::once(a.ident.clone()).chain(a.icao.clone()))
        .collect();
    let mut data = import(
        &load(
            Path::new(options.get("--cifp").ok_or("missing --cifp")?),
            128 * 1024 * 1024,
        )?,
        &ids,
    )?;
    if let Some(path) = options.get("--dtpp") {
        attach_plates(&mut data, &load(Path::new(path), 32 * 1024 * 1024)?)?;
    }
    let text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    Procedures::from_json(&text).map_err(|e| e.to_string())?;
    let out = Path::new(options.get("--out").ok_or("missing --out")?);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(out, format!("{text}\n")).map_err(|e| e.to_string())?;
    let mut counts = BTreeMap::<_, [usize; 3]>::new();
    for p in &data.procedures {
        let count = counts.entry(&p.airport).or_default();
        match p.kind {
            ProcedureKind::Departure => {
                if let Some(c) = count.get_mut(0) {
                    *c += 1;
                }
            }
            ProcedureKind::Arrival => {
                if let Some(c) = count.get_mut(1) {
                    *c += 1;
                }
            }
            ProcedureKind::Approach => {
                if let Some(c) = count.get_mut(2) {
                    *c += 1;
                }
            }
        }
    }
    for (airport, [sid, star, approach]) in counts {
        eprintln!("{airport}: {sid} departures, {star} arrivals, {approach} approaches");
    }
    eprintln!(
        "{} procedures, {} points, {} localizers, {} SBAS paths",
        data.procedures.len(),
        data.points.len(),
        data.localizers.len(),
        data.sbas.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    /// Hand-written, original fixtures use one-based inclusive columns documented in procedures.md.
    fn record(fields: &[(usize, &str)]) -> String {
        let mut row = vec![b' '; 132];
        for (start, text) in fields {
            for (offset, byte) in text.bytes().enumerate() {
                row[start - 1 + offset] = byte;
            }
        }
        String::from_utf8(row).unwrap()
    }
    fn leg(path: &str) -> String {
        record(&[
            (1, "SUSA"),
            (5, "P"),
            (7, "KTST"),
            (11, "K6"),
            (13, "F"),
            (14, "R09"),
            (20, "R"),
            (27, "010"),
            (30, "TEST"),
            (35, "K6"),
            (37, "PC"),
            (39, "0"),
            (40, "E  F"),
            (44, "R"),
            (48, path),
            (63, "0900"),
            (67, "0050"),
            (71, "0900"),
            (75, "T010"),
            (83, "B"),
            (85, "02000"),
            (90, "03000"),
            (100, "180"),
            (103, "-300"),
        ])
    }
    #[test]
    fn all_path_terminators_and_constraints() {
        for path in [
            "IF", "TF", "CF", "DF", "FA", "FC", "FD", "FM", "CA", "CD", "CI", "CR", "RF", "AF", "VA", "VD",
            "VI", "VM", "VR", "PI", "HA", "HF", "HM",
        ] {
            let l = parse_leg(&leg(path), "KTST").unwrap();
            assert_eq!(format!("{:?}", l.path), path);
            assert_eq!(l.sequence, 10);
            assert_eq!(l.time_s, Some(60.));
            assert_eq!(l.distance_nm, None);
            assert_eq!(l.altitude1_m, Some(609.6));
            assert_eq!(l.altitude2_m, Some(914.4000000000001));
            assert_eq!(l.vertical_angle_deg, Some(-3.));
            assert_eq!(l.turn, Some(Turn::R));
        }
    }
    #[test]
    fn original_columns_resolve_terminal_fixes_and_missed_boundary() {
        let point = record(&[
            (1, "SUSA"),
            (5, "P"),
            (7, "KTST"),
            (11, "K6"),
            (13, "C"),
            (14, "TEST"),
            (20, "K6"),
            (22, "0"),
            (33, "N40000000"),
            (42, "W075000000"),
            (75, "W0120"),
        ]);
        let mut map = leg("TF");
        map.replace_range(39..43, "GY M");
        let mut missed = leg("CA");
        missed.replace_range(26..29, "020");
        missed.replace_range(39..43, "  M ");
        let text = format!("HDR04 CYCLE 2611 EFFECTIVE 29 OCT 2026\n{point}\n{map}\n{missed}\n");
        let data = import(&text, &BTreeSet::from(["KTST".into()])).unwrap();
        assert_eq!(data.points.len(), 1);
        assert!(!data.procedures[0].routes[0].legs[0].missed);
        assert!(data.procedures[0].routes[0].legs[1].missed);
        assert!(
            import(
                &text.replace("N40000000", "N40600000"),
                &BTreeSet::from(["KTST".into()])
            )
            .is_err()
        );
        assert!(
            import(
                &text.replace("TEST K6PC", "LOST K6PC"),
                &BTreeSet::from(["KTST".into()])
            )
            .is_err()
        );
    }
    #[test]
    fn malformed_and_short_fields_are_errors() {
        assert!(parse_leg("short", "KTST").is_err());
        assert!(parse_leg(&leg("XX"), "KTST").is_err());
        assert!(coordinate("N90000001").is_err());
        assert!(coordinate("E180000001").is_err());
        assert!(coordinate("N40600000").is_err());
        assert_eq!(
            coordinate("S12345678").unwrap(),
            -(12. + 34. / 60. + 56.78 / 3600.)
        );
    }
    #[test]
    fn plate_cycle_and_path_validation() {
        let mut data = Procedures {
            version: 1,
            cycle: "2611".into(),
            effective_date: "2026-10-29".into(),
            attribution: PROCEDURES_ATTRIBUTION.into(),
            points: vec![],
            localizers: vec![],
            sbas: vec![],
            procedures: vec![Procedure {
                airport: "KTST".into(),
                ident: "R09".into(),
                kind: ProcedureKind::Approach,
                runway: Some("RW09".into()),
                plate: None,
                capabilities: Vec::new(),
                routes: vec![ProcedureRoute {
                    route_type: "R".into(),
                    transition: String::new(),
                    legs: vec![ProcedureLeg {
                        fix: None,
                        ..parse_leg(&leg("CA"), "KTST").unwrap()
                    }],
                }],
            }],
        };
        attach_plates(&mut data,r#"<digital_tpp cycle="2611"><airport_name apt_ident="TST" icao_ident="KTST"><record><chart_code>IAP</chart_code><chart_name>RNAV (GPS) RWY 09</chart_name><pdf_name>TEST09.PDF</pdf_name></record></airport_name></digital_tpp>"#).unwrap();
        assert_eq!(data.procedures[0].plate.as_ref().unwrap().pdf, "TEST09.PDF");
        assert!(attach_plates(&mut data, "<digital_tpp cycle=\"2610\"/>").is_err());
        data.procedures[0].plate.as_mut().unwrap().pdf = "../bad.PDF".into();
        assert!(data.validate().is_err());
    }
    #[test]
    fn plate_titles_cover_circling_copter_and_shared_runways() {
        let mut p = Procedure {
            airport: "KTST".into(),
            ident: "RNV-A".into(),
            kind: ProcedureKind::Approach,
            runway: None,
            plate: None,
            capabilities: vec![],
            routes: vec![],
        };
        assert!(plate_matches(&p, "RNAV (GPS)-A"));
        assert!(!plate_matches(&p, "RNAV (GPS)-B"));
        p.ident = "R027".into();
        assert_eq!(runway(&p.ident), None);
        assert!(plate_matches(&p, "COPTER RNAV (GPS) 027"));
        assert!(!plate_matches(&p, "RNAV (GPS) RWY 02"));
        p.ident = "S13R".into();
        p.runway = Some("RW13R".into());
        assert!(plate_matches(&p, "VOR OR GPS RWY 13L/R"));
        assert!(!plate_matches(&p, "VOR RWY 31L"));
        p.ident = "I06-Y".into();
        p.runway = Some("RW06".into());
        assert!(plate_matches(&p, "COPTER ILS Y OR LOC Y RWY 06"));
    }
}
