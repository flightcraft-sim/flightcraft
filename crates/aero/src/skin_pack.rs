//! Skin packs: artwork for one display model, shipped apart from the engine and from aircraft packs.
//!
//! A skin pack changes how a display looks (bezel colours, key and knob faces, short control
//! labels, a chrome font). It carries no behaviour: pages, knobs, menus, and failure logic belong
//! to the display model. An aircraft pack may recommend a skin by id; if that skin is not installed
//! the built-in public skin is used.
use crate::display_model::{BezelControl, DisplayModel};
use crate::panel_fidelity::SrgbHex;
use crate::{AircraftError, aircraft::require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_SKIN_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_SKIN_FONT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SKIN_ASSETS: usize = 16;
pub const MAX_SKIN_PACK_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SKINS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkinAssetKind {
    Font,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkinAsset {
    /// Portable relative path inside the skin directory.
    pub path: String,
    pub kind: SkinAssetKind,
    /// SPDX licence identifier of the asset itself.
    pub licence: String,
}

/// Colours for the bezel and its controls. Every field is optional; omission keeps the built-in value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SkinPalette {
    pub bezel: Option<SrgbHex>,
    pub bezel_trim: Option<SrgbHex>,
    pub key_face: Option<SrgbHex>,
    pub key_text: Option<SrgbHex>,
    pub knob: Option<SrgbHex>,
    pub knob_mark: Option<SrgbHex>,
    pub screen_surround: Option<SrgbHex>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkinPackManifest {
    pub kind: String,
    #[schemars(range(min = 1, max = 1))]
    pub format_version: u32,
    pub id: String,
    pub version: String,
    pub display_name: String,
    /// SPDX licence identifier of the skin pack.
    pub licence: String,
    /// The display model this skin dresses.
    pub display_model: DisplayModel,
    #[serde(default)]
    pub palette: SkinPalette,
    /// Short functional labels printed on keys, by control. At most 8 characters each.
    #[serde(default)]
    pub labels: BTreeMap<BezelControl, String>,
    /// Path of the chrome font among `assets`.
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    #[schemars(length(max = 16))]
    pub assets: Vec<SkinAsset>,
}

/// A path made only of lowercase ASCII letters, digits, `-`, `_`, `.`; no hidden or empty parts.
pub fn portable_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 128
        && path.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.')
        })
}

impl SkinPackManifest {
    pub fn from_json(text: &str) -> Result<Self, AircraftError> {
        require(
            text.len() <= MAX_SKIN_MANIFEST_BYTES,
            "skin.size",
            "manifest exceeds 64 KiB",
        )?;
        let manifest: Self =
            serde_json::from_str(text).map_err(|e| crate::aircraft::error("skin", e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), AircraftError> {
        require(self.kind == "skin_pack", "skin.kind", "requires kind skin_pack")?;
        require(self.format_version == 1, "skin.format_version", "requires 1")?;
        crate::layers::id(&self.id, "skin.id")?;
        let text_ok =
            |s: &str, max: usize| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control);
        require(
            text_ok(&self.version, 32),
            "skin.version",
            "requires 1 to 32 printable characters",
        )?;
        require(
            text_ok(&self.display_name, 80),
            "skin.display_name",
            "requires 1 to 80 printable characters",
        )?;
        require(
            text_ok(&self.licence, 64),
            "skin.licence",
            "an SPDX licence identifier is required",
        )?;
        let p = &self.palette;
        for c in [
            &p.bezel,
            &p.bezel_trim,
            &p.key_face,
            &p.key_text,
            &p.knob,
            &p.knob_mark,
            &p.screen_surround,
        ]
        .into_iter()
        .flatten()
        {
            c.validate()?;
        }
        if let (Some(face), Some(text)) = (&p.key_face, &p.key_text) {
            require(
                text.contrast(face).is_some_and(|v| v >= 4.5),
                "skin.palette",
                "key text requires contrast of at least 4.5:1 against the key face",
            )?;
        }
        require(self.labels.len() <= 8, "skin.labels", "at most 8 labels")?;
        for (control, label) in &self.labels {
            require(
                !control.is_knob() && control.label() != "CARD",
                "skin.labels",
                "only labelled keys may be renamed",
            )?;
            require(
                text_ok(label, 8) && label.is_ascii(),
                "skin.labels",
                "labels are 1 to 8 printable ASCII characters",
            )?;
        }
        require(
            self.assets.len() <= MAX_SKIN_ASSETS,
            "skin.assets",
            "at most 16 assets",
        )?;
        let mut seen = std::collections::BTreeSet::new();
        for a in &self.assets {
            require(
                portable_path(&a.path) && seen.insert(&a.path),
                "skin.assets",
                "paths must be unique, portable relative paths",
            )?;
            require(
                text_ok(&a.licence, 64),
                "skin.assets",
                "each asset names its SPDX licence",
            )?;
        }
        if let Some(font) = &self.font {
            require(
                self.assets
                    .iter()
                    .any(|a| &a.path == font && a.kind == SkinAssetKind::Font),
                "skin.font",
                "font must be a declared font asset",
            )?;
        }
        Ok(())
    }

    /// Label for a key: the skin's own, else the model default.
    pub fn label(&self, control: BezelControl) -> &str {
        self.labels
            .get(&control)
            .map_or_else(|| control.label(), String::as_str)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> serde_json::Value {
        json!({"kind":"skin_pack","format_version":1,"id":"landscape-touch-efis","version":"0.1.0",
            "display_name":"Landscape touch EFIS","licence":"Apache-2.0","display_model":"landscape_touch_efis",
            "palette":{"bezel":"#101010","key_face":"#202020","key_text":"#f0f0f0"},
            "labels":{"nearest_key":"NRST"},
            "font":"fonts/a.ttf","assets":[{"path":"fonts/a.ttf","kind":"font","licence":"OFL-1.1"}]})
    }
    fn parse(v: &serde_json::Value) -> Result<SkinPackManifest, AircraftError> {
        SkinPackManifest::from_json(&v.to_string())
    }

    #[test]
    fn a_complete_manifest_loads_and_labels_fall_back_to_the_model() {
        let m = parse(&base()).unwrap();
        assert_eq!(m.display_model, DisplayModel::LandscapeTouchEfis);
        assert_eq!(m.label(BezelControl::NearestKey), "NRST");
        assert_eq!(m.label(BezelControl::MenuKey), "MENU");
    }

    #[test]
    fn hostile_manifests_are_rejected() {
        let bad = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut v = base();
            f(&mut v);
            assert!(parse(&v).is_err(), "{v}");
        };
        bad(&|v| v["kind"] = json!("pack"));
        bad(&|v| v["id"] = json!("Bad Id"));
        bad(&|v| v["display_model"] = json!("made_up"));
        bad(&|v| v["palette"]["bezel"] = json!("red"));
        bad(&|v| v["palette"]["key_text"] = json!("#212121"));
        bad(&|v| v["labels"]["left_knob"] = json!("X"));
        bad(&|v| v["labels"]["nearest_key"] = json!("TOO LONG LABEL"));
        bad(&|v| v["assets"][0]["path"] = json!("../etc/passwd"));
        bad(&|v| v["assets"][0]["path"] = json!("/abs.ttf"));
        bad(&|v| v["font"] = json!("fonts/other.ttf"));
        bad(&|v| v["unknown"] = json!(1));
        bad(&|v| v["licence"] = json!(""));
        let huge = format!("{{\"pad\":\"{}\"}}", "x".repeat(MAX_SKIN_MANIFEST_BYTES));
        assert!(SkinPackManifest::from_json(&huge).is_err());
    }

    #[test]
    fn portable_paths_exclude_escapes() {
        assert!(portable_path("fonts/a.ttf"));
        for p in ["", "a//b", ".hidden", "a/../b", "a\\b", "A b", "/x"] {
            assert!(!portable_path(p), "{p:?}");
        }
    }
}
