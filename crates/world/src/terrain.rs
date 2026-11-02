//! Original, bounded height tiles. All coordinates are local north/east metres;
//! heights are metres MSL, and normals point out of the ground in NED.
use crate::{Ground, Surface, ground};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"FCHT";
const HEADER: usize = 32;
pub const MAX_TILE_SIZE: u32 = 257;
pub const TRAINING_SEED: u64 = 0x7661_6c6c_6579;

#[derive(Debug, thiserror::Error, PartialEq)]
#[error("invalid terrain tile: {0}")]
pub struct TerrainError(pub &'static str);

/// Square sample grid. North varies fastest; surfaces describe the cells between samples.
#[derive(Clone, Debug, PartialEq)]
pub struct HeightTile {
    origin_m: [f64; 2],
    spacing_m: f32,
    size: u32,
    samples: Vec<f32>,
    surfaces: Vec<Surface>,
}

impl HeightTile {
    pub fn new(
        origin_m: [f64; 2],
        spacing_m: f32,
        size: u32,
        samples: Vec<f32>,
        surfaces: Vec<Surface>,
    ) -> Result<Self, TerrainError> {
        if !(2..=MAX_TILE_SIZE).contains(&size)
            || !spacing_m.is_finite()
            || !(0.1..=10_000.0).contains(&spacing_m)
            || !origin_m.iter().all(|v| v.is_finite() && v.abs() <= 1e7)
        {
            return Err(TerrainError("header outside supported bounds"));
        }
        let width = size as usize;
        if samples.len() != width * width
            || surfaces.len() != (width - 1) * (width - 1)
            || !samples.iter().all(|v| v.is_finite() && v.abs() <= 20_000.0)
        {
            return Err(TerrainError("invalid sample or surface array"));
        }
        for (cell, surface) in surfaces.iter().enumerate() {
            if *surface != Surface::Water {
                continue;
            }
            let i = cell % (width - 1);
            let j = cell / (width - 1);
            let corners = [
                j * width + i,
                j * width + i + 1,
                (j + 1) * width + i,
                (j + 1) * width + i + 1,
            ];
            let level = samples.get(j * width + i);
            if corners.iter().any(|index| samples.get(*index) != level) {
                return Err(TerrainError("water cell must be flat"));
            }
        }
        Ok(Self {
            origin_m,
            spacing_m,
            size,
            samples,
            surfaces,
        })
    }

    /// Version 1: FCHT, u16 version, u16 reserved (zero), two f64 origins,
    /// f32 spacing, u32 sample width, width² f32 heights, then (width−1)² u8 surfaces.
    /// All numbers are little-endian; no trailing bytes are permitted.
    pub fn read(bytes: &[u8]) -> Result<Self, TerrainError> {
        let mut cursor = bytes;
        fn take<const N: usize>(cursor: &mut &[u8]) -> Result<[u8; N], TerrainError> {
            let (head, tail) = cursor
                .split_at_checked(N)
                .ok_or(TerrainError("truncated bytes"))?;
            *cursor = tail;
            head.try_into().map_err(|_| TerrainError("truncated field"))
        }
        if &take::<4>(&mut cursor)? != MAGIC
            || u16::from_le_bytes(take(&mut cursor)?) != 1
            || u16::from_le_bytes(take(&mut cursor)?) != 0
        {
            return Err(TerrainError("magic, version, or reserved field"));
        }
        let origin = [
            f64::from_le_bytes(take(&mut cursor)?),
            f64::from_le_bytes(take(&mut cursor)?),
        ];
        let spacing = f32::from_le_bytes(take(&mut cursor)?);
        let size = u32::from_le_bytes(take(&mut cursor)?);
        if !(2..=MAX_TILE_SIZE).contains(&size) {
            return Err(TerrainError("sample width"));
        }
        let width = size as usize;
        if bytes.len() != HEADER + width * width * 4 + (width - 1) * (width - 1) {
            return Err(TerrainError("byte length"));
        }
        let mut samples = Vec::with_capacity(width * width);
        for _ in 0..width * width {
            samples.push(f32::from_le_bytes(take(&mut cursor)?));
        }
        let surfaces = cursor
            .iter()
            .map(|b| match b {
                0 => Ok(Surface::Grass),
                1 => Ok(Surface::Pavement),
                2 => Ok(Surface::Water),
                _ => Err(TerrainError("unknown surface")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(origin, spacing, size, samples, surfaces)
    }

    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.samples.len() * 4 + self.surfaces.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        for v in self.origin_m {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.spacing_m.to_le_bytes());
        out.extend_from_slice(&self.size.to_le_bytes());
        for v in &self.samples {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend(self.surfaces.iter().map(|s| match s {
            Surface::Grass => 0,
            Surface::Pavement => 1,
            Surface::Water => 2,
        }));
        out
    }
    #[must_use]
    pub fn origin_m(&self) -> [f64; 2] {
        self.origin_m
    }
    #[must_use]
    pub fn spacing_m(&self) -> f64 {
        f64::from(self.spacing_m)
    }
    #[must_use]
    pub fn size(&self) -> u32 {
        self.size
    }
    #[must_use]
    pub fn extent_m(&self) -> f64 {
        f64::from(self.size - 1) * self.spacing_m()
    }

    /// Inclusive edges; deterministic first-tile ownership at shared edges in a World.
    #[must_use]
    pub fn sample(&self, north: f64, east: f64) -> Option<Ground> {
        let [on, oe] = self.origin_m;
        let n = (north - on) / self.spacing_m();
        let e = (east - oe) / self.spacing_m();
        let end = f64::from(self.size - 1);
        if !n.is_finite() || !e.is_finite() || !(0.0..=end).contains(&n) || !(0.0..=end).contains(&e) {
            return None;
        }
        let i = (libm::floor(n) as usize).min(self.size as usize - 2);
        let j = (libm::floor(e) as usize).min(self.size as usize - 2);
        let (u, v) = (n - i as f64, e - j as f64);
        let width = self.size as usize;
        let a = f64::from(*self.samples.get(j * width + i)?);
        let b = f64::from(*self.samples.get(j * width + i + 1)?);
        let c = f64::from(*self.samples.get((j + 1) * width + i)?);
        let d = f64::from(*self.samples.get((j + 1) * width + i + 1)?);
        let dn = ((b - a) * (1.0 - v) + (d - c) * v) / self.spacing_m();
        let de = ((c - a) * (1.0 - u) + (d - b) * u) / self.spacing_m();
        let length = libm::sqrt(1.0 + dn * dn + de * de);
        Some(Ground {
            height_msl_m: (a * (1.0 - u) + b * u) * (1.0 - v) + (c * (1.0 - u) + d * u) * v,
            normal_ned: [-dn / length, -de / length, -1.0 / length],
            surface: *self.surfaces.get(j * (width - 1) + i)?,
        })
    }
}

/// Built-in world names remain stable; other strings name an installed region.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum WorldChoice {
    #[default]
    Flat,
    TrainingValley,
    Region(String),
}
impl From<String> for WorldChoice {
    fn from(id: String) -> Self {
        match id.as_str() {
            "flat" => Self::Flat,
            "training-valley" => Self::TrainingValley,
            _ => Self::Region(id),
        }
    }
}
impl From<WorldChoice> for String {
    fn from(choice: WorldChoice) -> Self {
        match choice {
            WorldChoice::Flat => "flat".into(),
            WorldChoice::TrainingValley => "training-valley".into(),
            WorldChoice::Region(id) => id,
        }
    }
}
impl JsonSchema for WorldChoice {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "WorldChoice".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        String::json_schema(generator)
    }
}

/// Immutable tiles may be shared with presentation. Uncovered or non-finite queries use flat ground.
#[derive(Clone, Debug)]
pub struct World {
    choice: WorldChoice,
    tiles: Arc<[HeightTile]>,
    fingerprint: u64,
    region: Option<Arc<crate::Region>>,
}
impl Default for World {
    fn default() -> Self {
        Self {
            choice: WorldChoice::Flat,
            region: None,
            tiles: Arc::from([]),
            fingerprint: 0xcbf2_9ce4_8422_2325,
        }
    }
}
impl World {
    pub fn load(choice: WorldChoice) -> Result<Self, TerrainError> {
        let tiles = match &choice {
            WorldChoice::Flat => Vec::new(),
            WorldChoice::TrainingValley => bundled_training_valley()?,
            WorldChoice::Region(_) => return Err(TerrainError("region must be installed before selection")),
        };
        Self::build(choice, tiles)
    }
    pub fn from_tiles(tiles: Vec<HeightTile>) -> Result<Self, TerrainError> {
        if tiles.len() > 256 {
            return Err(TerrainError("too many tiles"));
        }
        Self::build(WorldChoice::Flat, tiles)
    }
    #[must_use]
    pub fn choice(&self) -> WorldChoice {
        self.choice.clone()
    }
    #[must_use]
    pub fn tiles(&self) -> &[HeightTile] {
        &self.tiles
    }
    #[must_use]
    pub fn ground(&self, north: f64, east: f64) -> Ground {
        let flat = if self.region.is_some() {
            let runway = self.runway();
            let (s, c) = libm::sincos(runway.heading_deg.to_radians());
            let along = north * c + east * s;
            let across = -north * s + east * c;
            Ground {
                height_msl_m: -runway.threshold_ned_m[2],
                normal_ned: [0.0, 0.0, -1.0],
                surface: if (0.0..=runway.length_m).contains(&along) && across.abs() <= runway.width_m * 0.5 {
                    Surface::Pavement
                } else {
                    Surface::Grass
                },
            }
        } else {
            ground(north, east)
        };
        if flat.surface == Surface::Pavement {
            return flat;
        }
        self.tiles
            .iter()
            .find_map(|tile| tile.sample(north, east))
            .unwrap_or(flat)
    }
    fn build(choice: WorldChoice, tiles: Vec<HeightTile>) -> Result<Self, TerrainError> {
        let mut fingerprint: u64 = 0xcbf2_9ce4_8422_2325;
        for tile in &tiles {
            for byte in tile.write() {
                fingerprint ^= u64::from(byte);
                fingerprint = fingerprint.wrapping_mul(0x100000001b3);
            }
        }
        Ok(Self {
            choice,
            tiles: tiles.into(),
            fingerprint,
            region: None,
        })
    }
    /// Construct a region only after all declared tiles and their grid positions validate.
    pub fn from_region(region: crate::Region, tiles: Vec<HeightTile>) -> Result<Self, TerrainError> {
        region.validate_manifest()?;
        if tiles.len() != region.tiles.len() {
            return Err(TerrainError("missing region tiles"));
        }
        let extent = f64::from(region.tile_size - 1) * f64::from(region.spacing_m);
        for (i, tile) in tiles.iter().enumerate() {
            let n = i % region.tiles_north as usize;
            let e = i / region.tiles_north as usize;
            let expected = [
                region.southwest_m[0] + n as f64 * extent,
                region.southwest_m[1] + e as f64 * extent,
            ];
            if tile.origin_m() != expected
                || tile.size() != region.tile_size
                || tile.spacing_m() != f64::from(region.spacing_m)
            {
                return Err(TerrainError("region tile grid mismatch"));
            }
        }
        let mut world = Self::build(WorldChoice::Region(region.id.clone()), tiles)?;
        // Runway and metadata affect replay identity as well as tile content.
        for byte in serde_json::to_vec(&region).map_err(|_| TerrainError("region identity"))? {
            world.fingerprint ^= u64::from(byte);
            world.fingerprint = world.fingerprint.wrapping_mul(0x100000001b3);
        }
        world.region = Some(Arc::new(region));
        Ok(world)
    }
    #[must_use]
    pub fn region(&self) -> Option<&crate::Region> {
        self.region.as_deref()
    }
    #[must_use]
    pub fn runway(&self) -> crate::Runway {
        self.region().map_or(crate::RUNWAY, crate::Region::runway)
    }
    #[must_use]
    pub fn attribution(&self) -> &str {
        self.region()
            .map_or("Original FlightCraft terrain · Apache-2.0", |r| {
                r.attribution.as_str()
            })
    }
    /// Stable content identity, computed once when tiles load.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
}

/// Original analytic landscape; seeded phases, without an external noise algorithm or data.
/// A 12 km square, 36 tiles, 31.25 m sample spacing, and a flat airfield apron.
pub fn generate_training_valley(seed: u64) -> Result<Vec<HeightTile>, TerrainError> {
    let mut bits = seed;
    let mut phase = || {
        bits = bits
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (bits >> 32) as f64 / u32::MAX as f64 * core::f64::consts::TAU
    };
    let (p, q) = (phase(), phase());
    let smooth = |x: f64| {
        let t = x.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let lake_distance = |n: f64, e: f64| ((n + 3250.0).abs() - 750.0).max((e - 3000.0).abs() - 500.0);
    let height = |n: f64, e: f64| {
        let ridge = 510.0
            * libm::exp(
                -((e - 2900.0 - 250.0 * libm::sin(n / 1400.0)) / 850.0)
                    * ((e - 2900.0 - 250.0 * libm::sin(n / 1400.0)) / 850.0)
                    - ((n - 2200.0) / 3100.0) * ((n - 2200.0) / 3100.0),
            );
        let hills = 65.0
            + 45.0 * libm::sin(n / 800.0 + p) * libm::cos(e / 1100.0 + q)
            + 25.0 * libm::sin((n + e) / 530.0 + q);
        let lake = smooth(lake_distance(n, e) / 500.0);
        let h = 35.0 * (1.0 - lake) + (hills + ridge).max(35.0) * lake;
        let apron = smooth(((n - 400.0).abs() - 1400.0).max(e.abs() - 400.0) / 800.0);
        let edge = smooth((6000.0 - n.abs().max(e.abs())) / 750.0);
        (h * apron * edge).min(610.0) as f32
    };
    let mut tiles = Vec::new();
    for e in 0..6 {
        for n in 0..6 {
            let origin = [-6000.0 + f64::from(n) * 2000.0, -6000.0 + f64::from(e) * 2000.0];
            let mut samples = Vec::new();
            let mut surfaces = Vec::new();
            for j in 0..65 {
                for i in 0..65 {
                    samples.push(height(
                        origin[0] + f64::from(i) * 31.25,
                        origin[1] + f64::from(j) * 31.25,
                    ));
                }
            }
            for j in 0..64 {
                for i in 0..64 {
                    let n = origin[0] + f64::from(i) * 31.25;
                    let e = origin[1] + f64::from(j) * 31.25;
                    let water = [(n, e), (n + 31.25, e), (n, e + 31.25), (n + 31.25, e + 31.25)]
                        .iter()
                        .all(|&(n, e)| lake_distance(n, e) <= 0.0);
                    surfaces.push(if water { Surface::Water } else { Surface::Grass });
                }
            }
            tiles.push(HeightTile::new(origin, 31.25, 65, samples, surfaces)?);
        }
    }
    Ok(tiles)
}

fn bundled_training_valley() -> Result<Vec<HeightTile>, TerrainError> {
    [
        include_bytes!("../../../assets/terrain/training-valley/00.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/01.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/02.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/03.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/04.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/05.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/06.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/07.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/08.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/09.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/10.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/11.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/12.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/13.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/14.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/15.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/16.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/17.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/18.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/19.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/20.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/21.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/22.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/23.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/24.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/25.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/26.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/27.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/28.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/29.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/30.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/31.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/32.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/33.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/34.fcht").as_slice(),
        include_bytes!("../../../assets/terrain/training-valley/35.fcht").as_slice(),
    ]
    .into_iter()
    .map(HeightTile::read)
    .collect()
}
