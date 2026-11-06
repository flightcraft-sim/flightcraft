//! Package a browser build for a static host, without Trunk.
use std::path::PathBuf;
use std::process::Command;

pub fn build() -> Result<(), String> {
    let root = super::root()?;
    let debug = std::env::args().nth(2).as_deref() == Some("--debug");
    let check = match (std::env::args().nth(2).as_deref(), std::env::args().nth(3)) {
        (None | Some("--debug"), None) => false,
        (Some("--check"), None) => true,
        _ => return Err("usage: cargo xtask web [--check|--debug]".into()),
    };
    if check {
        check_compile()?;
    }
    super::data::fetch(
        false,
        &root.join(".cache/data"),
        std::env::var("FLIGHTCRAFT_DATA_URL").ok().as_deref(),
    )?;
    if check {
        let stage = root.join(".cache/web-check");
        if stage.exists() {
            std::fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
        }
        for id in ["new-york", "eastern-pa"] {
            let source = root.join("assets/terrain").join(id);
            flightcraft_control::read_region(&source)?;
            validate_showcase_scenery(&source, id)?;
            copy_tree(&source, &stage.join("terrain").join(id))?;
            if !web_scenery(id) {
                // Desktop scenery (1N7) is installed from its own data release, never the site.
                let desktop_only = stage.join("terrain").join(id).join("scenery");
                if desktop_only.exists() {
                    std::fs::remove_dir_all(desktop_only).map_err(|e| e.to_string())?;
                }
            }
            for kind in ["sectional", "tac"] {
                let path = source.join("charts").join(kind).join("index.json");
                if path.exists() {
                    flightcraft_control::ChartIndex::from_json(&super::download::read(
                        &path,
                        4 * 1024 * 1024,
                    )?)?;
                }
            }
        }
        share_identical_chart_tiles(&stage.join("terrain"))?;
        let bytes = check_package_budget(&stage)?;
        eprintln!(
            "xtask: web compile and showcase packaging check passed ({bytes} data bytes; full packaging also checks the complete site)"
        );
        return Ok(());
    }
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).map_err(|e| e.to_string())?;
    let version = bindgen_version(&lock)?;
    let bindgen = std::env::var_os("WASM_BINDGEN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("wasm-bindgen"));
    let output = Command::new(&bindgen)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run wasm-bindgen: {e}; install wasm-bindgen-cli {version}"))?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim() != format!("wasm-bindgen {version}")
    {
        return Err(format!(
            "wasm-bindgen must match Cargo.lock; install with: cargo install wasm-bindgen-cli --version {version} --locked"
        ));
    }

    // Respect Cargo configuration and build queues that supply their own target directory.
    let metadata = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .current_dir(&root)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cannot read Cargo metadata: {e}"))?;
    if !metadata.status.success() {
        return Err(String::from_utf8_lossy(&metadata.stderr).into_owned());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).map_err(|e| e.to_string())?;
    let target = metadata
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .ok_or("Cargo metadata has no target_directory")?;
    let mut build = vec![
        "build",
        "--locked",
        "--target",
        "wasm32-unknown-unknown",
        "-p",
        "flightcraft-web",
    ];
    if !debug {
        build.push("--release");
    }
    super::cargo(&build)?;
    let profile = if debug { "debug" } else { "release" };

    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let status = Command::new(bindgen)
        .arg(PathBuf::from(target).join(format!("wasm32-unknown-unknown/{profile}/flightcraft-web.wasm")))
        .args(["--target", "web", "--out-name", "flightcraft_web", "--out-dir"])
        .arg(&dist)
        .status()
        .map_err(|e| format!("cannot run wasm-bindgen: {e}"))?;
    if !status.success() {
        return Err("wasm-bindgen failed".into());
    }
    let index = std::fs::read_to_string(root.join("apps/web/index.html"))
        .map_err(|e| e.to_string())?
        .replace(
            "<link data-trunk rel=\"rust\" data-bin=\"flightcraft-web\">",
            "<script type=\"module\" src=\"./bootstrap.js\"></script>",
        )
        .replace(
            "    <link data-trunk rel=\"copy-file\" href=\"takeoff.jsonl\">\n",
            "",
        )
        .replace(
            "    <link data-trunk rel=\"copy-dir\" href=\"../../assets/terrain\">\n",
            "",
        );
    let index = index.replace(
        "    <link data-trunk rel=\"copy-file\" href=\"screen-worker.js\">\n",
        "",
    );
    std::fs::copy(
        root.join("apps/web/screen-worker.js"),
        dist.join("screen-worker.js"),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(dist.join("index.html"), index).map_err(|e| e.to_string())?;
    std::fs::copy(root.join("apps/web/bootstrap.js"), dist.join("bootstrap.js"))
        .map_err(|e| e.to_string())?;
    std::fs::copy(root.join("apps/web/takeoff.jsonl"), dist.join("takeoff.jsonl"))
        .map_err(|e| e.to_string())?;
    for id in ["new-york", "eastern-pa"] {
        let region_source = root.join("assets/terrain").join(id);
        let region_target = dist.join("terrain").join(id);
        std::fs::create_dir_all(&region_target).map_err(|e| e.to_string())?;
        validate_showcase_scenery(&region_source, id)?;
        // Both imagery and city content use scenery/; include them in the complete-site budget.
        let scenery_target = region_target.join("scenery");
        if scenery_target.exists() {
            std::fs::remove_dir_all(&scenery_target).map_err(|e| e.to_string())?;
        }
        if web_scenery(id) {
            copy_tree(&region_source.join("scenery"), &scenery_target)?;
        }
        copy_tree(&region_source.join("charts"), &region_target.join("charts"))?;
        for entry in std::fs::read_dir(region_source).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file() {
                std::fs::copy(entry.path(), region_target.join(entry.file_name()))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    share_identical_chart_tiles(&dist.join("terrain"))?;
    let bytes = check_package_budget(&dist)?;
    eprintln!("xtask: static web build ready in dist/ ({bytes} bytes)");
    Ok(())
}

/// The web regions overlap, so their charts share many byte-identical tiles. The website keeps
/// each such tile once: the later region's index lists it under `shared` (read from the first
/// region's directory) and its own copy is removed. The desktop data is never changed.
fn share_identical_chart_tiles(terrain: &std::path::Path) -> Result<u64, String> {
    use flightcraft_control::{ChartIndex, SharedChartTiles};
    let (holder, sharer) = ("new-york", "eastern-pa");
    let mut saved = 0;
    for kind in ["sectional", "tac"] {
        let from = terrain.join(holder).join("charts").join(kind);
        let to = terrain.join(sharer).join("charts").join(kind);
        let (Ok(from_json), Ok(to_json)) = (
            std::fs::read(from.join("index.json")),
            std::fs::read(to.join("index.json")),
        ) else {
            continue;
        };
        let from_index = ChartIndex::from_json(&from_json)?;
        let mut to_index = ChartIndex::from_json(&to_json)?;
        if from_index.shared.is_some()
            || to_index.shared.is_some()
            || from_index.version != to_index.version
            || from_index.kind != to_index.kind
            || from_index.edition != to_index.edition
        {
            continue;
        }
        let mut shared = Vec::new();
        for tile in to_index.tiles.clone() {
            if !from_index.tiles.contains(&tile) {
                continue;
            }
            let path = to_index.tile_path(tile);
            let (Ok(a), Ok(b)) = (std::fs::read(from.join(&path)), std::fs::read(to.join(&path))) else {
                continue;
            };
            if a == b {
                std::fs::remove_file(to.join(&path)).map_err(|e| e.to_string())?;
                saved += b.len() as u64;
                shared.push(tile);
            }
        }
        if !shared.is_empty() {
            to_index.shared = Some(SharedChartTiles {
                region: holder.into(),
                tiles: shared,
            });
            let json = serde_json::to_vec_pretty(&to_index).map_err(|e| e.to_string())?;
            ChartIndex::from_json(&json)?;
            std::fs::write(to.join("index.json"), json).map_err(|e| e.to_string())?;
        }
    }
    if saved > 0 {
        eprintln!("xtask: {saved} bytes of chart tiles shared between {holder} and {sharer}");
    }
    Ok(saved)
}

/// The one scenery area the website carries: the Fly NYC Hudson corridor.
const FLY_NYC_AREA: &str = "hudson-corridor";

/// The website ships scenery only for the Fly NYC showcase region (docs/scenery-sources.md).
/// Other regions' scenery, such as the 1N7 tiles, is desktop-only data.
fn web_scenery(id: &str) -> bool {
    id == "new-york"
}

fn validate_showcase_scenery(region: &std::path::Path, id: &str) -> Result<(), String> {
    if !web_scenery(id) {
        return Ok(());
    }
    let scenery = region.join("scenery");
    // Until accepted Fly NYC scenery exists, the site ships on the region's existing terrain.
    // Once any scenery is staged, the showcase must be complete (buildings and budget below).
    let mut nyc = false;
    let files = super::data::files(&scenery)?;
    if files.is_empty() {
        return Ok(());
    }
    let mut roots = Vec::new();
    for path in &files {
        if path.file_name().is_some_and(|n| n == "tileset.json") {
            let bytes = super::download::read(path, flightcraft_world::tiles3d::MAX_TILESET_BYTES)?;
            flightcraft_world::tiles3d::Tileset::from_json(&bytes).map_err(|e| e.to_string())?;
            let set: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            super::data::validate_scenery_release(&set)?;
            roots.push(
                path.parent()
                    .ok_or("Scenery manifest has no directory")?
                    .to_path_buf(),
            );
            if *path == scenery.join(format!("{FLY_NYC_AREA}/tileset.json")) {
                flightcraft_render::scenery::nyc_lidar_source(
                    set.pointer("/extras/flightcraft")
                        .ok_or("Fly NYC provenance missing")?,
                )?;
                nyc = true;
            }
        }
    }
    if files
        .iter()
        .any(|path| !roots.iter().any(|root| path.starts_with(root)))
    {
        return Err("Website scenery contains files outside an accepted tileset directory".into());
    }
    if id == "new-york" {
        if !nyc {
            return Err(format!(
                "Fly NYC needs the accepted Hudson corridor at new-york/scenery/{FLY_NYC_AREA}; build it with `scenery build --area hudson-corridor`, package it, and `cargo xtask data fetch` (docs/scenery.md, \"Hudson corridor\")"
            ));
        }
        // The 250 MB building allocation covers the lidar building tiles; imagery, terrain, trees
        // and water share the rest of the 900 MB site budget checked below.
        let bytes = super::data::files(&scenery.join(FLY_NYC_AREA).join("buildings"))?
            .iter()
            .try_fold(0_u64, |total, p| {
                std::fs::metadata(p)
                    .map(|m| total + m.len())
                    .map_err(|e| e.to_string())
            })?;
        if bytes > flightcraft_render::scenery::NYC_BUILDING_BUDGET {
            return Err("Fly NYC buildings exceed their 250 MB share of the 1 GB website budget".into());
        }
    }
    Ok(())
}

/// Decimal MB, leaving room below the host's approximately 1 GB limit. Count the entire
/// installed site, including all LODs, textures, city tiles, charts, Wasm, and support files.
const SHOWCASE_MAX_BYTES: u64 = 900_000_000;
fn check_package_budget(root: &std::path::Path) -> Result<u64, String> {
    let bytes = super::data::files(root)?.iter().try_fold(0_u64, |sum, path| {
        let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        sum.checked_add(size)
            .ok_or_else(|| "showcase package size overflow".to_string())
    })?;
    if bytes > SHOWCASE_MAX_BYTES {
        return Err(format!(
            "Fly NYC showcase is {bytes} bytes, exceeding its 900 MB site budget"
        ));
    }
    Ok(bytes)
}

/// The same package set as CI, with warnings treated as build failures.
pub(crate) fn check_compile() -> Result<(), String> {
    eprintln!("xtask: checking all CI Web packages with RUSTFLAGS=-D warnings");
    let status = wasm_check_command()
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("strict Web compile check failed".into())
    }
}

fn wasm_check_command() -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.args([
        "check",
        "--target",
        "wasm32-unknown-unknown",
        "-p",
        "flightcraft-world",
        "-p",
        "flightcraft-aero",
        "-p",
        "flightcraft-sim",
        "-p",
        "flightcraft-control",
        "-p",
        "flightcraft-web",
        "-p",
        "flightcraft-mcp",
    ]);
    let mut flags = std::env::var("RUSTFLAGS").unwrap_or_default();
    flags.push_str(" -D warnings");
    command.env("RUSTFLAGS", flags.trim());
    // Cargo gives encoded flags priority over RUSTFLAGS when both are supplied.
    if let Ok(mut encoded) = std::env::var("CARGO_ENCODED_RUSTFLAGS") {
        if !encoded.is_empty() {
            encoded.push('\x1f');
        }
        encoded.push_str("-D\x1fwarnings");
        command.env("CARGO_ENCODED_RUSTFLAGS", encoded);
    }
    command
}

fn bindgen_version(lock: &str) -> Result<&str, String> {
    lock.split("[[package]]")
        .find(|package| {
            package
                .lines()
                .any(|line| line.trim() == "name = \"wasm-bindgen\"")
        })
        .and_then(|package| {
            package.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("version = \"")
                    .and_then(|s| s.strip_suffix('"'))
            })
        })
        .ok_or_else(|| "Cargo.lock has no wasm-bindgen version".into())
}

/// Chart pyramids are copied as encoded files; packaging never decodes a raster tile.
fn copy_tree(source: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target.join(entry.file_name())).map_err(|e| e.to_string())?;
        } else {
            return Err("chart package contains a non-regular file".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_check_includes_every_ci_package_and_denies_warnings() {
        let command = wasm_check_command();
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "check",
                "--target",
                "wasm32-unknown-unknown",
                "-p",
                "flightcraft-world",
                "-p",
                "flightcraft-aero",
                "-p",
                "flightcraft-sim",
                "-p",
                "flightcraft-control",
                "-p",
                "flightcraft-web",
                "-p",
                "flightcraft-mcp"
            ]
        );
        assert!(
            command.get_envs().any(|(name, value)| name == "RUSTFLAGS"
                && value.unwrap().to_str().unwrap().ends_with("-D warnings"))
        );
    }

    #[test]
    fn showcase_budget_counts_nested_city_tiles_mips_and_the_application() {
        let root = std::env::temp_dir().join(format!("fc-web-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("terrain/new-york/scenery/hudson-corridor/tiles/3")).unwrap();
        let tile =
            std::fs::File::create(root.join("terrain/new-york/scenery/hudson-corridor/tiles/3/city.glb"))
                .unwrap();
        // Sparse files exercise real packaging metadata without allocating a 900 MB fixture.
        tile.set_len(890_000_000).unwrap();
        let wasm = std::fs::File::create(root.join("flightcraft_web_bg.wasm")).unwrap();
        wasm.set_len(10_000_000).unwrap();
        assert_eq!(check_package_budget(&root).unwrap(), 900_000_000);
        std::fs::write(root.join("index.html"), b"x").unwrap();
        assert!(check_package_budget(&root).unwrap_err().contains("900 MB"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn version_comes_from_the_bindgen_package() {
        let lock = "version = 4\n[[package]]\nname = \"other\"\nversion = \"1.0\"\n\n[[package]]\nname = \"wasm-bindgen\"\nversion = \"0.2.130\"\n";
        assert_eq!(bindgen_version(lock).unwrap(), "0.2.130");
        assert!(bindgen_version("[[package]]\nname = \"wasm-bindgen-cli\"\nversion = \"0.2.130\"").is_err());
        assert!(bindgen_version("[[package]]\nname = \"wasm-bindgen\"").is_err());
    }
}

#[cfg(test)]
mod scenery_package_tests {
    use super::*;
    fn area_path(root: &std::path::Path) -> std::path::PathBuf {
        root.join(format!("scenery/{FLY_NYC_AREA}/tileset.json"))
    }
    #[test]
    fn desktop_only_scenery_never_reaches_the_website() {
        assert!(web_scenery("new-york") && !web_scenery("eastern-pa"));
        let root = std::env::temp_dir().join(format!("fc-web-desktop-scenery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // An installed 1N7 data release (even one the site validator would reject) is ignored:
        // `web` neither validates nor copies another region's scenery.
        std::fs::create_dir_all(root.join("scenery/1n7")).unwrap();
        std::fs::write(root.join("scenery/1n7/tileset.json"), b"not a tileset").unwrap();
        assert!(validate_showcase_scenery(&root, "eastern-pa").is_ok());
        assert!(validate_showcase_scenery(&root, "new-york").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn scenery_packaging_requires_lidar_counts_bytes_and_rejects_portal_and_tum() {
        let root = std::env::temp_dir().join(format!("fc-web-scenery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // No staged scenery: the site ships on the region's existing terrain.
        assert!(validate_showcase_scenery(&root, "new-york").is_ok());
        std::fs::create_dir_all(root.join("scenery")).unwrap();
        assert!(validate_showcase_scenery(&root, "new-york").is_ok());
        let area = root.join(format!("scenery/{FLY_NYC_AREA}"));
        std::fs::create_dir_all(area.join("tiles")).unwrap();
        std::fs::create_dir_all(area.join("buildings")).unwrap();
        let mut set = serde_json::json!({"asset":{"version":"1.1","copyright":"Original fixture"},"geometricError":0,"root":{"boundingVolume":{"region":[-1.3,0.7,-1.2,0.8,0,100]},"geometricError":0,"content":{"uri":"tiles/test.glb"}},"extras":{"flightcraft":crate::scenery::nyc_test_flags()}});
        super::super::download::write_json(&area.join("tileset.json"), &set).unwrap();
        std::fs::write(
            area.join("buildings/test.glb"),
            b"package accounting fixture, not render proof",
        )
        .unwrap();
        assert!(validate_showcase_scenery(&root, "new-york").is_ok());
        let target = root.join("staged");
        copy_tree(&root.join("scenery"), &target).unwrap();
        assert!(target.join(FLY_NYC_AREA).join("buildings/test.glb").exists());
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(area.join("buildings/test.glb"))
            .unwrap();
        file.set_len(250_000_001).unwrap();
        assert!(
            validate_showcase_scenery(&root, "new-york")
                .unwrap_err()
                .contains("250 MB")
        );
        file.set_len(1).unwrap();
        drop(file);
        for area in ["nyc-footprints", "nyc-lod2"] {
            set["extras"]["flightcraft"]["area"] = serde_json::json!(area);
            super::super::download::write_json(&area_path(&root), &set).unwrap();
            assert!(
                validate_showcase_scenery(&root, "new-york")
                    .unwrap_err()
                    .contains("Local-only")
            );
        }
        set["extras"]["flightcraft"]["localOnly"] = serde_json::json!(true);
        super::super::download::write_json(&area.join("tileset.json"), &set).unwrap();
        assert!(
            validate_showcase_scenery(&root, "new-york")
                .unwrap_err()
                .contains("Local")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod chart_sharing_tests {
    use super::share_identical_chart_tiles;
    use flightcraft_control::{BaseMap, ChartIndex, ChartTile};

    fn region(root: &std::path::Path, id: &str, tiles: &[(ChartTile, &[u8])]) {
        let dir = root.join(id).join("charts/sectional");
        for (tile, bytes) in tiles {
            let path = dir.join(format!("{}/{}/{}.webp", tile.z, tile.x, tile.y));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let index = ChartIndex {
            coverage_margin_nm: 40.0,
            version: 2,
            kind: BaseMap::Sectional,
            edition: "2026-10-29".into(),
            expires: "2026-12-23".into(),
            attribution: "FAA VFR raster charts".into(),
            min_zoom: 8,
            max_zoom: 8,
            tile_size: 256,
            tiles: tiles.iter().map(|(t, _)| *t).collect(),
            bytes: 0,
            shared: None,
        };
        std::fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
    }

    #[test]
    fn identical_tiles_ship_once_and_every_tile_still_resolves() {
        let root = std::env::temp_dir().join(format!("fc-web-chart-share-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let t = |x| ChartTile { z: 8, x, y: 95 };
        region(
            &root,
            "new-york",
            &[(t(73), b"same"), (t(74), b"ny edge"), (t(75), b"ny only")],
        );
        region(
            &root,
            "eastern-pa",
            &[(t(72), b"pa only"), (t(73), b"same"), (t(74), b"pa edge")],
        );
        assert_eq!(share_identical_chart_tiles(&root).unwrap(), 4);
        let dir = root.join("eastern-pa/charts/sectional");
        let index = ChartIndex::from_json(&std::fs::read(dir.join("index.json")).unwrap()).unwrap();
        assert_eq!(index.tiles.len(), 3);
        assert_eq!(index.shared.as_ref().unwrap().tiles, vec![t(73)]);
        assert!(!dir.join("8/73/95.webp").exists());
        // Each tile, own or shared, reads the region's original image through its index path.
        for (tile, bytes) in [(t(72), &b"pa only"[..]), (t(73), b"same"), (t(74), b"pa edge")] {
            let path = dir.join(index.tile_path(tile));
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{}", path.display());
        }
        // A second pass changes nothing.
        assert_eq!(share_identical_chart_tiles(&root).unwrap(), 0);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
