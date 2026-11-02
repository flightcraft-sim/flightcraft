//! Validated portable terrain region manifest, shared by offline and browser loaders.
use crate::{Runway, TerrainError};
use serde::{Deserialize, Serialize};

pub const SRTM_ATTRIBUTION: &str =
    "NASA Shuttle Radar Topography Mission, public domain; distributed by USGS";
pub const MAX_REGION_BYTES: usize = 64 * 1024;
pub const MAX_REGION_TILES: usize = 256;
pub const MAX_REGION_TILE_BYTES: usize = 32 + 257 * 257 * 4 + 256 * 256;
pub const MAX_REGIONS: usize = 64;
/// Combined installed region tile budget, shared by native and browser transports.
pub const MAX_TERRAIN_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegionOrigin {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    /// SRTM and the simulator use MSL heights, not ellipsoidal heights.
    pub field_elevation_m: f64,
    pub runway_heading_deg: f64,
    pub runway_length_m: f64,
    pub runway_width_m: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub id: String,
    pub name: String,
    pub origin: RegionOrigin,
    /// Southwest local corner of the output grid, north/east metres.
    pub southwest_m: [f64; 2],
    pub tiles_north: u32,
    pub tiles_east: u32,
    pub spacing_m: f32,
    pub tile_size: u32,
    pub runway_blend_m: f64,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub licence: String,
    #[serde(default)]
    pub attribution: String,
    #[serde(default)]
    pub tiles: Vec<String>,
    /// Optional adjacent airport catalogue; only the canonical basename is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub airports: Option<String>,
}

pub fn region_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !matches!(id, "flat" | "training-valley")
}

impl Region {
    pub fn from_json(text: &str) -> Result<Self, TerrainError> {
        if text.len() > MAX_REGION_BYTES {
            return Err(TerrainError("region manifest too large"));
        }
        let region: Self = serde_json::from_str(text).map_err(|_| TerrainError("invalid region JSON"))?;
        region.validate()?;
        Ok(region)
    }

    pub fn validate(&self) -> Result<(), TerrainError> {
        let o = &self.origin;
        let extent = f64::from(self.tile_size.saturating_sub(1)) * f64::from(self.spacing_m);
        if !region_id(&self.id)
            || self.name.is_empty()
            || self.name.len() > 256
            || !o.latitude_deg.is_finite()
            || o.latitude_deg.abs() > 85.0
            || !o.longitude_deg.is_finite()
            || !(-180.0..180.0).contains(&o.longitude_deg)
            || !o.field_elevation_m.is_finite()
            || !(-500.0..=9000.0).contains(&o.field_elevation_m)
            || !o.runway_heading_deg.is_finite()
            || !(0.0..=360.0).contains(&o.runway_heading_deg)
            || !o.runway_length_m.is_finite()
            || !(50.0..=10000.0).contains(&o.runway_length_m)
            || !o.runway_width_m.is_finite()
            || !(1.0..=200.0).contains(&o.runway_width_m)
            || !self.runway_blend_m.is_finite()
            || !(1.0..=5000.0).contains(&self.runway_blend_m)
            || !(2..=257).contains(&self.tile_size)
            || !self.spacing_m.is_finite()
            || !(0.1..=10000.0).contains(&self.spacing_m)
            || self.tiles_north == 0
            || self.tiles_east == 0
            || u64::from(self.tiles_north) * u64::from(self.tiles_east) > MAX_REGION_TILES as u64
            || self
                .southwest_m
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 250000.0)
            || self.southwest_m[0] + f64::from(self.tiles_north) * extent > 250000.0
            || self.southwest_m[1] + f64::from(self.tiles_east) * extent > 250000.0
            || [&self.source, &self.licence, &self.attribution]
                .iter()
                .any(|s| s.len() > 2048)
            || self.airports.as_deref().is_some_and(|s| s != "airports.json")
            || self.tiles.len() > MAX_REGION_TILES
            || self.tiles.iter().any(|s| !tile_name(s))
        {
            return Err(TerrainError("region fields outside supported bounds"));
        }
        let [sn, se] = self.southwest_m;
        let (s, c) = libm::sincos(o.runway_heading_deg.to_radians());
        let pad = o.runway_width_m * 0.5 + self.runway_blend_m + f64::from(self.spacing_m) * 2.0;
        let end_n = o.runway_length_m * c;
        let end_e = o.runway_length_m * s;
        if sn > end_n.min(0.0) - pad
            || se > end_e.min(0.0) - pad
            || sn + f64::from(self.tiles_north) * extent < end_n.max(0.0) + pad
            || se + f64::from(self.tiles_east) * extent < end_e.max(0.0) + pad
        {
            return Err(TerrainError("grid must enclose runway and blend margin"));
        }
        Ok(())
    }

    /// Encoded tile size, saturating for unvalidated callers.
    pub fn tile_bytes(&self) -> usize {
        let width = self.tile_size as usize;
        let one = 32_usize
            .saturating_add(width.saturating_mul(width).saturating_mul(4))
            .saturating_add(width.saturating_sub(1).saturating_mul(width.saturating_sub(1)));
        one.saturating_mul(self.tiles_north as usize)
            .saturating_mul(self.tiles_east as usize)
    }

    pub fn validate_manifest(&self) -> Result<(), TerrainError> {
        self.validate()?;
        if self.tiles.len() != self.tiles_north as usize * self.tiles_east as usize
            || self.source.is_empty()
            || self.licence.is_empty()
            || self.attribution.is_empty()
            || self
                .tiles
                .iter()
                .enumerate()
                .any(|(i, s)| s != &format!("{i:03}.fcht"))
        {
            return Err(TerrainError("incomplete region manifest or attribution"));
        }
        Ok(())
    }

    #[must_use]
    pub fn runway(&self) -> Runway {
        Runway {
            length_m: self.origin.runway_length_m,
            width_m: self.origin.runway_width_m,
            heading_deg: self.origin.runway_heading_deg,
            threshold_ned_m: [0.0, 0.0, -self.origin.field_elevation_m],
        }
    }
}

fn tile_name(s: &str) -> bool {
    s.len() <= 16
        && s.strip_suffix(".fcht")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}
