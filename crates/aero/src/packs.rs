//! Transport-independent, bounded, transactional aircraft pack ingestion.
use crate::aircraft::{error, require};
use crate::{AircraftCatalog, AircraftError, DefinitionSource, FileKind, LayerFile, MAX_AIRCRAFT_BYTES};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_PACK_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PACK_FILES: usize = 64;
pub const MAX_PACKS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub licence: String,
    #[schemars(length(min = 1, max = 32))]
    pub authors: Vec<String>,
    pub affiliation: String,
    #[schemars(length(min = 1, max = 64))]
    pub files: Vec<PackFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    pub path: String,
    pub kind: FileKind,
}

/// Portable relative paths only: no traversal, URL syntax, hidden entries, or platform aliases.
pub fn validate_pack_path(path: &str) -> Result<(), AircraftError> {
    require(
        !path.is_empty()
            && path.len() <= 256
            && path.split('/').all(|part| {
                !part.is_empty()
                    && !part.starts_with('.')
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
            }),
        "files.path",
        "requires a portable relative path without traversal",
    )?;
    require(
        path != "pack.json" && path.ends_with(".json"),
        "files.path",
        "requires a JSON layer file other than pack.json",
    )
}

impl PackManifest {
    pub fn from_json(text: &str) -> Result<Self, AircraftError> {
        require(
            text.len() <= MAX_MANIFEST_BYTES,
            "pack.json",
            "manifest exceeds 64 KiB",
        )?;
        let manifest: Self = serde_json::from_str(text).map_err(|e| error("pack.json", e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), AircraftError> {
        require(
            serde_json::to_vec(self)
                .map_err(|e| error("pack.json", e.to_string()))?
                .len()
                <= MAX_MANIFEST_BYTES,
            "pack.json",
            "manifest exceeds 64 KiB",
        )?;
        require(
            !self.id.is_empty()
                && self.id.len() <= 128
                && self
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'),
            "pack.id",
            "invalid pack id",
        )?;
        for (field, text) in [
            ("name", &self.name),
            ("version", &self.version),
            ("licence", &self.licence),
            ("affiliation", &self.affiliation),
        ] {
            require(
                !text.trim().is_empty() && text.len() <= 4096,
                &format!("pack.{field}"),
                "requires 1 to 4096 bytes of text",
            )?;
        }
        require(
            (1..=32).contains(&self.authors.len())
                && self
                    .authors
                    .iter()
                    .all(|a| !a.trim().is_empty() && a.len() <= 4096),
            "pack.authors",
            "requires 1 to 32 nonempty authors",
        )?;
        require(
            (1..=MAX_PACK_FILES).contains(&self.files.len()),
            "pack.files",
            "requires 1 to 64 files",
        )?;
        let mut paths = BTreeSet::new();
        for file in &self.files {
            validate_pack_path(&file.path)?;
            // Case-insensitive duplicates are ambiguous across native file systems.
            require(
                paths.insert(file.path.to_ascii_lowercase()),
                "files.path",
                "duplicate file path",
            )?;
        }
        Ok(())
    }
}

impl AircraftCatalog {
    /// Callers fetch only manifest-listed JSON files. Failure leaves the entire catalogue unchanged.
    pub fn install_pack(
        &mut self,
        manifest: &PackManifest,
        files: &BTreeMap<String, String>,
    ) -> Result<(), AircraftError> {
        manifest.validate()?;
        require(
            files.len() == manifest.files.len(),
            "pack.files",
            "missing or unlisted files",
        )?;
        require(
            !self.has_pack(&manifest.id),
            "pack.id",
            "pack is already installed",
        )?;
        require(self.pack_count() < MAX_PACKS, "pack.id", "at most 64 packs")?;
        let mut total = serde_json::to_vec(manifest)
            .map_err(|e| error("pack.json", e.to_string()))?
            .len();
        for text in files.values() {
            require(
                text.len() <= MAX_AIRCRAFT_BYTES,
                "file_size",
                "layer exceeds 1 MiB",
            )?;
            total = total.saturating_add(text.len());
        }
        require(total <= MAX_PACK_BYTES, "pack_size", "pack exceeds 8 MiB")?;
        let mut candidate = self.clone();
        for entry in &manifest.files {
            let text = files
                .get(&entry.path)
                .ok_or_else(|| error("files.path", format!("missing {}", entry.path)))?;
            let file = LayerFile::from_json(text)?;
            require(
                file.kind() == entry.kind,
                "files.kind",
                "manifest and document kinds differ",
            )?;
            candidate.insert(
                file,
                DefinitionSource::Pack {
                    id: manifest.id.clone(),
                },
            )?;
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    fn has_pack(&self, id: &str) -> bool {
        self.pack_ids().contains(id)
    }
    fn pack_count(&self) -> usize {
        self.pack_ids().len()
    }
}
