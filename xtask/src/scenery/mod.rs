//! Scenery acquisition is deliberately separate from the offline, reproducible build.
mod dem;
mod grid;
mod tiles;
mod transport;
use crate::download::{self, Source};
use flightcraft_world::scenery::{self, TileKey};
use grid::{ImageryGrid, SceneryFile, SceneryTile};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const NAIP: &str = "https://imagery.nationalmap.gov/arcgis/rest/services/USGSNAIPImagery/ImageServer";
const TNM: &str = "https://tnmaccess.nationalmap.gov/api/v1/products";
const ATTRIBUTION: &str = "USDA NAIP imagery distributed by USGS, public domain (US government data). Elevation: USGS 3DEP, public domain. For simulation only.";
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub operation: Operation,
    pub region: String,
    pub area: String,
    pub cache: PathBuf,
    pub out: PathBuf,
    /// Offline reference image, only for the documented 1N7 NAIP sample bbox.
    pub sample: Option<PathBuf>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    Fetch,
    Build,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    pub version: u8,
    pub region: String,
    pub area: String,
    pub bounds: [f64; 4],
    pub tile_pixels: u32,
    pub max_level: u8,
    pub ground_sample_m: f64,
    pub inputs: Vec<Input>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub kind: InputKind,
    pub key: Option<TileKey>,
    pub source: Source,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum InputKind {
    NaipMetadata,
    Imagery,
    DemDiscovery,
    Dem,
    LidarDiscovery,
    Lidar,
    ReferenceImage,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub region: String,
    pub area: String,
    pub manifest: PathBuf,
    pub imagery_tiles: usize,
    pub source_files: usize,
    pub preview_only: bool,
    pub elevation_built: bool,
    pub vertical_datum_verified: bool,
    pub runway_alignment_verified: bool,
    pub lidar_built: bool,
    pub release_ready: bool,
}

fn plan(region: &str, area: &str) -> Result<Sources, String> {
    if !matches!(region, "eastern-pa" | "new-york") {
        return Err("scenery currently supports eastern-pa and new-york".into());
    }
    // Square powers of two keep tile edges identical at every level. Larger presets include
    // padding beyond the requested envelopes; acquire the test first (see docs/scenery.md).
    let (lat, lon, gsd, max_level) = match area {
        "1n7-test" => (40.97113313888889, -74.99750405555555, 0.3, 3),
        "1n7" => (40.97113313888889, -74.99750405555555, 0.3, 7),
        "water-gap" => (40.990, -75.075, 0.6, 7),
        "surroundings" => (40.97113313888889, -74.99750405555555, 1.2, 7),
        _ => return Err("area must be 1n7-test, 1n7, water-gap, or surroundings".into()),
    };
    let [x, y] = scenery::mercator(lat, lon).map_err(|e| e.to_string())?;
    let width = gsd * 512. * f64::from(1_u32 << max_level) / libm::cos(lat.to_radians());
    Ok(Sources {
        version: 1,
        region: region.into(),
        area: area.into(),
        bounds: [x - width / 2., y - width / 2., x + width / 2., y + width / 2.],
        tile_pixels: 512,
        max_level,
        ground_sample_m: gsd,
        inputs: Vec::new(),
    })
}
fn manifest(s: &Sources, preview_only: bool) -> ImageryGrid {
    ImageryGrid {
        version: 1,
        region: s.region.clone(),
        area: s.area.clone(),
        bounds: s.bounds,
        tile_pixels: s.tile_pixels,
        max_level: s.max_level,
        ground_sample_m: s.ground_sample_m,
        preview_only,
        attribution: ATTRIBUTION.into(),
        tiles: Vec::new(),
    }
}
pub fn run() -> Result<(), String> {
    run_args(std::env::args().skip(2))
}
pub(super) fn run_args(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let operation=match args.next().as_deref() {Some("fetch")=>Operation::Fetch,Some("build")=>Operation::Build,_=>return Err("usage: cargo xtask scenery fetch|build --region <id> --area <name> [--cache <directory>] [--out <new-directory>] [--sample <jpg>]".into())};
    let mut values = BTreeMap::new();
    while let Some(key) = args.next() {
        if !["--region", "--area", "--cache", "--out", "--sample"].contains(&key.as_str()) {
            return Err(format!("unknown scenery option {key}"));
        }
        let value = args.next().ok_or("missing scenery option value")?;
        if values.insert(key, value).is_some() {
            return Err("duplicate scenery option".into());
        }
    }
    let region = values.remove("--region").ok_or("missing --region")?;
    let area = values
        .remove("--area")
        .ok_or("missing --area (start with 1n7-test)")?;
    plan(&region, &area)?;
    let base = PathBuf::from("/var/mnt/inland/builds/codex-jobs/scenery")
        .join(&region)
        .join(&area);
    let request = Request {
        operation,
        region,
        area,
        cache: values
            .remove("--cache")
            .map(PathBuf::from)
            .unwrap_or_else(|| base.join("cache")),
        out: values
            .remove("--out")
            .map(PathBuf::from)
            .unwrap_or_else(|| base.join("built")),
        sample: values.remove("--sample").map(PathBuf::from),
    };
    match request.operation {
        Operation::Fetch => {
            if request.sample.is_some() {
                return Err("--sample is only valid for an offline build".into());
            }
            let sources = fetch(&request, &mut transport::Http::default())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&sources).map_err(|e| e.to_string())?
            );
        }
        Operation::Build => println!(
            "{}",
            serde_json::to_string_pretty(&build(&request)?).map_err(|e| e.to_string())?
        ),
    }
    Ok(())
}
fn acquire(
    request: &Request,
    sources: &mut Sources,
    remote: &mut impl transport::Remote,
    kind: InputKind,
    key: Option<TileKey>,
    url: &str,
    limit: u64,
) -> Result<PathBuf, String> {
    let (path, source) = transport::fetch(remote, url, &request.cache.join("objects"), limit)?;
    sources.inputs.push(Input { kind, key, source });
    // A receipt is committed after every successful object. A rerun verifies objects and fills
    // the remainder; no successful image or large DEM is fetched again.
    download::write_json(&request.cache.join("sources.json"), sources)?;
    Ok(path)
}
fn fetch(request: &Request, remote: &mut impl transport::Remote) -> Result<Sources, String> {
    let mut sources = plan(&request.region, &request.area)?;
    let metadata = acquire(
        request,
        &mut sources,
        remote,
        InputKind::NaipMetadata,
        None,
        &format!("{NAIP}?f=json"),
        4 * 1024 * 1024,
    )?;
    let metadata: serde_json::Value = download::json(&metadata)?;
    if metadata.get("error").is_some() {
        return Err("NAIP returned an ArcGIS service error".into());
    }
    for name in ["maxImageWidth", "maxImageHeight"] {
        if metadata
            .get(name)
            .and_then(|v| v.as_u64())
            .is_none_or(|n| n < u64::from(sources.tile_pixels))
        {
            return Err(format!("NAIP {name} is smaller than the 512-pixel tile request"));
        }
    }
    let layout = manifest(&sources, false);
    let side = 1_u32 << sources.max_level;
    for y in 0..side {
        for x in 0..side {
            let key = TileKey {
                level: sources.max_level,
                x,
                y,
            };
            let [w, s, e, n] = layout.tile_bounds(key).map_err(|e| e.to_string())?;
            // Explicit Mercator bounds plus adjustAspectRatio=false prevent ArcGIS from changing
            // the returned image's geographic footprint to fit its pixel dimensions.
            let url = format!(
                "{NAIP}/exportImage?bbox={w:.9},{s:.9},{e:.9},{n:.9}&bboxSR=3857&imageSR=3857&size=512,512&format=jpg&interpolation=RSP_BilinearInterpolation&adjustAspectRatio=false&f=image"
            );
            let path = acquire(
                request,
                &mut sources,
                remote,
                InputKind::Imagery,
                Some(key),
                &url,
                8 * 1024 * 1024,
            )?;
            decode_image(&download::read(&path, 8 * 1024 * 1024)?, Some(512))?;
        }
    }
    let [w, s, e, n] = sources.bounds;
    let [south, west] = scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
    let [north, east] = scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
    let mut offset = 0_u64;
    let mut products = BTreeMap::new();
    loop {
        let url = format!(
            "{TNM}?datasets=Digital%20Elevation%20Model%20%28DEM%29%201%20meter&bbox={west:.9},{south:.9},{east:.9},{north:.9}&prodFormats=GeoTIFF&max=100&offset={offset}"
        );
        let path = acquire(
            request,
            &mut sources,
            remote,
            InputKind::DemDiscovery,
            None,
            &url,
            16 * 1024 * 1024,
        )?;
        let result: serde_json::Value = download::json(&path)?;
        let total = result
            .get("total")
            .and_then(|v| v.as_u64())
            .ok_or("TNM response missing total")?;
        if total > 10_000 {
            return Err("too many 3DEP products; narrow the area".into());
        }
        let items = result
            .get("items")
            .and_then(|v| v.as_array())
            .ok_or("TNM response missing items")?;
        if items.len() > 100 || offset.saturating_add(items.len() as u64) > total {
            return Err("inconsistent TNM pagination".into());
        }
        for item in items {
            let url = item
                .get("downloadURL")
                .and_then(|v| v.as_str())
                .ok_or("3DEP product missing downloadURL")?;
            if !transport::allowed_url(url) || !url.to_ascii_lowercase().ends_with(".tif") {
                return Err("3DEP product needs a USGS GeoTIFF URL".into());
            }
            products.insert(url.to_owned(), ());
        }
        offset += items.len() as u64;
        if offset >= total {
            break;
        }
        if items.is_empty() {
            return Err("TNM pagination ended before total".into());
        }
    }
    if products.is_empty() {
        return Err("no 1 m 3DEP products cover this area; retain the receipts and inspect coverage before enlarging the fetch".into());
    }
    for url in products.keys() {
        acquire(
            request,
            &mut sources,
            remote,
            InputKind::Dem,
            None,
            url,
            transport::MAX_OBJECT,
        )?;
    }
    // Download classified original point clouds. EPT project selection can be added from
    // the same reviewable metadata; no footprint source is substituted silently.
    let mut offset = 0_u64;
    let mut lidar = BTreeMap::new();
    loop {
        let url = format!(
            "{TNM}?datasets=Lidar%20Point%20Cloud%20%28LPC%29&bbox={west:.9},{south:.9},{east:.9},{north:.9}&prodFormats=LAZ&max=100&offset={offset}"
        );
        let path = acquire(
            request,
            &mut sources,
            remote,
            InputKind::LidarDiscovery,
            None,
            &url,
            16 * 1024 * 1024,
        )?;
        let result: serde_json::Value = download::json(&path)?;
        let total = result
            .get("total")
            .and_then(|v| v.as_u64())
            .ok_or("TNM lidar response missing total")?;
        if total > 10_000 {
            return Err("too many lidar products; narrow the area".into());
        }
        let items = result
            .get("items")
            .and_then(|v| v.as_array())
            .ok_or("TNM lidar response missing items")?;
        if items.len() > 100 || offset.saturating_add(items.len() as u64) > total {
            return Err("inconsistent TNM lidar pagination".into());
        }
        for item in items {
            let url = item
                .get("downloadURL")
                .and_then(|v| v.as_str())
                .ok_or("lidar product missing downloadURL")?;
            if !transport::allowed_url(url) || !url.to_ascii_lowercase().ends_with(".laz") {
                return Err("lidar product needs a USGS LAZ URL".into());
            }
            lidar.insert(url.to_owned(), ());
        }
        offset += items.len() as u64;
        if offset >= total {
            break;
        }
        if items.is_empty() {
            return Err("TNM lidar pagination ended before total".into());
        }
    }
    for url in lidar.keys() {
        acquire(
            request,
            &mut sources,
            remote,
            InputKind::Lidar,
            None,
            url,
            transport::MAX_OBJECT,
        )?;
    }
    Ok(sources)
}
fn decode_image(bytes: &[u8], size: Option<u32>) -> Result<image::RgbaImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    if size.is_some_and(|n| image.width() != n || image.height() != n) {
        return Err("NAIP returned unexpected dimensions".into());
    }
    Ok(image)
}
fn input_bytes(request: &Request, input: &Input) -> Result<Vec<u8>, String> {
    let bytes = download::read(
        &request
            .cache
            .join("objects")
            .join(download::sha256(input.source.url.as_bytes())),
        16 * 1024 * 1024,
    )?;
    if bytes.len() as u64 != input.source.bytes || download::sha256(&bytes) != input.source.sha256 {
        return Err("scenery input differs from source receipt".into());
    }
    Ok(bytes)
}
fn write_tile(out: &Path, key: TileKey, image: image::RgbaImage) -> Result<SceneryTile, String> {
    let path = format!("imagery/{}/m0.webp", key.name());
    let destination = out.join(&path);
    std::fs::create_dir_all(destination.parent().ok_or("tile parent")?).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
        .encode(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    std::fs::write(destination, &bytes).map_err(|e| e.to_string())?;
    Ok(SceneryTile {
        height_bounds: [0.; 2],
        key,
        mips: vec![SceneryFile {
            path,
            bytes: bytes.len() as u64,
            sha256: download::sha256(&bytes),
        }],
    })
}
fn build(request: &Request) -> Result<Report, String> {
    let expected = plan(&request.region, &request.area)?;
    let preview = request.sample.is_some();
    if preview && request.area != "1n7-test" {
        return Err("sample builds are limited to 1n7-test".into());
    }
    let mut sources = if preview {
        expected.clone()
    } else {
        download::json::<Sources>(&request.cache.join("sources.json"))?
    };
    if let Some(path) = &request.sample {
        let bytes = download::read(path, 8 * 1024 * 1024)?;
        sources.inputs.push(Input {kind:InputKind::ReferenceImage,key:None,source:Source {url:format!("{NAIP}/exportImage?bbox=-75.0130,40.9650,-74.9820,40.9775&bboxSR=4326&imageSR=3857&size=1600,860&f=image&format=jpg"),edition:Some("Reference supplied by flight lead; retrieval date unverified".into()),downloaded_at:download::now()?,bytes:bytes.len() as u64,sha256:download::sha256(&bytes)}});
    }
    if sources.version != 1
        || sources.region != expected.region
        || sources.area != expected.area
        || sources.bounds != expected.bounds
        || sources.max_level != expected.max_level
        || sources.tile_pixels != 512
        || sources.ground_sample_m != expected.ground_sample_m
        || sources.inputs.len() > 100_000
    {
        return Err("source plan differs from requested area".into());
    }
    if request.out.exists() {
        return Err("scenery build output must be a new directory".into());
    }
    let staging = request.out.with_extension("building");
    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    let result = build_inner(request, &sources, &staging);
    match result {
        Ok(mut report) => {
            std::fs::rename(&staging, &request.out).map_err(|e| e.to_string())?;
            report.manifest = request.out.join("tileset.json");
            Ok(report)
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(staging);
            Err(error)
        }
    }
}
fn build_inner(request: &Request, sources: &Sources, out: &Path) -> Result<Report, String> {
    let preview = request.sample.is_some();
    let mut layout = manifest(sources, preview);
    let mut dem = if preview {
        None
    } else {
        Some(dem::Dems::open(request, sources)?)
    };
    let sample = request
        .sample
        .as_ref()
        .map(|p| download::read(p, 8 * 1024 * 1024).and_then(|b| decode_image(&b, None)))
        .transpose()?;
    let mut leaves = BTreeMap::new();
    for input in sources.inputs.iter().filter(|i| i.kind == InputKind::Imagery) {
        let key = input.key.ok_or("imagery source missing key")?;
        if !key.valid() || key.level != sources.max_level || leaves.insert(key, input).is_some() {
            return Err("duplicate or invalid imagery source key".into());
        }
    }
    for level in (0..=sources.max_level).rev() {
        let side = 1_u32 << level;
        for y in 0..side {
            for x in 0..side {
                let key = TileKey { level, x, y };
                let mut image = if level == sources.max_level {
                    if let Some(sample) = &sample {
                        sample_tile(&layout, key, sample)?
                    } else {
                        decode_image(
                            &input_bytes(request, leaves.get(&key).ok_or("missing imagery source tile")?)?,
                            Some(512),
                        )?
                    }
                } else {
                    let mut mosaic = image::RgbaImage::new(1024, 1024);
                    for cy in 0..2 {
                        for cx in 0..2 {
                            let child = TileKey {
                                level: level + 1,
                                x: 2 * x + cx,
                                y: 2 * y + cy,
                            };
                            let image = decode_image(
                                &download::read(
                                    &out.join(format!("imagery/{}/m0.webp", child.name())),
                                    8 * 1024 * 1024,
                                )?,
                                Some(512),
                            )?;
                            image::imageops::replace(
                                &mut mosaic,
                                &image,
                                i64::from(cx * 512),
                                i64::from(cy * 512),
                            );
                        }
                    }
                    image::imageops::resize(&mosaic, 512, 512, image::imageops::FilterType::Triangle)
                };
                if level == sources.max_level {
                    delight(&mut image);
                }
                let mut tile = write_tile(out, key, image)?;
                tile.height_bounds = tiles::write_glb(
                    out,
                    &layout,
                    key,
                    [40.96905613156273, -75.00242278301268],
                    dem.as_mut(),
                )?;
                layout.tiles.push(tile);
            }
        }
    }
    layout.tiles.sort_by_key(|t| t.key);
    layout.validate().map_err(|e| e.to_string())?;
    download::write_json(
        &out.join("tileset.json"),
        &tiles::tileset(&layout, [40.96905613156273, -75.00242278301268])?,
    )?;
    std::fs::remove_dir_all(out.join("imagery")).map_err(|e| e.to_string())?;
    download::write_json(&out.join("sources.json"), sources)?;
    // Sampling a DEM is separate from accepting its horizontal/vertical registration and
    // reconstructed lidar objects. Keep the remaining acceptance gates explicit in output.
    let report = Report {
        region: request.region.clone(),
        area: request.area.clone(),
        manifest: request.out.join("tileset.json"),
        imagery_tiles: layout.tiles.len(),
        source_files: sources.inputs.len(),
        preview_only: preview,
        elevation_built: !preview,
        vertical_datum_verified: false,
        runway_alignment_verified: false,
        lidar_built: false,
        release_ready: false,
    };
    download::write_json(&out.join("build-report.json"), &report)?;
    Ok(report)
}
fn delight(image: &mut image::RgbaImage) {
    // Conservative global levels only: retain black markings and white roofs, lift the
    // darkest quarter very slightly. This cannot reconstruct detail hidden by baked shadows.
    for pixel in image.pixels_mut() {
        for c in pixel.0.iter_mut().take(3) {
            let v = f32::from(*c) / 255.;
            *c = ((v + 0.035 * v * (1. - v) * (1. - v)) * 255.)
                .round()
                .clamp(0., 255.) as u8;
        }
    }
}
fn sample_tile(
    layout: &ImageryGrid,
    key: TileKey,
    sample: &image::RgbaImage,
) -> Result<image::RgbaImage, String> {
    if sample.width() != 1600 || sample.height() != 860 {
        return Err("--sample requires the documented 1600×860 1N7 image".into());
    }
    let [w, s] = scenery::mercator(40.9650, -75.0130).map_err(|e| e.to_string())?;
    let [e, n] = scenery::mercator(40.9775, -74.9820).map_err(|e| e.to_string())?;
    let [tw, ts, te, tn] = layout.tile_bounds(key).map_err(|e| e.to_string())?;
    if tw < w || ts < s || te > e || tn > n {
        return Err("sample does not cover test tile".into());
    }
    Ok(image::RgbaImage::from_fn(512, 512, |x, y| {
        let mx = tw + (f64::from(x) + 0.5) / 512. * (te - tw);
        let my = tn - (f64::from(y) + 0.5) / 512. * (tn - ts);
        let px = ((mx - w) / (e - w) * f64::from(sample.width())).floor() as u32;
        let py = ((n - my) / (n - s) * f64::from(sample.height())).floor() as u32;
        sample
            .get_pixel_checked(px, py)
            .copied()
            .unwrap_or(image::Rgba([0, 0, 0, 255]))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn faa_thresholds_survive_image_pixels_ecef_and_tm_within_two_meters() {
        let layout = manifest(&plan("eastern-pa", "1n7-test").unwrap(), true);
        let origin = [40.96905613156273, -75.00242278301268];
        let projection = flightcraft_world::geo::LocalProjection::new(origin[0], origin[1]).unwrap();
        // Independently sourced FAA runway ends from the region airport catalogue. This tests
        // registration math; it does not claim that the painted pixels have been surveyed.
        for (lat, lon) in [
            (40.96905613156273, -75.00242278301268),
            (40.973226688232224, -74.99258941913907),
        ] {
            let key = TileKey { level: 0, x: 0, y: 0 };
            let [x, y] = layout.pixel(key, lat, lon).unwrap();
            let actual = layout
                .local_at_uv(key, [x / 512., y / 512.], &projection)
                .unwrap();
            let expected = projection.project(lat, lon).unwrap();
            assert!(libm::hypot(actual[0] - expected[0], actual[1] - expected[1]) < 0.001);
            let [latitude, longitude, height] = flightcraft_world::tiles3d::geodetic(
                flightcraft_world::tiles3d::ecef(lat, lon, 112.).unwrap(),
            )
            .unwrap();
            let [n, e] = projection.project(latitude, longitude).unwrap();
            assert!(libm::hypot(n - expected[0], e - expected[1]) < 0.001);
            assert!((height - 112.).abs() < 0.001);
        }
    }
    #[test]
    fn source_plan_rejects_unknown_areas_and_command_duplicates() {
        assert!(plan("../x", "1n7-test").is_err());
        assert!(plan("eastern-pa", "unknown").is_err());
        assert!(
            run_args(
                [
                    "fetch",
                    "--region",
                    "eastern-pa",
                    "--region",
                    "new-york",
                    "--area",
                    "1n7-test"
                ]
                .into_iter()
                .map(str::to_owned)
            )
            .is_err()
        );
        assert!(decode_image(b"{\"error\":{\"message\":\"ArcGIS error\"}}", Some(512)).is_err());
    }
    #[test]
    fn explicit_tree_has_one_content_per_node_and_standard_y_up_transform() {
        let layout = manifest(&plan("eastern-pa", "1n7-test").unwrap(), true);
        let set = tiles::tileset(&layout, [40.96905613156273, -75.00242278301268]).unwrap();
        let bytes = serde_json::to_vec(&set).unwrap();
        flightcraft_world::tiles3d::Tileset::from_json(&bytes).unwrap();
        let mut pending = vec![&set.root];
        let mut count = 0;
        while let Some(tile) = pending.pop() {
            count += 1;
            assert!(tile.content.as_ref().unwrap().uri.ends_with(".glb"));
            pending.extend(&tile.children);
        }
        assert_eq!(count, 85);
        assert_eq!(set.asset.version, "1.1");
    }
}

#[cfg(test)]
mod fetch_tests {
    use super::*;
    struct Recorded {
        calls: usize,
        jpeg: Vec<u8>,
    }
    impl transport::Remote for Recorded {
        fn open(&mut self, url: &str, offset: u64, _: Option<&str>) -> Result<transport::Response, String> {
            self.calls += 1;
            assert_eq!(offset, 0);
            let bytes = if url == format!("{NAIP}?f=json") {
                br#"{"maxImageWidth":1024,"maxImageHeight":1024}"#.to_vec()
            } else if url.starts_with(&format!("{NAIP}/exportImage?")) {
                assert!(url.contains("bboxSR=3857&imageSR=3857&size=512,512"));
                assert!(url.contains("adjustAspectRatio=false"));
                self.jpeg.clone()
            } else if url.starts_with(TNM) && url.contains("Lidar") {
                br#"{"total":1,"items":[{"downloadURL":"https://prd-tnm.s3.amazonaws.com/original-fixture.laz"}]}"#.to_vec()
            } else if url.starts_with(TNM) {
                if url.ends_with("offset=0") {
                    br#"{"total":2,"items":[{"downloadURL":"https://prd-tnm.s3.amazonaws.com/original-fixture-a.tif"}]}"#.to_vec()
                } else if url.ends_with("offset=1") {
                    br#"{"total":2,"items":[{"downloadURL":"https://prd-tnm.s3.amazonaws.com/original-fixture-b.tif"}]}"#.to_vec()
                } else {
                    return Err("unexpected TNM page".into());
                }
            } else if url.starts_with("https://prd-tnm.s3.amazonaws.com/original-fixture") {
                b"original acquisition fixture; not a real geographic product".to_vec()
            } else {
                return Err("unrecorded source".into());
            };
            Ok(transport::Response {
                status: 200,
                validator: Some("\"original-fixture\"".into()),
                range: None,
                reader: Box::new(std::io::Cursor::new(bytes)),
            })
        }
    }
    #[test]
    fn fetch_paginates_classified_lidar_and_dem_and_resumes_completed_objects() {
        let root = std::env::temp_dir().join(format!("fc-scenery-recorded-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let request = Request {
            operation: Operation::Fetch,
            region: "eastern-pa".into(),
            area: "1n7-test".into(),
            cache: root.join("cache"),
            out: root.join("out"),
            sample: None,
        };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&vec![80; 512 * 512 * 3], 512, 512, image::ExtendedColorType::Rgb8)
            .unwrap();
        let mut remote = Recorded { calls: 0, jpeg };
        let first = fetch(&request, &mut remote).unwrap();
        assert_eq!(remote.calls, 71);
        assert_eq!(
            first
                .inputs
                .iter()
                .filter(|i| i.kind == InputKind::Imagery)
                .count(),
            64
        );
        assert_eq!(
            first.inputs.iter().filter(|i| i.kind == InputKind::Dem).count(),
            2
        );
        assert_eq!(
            first.inputs.iter().filter(|i| i.kind == InputKind::Lidar).count(),
            1
        );
        assert!(
            first
                .inputs
                .iter()
                .all(|i| i.source.sha256.len() == 64 && i.source.bytes > 0)
        );
        remote.calls = 0;
        fetch(&request, &mut remote).unwrap();
        assert_eq!(remote.calls, 0);
        assert!(
            build(&Request {
                operation: Operation::Build,
                ..request
            })
            .is_err(),
            "invalid DEM payload cannot produce a completed tileset"
        );
        assert!(!root.join("out").exists());
        assert!(!root.join("out.building").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
