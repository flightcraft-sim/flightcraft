//! Consume fc-scenery's reconstructed buildings. No EPT selection, LAZ decoding, footprint
//! generation, or roof-plane fitting lives here. Retained output is OGC 3D Tiles/GLB only.
use super::{Operation, Request, city, city_mesh, download};
use flightcraft_render::scenery::{self, Location, NycLidarSource, Provider};
use flightcraft_world::tiles3d::{self, Tile, Tileset};
use glam::{DMat4, DVec3};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LidarReport {
    pub region: String,
    pub area: String,
    pub manifest: std::path::PathBuf,
    pub bounds: [f64; 4],
    pub buildings: usize,
    pub excluded_buildings: usize,
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
fn flags(set: &Tileset) -> Result<&Value, String> {
    set.extras
        .as_ref()
        .and_then(|v| v.get("flightcraft"))
        .ok_or_else(|| "Reconstruction provider has no provenance".into())
}
fn source(set: &Tileset) -> Result<NycLidarSource, String> {
    scenery::nyc_lidar_source(flags(set)?)
}
fn contained(root: &Path, uri: &str) -> Result<std::path::PathBuf, String> {
    if !tiles3d::local_uri(uri) {
        return Err("Unsafe reconstruction GLB path".into());
    }
    let path = root.join(uri).canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(root) {
        return Err("Reconstruction GLB escapes its provider directory".into());
    }
    Ok(path)
}
fn build(request: &Request, out: &Path) -> Result<LidarReport, String> {
    let descriptor = request.cache.join("reconstruction-provider.json");
    let provider:Provider=serde_json::from_slice(&download::read(&descriptor,64*1024).map_err(|_|"Fly NYC needs cache/reconstruction-provider.json pointing to fc-scenery's reconstructed 3DEP buildings; no Socrata or TUM fallback is allowed")?).map_err(|_|"Invalid reconstruction provider descriptor")?;
    if !provider.enabled || provider.token_config_key.is_some() {
        return Err("Fly NYC reconstruction needs an enabled local provider without credentials".into());
    }
    let Location::LocalPath(path) = &provider.location else {
        return Err("Fly NYC build consumes a completed local reconstruction provider; no online reconstruction is performed".into());
    };
    // Relative locations are relative to the descriptor, never to the process working directory.
    let path = if path.is_absolute() {
        path.clone()
    } else {
        request.cache.join(path)
    };
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let root = path
        .parent()
        .ok_or("Reconstruction provider has no directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let bytes = download::read(&path, tiles3d::MAX_TILESET_BYTES)?;
    let set = Tileset::from_json(&bytes).map_err(|e| e.to_string())?;
    scenery::validate_static_scenery(&set)?;
    let provenance = source(&set)?;
    if set.root.transform.is_none() {
        return Err("Reconstructed buildings require an explicit ECEF root frame; EPSG:3857 point conversion belongs to fc-scenery".into());
    }
    let input_flags = flags(&set)?;
    let vertical = input_flags.get("verticalDatumVerified").and_then(Value::as_bool) == Some(true);
    let runway = input_flags
        .get("runwayAlignmentVerified")
        .and_then(Value::as_bool)
        == Some(true);
    let accepted =
        vertical && runway && input_flags.get("previewOnly").and_then(Value::as_bool) == Some(false);
    let (frame, inverse) = city_mesh::frame()?;
    let to_earth = DMat4::from_cols_array(&frame) * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
    let mut writer = city_mesh::Writer::new(out, scenery::NYC_BUILDING_BUDGET)?;
    let mut pending: Vec<(&Tile, DMat4)> = vec![(&set.root, DMat4::IDENTITY)];
    let mut seen = BTreeSet::new();
    let (mut buildings, mut excluded, mut leaves) = (0, 0, 0);
    while let Some((tile, parent)) = pending.pop() {
        let transform = parent
            * tile
                .transform
                .map(|t| DMat4::from_cols_array(&t))
                .unwrap_or(DMat4::IDENTITY);
        if !tile.children.is_empty() {
            if tile.content.is_some() {
                return Err("Reconstruction handoff requires content only at leaves, one measured building per leaf; this lane generates LODs".into());
            }
            pending.extend(tile.children.iter().rev().map(|child| (child, transform)));
            continue;
        }
        let content = tile
            .content
            .as_ref()
            .ok_or("Reconstruction leaf has no building GLB")?;
        if !seen.insert(content.uri.clone()) {
            return Err("Reconstruction repeats a building GLB".into());
        }
        leaves += 1;
        let model = city::Model {
            url: content.uri.clone(),
            gltf_to_ecef: (transform * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2)).to_cols_array(),
        };
        let mesh = city::triangles(
            &download::read(&contained(&root, &content.uri)?, tiles3d::MAX_TILE_CONTENT_BYTES)?,
            &model,
            inverse,
        )?;
        let (mut low, mut high) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for p in mesh.iter().flatten() {
            low = low.min(*p);
            high = high.max(*p);
        }
        let [lat, lon, _] = tiles3d::geodetic(to_earth.transform_point3((low + high) * 0.5).to_array())
            .map_err(|e| e.to_string())?;
        let [west, south, east, north] = scenery::NYC_BOUNDS;
        // Keep complete buildings whose centers are inside the showcase; never cut roofs at a boundary.
        if !(south..=north).contains(&lat) || !(west..=east).contains(&lon) {
            excluded += 1;
            continue;
        }
        writer.push(&mesh)?;
        buildings += 1;
    }
    if leaves as u64 != provenance.reconstruction_buildings {
        return Err("Reconstructed building count differs from the lidar provenance".into());
    }
    let mut extras = json!({"flightcraft":{"region":"new-york","area":"nyc-lidar","bounds":scenery::NYC_BOUNDS,"sourceKind":"usgs-3dep-lidar","roofPolicy":"lidar-only","localOnly":false,"redistributionAllowed":true,"previewOnly":!accepted,"verticalDatumVerified":vertical,"runwayAlignmentVerified":runway,"lidarBuilt":true,"buildingCount":buildings,"nycLidar":provenance}});
    // Geometry remains in ellipsoidal ECEF. Preserve the input's regional MSL conversion
    // through repackaging so native and browser decoders recover the same runway datum.
    if let Some(grid) = set.geoid_grid().map_err(|e| e.to_string())? {
        extras["flightcraft"]["geoid"] = serde_json::to_value(grid).map_err(|e| e.to_string())?;
    }
    let (_, _, tiles) = writer.finish(scenery::NYC_ATTRIBUTION, extras)?;
    // Retain the independently reconstructable input provenance, not a user's local filesystem path.
    download::write_json(
        &out.join("reconstruction-source.json"),
        &json!({"tilesetSha256":download::sha256(&bytes),"nycLidar":provenance}),
    )?;
    download::write_json(&out.join("area.json"), &scenery::fly_nyc_area())?;
    let bytes = crate::data::files(out)?.iter().try_fold(0_u64, |n, p| {
        std::fs::metadata(p)
            .map(|m| n + m.len())
            .map_err(|e| e.to_string())
    })?;
    let mut report = LidarReport {
        region: "new-york".into(),
        area: "nyc-lidar".into(),
        manifest: request.out.join("tileset.json"),
        bounds: scenery::NYC_BOUNDS,
        buildings,
        excluded_buildings: excluded,
        tiles,
        package_bytes: bytes,
        building_budget_bytes: scenery::NYC_BUILDING_BUDGET,
        total_web_budget_bytes: scenery::WEB_BUDGET,
        local_only: false,
        redistribution_allowed: true,
        preview_only: !accepted,
        vertical_datum_verified: vertical,
        runway_alignment_verified: runway,
        release_ready: accepted,
    };
    for _ in 0..4 {
        let size = bytes
            + serde_json::to_vec_pretty(&report)
                .map_err(|e| e.to_string())?
                .len() as u64
            + 1;
        if size == report.package_bytes {
            break;
        }
        report.package_bytes = size;
    }
    if report.package_bytes > scenery::NYC_BUILDING_BUDGET {
        return Err("Fly NYC buildings, LODs, and provenance exceed the 250 MB allocation".into());
    }
    download::write_json(&out.join("build-report.json"), &report)?;
    Ok(report)
}
pub(super) fn run(request: &Request) -> Result<(), String> {
    if matches!(request.operation, Operation::Fetch) {
        return Err("NYC EPT acquisition and building reconstruction belong to fc-scenery; use assets/scenery/fly-nyc.json as the selection and supply reconstruction-provider.json for the offline build".into());
    }
    if request.sample.is_some() {
        return Err(
            "Fly NYC cannot substitute sample imagery or footprint extrusions for lidar buildings".into(),
        );
    }
    if request.out.exists() {
        return Err("Fly NYC output must be a new directory".into());
    }
    let stage = request.out.with_extension("building");
    if let Some(parent) = stage.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir(&stage).map_err(|e| e.to_string())?;
    match build(request, &stage) {
        Ok(report) => {
            std::fs::rename(&stage, &request.out).map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(stage);
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(name: &str) -> Request {
        let root = std::env::temp_dir().join(format!("fc-nyc-lidar-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Request {
            operation: Operation::Build,
            fetch_layer: super::super::FetchLayer::All,
            lidar_source: super::super::LidarSource::Ept,
            region: "new-york".into(),
            area: "nyc-lidar".into(),
            cache: root.join("cache"),
            out: root.join("built"),
            sample: None,
            allow_http: false,
        }
    }
    fn fixture(request: &Request) -> Tileset {
        let root = request.cache.parent().unwrap().join("reconstruction");
        std::fs::create_dir_all(&root).unwrap();
        let (frame, inverse) = city_mesh::frame().unwrap();
        let mut children = Vec::new();
        for (name, lat) in [("hudson", 40.75), ("outside", 40.86)] {
            let p = city_mesh::local(inverse, lat, -74., 10.).unwrap();
            let mesh = vec![
                [p, p + DVec3::X * 20., p + DVec3::new(20., 10., 0.)],
                [p, p + DVec3::new(20., 10., 0.), p + DVec3::Y * 10.],
                [
                    p + DVec3::Y * 10.,
                    p + DVec3::new(20., 10., 0.),
                    p + DVec3::new(10., 16., -10.),
                ],
                [
                    p + DVec3::new(0., 10., -20.),
                    p + DVec3::new(10., 16., -10.),
                    p + DVec3::new(20., 10., -20.),
                ],
            ];
            let uri = format!("{name}.glb");
            std::fs::write(root.join(&uri), city::encode(&mesh, 0.).unwrap()).unwrap();
            children.push(Tile {
                bounding_volume: city::cell_volume(&mesh, frame).unwrap(),
                geometric_error: 0.,
                refine: None,
                transform: None,
                content: Some(tiles3d::Content { uri }),
                children: Vec::new(),
            });
        }
        let mut tile = city::parent(children);
        tile.transform = Some(frame);
        let mut flags = super::super::nyc_test_flags();
        flags["nycLidar"]["reconstructionBuildings"] = json!(2);
        let set = Tileset {
            asset: tiles3d::Asset {
                version: "1.1".into(),
                copyright: Some("Original synthetic FlightCraft fixture; not real lidar".into()),
            },
            geometric_error: 128.,
            root: tile,
            extensions_required: Vec::new(),
            extras: Some(json!({"flightcraft":flags})),
        };
        download::write_json(&root.join("tileset.json"), &set).unwrap();
        download::write_json(
            &request.cache.join("reconstruction-provider.json"),
            &Provider {
                location: Location::LocalPath("../reconstruction/tileset.json".into()),
                enabled: true,
                token_config_key: None,
            },
        )
        .unwrap();
        set
    }
    #[test]
    fn scenery_lidar_consumer_selects_showcase_preserves_roofs_and_emits_budgeted_lods() {
        let request = request("consume");
        let mut input = fixture(&request);
        let grid = flightcraft_world::geoid::GeoidGrid {
            model: flightcraft_world::geoid::GeoidModel::NgsGeoid18,
            source_sha256: "a".repeat(64),
            south_deg: 40.,
            west_deg: -75.,
            step_deg: [1. / 60.; 2],
            rows: 120,
            cols: 120,
            undulation_m: vec![-31.; 120 * 120],
        };
        input.extras.as_mut().unwrap()["flightcraft"]["geoid"] = serde_json::to_value(&grid).unwrap();
        download::write_json(
            &request
                .cache
                .parent()
                .unwrap()
                .join("reconstruction/tileset.json"),
            &input,
        )
        .unwrap();
        run(&request).unwrap();
        let report: LidarReport = download::json(&request.out.join("build-report.json")).unwrap();
        assert_eq!(report.buildings, 1);
        assert_eq!(report.excluded_buildings, 1);
        assert!(report.release_ready);
        assert_eq!(report.bounds, scenery::NYC_BOUNDS);
        assert!(report.package_bytes < scenery::NYC_BUILDING_BUDGET);
        let bytes: u64 = crate::data::files(&request.out)
            .unwrap()
            .iter()
            .map(|p| std::fs::metadata(p).unwrap().len())
            .sum();
        assert_eq!(bytes, report.package_bytes);
        assert!(!request.out.join("scratch").exists());
        let encoded = download::read(&request.out.join("tileset.json"), tiles3d::MAX_TILESET_BYTES).unwrap();
        let set = Tileset::from_json(&encoded).unwrap();
        assert_eq!(
            set.extras.as_ref().unwrap().pointer("/flightcraft/geoid"),
            input.extras.as_ref().unwrap().pointer("/flightcraft/geoid")
        );
        assert_eq!(
            set.geoid_grid()
                .unwrap()
                .unwrap()
                .separation_m(40.75, -74.)
                .unwrap(),
            -31.
        );
        scenery::validate_static_scenery(&set).unwrap();
        crate::data::validate_scenery_release(&serde_json::from_slice(&encoded).unwrap()).unwrap();
        let mut pending = vec![&set.root];
        let mut errors = Vec::new();
        let (_, inverse) = city_mesh::frame().unwrap();
        let mut highest = f64::NEG_INFINITY;
        while let Some(tile) = pending.pop() {
            errors.push(tile.geometric_error);
            let content = tile.content.as_ref().unwrap();
            let bytes =
                download::read(&request.out.join(&content.uri), tiles3d::MAX_TILE_CONTENT_BYTES).unwrap();
            scenery::decode_glb(
                &bytes,
                set.root.transform.unwrap(),
                [40.96905613156273, -75.00242278301268],
            )
            .unwrap();
            if tile.geometric_error == 0. {
                let model = city::Model {
                    url: content.uri.clone(),
                    gltf_to_ecef: (DMat4::from_cols_array(&set.root.transform.unwrap())
                        * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2))
                    .to_cols_array(),
                };
                for p in city::triangles(&bytes, &model, inverse).unwrap().iter().flatten() {
                    highest = highest.max(p.y);
                }
            }
            pending.extend(&tile.children);
        }
        assert!(errors.contains(&0.) && errors.contains(&4.) && errors.contains(&1024.));
        let p = city_mesh::local(inverse, 40.75, -74., 10.).unwrap();
        assert!((highest - p.y - 16.).abs() < 0.02);
        std::fs::remove_dir_all(request.cache.parent().unwrap()).unwrap();
    }
    #[test]
    fn scenery_lidar_refuses_external_footprints_portal_receipts_and_missing_reconstruction() {
        let request = request("reject");
        assert!(run(&request).unwrap_err().contains("reconstruction-provider"));
        assert!(!request.out.exists());
        assert!(!request.out.with_extension("building").exists());
        let mut set = fixture(&request);
        let path = request
            .cache
            .parent()
            .unwrap()
            .join("reconstruction/tileset.json");
        set.extras.as_mut().unwrap()["flightcraft"]["nycLidar"]["usesExternalFootprints"] = json!(true);
        download::write_json(&path, &set).unwrap();
        assert!(run(&request).is_err());
        assert!(!request.out.exists());
        set.extras.as_mut().unwrap()["flightcraft"]["nycLidar"]["usesExternalFootprints"] = json!(false);
        set.extras.as_mut().unwrap()["flightcraft"]["nycLidar"]["receipts"][1]["url"] =
            json!("https://data.cityofnewyork.us/resource/nqwf-w8eh.json");
        download::write_json(&path, &set).unwrap();
        assert!(run(&request).unwrap_err().contains("outside"));
        assert!(!request.out.exists());
        std::fs::remove_dir_all(request.cache.parent().unwrap()).unwrap();
    }
}
