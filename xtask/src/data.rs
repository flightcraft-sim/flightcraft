//! Deterministic release archives. Publishing prepares files and prints commands, never uploads.
use super::download::{self, Http, MAX_DOWNLOAD, Transport, read, sha256, write_json};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub region: String,
    pub dataset: String,
    pub tag: String,
    pub asset: String,
    pub bytes: u64,
    pub sha256: String,
    pub files: Vec<FileEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    pub version: u8,
    pub base_url: String,
    pub assets: Vec<Asset>,
    /// Immutable upstream payloads used by scripted importers, separate from release assets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<download::Source>,
}

pub fn safe_path(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\\', ':'])
        && !name.starts_with('/')
        && name.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Shared by release publication and website packaging. Source restrictions are permanent:
/// accepting geometry never grants redistribution rights for portal footprints or TUM.
pub(crate) fn validate_scenery_release(set: &serde_json::Value) -> Result<(), String> {
    let flags = set
        .pointer("/extras/flightcraft")
        .ok_or("Scenery release metadata missing")?;
    flightcraft_render::scenery::validate_redistribution(flags)?;
    for gate in ["verticalDatumVerified", "runwayAlignmentVerified", "lidarBuilt"] {
        if flags.get(gate).and_then(|v| v.as_bool()) != Some(true) {
            return Err(format!("Scenery cannot ship before {gate} passes"));
        }
    }
    if flags.get("previewOnly").and_then(|v| v.as_bool()) != Some(false) {
        return Err("Preview scenery cannot be published".into());
    }
    Ok(())
}

pub fn files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    if !root.exists() {
        return Ok(result);
    }
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            result.extend(files(&entry.path())?);
        } else if kind.is_file() {
            result.push(entry.path());
        } else {
            return Err("data tree contains a symlink or non-regular file".into());
        }
        if result.len() > 100_000 {
            return Err("too many dataset files".into());
        }
    }
    result.sort();
    Ok(result)
}
fn checksum(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn release_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
}
fn validate(lock: &Lock) -> Result<(), String> {
    if lock.version != 1 || !lock.base_url.starts_with("https://") || lock.assets.len() > 100_000 {
        return Err("invalid data lock".into());
    }
    let mut names = BTreeSet::new();
    for source in &lock.sources {
        if !source.url.starts_with("https://")
            || source.url.contains('@')
            || !checksum(&source.sha256)
            || source.bytes > MAX_DOWNLOAD as u64
        {
            return Err("invalid pinned upstream source".into());
        }
    }
    for asset in &lock.assets {
        if !flightcraft_world::region_id(&asset.region)
            || !matches!(
                asset.dataset.as_str(),
                "terrain" | "charts" | "navigation" | "scenery"
            )
            || !release_name(&asset.asset)
            || !release_name(&asset.tag)
            || asset.bytes > MAX_DOWNLOAD as u64
            || !checksum(&asset.sha256)
            || asset.files.is_empty()
            || asset.files.len() > 100_000
        {
            return Err("invalid locked asset".into());
        }
        let mut total = 0_u64;
        for file in &asset.files {
            total = total.checked_add(file.bytes).ok_or("dataset size overflow")?;
            if !safe_path(&file.path)
                || !checksum(&file.sha256)
                || file.bytes > MAX_DOWNLOAD as u64
                || !names.insert(file.path.clone())
                || !file.path.starts_with(&format!("terrain/{}/", asset.region))
            {
                return Err("invalid or duplicate locked path".into());
            }
        }
        if total > MAX_DOWNLOAD as u64 {
            return Err("dataset exceeds 256 MiB; split the cell or dataset".into());
        }
    }
    Ok(())
}
pub fn fetch(offline: bool, cache: &Path, base: Option<&str>) -> Result<(), String> {
    let root = super::root()?;
    let lock: Lock = download::json(&root.join("data.lock"))?;
    fetch_with(&lock, &root.join("assets"), cache, base, offline, &mut Http)
}
/// Offline restore from a seeded cache, as `cargo xtask data fetch --offline` does.
#[cfg(test)]
pub(crate) fn fetch_offline_for_tests(lock: &Lock, assets: &Path, cache: &Path) -> Result<(), String> {
    fetch_with(lock, assets, cache, None, true, &mut Http)
}
fn fetch_with(
    lock: &Lock,
    assets: &Path,
    cache: &Path,
    base: Option<&str>,
    offline: bool,
    http: &mut impl Transport,
) -> Result<(), String> {
    validate(lock)?;
    let base = base.unwrap_or(&lock.base_url);
    if !base.starts_with("https://") || base.contains('@') {
        return Err("data base URL must be credential-free HTTPS".into());
    }
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    for asset in &lock.assets {
        let installed = asset.files.iter().all(|f| {
            read(&assets.join(&f.path), MAX_DOWNLOAD)
                .is_ok_and(|b| b.len() as u64 == f.bytes && sha256(&b) == f.sha256)
        });
        if installed {
            continue;
        }
        let path = cache.join(&asset.sha256);
        let bytes = if path.exists() {
            read(&path, MAX_DOWNLOAD)?
        } else {
            if offline {
                return Err(format!(
                    "{} absent locally and in cache; run cargo xtask data fetch online",
                    asset.asset
                ));
            }
            http.get(
                &format!("{}/{}/{}", base.trim_end_matches('/'), asset.tag, asset.asset),
                MAX_DOWNLOAD,
            )?
        };
        if bytes.len() as u64 != asset.bytes || sha256(&bytes) != asset.sha256 {
            return Err(format!("{} checksum or size mismatch", asset.asset));
        }
        let decoded = unpack(&bytes, &asset.files)?;
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        // The archive is fully validated before any installed file changes.
        for (name, bytes) in decoded {
            let path = assets.join(name);
            let parent = path.parent().ok_or("asset needs parent")?;
            // Reject symlink components in the destination tree as well as archive entries.
            let mut current = assets.to_path_buf();
            for component in path.strip_prefix(assets).map_err(|e| e.to_string())?.components() {
                current.push(component);
                if std::fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
                    return Err("symlink in asset destination".into());
                }
            }
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let pending = path.with_extension("download-pending");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&pending)
                .map_err(|e| e.to_string())?;
            if let Err(error) = file.write_all(&bytes) {
                let _ = std::fs::remove_file(&pending);
                return Err(error.to_string());
            }
            std::fs::rename(pending, path).map_err(|e| e.to_string())?;
        }
        eprintln!("data: verified {}", asset.asset);
    }
    Ok(())
}
fn unpack(bytes: &[u8], expected: &[FileEntry]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    if archive.len() != expected.len() + 1 {
        return Err("archive file count differs from lock".into());
    }
    let mut result = BTreeMap::new();
    let mut manifest = false;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        if !safe_path(&name)
            || entry.is_dir()
            || entry.encrypted()
            || entry
                .unix_mode()
                .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000))
        {
            return Err("unsafe archive entry".into());
        }
        let limit = if name == "manifest.json" {
            16 * 1024 * 1024
        } else {
            expected
                .iter()
                .find(|f| f.path == name)
                .ok_or("unexpected archive entry")?
                .bytes
        };
        if entry.size() > limit {
            return Err("archive entry exceeds bound".into());
        }
        let mut data = Vec::new();
        (&mut entry)
            .take(limit + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        if name == "manifest.json" {
            let actual: Vec<FileEntry> = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
            if manifest
                || serde_json::to_value(actual).map_err(|e| e.to_string())?
                    != serde_json::to_value(expected).map_err(|e| e.to_string())?
            {
                return Err("archive manifest differs from lock".into());
            }
            manifest = true;
        } else {
            let file = expected
                .iter()
                .find(|f| f.path == name)
                .ok_or("unexpected archive file")?;
            if file.bytes != data.len() as u64
                || file.sha256 != sha256(&data)
                || result.insert(name, data).is_some()
            {
                return Err("archive file checksum, size, or duplication".into());
            }
        }
    }
    if !manifest || result.len() != expected.len() {
        return Err("missing archive files".into());
    }
    Ok(result)
}

pub fn publish(tag: &str, output: &Path) -> Result<(), String> {
    publish_only(tag, output, None)
}
/// With `only`, just that dataset's archives are rebuilt and merged into the existing lock;
/// every other locked archive (and its release tag) is left exactly as pinned. This lets one
/// dataset, such as 1N7 scenery, ship in its own release without the full region data tree.
pub fn publish_only(tag: &str, output: &Path, only: Option<&str>) -> Result<(), String> {
    publish_at(&super::root()?, tag, output, only)
}
/// `publish_only` against an explicit repository root (tests use a scratch tree).
pub(crate) fn publish_at(root: &Path, tag: &str, output: &Path, only: Option<&str>) -> Result<(), String> {
    if !release_name(tag) {
        return Err("invalid release tag".into());
    }
    if only.is_some_and(|d| !matches!(d, "terrain" | "charts" | "navigation" | "scenery")) {
        return Err("unknown dataset for --only".into());
    }
    let assets = root.join("assets");
    let previous: Lock = download::json(&root.join("data.lock"))?;
    let mut lock = Lock {
        version: 1,
        base_url: "https://github.com/flightcraft-sim/flightcraft/releases/download".into(),
        assets: Vec::new(),
        sources: previous.sources.clone(),
    };
    let mut replaced = BTreeSet::new();
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
    for region in std::fs::read_dir(assets.join("terrain")).map_err(|e| e.to_string())? {
        let region = region.map_err(|e| e.to_string())?;
        if !region.path().join("region.json").exists() {
            continue;
        }
        let id = region
            .file_name()
            .to_str()
            .ok_or("region ID requires UTF-8")?
            .to_owned();
        for dataset in ["terrain", "charts", "navigation", "scenery"] {
            if only.is_some_and(|d| d != dataset) {
                continue;
            }
            let paths: Vec<_> = files(&region.path())?
                .into_iter()
                .filter(|p| {
                    let scenery = p
                        .strip_prefix(region.path())
                        .is_ok_and(|p| p.starts_with("scenery"));
                    if dataset == "scenery" {
                        scenery
                    } else if scenery {
                        false
                    } else if dataset == "terrain" {
                        p.extension().is_some_and(|e| e == "fcht")
                    } else if dataset == "charts" {
                        p.extension().is_some_and(|e| e == "png" || e == "webp")
                    } else {
                        p.extension().is_some_and(|e| e == "json" || e == "md")
                    }
                })
                .collect();
            if paths.is_empty() {
                if dataset == "terrain" || (dataset == "charts" && region.path().join("charts").exists()) {
                    return Err(format!(
                        "{} {dataset} data absent; run cargo xtask data fetch before publishing",
                        id
                    ));
                }
                continue;
            }
            if dataset == "scenery" {
                if !paths
                    .iter()
                    .any(|p| p.file_name().is_some_and(|n| n == "tileset.json"))
                {
                    return Err("scenery needs a tileset manifest".into());
                }
                for path in paths
                    .iter()
                    .filter(|p| p.file_name().is_some_and(|n| n == "tileset.json"))
                {
                    let bytes = read(path, flightcraft_world::tiles3d::MAX_TILESET_BYTES)?;
                    flightcraft_world::tiles3d::Tileset::from_json(&bytes).map_err(|e| e.to_string())?;
                    let set: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                    validate_scenery_release(&set)?;
                }
            }
            // 128 MiB raw tile ranges leave room for ZIP headers and the manifest beneath
            // the existing 256 MiB verified transport cap (also below GitHub's 2 GB cap).
            let mut groups = Vec::<Vec<PathBuf>>::new();
            let mut group = Vec::new();
            let mut total = 0_u64;
            for path in paths {
                let bytes = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                if bytes > 128 * 1024 * 1024 {
                    return Err("release tile exceeds 128 MiB".into());
                }
                if total + bytes > 128 * 1024 * 1024 && !group.is_empty() {
                    groups.push(std::mem::take(&mut group));
                    total = 0;
                }
                total += bytes;
                group.push(path);
            }
            if !group.is_empty() {
                groups.push(group);
            }
            let split = groups.len() > 1;
            let mut first = 0;
            for paths in groups {
                let last = first + paths.len() - 1;
                let suffix = if split {
                    format!("-{first:06}-{last:06}")
                } else {
                    String::new()
                };
                replaced.insert((id.clone(), dataset.to_owned()));
                lock.assets
                    .push(archive(&assets, &paths, output, &id, dataset, tag, &suffix)?);
                first = last + 1;
            }
        }
    }
    if only.is_some() {
        // Keep every pinned archive this run did not regenerate.
        lock.assets.extend(
            previous
                .assets
                .into_iter()
                .filter(|a| !replaced.contains(&(a.region.clone(), a.dataset.clone()))),
        );
    }
    lock.assets
        .sort_by(|a, b| (&a.region, &a.dataset, &a.asset).cmp(&(&b.region, &b.dataset, &b.asset)));
    validate(&lock)?;
    write_json(&root.join("data.lock"), &lock)?;
    // Seed the offline cache so removal from the checkout never removes the maintainer's data.
    let cache = root.join(".cache/data");
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    for asset in lock.assets.iter().filter(|a| output.join(&a.asset).exists()) {
        std::fs::copy(output.join(&asset.asset), cache.join(&asset.sha256)).map_err(|e| e.to_string())?;
    }
    for line in release_commands(tag, output, only) {
        println!("{line}");
    }
    Ok(())
}

/// The exact commands that publish what `publish` wrote. Nothing here is run by xtask.
pub(crate) fn release_commands(tag: &str, output: &Path, only: Option<&str>) -> Vec<String> {
    let notes = if only == Some("scenery") {
        "Public-domain scenery tiles built from USDA NAIP, USGS 3DEP lidar/DEM, USGS NHD and NOAA NGS GEOID18; see sources.json and ATTRIBUTION.txt in each scenery directory."
    } else {
        "Public-domain terrain and FAA data; see manifests and ATTRIBUTION.md."
    };
    let title = if only == Some("scenery") {
        "Scenery data"
    } else {
        "Region data"
    };
    vec![
        format!(
            "gh release create {tag} --repo flightcraft-sim/flightcraft --title '{title} {tag}' --notes '{notes}'"
        ),
        format!(
            "gh release upload {tag} {}/*.zip {}/*.manifest.json --repo flightcraft-sim/flightcraft",
            output.display(),
            output.display()
        ),
    ]
}

fn archive(
    assets: &Path,
    paths: &[PathBuf],
    output: &Path,
    id: &str,
    dataset: &str,
    tag: &str,
    suffix: &str,
) -> Result<Asset, String> {
    let mut entries = Vec::new();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for path in paths {
        let name = path
            .strip_prefix(assets)
            .map_err(|e| e.to_string())?
            .to_str()
            .ok_or("UTF-8 asset path")?
            .replace('\\', "/");
        let bytes = read(path, MAX_DOWNLOAD)?;
        entries.push(FileEntry {
            path: name.clone(),
            bytes: bytes.len() as u64,
            sha256: sha256(&bytes),
        });
        writer.start_file(&name, options).map_err(|e| e.to_string())?;
        writer.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    writer
        .start_file("manifest.json", options)
        .map_err(|e| e.to_string())?;
    writer
        .write_all(&serde_json::to_vec_pretty(&entries).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let bytes = writer.finish().map_err(|e| e.to_string())?.into_inner();
    if bytes.len() > MAX_DOWNLOAD {
        return Err("release range exceeds transport bound".into());
    }
    let asset = format!("{id}-{dataset}{suffix}.zip");
    std::fs::write(output.join(&asset), &bytes).map_err(|e| e.to_string())?;
    write_json(
        &output.join(format!("{id}-{dataset}{suffix}.manifest.json")),
        &entries,
    )?;
    Ok(Asset {
        region: id.into(),
        dataset: dataset.into(),
        tag: tag.into(),
        asset,
        bytes: bytes.len() as u64,
        sha256: sha256(&bytes),
        files: entries,
    })
}

pub fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(2);
    let action = args.next().ok_or("data fetch|publish")?;
    let (mut offline, mut base) = (false, None);
    let mut cache = super::root()?.join(".cache/data");
    let mut output = super::root()?.join("out/data");
    let mut tag = "region-data-2611".to_string();
    let mut baseline = None;
    let mut only = None;
    while let Some(key) = args.next() {
        if key == "--offline" {
            offline = true;
            continue;
        }
        let value = args.next().ok_or("missing data option value")?;
        match key.as_str() {
            "--cache" => cache = value.into(),
            "--out" => output = value.into(),
            "--tag" => tag = value,
            "--base-url" => base = Some(value),
            "--baseline" => baseline = Some(PathBuf::from(value)),
            "--only" => only = Some(value),
            _ => return Err(format!("unknown data option {key}")),
        }
    }
    let base = base.or_else(|| std::env::var("FLIGHTCRAFT_DATA_URL").ok());
    match action.as_str() {
        "fetch" => fetch(offline, &cache, base.as_deref()),
        "publish" => publish_only(&tag, &output, only.as_deref()),
        "measure" => super::validation::measure(),
        "compact" => compact(),
        "compare" => compare_charts(&baseline.ok_or("compare needs --baseline DIR")?),
        _ => Err("data fetch|publish".into()),
    }
}

/// Migrate installed showcase assets without resampling or changing chart pixels.
fn compact() -> Result<(), String> {
    let root = super::root()?;
    for path in files(&root.join("assets/terrain"))? {
        if path.extension().is_some_and(|e| e == "fcht")
            && !path.components().any(|c| c.as_os_str() == "training-valley")
        {
            let tile =
                flightcraft_world::HeightTile::read(&read(&path, flightcraft_world::MAX_REGION_TILE_BYTES)?)
                    .map_err(|e| e.to_string())?;
            std::fs::write(path, tile.write_compact().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
    }
    for id in ["new-york", "eastern-pa"] {
        let directory = root.join("assets/terrain").join(id);
        let report = directory.join("validation.json");
        if report.exists() {
            let mut report: super::validation::Report = download::json(&report)?;
            report.bytes = files(&directory)?
                .iter()
                .filter(|p| {
                    p.file_name()
                        .is_none_or(|n| n != "validation.json" && n != "validation.md")
                })
                .map(|p| std::fs::metadata(p).map(|m| m.len()).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum();
            super::validation::write(&directory, &report)?;
        }
    }
    Ok(())
}

fn compare_charts(baseline: &Path) -> Result<(), String> {
    let root = super::root()?;
    let mut result = Vec::new();
    for id in ["new-york", "eastern-pa"] {
        let mut count = 0;
        let mut different = 0;
        let previous = baseline.join(id).join("charts");
        for path in files(&previous)? {
            if path.extension().is_none_or(|e| e != "png") {
                continue;
            }
            let relative = path.strip_prefix(&previous).map_err(|e| e.to_string())?;
            let current = root
                .join("assets/terrain")
                .join(id)
                .join("charts")
                .join(relative)
                .with_extension("webp");
            let a = image::open(&path).map_err(|e| e.to_string())?.into_rgba8();
            let b = image::open(current).map_err(|e| e.to_string())?.into_rgba8();
            count += 1;
            if a != b {
                different += 1;
            }
        }
        if count == 0 {
            return Err("baseline has no PNG chart tiles".into());
        }
        result.push(serde_json::json!({"region":id,"tiles":count,"rgba_differences":different}));
    }
    write_json(&root.join("datasets/chart-comparison.json"), &result)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn verified_release_fetch_restores_offline_and_rejects_corruption() {
        let root = std::env::temp_dir().join(format!("flightcraft-data-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = "terrain/example/000.fcht";
        let data = b"fixture tile bytes";
        let files = vec![FileEntry {
            path: path.into(),
            bytes: data.len() as u64,
            sha256: sha256(data),
        }];
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file(path, options).unwrap();
        writer.write_all(data).unwrap();
        writer.start_file("manifest.json", options).unwrap();
        writer.write_all(&serde_json::to_vec(&files).unwrap()).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let asset = Asset {
            region: "example".into(),
            dataset: "terrain".into(),
            tag: "region-data-2611".into(),
            asset: "example-terrain.zip".into(),
            bytes: bytes.len() as u64,
            sha256: sha256(&bytes),
            files,
        };
        let lock = Lock {
            version: 1,
            base_url: "https://example.test/data".into(),
            assets: vec![asset.clone()],
            sources: Vec::new(),
        };
        let url = format!("{}/{}/{}", lock.base_url, asset.tag, asset.asset);
        let mut http = download::Recorded(BTreeMap::from([(url, bytes)]));
        fetch_with(
            &lock,
            &root.join("assets"),
            &root.join("cache"),
            None,
            false,
            &mut http,
        )
        .unwrap();
        assert_eq!(read(&root.join("assets").join(path), 100).unwrap(), data);
        std::fs::remove_dir_all(root.join("assets")).unwrap();
        http.0.clear();
        fetch_with(
            &lock,
            &root.join("assets"),
            &root.join("cache"),
            None,
            true,
            &mut http,
        )
        .unwrap();
        std::fs::remove_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("cache").join(&asset.sha256), b"bad").unwrap();
        assert!(
            fetch_with(
                &lock,
                &root.join("assets"),
                &root.join("cache"),
                None,
                true,
                &mut http
            )
            .is_err()
        );
        assert!(!root.join("assets").exists());
        let mut invalid = lock.clone();
        if let Some(asset) = invalid.assets.first_mut() {
            asset.sha256 = "../".repeat(22);
        }
        assert!(validate(&invalid).is_err());
        assert!(!safe_path("../escape"));
        assert!(!safe_path("/absolute"));
        assert!(!safe_path("a\\b"));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod scenery_release_tests {
    use super::*;
    #[test]
    fn scenery_release_rejects_portal_and_tum_even_with_stale_accepted_flags() {
        for area in ["nyc-footprints", "nyc-lod2", "nyc-reviewed-glb"] {
            let set = serde_json::json!({"extras":{"flightcraft":{"area":area,"localOnly":false,"redistributionAllowed":true,"previewOnly":false,"verticalDatumVerified":true,"runwayAlignmentVerified":true,"lidarBuilt":true}}});
            assert!(validate_scenery_release(&set).unwrap_err().contains("Local-only"));
        }
    }
}
