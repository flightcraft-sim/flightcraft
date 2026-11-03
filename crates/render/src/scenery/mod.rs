//! One scenery content path: OGC 3D Tiles 1.1, glTF 2.0 GLB, and geometric-error LOD.
mod glb;
pub(crate) mod gpu;
mod provider;
mod stream;
pub use glb::{DecodedTile, decode_glb};
pub use provider::{CachePolicy, Content, LocalProvider, TileProvider};
use serde::{Deserialize, Serialize};
pub use stream::{ContentRequest, Selection, Stream, StreamStats};

/// Persist in the user's preferences, never in region data. Token values are deliberately
/// absent from this descriptor: future online adapters resolve this config key privately.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub location: Location,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_config_key: Option<String>,
    /// Online providers must explicitly opt in. The local adapter always rejects URLs.
    #[serde(default)]
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum Location {
    LocalPath(std::path::PathBuf),
    Url(String),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    #[default]
    Medium,
    High,
}
impl Quality {
    pub fn screen_error_px(self) -> f64 {
        match self {
            Self::Low => 24.,
            Self::Medium => 12.,
            Self::High => 6.,
        }
    }
    pub fn resident_bytes(self) -> usize {
        match self {
            Self::Low => 48 * 1024 * 1024,
            Self::Medium => 96 * 1024 * 1024,
            Self::High => 192 * 1024 * 1024,
        }
    }
    pub fn uploads_per_frame(self) -> usize {
        match self {
            Self::Low => 1,
            Self::Medium | Self::High => 2,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}
