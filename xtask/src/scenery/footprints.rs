//! NYC portal footprints are local and user-fetched only; agency/NYC terms are not an open license. Acquisition is bounded and separate from offline builds.
use super::{Operation, Request, city, city_mesh, download, transport};
use crate::download::Source;
use flightcraft_world::tiles3d::{self, Tileset};
use glam::{DMat4, DVec3};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

const HOST: &str = "data.cityofnewyork.us";
const DATASET: &str = "nqwf-w8eh";
const METADATA: &str = "https://data.cityofnewyork.us/api/views/nqwf-w8eh.json";
const TERMS: &str = "https://www.nyc.gov/html/data/terms.html";
const CREDIT: &str = "Buildings: City of New York, NYC Open Data, Building Footprints. NYC.gov and supplying-agency terms apply; local user-fetched use only. Original FlightCraft procedural materials. For simulation only.";
// Fly NYC: Manhattan, the Hudson/VFR corridor, and adjacent NYC shoreline. No citywide bulk export.
const BOUNDS: [f64; 4] = flightcraft_render::scenery::NYC_BOUNDS;
const PAGE_ROWS: usize = 1000;
const PAGE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintSources {
    pub dataset: String,
    pub bounds: [f64; 4],
    pub metadata: Source,
    pub final_metadata: Option<Source>,
    pub revision: u64,
    pub terms_url: String,
    pub attribution: String,
    pub geometry_field: String,
    pub roof_field: String,
    pub ground_field: String,
    pub pages: Vec<Source>,
    pub rows: usize,
    pub complete: bool,
    pub local_only: bool,
    pub redistribution_allowed: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HeightUnit {
    Meters,
    Feet,
    UsSurveyFeet,
}
impl HeightUnit {
    fn meters(self) -> f64 {
        match self {
            Self::Meters => 1.,
            Self::Feet => 0.3048,
            Self::UsSurveyFeet => 1200. / 3937.,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RoofReference {
    AboveGround,
    Absolute,
}
/// Source semantics must be reviewed from the fetched metadata, never inferred from field names.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintReview {
    pub metadata_sha256: String,
    pub height_unit: Option<HeightUnit>,
    pub roof_reference: Option<RoofReference>,
    pub ground_datum: String,
    pub evidence: String,
    /// Source ground datum to the renderer's presentation MSL, not an invented geoid separation.
    pub presentation_height_offset_m: f64,
    pub vertical_datum_verified: bool,
    pub runway_alignment_verified: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintReport {
    pub region: String,
    pub area: String,
    pub manifest: std::path::PathBuf,
    pub bounds: [f64; 4],
    pub buildings: usize,
    pub flat_roof_buildings: usize,
    pub lidar_roof_buildings: usize,
    pub missing_height_rows: usize,
    pub tiles: usize,
    pub package_bytes: u64,
    pub building_budget_bytes: u64,
    pub total_web_budget_bytes: u64,
    pub local_only: bool,
    pub redistribution_allowed: bool,
    pub preview_only: bool,
    pub vertical_datum_verified: bool,
    pub runway_alignment_verified: bool,
    pub release_ready: bool,
}
fn revision(metadata: &Value) -> Result<u64, String> {
    metadata
        .get("rowsUpdatedAt")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Socrata metadata has no dataset revision".into())
}
fn field(metadata: &Value, aliases: &[&str], kind: Option<&str>) -> Result<String, String> {
    let columns = metadata
        .get("columns")
        .and_then(Value::as_array)
        .ok_or("Missing Socrata columns")?;
    for col in columns {
        if let Some(name) = col.get("fieldName").and_then(Value::as_str)
            && aliases.contains(&name)
            && kind.is_none_or(|k| col.get("dataTypeName").and_then(Value::as_str) == Some(k))
        {
            return Ok(name.into());
        }
    }
    Err(format!(
        "NYC metadata lacks a recognized {} field; review the dataset before fetching geometry",
        aliases.first().copied().unwrap_or("building")
    ))
}
fn percent(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
fn page_url(s: &FootprintSources, offset: usize) -> String {
    let [w, south, e, n] = s.bounds;
    let select = format!(
        ":id as source_row_id,{},{},{}",
        s.geometry_field, s.roof_field, s.ground_field
    );
    let filter = format!("within_box({}, {n}, {w}, {south}, {e})", s.geometry_field);
    format!(
        "https://{HOST}/resource/{DATASET}.json?$select={}&$where={}&$order=%3Aid&$limit={PAGE_ROWS}&$offset={offset}",
        percent(&select),
        percent(&filter)
    )
}
fn fetch(request: &Request, remote: &mut impl transport::Remote) -> Result<FootprintSources, String> {
    let (path, metadata) = transport::fetch(remote, METADATA, &request.cache.join("objects"), PAGE_BYTES)?;
    let value: Value = download::json(&path)?;
    download::write_json(&request.cache.join("footprint-metadata.json"), &value)?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if !name.contains("building")
        || !name.contains("footprint")
        || value.get("id").and_then(Value::as_str) != Some(DATASET)
    {
        return Err("Socrata dataset identity does not match NYC Building Footprints".into());
    }
    let mut sources = FootprintSources {
        dataset: DATASET.into(),
        bounds: BOUNDS,
        revision: revision(&value)?,
        terms_url: TERMS.into(),
        attribution: CREDIT.into(),
        geometry_field: field(&value, &["the_geom", "multipolygon"], Some("multipolygon"))?,
        roof_field: field(&value, &["heightroof", "height_roof"], Some("number"))?,
        ground_field: field(&value, &["groundelev", "ground_elevation"], Some("number"))?,
        metadata,
        final_metadata: None,
        pages: Vec::new(),
        rows: 0,
        complete: false,
        local_only: true,
        redistribution_allowed: false,
    };
    let example = FootprintReview {
        metadata_sha256: sources.metadata.sha256.clone(),
        height_unit: None,
        roof_reference: None,
        ground_datum: "Unverified; read the fetched metadata descriptions".into(),
        evidence: String::new(),
        presentation_height_offset_m: 0.,
        vertical_datum_verified: false,
        runway_alignment_verified: false,
    };
    download::write_json(&request.cache.join("footprint-review.example.json"), &example)?;
    loop {
        if sources.pages.len() >= 250 {
            return Err(
                "Fly NYC fetch exceeds 250,000 buildings; narrow the reviewed showcase bounds".into(),
            );
        }
        let url = page_url(&sources, sources.rows);
        let (path, receipt) = transport::fetch(remote, &url, &request.cache.join("objects"), PAGE_BYTES)?;
        let rows: Vec<Value> = download::json(&path)?;
        if rows.len() > PAGE_ROWS {
            return Err("Socrata page exceeds requested row count".into());
        }
        sources.rows += rows.len();
        sources.pages.push(receipt);
        download::write_json(&request.cache.join("footprint-sources.json"), &sources)?;
        if rows.len() < PAGE_ROWS {
            break;
        }
    }
    // A second, independently acquired metadata object detects edits during offset pagination.
    let (path, receipt) = transport::fetch(
        remote,
        METADATA,
        &request.cache.join("final-metadata"),
        PAGE_BYTES,
    )?;
    if revision(&download::json::<Value>(&path)?)? != sources.revision {
        return Err("NYC dataset changed during pagination; use a fresh cache and fetch again".into());
    }
    sources.final_metadata = Some(receipt);
    sources.complete = true;
    download::write_json(&request.cache.join("footprint-sources.json"), &sources)?;
    Ok(sources)
}
fn verified(request: &Request, source: &Source, final_metadata: bool) -> Result<std::path::PathBuf, String> {
    let path = request
        .cache
        .join(if final_metadata {
            "final-metadata"
        } else {
            "objects"
        })
        .join(download::sha256(source.url.as_bytes()));
    let (bytes, sha) = transport::digest(&path)?;
    if bytes != source.bytes || sha != source.sha256 || bytes > PAGE_BYTES {
        return Err("NYC source differs from its acquisition receipt".into());
    }
    Ok(path)
}
fn number(row: &Value, field: &str) -> Option<f64> {
    let v = row.get(field)?;
    v.as_f64()
        .or_else(|| v.as_str()?.parse::<f64>().ok())
        .filter(|n| n.is_finite())
}
fn rings(polygon: &Value) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let mut rings = Vec::new();
    let values = polygon.as_array().ok_or("Invalid GeoJSON polygon")?;
    if values.len() > 256 {
        return Err("Footprint ring budget exceeded".into());
    }
    let mut count = 0;
    for value in values {
        let values = value.as_array().ok_or("Invalid GeoJSON ring")?;
        count += values.len();
        if count > 4096 {
            return Err("Footprint vertex budget exceeded".into());
        }
        let mut ring = Vec::new();
        for p in values {
            let p = p.as_array().ok_or("Invalid GeoJSON coordinate")?;
            if p.len() != 2 {
                return Err("Socrata footprint must be longitude/latitude in WGS84".into());
            }
            let lon = p
                .first()
                .and_then(Value::as_f64)
                .ok_or("Invalid footprint longitude")?;
            let lat = p
                .get(1)
                .and_then(Value::as_f64)
                .ok_or("Invalid footprint latitude")?;
            ring.push([lat, lon]);
        }
        if ring.len() < 4 || ring.first() != ring.last() {
            return Err("Footprint ring is not closed".into());
        }
        rings.push(ring);
    }
    if rings.is_empty() {
        return Err("Empty footprint polygon".into());
    }
    Ok(rings)
}
fn extrude(
    rings: &[Vec<[f64; 2]>],
    base: f64,
    roof: f64,
    inverse: DMat4,
) -> Result<Vec<city::Triangle>, String> {
    let bottom: Vec<Vec<_>> = rings
        .iter()
        .map(|r| {
            r.iter()
                .map(|[lat, lon]| city_mesh::local(inverse, *lat, *lon, base))
                .collect::<Result<_, _>>()
        })
        .collect::<Result<_, _>>()?;
    let top: Vec<Vec<_>> = rings
        .iter()
        .map(|r| {
            r.iter()
                .map(|[lat, lon]| city_mesh::local(inverse, *lat, *lon, roof))
                .collect::<Result<_, _>>()
        })
        .collect::<Result<_, _>>()?;
    // Geographic roofs follow Earth curvature; tessellate in the common projected footprint plane,
    // then retain each interpolated roof height. The small per-building curvature is below 10 cm.
    let mut triangles = city_mesh::triangulate(&top)?;
    for (bottom, top) in bottom.iter().zip(&top) {
        for (b, t) in bottom.windows(2).zip(top.windows(2)) {
            triangles.extend([[b[0], b[1], t[1]], [b[0], t[1], t[0]]]);
        }
    }
    Ok(triangles)
}
/// fc-scenery may supply one measured building as ordinary OGC 3D Tiles/GLB. The source row ID
/// and public-domain source receipts must accompany it. No TUM output may enter this path.
fn measured(request: &Request, id: &str, inverse: DMat4) -> Result<Option<Vec<city::Triangle>>, String> {
    let directory = request
        .cache
        .join("lidar-roofs")
        .join(download::sha256(id.as_bytes()));
    let manifest = directory.join("tileset.json");
    if !manifest.exists() {
        return Ok(None);
    }
    let bytes = download::read(&manifest, tiles3d::MAX_TILESET_BYTES)?;
    let set = Tileset::from_json(&bytes).map_err(|e| e.to_string())?;
    let metadata = set
        .extras
        .as_ref()
        .and_then(|v| v.get("flightcraft"))
        .ok_or("Lidar building needs source metadata")?;
    if metadata.get("sourceRowId").and_then(Value::as_str) != Some(id)
        || metadata.get("buildingReconstruction").and_then(Value::as_bool) != Some(true)
        || metadata.get("redistributionAllowed").and_then(Value::as_bool) != Some(true)
        || metadata.get("localOnly").and_then(Value::as_bool) != Some(false)
    {
        return Err("Lidar building identity, reconstruction, or redistribution policy is unverified".into());
    }
    let receipts: Vec<Source> = serde_json::from_value(
        metadata
            .get("sourceReceipts")
            .ok_or("Lidar building needs receipts")?
            .clone(),
    )
    .map_err(|e| e.to_string())?;
    if receipts.is_empty()
        || receipts.iter().any(|s| {
            !transport::allowed_url(&s.url)
                || s.sha256.len() != 64
                || !s.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || s.bytes == 0
        })
    {
        return Err("Lidar roofs require reviewed US public-domain source receipts".into());
    }
    if !set.root.children.is_empty() {
        return Err("Lidar building interchange requires a single complete building GLB".into());
    }
    let content = set.root.content.ok_or("Lidar building GLB missing")?;
    let path = directory
        .join(&content.uri)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !path.starts_with(directory.canonicalize().map_err(|e| e.to_string())?) {
        return Err("Lidar building path escapes its source directory".into());
    }
    let transform = DMat4::from_cols_array(
        &set.root
            .transform
            .ok_or("Lidar building requires an ECEF frame")?,
    ) * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
    let model = city::Model {
        url: content.uri,
        gltf_to_ecef: transform.to_cols_array(),
    };
    let triangles = city::triangles(
        &download::read(&path, tiles3d::MAX_TILE_CONTENT_BYTES)?,
        &model,
        inverse,
    )?;
    Ok(Some(triangles))
}
fn build(request: &Request, out: &Path) -> Result<FootprintReport, String> {
    let sources: FootprintSources = download::json(&request.cache.join("footprint-sources.json"))?;
    if sources.dataset != DATASET
        || sources.bounds != BOUNDS
        || sources.terms_url != TERMS
        || sources.redistribution_allowed
        || !sources.local_only
        || !sources.complete
        || sources.pages.is_empty()
        || sources.pages.len() > 250
        || sources.metadata.url != METADATA
        || sources.final_metadata.as_ref().is_none_or(|s| s.url != METADATA)
    {
        return Err("NYC acquisition is incomplete or does not match the showcase source plan".into());
    }
    let metadata: Value = download::json(&verified(request, &sources.metadata, false)?)?;
    if revision(&metadata)? != sources.revision
        || revision(&download::json::<Value>(&verified(
            request,
            sources.final_metadata.as_ref().ok_or("Missing final metadata")?,
            true,
        )?)?)?
            != sources.revision
        || field(&metadata, &["the_geom", "multipolygon"], Some("multipolygon"))? != sources.geometry_field
        || field(&metadata, &["heightroof", "height_roof"], Some("number"))? != sources.roof_field
        || field(&metadata, &["groundelev", "ground_elevation"], Some("number"))? != sources.ground_field
    {
        return Err("NYC source metadata differs from its reviewed revision".into());
    }
    let review:FootprintReview=download::json(&request.cache.join("footprint-review.json")).map_err(|_|"NYC build needs footprint-review.json: review the fetched height units, roof reference, and vertical datum using footprint-review.example.json")?;
    if review.metadata_sha256 != sources.metadata.sha256
        || review.evidence.trim().is_empty()
        || review.evidence.len() > 8192
        || review.ground_datum != "NAVD88"
        || !review.presentation_height_offset_m.is_finite()
        || review.presentation_height_offset_m.abs() > 100.
    {
        return Err("NYC height review must match the source metadata and document NAVD88/presentation MSL registration".into());
    }
    let unit = review
        .height_unit
        .ok_or("Verify NYC height units in footprint-review.json")?
        .meters();
    let reference = review
        .roof_reference
        .ok_or("Verify whether roof heights are absolute or above ground")?;
    let (_, inverse) = city_mesh::frame()?;
    let mut writer = city_mesh::Writer::new(out, city_mesh::WEB_BUILDING_BUDGET)?;
    let mut seen = BTreeSet::new();
    let (mut buildings, mut missing, mut lidar, mut rows) = (0, 0, 0, 0);
    let mut roof_sources = Vec::new();
    let mut missing_rows = Vec::new();
    for (page_index, page) in sources.pages.iter().enumerate() {
        if page.url != page_url(&sources, rows) {
            return Err("NYC page URL differs from bounded acquisition plan".into());
        }
        let page_rows: Vec<Value> = download::json(&verified(request, page, false)?)?;
        if page_rows.len() > PAGE_ROWS
            || (page_rows.len() != PAGE_ROWS && page_index + 1 != sources.pages.len())
        {
            return Err("NYC page lengths are inconsistent".into());
        }
        rows += page_rows.len();
        for row in page_rows {
            let id = row
                .get("source_row_id")
                .and_then(Value::as_str)
                .ok_or("NYC row lacks stable source ID")?;
            if id.is_empty() || id.len() > 128 || !seen.insert(id.to_owned()) {
                return Err("NYC acquisition has duplicate or invalid row IDs".into());
            }
            let (Some(ground), Some(height)) = (
                number(&row, &sources.ground_field),
                number(&row, &sources.roof_field),
            ) else {
                missing_rows.push(json!({"sourceRowId":id,"reason":"missing-or-invalid-height"}));
                missing += 1;
                continue;
            };
            let base = ground * unit + review.presentation_height_offset_m;
            let roof = match reference {
                RoofReference::Absolute => height * unit + review.presentation_height_offset_m,
                RoofReference::AboveGround => base + height * unit,
            };
            if !(0.5..=1000.).contains(&(roof - base)) || !(-100.0..=500.).contains(&base) || roof > 1200. {
                missing_rows.push(json!({"sourceRowId":id,"reason":"missing-or-invalid-height"}));
                missing += 1;
                continue;
            }
            let geometry = row
                .get(&sources.geometry_field)
                .ok_or("NYC row missing geometry")?;
            if geometry.get("type").and_then(Value::as_str) != Some("MultiPolygon") {
                return Err("NYC geometry must be WGS84 GeoJSON MultiPolygon".into());
            }
            let polygons = geometry
                .get("coordinates")
                .and_then(Value::as_array)
                .ok_or("Missing footprint polygons")?;
            if polygons.is_empty() || polygons.len() > 256 {
                return Err("NYC polygon count exceeds building budget".into());
            }
            let mut triangles = Vec::new();
            for polygon in polygons {
                triangles.extend(extrude(&rings(polygon)?, base, roof, inverse)?);
                if triangles.len() > 200_000 {
                    return Err("NYC building exceeds triangle budget".into());
                }
            }
            if let Some(measured) = measured(request, id, inverse)? {
                // Reject a measured reconstruction assigned to the wrong footprint or datum.
                let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
                for p in triangles.iter().flatten() {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
                if measured.iter().flatten().any(|p| {
                    p.x < lo.x - 2.
                        || p.x > hi.x + 2.
                        || p.z < lo.z - 2.
                        || p.z > hi.z + 2.
                        || p.y < lo.y - 2.
                        || p.y > hi.y + 50.
                }) {
                    return Err("Lidar reconstruction does not align with its NYC footprint".into());
                }
                let (mut measured_low, mut measured_high) =
                    (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
                for p in measured.iter().flatten() {
                    measured_low = measured_low.min(*p);
                    measured_high = measured_high.max(*p);
                }
                if measured_low.y > lo.y + 2.
                    || measured_high.y - lo.y < (hi.y - lo.y) * 0.25
                    || measured_low.x > lo.x + 2.
                    || measured_high.x < hi.x - 2.
                    || measured_low.z > lo.z + 2.
                    || measured_high.z < hi.z - 2.
                {
                    return Err("Lidar input must reconstruct the complete building, including its walls and footprint extent".into());
                }
                let source = request
                    .cache
                    .join("lidar-roofs")
                    .join(download::sha256(id.as_bytes()))
                    .join("tileset.json");
                roof_sources.push(json!({"sourceRowId":id,"tileset":download::json::<Value>(&source)?}));
                triangles = measured;
                lidar += 1;
            }
            writer.push(&triangles)?;
            buildings += 1;
        }
    }
    if rows != sources.rows {
        return Err("NYC row count differs from receipts".into());
    }
    // Geometric acceptance cannot grant redistribution rights under NYC/agency terms.
    let extras = json!({"flightcraft":{"region":"new-york","area":"nyc-footprints","bounds":BOUNDS,"localOnly":true,"redistributionAllowed":false,"previewOnly":true,"verticalDatumVerified":review.vertical_datum_verified,"runwayAlignmentVerified":review.runway_alignment_verified,"roofPolicy":"measured-where-available-flat-fallback","buildingCount":buildings,"flatRoofBuildings":buildings-lidar,"lidarRoofBuildings":lidar,"missingHeightRows":missing,"lidarBuilt":lidar>0,"sourceMetadataSha256":sources.metadata.sha256}});
    let (_, _, tiles) = writer.finish(CREDIT, extras)?;
    download::write_json(&out.join("footprint-sources.json"), &sources)?;
    download::write_json(&out.join("footprint-review.json"), &review)?;
    download::write_json(&out.join("source-metadata.json"), &metadata)?;
    download::write_json(&out.join("lidar-roof-sources.json"), &roof_sources)?;
    download::write_json(&out.join("missing-height-rows.json"), &missing_rows)?;
    // Include provenance and the report itself, not only content, in the byte ceiling.
    let package_bytes = super::super::data::files(out)?.iter().try_fold(0_u64, |n, p| {
        std::fs::metadata(p)
            .map(|m| n + m.len())
            .map_err(|e| e.to_string())
    })?;
    let mut report = FootprintReport {
        region: "new-york".into(),
        area: "nyc-footprints".into(),
        manifest: request.out.join("tileset.json"),
        bounds: BOUNDS,
        buildings,
        flat_roof_buildings: buildings - lidar,
        lidar_roof_buildings: lidar,
        missing_height_rows: missing,
        tiles,
        package_bytes,
        building_budget_bytes: city_mesh::WEB_BUILDING_BUDGET,
        total_web_budget_bytes: 1_000_000_000,
        local_only: true,
        redistribution_allowed: false,
        preview_only: true,
        vertical_datum_verified: review.vertical_datum_verified,
        runway_alignment_verified: review.runway_alignment_verified,
        release_ready: false,
    };
    for _ in 0..4 {
        let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
        let count = package_bytes + bytes.len() as u64 + 1;
        if count == report.package_bytes {
            break;
        }
        report.package_bytes = count;
    }
    if report.package_bytes > city_mesh::WEB_BUILDING_BUDGET {
        return Err("NYC package and provenance exceed the 250 MB building allocation".into());
    }
    download::write_json(&out.join("build-report.json"), &report)?;
    Ok(report)
}
pub(super) fn run(request: &Request) -> Result<(), String> {
    if request.sample.is_some() {
        return Err("NYC buildings do not accept an imagery sample".into());
    }
    match request.operation {
        Operation::Fetch => {
            let sources = fetch(request, &mut transport::Http::for_host(HOST))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&sources).map_err(|e| e.to_string())?
            );
        }
        Operation::Build => {
            if request.out.exists() {
                return Err("NYC output must be a new directory".into());
            }
            let staging = request.out.with_extension("building");
            if let Some(parent) = staging.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
            match build(request, &staging) {
                Ok(report) => {
                    std::fs::rename(&staging, &request.out).map_err(|e| e.to_string())?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
                    );
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(staging);
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) struct Fake {
        pub(super) calls: usize,
        pub(super) changed: bool,
    }
    fn metadata(revision: u64) -> Value {
        json!({"id":DATASET,"name":"Building Footprints","rowsUpdatedAt":revision,"columns":[{"fieldName":"the_geom","dataTypeName":"multipolygon"},{"fieldName":"heightroof","dataTypeName":"number","description":"Original fixture roof height above ground in international feet"},{"fieldName":"groundelev","dataTypeName":"number","description":"Original fixture NAVD88 elevation in international feet"}]})
    }
    pub(super) fn row() -> Value {
        json!({"source_row_id":"fixture-building","heightroof":"100","groundelev":"10","the_geom":{"type":"MultiPolygon","coordinates":[[[[-74.0,40.75],[-73.9998,40.75],[-73.9998,40.7502],[-74.0,40.7502],[-74.0,40.75]],[[-73.99995,40.75005],[-73.99985,40.75005],[-73.99985,40.75015],[-73.99995,40.75015],[-73.99995,40.75005]]]]}})
    }
    impl transport::Remote for Fake {
        fn allows(&self, url: &str) -> bool {
            transport::exact_host(url, HOST)
        }
        fn open(&mut self, url: &str, offset: u64, _: Option<&str>) -> Result<transport::Response, String> {
            assert_eq!(offset, 0);
            self.calls += 1;
            let data = if url == METADATA {
                metadata(if self.changed && self.calls > 1 { 2 } else { 1 })
            } else {
                assert!(url.contains("within_box%28the_geom"));
                assert!(url.ends_with("$offset=0"));
                json!([row()])
            };
            Ok(transport::Response {
                status: 200,
                validator: Some("\"fixture\"".into()),
                range: None,
                reader: Box::new(std::io::Cursor::new(serde_json::to_vec(&data).unwrap())),
            })
        }
    }
    pub(super) fn fixture_request(name: &str) -> Request {
        let root = std::env::temp_dir().join(format!("fc-footprints-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Request {
            operation: Operation::Build,
            fetch_layer: super::super::FetchLayer::All,
            lidar_source: super::super::LidarSource::Ept,
            region: "new-york".into(),
            area: "nyc-footprints".into(),
            cache: root.join("cache"),
            out: root.join("built"),
            sample: None,
            allow_http: false,
        }
    }
    pub(super) fn review(request: &Request, sources: &FootprintSources) {
        download::write_json(
            &request.cache.join("footprint-review.json"),
            &FootprintReview {
                metadata_sha256: sources.metadata.sha256.clone(),
                height_unit: Some(HeightUnit::Feet),
                roof_reference: Some(RoofReference::AboveGround),
                ground_datum: "NAVD88".into(),
                evidence: "Original synthetic metadata, feet above ground; no source-data acceptance claim"
                    .into(),
                presentation_height_offset_m: 0.,
                vertical_datum_verified: true,
                runway_alignment_verified: true,
            },
        )
        .unwrap();
    }
    #[test]
    fn acquisition_resumes_and_extrusion_keeps_real_heights_holes_lods_and_budget() {
        let request = fixture_request("build");
        let mut fake = Fake {
            calls: 0,
            changed: false,
        };
        let sources = fetch(&request, &mut fake).unwrap();
        assert_eq!(fake.calls, 3);
        assert_eq!(sources.rows, 1);
        fake.calls = 0;
        fetch(&request, &mut fake).unwrap();
        assert_eq!(fake.calls, 0);
        let stage = request.out.with_extension("building");
        std::fs::create_dir(&stage).unwrap();
        assert!(build(&request, &stage).is_err());
        review(&request, &sources);
        let report = build(&request, &stage).unwrap();
        assert_eq!(report.buildings, 1);
        assert_eq!(report.flat_roof_buildings, 1);
        assert_eq!(report.lidar_roof_buildings, 0);
        assert!(!report.release_ready);
        assert!(report.local_only);
        assert!(!report.redistribution_allowed);
        let metadata: Value = download::json(&stage.join("tileset.json")).unwrap();
        assert!(crate::data::validate_scenery_release(&metadata).is_err());
        assert!(
            flightcraft_render::scenery::validate_static_scenery(
                &Tileset::from_json(&serde_json::to_vec(&metadata).unwrap()).unwrap()
            )
            .is_err()
        );
        assert!(report.package_bytes < 250_000_000);
        assert!(!stage.join("scratch").exists());
        let actual: u64 = crate::data::files(&stage)
            .unwrap()
            .iter()
            .map(|p| std::fs::metadata(p).unwrap().len())
            .sum();
        assert_eq!(actual, report.package_bytes);
        let set = Tileset::from_json(
            &download::read(&stage.join("tileset.json"), tiles3d::MAX_TILESET_BYTES).unwrap(),
        )
        .unwrap();
        let mut pending = vec![&set.root];
        let mut errors = Vec::new();
        while let Some(node) = pending.pop() {
            errors.push(node.geometric_error);
            if let Some(content) = &node.content {
                let bytes =
                    download::read(&stage.join(&content.uri), tiles3d::MAX_TILE_CONTENT_BYTES).unwrap();
                flightcraft_render::scenery::decode_glb(
                    &bytes,
                    set.root.transform.unwrap(),
                    [40.96905613156273, -75.00242278301268],
                )
                .unwrap();
            }
            pending.extend(&node.children);
        }
        assert!(errors.contains(&0.));
        assert!(errors.contains(&4.));
        assert!(errors.contains(&1024.));
        // Independently inspect source extrusion, including a courtyard that must remain empty.
        let geometry = row();
        let polygon = &geometry["the_geom"]["coordinates"][0];
        let (_, inverse) = city_mesh::frame().unwrap();
        let mesh = extrude(&rings(polygon).unwrap(), 3.048, 33.528, inverse).unwrap();
        let matrix = DMat4::from_cols_array(&city_mesh::frame().unwrap().0)
            * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
        for p in mesh.iter().flatten() {
            let [_, _, h] = tiles3d::geodetic(matrix.transform_point3(*p).to_array()).unwrap();
            assert!((h - 3.048).abs() < 0.001 || (h - 33.528).abs() < 0.001);
        }
        std::fs::remove_dir_all(request.cache.parent().unwrap()).unwrap();
    }
    #[test]
    fn revision_change_duplicate_rows_and_tampered_receipts_cannot_ship() {
        let request = fixture_request("revision");
        let error = fetch(
            &request,
            &mut Fake {
                calls: 0,
                changed: true,
            },
        )
        .unwrap_err();
        assert!(error.contains("changed during"));
        let sources: FootprintSources =
            download::json(&request.cache.join("footprint-sources.json")).unwrap();
        assert!(!sources.complete);
        std::fs::remove_dir_all(request.cache.parent().unwrap()).unwrap();
        let request = fixture_request("tamper");
        let sources = fetch(
            &request,
            &mut Fake {
                calls: 0,
                changed: false,
            },
        )
        .unwrap();
        review(&request, &sources);
        let page = verified(&request, &sources.pages[0], false).unwrap();
        std::fs::write(page, b"[]").unwrap();
        std::fs::create_dir(&request.out).unwrap();
        assert!(build(&request, &request.out).unwrap_err().contains("receipt"));
        std::fs::remove_dir_all(request.cache.parent().unwrap()).unwrap();
    }
    #[test]
    fn city_writer_hard_budget_fails_instead_of_dropping_buildings() {
        let request = fixture_request("budget");
        std::fs::create_dir_all(&request.out).unwrap();
        let (_, inverse) = city_mesh::frame().unwrap();
        let p = city_mesh::local(inverse, 40.75, -74., 10.).unwrap();
        let mut writer = city_mesh::Writer::new(&request.out, 1).unwrap();
        writer
            .push(&[[p, p + DVec3::X * 10., p + DVec3::Y * 20.]])
            .unwrap();
        assert!(writer.finish("Original fixture", json!({})).is_err());
        std::fs::remove_dir_all(request.cache.parent().unwrap()).unwrap();
    }
}

#[cfg(test)]
mod pagination_tests {
    use super::*;
    struct Pages {
        metadata: super::tests::Fake,
        calls: usize,
        duplicate: bool,
    }
    impl transport::Remote for Pages {
        fn allows(&self, url: &str) -> bool {
            transport::exact_host(url, HOST)
        }
        fn open(
            &mut self,
            url: &str,
            offset: u64,
            validator: Option<&str>,
        ) -> Result<transport::Response, String> {
            self.calls += 1;
            if url == METADATA {
                return self.metadata.open(url, offset, validator);
            }
            assert_eq!(offset, 0);
            assert!(url.contains("$order=%3Aid&$limit=1000"));
            let count = if self.duplicate {
                2
            } else if url.ends_with("$offset=0") {
                1000
            } else {
                assert!(url.ends_with("$offset=1000"));
                1
            };
            let mut rows = Vec::new();
            for n in 0..count {
                let mut row = super::tests::row();
                if !self.duplicate {
                    row["source_row_id"] = json!(format!("original-{n}-{}", url.ends_with("$offset=0")));
                }
                rows.push(row);
            }
            Ok(transport::Response {
                status: 200,
                validator: Some("\"page-fixture\"".into()),
                range: None,
                reader: Box::new(std::io::Cursor::new(
                    serde_json::to_vec(&rows).map_err(|e| e.to_string())?,
                )),
            })
        }
    }
    #[test]
    fn socrata_fetch_paginates_in_stable_order_and_duplicate_rows_fail_the_build() {
        let request = super::tests::fixture_request("pages");
        let mut remote = Pages {
            metadata: super::tests::Fake {
                calls: 0,
                changed: false,
            },
            calls: 0,
            duplicate: false,
        };
        let sources = fetch(&request, &mut remote).unwrap();
        assert_eq!(sources.rows, 1001);
        assert_eq!(sources.pages.len(), 2);
        assert_eq!(remote.calls, 4);
        remote.calls = 0;
        fetch(&request, &mut remote).unwrap();
        assert_eq!(remote.calls, 0);
        std::fs::remove_dir_all(request.cache.parent().unwrap()).unwrap();
        let request = super::tests::fixture_request("duplicates");
        remote.duplicate = true;
        let sources = fetch(&request, &mut remote).unwrap();
        super::tests::review(&request, &sources);
        std::fs::create_dir(&request.out).unwrap();
        assert!(build(&request, &request.out).unwrap_err().contains("duplicate"));
        std::fs::remove_dir_all(request.cache.parent().unwrap()).unwrap();
    }
}

#[cfg(test)]
mod lidar_interchange_tests {
    use super::*;
    #[test]
    fn measured_building_uses_the_same_glb_path_and_rejects_local_only_derivatives() {
        let request = super::tests::fixture_request("lidar");
        let sources = fetch(
            &request,
            &mut super::tests::Fake {
                calls: 0,
                changed: false,
            },
        )
        .unwrap();
        super::tests::review(&request, &sources);
        let (_, inverse) = city_mesh::frame().unwrap();
        let row = super::tests::row();
        let mut triangles = extrude(
            &rings(&row["the_geom"]["coordinates"][0]).unwrap(),
            3.048,
            33.528,
            inverse,
        )
        .unwrap();
        let minx = triangles
            .iter()
            .flatten()
            .map(|p| p.x)
            .fold(f64::INFINITY, f64::min);
        let earth = DMat4::from_cols_array(&city_mesh::frame().unwrap().0)
            * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
        for p in triangles.iter_mut().flatten() {
            let [_, _, h] = tiles3d::geodetic(earth.transform_point3(*p).to_array()).unwrap();
            if h > 20. {
                p.y += (p.x - minx) * 0.1;
            }
        }
        let directory = request
            .cache
            .join("lidar-roofs")
            .join(download::sha256(b"fixture-building"));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("building.glb"),
            city::encode(&triangles, 0.).unwrap(),
        )
        .unwrap();
        let (transform, _) = city_mesh::frame().unwrap();
        let mut tile = city::parent(Vec::new());
        tile.bounding_volume = city::cell_volume(&triangles, transform).unwrap();
        tile.transform = Some(transform);
        tile.content = Some(tiles3d::Content {
            uri: "building.glb".into(),
        });
        let mut set = Tileset {
            asset: tiles3d::Asset {
                version: "1.1".into(),
                copyright: Some("Original synthetic measured-roof fixture".into()),
            },
            geometric_error: 128.,
            root: tile,
            extensions_required: Vec::new(),
            extras: Some(
                json!({"flightcraft":{"sourceRowId":"fixture-building","buildingReconstruction":true,"redistributionAllowed":true,"localOnly":false,"sourceReceipts":[{"url":"https://prd-tnm.s3.amazonaws.com/original-lidar-fixture.laz","edition":"Original test fixture, not real lidar","downloaded_at":"2026-11-03T00:00:00Z","bytes":1,"sha256":download::sha256(b"original fixture")}]}}),
            ),
        };
        download::write_json(&directory.join("tileset.json"), &set).unwrap();
        std::fs::create_dir(&request.out).unwrap();
        let report = build(&request, &request.out).unwrap();
        assert_eq!(report.lidar_roof_buildings, 1);
        assert_eq!(report.flat_roof_buildings, 0);
        assert!(request.out.join("lidar-roof-sources.json").exists());
        set.extras.as_mut().unwrap()["flightcraft"]["localOnly"] = json!(true);
        download::write_json(&directory.join("tileset.json"), &set).unwrap();
        assert!(
            measured(&request, "fixture-building", inverse)
                .unwrap_err()
                .contains("redistribution")
        );
        std::fs::remove_dir_all(request.cache.parent().unwrap()).unwrap();
    }
}
