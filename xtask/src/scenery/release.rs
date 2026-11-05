//! Packaging of an accepted scenery build as a desktop data release.
//!
//! `cargo xtask scenery package` re-verifies the build's acceptance evidence, installs it under
//! `assets/terrain/<region>/scenery/<name>/` (git-ignored), writes the attribution, and runs the
//! existing `data publish --only scenery` machinery: per-file and per-archive SHA-256, a manifest
//! per archive, and `data.lock` entries that `cargo xtask data fetch` installs. Nothing is
//! uploaded; the commands for the maintainer are written to `PUBLISH.sh`.
use crate::{data, download};
use std::path::{Path, PathBuf};

pub(super) fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let (mut region, mut built, mut name, mut tag, mut out) = (None, None, None, None, None);
    while let Some(key) = args.next() {
        let value = args.next().ok_or("missing package option value")?;
        match key.as_str() {
            "--region" => region = Some(value),
            "--built" => built = Some(PathBuf::from(value)),
            "--name" => name = Some(value),
            "--tag" => tag = Some(value),
            "--out" => out = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown package option {key}")),
        }
    }
    let usage = "usage: cargo xtask scenery package --region <id> --built <accepted-build-dir> --name <area> --tag <release-tag> --out <new-directory>";
    let lines = package(
        &crate::root()?,
        &built.ok_or(usage)?,
        &region.ok_or(usage)?,
        &name.ok_or(usage)?,
        &tag.ok_or(usage)?,
        &out.ok_or(usage)?,
    )?;
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

fn passed(built: &Path, file: &str) -> Result<(), String> {
    let value: serde_json::Value = download::json(&built.join(file))?;
    if value.get("passed").and_then(|v| v.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err(format!("{file} does not record a pass; scenery cannot ship"))
    }
}

pub(crate) fn attribution(built: &Path, region: &str, name: &str) -> Result<String, String> {
    let sources: super::Sources = download::json(&built.join("sources.json"))?;
    let count = |kind: super::InputKind| sources.inputs.iter().filter(|i| i.kind == kind).count();
    let registration = if name == super::corridor::AREA {
        "lidar building footprints compared with the NAIP orthophoto (no runway lies in the\n           Fly NYC envelope) and Teterboro's FAA field elevation compared with the DEM; see\n           alignment-audit.json and datum-audit.json."
    } else {
        "FAA runway data (NASR) compared with the painted runway markings measured in the NAIP\n           imagery; see alignment-audit.json."
    };
    let mut text = format!(
        "FlightCraft scenery: {name} ({region})\n\
         \n\
         Built from public-domain US Government data. Every input is listed with its URL,\n\
         retrieval time, size and SHA-256 in sources.json (and lidar-sources.jsonl for each\n\
         lidar node). Nothing in this package is licensed or proprietary imagery or models.\n\
         \n\
         - Imagery: USDA National Agriculture Imagery Program (NAIP), distributed by USGS The\n\
           National Map ImageServer ({} tiles). Public domain.\n\
         - Elevation: USGS 3D Elevation Program 1 m DEM, NAVD88 ({} files). Public domain.\n\
         - Lidar: USGS 3DEP classified point cloud (Entwine/EPT, {} nodes) used for canopy\n\
           and building reconstruction. Public domain.\n\
         - Geoid: NOAA National Geodetic Survey GEOID18 CONUS model and control point.\n\
           Public domain.\n\
         - Water: USGS National Hydrography Dataset Best Resolution, HU-8 shapefiles ({} files).\n\
           Public domain.\n\
         - Registration checks: {}\n\
         \n\
         Trees, buildings and water surfaces are original reconstructions measured from the data\n\
         above (see lidar-acceptance.json and water-audit.json). Credit: USDA FSA NAIP, USGS,\n\
         NOAA NGS. For simulation only; not for navigation.\n",
        count(super::InputKind::Imagery),
        count(super::InputKind::Dem),
        count(super::InputKind::EptLidar),
        count(super::InputKind::Nhd),
        registration,
    );
    if count(super::InputKind::Nhd) == 0 {
        text = text.replace("- Water: USGS National Hydrography Dataset Best Resolution, HU-8 shapefiles (0 files). Public domain.\n", "");
    }
    Ok(text)
}

/// Installs `built` and prepares the release. Returns the maintainer's publication commands.
pub(crate) fn package(
    root: &Path,
    built: &Path,
    region: &str,
    name: &str,
    tag: &str,
    out: &Path,
) -> Result<Vec<String>, String> {
    if !flightcraft_world::region_id(region) || !flightcraft_world::region_id(name) {
        return Err("region and scenery names must be lowercase region identifiers".into());
    }
    let report: serde_json::Value = download::json(&built.join("build-report.json"))?;
    if report.get("release_ready").and_then(|v| v.as_bool()) != Some(true)
        || report.get("preview_only").and_then(|v| v.as_bool()) != Some(false)
        || report.get("region").and_then(|v| v.as_str()) != Some(region)
    {
        return Err("build-report.json is not a release-ready build of this region".into());
    }
    // Re-read each gate's own evidence; a flag alone is not trusted.
    for file in [
        "datum-audit.json",
        "alignment-audit.json",
        "lidar-acceptance.json",
    ] {
        passed(built, file)?;
    }
    let files = data::files(built)?;
    let mut tilesets = 0;
    for path in files
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "tileset.json"))
    {
        let bytes = download::read(path, flightcraft_world::tiles3d::MAX_TILESET_BYTES)?;
        flightcraft_world::tiles3d::Tileset::from_json(&bytes).map_err(|e| e.to_string())?;
        let set: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        data::validate_scenery_release(&set)?;
        tilesets += 1;
    }
    if tilesets == 0 || !built.join("tileset.json").exists() {
        return Err("build has no tileset.json".into());
    }
    let destination = root
        .join("assets/terrain")
        .join(region)
        .join("scenery")
        .join(name);
    if destination.exists() {
        return Err(format!(
            "{} already exists; remove it deliberately before packaging a new build",
            destination.display()
        ));
    }
    if out.exists() {
        return Err("package output must be a new directory".into());
    }
    let staging = destination.with_extension("installing");
    let _ = std::fs::remove_dir_all(&staging);
    for path in &files {
        let relative = path.strip_prefix(built).map_err(|e| e.to_string())?;
        let target = staging.join(relative);
        std::fs::create_dir_all(target.parent().ok_or("scenery file parent")?).map_err(|e| e.to_string())?;
        std::fs::copy(path, &target).map_err(|e| e.to_string())?;
    }
    std::fs::write(staging.join("ATTRIBUTION.txt"), attribution(built, region, name)?)
        .map_err(|e| e.to_string())?;
    std::fs::rename(&staging, &destination).map_err(|e| e.to_string())?;
    data::publish_at(root, tag, out, Some("scenery"))?;
    let commands = data::release_commands(tag, out, Some("scenery"));
    let mut script = String::from(
        "#!/bin/sh\n# Publishes the scenery data release written by `cargo xtask scenery package`.\n# Review the archives and manifests first; this uploads publicly.\nset -eu\n",
    );
    for line in &commands {
        script.push_str(line);
        script.push('\n');
    }
    script.push_str(&format!(
        "# Then land the lock that points `cargo xtask data fetch` at this release:\n# git add data.lock && git commit -m 'Pin the {name} scenery data release {tag}'\n"
    ));
    std::fs::write(out.join("PUBLISH.sh"), script).map_err(|e| e.to_string())?;
    let mut lines = vec![
        format!("installed {}", destination.display()),
        format!("release files in {}", out.display()),
        "publish with (also saved as PUBLISH.sh):".into(),
    ];
    lines.extend(commands);
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_build(dir: &Path, passed_flag: bool) {
        let flags = serde_json::json!({
            "region":"eastern-pa","area":"1n7-test","localOnly":false,"redistributionAllowed":true,
            "previewOnly":false,"verticalDatumVerified":true,"runwayAlignmentVerified":true,"lidarBuilt":true
        });
        let set = serde_json::json!({
            "asset":{"version":"1.1","copyright":"Original fixture"},"geometricError":0,
            "root":{"boundingVolume":{"region":[-1.3,0.7,-1.2,0.8,0,100]},"geometricError":0,"content":{"uri":"tiles/a.glb"}},
            "extras":{"flightcraft":flags}
        });
        std::fs::create_dir_all(dir.join("tiles")).unwrap();
        std::fs::write(dir.join("tileset.json"), serde_json::to_vec(&set).unwrap()).unwrap();
        std::fs::write(dir.join("tiles/a.glb"), b"original fixture content").unwrap();
        std::fs::write(
            dir.join("build-report.json"),
            serde_json::json!({"region":"eastern-pa","area":"1n7-test","release_ready":true,"preview_only":false}).to_string(),
        )
        .unwrap();
        for file in [
            "datum-audit.json",
            "alignment-audit.json",
            "lidar-acceptance.json",
        ] {
            std::fs::write(
                dir.join(file),
                serde_json::json!({"passed":passed_flag}).to_string(),
            )
            .unwrap();
        }
        let sources = serde_json::json!({"version":1,"region":"eastern-pa","area":"1n7-test","bounds":[0.0,0.0,1.0,1.0],"tile_pixels":512,"max_level":0,"ground_sample_m":0.3,"inputs":[]});
        std::fs::write(dir.join("sources.json"), sources.to_string()).unwrap();
    }
    fn scratch_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("fc-scenery-release-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("assets/terrain/eastern-pa")).unwrap();
        std::fs::write(root.join("assets/terrain/eastern-pa/region.json"), b"{}").unwrap();
        std::fs::write(
            root.join("data.lock"),
            serde_json::json!({"version":1,"base_url":"https://github.com/flightcraft-sim/flightcraft/releases/download","assets":[]}).to_string(),
        )
        .unwrap();
        root
    }

    #[test]
    fn packaging_installs_publishes_and_a_fresh_checkout_restores_it_offline() {
        let root = scratch_root("ok");
        let built = root.join("built");
        fixture_build(&built, true);
        let out = root.join("release");
        let lines = package(
            &root,
            &built,
            "eastern-pa",
            "1n7",
            "region-data-2611-scenery-1n7-test",
            &out,
        )
        .unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("gh release create region-data-2611-scenery-1n7-test"))
        );
        assert!(lines.iter().any(|l| l.contains("gh release upload")));
        // The commands are pasted into a shell: quotes in the notes must balance.
        assert!(
            lines.iter().all(|l| l.matches('\'').count() % 2 == 0),
            "{lines:?}"
        );
        assert!(
            std::fs::read_to_string(out.join("PUBLISH.sh"))
                .unwrap()
                .contains("gh release create")
        );
        let installed = root.join("assets/terrain/eastern-pa/scenery/1n7");
        assert!(installed.join("ATTRIBUTION.txt").exists() && installed.join("tiles/a.glb").exists());
        // The lock now pins a scenery archive whose files all live under terrain/<region>/.
        let lock: data::Lock = download::json(&root.join("data.lock")).unwrap();
        assert_eq!(lock.assets.len(), 1);
        let asset = &lock.assets[0];
        assert_eq!(
            (asset.dataset.as_str(), asset.region.as_str()),
            ("scenery", "eastern-pa")
        );
        assert!(
            asset
                .files
                .iter()
                .all(|f| f.path.starts_with("terrain/eastern-pa/scenery/1n7/"))
        );
        assert!(asset.files.iter().any(|f| f.path.ends_with("tileset.json")));
        assert!(out.join(&asset.asset).exists() && out.join("eastern-pa-scenery.manifest.json").exists());
        // Simulate a downloaded app: nothing installed, cache seeded -> `data fetch --offline` installs it.
        std::fs::remove_dir_all(&installed).unwrap();
        data::fetch_offline_for_tests(&lock, &root.join("assets"), &root.join(".cache/data")).unwrap();
        assert!(installed.join("tileset.json").exists());
        assert_eq!(
            std::fs::read(installed.join("tiles/a.glb")).unwrap(),
            b"original fixture content"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn publishing_scenery_preserves_every_other_locked_archive() {
        let root = scratch_root("keep");
        let existing = serde_json::json!({"version":1,"base_url":"https://github.com/flightcraft-sim/flightcraft/releases/download","assets":[{
            "region":"eastern-pa","dataset":"charts","tag":"region-data-2611-radio-20261029","asset":"eastern-pa-charts.zip",
            "bytes":10,"sha256":"a".repeat(64),"files":[{"path":"terrain/eastern-pa/charts/x.webp","bytes":1,"sha256":"b".repeat(64)}]}]});
        std::fs::write(root.join("data.lock"), existing.to_string()).unwrap();
        let built = root.join("built");
        fixture_build(&built, true);
        package(
            &root,
            &built,
            "eastern-pa",
            "1n7",
            "region-data-2611-scenery-1n7-test",
            &root.join("release"),
        )
        .unwrap();
        let lock: data::Lock = download::json(&root.join("data.lock")).unwrap();
        let datasets: Vec<_> = lock
            .assets
            .iter()
            .map(|a| (a.dataset.as_str(), a.tag.as_str()))
            .collect();
        assert_eq!(
            datasets,
            [
                ("charts", "region-data-2611-radio-20261029"),
                ("scenery", "region-data-2611-scenery-1n7-test")
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unaccepted_or_preview_scenery_cannot_be_packaged_and_nothing_is_installed() {
        let root = scratch_root("refuse");
        let built = root.join("built");
        fixture_build(&built, false);
        let error = package(
            &root,
            &built,
            "eastern-pa",
            "1n7",
            "region-data-2611-scenery-1n7-test",
            &root.join("release"),
        )
        .unwrap_err();
        assert!(error.contains("does not record a pass"), "{error}");
        assert!(!root.join("assets/terrain/eastern-pa/scenery").exists());
        // A tileset that claims a preview is refused even when every audit passes.
        fixture_build(&built, true);
        let path = built.join("tileset.json");
        let mut set: serde_json::Value = download::json(&path).unwrap();
        set["extras"]["flightcraft"]["previewOnly"] = serde_json::json!(true);
        std::fs::write(&path, set.to_string()).unwrap();
        assert!(
            package(
                &root,
                &built,
                "eastern-pa",
                "1n7",
                "region-data-2611-scenery-1n7-test",
                &root.join("release")
            )
            .is_err()
        );
        // Local-only provenance can never be released.
        set["extras"]["flightcraft"]["previewOnly"] = serde_json::json!(false);
        set["extras"]["flightcraft"]["localOnly"] = serde_json::json!(true);
        std::fs::write(&path, set.to_string()).unwrap();
        assert!(
            package(
                &root,
                &built,
                "eastern-pa",
                "1n7",
                "region-data-2611-scenery-1n7-test",
                &root.join("release")
            )
            .is_err()
        );
        assert!(!root.join("assets/terrain/eastern-pa/scenery").exists());
        // An existing install is never overwritten.
        fixture_build(&built, true);
        std::fs::create_dir_all(root.join("assets/terrain/eastern-pa/scenery/1n7")).unwrap();
        assert!(
            package(
                &root,
                &built,
                "eastern-pa",
                "1n7",
                "region-data-2611-scenery-1n7-test",
                &root.join("release")
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn attribution_names_every_public_domain_source() {
        let root = scratch_root("attr");
        let built = root.join("built");
        fixture_build(&built, true);
        let text = attribution(&built, "eastern-pa", "1n7").unwrap();
        for needle in ["NAIP", "3DEP", "GEOID18", "public-domain", "sources.json"] {
            assert!(
                text.to_ascii_lowercase().contains(&needle.to_ascii_lowercase()),
                "{needle}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
