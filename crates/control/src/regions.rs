//! Native and browser transports install immutable, validated terrain before reset.
use crate::{Command, CommandError, Session, WorldChoice};
use flightcraft_world::{HeightTile, TerrainError, World};
pub use flightcraft_world::{MAX_REGION_BYTES, MAX_REGION_TILE_BYTES, MAX_REGIONS, MAX_TERRAIN_BYTES};
pub use flightcraft_world::{Region, World as TerrainWorld};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug)]
pub struct RegionOptions {
    pub directory: Option<String>,
    pub fetched: Vec<World>,
    pub errors: Vec<String>,
}
impl Session {
    pub fn install_region(&mut self, world: World) -> Result<(), CommandError> {
        let Some(region) = world.region() else {
            return Err(CommandError::Failed("region metadata required".into()));
        };
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
    region.validate_manifest()?;
    if tiles.len() != region.tiles.len() {
        return Err(TerrainError("missing region tiles"));
    }
    let tiles = tiles
        .iter()
        .map(|b| HeightTile::read(b))
        .collect::<Result<Vec<_>, _>>()?;
    World::from_region(region, tiles)
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
    region_world(region, tiles).map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize, JsonSchema)]
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
