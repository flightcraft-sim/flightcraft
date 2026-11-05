//! Discovery of scenery installed by `cargo xtask data fetch` (desktop).
//!
//! Data releases unpack to `<regions>/<region>/scenery/<area>/tileset.json`. The app uses such a
//! tileset automatically only when its own metadata says it passed the release gates: vertical
//! datum, painted-runway alignment and lidar acceptance, not preview-only, and redistributable.
//! A tileset the user typed into Preferences is never second-guessed.
use std::path::{Path, PathBuf};

const MAX_TILESET_BYTES: u64 = 16 * 1024 * 1024;

fn accepted(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() > MAX_TILESET_BYTES {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let Ok(set) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    let Some(flags) = set.pointer("/extras/flightcraft") else {
        return false;
    };
    let on = |key: &str| flags.get(key).and_then(|v| v.as_bool());
    flightcraft_render::scenery::validate_redistribution(flags).is_ok()
        && on("verticalDatumVerified") == Some(true)
        && on("runwayAlignmentVerified") == Some(true)
        && on("lidarBuilt") == Some(true)
        && on("previewOnly") == Some(false)
}

/// The first accepted installed tileset (by directory name) for `region`, if any.
pub fn installed_tileset(regions: &Path, region: &str) -> Option<PathBuf> {
    if region.is_empty() || region.contains(['/', '\\', '.']) {
        return None;
    }
    let directory = regions.join(region).join("scenery");
    let mut areas: Vec<String> = std::fs::read_dir(&directory)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            entry
                .file_type()
                .ok()?
                .is_dir()
                .then(|| entry.file_name().into_string().ok())?
        })
        .collect();
    areas.sort();
    areas
        .into_iter()
        .map(|area| directory.join(area).join("tileset.json"))
        .find(|path| accepted(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, area: &str, extras: serde_json::Value) {
        let dir = root.join("eastern-pa/scenery").join(area);
        std::fs::create_dir_all(&dir).unwrap();
        let set =
            serde_json::json!({"asset":{"version":"1.1"},"geometricError":0,"extras":{"flightcraft":extras}});
        std::fs::write(dir.join("tileset.json"), serde_json::to_vec(&set).unwrap()).unwrap();
    }
    fn good() -> serde_json::Value {
        serde_json::json!({"verticalDatumVerified":true,"runwayAlignmentVerified":true,"lidarBuilt":true,"previewOnly":false,"localOnly":false,"redistributionAllowed":true})
    }
    #[test]
    fn only_accepted_redistributable_scenery_is_used_automatically() {
        let root = std::env::temp_dir().join(format!("fc-installed-scenery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(installed_tileset(&root, "eastern-pa").is_none());
        for (area, key, value) in [
            ("a-preview", "previewOnly", serde_json::json!(true)),
            ("b-runway", "runwayAlignmentVerified", serde_json::json!(false)),
            ("c-lidar", "lidarBuilt", serde_json::json!(false)),
            ("d-datum", "verticalDatumVerified", serde_json::json!(false)),
            ("e-local", "localOnly", serde_json::json!(true)),
        ] {
            let mut flags = good();
            flags[key] = value;
            write(&root, area, flags);
        }
        assert!(installed_tileset(&root, "eastern-pa").is_none());
        write(&root, "f-1n7", good());
        write(&root, "g-later", good());
        let found = installed_tileset(&root, "eastern-pa").unwrap();
        assert!(found.ends_with("eastern-pa/scenery/f-1n7/tileset.json"));
        assert!(installed_tileset(&root, "new-york").is_none());
        assert!(installed_tileset(&root, "../eastern-pa").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
