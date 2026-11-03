//! SRTM decoding, WGS84 transverse Mercator, and deterministic resampling.
use crate::{HeightTile, Region, SRTM_ATTRIBUTION, Surface, TerrainError};

pub const MAX_HGT_BYTES: usize = 3601 * 3601 * 2;
pub const MAX_HGT_TOTAL_BYTES: usize = 256 * 1024 * 1024;
const VOID: i16 = i16::MIN;
pub use crate::geo::LocalProjection;

#[derive(Debug)]
pub struct HgtTile {
    south: i32,
    west: i32,
    width: usize,
    samples: Vec<i16>,
}
impl HgtTile {
    /// Canonical basename only. Rows run north to south; columns run west to east.
    pub fn read(name: &str, bytes: &[u8]) -> Result<Self, TerrainError> {
        let stem = name
            .strip_suffix(".hgt")
            .ok_or(TerrainError("expected .hgt filename"))?;
        let b = stem.as_bytes();
        if b.len() != 7
            || !matches!(b.first(), Some(b'N' | b'S'))
            || !matches!(b.get(3), Some(b'E' | b'W'))
            || !b.get(1..3).is_some_and(|x| x.iter().all(u8::is_ascii_digit))
            || !b.get(4..7).is_some_and(|x| x.iter().all(u8::is_ascii_digit))
        {
            return Err(TerrainError("invalid HGT basename"));
        }
        let lat: i32 = stem
            .get(1..3)
            .ok_or(TerrainError("latitude"))?
            .parse()
            .map_err(|_| TerrainError("latitude"))?;
        let lon: i32 = stem
            .get(4..7)
            .ok_or(TerrainError("longitude"))?
            .parse()
            .map_err(|_| TerrainError("longitude"))?;
        let south = if b.first() == Some(&b'S') { -lat } else { lat };
        let west = if b.get(3) == Some(&b'W') { -lon } else { lon };
        if !(-90..90).contains(&south)
            || !(-180..180).contains(&west)
            || (lat == 0 && b.first() == Some(&b'S'))
            || (lon == 0 && b.get(3) == Some(&b'W'))
        {
            return Err(TerrainError("HGT corner outside supported bounds"));
        }
        let width = match bytes.len() {
            n if n == 1201 * 1201 * 2 => 1201,
            MAX_HGT_BYTES => 3601,
            _ => return Err(TerrainError("HGT must be 1201² or 3601² big-endian i16 samples")),
        };
        let samples = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(i16::from_be_bytes)
            .collect();
        Ok(Self {
            south,
            west,
            width,
            samples,
        })
    }
    pub fn corner(&self) -> [i32; 2] {
        [self.south, self.west]
    }
    pub fn byte_len(&self) -> usize {
        self.samples.len() * 2
    }
    fn point(&self, lat: f64, lon: f64) -> Option<i16> {
        let lon = (lon - f64::from(self.west) + 180.0).rem_euclid(360.0) - 180.0;
        let row = (f64::from(self.south + 1) - lat) * (self.width - 1) as f64;
        let col = lon * (self.width - 1) as f64;
        if row < -1e-6
            || col < -1e-6
            || row > (self.width - 1) as f64 + 1e-6
            || col > (self.width - 1) as f64 + 1e-6
        {
            return None;
        }
        self.samples
            .get(libm::round(row).max(0.0) as usize * self.width + libm::round(col).max(0.0) as usize)
            .copied()
            .filter(|v| *v != VOID)
    }
}

#[derive(Default, Debug)]
pub struct Srtm {
    tiles: Vec<HgtTile>,
    bytes: usize,
}
impl Srtm {
    pub fn insert(&mut self, tile: HgtTile) -> Result<(), TerrainError> {
        if self.tiles.len() >= 64
            || self.bytes + tile.byte_len() > MAX_HGT_TOTAL_BYTES
            || self.tiles.iter().any(|t| t.corner() == tile.corner())
        {
            return Err(TerrainError("duplicate HGT corner or source budget exceeded"));
        }
        self.bytes += tile.byte_len();
        self.tiles.push(tile);
        self.tiles.sort_by_key(HgtTile::corner);
        Ok(())
    }
    fn valid_point(&self, lat: f64, lon: f64) -> Option<f64> {
        self.tiles.iter().find_map(|t| t.point(lat, lon)).map(f64::from)
    }
    fn filled(&self, lat: f64, lon: f64, step: f64) -> Result<f64, TerrainError> {
        if let Some(h) = self.valid_point(lat, lon) {
            return Ok(h);
        }
        // Nearest Chebyshev ring, averaged symmetrically. Search crosses HGT boundaries.
        // A bounded search prevents large all-void files from causing unbounded work.
        for radius in 1_i32..=32 {
            let mut total = 0.0;
            let mut count = 0;
            for j in -radius..=radius {
                for i in -radius..=radius {
                    if i.abs().max(j.abs()) != radius {
                        continue;
                    }
                    if let Some(h) = self.valid_point(lat + f64::from(j) * step, lon + f64::from(i) * step) {
                        total += h;
                        count += 1;
                    }
                }
            }
            if count > 0 {
                return Ok(total / f64::from(count));
            }
        }
        Err(TerrainError(
            "void has no valid neighbour within 32 source samples",
        ))
    }
    pub fn height(&self, lat: f64, lon: f64) -> Result<f64, TerrainError> {
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
            return Err(TerrainError("invalid sample coordinate"));
        }
        let tile = self
            .tiles
            .iter()
            .find(|t| {
                let d = (lon - f64::from(t.west) + 180.0).rem_euclid(360.0) - 180.0;
                lat >= f64::from(t.south) && lat <= f64::from(t.south + 1) && (0.0..=1.0).contains(&d)
            })
            .ok_or(TerrainError("missing HGT coverage"))?;
        let steps = (tile.width - 1) as f64;
        let x = ((lon - f64::from(tile.west) + 180.0).rem_euclid(360.0) - 180.0) * steps;
        let y = (lat - f64::from(tile.south)) * steps;
        let i = libm::floor(x).min(steps - 1.0);
        let j = libm::floor(y).min(steps - 1.0);
        let (u, v) = (x - i, y - j);
        let mut height = 0.0;
        for (dx, dy, weight) in [
            (0.0, 0.0, (1.0 - u) * (1.0 - v)),
            (1.0, 0.0, u * (1.0 - v)),
            (0.0, 1.0, (1.0 - u) * v),
            (1.0, 1.0, u * v),
        ] {
            if weight > 0.0 {
                height += weight
                    * self.filled(
                        f64::from(tile.south) + (j + dy) / steps,
                        f64::from(tile.west) + (i + dx) / steps,
                        1.0 / steps,
                    )?;
            }
        }
        Ok(height)
    }
    /// Uses global grid indices so adjacent tile edges are bit-identical.
    pub fn import(&self, input: &Region) -> Result<(Region, Vec<HeightTile>), TerrainError> {
        input.validate()?;
        let mut region = input.clone();
        if region.source.is_empty() {
            region.source = "SRTM".into();
        }
        if region.licence.is_empty() {
            region.licence = "public domain".into();
        }
        if region.attribution.is_empty() {
            region.attribution = SRTM_ATTRIBUTION.into();
        }
        region.tiles.clear();
        let projection = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)?;
        let mut tiles = Vec::new();
        let width = region.tile_size;
        let spacing = f64::from(region.spacing_m);
        for e in 0..region.tiles_east {
            for n in 0..region.tiles_north {
                let local = |i: u32, j: u32| {
                    [
                        region.southwest_m[0] + f64::from(n * (width - 1) + i) * spacing,
                        region.southwest_m[1] + f64::from(e * (width - 1) + j) * spacing,
                    ]
                };
                let mut samples = Vec::new();
                for j in 0..width {
                    for i in 0..width {
                        let [north, east] = local(i, j);
                        let [lat, lon] = projection.unproject(north, east)?;
                        let original = self.height(lat, lon)?;
                        samples.push(if region.airports.is_some() {
                            original
                        } else {
                            runway_height(&region, north, east, original)
                        } as f32);
                    }
                }
                tiles.push(HeightTile::new(
                    local(0, 0),
                    region.spacing_m,
                    width,
                    samples,
                    vec![Surface::Grass; (width as usize - 1) * (width as usize - 1)],
                )?);
                region.tiles.push(format!("{:03}.fcht", tiles.len() - 1));
            }
        }
        Ok((region, tiles))
    }
}

/// Flatten a padded rectangle so bilinear cells touching the actual runway are also flat.
pub fn runway_height(region: &Region, north: f64, east: f64, original: f64) -> f64 {
    let (s, c) = libm::sincos(region.origin.runway_heading_deg.to_radians());
    let along = north * c + east * s;
    let across = -north * s + east * c;
    let pad = f64::from(region.spacing_m) * 2.0;
    let distance = (-along)
        .max(along - region.origin.runway_length_m)
        .max(across.abs() - region.origin.runway_width_m * 0.5)
        - pad;
    let t = (distance / region.runway_blend_m).clamp(0.0, 1.0);
    let blend = t * t * (3.0 - 2.0 * t);
    region.origin.field_elevation_m * (1.0 - blend) + original * blend
}
