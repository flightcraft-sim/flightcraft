//! Build receipts include checks a reviewer can inspect, rather than only a success flag.
use super::download::{self, MAX_DOWNLOAD, read, write_json};
use flightcraft_world::{Airports, HeightTile, Region, World, geo::LocalProjection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub version: u8,
    pub dataset: String,
    pub counts: BTreeMap<String, u64>,
    pub bytes: u64,
    pub checks: Vec<Value>,
    pub procedure_parse_failures: Vec<String>,
    pub warnings: Vec<String>,
}
pub fn region(directory: &Path, inputs: &Path) -> Result<Report, String> {
    let region: Region = download::json(&directory.join("region.json"))?;
    let tiles = region
        .tiles
        .iter()
        .map(|name| {
            HeightTile::read(&read(
                &directory.join(name),
                flightcraft_world::MAX_REGION_TILE_BYTES,
            )?)
            .map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let terrain = World::from_region(region.clone(), tiles).map_err(|e| e.to_string())?;
    let airports: Airports = download::json(&directory.join("airports.json"))?;
    let projection = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
        .map_err(|e| e.to_string())?;
    let mut report = Report {
        version: 1,
        dataset: region.id.clone(),
        counts: BTreeMap::new(),
        bytes: 0,
        checks: Vec::new(),
        procedure_parse_failures: Vec::new(),
        warnings: Vec::new(),
    };
    report
        .counts
        .insert("airports".into(), airports.airports.len() as u64);
    report.counts.insert(
        "runways".into(),
        airports.airports.iter().map(|a| a.runways.len() as u64).sum(),
    );
    report
        .counts
        .insert("terrain_tiles".into(), region.tiles.len() as u64);
    for (file, keys) in [
        ("navdata.json", vec!["navaids", "fixes", "routes", "holds", "ils"]),
        ("procedures.json", vec!["procedures", "points"]),
        ("airspace.json", vec!["volumes"]),
    ] {
        let value: Value = download::json(&directory.join(file))?;
        for key in keys {
            report
                .counts
                .insert(key.into(), value[key].as_array().map_or(0, Vec::len) as u64);
        }
    }
    for airport in airports.airports.iter().take(32) {
        let [n, e] = projection
            .project(airport.latitude_deg, airport.longitude_deg)
            .map_err(|e| e.to_string())?;
        if let Some(sample) = terrain.terrain_ground(n, e) {
            let error = (sample.height_msl_m - airport.elevation_m).abs();
            report.checks.push(json!({"check":"airport_elevation","airport":airport.ident,"faa_m":airport.elevation_m,"terrain_m":sample.height_msl_m,"difference_m":error,"tolerance_m":15,"pass":error<=15.}));
            if error > 15. {
                report.warnings.push(format!(
                    "{}: FAA elevation and terrain differ by {error:.2} m",
                    airport.ident
                ));
            }
        } else {
            report.warnings.push(format!(
                "{}: no terrain sample at airport reference point",
                airport.ident
            ));
        }
    }
    let raw: Value = download::json(&inputs.join("Runways.geojson"))?;
    for airport in &airports.airports {
        for runway in &airport.runways {
            let Some(feature) = raw["features"].as_array().into_iter().flatten().find(|f| {
                f["properties"]["DESIGNATOR"].as_str() == Some(&runway.designator) && {
                    super::navdata_faa::centreline(f, &projection).is_ok_and(|[a, _]| {
                        let end = &runway.ends[0];
                        projection
                            .project(end.latitude_deg, end.longitude_deg)
                            .is_ok_and(|b| libm::hypot(a[0] - b[0], a[1] - b[1]) < runway.length_m + 10.)
                    })
                }
            }) else {
                report.warnings.push(format!(
                    "{} {}: no source polygon matched for report",
                    airport.ident, runway.designator
                ));
                continue;
            };
            let [a, b] = super::navdata_faa::centreline(feature, &projection)?;
            let measured = libm::hypot(a[0] - b[0], a[1] - b[1]);
            let ratio = (measured / runway.length_m - 1.).abs();
            report.checks.push(json!({"check":"runway_length","airport":airport.ident,"runway":runway.designator,"published_m":runway.length_m,"polygon_m":measured,"difference_fraction":ratio,"tolerance_fraction":0.05,"pass":ratio<=0.05}));
        }
    }
    for path in super::data::files(directory)? {
        report.bytes += std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    }
    Ok(report)
}
pub fn write(directory: &Path, report: &Report) -> Result<(), String> {
    write_json(&directory.join("validation.json"), report)?;
    let mut markdown = format!(
        "# Validation: {}\n\nDataset bytes: {}. Procedure parse failures: {}.\n\n| Type | Count |\n|---|---:|\n",
        report.dataset,
        report.bytes,
        report.procedure_parse_failures.len()
    );
    for (name, count) in &report.counts {
        markdown.push_str(&format!("| {name} | {count} |\n"));
    }
    markdown.push_str("\nElevation spot checks use up to 32 airports in identifier order after runway preparation, with a 15 m tolerance. Runway polygons are checked against published lengths with a 5% tolerance. Warnings require review; they are not silently corrected. JSON contains every measured value.\n");
    if report.checks.is_empty() {
        markdown.push_str("\nNo terrain checks apply to a national navigation build.\n");
    }
    for warning in &report.warnings {
        markdown.push_str(&format!("\n- {warning}\n"));
    }
    std::fs::write(directory.join("validation.md"), markdown).map_err(|e| e.to_string())
}

pub fn measure() -> Result<(), String> {
    let root = super::root()?;
    let mut rows = Vec::new();
    let (mut v1, mut compact, mut png, mut webp, mut pixels) = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
    for id in ["new-york", "eastern-pa"] {
        let directory = root.join("assets/terrain").join(id);
        let mut terrain_v1 = 0;
        let mut terrain_v2 = 0;
        let mut chart_png = 0;
        let mut chart_webp = 0;
        let mut chart_pixels = 0;
        for path in super::data::files(&directory)? {
            if path.extension().is_some_and(|e| e == "fcht") {
                let tile = HeightTile::read(&read(&path, flightcraft_world::MAX_REGION_TILE_BYTES)?)
                    .map_err(|e| e.to_string())?;
                terrain_v1 += tile.write().len() as u64;
                terrain_v2 += tile.write_compact().map_err(|e| e.to_string())?.len() as u64;
            } else if path.extension().is_some_and(|e| e == "png" || e == "webp") {
                let image =
                    image::load_from_memory(&read(&path, MAX_DOWNLOAD)?).map_err(|e| e.to_string())?;
                let mut p = std::io::Cursor::new(Vec::new());
                image
                    .write_to(&mut p, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
                let mut w = std::io::Cursor::new(Vec::new());
                image
                    .write_to(&mut w, image::ImageFormat::WebP)
                    .map_err(|e| e.to_string())?;
                let decoded = image::load_from_memory(w.get_ref()).map_err(|e| e.to_string())?;
                if decoded.into_rgba8() != image.to_rgba8() {
                    return Err("lossless chart round trip differs".into());
                }
                chart_png += p.get_ref().len() as u64;
                chart_webp += w.get_ref().len() as u64;
                chart_pixels += u64::from(image.width()) * u64::from(image.height());
            }
        }
        rows.push(json!({"region":id,"terrain_v1":terrain_v1,"terrain_v2":terrain_v2,"chart_png":chart_png,"chart_webp":chart_webp,"chart_pixels":chart_pixels}));
        v1 += terrain_v1;
        compact += terrain_v2;
        png += chart_png;
        webp += chart_webp;
        pixels += chart_pixels;
    }
    if v1 == 0 || png == 0 || pixels == 0 {
        return Err("fetch showcase data before measuring".into());
    }
    let terrain_ratio = compact as f64 / v1 as f64;
    let chart_ratio = webp as f64 / png as f64;
    let area = 9.83e12_f64;
    // 60 m samples, two bytes per height plus one cell surface before deflate.
    let terrain_projection = area / (60. * 60.) * 5. * terrain_ratio;
    let pixel_m = 156543.033928 * libm::cos(40_f64.to_radians()) / 2048.;
    let chart_projection =
        area / (pixel_m * pixel_m) * (1. + 0.25 + 0.0625 + 0.015625) * (webp as f64 / pixels as f64);
    let result = json!({"version":1,"regions":rows,"terrain_ratio":terrain_ratio,"chart_ratio":chart_ratio,"assumptions":{"area_m2":area,"spacing_m":60,"chart_max_zoom":11,"representative_latitude_deg":40,"overlap_factor":1},"projected_terrain_bytes":terrain_projection,"projected_chart_bytes":chart_projection,"projected_total_bytes":terrain_projection+chart_projection,"target_bytes":[5_000_000_000_u64,8_000_000_000_u64],"caveat":"Regional extrapolation, not a national inventory; excludes duplicate chart coverage, navigation, and archive overhead. Alaska, islands, relief, source coverage, and latitude change the result. Pages ships only showcase regions, with a hard 1 GB site cap."});
    write_json(&root.join("datasets/size-report.json"), &result)?;
    let text = format!(
        "# Data size projection\n\nMeasured terrain v2/v1 ratio: {terrain_ratio:.3}. Lossless WebP/PNG ratio: {chart_ratio:.3}.\n\nProjected national terrain: {:.2} GB; charts: {:.2} GB; total: {:.2} GB (decimal). Target: about 5–8 GB.\n\nThis extrapolates the two showcase regions across 9.83 million square kilometres at 60 m terrain spacing and chart zooms 8–11, with 40° representative latitude. It counts one chart coverage per point. Regional overlaps are not a national inventory: relief, Alaska, islands, chart duplication, and latitude can change these estimates substantially. The JSON records measured byte counts and assumptions. A full inventory must confirm the target as cells are added.\n\nCharts use the pure-Rust MIT/Apache-2.0 image-webp lossless encoder. Every measured tile is decoded again and compared byte-for-byte in RGBA. Terrain error is at most 0.125 m at stored samples; the original training valley stays exact.\n\nDesktop data comes from GitHub releases. Pages includes only the showcase regions and refuses a site reaching 1 GB. The data base URL is configurable without credentials; no additional hosting service is required.\n",
        terrain_projection / 1e9,
        chart_projection / 1e9,
        (terrain_projection + chart_projection) / 1e9
    );
    std::fs::write(root.join("docs/data-size.md"), text).map_err(|e| e.to_string())
}
