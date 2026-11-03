use super::*;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub(super) fn directory() -> Result<PathBuf, CommandError> {
    crate::navigation_data_directory()
        .map(|p| p.join("flights"))
        .ok_or_else(|| invalid("flight.save", "application data directory unavailable"))
}
fn read(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
        return Err("flight file exceeds 128 MiB".into());
    }
    let mut text = String::new();
    file.take(128 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    Ok(text)
}
pub(super) fn save(directory: &Path, saved: &SavedFlight) -> Result<(), String> {
    if !valid_id(&saved.summary.id) {
        return Err("invalid flight id".into());
    }
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(saved).map_err(|e| e.to_string())?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err("flight file exceeds 128 MiB".into());
    }
    let pending = directory.join(format!("{}.pending", saved.summary.id));
    let path = directory.join(format!("{}.json", saved.summary.id));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        // Windows does not replace an existing target with rename. Keep the previous complete file as a backup until the new file is installed.
        let backup = directory.join(format!("{}.backup", saved.summary.id));
        if path.exists() {
            if backup.exists() {
                std::fs::remove_file(&backup).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&path, &backup).map_err(|e| e.to_string())?;
        }
        if let Err(e) = std::fs::rename(&pending, &path) {
            let _ = std::fs::rename(&backup, &path);
            return Err(e.to_string());
        }
        if backup.exists() {
            std::fs::remove_file(backup).map_err(|e| e.to_string())?;
        }
        #[cfg(unix)]
        std::fs::File::open(directory)
            .and_then(|d| d.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&pending);
    }
    result
}
pub(super) fn load(directory: &Path, id: &str) -> Result<SavedFlight, String> {
    if !valid_id(id) {
        return Err("invalid flight id".into());
    }
    let path = directory.join(format!("{id}.json"));
    let path = if path.exists() {
        path
    } else {
        directory.join(format!("{id}.backup"))
    };
    let saved = SavedFlight::from_json(&read(&path)?)?;
    if saved.summary.id != id {
        return Err("flight id differs from file".into());
    }
    Ok(saved)
}
pub(super) fn list(directory: &Path) -> Result<Vec<FlightSummary>, String> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut result = BTreeSet::new();
    for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json" || e == "backup")
            && entry.file_type().map_err(|e| e.to_string())?.is_file()
            && let Some(id) = path.file_stem().and_then(|v| v.to_str())
        {
            result.insert(id.to_owned());
        }
        if result.len() > 10000 {
            return Err("flight archive listing exceeds 10000 files".into());
        }
    }
    let mut flights = Vec::new();
    for id in result {
        flights.push(load(directory, &id)?.summary);
    }
    flights.sort_by(|a, b| b.created_unix_ms.cmp(&a.created_unix_ms));
    Ok(flights)
}
use std::collections::BTreeSet;
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn durable_round_trip_reopen_and_hostile_paths() {
        let root = std::env::temp_dir().join(format!(
            "flightcraft-archive-{}-{}",
            std::process::id(),
            flight_nonce()
        ));
        let saved = SavedFlight {
            summary: FlightSummary {
                id: "flight-123".into(),
                created_unix_ms: 123,
                aircraft: "corvid".into(),
                registration: "N371FC".into(),
                duration_s: 42.,
                events: 0,
            },
            record: FlightRecord::default(),
        };
        save(&root, &saved).unwrap();
        save(&root, &saved).unwrap();
        assert_eq!(list(&root).unwrap().len(), 1);
        assert_eq!(load(&root, "flight-123").unwrap().summary.duration_s, 42.);
        assert!(load(&root, "../escape").is_err());
        std::fs::rename(root.join("flight-123.json"), root.join("flight-123.backup")).unwrap();
        assert_eq!(list(&root).unwrap().len(), 1);
        assert_eq!(load(&root, "flight-123").unwrap().summary.id, "flight-123");
        std::fs::remove_dir_all(root).unwrap();
    }
}
