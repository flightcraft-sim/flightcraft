//! One scenery content path: OGC 3D Tiles 1.1, glTF 2.0 GLB, and geometric-error LOD.
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod esri;
#[cfg(target_arch = "wasm32")]
pub use browser::BrowserProvider;
mod glb;
mod surface;
#[cfg(not(target_arch = "wasm32"))]
pub use esri::EsriProvider;
pub use surface::{pack_glb, surface_glb};
pub(crate) mod gpu;
mod instances;
mod nyc;
mod provider;
pub use nyc::{
    FlyNycArea, NYC_ATTRIBUTION, NYC_BOUNDS, NYC_BUILDING_BUDGET, NYC_EPT, NycLidarReceipt, NycLidarSource,
    WEB_BUDGET, fly_nyc_area, nyc_lidar_source, validate_redistribution, validate_static_scenery,
};
mod stream;
pub use glb::{DecodedTile, decode_glb, decode_glb_with_geoid};
pub use provider::{CachePolicy, Content, LocalProvider, TileProvider};
use serde::{Deserialize, Serialize};
pub use stream::{ContentRequest, Selection, Stream, StreamStats};

/// Persist in the user's preferences, never in region data. Token values are deliberately
/// absent from this descriptor: online adapters resolve this config key privately.
/// Fly NYC reconstruction uses this descriptor plus OGC tileset extras conforming to
/// schema/scenery-nyc-lidar.json. NYC portal footprints and TUM remain local-only.
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
            Self::Medium => 160 * 1024 * 1024,
            Self::High => 256 * 1024 * 1024,
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

#[cfg(not(target_arch = "wasm32"))]
fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(u64::MAX)
}

#[cfg(target_arch = "wasm32")]
fn unix_ms() -> u64 {
    let time = js_sys::Date::now();
    if time.is_finite() && time >= 0. {
        time as u64
    } else {
        u64::MAX
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Instance {
    pub matrix: [[f32; 4]; 4],
}
#[derive(Clone)]
pub(crate) struct InstanceBatch {
    pub vertices: Vec<Vertex>,
    pub instances: Vec<Instance>,
}
