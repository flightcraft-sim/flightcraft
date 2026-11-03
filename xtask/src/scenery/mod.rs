//! Scenery acquisition is deliberately separate from the offline, reproducible build.
mod city;
mod city_crs;
mod city_mesh;
mod citygml;
mod footprints;
mod nyc;
pub use city::Review as CitySourceReview;
pub use citygml::{CityReport as CityGmlReport, CitySources as CityGmlSources};
pub use footprints::{FootprintReport, FootprintReview, FootprintSources};
pub use nyc::LidarReport as NycLidarReport;
mod building_mesh;
mod buildings;
mod datum;
mod dem;
mod ept;
mod geoid;
mod grid;
mod lidar;
mod lidar_ground;
mod objects;
mod surface;
mod tiles;
mod transport;
mod trees;
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
    #[serde(default)]
    pub fetch_layer: FetchLayer,
    #[serde(default)]
    pub lidar_source: LidarSource,
    pub region: String,
    pub area: String,
    pub cache: PathBuf,
    pub out: PathBuf,
    /// Offline reference image, only for the documented 1N7 NAIP sample bbox.
    pub sample: Option<PathBuf>,
    /// Explicit consent to the exact reviewed TUM HTTP prefix; never permits other HTTP URLs.
    #[serde(default)]
    pub allow_http: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    Fetch,
    Build,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FetchLayer {
    #[default]
    All,
    Terrain,
    Lidar,
    Geoid,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LidarSource {
    #[default]
    Ept,
    Rockyweb,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ept_selection: Option<ept::SelectionSummary>,
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
    EptMetadata,
    EptHierarchy,
    EptLidar,
    GeoidModel,
    GeoidControl,
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
    pub geoid_applied: bool,
    pub vertical_datum_verified: bool,
    pub runway_alignment_verified: bool,
    pub lidar_built: bool,
    pub lidar_points_decoded: u64,
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
        "nyc-test" if region == "new-york" => (40.7536, -73.9832, 0.6, 2),
        _ => return Err("area must be 1n7-test, 1n7, water-gap, surroundings, or new-york/nyc-test".into()),
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
        ept_selection: None,
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
    let operation=match args.next().as_deref() {Some("fetch")=>Operation::Fetch,Some("build")=>Operation::Build,_=>return Err("usage: cargo xtask scenery fetch|build --region <id> --area <name> [--cache <directory>] [--out <new-directory>] [--sample <jpg>] [--layer all|terrain|lidar|geoid] [--lidar-source ept|rockyweb] [--allow-http]".into())};
    let mut values = BTreeMap::new();
    while let Some(key) = args.next() {
        if key == "--allow-http" {
            if values.insert(key, "true".into()).is_some() {
                return Err("duplicate scenery option".into());
            }
            continue;
        }
        if ![
            "--region",
            "--area",
            "--cache",
            "--out",
            "--sample",
            "--layer",
            "--lidar-source",
        ]
        .contains(&key.as_str())
        {
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
    if !matches!(
        area.as_str(),
        "nyc-lod2" | "nyc-footprints" | "nyc-reviewed-glb" | "nyc-lidar"
    ) {
        plan(&region, &area)?;
    } else if region != "new-york" {
        return Err("NYC scenery requires the New York region".into());
    }
    let base = PathBuf::from("/var/mnt/inland/builds/codex-jobs/scenery")
        .join(&region)
        .join(&area);
    if matches!(operation, Operation::Build)
        && (values.contains_key("--layer") || values.contains_key("--lidar-source"))
    {
        return Err("--layer and --lidar-source are only valid for acquisition".into());
    }
    let fetch_layer = match values.remove("--layer").as_deref() {
        None | Some("all") => FetchLayer::All,
        Some("terrain") => FetchLayer::Terrain,
        Some("lidar") => FetchLayer::Lidar,
        Some("geoid") => FetchLayer::Geoid,
        _ => return Err("layer must be all, terrain, lidar, or geoid".into()),
    };
    let lidar_source = match values.remove("--lidar-source").as_deref() {
        None | Some("ept") => LidarSource::Ept,
        Some("rockyweb") => LidarSource::Rockyweb,
        _ => return Err("lidar source must be ept or rockyweb".into()),
    };
    let request = Request {
        operation,
        fetch_layer,
        lidar_source,
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
        allow_http: values.remove("--allow-http").is_some(),
    };
    if request.allow_http && !(request.area == "nyc-lod2" && matches!(request.operation, Operation::Fetch)) {
        return Err("--allow-http applies only to the reviewed TUM CityGML fetch".into());
    }
    match request.area.as_str() {
        "nyc-lod2" => return citygml::run(&request),
        "nyc-footprints" => return footprints::run(&request),
        "nyc-lidar" => return nyc::run(&request),
        "nyc-reviewed-glb" => return city::run(&request),
        _ => (),
    }
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
    let input = Input { kind, key, source };
    if let Some(previous) = sources
        .inputs
        .iter_mut()
        .find(|i| i.kind == input.kind && i.key == input.key && i.source.url == input.source.url)
    {
        *previous = input;
    } else {
        sources.inputs.push(input);
    }

    // A receipt is committed after every successful object. A rerun verifies objects and fills
    // the remainder; no successful image or large DEM is fetched again.
    write_sources(&request.cache, sources)?;
    Ok(path)
}
fn write_sources(cache: &Path, sources: &Sources) -> Result<(), String> {
    let staging = cache.join("sources.json.new");
    download::write_json(&staging, sources)?;
    std::fs::rename(staging, cache.join("sources.json")).map_err(|e| e.to_string())
}
struct EptCache<'a, R> {
    request: &'a Request,
    sources: &'a mut Sources,
    remote: &'a mut R,
}
impl<R: transport::Remote> ept::Acquisition for EptCache<'_, R> {
    fn metadata(&mut self, kind: ept::MetadataKind, url: &str, limit: usize) -> Result<Vec<u8>, String> {
        let kind = match kind {
            ept::MetadataKind::Root => InputKind::EptMetadata,
            ept::MetadataKind::Hierarchy => InputKind::EptHierarchy,
        };
        let path = acquire(
            self.request,
            self.sources,
            self.remote,
            kind,
            None,
            url,
            limit as u64,
        )?;
        download::read(&path, limit)
    }
    fn points(&mut self, url: &str) -> Result<(), String> {
        acquire(
            self.request,
            self.sources,
            self.remote,
            InputKind::EptLidar,
            None,
            url,
            transport::MAX_OBJECT,
        )?;
        Ok(())
    }
}
fn fetch(request: &Request, remote: &mut impl transport::Remote) -> Result<Sources, String> {
    std::fs::create_dir_all(&request.cache).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(request.cache.join(".fetch-lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|e| format!("another scenery fetch may be using this cache: {e}"))?;
    let planned = plan(&request.region, &request.area)?;
    let mut sources: Sources = if request.cache.join("sources.json").exists() {
        serde_json::from_slice(&download::read(
            &request.cache.join("sources.json"),
            64 * 1024 * 1024,
        )?)
        .map_err(|e| e.to_string())?
    } else {
        planned.clone()
    };
    if sources.version != planned.version
        || sources.region != planned.region
        || sources.area != planned.area
        || sources.bounds != planned.bounds
        || sources.tile_pixels != planned.tile_pixels
        || sources.max_level != planned.max_level
        || sources.ground_sample_m != planned.ground_sample_m
        || sources.inputs.len() > 100_000
    {
        return Err("existing scenery source catalog does not match this area plan".into());
    }
    let mut unique = std::collections::BTreeSet::new();
    if sources
        .inputs
        .iter()
        .any(|input| !unique.insert(&input.source.url))
    {
        return Err("duplicate URL in existing source catalog".into());
    }
    if matches!(request.fetch_layer, FetchLayer::All | FetchLayer::Terrain) {
        fetch_terrain(request, &mut sources, remote)?;
    }
    if matches!(request.fetch_layer, FetchLayer::All | FetchLayer::Lidar) {
        match request.lidar_source {
            LidarSource::Ept => {
                sources.ept_selection = None;
                write_sources(&request.cache, &sources)?;
                let selection = ept::fetch(
                    if request.region == "new-york"
                        && matches!(request.area.as_str(), "nyc-test" | "nyc-lidar")
                    {
                        ept::NYC_ROOT
                    } else {
                        ept::ROOT
                    },
                    sources.bounds,
                    0.5,
                    &mut EptCache {
                        request,
                        sources: &mut sources,
                        remote,
                    },
                )?;
                sources.ept_selection = Some(selection);
                write_sources(&request.cache, &sources)?;
            }
            LidarSource::Rockyweb => fetch_legacy_lidar(request, &mut sources, remote)?,
        }
    }
    if matches!(request.fetch_layer, FetchLayer::All | FetchLayer::Geoid) {
        geoid::fetch(request, &mut sources, remote)?;
    }
    Ok(sources)
}
fn fetch_terrain(
    request: &Request,
    sources: &mut Sources,
    remote: &mut impl transport::Remote,
) -> Result<(), String> {
    let metadata = acquire(
        request,
        sources,
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
    let layout = manifest(sources, false);
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
                sources,
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
            sources,
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
            sources,
            remote,
            InputKind::Dem,
            None,
            url,
            transport::MAX_OBJECT,
        )?;
    }
    Ok(())
}
fn fetch_legacy_lidar(
    request: &Request,
    sources: &mut Sources,
    remote: &mut impl transport::Remote,
) -> Result<(), String> {
    let [w, s, e, n] = sources.bounds;
    let [south, west] = scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
    let [north, east] = scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
    // Explicit fallback only. No retry is made here: one HTTP 429 aborts this path.
    let mut offset = 0_u64;
    let mut lidar = BTreeMap::new();
    loop {
        let url = format!(
            "{TNM}?datasets=Lidar%20Point%20Cloud%20%28LPC%29&bbox={west:.9},{south:.9},{east:.9},{north:.9}&prodFormats=LAZ&max=100&offset={offset}"
        );
        let path = acquire(
            request,
            sources,
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
            sources,
            remote,
            InputKind::Lidar,
            None,
            url,
            transport::MAX_OBJECT,
        )?;
    }
    Ok(())
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
    let geoid = if preview {
        None
    } else {
        geoid::load(request, sources)?
    };
    let vertical = datum::evaluate(request, sources, geoid.as_ref(), dem.as_mut())?;
    download::write_json(&out.join("datum-audit.json"), &vertical)?;
    let (lidar_points_decoded, mut surfaces) = if preview {
        (0, None)
    } else {
        lidar::audit_sources(
            request,
            sources,
            out,
            dem.as_mut().ok_or("missing terrain for lidar ground control")?,
            geoid.as_ref(),
            vertical.passed,
        )?
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
                    geoid.as_ref(),
                )?;
                layout.tiles.push(tile);
            }
        }
    }
    layout.tiles.sort_by_key(|t| t.key);
    layout.validate().map_err(|e| e.to_string())?;
    let mut tileset = tiles::tileset(&layout, [40.96905613156273, -75.00242278301268], geoid.as_ref())?;
    let context = tileset
        .extras
        .as_mut()
        .and_then(|v| v.get_mut("flightcraft"))
        .and_then(|v| v.as_object_mut())
        .ok_or("missing tileset acceptance metadata")?;
    context.insert("verticalDatumVerified".into(), serde_json::json!(vertical.passed));
    if let Some(grid) = surfaces.as_mut() {
        let buildings = buildings::detect(grid)?;
        building_mesh::write(
            out,
            &mut tileset,
            &buildings,
            grid,
            &layout,
            dem.as_mut().ok_or("Building terrain")?,
            geoid.as_ref().ok_or("Building geoid")?,
            sources,
        )?;
        download::write_json(&out.join("buildings-audit.json"), &buildings)?;
        let mut trees = trees::detect(grid)?;
        // Roof footprints take precedence over unclassified canopy hypotheses.
        trees.retain(|t| {
            let Ok([x, y]) = scenery::mercator(t.lat, t.lon) else {
                return false;
            };
            let col = libm::floor((x - grid.bounds[0]) / grid.step) as usize;
            let row = libm::floor((y - grid.bounds[1]) / grid.step) as usize;
            let index = row * grid.width + col;
            !buildings.iter().any(|b| b.cells.contains_key(&index))
        });
        objects::write_trees(
            out,
            &mut tileset,
            &trees,
            geoid.as_ref().ok_or("Tree export requires a geoid")?,
        )?;
        download::write_json(&out.join("trees.json"), &trees)?;
        download::write_json(
            &out.join("trees-audit.json"),
            &serde_json::json!({
                "trees":trees.len(),"inferred_classification":trees.iter().filter(|t|t.inferred_classification).count(),
                "deciduous":trees.iter().filter(|t|t.species==trees::Species::Deciduous).count(),
                "conifer":trees.iter().filter(|t|t.species==trees::Species::Conifer).count(),
                "cell_m":1,"placement":"CHM local maxima and descending measured crowns","species":"crown radius/height heuristic; no NLCD claim",
                "gpu_instancing":"EXT_mesh_gpu_instancing","lod":"explicit quadtree, original 3D crowns nearby and alpha-mask crossed impostors farther away"
            }),
        )?;
    }
    tileset.validate().map_err(|e| e.to_string())?;
    download::write_json(&out.join("tileset.json"), &tileset)?;
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
        geoid_applied: geoid.is_some(),
        vertical_datum_verified: vertical.passed,
        runway_alignment_verified: false,
        lidar_built: tileset
            .extras
            .as_ref()
            .and_then(|v| v.pointer("/flightcraft/lidarBuilt"))
            .and_then(|v| v.as_bool())
            == Some(true),
        lidar_points_decoded,
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
    fn nyc_test_is_a_small_real_midtown_block_not_the_whole_city() {
        let preset = plan("new-york", "nyc-test").unwrap();
        let [lat, lon] = scenery::inverse_mercator(
            (preset.bounds[0] + preset.bounds[2]) / 2.,
            (preset.bounds[1] + preset.bounds[3]) / 2.,
        )
        .unwrap();
        assert!((lat - 40.7536).abs() < 1e-8);
        assert!((lon + 73.9832).abs() < 1e-8);
        assert_eq!(preset.max_level, 2);
        assert_eq!(preset.ground_sample_m, 0.6);
        assert!(super::plan("eastern-pa", "nyc-test").is_err());
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
        let set = tiles::tileset(&layout, [40.96905613156273, -75.00242278301268], None).unwrap();
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
        bad_control: bool,
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
            } else if url.starts_with("https://geodesy.noaa.gov/api/geoid/ght?") {
                assert!(!url.contains("model=18"));
                if self.bad_control {
                    br#"{"error":"No suitable Geoid model found for model 18"}"#.to_vec()
                } else {
                    br#"{"geoidModel":"GEOID18","units":"m","geoidHeight":-33.195,"error":0.035}"#.to_vec()
                }
            } else if url.starts_with("https://geodesy.noaa.gov/") {
                b"Original synthetic geoid acquisition fixture; decoding tested separately".to_vec()
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
    fn geoid_fetch_rejects_http_success_with_the_real_ngs_service_error() {
        let root = std::env::temp_dir().join(format!("fc-geoid-service-error-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let request = Request {
            operation: Operation::Fetch,
            fetch_layer: FetchLayer::Geoid,
            lidar_source: LidarSource::Ept,
            region: "eastern-pa".into(),
            area: "1n7-test".into(),
            cache: root.join("cache"),
            out: root.join("out"),
            sample: None,
            allow_http: false,
        };
        let mut remote = Recorded {
            calls: 0,
            jpeg: Vec::new(),
            bad_control: true,
        };
        assert!(
            fetch(&request, &mut remote)
                .unwrap_err()
                .contains("NGS control must identify GEOID18")
        );
        assert_eq!(remote.calls, 2);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fetch_paginates_classified_lidar_and_dem_and_resumes_completed_objects() {
        let root = std::env::temp_dir().join(format!("fc-scenery-recorded-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let request = Request {
            operation: Operation::Fetch,
            fetch_layer: FetchLayer::All,
            lidar_source: LidarSource::Rockyweb,
            region: "eastern-pa".into(),
            area: "1n7-test".into(),
            cache: root.join("cache"),
            out: root.join("out"),
            sample: None,
            allow_http: false,
        };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&vec![80; 512 * 512 * 3], 512, 512, image::ExtendedColorType::Rgb8)
            .unwrap();
        let mut remote = Recorded {
            calls: 0,
            jpeg,
            bad_control: false,
        };
        let first = fetch(&request, &mut remote).unwrap();
        assert_eq!(remote.calls, 73);
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
        let second = fetch(&request, &mut remote).unwrap();
        assert_eq!(remote.calls, 0);
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
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

#[cfg(test)]
pub(crate) fn nyc_test_flags() -> serde_json::Value {
    let receipts:Vec<_>=[
        flightcraft_render::scenery::NYC_EPT.to_owned(),
        "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/NY_NewYorkCity/ept-data/0-0-0-0.laz".into(),
    ].into_iter().map(|url|serde_json::json!({"url":url,"edition":"Original synthetic fixture, not a real source acquisition","downloaded_at":"2026-11-03T00:00:00Z","bytes":1,"sha256":"0".repeat(64)})).collect();
    serde_json::json!({"region":"new-york","area":"nyc-lidar","sourceKind":"usgs-3dep-lidar","roofPolicy":"lidar-only","localOnly":false,"redistributionAllowed":true,"previewOnly":false,"verticalDatumVerified":true,"runwayAlignmentVerified":true,"lidarBuilt":true,"nycLidar":{"selection":flightcraft_render::scenery::fly_nyc_area(),"reconstructionBuildings":1,"reconstructionLeavesAreBuildings":true,"usesExternalFootprints":false,"receipts":receipts}})
}

#[cfg(test)]
mod ept_fetch_tests {
    use super::*;
    struct RecordedEpt {
        calls: Vec<String>,
        fail_child: bool,
    }
    impl transport::Remote for RecordedEpt {
        fn open(&mut self, url: &str, offset: u64, _: Option<&str>) -> Result<transport::Response, String> {
            self.calls.push(url.into());
            assert_eq!(offset, 0);
            let bytes = if url == format!("{}/ept.json", ept::ROOT) {
                let schema: Vec<_> = ["X", "Y", "Z", "Classification", "ReturnNumber", "NumberOfReturns"]
                    .into_iter()
                    .map(|name| serde_json::json!({"name":name,"type":"signed","size":4,"scale":0.001,"offset":0.}))
                    .collect();
                serde_json::to_vec(&serde_json::json!({"bounds":[-8384951.,4915123.,-74944.,-8234569.,5065505.,75438.],"boundsConforming":[-8384951.,4915123.,80.,-8234569.,5065505.,200.],"span":128,"points":30097024116_u64,"dataType":"laszip","hierarchyType":"json","srs":{"authority":"EPSG","horizontal":"3857","wkt":"Original synthetic EPT fixture; not a real project declaration"},"schema":schema})).unwrap()
            } else if url.ends_with("/ept-hierarchy/0-0-0-0.json") {
                br#"{"0-0-0-0":100,"1-0-1-0":-1,"1-1-1-0":50,"1-0-0-0":40}"#.to_vec()
            } else if url.ends_with("/ept-hierarchy/1-0-1-0.json") {
                br#"{"1-0-1-0":50}"#.to_vec()
            } else if url.ends_with("/ept-data/0-0-0-0.laz") {
                b"Original parent data acquisition fixture; not a valid LAS file".to_vec()
            } else if url.ends_with("/ept-data/1-0-1-0.laz") {
                if self.fail_child {
                    return Err("recorded interrupted child request".into());
                }
                b"Original child data acquisition fixture; not a valid LAS file".to_vec()
            } else {
                return Err(format!("unexpected URL: {url}"));
            };
            Ok(transport::Response {
                status: 200,
                validator: Some("\"original-ept-fixture\"".into()),
                range: None,
                reader: Box::new(std::io::Cursor::new(bytes)),
            })
        }
    }
    fn request(root: &Path, source: LidarSource) -> Request {
        Request {
            operation: Operation::Fetch,
            fetch_layer: FetchLayer::Lidar,
            lidar_source: source,
            region: "eastern-pa".into(),
            area: "1n7-test".into(),
            cache: root.join("cache"),
            out: root.join("built"),
            sample: None,
            allow_http: false,
        }
    }
    #[test]
    fn ept_only_preserves_base_catalog_and_resumes_without_duplicate_records() {
        let root = std::env::temp_dir().join(format!("fc-ept-resume-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let request = request(&root, LidarSource::Ept);
        let mut original = plan(&request.region, &request.area).unwrap();
        original.inputs.push(Input {
            kind: InputKind::Dem,
            key: None,
            source: Source {
                url: "https://prd-tnm.s3.amazonaws.com/original-existing-dem.tif".into(),
                edition: Some("Original existing-catalog fixture".into()),
                downloaded_at: "2026-11-03T00:00:00Z".into(),
                bytes: 3,
                sha256: download::sha256(b"dem"),
            },
        });
        write_sources(&request.cache, &original).unwrap();
        let mut remote = RecordedEpt {
            calls: Vec::new(),
            fail_child: true,
        };
        assert!(fetch(&request, &mut remote).is_err());
        let interrupted: Sources = download::json(&request.cache.join("sources.json")).unwrap();
        assert!(interrupted.ept_selection.is_none());
        assert_eq!(
            interrupted
                .inputs
                .iter()
                .filter(|i| i.kind == InputKind::Dem)
                .count(),
            1
        );
        remote.calls.clear();
        remote.fail_child = false;
        let completed = fetch(&request, &mut remote).unwrap();
        assert_eq!(remote.calls, [format!("{}/ept-data/1-0-1-0.laz", ept::ROOT)]);
        assert_eq!(completed.inputs.len(), 6);
        assert_eq!(completed.ept_selection.as_ref().unwrap().node_count, 2);
        assert_eq!(
            completed
                .ept_selection
                .as_ref()
                .unwrap()
                .selected_points_before_clipping,
            150
        );
        assert_eq!(completed.ept_selection.as_ref().unwrap().depth, 11);
        assert_eq!(
            serde_json::to_value(completed.inputs.first()).unwrap(),
            serde_json::to_value(original.inputs.first()).unwrap()
        );
        remote.calls.clear();
        let second = fetch(&request, &mut remote).unwrap();
        assert!(remote.calls.is_empty());
        assert_eq!(
            serde_json::to_value(completed).unwrap(),
            serde_json::to_value(second).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    struct Rocky429 {
        point_calls: usize,
    }
    impl transport::Remote for Rocky429 {
        fn open(&mut self, url: &str, _: u64, _: Option<&str>) -> Result<transport::Response, String> {
            let (status, bytes) = if url.starts_with(TNM) {
                (200,br#"{"total":2,"items":[{"downloadURL":"https://rockyweb.usgs.gov/original-a.laz"},{"downloadURL":"https://rockyweb.usgs.gov/original-b.laz"}]}"#.to_vec())
            } else {
                self.point_calls += 1;
                (429, Vec::new())
            };
            Ok(transport::Response {
                status,
                validator: None,
                range: None,
                reader: Box::new(std::io::Cursor::new(bytes)),
            })
        }
    }
    #[test]
    fn explicit_rockyweb_fallback_stops_after_one_429() {
        let root = std::env::temp_dir().join(format!("fc-rocky429-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut remote = Rocky429 { point_calls: 0 };
        let result = fetch(&request(&root, LidarSource::Rockyweb), &mut remote);
        assert!(result.unwrap_err().contains("429"));
        assert_eq!(remote.point_calls, 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn another_fetch_cannot_mutate_a_locked_catalog() {
        let root = std::env::temp_dir().join(format!("fc-fetch-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let request = request(&root, LidarSource::Ept);
        std::fs::create_dir_all(&request.cache).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(request.cache.join(".fetch-lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let mut remote = RecordedEpt {
            calls: Vec::new(),
            fail_child: false,
        };
        assert!(fetch(&request, &mut remote).is_err());
        assert!(remote.calls.is_empty());
        drop(lock);
        std::fs::remove_dir_all(root).unwrap();
    }
}
