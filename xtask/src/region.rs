//! Region orchestration. Network access ends at Transport; importers consume local files.
use super::download::{self, Cache, Http, MAX_DOWNLOAD, Source, Transport, read, write_json};
use flightcraft_world::{Region, RegionOrigin, geo::LocalProjection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub const LAYERS: &[&str] = &[
    "US_Airport",
    "Runways",
    "Class_Airspace",
    "NAVAIDSystem",
    "NavaidComponent",
    "DesignatedPoint",
    "ILS_Component",
    "ATS_Route",
    "HoldingPattern",
];
pub const CIFP: &str = "https://external-api.faa.gov/apra/cifp/chart?edition=current";
pub const DTPP: &str = "https://external-api.faa.gov/apra/dtpp/info?edition=current";
const ADDS: &str = "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services";

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: Option<String>,
    pub name: Option<String>,
    pub origin: Option<String>,
    pub bbox: Option<[f64; 4]>,
    pub cell: Option<String>,
    pub national: bool,
    pub charts: Option<Vec<String>>,
    pub spacing: f32,
    pub out: PathBuf,
    pub offline: Option<PathBuf>,
    pub cache: PathBuf,
    pub national_data: Option<PathBuf>,
    pub keep_v1: bool,
}
#[derive(Clone, Deserialize)]
pub struct Bundled {
    pub id: String,
    pub origin: String,
    pub bbox: [f64; 4],
    pub charts: Vec<String>,
    pub snapshot_date: String,
    pub grid: Region,
}
#[derive(Clone, Deserialize)]
pub struct ChartSource {
    pub name: String,
    pub kind: String,
    pub bounds: [f64; 4],
    pub basename: String,
    pub neatline: Option<[f64; 4]>,
}
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    pub version: u8,
    pub sources: Vec<Source>,
}

#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellEntry {
    pub bounds: [f64; 4],
    pub origin: RegionOrigin,
    pub manifest: String,
    pub manifest_sha256: String,
    pub content_sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellIndex {
    pub version: u8,
    pub grid_degrees: u8,
    pub cells: BTreeMap<String, CellEntry>,
}

pub fn bbox(text: &str) -> Result<[f64; 4], String> {
    let parts = text
        .split(',')
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let result: [f64; 4] = parts.try_into().map_err(|_| "bbox needs west,south,east,north")?;
    let [w, s, e, n] = result;
    if !result.iter().all(|v| v.is_finite())
        || w < -180.
        || e > 180.
        || s < -84.
        || n > 84.
        || w >= e
        || s >= n
    {
        return Err("invalid geographic bounds (split at the antimeridian)".into());
    }
    Ok(result)
}
pub fn cell(id: &str) -> Result<[f64; 4], String> {
    if id.len() != 7 || !id.is_ascii() {
        return Err("cell ID must look like n40w076".into());
    }
    let lat = id
        .get(1..3)
        .ok_or("cell latitude")?
        .parse::<i32>()
        .map_err(|e| e.to_string())?;
    let lon = id
        .get(4..7)
        .ok_or("cell longitude")?
        .parse::<i32>()
        .map_err(|e| e.to_string())?;
    let south = match id.get(..1) {
        Some("n") => lat,
        Some("s") if lat != 0 => -lat,
        _ => return Err("invalid cell latitude hemisphere".into()),
    };
    let west = match id.get(3..4) {
        Some("e") => lon,
        Some("w") if lon != 0 => -lon,
        _ => return Err("invalid cell longitude hemisphere".into()),
    };
    if south % 2 != 0 || west % 2 != 0 {
        return Err("cell southwest corner must be on the even-degree grid".into());
    }
    bbox(&format!("{west},{south},{},{}", west + 2, south + 2))
}
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Request, String> {
    if args.next().as_deref() != Some("build") {
        return Err("region build --id ID --name NAME --origin IDENT --bbox W,S,E,N --out DIR [--offline DIR] | --cell ID | --national".into());
    }
    let mut values = BTreeMap::new();
    let (mut national, mut keep_v1) = (false, false);
    while let Some(key) = args.next() {
        match key.as_str() {
            "--national" if !national => {
                national = true;
                continue;
            }
            "--keep-v1" if !keep_v1 => {
                keep_v1 = true;
                continue;
            }
            _ => (),
        }
        if ![
            "--id",
            "--name",
            "--origin",
            "--bbox",
            "--cell",
            "--charts",
            "--spacing",
            "--out",
            "--offline",
            "--cache",
            "--national-data",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unknown region option {key}"));
        }
        let value = args.next().ok_or("missing region option value")?;
        if values.insert(key, value).is_some() {
            return Err("duplicate region option".into());
        }
    }
    let take = |key: &str| values.get(key).cloned();
    let request = Request {
        id: take("--id"),
        name: take("--name"),
        origin: take("--origin"),
        bbox: take("--bbox").map(|s| bbox(&s)).transpose()?,
        cell: take("--cell"),
        national,
        charts: take("--charts").map(|s| s.split(',').map(str::to_owned).collect()),
        spacing: take("--spacing")
            .unwrap_or_else(|| "60".into())
            .parse::<f32>()
            .map_err(|e| e.to_string())?,
        out: take("--out").ok_or("missing --out")?.into(),
        offline: take("--offline").map(PathBuf::from),
        cache: take("--cache")
            .map(PathBuf::from)
            .unwrap_or(super::root()?.join(".cache/sources")),
        national_data: take("--national-data").map(PathBuf::from),
        keep_v1,
    };
    if !request.spacing.is_finite() || !(30.0..=1000.0).contains(&request.spacing) {
        return Err("spacing must be 30–1000 m".into());
    }
    if request.national
        && (request.cell.is_some()
            || request.bbox.is_some()
            || request.origin.is_some()
            || request.charts.is_some()
            || request.national_data.is_some())
    {
        return Err("--national does not take terrain or chart options".into());
    }
    if request.cell.is_some() && (request.bbox.is_some() || request.id.is_some()) {
        return Err("--cell determines ID and bounds".into());
    }
    if request
        .charts
        .as_ref()
        .is_some_and(|k| k.iter().any(|k| !matches!(k.as_str(), "sectional" | "tac")))
    {
        return Err("charts must be sectional,tac".into());
    }
    Ok(request)
}
pub fn run() -> Result<(), String> {
    if std::env::args().nth(2).as_deref() == Some("refresh") {
        if std::env::args().nth(3).is_some() {
            return Err("usage: cargo xtask region refresh".into());
        }
        return super::refresh::run();
    }
    build(parse(std::env::args().skip(2))?, Http)
}
pub fn build<T: Transport>(request: Request, transport: T) -> Result<(), String> {
    if request.out.exists() {
        return Err("region output must be a new directory".into());
    }
    let mut cache = Cache {
        transport,
        directory: request.cache.clone(),
        sources: Vec::new(),
    };
    let inputs = request.out.with_extension("inputs-pending");
    if let Some(parent) = inputs.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir(&inputs).map_err(|e| e.to_string())?;
    let result = if request.national {
        super::national::build(&request, &inputs, &mut cache)
    } else {
        build_region(&request, &inputs, &mut cache)
    };
    let _ = std::fs::remove_dir_all(&inputs);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&request.out);
    }
    result
}

pub fn query_url(layer: &str, bounds: Option<[f64; 4]>, offset: usize) -> String {
    let geometry = bounds.map_or(String::new(), |[w,s,e,n]| format!("&geometry={w}%2C{s}%2C{e}%2C{n}&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects"));
    format!(
        "{ADDS}/{layer}/FeatureServer/0/query?where=1%3D1{geometry}&outSR=4326&outFields=*&returnGeometry=true&f=geojson&orderByFields=OBJECTID&resultRecordCount=2000&resultOffset={offset}"
    )
}
pub fn layer<T: Transport>(
    cache: &mut Cache<T>,
    name: &str,
    bounds: Option<[f64; 4]>,
) -> Result<Value, String> {
    let mut offset = 0;
    let mut all = Vec::new();
    for _ in 0..10000 {
        let bytes = cache.get(&query_url(name, bounds, offset), None, true)?;
        let page: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if page.get("error").is_some() || page["type"] != "FeatureCollection" {
            return Err(format!("FAA {name} did not return GeoJSON"));
        }
        let features = page["features"].as_array().ok_or("missing FAA features")?;
        let more = page
            .get("exceededTransferLimit")
            .or_else(|| {
                page.get("properties")
                    .and_then(|p| p.get("exceededTransferLimit"))
            })
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if more && features.is_empty() {
            return Err("FAA pagination made no progress".into());
        }
        offset += features.len();
        if offset > 2_000_000 {
            return Err("FAA layer exceeds feature bound".into());
        }
        all.extend(features.iter().cloned());
        if !more {
            return Ok(json!({"type":"FeatureCollection","features":all}));
        }
    }
    Err("FAA pagination exceeds page bound".into())
}
pub fn acquire_layers<T: Transport>(
    request: &Request,
    inputs: &Path,
    cache: &mut Cache<T>,
    bounds: Option<[f64; 4]>,
) -> Result<(), String> {
    if let Some(national) = &request.national_data {
        return super::national::views(national, inputs, bounds.ok_or("view requires bounds")?);
    }
    for name in LAYERS {
        let value = if let Some(offline) = &request.offline {
            let directory = if ["US_Airport", "Runways", "Class_Airspace"].contains(name) {
                "faa"
            } else {
                "nav"
            };
            let bytes = cache.offline(
                &offline.join(directory).join(format!("{name}.geojson")),
                query_url(name, bounds, 0),
                None,
            )?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if value["exceededTransferLimit"] == true {
                return Err("offline layer is truncated".into());
            }
            value
        } else {
            layer(cache, name, bounds)?
        };
        write_json(&inputs.join(format!("{name}.geojson")), &value)?;
    }
    Ok(())
}

/// APRA chart/info responses nest edition records. Only HTTPS payload links are accepted.
pub fn payload(value: &Value, suffix: &str) -> Result<String, String> {
    fn visit(value: &Value, suffix: &str, found: &mut Vec<String>) {
        match value {
            Value::String(s)
                if s.starts_with("https://")
                    && s.to_ascii_lowercase()
                        .split('?')
                        .next()
                        .is_some_and(|s| s.ends_with(suffix)) =>
            {
                found.push(s.clone())
            }
            Value::Array(a) => {
                for v in a {
                    visit(v, suffix, found);
                }
            }
            Value::Object(o) => {
                for v in o.values() {
                    visit(v, suffix, found);
                }
            }
            _ => (),
        }
    }
    let mut found = Vec::new();
    visit(value, suffix, &mut found);
    found.sort();
    found.dedup();
    if found.len() != 1 {
        return Err(format!("APRA must identify exactly one {suffix} payload"));
    }
    found.pop().ok_or_else(|| "missing APRA payload".into())
}
pub fn cycle(value: &Value) -> Result<String, String> {
    if let Some(o) = value.as_object() {
        for key in ["cycle", "cycleNumber", "edition", "editionNumber", "productCode"] {
            if let Some(s) = o
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit()))
            {
                return Ok(s.into());
            }
        }
        for v in o.values() {
            if let Ok(s) = cycle(v) {
                return Ok(s);
            }
        }
    }
    if let Some(a) = value.as_array() {
        for v in a {
            if let Ok(s) = cycle(v) {
                return Ok(s);
            }
        }
    }
    Err("APRA response has no four-digit cycle".into())
}
pub fn extract(bytes: &[u8], output: &Path, accept: impl Fn(&str) -> bool) -> Result<Vec<PathBuf>, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    if zip.len() > 4096 {
        return Err("source archive has too many entries".into());
    }
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    let mut total = 0_u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        if !super::data::safe_path(name.trim_end_matches('/'))
            || entry.encrypted()
            || entry
                .unix_mode()
                .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000 | 0o040000))
        {
            return Err("unsafe source ZIP entry".into());
        }
        if entry.is_dir() || !accept(&name) {
            continue;
        }
        total = total.checked_add(entry.size()).ok_or("source size overflow")?;
        if total > 512 * 1024 * 1024 || entry.size() > MAX_DOWNLOAD as u64 {
            return Err("source archive exceeds decoded bound".into());
        }
        let base = Path::new(&name).file_name().ok_or("source basename")?;
        let path = output.join(base);
        if path.exists() {
            return Err("duplicate source ZIP basename".into());
        }
        let mut data = Vec::new();
        (&mut entry)
            .take(MAX_DOWNLOAD as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        if data.len() > MAX_DOWNLOAD {
            return Err("source entry too large".into());
        }
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
        paths.push(path);
    }
    Ok(paths)
}
pub fn acquire_procedures<T: Transport>(
    request: &Request,
    inputs: &Path,
    cache: &mut Cache<T>,
) -> Result<(String, String), String> {
    let (cifp, dtpp) = if let Some(offline) = &request.offline {
        (
            cache.offline(&offline.join("cifp/FAACIFP18"), CIFP.into(), None)?,
            cache.offline(&offline.join("dtpp.xml"), DTPP.into(), None)?,
        )
    } else {
        let api: Value = serde_json::from_slice(&cache.get(CIFP, None, true)?).map_err(|e| e.to_string())?;
        let url = payload(&api, ".zip")?;
        let known_cycle = cycle(&api).ok();
        let zip = cache.get(&url, known_cycle.clone(), known_cycle.is_none())?;
        let paths = extract(&zip, &inputs.join("cifp"), |n| n.ends_with("FAACIFP18"))?;
        if paths.len() != 1 {
            return Err("CIFP ZIP must contain FAACIFP18".into());
        }
        let cifp = read(paths.first().ok_or("missing CIFP")?, MAX_DOWNLOAD)?;
        let api: Value = serde_json::from_slice(&cache.get(DTPP, None, true)?).map_err(|e| e.to_string())?;
        let cycle = cycle(&api)?;
        let url = format!("https://aeronav.faa.gov/d-tpp/{cycle}/xml_data/d-tpp_Metafile.xml");
        (cifp, cache.get(&url, Some(cycle), false)?)
    };
    let text = std::str::from_utf8(&cifp).map_err(|e| e.to_string())?;
    let edition = super::procedures_faa::edition(text)?;
    std::fs::write(inputs.join("FAACIFP18"), cifp).map_err(|e| e.to_string())?;
    std::fs::write(inputs.join("dtpp.xml"), dtpp).map_err(|e| e.to_string())?;
    Ok(edition)
}

fn args(pairs: &[(&str, PathBuf)]) -> std::vec::IntoIter<String> {
    pairs
        .iter()
        .flat_map(|(k, v)| [k.to_string(), v.to_string_lossy().into_owned()])
        .collect::<Vec<_>>()
        .into_iter()
}
fn build_region<T: Transport>(request: &Request, inputs: &Path, cache: &mut Cache<T>) -> Result<(), String> {
    let bundled: Vec<Bundled> = download::json(&super::root()?.join("datasets/regions.json"))?;
    let id = request
        .cell
        .as_ref()
        .or(request.id.as_ref())
        .ok_or("missing --id or --cell")?;
    let preset = bundled.iter().find(|r| r.id == *id);
    let bounds = if let Some(cell_id) = &request.cell {
        cell(cell_id)?
    } else {
        request
            .bbox
            .or_else(|| preset.map(|p| p.bbox))
            .ok_or("missing --bbox")?
    };
    acquire_layers(request, inputs, cache, Some(bounds))?;
    let (cycle, effective) = if let Some(national) = &request.national_data {
        let index: super::national::Index = download::json(&national.join("index.json"))?;
        (index.cycle, index.effective_date)
    } else {
        acquire_procedures(request, inputs, cache)?
    };
    let region = if let Some(preset) = preset.filter(|p| {
        p.bbox == bounds
            && request.spacing == p.grid.spacing_m
            && request.origin.as_ref().is_none_or(|o| o == &p.origin)
    }) {
        let mut region = preset.grid.clone();
        if let Some(name) = &request.name {
            region.name = name.clone();
        }
        region
    } else {
        grid(request, id, bounds, inputs)?
    };
    write_json(&inputs.join("region.json"), &region)?;
    acquire_hgt(request, inputs, cache, &region)?;
    super::terrain_srtm::run_args(args(&[
        ("--region", inputs.join("region.json")),
        ("--hgt", inputs.join("hgt")),
        ("--out", request.out.clone()),
    ]))?;
    let region_path = request.out.join("region.json");
    super::navdata_faa::run_args(args(&[
        ("--region", region_path.clone()),
        ("--airports", inputs.join("US_Airport.geojson")),
        ("--runways", inputs.join("Runways.geojson")),
        ("--out", request.out.join("airports.json")),
    ]))?;
    super::airspace_faa::run_args(args(&[
        ("--region", region_path.clone()),
        ("--airspace", inputs.join("Class_Airspace.geojson")),
        ("--out", request.out.join("airspace.json")),
    ]))?;
    let snapshot = if let Some(metadata) = request
        .offline
        .as_ref()
        .map(|p| p.join("snapshot.json"))
        .filter(|p| p.exists())
    {
        let metadata: Value = download::json(&metadata)?;
        metadata["snapshot_date"]
            .as_str()
            .ok_or("snapshot.json needs snapshot_date")?
            .to_owned()
    } else if request.offline.is_some() && preset.is_some() {
        preset.ok_or("bundled recipe")?.snapshot_date.clone()
    } else {
        download::now()?.get(..10).ok_or("snapshot date")?.into()
    };
    let mut navigation = args(&[
        ("--region", region_path.clone()),
        ("--layers", inputs.into()),
        ("--out", request.out.join("navdata.json")),
        ("--snapshot-date", snapshot.into()),
    ])
    .collect::<Vec<_>>();
    // ADDS is a live snapshot; a CIFP effective date does not establish an ADDS cycle.
    if let Some(national) = &request.national_data {
        cache
            .sources
            .extend(download::json::<Sources>(&national.join("sources.json"))?.sources);
    }
    super::navigation_faa::run_args(navigation.drain(..))?;
    if let Some(national) = &request.national_data {
        let airports: flightcraft_world::Airports = download::json(&request.out.join("airports.json"))?;
        let ids = airports
            .airports
            .iter()
            .flat_map(|a| std::iter::once(a.ident.clone()).chain(a.icao.clone()))
            .collect();
        let data = super::national::procedure_view(national, &ids)?;
        write_json(&request.out.join("procedures.json"), &data)?;
    } else {
        super::procedures_faa::run_args(args(&[
            ("--region", region_path),
            ("--cifp", inputs.join("FAACIFP18")),
            ("--dtpp", inputs.join("dtpp.xml")),
            ("--out", request.out.join("procedures.json")),
        ]))?;
    }
    let kinds = request
        .charts
        .clone()
        .unwrap_or_else(|| preset.map_or(Vec::new(), |p| p.charts.clone()));
    acquire_charts(request, inputs, cache, bounds, &kinds)?;
    let mut report = super::validation::region(&request.out, inputs)?;
    if !request.keep_v1 {
        let region: Region = download::json(&request.out.join("region.json"))?;
        for name in region.tiles {
            let path = request.out.join(name);
            let tile =
                flightcraft_world::HeightTile::read(&read(&path, flightcraft_world::MAX_REGION_TILE_BYTES)?)
                    .map_err(|e| e.to_string())?;
            std::fs::write(path, tile.write_compact().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
    }
    for source in &mut cache.sources {
        if source.edition.is_none()
            && (source.url.to_ascii_lowercase().contains("cifp")
                || source.url.contains("dtpp")
                || source.url.contains("d-tpp"))
        {
            source.edition = Some(format!("{cycle} ({effective})"));
        }
    }
    write_json(
        &request.out.join("sources.json"),
        &Sources {
            version: 1,
            sources: cache.sources.clone(),
        },
    )?;
    report.bytes = super::data::files(&request.out)?
        .iter()
        .map(|p| std::fs::metadata(p).map(|m| m.len()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum();
    super::validation::write(&request.out, &report)?;
    if request.cell.is_some() {
        update_cells(&request.out, bounds)?;
    }
    Ok(())
}

pub fn grid(request: &Request, id: &str, bounds: [f64; 4], inputs: &Path) -> Result<Region, String> {
    let [w, s, e, n] = bounds;
    let origin = if let Some(ident) = &request.origin {
        let airports: Value = download::json(&inputs.join("US_Airport.geojson"))?;
        let runways: Value = download::json(&inputs.join("Runways.geojson"))?;
        let airport = airports["features"]
            .as_array()
            .ok_or("airport features")?
            .iter()
            .find(|f| {
                f["properties"]["IDENT"].as_str() == Some(ident)
                    || f["properties"]["ICAO_ID"].as_str() == Some(ident)
            })
            .ok_or("origin airport absent from FAA source")?;
        let props = &airport["properties"];
        let lat = super::navdata_faa::dms(props["LATITUDE"].as_str().ok_or("origin latitude")?)?;
        let lon = super::navdata_faa::dms(props["LONGITUDE"].as_str().ok_or("origin longitude")?)?;
        let projection = LocalProjection::new(lat, lon).map_err(|e| e.to_string())?;
        let runway = runways["features"]
            .as_array()
            .ok_or("runway features")?
            .iter()
            .find(|f| f["properties"]["AIRPORT_ID"] == props["GLOBAL_ID"])
            .ok_or("origin airport has no runway")?;
        let ends = super::navdata_faa::centreline(runway, &projection)?;
        let [a, b] = ends;
        let [lat, lon] = projection.unproject(a[0], a[1]).map_err(|e| e.to_string())?;
        RegionOrigin {
            latitude_deg: lat,
            longitude_deg: lon,
            field_elevation_m: props["ELEVATION"].as_f64().ok_or("origin elevation")? * 0.3048,
            runway_heading_deg: super::navdata_faa::true_heading(&projection, a, b)?,
            runway_length_m: runway["properties"]["LENGTH"]
                .as_f64()
                .ok_or("origin runway length")?
                * 0.3048,
            runway_width_m: runway["properties"]["WIDTH"]
                .as_f64()
                .ok_or("origin runway width")?
                * 0.3048,
        }
    } else if request.cell.is_some() {
        RegionOrigin {
            latitude_deg: (s + n) / 2.,
            longitude_deg: (w + e) / 2.,
            field_elevation_m: 0.,
            runway_heading_deg: 0.,
            runway_length_m: 900.,
            runway_width_m: 23.,
        }
    } else {
        return Err("missing --origin".into());
    };
    let projection =
        LocalProjection::new(origin.latitude_deg, origin.longitude_deg).map_err(|e| e.to_string())?;
    let corners = [[s, w], [s, e], [n, w], [n, e]]
        .map(|[lat, lon]| projection.project(lat, lon).map_err(|e| e.to_string()))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let south = corners.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min).floor() - 300.;
    let west = corners.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min).floor() - 300.;
    let north = corners
        .iter()
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        + 300.;
    let east = corners
        .iter()
        .map(|p| p[1])
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        + 300.;
    let span = f64::from(request.spacing) * 256.;
    let region = Region {
        id: id.into(),
        name: request
            .name
            .clone()
            .unwrap_or_else(|| format!("Terrain cell {id}")),
        origin,
        southwest_m: [south, west],
        tiles_north: ((north - south) / span).ceil() as u32,
        tiles_east: ((east - west) / span).ceil() as u32,
        spacing_m: request.spacing,
        tile_size: 257,
        runway_blend_m: 200.,
        source: "Terrain Tiles (USGS 3DEP and NASA SRTM)".into(),
        licence: "public domain".into(),
        attribution: "Elevation: USGS 3DEP and NASA SRTM, public domain, via the Terrain Tiles open dataset"
            .into(),
        tiles: Vec::new(),
        airports: Some("airports.json".into()),
    };
    region
        .validate()
        .map_err(|e| format!("grid exceeds engine limits; increase --spacing: {e}"))?;
    Ok(region)
}
pub fn grid_bounds(region: &Region) -> Result<[f64; 4], String> {
    let projection = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
        .map_err(|e| e.to_string())?;
    let [s, w] = region.southwest_m;
    let span = f64::from(region.spacing_m) * f64::from(region.tile_size - 1);
    let (n, e) = (
        s + span * f64::from(region.tiles_north),
        w + span * f64::from(region.tiles_east),
    );
    let mut corners = [[s, w], [s, e], [n, w], [n, e]]
        .map(|[n, e]| projection.unproject(n, e).map_err(|e| e.to_string()))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    for point in &mut corners {
        point[1] = region.origin.longitude_deg
            + (point[1] - region.origin.longitude_deg + 180.).rem_euclid(360.)
            - 180.;
    }
    Ok([
        corners.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
        corners.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
        corners.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max),
        corners.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max),
    ])
}
fn acquire_hgt<T: Transport>(
    request: &Request,
    inputs: &Path,
    cache: &mut Cache<T>,
    region: &Region,
) -> Result<(), String> {
    let output = inputs.join("hgt");
    std::fs::create_dir(&output).map_err(|e| e.to_string())?;
    let [w, s, e, n] = grid_bounds(region)?;
    let mut count = 0;
    for lat in s.floor() as i32..=n.floor() as i32 {
        for lon in w.floor() as i32..=e.floor() as i32 {
            let lon = (lon + 180).rem_euclid(360) - 180;
            count += 1;
            if count > 64 {
                return Err("region requires more than 64 HGT squares".into());
            }
            let name = format!(
                "{}{:02}{}{:03}",
                if lat >= 0 { "N" } else { "S" },
                lat.abs(),
                if lon >= 0 { "E" } else { "W" },
                lon.abs()
            );
            let url = format!(
                "https://s3.amazonaws.com/elevation-tiles-prod/skadi/{}{:02}/{name}.hgt.gz",
                if lat >= 0 { "N" } else { "S" },
                lat.abs()
            );
            let (filename, bytes) = if let Some(offline) = &request.offline {
                let name = [
                    format!("{name}.hgt"),
                    format!("{name}.hgt.gz"),
                    format!("{name}.hgt.zip"),
                ]
                .into_iter()
                .find(|n| offline.join("hgt").join(n).exists())
                .ok_or_else(|| format!("missing offline HGT {name}"))?;
                let bytes = cache.offline(&offline.join("hgt").join(&name), url, None)?;
                (name, bytes)
            } else {
                (format!("{name}.hgt.gz"), cache.get(&url, None, false)?)
            };
            std::fs::write(output.join(filename), bytes).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn intersects([w, s, e, n]: [f64; 4], [w1, s1, e1, n1]: [f64; 4]) -> bool {
    w <= e1 && e >= w1 && s <= n1 && n >= s1
}
fn acquire_charts<T: Transport>(
    request: &Request,
    inputs: &Path,
    cache: &mut Cache<T>,
    bounds: [f64; 4],
    kinds: &[String],
) -> Result<(), String> {
    let catalog: Vec<ChartSource> = download::json(&super::root()?.join("datasets/charts.json"))?;
    for kind in kinds {
        let selected: Vec<_> = catalog
            .iter()
            .filter(|c| c.kind == *kind && intersects(c.bounds, bounds))
            .collect();
        if selected.is_empty() {
            return Err(format!(
                "no verified {kind} chart coverage in datasets/charts.json; add FAA metadata bounds"
            ));
        }
        // Each chart is imported separately, then overlapping tiles are composited deterministically.
        let mut indexes = Vec::new();
        for (i, chart) in selected.iter().enumerate() {
            let receipt_start = cache.sources.len();
            let chart_dir = inputs.join(format!("chart-{kind}-{i}"));
            std::fs::create_dir(&chart_dir).map_err(|e| e.to_string())?;
            let url = format!(
                "https://external-api.faa.gov/apra/vfr/{kind}/chart?geoname={}&edition=current&format=tiff",
                chart.name.replace(' ', "%20")
            );
            if let Some(offline) = &request.offline {
                for extension in ["tif", "tfw", "htm"] {
                    let filename = format!("{}.{extension}", chart.basename);
                    let bytes = cache.offline(
                        &offline.join("charts").join(kind).join(&filename),
                        url.clone(),
                        None,
                    )?;
                    std::fs::write(chart_dir.join(filename), bytes).map_err(|e| e.to_string())?;
                }
            } else {
                let api: Value =
                    serde_json::from_slice(&cache.get(&url, None, true)?).map_err(|e| e.to_string())?;
                let zip_url = payload(&api, ".zip")?;
                let edition = super::refresh::edition(&api);
                extract(
                    &cache.get(&zip_url, edition.clone(), edition.is_none())?,
                    &chart_dir,
                    |name| {
                        ["tif", "tfw", "htm"]
                            .iter()
                            .any(|e| name == format!("{}.{e}", chart.basename))
                    },
                )?;
            }
            let output = inputs.join(format!("tiles-{kind}-{i}"));
            let mut options = args(&[
                ("--region", request.out.join("region.json")),
                ("--geotiff", chart_dir.join(format!("{}.tif", chart.basename))),
                ("--tfw", chart_dir.join(format!("{}.tfw", chart.basename))),
                ("--kind", kind.into()),
                ("--out", output.clone()),
            ])
            .collect::<Vec<_>>();
            if let Some([w, s, e, n]) = chart.neatline {
                options.extend(["--neatline".into(), format!("{w},{s},{e},{n}")]);
            }
            super::charts_faa::run_args(options.into_iter())?;
            let index: flightcraft_control::ChartIndex = download::json(&output.join("index.json"))?;
            for source in cache.sources.iter_mut().skip(receipt_start) {
                source.edition = Some(index.edition.clone());
            }
            indexes.push((output, index));
        }
        merge_charts(&indexes, &request.out.join("charts").join(kind))?;
    }
    Ok(())
}
fn merge_charts(indexes: &[(PathBuf, flightcraft_control::ChartIndex)], output: &Path) -> Result<(), String> {
    let mut merged = indexes.first().ok_or("chart indexes")?.1.clone();
    merged.tiles.clear();
    merged.bytes = 0;
    let mut tiles = BTreeMap::<_, image::RgbaImage>::new();
    for (directory, index) in indexes {
        if index.edition != merged.edition || index.expires != merged.expires {
            return Err("overlapping charts have different editions".into());
        }
        for tile in &index.tiles {
            let image = image::open(directory.join(index.tile_path(*tile)))
                .map_err(|e| e.to_string())?
                .into_rgba8();
            if let Some(old) = tiles.get_mut(tile) {
                image::imageops::overlay(old, &image, 0, 0);
            } else {
                tiles.insert(*tile, image);
            }
        }
    }
    for (tile, image) in tiles {
        let path = output.join(merged.tile_path(tile));
        std::fs::create_dir_all(path.parent().ok_or("chart tile parent")?).map_err(|e| e.to_string())?;
        image.save(&path).map_err(|e| e.to_string())?;
        merged.bytes += std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        merged.tiles.push(tile);
        if merged.bytes > 65 * 1024 * 1024 || merged.tiles.len() > 20000 {
            return Err("combined chart set exceeds client budget; use a smaller cell".into());
        }
    }
    write_json(&output.join("index.json"), &merged)
}
fn update_cells(output: &Path, bounds: [f64; 4]) -> Result<(), String> {
    let parent = output.parent().ok_or("cell output needs parent")?;
    let path = parent.join("cells.json");
    let mut index: CellIndex = if path.exists() {
        download::json(&path)?
    } else {
        CellIndex {
            version: 1,
            grid_degrees: 2,
            cells: BTreeMap::new(),
        }
    };
    if index.version != 1 || index.grid_degrees != 2 {
        return Err("unsupported cell index".into());
    }
    let region: Region = download::json(&output.join("region.json"))?;
    let mut contents = Vec::new();
    let mut bytes = 0;
    for path in super::data::files(output)? {
        let data = read(&path, MAX_DOWNLOAD)?;
        bytes += data.len() as u64;
        contents.push((
            path.strip_prefix(output)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/"),
            download::sha256(&data),
        ));
    }
    let directory = output
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("cell output directory requires UTF-8")?;
    index.cells.insert(
        region.id,
        CellEntry {
            bounds,
            origin: region.origin,
            manifest: format!("{directory}/region.json"),
            manifest_sha256: download::sha256(&read(&output.join("region.json"), 65536)?),
            content_sha256: download::sha256(&serde_json::to_vec(&contents).map_err(|e| e.to_string())?),
            bytes,
        },
    );
    write_json(&path, &index)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::super::download::Recorded;
    use super::*;
    use std::io::Write;

    fn temporary(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("flightcraft-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn zip(name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap().into_inner()
    }
    #[test]
    fn pagination_cache_integrity_and_discovery() {
        let directory = temporary("download-test");
        let responses = BTreeMap::from([
            (
                query_url("US_Airport", None, 0),
                include_bytes!("../tests/fixtures/arcgis-page-1.json").to_vec(),
            ),
            (
                query_url("US_Airport", None, 1),
                include_bytes!("../tests/fixtures/arcgis-page-2.json").to_vec(),
            ),
        ]);
        let mut cache = Cache {
            transport: Recorded(responses),
            directory: directory.clone(),
            sources: Vec::new(),
        };
        let result = layer(&mut cache, "US_Airport", None).unwrap();
        assert_eq!(result["features"].as_array().unwrap().len(), 2);
        assert_eq!(cache.sources.len(), 2);
        let url = query_url("US_Airport", None, 0);
        cache.transport.0.clear();
        assert!(cache.get(&url, None, false).is_ok());
        assert!(cache.get(&url, Some("new-edition".into()), false).is_err());
        std::fs::write(directory.join(download::sha256(url.as_bytes())), b"corrupt").unwrap();
        assert!(cache.get(&url, None, false).is_err());
        let api: Value = serde_json::from_slice(include_bytes!("../tests/fixtures/apra-cifp.json")).unwrap();
        assert_eq!(
            payload(&api, ".zip").unwrap(),
            "https://example.test/CIFP_261029.zip"
        );
        assert_eq!(cycle(&api).unwrap(), "2611");
        assert!(payload(&json!({"url":"http://example.test/data.zip"}), ".zip").is_err());
        assert!(extract(&zip("../escape", b"bad"), &directory.join("unsafe"), |_| true).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn national_grid_bounds_and_hostile_options() {
        assert_eq!(cell("n40w076").unwrap(), [-76., 40., -74., 42.]);
        assert_eq!(cell("s16w172").unwrap(), [-172., -16., -170., -14.]);
        assert_eq!(cell("n50e178").unwrap(), [178., 50., 180., 52.]);
        for id in ["n41w076", "n40w075", "N40W076", "../evil", "n90w076", "n00w000"] {
            assert!(cell(id).is_err());
        }
        assert!(bbox("NaN,0,1,2").is_err());
        assert!(
            parse(
                ["build", "--national", "--cell", "n40w076", "--out", "unused"]
                    .map(str::to_owned)
                    .into_iter()
            )
            .is_err()
        );
        let template = temporary("cell-grid");
        let request = parse(
            ["build", "--cell", "n40w076", "--out", "unused"]
                .map(str::to_owned)
                .into_iter(),
        )
        .unwrap();
        let grid = grid(&request, "n40w076", cell("n40w076").unwrap(), &template).unwrap();
        assert_eq!(grid.origin.latitude_deg, 41.);
        assert_eq!(grid.origin.longitude_deg, -75.);
        assert!(grid.tiles_north * grid.tiles_east <= 256);
        let mut edge = request.clone();
        edge.cell = Some("n50e178".into());
        let edge_grid = super::grid(&edge, "n50e178", cell("n50e178").unwrap(), &template).unwrap();
        let [west, _, east, _] = grid_bounds(&edge_grid).unwrap();
        assert!(
            east - west < 4.,
            "antimeridian grid must retain a local longitude span"
        );

        std::fs::remove_dir_all(template).unwrap();
    }
    #[test]
    fn recorded_transport_builds_a_complete_region_without_network() {
        let directory = temporary("pipeline-fixture");
        let output = directory.join("region");
        let request = parse(
            [
                "build",
                "--id",
                "fixture",
                "--name",
                "Fixture",
                "--origin",
                "T01",
                "--bbox",
                "0.499,0.499,0.511,0.501",
                "--out",
                output.to_str().unwrap(),
                "--cache",
                directory.join("cache").to_str().unwrap(),
            ]
            .map(str::to_owned)
            .into_iter(),
        )
        .unwrap();
        let mut responses = BTreeMap::new();
        for name in LAYERS {
            let bytes = match *name {
                "US_Airport" => include_bytes!("../tests/fixtures/faa/airports.geojson").to_vec(),
                "Runways" => include_bytes!("../tests/fixtures/faa/runways.geojson").to_vec(),
                _ => serde_json::to_vec(
                    &json!({"type":"FeatureCollection","exceededTransferLimit":false,"features":[]}),
                )
                .unwrap(),
            };
            responses.insert(query_url(name, request.bbox, 0), bytes);
        }
        responses.insert(
            CIFP.into(),
            include_bytes!("../tests/fixtures/apra-cifp.json").to_vec(),
        );
        responses.insert(
            DTPP.into(),
            include_bytes!("../tests/fixtures/apra-dtpp.json").to_vec(),
        );
        responses.insert(
            "https://example.test/CIFP_261029.zip".into(),
            zip("FAACIFP18", b"HDR04 CYCLE 2611 EFFECTIVE 29 OCT 2026\n"),
        );
        responses.insert(
            "https://aeronav.faa.gov/d-tpp/2611/xml_data/d-tpp_Metafile.xml".into(),
            b"<digital_tpp cycle=\"2611\"/>".to_vec(),
        );
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder
            .write_all(
                &[150_i16.to_be_bytes()]
                    .repeat(1201 * 1201)
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        responses.insert(
            "https://s3.amazonaws.com/elevation-tiles-prod/skadi/N00/N00E000.hgt.gz".into(),
            encoder.finish().unwrap(),
        );
        build(request, Recorded(responses)).unwrap();
        for name in [
            "region.json",
            "airports.json",
            "airspace.json",
            "navdata.json",
            "procedures.json",
            "sources.json",
            "validation.json",
            "validation.md",
        ] {
            assert!(output.join(name).is_file(), "{name}");
        }
        let region: Region = download::json(&output.join("region.json")).unwrap();
        let tile = read(
            &output.join(region.tiles.first().unwrap()),
            flightcraft_world::MAX_REGION_TILE_BYTES,
        )
        .unwrap();
        assert_eq!(tile.get(4..6), Some(2_u16.to_le_bytes().as_slice()));
        let report: super::super::validation::Report =
            download::json(&output.join("validation.json")).unwrap();
        assert_eq!(report.counts.get("airports"), Some(&1));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
