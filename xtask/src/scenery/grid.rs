use flightcraft_world::{TerrainError, scenery::TileKey};
#[cfg(test)]
use flightcraft_world::{
    geo::LocalProjection,
    scenery::{inverse_mercator, mercator},
};
use serde::{Deserialize, Serialize};
const MAX_SCENERY_TILES: usize = 100_000;
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneryTile {
    pub height_bounds: [f64; 2],
    pub key: TileKey,
    /// One independently decodable lossless WebP for each mip, including 1×1.
    pub mips: Vec<SceneryFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneryFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageryGrid {
    pub version: u8,
    pub region: String,
    pub area: String,
    /// [west, south, east, north], EPSG:3857; square and pixel-area registered.
    pub bounds: [f64; 4],
    pub tile_pixels: u32,
    pub max_level: u8,
    /// Requested ground sampling distance at the area's center (Mercator scale corrected).
    pub ground_sample_m: f64,
    /// True for resampled reference photographs, which must never pass release validation.
    pub preview_only: bool,
    pub attribution: String,
    pub tiles: Vec<SceneryTile>,
}
impl ImageryGrid {
    pub fn validate(&self) -> Result<(), TerrainError> {
        let [w, s, e, n] = self.bounds;
        let count = (4_usize.pow(u32::from(self.max_level.min(12)) + 1) - 1) / 3;
        if self.version != 1
            || !flightcraft_world::region_id(&self.region)
            || !flightcraft_world::region_id(&self.area)
            || self
                .bounds
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 20_037_509.)
            || w >= e
            || s >= n
            || ((e - w) - (n - s)).abs() > 0.00001
            || e - w > 250_000.
            || !matches!(self.tile_pixels, 256 | 512 | 1024)
            || self.max_level > 8
            || count > MAX_SCENERY_TILES
            || self.tiles.len() != count
            || !self.ground_sample_m.is_finite()
            || !(0.1..=10.).contains(&self.ground_sample_m)
            || self.attribution.is_empty()
            || self.attribution.len() > 2048
        {
            return Err(TerrainError("invalid scenery bounds or budgets"));
        }
        let mut keys = std::collections::BTreeSet::new();
        for tile in &self.tiles {
            if !tile.key.valid()
                || tile.key.level > self.max_level
                || !keys.insert(tile.key)
                || tile.mips.len() != 1
            {
                return Err(TerrainError("incomplete or duplicate scenery quadtree"));
            }
            for (mip, file) in tile.mips.iter().enumerate() {
                if file.path != format!("imagery/{}/m{mip}.webp", tile.key.name())
                    || file.bytes == 0
                    || file.bytes > 8 * 1024 * 1024
                    || file.sha256.len() != 64
                    || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(TerrainError("invalid scenery tile receipt or path"));
                }
            }
        }
        Ok(())
    }
    pub fn tile_bounds(&self, key: TileKey) -> Result<[f64; 4], TerrainError> {
        if !key.valid() || key.level > self.max_level {
            return Err(TerrainError("invalid imagery tile key"));
        }
        let [w, _, e, n] = self.bounds;
        let size = (e - w) / f64::from(1_u32 << key.level);
        let west = w + f64::from(key.x) * size;
        let north = n - f64::from(key.y) * size;
        Ok([west, north - size, west + size, north])
    }
    /// Pixel-edge coordinates; centers have half-integer coordinates. No half-pixel shift.
    #[cfg(test)]
    pub fn pixel(&self, key: TileKey, latitude: f64, longitude: f64) -> Result<[f64; 2], TerrainError> {
        let [x, y] = mercator(latitude, longitude)?;
        let [w, s, e, n] = self.tile_bounds(key)?;
        Ok([
            (x - w) / (e - w) * f64::from(self.tile_pixels),
            (n - y) / (n - s) * f64::from(self.tile_pixels),
        ])
    }
    #[cfg(test)]
    pub fn local_at_uv(
        &self,
        key: TileKey,
        uv: [f64; 2],
        projection: &LocalProjection,
    ) -> Result<[f64; 2], TerrainError> {
        if uv.iter().any(|v| !v.is_finite() || !(0.0..=1.).contains(v)) {
            return Err(TerrainError("invalid imagery UV"));
        }
        let [w, s, e, n] = self.tile_bounds(key)?;
        let [u, v] = uv;
        let [lat, lon] = inverse_mercator(w + u * (e - w), n - v * (n - s))?;
        projection.project(lat, lon)
    }
}
