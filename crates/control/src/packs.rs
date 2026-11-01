//! Pack transports stay above the deterministic aircraft definition layer.
use crate::Session;
pub use flightcraft_aero::{
    AircraftCatalog, MAX_AIRCRAFT_BYTES, MAX_MANIFEST_BYTES, MAX_PACK_BYTES, MAX_PACK_FILES, MAX_PACKS,
    PackManifest,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct PackContents {
    pub manifest: PackManifest,
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
pub struct PackOptions {
    /// Native directory override. The app persists it as a preference.
    pub directory: Option<String>,
    /// Browser transports fetch these before starting the app.
    pub fetched: Vec<PackContents>,
    pub errors: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_bounded(path: &std::path::Path, max: usize) -> Result<String, String> {
    use std::io::Read;
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err("requires a regular file".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("requires a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err(format!("file_size: exceeds {max} bytes"));
    }
    String::from_utf8(bytes).map_err(|e| format!("requires UTF-8: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_pack(directory: &std::path::Path) -> Result<PackContents, String> {
    fn regular_descendant(root: &std::path::Path, relative: &str) -> Result<std::path::PathBuf, String> {
        let mut path = root.to_path_buf();
        for part in relative.split('/') {
            path.push(part);
            if std::fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("symlinks are not permitted in packs".into());
            }
        }
        if !path.canonicalize().map_err(|e| e.to_string())?.starts_with(root) {
            return Err("file escapes pack directory".into());
        }
        Ok(path)
    }
    if std::fs::symlink_metadata(directory)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("symlink pack directories are not permitted".into());
    }
    let root = directory.canonicalize().map_err(|e| e.to_string())?;
    let manifest_text = read_bounded(&regular_descendant(&root, "pack.json")?, MAX_MANIFEST_BYTES)?;
    let manifest = PackManifest::from_json(&manifest_text).map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    let mut total = manifest_text.len();
    for entry in &manifest.files {
        let text = read_bounded(&regular_descendant(&root, &entry.path)?, MAX_AIRCRAFT_BYTES)?;
        total = total.saturating_add(text.len());
        if total > MAX_PACK_BYTES {
            return Err("pack_size: exceeds 8 MiB".into());
        }
        files.insert(entry.path.clone(), text);
    }
    Ok(PackContents { manifest, files })
}

impl Session {
    /// Each bad pack is reported and skipped; independent good packs remain available.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_packs_directory(&mut self, directory: &std::path::Path) -> Vec<String> {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => return vec![format!("packs directory: {e}")],
        };
        let mut errors = Vec::new();
        let mut paths = Vec::new();
        for entry in entries.take(MAX_PACKS + 1) {
            match entry {
                Ok(e) => paths.push(e.path()),
                Err(e) => errors.push(e.to_string()),
            }
        }
        if paths.len() > MAX_PACKS {
            errors.push("packs directory: at most 64 entries; skipped directory".into());
            return errors;
        }
        paths.sort();
        for path in paths {
            if !path.is_dir() {
                continue;
            }
            match read_pack(&path).and_then(|pack| self.install_pack(&pack).map_err(|e| e.to_string())) {
                Ok(()) => {}
                Err(e) => errors.push(format!(
                    "pack {}: {e}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )),
            }
        }
        errors
    }
}
