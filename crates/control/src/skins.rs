//! Installed skin packs: artwork for a display model, loaded apart from aircraft packs.
//!
//! A skin pack never carries behaviour. The session keeps the packs the host loaded; a display
//! shows the skin the pilot chose, else the one its aircraft pack recommends, else the built-in
//! public skin. An unknown or unloadable skin is never an error at draw time.
use crate::Session;
pub use flightcraft_aero::skin_pack::{
    MAX_SKIN_FONT_BYTES, MAX_SKIN_MANIFEST_BYTES, MAX_SKIN_PACK_BYTES, MAX_SKINS, SkinPackManifest,
};
use std::collections::BTreeMap;

/// The pilot's explicit choice of the engine's own skin, which overrides a recommendation.
pub const BUILT_IN_SKIN: &str = "built-in";

#[derive(Clone, Debug)]
pub struct SkinContents {
    pub manifest: SkinPackManifest,
    /// Declared assets by path (fonts).
    pub assets: BTreeMap<String, Vec<u8>>,
}

impl SkinContents {
    pub fn validate(&self) -> Result<(), String> {
        self.manifest.validate().map_err(|e| e.to_string())?;
        let mut total = 0usize;
        for a in &self.manifest.assets {
            let bytes = self
                .assets
                .get(&a.path)
                .ok_or_else(|| format!("skin asset {} is missing", a.path))?;
            if bytes.len() > MAX_SKIN_FONT_BYTES {
                return Err(format!("skin asset {} exceeds 4 MiB", a.path));
            }
            total += bytes.len();
        }
        if total > MAX_SKIN_PACK_BYTES || self.assets.len() != self.manifest.assets.len() {
            return Err("skin pack assets exceed the limit or are not declared".into());
        }
        Ok(())
    }
    /// The chrome font bytes, if the skin has one.
    pub fn font(&self) -> Option<&[u8]> {
        self.manifest
            .font
            .as_ref()
            .and_then(|p| self.assets.get(p))
            .map(Vec::as_slice)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SkinRegistry {
    skins: BTreeMap<String, SkinContents>,
}

impl Session {
    /// Install a skin pack. A pack with the same id replaces the earlier one.
    pub fn install_skin(&mut self, skin: SkinContents) -> Result<(), String> {
        skin.validate()?;
        if !self.skins.skins.contains_key(&skin.manifest.id) && self.skins.skins.len() >= MAX_SKINS {
            return Err("at most 32 skin packs".into());
        }
        self.skins.skins.insert(skin.manifest.id.clone(), skin);
        Ok(())
    }
    pub fn skin(&self, id: &str) -> Option<&SkinContents> {
        self.skins.skins.get(id)
    }
    /// Installed skins in id order.
    pub fn skins(&self) -> impl Iterator<Item = &SkinContents> {
        self.skins.skins.values()
    }
    /// Skins that dress `model`, in id order.
    pub fn skins_for(&self, model: flightcraft_aero::display_model::DisplayModel) -> Vec<&SkinContents> {
        self.skins()
            .filter(|s| s.manifest.display_model == model)
            .collect()
    }
    /// The skin a display shows: choice, else recommendation, each only if installed and for the model.
    pub(crate) fn resolve_skin(
        &self,
        model: Option<flightcraft_aero::display_model::DisplayModel>,
        chosen: Option<&str>,
        recommended: Option<&str>,
    ) -> Option<String> {
        let model = model?;
        let usable = |id: &str| {
            self.skin(id)
                .filter(|s| s.manifest.display_model == model)
                .map(|s| s.manifest.id.clone())
        };
        match chosen {
            Some(BUILT_IN_SKIN) => None,
            Some(id) if usable(id).is_some() => usable(id),
            _ => recommended.and_then(usable),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_skin(directory: &std::path::Path) -> Result<SkinContents, String> {
    use std::io::Read;
    fn regular(root: &std::path::Path, relative: &str, max: usize) -> Result<Vec<u8>, String> {
        let mut path = root.to_path_buf();
        for part in relative.split('/') {
            path.push(part);
            if std::fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("symlinks are not permitted in skin packs".into());
            }
        }
        if !path.canonicalize().map_err(|e| e.to_string())?.starts_with(root) {
            return Err("file escapes skin directory".into());
        }
        if !std::fs::metadata(&path).map_err(|e| e.to_string())?.is_file() {
            return Err("requires a regular file".into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(max as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > max {
            return Err(format!("{relative} exceeds {max} bytes"));
        }
        Ok(bytes)
    }
    if std::fs::symlink_metadata(directory)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("symlink skin directories are not permitted".into());
    }
    let root = directory.canonicalize().map_err(|e| e.to_string())?;
    let text = String::from_utf8(regular(&root, "skin.json", MAX_SKIN_MANIFEST_BYTES)?)
        .map_err(|e| format!("skin.json requires UTF-8: {e}"))?;
    let manifest = SkinPackManifest::from_json(&text).map_err(|e| e.to_string())?;
    let mut assets = BTreeMap::new();
    for a in &manifest.assets {
        assets.insert(a.path.clone(), regular(&root, &a.path, MAX_SKIN_FONT_BYTES)?);
    }
    let skin = SkinContents { manifest, assets };
    skin.validate()?;
    Ok(skin)
}

impl Session {
    /// Each bad skin is reported and skipped. A missing directory is not an error.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_skins_directory(&mut self, directory: &std::path::Path) -> Vec<String> {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => return vec![format!("skins directory: {e}")],
        };
        let mut errors = Vec::new();
        let mut paths = Vec::new();
        for entry in entries.take(MAX_SKINS + 1) {
            match entry {
                Ok(e) => paths.push(e.path()),
                Err(e) => errors.push(e.to_string()),
            }
        }
        if paths.len() > MAX_SKINS {
            errors.push("skins directory: at most 32 entries; skipped directory".into());
            return errors;
        }
        paths.sort();
        for path in paths.into_iter().filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                && p.join("skin.json").exists()
        }) {
            if let Err(e) = read_skin(&path).and_then(|s| self.install_skin(s)) {
                errors.push(format!(
                    "skin {}: {e}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }
        errors
    }
}
