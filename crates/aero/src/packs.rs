//! Transport-independent, bounded, transactional aircraft pack ingestion.
use crate::aircraft::{error, require};
use crate::{AircraftCatalog, AircraftError, DefinitionSource, FileKind, LayerFile, MAX_AIRCRAFT_BYTES};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_PACK_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_PACK_TEXT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PACK_FILES: usize = 64;
pub const MAX_PACKS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = pack_version_schema)]
pub struct PackManifest {
    /// Legacy manifests remain readable; exteriors require >=2, slide motion >=3, and a
    /// pre-flight walkaround section in the systems layer >=4.
    #[serde(default = "manifest_v1")]
    #[schemars(range(min = 1, max = 4))]
    pub format_version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 16))]
    pub exterior: Vec<crate::exterior::Exterior>,
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

fn pack_version_schema(schema: &mut schemars::Schema) {
    schema.insert("allOf".into(), serde_json::json!([
        {"if":{"not":{"properties":{"format_version":{"enum":[2,3,4]}},"required":["format_version"]}},
         "then":{"properties":{"exterior":{"maxItems":0}}}},
        {"if":{"not":{"properties":{"format_version":{"enum":[3,4]}},"required":["format_version"]}},
         "then":{"properties":{"exterior":{"items":{"properties":{"moving_parts":{"items":{"properties":{"motion":{"not":{"const":"slide"}}}}}}}}}}}
    ]));
}

fn manifest_v1() -> u32 {
    1
}

/// GLB paths share the portable path policy, with a different extension.
pub fn validate_asset_path(path: &str) -> Result<(), AircraftError> {
    require(path.ends_with(".glb"), "exterior.path", "requires .glb")?;
    validate_pack_path(&format!("{}.json", path.strip_suffix(".glb").unwrap_or(path)))
}

impl PackManifest {
    pub fn asset_paths(&self) -> BTreeSet<&str> {
        self.exterior.iter().flat_map(|e| e.paths()).collect()
    }
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
        require(
            (1..=4).contains(&self.format_version) && (self.exterior.is_empty() || self.format_version >= 2),
            "pack.format_version",
            "exterior assets require pack format version 2 or later",
        )?;
        require(self.exterior.len() <= 16, "pack.exterior", "at most 16 exteriors")?;
        let mut aircraft = BTreeSet::new();
        for e in &self.exterior {
            e.validate()?;
            require(
                self.format_version >= 3
                    || !e
                        .moving_parts
                        .iter()
                        .any(|p| p.motion == crate::exterior::Motion::Slide),
                "pack.format_version",
                "slide motion requires pack format version 3",
            )?;
            require(
                aircraft.insert(&e.aircraft),
                "pack.exterior",
                "duplicate aircraft exterior",
            )?;
        }
        require(
            self.files.len() + self.asset_paths().len() <= MAX_PACK_FILES,
            "pack.files",
            "at most 64 layer and asset files combined",
        )?;
        let mut paths = BTreeSet::new();
        for path in self.asset_paths() {
            require(
                paths.insert(path.to_ascii_lowercase()),
                "exterior.path",
                "ambiguous asset path",
            )?;
        }
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
        self.install_pack_with_assets(manifest, files, &BTreeMap::new())
    }

    pub fn install_pack_with_assets(
        &mut self,
        manifest: &PackManifest,
        files: &BTreeMap<String, String>,
        assets: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), AircraftError> {
        manifest.validate()?;
        require(
            assets.len() == manifest.asset_paths().len()
                && assets.keys().all(|k| manifest.asset_paths().contains(k.as_str())),
            "pack.assets",
            "missing or unlisted assets",
        )?;
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
        require(
            total <= MAX_PACK_TEXT_BYTES,
            "pack_size",
            "pack text exceeds 8 MiB",
        )?;
        for bytes in assets.values() {
            require(
                bytes.len() <= crate::exterior::MAX_MODEL_BYTES,
                "exterior.size",
                "model exceeds 64 MiB",
            )?;
            total = total.saturating_add(bytes.len());
        }
        require(total <= MAX_PACK_BYTES, "pack_size", "pack exceeds 128 MiB")?;
        let mut candidate = self.clone();
        for entry in &manifest.files {
            let text = files
                .get(&entry.path)
                .ok_or_else(|| error("files.path", format!("missing {}", entry.path)))?;
            let file = LayerFile::from_json(text)?;
            require(
                manifest.format_version >= 4
                    || !matches!(&file, LayerFile::Systems(s) if s.preflight.is_some()),
                "pack.format_version",
                "a pre-flight walkaround section requires pack format version 4",
            )?;
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
        for definition in &manifest.exterior {
            require(
                candidate.source(FileKind::Aircraft, &definition.aircraft)
                    == Some(&DefinitionSource::Pack {
                        id: manifest.id.clone(),
                    }),
                "exterior.aircraft",
                "exterior must belong to an aircraft defined by this pack",
            )?;
            let resolved = candidate.resolve_aircraft(&definition.aircraft)?;
            require(
                (definition.span_m as f64 / resolved.aircraft.geometry.span_m - 1.).abs() <= 0.02,
                "exterior.span_m",
                "exterior and family span differ by more than 2%",
            )?;
            require(
                (definition.prop_diameter_m as f64 / resolved.aircraft.propulsion.prop_diameter_m - 1.).abs()
                    <= 0.02,
                "exterior.prop_diameter_m",
                "exterior and engine propeller diameters differ by more than 2%",
            )?;
            for part in &definition.moving_parts {
                if matches!(
                    part.motion,
                    crate::exterior::Motion::Oleo
                        | crate::exterior::Motion::Wheel
                        | crate::exterior::Motion::Steering
                ) {
                    require(
                        part.gear_index < resolved.aircraft.geometry.gear.len(),
                        "exterior.gear_index",
                        "unresolved gear contact",
                    )?;
                }
            }
            require(
                resolved.aircraft.identity.registration.is_empty()
                    || definition.registration.text == resolved.aircraft.identity.registration,
                "exterior.registration",
                "painted registration differs from aircraft identity",
            )?;
            for light in &definition.lights {
                if let Some(load) = &light.power_load {
                    require(
                        resolved
                            .installed_systems
                            .definition
                            .as_ref()
                            .is_some_and(|d| d.electrical.loads.iter().any(|l| &l.id == load)),
                        "exterior.power_load",
                        "unresolved electrical load",
                    )?;
                }
            }
            if let Some(preflight) = &resolved.installed_systems.preflight {
                for part in preflight.driven_parts() {
                    require(
                        definition.moving_parts.iter().any(|m| m.node == part),
                        "preflight.items",
                        "a hand-moved control is not a moving part of the exterior",
                    )?;
                }
            }
            let mut image_cache = BTreeMap::new();
            let mut model = |path: &str| -> Result<_, AircraftError> {
                let bytes = assets
                    .get(path)
                    .ok_or_else(|| error("exterior.path", "missing asset"))?;
                crate::exterior::load_glb_with_assets(bytes, definition, path, assets, &mut image_cache)
                    .map(std::sync::Arc::new)
            };
            let asset = crate::exterior::ExteriorAsset {
                definition: definition.clone(),
                lod0: model(&definition.model)?,
                lod1: model(&definition.lod1)?,
                lod2: definition.lod2.as_deref().map(model).transpose()?,
            };
            require(
                asset.lod1.triangles < asset.lod0.triangles
                    && asset.lod1.triangles <= 25000
                    && asset
                        .lod2
                        .as_ref()
                        .is_none_or(|m| m.triangles <= 5000 && m.triangles < asset.lod1.triangles),
                "exterior.lod1",
                "LOD1 must reduce triangles",
            )?;
            candidate
                .exteriors
                .insert(definition.aircraft.clone(), std::sync::Arc::new(asset));
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
