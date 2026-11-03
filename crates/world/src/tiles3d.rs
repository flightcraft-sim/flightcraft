//! OGC 3D Tiles 1.1 explicit trees. No provider credentials or network I/O live in the world.
use crate::TerrainError;
use serde::{Deserialize, Serialize};

pub const MAX_TILESET_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TILE_CONTENT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_TILESET_TILES: usize = 100_000;
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Tileset {
    pub asset: Asset,
    pub geometric_error: f64,
    pub root: Tile,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions_required: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extras: Option<serde_json::Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Asset {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    pub bounding_volume: BoundingVolume,
    pub geometric_error: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine: Option<Refine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<[f64; 16]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Tile>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Refine {
    Add,
    Replace,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Content {
    pub uri: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BoundingVolume {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<[f64; 6]>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "box")]
    pub box_volume: Option<[f64; 12]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sphere: Option<[f64; 4]>,
}
impl BoundingVolume {
    pub fn region(region: [f64; 6]) -> Self {
        Self {
            region: Some(region),
            box_volume: None,
            sphere: None,
        }
    }
    fn valid(&self) -> bool {
        if usize::from(self.region.is_some())
            + usize::from(self.box_volume.is_some())
            + usize::from(self.sphere.is_some())
            != 1
        {
            return false;
        }
        if let Some([w, s, e, n, lo, hi]) = self.region {
            return [w, s, e, n, lo, hi].iter().all(|v| v.is_finite())
                && w >= -core::f64::consts::PI
                && e <= core::f64::consts::PI
                && w < e
                && s >= -core::f64::consts::FRAC_PI_2
                && n <= core::f64::consts::FRAC_PI_2
                && s < n
                && lo >= -20_000.
                && hi <= 100_000.
                && lo <= hi;
        }
        if let Some(b) = self.box_volume {
            return b.iter().all(|v| v.is_finite() && v.abs() < 100_000_000.);
        }
        self.sphere.is_some_and(|s| {
            s.iter().all(|v| v.is_finite() && v.abs() < 100_000_000.) && s.get(3).is_some_and(|r| *r > 0.)
        })
    }
}
pub fn local_uri(uri: &str) -> bool {
    !uri.is_empty()
        && uri.len() < 512
        && !uri.contains(['\\', ':', '?', '#', '%'])
        && !uri.starts_with('/')
        && uri.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}
impl Tileset {
    pub fn from_json(bytes: &[u8]) -> Result<Self, TerrainError> {
        if bytes.len() > MAX_TILESET_BYTES {
            return Err(TerrainError("3D Tiles tileset exceeds size limit"));
        }
        let set: Self = serde_json::from_slice(bytes).map_err(|_| TerrainError("invalid 3D Tiles JSON"))?;
        set.validate()?;
        Ok(set)
    }
    pub fn validate(&self) -> Result<(), TerrainError> {
        if self.asset.version != "1.1"
            || !self.extensions_required.is_empty()
            || !self.geometric_error.is_finite()
            || self.geometric_error < 0.
            || self.geometric_error > 1_000_000.
            || self.asset.copyright.as_ref().is_some_and(|s| s.len() > 8192)
        {
            return Err(TerrainError("unsupported 3D Tiles asset or extensions"));
        }
        let mut pending = vec![(&self.root, self.geometric_error, 0)];
        let mut count = 0;
        while let Some((tile, parent_error, depth)) = pending.pop() {
            count += 1;
            if count > MAX_TILESET_TILES
                || depth > 24
                || !tile.bounding_volume.valid()
                || !tile.geometric_error.is_finite()
                || tile.geometric_error < 0.
                || tile.geometric_error > parent_error
                || tile.children.len() > 64
                || tile
                    .content
                    .as_ref()
                    .is_some_and(|c| !local_uri(&c.uri) || !c.uri.ends_with(".glb"))
                || tile.transform.is_some_and(|t| {
                    t.iter().any(|v| !v.is_finite() || v.abs() > 100_000_000.)
                        || t.get(3) != Some(&0.)
                        || t.get(7) != Some(&0.)
                        || t.get(11) != Some(&0.)
                        || t.get(15) != Some(&1.)
                })
            {
                return Err(TerrainError(
                    "invalid 3D Tiles bounds, transform, content, or tree budget",
                ));
            }
            if tile.content.is_none() && tile.children.is_empty() {
                return Err(TerrainError("empty 3D Tiles leaf"));
            }
            pending.extend(tile.children.iter().map(|c| (c, tile.geometric_error, depth + 1)));
        }
        Ok(())
    }
}
/// WGS84 ellipsoidal height. A DEM's orthometric height needs a geoid correction before use.
pub fn ecef(lat: f64, lon: f64, height: f64) -> Result<[f64; 3], TerrainError> {
    if !lat.is_finite()
        || !lon.is_finite()
        || !height.is_finite()
        || lat.abs() > 90.
        || lon.abs() > 180.
        || !(-20_000.0..=100_000.).contains(&height)
    {
        return Err(TerrainError("invalid ECEF input"));
    }
    let (s, c) = libm::sincos(lat.to_radians());
    let (sl, cl) = libm::sincos(lon.to_radians());
    let e2 = 6.694_379_990_141_316_5e-3;
    let radius = 6_378_137. / libm::sqrt(1. - e2 * s * s);
    Ok([
        (radius + height) * c * cl,
        (radius + height) * c * sl,
        (radius * (1. - e2) + height) * s,
    ])
}
/// Column-major local east/north/up to ECEF (3D Tiles applies glTF's Y-up conversion first).
pub fn enu_transform(lat: f64, lon: f64, height: f64) -> Result<[f64; 16], TerrainError> {
    let [x, y, z] = ecef(lat, lon, height)?;
    let (s, c) = libm::sincos(lat.to_radians());
    let (sl, cl) = libm::sincos(lon.to_radians());
    Ok([
        -sl,
        cl,
        0.,
        0.,
        -s * cl,
        -s * sl,
        c,
        0.,
        c * cl,
        c * sl,
        s,
        0.,
        x,
        y,
        z,
        1.,
    ])
}

/// Inverse WGS84; returns latitude, longitude, and ellipsoidal height.
pub fn geodetic([x, y, z]: [f64; 3]) -> Result<[f64; 3], TerrainError> {
    if [x, y, z].iter().any(|v| !v.is_finite()) {
        return Err(TerrainError("invalid ECEF coordinate"));
    }
    let p = libm::hypot(x, y);
    if !(1_000_000.0..=100_000_000.).contains(&libm::hypot(p, z)) {
        return Err(TerrainError("ECEF coordinate outside Earth bounds"));
    }
    let e2 = 6.694_379_990_141_316_5e-3;
    let mut lat = libm::atan2(z, p * (1. - e2));
    let mut height = 0.;
    for _ in 0..8 {
        let s = libm::sin(lat);
        let c = libm::cos(lat);
        let radius = 6_378_137. / libm::sqrt(1. - e2 * s * s);
        height = if c.abs() > 1e-10 {
            p / c - radius
        } else {
            z.abs() - radius * (1. - e2)
        };
        lat = libm::atan2(z, p * (1. - e2 * radius / (radius + height)));
    }
    Ok([lat.to_degrees(), libm::atan2(y, x).to_degrees(), height])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_tilesets() {
        for uri in [
            "../x.glb",
            "/x.glb",
            "https://evil/x.glb",
            "x%2f.glb",
            "x.glb?token=secret",
            "a\\x.glb",
        ] {
            assert!(!local_uri(uri));
        }
        assert!(local_uri("tiles/3/1/2.glb"));
        let valid=br#"{"asset":{"version":"1.1"},"geometricError":10,"root":{"boundingVolume":{"region":[-1,0,-0.9,0.1,0,100]},"geometricError":0,"content":{"uri":"tile.glb"}}}"#;
        let set = Tileset::from_json(valid).unwrap();
        let mut bad = set.clone();
        bad.root.geometric_error = f64::NAN;
        assert!(bad.validate().is_err());
        let mut bad = set.clone();
        bad.root.bounding_volume.sphere = Some([0., 0., 0., 1.]);
        assert!(bad.validate().is_err());
        let mut bad = set;
        bad.asset.version = "1.0".into();
        assert!(bad.validate().is_err());
    }
}
