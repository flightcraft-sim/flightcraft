//! Bounded HTTP transport, content-addressed cache, and reviewable source receipts.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

pub const MAX_DOWNLOAD: usize = 256 * 1024 * 1024;

pub trait Transport {
    fn get(&mut self, url: &str, limit: usize) -> Result<Vec<u8>, String>;
}

pub struct Http;
impl Transport for Http {
    fn get(&mut self, url: &str, limit: usize) -> Result<Vec<u8>, String> {
        if !url.starts_with("https://") {
            return Err("downloads require HTTPS".into());
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(300)))
            .build()
            .into();
        let mut last = String::new();
        for _ in 0..3 {
            match agent.get(url).call() {
                Ok(mut response) => {
                    let mut bytes = Vec::new();
                    response
                        .body_mut()
                        .as_reader()
                        .take(limit as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| e.to_string())?;
                    if bytes.len() > limit {
                        return Err("download exceeds bound".into());
                    }
                    return Ok(bytes);
                }
                Err(error) => last = error.to_string(),
            }
        }
        Err(format!("download {url}: {last}"))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub url: String,
    pub edition: Option<String>,
    pub downloaded_at: String,
    pub bytes: u64,
    pub sha256: String,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn now() -> Result<String, String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| e.to_string())
}
pub fn read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    super::terrain_srtm::bounded(path, limit).map_err(|e| format!("{}: {e}", path.display()))
}
pub fn json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&read(path, MAX_DOWNLOAD)?).map_err(|e| e.to_string())
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

pub struct Cache<T> {
    pub transport: T,
    pub directory: PathBuf,
    pub sources: Vec<Source>,
}
impl<T: Transport> Cache<T> {
    /// Discovery endpoints are always revalidated; immutable payloads can reuse a verified receipt.
    pub fn get(&mut self, url: &str, edition: Option<String>, fresh: bool) -> Result<Vec<u8>, String> {
        std::fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
        let key = sha256(url.as_bytes());
        let receipt = self.directory.join(format!("{key}.json"));
        let path = self.directory.join(&key);
        if !fresh && receipt.exists() && path.exists() {
            let source: Source = json(&receipt)?;
            let bytes = read(&path, MAX_DOWNLOAD)?;
            if source.url != url || source.bytes != bytes.len() as u64 || source.sha256 != sha256(&bytes) {
                return Err("corrupt download cache; remove the affected receipt and payload".into());
            }
            if edition.is_none() || source.edition == edition {
                self.sources.push(source);
                return Ok(bytes);
            }
        }
        let bytes = self.transport.get(url, MAX_DOWNLOAD)?;
        let source = Source {
            url: url.into(),
            edition,
            downloaded_at: now()?,
            bytes: bytes.len() as u64,
            sha256: sha256(&bytes),
        };
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        write_json(&receipt, &source)?;
        self.sources.push(source);
        Ok(bytes)
    }
    pub fn offline(&mut self, path: &Path, url: String, edition: Option<String>) -> Result<Vec<u8>, String> {
        let bytes = read(path, MAX_DOWNLOAD)?;
        self.sources.push(Source {
            url,
            edition,
            downloaded_at: now()?,
            bytes: bytes.len() as u64,
            sha256: sha256(&bytes),
        });
        Ok(bytes)
    }
}

#[cfg(test)]
pub struct Recorded(pub std::collections::BTreeMap<String, Vec<u8>>);
#[cfg(test)]
impl Transport for Recorded {
    fn get(&mut self, url: &str, limit: usize) -> Result<Vec<u8>, String> {
        let bytes = self
            .0
            .get(url)
            .ok_or_else(|| format!("unrecorded URL {url}"))?
            .clone();
        if bytes.len() > limit {
            return Err("fixture exceeds bound".into());
        }
        Ok(bytes)
    }
}
