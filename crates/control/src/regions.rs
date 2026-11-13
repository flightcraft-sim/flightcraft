//! Native and browser transports install immutable, validated terrain before reset.
use crate::{Command, CommandError, Session, WorldChoice};
pub use flightcraft_world::MAX_AIRSPACE_BYTES;
pub use flightcraft_world::{Airport, Airports, MAX_AIRPORT_BYTES, Region, World as TerrainWorld};
use flightcraft_world::{HeightTile, TerrainError, World};
pub use flightcraft_world::{MAX_REGION_BYTES, MAX_REGION_TILE_BYTES, MAX_REGIONS, MAX_TERRAIN_BYTES};

/// Browser and native transports use the same bounded airspace reader.
pub fn attach_region_airspace(world: World, text: &str) -> Result<World, TerrainError> {
    world.with_airspace(flightcraft_world::AirspaceFile::from_json(text)?)
}
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug)]
pub struct RegionOptions {
    pub directory: Option<String>,
    pub fetched: Vec<World>,
    pub errors: Vec<String>,
}
impl Session {
    /// Whether a region with this id is installed and can be named in `sim.reset`.
    #[must_use]
    pub fn has_region(&self, id: &str) -> bool {
        self.regions.contains_key(id)
    }
    pub fn install_region(&mut self, world: World) -> Result<(), CommandError> {
        let Some(region) = world.region() else {
            return Err(CommandError::Failed("region metadata required".into()));
        };
        if region.airports.is_some() != world.airports().is_some() {
            return Err(CommandError::Failed(
                "region airport catalogue is incomplete".into(),
            ));
        }
        let installed = self
            .regions
            .values()
            .filter_map(World::region)
            .fold(0_usize, |total, r| total.saturating_add(r.tile_bytes()));
        if self.regions.len() >= MAX_REGIONS
            || self.regions.contains_key(&region.id)
            || installed.saturating_add(region.tile_bytes()) > MAX_TERRAIN_BYTES
        {
            return Err(CommandError::Failed(
                "duplicate region or region count/128 MiB budget exceeded".into(),
            ));
        }
        self.regions.insert(region.id.clone(), world);
        Ok(())
    }
    pub(crate) fn resolve_world(&self, choice: WorldChoice) -> Result<World, CommandError> {
        match choice {
            WorldChoice::Region(id) => self
                .regions
                .get(&id)
                .cloned()
                .ok_or_else(|| CommandError::Failed(format!("world region is not installed: {id}"))),
            builtin => World::load(builtin).map_err(|e| CommandError::Sim(e.into())),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_regions_directory(&mut self, directory: &std::path::Path) -> Vec<String> {
        let entries = match std::fs::read_dir(directory) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => return vec![format!("regions directory: {e}")],
        };
        let mut paths = Vec::new();
        let mut errors = Vec::new();
        for entry in entries.take(MAX_REGIONS + 2) {
            match entry {
                Ok(e) => paths.push(e.path()),
                Err(e) => errors.push(e.to_string()),
            }
        }
        // The default assets directory also holds the built-in training valley.
        if paths.len() > MAX_REGIONS + 1 {
            return vec!["too many region directories".into()];
        }
        paths.sort();
        for path in paths {
            if !path.is_dir() || !path.join("region.json").exists() {
                continue;
            }
            match read_region(&path).and_then(|world| self.install_region(world).map_err(|e| e.to_string())) {
                Ok(()) => {}
                Err(e) => errors.push(format!("region {}: {e}", path.display())),
            }
        }
        errors
    }
}

/// Loaders for every transport finish validation here; no partial world can be selected.
pub fn region_world(region: Region, tiles: Vec<Vec<u8>>) -> Result<World, TerrainError> {
    region_world_with_airports(region, tiles, None)
}
/// Common validation for native and web transports, including the adjacent catalogue.
pub fn region_world_with_airports(
    region: Region,
    tiles: Vec<Vec<u8>>,
    airports: Option<Vec<u8>>,
) -> Result<World, TerrainError> {
    region.validate_manifest()?;
    if region.airports.is_some() != airports.is_some() {
        return Err(TerrainError("missing or unexpected airport catalogue"));
    }
    let catalogue = airports
        .as_ref()
        .map(|b| Airports::from_json(b, &region))
        .transpose()?;
    if tiles.len() != region.tiles.len() {
        return Err(TerrainError("missing region tiles"));
    }
    let tiles = tiles
        .iter()
        .map(|b| HeightTile::read(b))
        .collect::<Result<Vec<_>, _>>()?;
    let world = World::from_region(region, tiles)?;
    match catalogue {
        Some(a) => world.with_airports(a),
        None => Ok(world),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_region(directory: &std::path::Path) -> Result<World, String> {
    use std::io::Read;
    fn bounded(path: &std::path::Path, max: usize) -> Result<Vec<u8>, String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > max as u64 {
            return Err("region requires bounded regular files without symlinks".into());
        }
        let mut bytes = Vec::new();
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("region requires regular file".into());
        }
        file.take(max as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > max {
            return Err("region file too large".into());
        }
        Ok(bytes)
    }
    if std::fs::symlink_metadata(directory)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("symlink region directory".into());
    }
    let bytes = bounded(&directory.join("region.json"), MAX_REGION_BYTES)?;
    let region = Region::from_json(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    region.validate_manifest().map_err(|e| e.to_string())?;
    let tiles = region
        .tiles
        .iter()
        .map(|name| bounded(&directory.join(name), MAX_REGION_TILE_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    let airports = region
        .airports
        .as_ref()
        .map(|name| bounded(&directory.join(name), MAX_AIRPORT_BYTES))
        .transpose()?;
    let world = region_world_with_airports(region, tiles, airports).map_err(|e| e.to_string())?;
    let world = match std::fs::symlink_metadata(directory.join("navdata.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => world,
        Err(e) => return Err(e.to_string()),
        Ok(_) => {
            let bytes = bounded(&directory.join("navdata.json"), MAX_NAVDATA_BYTES)?;
            attach_region_navdata(world, std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        }
    };
    let world = match std::fs::symlink_metadata(directory.join("procedures.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => world,
        Err(e) => return Err(e.to_string()),
        Ok(_) => {
            let bytes = bounded(&directory.join("procedures.json"), MAX_PROCEDURES_BYTES)?;
            attach_region_procedures(world, std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        }
    };
    let world = match std::fs::symlink_metadata(directory.join("landmarks.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => world,
        Err(e) => return Err(e.to_string()),
        Ok(_) => {
            let bytes = bounded(
                &directory.join("landmarks.json"),
                flightcraft_world::MAX_LANDMARK_BYTES,
            )?;
            attach_region_landmarks(world, std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        }
    };
    let path = directory.join("airspace.json");
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(world),
        Err(e) => Err(e.to_string()),
        Ok(_) => {
            let bytes = bounded(&path, MAX_AIRSPACE_BYTES)?;
            attach_region_airspace(world, std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListWorlds {}
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorldInfo {
    pub id: WorldChoice,
    pub name: String,
    pub attribution: String,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorldList {
    pub worlds: Vec<WorldInfo>,
}
impl Command for ListWorlds {
    const NAME: &'static str = "world.list";
    const DESCRIPTION: &'static str =
        "List built-in worlds and installed terrain regions available to sim.reset.";
    type Output = WorldList;
    fn run(self, session: &mut Session) -> Result<WorldList, CommandError> {
        let mut worlds = vec![
            WorldInfo {
                id: WorldChoice::Flat,
                name: "Flat".into(),
                attribution: World::default().attribution().into(),
            },
            WorldInfo {
                id: WorldChoice::TrainingValley,
                name: "Training valley".into(),
                attribution: World::default().attribution().into(),
            },
        ];
        for world in session.regions.values() {
            if let Some(region) = world.region() {
                worlds.push(WorldInfo {
                    id: world.choice(),
                    name: region.name.clone(),
                    attribution: region.attribution.clone(),
                });
            }
        }
        Ok(WorldList { worlds })
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AirportQuery {
    /// Optional local north/east position; results are ordered by airport distance.
    #[serde(default)]
    pub nearest_to_m: Option<[f64; 2]>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct AirportList {
    pub attribution: String,
    pub airports: Vec<Airport>,
}
impl Command for AirportQuery {
    const NAME: &'static str = "world.airports";
    const DESCRIPTION: &'static str = "List active region airports and runways, optionally nearest first.";
    type Output = AirportList;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        let world = session.sim.world();
        let catalogue = world.airports();
        let mut airports = catalogue.map_or_else(Vec::new, |a| a.airports.clone());
        if let Some([n, e]) = self.nearest_to_m {
            let p = projection(world)?;
            p.unproject(n, e)
                .map_err(|e| CommandError::Failed(e.to_string()))?;
            let distance = |a: &Airport| -> f64 {
                p.project(a.latitude_deg, a.longitude_deg)
                    .map_or(f64::MAX, |[an, ae]| (an - n) * (an - n) + (ae - e) * (ae - e))
            };
            airports.sort_by(|a, b| distance(a).total_cmp(&distance(b)).then(a.ident.cmp(&b.ident)));
        }
        Ok(AirportList {
            attribution: catalogue.map_or_else(String::new, |a| a.attribution.clone()),
            airports,
        })
    }
}
fn projection(world: &World) -> Result<flightcraft_world::geo::LocalProjection, CommandError> {
    let r = world
        .region()
        .ok_or_else(|| CommandError::Failed("active world has no geographic origin".into()))?;
    flightcraft_world::geo::LocalProjection::new(r.origin.latitude_deg, r.origin.longitude_deg)
        .map_err(|e| CommandError::Failed(e.to_string()))
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(untagged)]
#[schemars(extend("type" = "object"))]
pub enum Geodetic {
    ToGeographic(LocalCoordinate),
    ToLocal(GeographicCoordinate),
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalCoordinate {
    pub north_m: f64,
    pub east_m: f64,
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeographicCoordinate {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct Coordinate {
    pub north_m: f64,
    pub east_m: f64,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
}
impl Command for Geodetic {
    const NAME: &'static str = "world.geodetic";
    const DESCRIPTION: &'static str =
        "Convert local north/east metres and WGS84 latitude/longitude in the active region.";
    type Output = Coordinate;
    fn run(self, session: &mut Session) -> Result<Coordinate, CommandError> {
        let p = projection(session.sim.world())?;
        let result = match self {
            Self::ToGeographic(LocalCoordinate { north_m, east_m }) => {
                p.unproject(north_m, east_m)
                    .map(|[latitude_deg, longitude_deg]| Coordinate {
                        north_m,
                        east_m,
                        latitude_deg,
                        longitude_deg,
                    })
            }
            Self::ToLocal(GeographicCoordinate {
                latitude_deg,
                longitude_deg,
            }) => p
                .project(latitude_deg, longitude_deg)
                .map(|[north_m, east_m]| Coordinate {
                    north_m,
                    east_m,
                    latitude_deg,
                    longitude_deg,
                }),
        };
        result.map_err(|e| CommandError::Failed(e.to_string()))
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AirportRunwayStart {
    pub airport: String,
    pub end: String,
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AirportAirStart {
    pub airport: String,
    pub distance_m: f64,
    /// Magnetic bearing from the airport, clockwise in degrees.
    pub bearing_deg: f64,
    /// Height above airport elevation.
    pub height_m: f64,
    pub airspeed_mps: f64,
    /// Magnetic heading, radians clockwise from magnetic north.
    pub heading_rad: f64,
}
pub(crate) fn airport<'a>(world: &'a World, ident: &str) -> Result<&'a Airport, CommandError> {
    world
        .airports()
        .and_then(|a| {
            a.airports
                .iter()
                .find(|a| a.ident == ident || a.icao.as_deref() == Some(ident))
        })
        .ok_or_else(|| CommandError::Failed(format!("airport is not in the active region: {ident}")))
}
pub(crate) fn runway_start(
    world: &World,
    start: AirportRunwayStart,
) -> Result<flightcraft_sim::Start, CommandError> {
    let airport = airport(world, &start.airport)?;
    let (runway, end) = airport
        .runways
        .iter()
        .find_map(|r| r.ends.iter().find(|e| e.ident == start.end).map(|e| (r, e)))
        .ok_or_else(|| CommandError::Failed("unknown runway end".into()))?;
    let p = projection(world)?;
    let [n, e] = p
        .project(end.latitude_deg, end.longitude_deg)
        .map_err(|e| CommandError::Failed(e.to_string()))?;
    let opposite = runway
        .ends
        .iter()
        .find(|e| e.ident != end.ident)
        .ok_or_else(|| CommandError::Failed("invalid runway ends".into()))?;
    let [on, oe] = p
        .project(opposite.latitude_deg, opposite.longitude_deg)
        .map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok(flightcraft_sim::Start::RunwayAt {
        threshold_ned_m: [n, e, -end.elevation_m],
        heading_rad: libm::atan2(oe - e, on - n).rem_euclid(core::f64::consts::TAU),
    })
}
pub(crate) fn air_start(
    world: &World,
    start: AirportAirStart,
    date: flightcraft_world::geomag::SimulationDate,
) -> Result<flightcraft_sim::Start, CommandError> {
    let airport = airport(world, &start.airport)?;
    crate::range("sim.reset", "distance_m", start.distance_m, 0.0, 250000.0)?;
    crate::range("sim.reset", "bearing_deg", start.bearing_deg, 0.0, 360.0)?;
    crate::range("sim.reset", "height_m", start.height_m, 2.0, 20000.0)?;
    crate::range(
        "sim.reset",
        "heading_rad",
        start.heading_rad,
        0.0,
        core::f64::consts::TAU,
    )?;
    let p = projection(world)?;
    let [n, e] = p
        .project(airport.latitude_deg, airport.longitude_deg)
        .map_err(|e| CommandError::Failed(e.to_string()))?;
    // Resolve true bearing in the local grid using a short meridian segment.
    let [nn, ne] = p
        .project(airport.latitude_deg + 0.00001, airport.longitude_deg)
        .map_err(|e| CommandError::Failed(e.to_string()))?;
    let reference = flightcraft_sim::magnetic::reference_at(world, [n, e, -airport.elevation_m], date)
        .map_err(flightcraft_sim::SimError::from)?;
    let bearing = reference.magnetic_to_true(start.bearing_deg.to_radians()) + libm::atan2(ne - e, nn - n);
    let (n, e) = (
        n + start.distance_m * libm::cos(bearing),
        e + start.distance_m * libm::sin(bearing),
    );
    flightcraft_world::inside(
        world
            .region()
            .ok_or_else(|| CommandError::Failed("region required".into()))?,
        [n, e],
    )
    .map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok(flightcraft_sim::Start::AirAt {
        position_m: [n, e, -(airport.elevation_m + start.height_m)],
        airspeed_mps: start.airspeed_mps,
        heading_rad: start.heading_rad,
    })
}

/// Shared bounded navigation sidecar loader.
pub fn attach_region_navdata(world: World, text: &str) -> Result<World, TerrainError> {
    world.with_navdata(flightcraft_world::Navdata::from_json(text)?)
}
pub use flightcraft_world::MAX_NAVDATA_BYTES;

/// Shared strict procedure sidecar loader for native and browser transports.
pub fn attach_region_procedures(world: World, text: &str) -> Result<World, TerrainError> {
    world.with_procedures(flightcraft_world::Procedures::from_json(text)?)
}
pub use flightcraft_world::MAX_PROCEDURES_BYTES;

/// Shared bounded GNIS sidecar loader for native and browser transports.
pub fn attach_region_landmarks(world: World, text: &str) -> Result<World, TerrainError> {
    world.with_landmarks(flightcraft_world::Landmarks::from_json(text)?)
}
