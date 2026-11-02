//! Pinned NASR 28-day CSV ingestion. FRQ publishes channels; AWOS and ATC enrich their metadata.
use super::download::{self, Cache, Http, MAX_DOWNLOAD, Source, Transport};
use flightcraft_world::comms::ComMetadata;
use flightcraft_world::{ComStation, FacilityKind, GeoPoint, Navdata};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    path::Path,
};

pub const DISCOVERY: &str = "https://external-api.faa.gov/apra/nasr/chart?edition=current";
const TABLES: &[&str] = &[
    "APT_BASE.csv",
    "FRQ.csv",
    "AWOS.csv",
    "ATC_BASE.csv",
    "ATC_ATIS.csv",
    "ATC_SVC.csv",
];
type Row = BTreeMap<String, String>;
fn table(text: &str) -> Result<Vec<Row>, String> {
    let mut rows = super::comms_faa::csv(text.trim_start_matches('\u{feff}'))?.into_iter();
    let header = rows.next().ok_or("missing NASR header")?;
    let mut result = Vec::new();
    for row in rows {
        if row.len() != header.len() {
            return Err("NASR column count differs from layout".into());
        }
        result.push(header.iter().cloned().zip(row).collect());
    }
    Ok(result)
}
fn get<'a>(row: &'a Row, key: &str) -> &'a str {
    row.get(key).map_or("", |v| v.trim())
}
fn number(row: &Row, key: &str) -> Option<f64> {
    get(row, key).parse::<f64>().ok().filter(|v| v.is_finite())
}
fn point(row: &Row) -> Option<GeoPoint> {
    let p = GeoPoint {
        latitude_deg: number(row, "LAT_DECIMAL")?,
        longitude_deg: number(row, "LONG_DECIMAL")?,
    };
    p.validate().ok()?;
    Some(p)
}
fn kinds(value: &str) -> Vec<FacilityKind> {
    use FacilityKind::*;
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    let mut kinds = Vec::new();
    for (matches, kind) in [
        (tokens.contains(&"CTAF"), Ctaf),
        (tokens.contains(&"UNICOM"), Unicom),
        (tokens.iter().any(|v| v.starts_with("AWOS")), Awos),
        (tokens.contains(&"ASOS"), Asos),
        (tokens.iter().any(|v| matches!(*v, "ATIS" | "D-ATIS")), Atis),
        (tokens.iter().any(|v| v.starts_with("LCL/")), Tower),
        (tokens.iter().any(|v| v.starts_with("GND/")), Ground),
        (
            tokens.iter().any(|v| v.starts_with("CD/") || *v == "CD") || tokens.contains(&"GCO"),
            Clearance,
        ),
        (
            tokens.iter().any(|v| v.starts_with("APCH/") || *v == "STAR")
                || matches!(value, "CLASS B" | "CLASS B/S" | "CLASS C" | "TRSA" | "VFR ADZY"),
            Approach,
        ),
        (
            tokens.iter().any(|v| v.starts_with("DEP/") || *v == "DP"),
            Departure,
        ),
    ] {
        if matches {
            kinds.push(kind);
        }
    }
    kinds
}
/// Frequencies are accepted only from FRQ. AWOS.csv has no frequency column in this cycle.
/// Its phones and remarks are intentionally never emitted.
pub fn convert(files: &BTreeMap<String, String>, source: &Source) -> Result<Vec<ComStation>, String> {
    let read = |name: &str| table(files.get(name).ok_or_else(|| format!("missing NASR {name}"))?);
    let mut airports = BTreeMap::new();
    for row in read("APT_BASE.csv")? {
        airports.insert(get(&row, "ARPT_ID").to_owned(), row);
    }
    let mut awos = BTreeMap::new();
    for row in read("AWOS.csv")? {
        awos.insert(get(&row, "ASOS_AWOS_ID").to_owned(), row);
    }
    let mut atc = BTreeMap::new();
    for row in read("ATC_BASE.csv")? {
        atc.insert(get(&row, "FACILITY_ID").to_owned(), row);
    }
    let mut atis = BTreeMap::new();
    for row in read("ATC_ATIS.csv")? {
        atis.insert(
            get(&row, "FACILITY_ID").to_owned(),
            get(&row, "ATIS_HRS").to_owned(),
        );
    }
    let mut services = BTreeMap::<String, Vec<String>>::new();
    for row in read("ATC_SVC.csv")? {
        let value = get(&row, "CTL_SVC");
        if !value.is_empty() {
            services
                .entry(get(&row, "FACILITY_ID").to_owned())
                .or_default()
                .push(value.into());
        }
    }
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, row) in read("FRQ.csv")?.into_iter().enumerate() {
        let kinds = kinds(get(&row, "FREQ_USE"));
        if kinds.is_empty() {
            continue;
        }
        let Some(mhz) = number(&row, "FREQ") else {
            continue;
        };
        let khz = (mhz * 1000.).round() as u32;
        if !flightcraft_world::comms::valid_channel(khz, false) {
            continue;
        } // Exclude UHF, receive-only, and NAV channels.
        let served = get(&row, "SERVICED_FACILITY");
        let provider = get(&row, "FACILITY");
        let Some(airport) = airports.get(served) else {
            continue;
        };
        let airport_id = get(airport, "ICAO_ID");
        let airport_id = if airport_id.is_empty() { served } else { airport_id };
        let effective = get(&row, "EFF_DATE").replace('/', "-");
        if source.edition.as_deref() != Some(effective.as_str()) {
            return Err("NASR row cycle differs from pinned source".into());
        }
        for kind in kinds {
            let weather = matches!(kind, FacilityKind::Awos | FacilityKind::Asos);
            let remote = matches!(kind, FacilityKind::Approach | FacilityKind::Departure);
            let surveyed_weather = awos.get(served).filter(|r| point(r).is_some());
            let location = if weather {
                surveyed_weather.unwrap_or(airport)
            } else if remote {
                airports.get(provider).unwrap_or(airport)
            } else {
                airport
            };
            let Some(position) = point(location) else {
                continue;
            };
            let elevation_m = number(location, "ELEV").unwrap_or(0.) * 0.3048;
            let base = atc.get(provider);
            let call = if remote {
                get(&row, "PRIMARY_APPROACH_RADIO_CALL")
            } else {
                get(&row, "TOWER_OR_COMM_CALL")
            };
            let name = if call.is_empty() {
                get(airport, "ARPT_NAME")
            } else {
                call
            };
            let hours = if kind == FacilityKind::Atis {
                atis.get(provider).map_or("", String::as_str)
            } else {
                base.map_or(get(&row, "TOWER_HRS"), |r| {
                    get(r, if remote { "CTL_PRVDING_HRS" } else { "TWR_HRS" })
                })
            };
            let sector = get(&row, "SECTORIZATION");
            let key = format!("{airport_id}-{provider}-{kind:?}-{khz}-{sector}");
            if !seen.insert(key) {
                continue;
            }
            let station = ComStation {
                id: format!(
                    "nasr-{}-{index}-{kind:?}",
                    source.edition.as_deref().unwrap_or_default()
                ),
                airport: airport_id.into(),
                name: name.into(),
                kind,
                channel_khz: khz,
                position,
                elevation_m,
                antenna_agl_m: if remote { 30. } else { 10. },
                range_nm: if remote { 100. } else { 40. },
                source: format!(
                    "{}#FRQ.csv:row={};cycle={effective};sha256={}",
                    source.url,
                    index + 2,
                    source.sha256
                ),
                metadata: Some(ComMetadata {
                    provider: provider.into(),
                    callsign: call.into(),
                    hours: hours.into(),
                    sector: sector.into(),
                    services: services.get(provider).cloned().unwrap_or_default(),
                    effective_date: effective.clone(),
                    published_use: get(&row, "FREQ_USE").into(),
                    weather_sensor: awos
                        .get(served)
                        .map(|r| get(r, "ASOS_AWOS_TYPE").to_owned())
                        .filter(|s| !s.is_empty()),
                    position_estimated: !weather || surveyed_weather.is_none(),
                }),
            };
            station.validate().map_err(|e| e.to_string())?;
            result.push(station);
        }
    }
    result.sort_by(|a, b| (&a.airport, a.channel_khz, &a.id).cmp(&(&b.airport, b.channel_khz, &b.id)));
    Ok(result)
}
fn extract(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        let leaf = name.rsplit('/').next().unwrap_or_default();
        if !TABLES.contains(&leaf) {
            continue;
        }
        if entry.size() > 64 * 1024 * 1024 || entry.encrypted() {
            return Err("NASR table exceeds bounds".into());
        }
        let mut text = String::new();
        (&mut entry)
            .take(64 * 1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() > 64 * 1024 * 1024 || files.insert(leaf.to_owned(), text).is_some() {
            return Err("NASR table size or duplication".into());
        }
    }
    Ok(files)
}
pub fn acquire<T: Transport>(
    cache: &mut Cache<T>,
    local: Option<&Path>,
) -> Result<(Vec<ComStation>, Source), String> {
    let lock: super::data::Lock = download::json(&super::root()?.join("data.lock"))?;
    let source = lock
        .sources
        .iter()
        .find(|s| s.url.contains("28DaySub") && s.url.ends_with("_CSV.zip"))
        .ok_or("data.lock lacks pinned NASR CSV source")?
        .clone();
    acquire_pinned(cache, local, &source)
}
pub(super) fn acquire_pinned<T: Transport>(
    cache: &mut Cache<T>,
    local: Option<&Path>,
    source: &Source,
) -> Result<(Vec<ComStation>, Source), String> {
    let bytes = if let Some(local) = local {
        download::read(local, MAX_DOWNLOAD)?
    } else {
        cache.get(&source.url, source.edition.clone(), false)?
    };
    if bytes.len() as u64 != source.bytes || download::sha256(&bytes) != source.sha256 {
        return Err("NASR source checksum/size mismatch".into());
    }
    cache.sources.push(source.clone());
    Ok((convert(&extract(&bytes)?, source)?, source.clone()))
}
pub fn install(directory: &Path, stations: &[ComStation], source: &Source) -> Result<(), String> {
    let airports: flightcraft_world::Airports = download::json(&directory.join("airports.json"))?;
    let ids = airports
        .airports
        .iter()
        .flat_map(|a| std::iter::once(a.ident.as_str()).chain(a.icao.as_deref()))
        .collect::<BTreeSet<_>>();
    let mut nav: Navdata = download::json(&directory.join("navdata.json"))?;
    nav.comms = stations
        .iter()
        .filter(|s| ids.contains(s.airport.as_str()))
        .cloned()
        .collect();
    nav.validate().map_err(|e| e.to_string())?;
    download::write_json(&directory.join("navdata.json"), &nav)?;
    let mut counts = BTreeMap::new();
    for s in &nav.comms {
        *counts.entry(format!("{:?}", s.kind)).or_insert(0_u64) += 1;
    }
    download::write_json(
        &directory.join("comms-manifest.json"),
        &json!({"version":1,"cycle":source.edition,"discovery":DISCOVERY,"source":source,"tables":TABLES,"counts":counts,"navdata_sha256":download::sha256(&download::read(&directory.join("navdata.json"),MAX_DOWNLOAD)?),"notes":"FRQ channels, APT facility locations, AWOS surveyed sites, ATC hours and services. Facility coordinates approximate remote transmitters. Hours are metadata, not an operational schedule model. Phones are not exported."}),
    )?;
    eprintln!("NASR: {} stations in {}", nav.comms.len(), directory.display());
    Ok(())
}
pub fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(2);
    let (mut archive, mut directory) = (None, None);
    while let Some(key) = args.next() {
        let value = args.next().ok_or("missing NASR option")?;
        match key.as_str() {
            "--archive" => archive = Some(std::path::PathBuf::from(value)),
            "--regions" => directory = Some(std::path::PathBuf::from(value)),
            _ => return Err("comms-nasr [--archive CSV.zip] --regions DIR".into()),
        }
    }
    let directory = directory.ok_or("--regions required")?;
    let mut cache = Cache {
        transport: Http,
        directory: super::root()?.join(".cache/sources"),
        sources: Vec::new(),
    };
    let (stations, source) = acquire(&mut cache, archive.as_deref())?;
    for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !path.join("navdata.json").exists() || !path.join("airports.json").exists() {
            continue;
        }
        install(&path, &stations, &source)?;
        let source_path = path.join("sources.json");
        let mut sources = if source_path.exists() {
            download::json::<super::region::Sources>(&source_path)?
        } else {
            super::region::Sources {
                version: 1,
                sources: Vec::new(),
            }
        };
        sources.sources.retain(|s| s.url != source.url);
        sources.sources.push(source.clone());
        download::write_json(&source_path, &sources)?;
        if path.join("validation.json").exists() {
            let mut report: super::validation::Report = download::json(&path.join("validation.json"))?;
            let nav: Navdata = download::json(&path.join("navdata.json"))?;
            report.counts.insert("comms".into(), nav.comms.len() as u64);
            report.bytes = super::data::files(&path)?
                .iter()
                .filter(|p| {
                    p.file_name()
                        .is_none_or(|n| n != "validation.json" && n != "validation.md")
                })
                .map(|p| std::fs::metadata(p).map(|m| m.len()).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum();
            super::validation::write(&path, &report)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    fn fixture() -> (BTreeMap<String, String>, Source) {
        let files=BTreeMap::from([
            ("APT_BASE.csv".into(),"ARPT_ID,ICAO_ID,ARPT_NAME,LAT_DECIMAL,LONG_DECIMAL,ELEV\nQ01,,TEST FIELD,40,-75,400\nQ02,KQ02,PROVIDER,40.1,-75.1,500\n".into()),
            ("FRQ.csv".into(),"EFF_DATE,FACILITY,SERVICED_FACILITY,FREQ,FREQ_USE,PRIMARY_APPROACH_RADIO_CALL,TOWER_OR_COMM_CALL,SECTORIZATION\n2026/10/29,Q01,Q01,123.0,CTAF,,,\n2026/10/29,Q02,Q01,124.45,APCH/P DEP/P,TEST CONTROL,,WEST\n2026/10/29,Q01,Q01,118.025,Q01 AWOS-3,,,\n2026/10/29,Q02,Q02,351.8,LCL/P,,,\n2026/10/29,Q02,Q02,119.1,LCL/P,,TEST TOWER,\n2026/10/29,Q02,Q02,127.2,ATIS,,,\n".into()),
            ("AWOS.csv".into(),"ASOS_AWOS_ID,LAT_DECIMAL,LONG_DECIMAL,ELEV,PHONE_NO\nQ01,40.001,-75.001,401,555-PRIVATE\n".into()),
            ("ATC_BASE.csv".into(),"FACILITY_ID,TWR_HRS,CTL_PRVDING_HRS\nQ02,0700-2200,24\n".into()),
            ("ATC_ATIS.csv".into(),"FACILITY_ID,ATIS_HRS,ATIS_PHONE_NO\nQ02,0700-2200,555-PRIVATE\n".into()),
            ("ATC_SVC.csv".into(),"FACILITY_ID,CTL_SVC\nQ02,CLASS D\n".into()),
        ]);
        let source = Source {
            url: "https://example.test/CSV.zip".into(),
            edition: Some("2026-10-29".into()),
            downloaded_at: "2026-11-02T12:00:00Z".into(),
            bytes: 1,
            sha256: "a".repeat(64),
        };
        (files, source)
    }
    #[test]
    fn frq_channels_awos_sites_provider_sector_hours_and_no_phones() {
        let (files, source) = fixture();
        let stations = convert(&files, &source).unwrap();
        assert_eq!(stations.len(), 6);
        let approach = stations
            .iter()
            .find(|s| s.kind == FacilityKind::Approach)
            .unwrap();
        assert_eq!(approach.airport, "Q01");
        assert_eq!(approach.position.latitude_deg, 40.1);
        assert_eq!(approach.channel_khz, 124450);
        let m = approach.metadata.as_ref().unwrap();
        assert_eq!(m.sector, "WEST");
        assert_eq!(m.hours, "24");
        assert_eq!(m.callsign, "TEST CONTROL");
        assert!(m.position_estimated);
        let awos = stations.iter().find(|s| s.kind == FacilityKind::Awos).unwrap();
        assert_eq!(awos.position.latitude_deg, 40.001);
        assert!(!awos.metadata.as_ref().unwrap().position_estimated);
        assert!(
            stations
                .iter()
                .filter(|s| matches!(s.kind, FacilityKind::Tower | FacilityKind::Atis))
                .all(|s| s.metadata.as_ref().unwrap().hours == "0700-2200")
        );
        assert!(!serde_json::to_string(&stations).unwrap().contains("PRIVATE"));
        let mut source = source;
        source.edition = Some("2026-11-26".into());
        assert!(convert(&files, &source).is_err());
    }
}

#[cfg(test)]
mod service_tests {
    #[test]
    fn published_radar_arrivals_departures_and_ground_outlets_are_classified() {
        use flightcraft_world::FacilityKind::*;
        for label in [
            "CLASS B",
            "CLASS B/S",
            "CLASS C",
            "TRSA",
            "VFR ADZY",
            "ORIGINAL STAR",
        ] {
            assert_eq!(super::kinds(label), vec![Approach]);
        }
        assert_eq!(super::kinds("ORIGINAL DP"), vec![Departure]);
        assert_eq!(super::kinds("APT REMARK 4 WITH GCO FREQ"), vec![Clearance]);
        assert!(super::kinds("RAMP CTL").is_empty());
    }
}
