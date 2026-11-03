//! Provider boundary for local and opt-in streaming sources. The renderer receives GLB bytes,
//! attribution, and cache policy; it never sees account keys or provider-specific endpoints.
#[cfg(not(target_arch = "wasm32"))]
use super::Location;
use super::Provider;
use flightcraft_world::tiles3d;
use std::path::{Path, PathBuf};

/// Local redistributed data may be retained. A streaming adapter must use MemoryOnly and
/// supply an expiry when its terms require one; an expired payload must leave CPU/GPU caches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachePolicy {
    LocalData,
    MemoryOnly,
}
pub struct Content {
    pub bytes: Vec<u8>,
    pub attribution: Vec<String>,
    pub cache_policy: CachePolicy,
    pub expires_at_unix_ms: Option<u64>,
}
/// Implementations resolve authentication privately from user config using token_config_key.
/// No API accepts a credential as part of a tileset URL or a release source receipt.
pub trait TileProvider: Send {
    fn tileset(&mut self) -> Result<Vec<u8>, String>;
    fn content(&mut self, uri: &str) -> Result<Content, String>;
}
/// Local user-fetched or redistributable data. No HTTP or persistent network cache.
pub struct LocalProvider {
    root: PathBuf,
    tileset: PathBuf,
}
impl LocalProvider {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(settings: &Provider) -> Result<Self, String> {
        if !settings.enabled {
            return Err("scenery provider is disabled".into());
        }
        if settings.token_config_key.is_some() {
            return Err("local scenery does not use tokens".into());
        }
        let Location::LocalPath(path) = &settings.location else {
            return Err("URL scenery providers are not implemented; use a local tileset".into());
        };
        let root = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            root,
            tileset: path.clone(),
        })
    }
    pub fn open(settings: &Provider, origin: [f64; 2]) -> Result<super::Stream, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            super::Stream::with_provider(Box::new(Self::new(settings)?), origin)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (settings, origin);
            Err("local filesystem scenery is unavailable in the browser".into())
        }
    }
}
impl TileProvider for LocalProvider {
    fn tileset(&mut self) -> Result<Vec<u8>, String> {
        read(&self.tileset, tiles3d::MAX_TILESET_BYTES)
    }
    fn content(&mut self, uri: &str) -> Result<Content, String> {
        let path = contained_path(&self.root, uri)?;
        Ok(Content {
            bytes: read(&path, tiles3d::MAX_TILE_CONTENT_BYTES)?,
            attribution: Vec::new(),
            cache_policy: CachePolicy::LocalData,
            expires_at_unix_ms: None,
        })
    }
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err("scenery needs a bounded regular file".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("scenery content exceeds limit".into());
    }
    Ok(bytes)
}
fn contained_path(root: &Path, uri: &str) -> Result<PathBuf, String> {
    if !tiles3d::local_uri(uri) {
        return Err("unsafe local scenery URI".into());
    }
    let path = root.join(uri).canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(root) {
        return Err("scenery content escapes tileset directory".into());
    }
    Ok(path)
}
