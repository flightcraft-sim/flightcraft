//! Version 2 source documents compose by stable ids. Physics consumes a resolved `Aircraft`.
use crate::aircraft::{error, finite_tree, positive, require};
use crate::{Aerodynamics, Aircraft, AircraftError, Geometry, Identity, Mass, Propulsion, Systems, Table};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const FORMAT_VERSION: u32 = 2;
pub const INSTRUMENT_MODULES: &[&str] = &[
    "airspeed",
    "attitude",
    "altimeter",
    "turn_coordinator",
    "heading",
    "vertical_speed",
    "tachometer",
    "oil_temperature",
    "oil_pressure",
    "egt",
    "cht",
    "fuel_left",
    "fuel_right",
    "fuel_flow",
    "ammeter",
    "volts",
    "suction",
    "annunciators",
    "electronic_attitude",
    "electronic_hsi",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Family {
    #[schemars(range(min = 2, max = 2))]
    pub format_version: u32,
    pub identity: Identity,
    pub geometry: Geometry,
    pub aerodynamics: Aerodynamics,
    pub controls: AirframeControls,
}

/// Surface travel and ground contact properties belong to the airframe; detents to the variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AirframeControls {
    pub elevator: [f64; 2],
    pub aileron: [f64; 2],
    pub rudder: [f64; 2],
    pub throttle: [f64; 2],
    pub flaps: [f64; 2],
    pub brake: [f64; 2],
    pub gear_travel_m: f64,
    pub max_normal_force_n: f64,
    pub hard_touchdown_mps: f64,
    pub rolling_pavement: f64,
    pub rolling_other: f64,
    pub brake_friction: f64,
    pub lateral_friction: f64,
}

impl AirframeControls {
    fn resolve(&self, flap_stages: Vec<f64>) -> Systems {
        Systems {
            elevator: self.elevator,
            aileron: self.aileron,
            rudder: self.rudder,
            throttle: self.throttle,
            flaps: self.flaps,
            brake: self.brake,
            flap_stages,
            gear_travel_m: self.gear_travel_m,
            max_normal_force_n: self.max_normal_force_n,
            hard_touchdown_mps: self.hard_touchdown_mps,
            rolling_pavement: self.rolling_pavement,
            rolling_other: self.rolling_other,
            brake_friction: self.brake_friction,
            lateral_friction: self.lateral_friction,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    #[schemars(range(min = 2, max = 2))]
    pub format_version: u32,
    pub identity: Identity,
    pub family: String,
    pub engine: String,
    pub systems: String,
    pub empty_mass: Mass,
    pub min_mass_kg: f64,
    pub max_mass_kg: f64,
    /// Polygon vertices [longitudinal CG in metres, total mass in kg], ordered around its edge.
    #[schemars(length(min = 3, max = 64))]
    pub cg_envelope: Vec<[f64; 2]>,
    #[schemars(length(min = 2, max = 32))]
    pub flap_stages: Vec<FlapStage>,
    pub fuel_capacity_kg: f64,
    #[schemars(length(max = 64))]
    pub stations: Vec<LoadStation>,
    pub v_speeds: VSpeeds,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlapStage {
    pub position: f64,
    pub max_speed_mps: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VSpeeds {
    pub vs_mps: f64,
    pub vso_mps: f64,
    pub vr_mps: f64,
    pub vx_mps: f64,
    pub vy_mps: f64,
    pub vfe_mps: f64,
    pub vno_mps: f64,
    pub vne_mps: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StationKind {
    Pilot,
    Passenger,
    Baggage,
    Fuel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoadStation {
    pub id: String,
    pub kind: StationKind,
    pub position_m: [f64; 3],
    pub max_mass_kg: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Load {
    #[schemars(length(max = 64))]
    pub stations: Vec<StationLoad>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StationLoad {
    pub station: String,
    pub mass_kg: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Induction {
    Carburettor,
    Injection,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Engine {
    #[schemars(range(min = 2, max = 2))]
    pub format_version: u32,
    pub identity: Identity,
    /// Shaft power versus density ratio and fixed-pitch propeller CT/CP versus advance ratio.
    pub propulsion: Propulsion,
    pub induction: Induction,
    /// Fuel flow against normalised shaft power, kg/s. Metadata until fuel burn is implemented.
    pub fuel_flow_kg_s: Table,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstalledSystems {
    #[schemars(range(min = 2, max = 2))]
    pub format_version: u32,
    pub identity: Identity,
    #[schemars(length(max = 64))]
    pub installed: Vec<String>,
    /// Absent for legacy placeholder installations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<crate::SystemsDefinition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    #[schemars(range(min = 2, max = 2))]
    pub format_version: u32,
    pub identity: Identity,
    #[schemars(length(min = 1, max = 64))]
    pub instruments: Vec<InstrumentInstallation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstrumentInstallation {
    pub id: String,
    pub module: String,
    /// Runtime skin registry id. Omitted declarations use the original public skin.
    #[serde(default = "default_skin")]
    pub skin: String,
    /// Panel grid position and size, in arbitrary panel units.
    pub position: [f64; 2],
    pub size: [f64; 2],
}

fn default_skin() -> String {
    "original".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AircraftDefinition {
    #[schemars(range(min = 2, max = 2))]
    pub format_version: u32,
    pub identity: Identity,
    pub variant: String,
    pub panel: String,
    pub load: Load,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerFile {
    Family(Box<Family>),
    Variant(Variant),
    Engine(Engine),
    Systems(InstalledSystems),
    Panel(Panel),
    Aircraft(AircraftDefinition),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Family,
    Variant,
    Engine,
    Systems,
    Panel,
    Aircraft,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DefinitionSource {
    BuiltIn,
    Pack { id: String },
}

#[derive(Clone, Debug)]
pub struct ResolvedAircraft {
    pub aircraft: Aircraft,
    pub variant: Variant,
    pub engine: Engine,
    pub installed_systems: InstalledSystems,
    pub panel: Panel,
    pub load: Load,
    pub weight_balance: WeightBalance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeightBalance {
    pub mass_kg: f64,
    pub weight_n: f64,
    pub cg_m: [f64; 3],
    pub in_limits: bool,
    pub violations: Vec<String>,
}

pub(crate) fn id(value: &str, field: &str) -> Result<(), AircraftError> {
    require(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_'),
        field,
        "requires 1 to 128 lowercase ASCII letters, digits, hyphens, or underscores",
    )
}

fn numeric<T: Serialize>(value: &T) -> Result<(), AircraftError> {
    finite_tree(
        &serde_json::to_value(value).map_err(|e| error("document", e.to_string()))?,
        "",
    )
}

impl LayerFile {
    pub fn from_json(text: &str) -> Result<Self, AircraftError> {
        require(
            text.len() <= crate::MAX_AIRCRAFT_BYTES,
            "file_size",
            "layer exceeds 1 MiB",
        )?;
        let mut decoder = serde_json::Deserializer::from_str(text);
        let file: Self = serde_path_to_error::deserialize(&mut decoder)
            .map_err(|e| error(e.path().to_string(), e.inner().to_string()))?;
        decoder.end().map_err(|e| error("document", e.to_string()))?;
        file.validate()?;
        Ok(file)
    }

    pub fn kind(&self) -> FileKind {
        match self {
            Self::Family(_) => FileKind::Family,
            Self::Variant(_) => FileKind::Variant,
            Self::Engine(_) => FileKind::Engine,
            Self::Systems(_) => FileKind::Systems,
            Self::Panel(_) => FileKind::Panel,
            Self::Aircraft(_) => FileKind::Aircraft,
        }
    }

    pub fn identity(&self) -> &Identity {
        match self {
            Self::Family(v) => &v.identity,
            Self::Variant(v) => &v.identity,
            Self::Engine(v) => &v.identity,
            Self::Systems(v) => &v.identity,
            Self::Panel(v) => &v.identity,
            Self::Aircraft(v) => &v.identity,
        }
    }

    pub fn validate(&self) -> Result<(), AircraftError> {
        numeric(self)?;
        let version = match self {
            Self::Family(v) => v.format_version,
            Self::Variant(v) => v.format_version,
            Self::Engine(v) => v.format_version,
            Self::Systems(v) => v.format_version,
            Self::Panel(v) => v.format_version,
            Self::Aircraft(v) => v.format_version,
        };
        require(
            version == FORMAT_VERSION,
            "format_version",
            "only version 2 is supported",
        )?;
        let identity = self.identity();
        id(&identity.id, "identity.id")?;
        for (name, text) in [
            ("display_name", &identity.display_name),
            ("description", &identity.description),
            ("author", &identity.author),
        ] {
            require(
                !text.trim().is_empty() && text.len() <= 4096,
                name,
                "requires 1 to 4096 bytes of text",
            )?;
        }
        match self {
            Self::Variant(v) => v.validate(),
            Self::Panel(p) => {
                require(
                    (1..=64).contains(&p.instruments.len()),
                    "instruments",
                    "requires 1 to 64 installations",
                )?;
                let mut seen = BTreeSet::new();
                for instrument in &p.instruments {
                    id(&instrument.id, "instruments.id")?;
                    require(
                        seen.insert(&instrument.id),
                        "instruments.id",
                        "duplicate installation id",
                    )?;
                    require(
                        INSTRUMENT_MODULES.contains(&instrument.module.as_str()),
                        "instruments.module",
                        "unknown instrument module",
                    )?;
                    id(&instrument.skin, "instruments.skin")?;
                    for size in instrument.size {
                        positive(size, "instruments.size")?;
                    }
                    require(
                        instrument
                            .position
                            .iter()
                            .all(|v| v.is_finite() && *v >= 0.0 && *v <= 10000.0),
                        "instruments.position",
                        "must be nonnegative",
                    )?;
                }
                Ok(())
            }
            Self::Systems(s) => {
                if let Some(d) = &s.definition {
                    d.validate()?;
                }
                require(s.installed.len() <= 64, "installed", "at most 64 systems")?;
                let mut seen = BTreeSet::new();
                for system in &s.installed {
                    id(system, "installed")?;
                    require(seen.insert(system), "installed", "duplicate system")?;
                }
                Ok(())
            }
            Self::Engine(e) => {
                e.fuel_flow_kg_s.validate("fuel_flow_kg_s")?;
                require(
                    e.fuel_flow_kg_s.points.first().map(|p| p.0) == Some(0.0)
                        && e.fuel_flow_kg_s.points.last().map(|p| p.0) == Some(1.0)
                        && e.fuel_flow_kg_s.points.iter().all(|(_, y)| *y >= 0.0),
                    "fuel_flow_kg_s",
                    "requires power fractions 0 through 1 and nonnegative flow",
                )
            }
            Self::Aircraft(a) => {
                id(&a.variant, "variant")?;
                id(&a.panel, "panel")?;
                validate_load(&a.load)
            }
            Self::Family(_) => Ok(()), // Full physical validation follows reference resolution.
        }
    }
}

fn validate_load(load: &Load) -> Result<(), AircraftError> {
    numeric(load)?;
    require(load.stations.len() <= 64, "load.stations", "at most 64 stations")?;
    let mut seen = BTreeSet::new();
    for entry in &load.stations {
        id(&entry.station, "load.stations.station")?;
        require(seen.insert(&entry.station), "load.stations", "duplicate station")?;
        require(
            entry.mass_kg >= 0.0,
            "load.stations.mass_kg",
            "must be nonnegative",
        )?;
    }
    Ok(())
}

// Cross products use metre/kg coordinates. Polygon validation rejects zero edges and intersections.
fn cross([ax, ay]: [f64; 2], [bx, by]: [f64; 2], [cx, cy]: [f64; 2]) -> f64 {
    (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
}
fn on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    let [ax, ay] = a;
    let [bx, by] = b;
    let [px, py] = p;
    cross(a, b, p).abs() <= 1e-9
        && px >= ax.min(bx)
        && px <= ax.max(bx)
        && py >= ay.min(by)
        && py <= ay.max(by)
}
fn edges(polygon: &[[f64; 2]]) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
    polygon
        .iter()
        .copied()
        .zip(polygon.iter().copied().cycle().skip(1))
        .take(polygon.len())
}
fn contains(polygon: &[[f64; 2]], point: [f64; 2]) -> bool {
    let [x, y] = point;
    let mut inside = false;
    for (a, b) in edges(polygon) {
        if on_segment(a, b, point) {
            return true;
        }
        let [ax, ay] = a;
        let [bx, by] = b;
        if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
            inside = !inside;
        }
    }
    inside
}

impl Variant {
    fn validate(&self) -> Result<(), AircraftError> {
        for (field, reference) in [
            ("family", &self.family),
            ("engine", &self.engine),
            ("systems", &self.systems),
        ] {
            id(reference, field)?;
        }
        positive(self.empty_mass.mass_kg, "empty_mass.mass_kg")?;
        positive(self.min_mass_kg, "min_mass_kg")?;
        require(
            self.max_mass_kg >= self.min_mass_kg,
            "max_mass_kg",
            "must be at least min_mass_kg",
        )?;
        require(
            self.fuel_capacity_kg >= 0.0,
            "fuel_capacity_kg",
            "must be nonnegative",
        )?;
        require(
            (3..=64).contains(&self.cg_envelope.len()),
            "cg_envelope",
            "requires 3 to 64 vertices",
        )?;
        let area: f64 = edges(&self.cg_envelope)
            .map(|([ax, ay], [bx, by])| ax * by - bx * ay)
            .sum();
        require(area.abs() > 1e-9, "cg_envelope", "polygon must have nonzero area")?;
        for (i, (a, b)) in edges(&self.cg_envelope).enumerate() {
            require(a != b, "cg_envelope", "zero length edge")?;
            for (j, (c, d)) in edges(&self.cg_envelope).enumerate().skip(i + 1) {
                if j == i + 1 || (i == 0 && j + 1 == self.cg_envelope.len()) {
                    continue;
                }
                let crosses = cross(a, b, c) * cross(a, b, d) < 0.0 && cross(c, d, a) * cross(c, d, b) < 0.0;
                require(
                    !crosses
                        && !on_segment(a, b, c)
                        && !on_segment(a, b, d)
                        && !on_segment(c, d, a)
                        && !on_segment(c, d, b),
                    "cg_envelope",
                    "polygon must be simple",
                )?;
            }
        }
        require(
            (2..=32).contains(&self.flap_stages.len()),
            "flap_stages",
            "requires 2 to 32 stages",
        )?;
        for stage in &self.flap_stages {
            positive(stage.max_speed_mps, "flap_stages.max_speed_mps")?;
        }
        require(self.stations.len() <= 64, "stations", "at most 64 stations")?;
        let mut seen = BTreeSet::new();
        for station in &self.stations {
            id(&station.id, "stations.id")?;
            require(seen.insert(&station.id), "stations.id", "duplicate station")?;
            positive(station.max_mass_kg, "stations.max_mass_kg")?;
        }
        let v = &self.v_speeds;
        for speed in [
            v.vs_mps, v.vso_mps, v.vr_mps, v.vx_mps, v.vy_mps, v.vfe_mps, v.vno_mps, v.vne_mps,
        ] {
            positive(speed, "v_speeds")?;
        }
        require(
            v.vso_mps <= v.vs_mps
                && v.vs_mps <= v.vr_mps
                && v.vr_mps <= v.vne_mps
                && v.vx_mps <= v.vy_mps
                && v.vy_mps <= v.vne_mps
                && v.vfe_mps <= v.vno_mps
                && v.vno_mps <= v.vne_mps,
            "v_speeds",
            "speeds must be ordered",
        )
    }

    pub fn weight_balance(&self, load: &Load) -> Result<WeightBalance, AircraftError> {
        LayerFile::Variant(self.clone()).validate()?;
        validate_load(load)?;
        let mut mass = self.empty_mass.mass_kg;
        let mut moment = self.empty_mass.cg_m.map(|x| x * mass);
        let mut fuel = 0.0;
        let mut violations = Vec::new();
        for entry in &load.stations {
            let station = self
                .stations
                .iter()
                .find(|s| s.id == entry.station)
                .ok_or_else(|| {
                    error(
                        "load.stations.station",
                        format!("unknown station {}", entry.station),
                    )
                })?;
            mass += entry.mass_kg;
            for (m, arm) in moment.iter_mut().zip(station.position_m) {
                *m += arm * entry.mass_kg;
            }
            if entry.mass_kg > station.max_mass_kg {
                violations.push(format!("station {} exceeds its mass limit", station.id));
            }
            if station.kind == StationKind::Fuel {
                fuel += entry.mass_kg;
            }
        }
        if fuel > self.fuel_capacity_kg {
            violations.push("fuel capacity exceeded".into());
        }
        let cg = moment.map(|m| m / mass);
        if mass < self.min_mass_kg || mass > self.max_mass_kg {
            violations.push("mass outside variant limits".into());
        }
        let [x, _, _] = cg;
        if !contains(&self.cg_envelope, [x, mass]) {
            violations.push("CG outside envelope".into());
        }
        let result = WeightBalance {
            mass_kg: mass,
            weight_n: mass * 9.80665,
            cg_m: cg,
            in_limits: violations.is_empty(),
            violations,
        };
        numeric(&result)?;
        Ok(result)
    }

    /// Recompute mass, CG, and inertia for a validated station load, including depleted fuel.
    pub fn loaded_mass(&self, load: &Load) -> Result<Mass, AircraftError> {
        let balance = self.weight_balance(load)?;
        Ok(self.mass(load, &balance))
    }

    fn mass(&self, load: &Load, balance: &WeightBalance) -> Mass {
        let mut tensor = self.empty_mass.inertia_kg_m2;
        // Parallel-axis theorem: empty body and point station masses, all about the loaded CG.
        let bodies = std::iter::once((self.empty_mass.mass_kg, self.empty_mass.cg_m)).chain(
            load.stations.iter().filter_map(|e| {
                self.stations
                    .iter()
                    .find(|s| s.id == e.station)
                    .map(|s| (e.mass_kg, s.position_m))
            }),
        );
        for (mass, position) in bodies {
            let mut delta = [0.0; 3];
            for ((d, p), c) in delta.iter_mut().zip(position).zip(balance.cg_m) {
                *d = p - c;
            }
            let r2: f64 = delta.iter().map(|d| d * d).sum();
            for (i, (row, di)) in tensor.iter_mut().zip(delta).enumerate() {
                for (j, (cell, dj)) in row.iter_mut().zip(delta).enumerate() {
                    *cell += mass * (if i == j { r2 } else { 0.0 } - di * dj);
                }
            }
        }
        Mass {
            mass_kg: balance.mass_kg,
            cg_m: balance.cg_m,
            inertia_kg_m2: tensor,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AircraftCatalog {
    files: BTreeMap<(FileKind, String), (LayerFile, DefinitionSource)>,
}

impl AircraftCatalog {
    pub fn builtins() -> Result<Self, AircraftError> {
        let mut catalog = Self::default();
        for text in BUILTIN_LAYERS {
            catalog.insert(LayerFile::from_json(text)?, DefinitionSource::BuiltIn)?;
        }
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn insert(&mut self, file: LayerFile, source: DefinitionSource) -> Result<(), AircraftError> {
        file.validate()?;
        let key = (file.kind(), file.identity().id.clone());
        require(
            !self.files.contains_key(&key),
            "identity.id",
            "definition already installed; packs cannot replace files",
        )?;
        self.files.insert(key, (file, source));
        Ok(())
    }

    pub fn get(&self, kind: FileKind, name: &str) -> Result<&LayerFile, AircraftError> {
        self.files
            .get(&(kind, name.into()))
            .map(|(f, _)| f)
            .ok_or_else(|| error(format!("{kind:?}"), format!("unresolved reference {name}")))
    }

    pub fn source(&self, kind: FileKind, name: &str) -> Option<&DefinitionSource> {
        self.files.get(&(kind, name.into())).map(|(_, source)| source)
    }

    pub fn aircraft_definitions(&self) -> impl Iterator<Item = (&AircraftDefinition, &DefinitionSource)> {
        // BTreeMap iteration keeps list output deterministic.
        self.files.values().filter_map(|(f, source)| {
            if let LayerFile::Aircraft(a) = f {
                Some((a, source))
            } else {
                None
            }
        })
    }

    pub fn variant(&self, name: &str) -> Result<&Variant, AircraftError> {
        match self.get(FileKind::Variant, name)? {
            LayerFile::Variant(v) => Ok(v),
            _ => Err(error("variant", "wrong file kind")),
        }
    }

    pub fn resolve_aircraft(&self, name: &str) -> Result<ResolvedAircraft, AircraftError> {
        match self.get(FileKind::Aircraft, name)? {
            LayerFile::Aircraft(a) => self.resolve_definition(a),
            _ => Err(error("aircraft", "wrong file kind")),
        }
    }

    pub fn resolve_definition(&self, a: &AircraftDefinition) -> Result<ResolvedAircraft, AircraftError> {
        LayerFile::Aircraft(a.clone()).validate()?;
        let mut resolved = self.resolve(&a.variant, &a.panel, &a.load)?;
        resolved.aircraft.identity = a.identity.clone();
        Ok(resolved)
    }

    pub fn resolve(
        &self,
        variant: &str,
        panel: &str,
        load: &Load,
    ) -> Result<ResolvedAircraft, AircraftError> {
        let v = self.variant(variant)?;
        let LayerFile::Panel(p) = self.get(FileKind::Panel, panel)? else {
            return Err(error("panel", "wrong file kind"));
        };
        let aircraft = self.resolve_model(v, load)?;
        let LayerFile::Engine(e) = self.get(FileKind::Engine, &v.engine)? else {
            return Err(error("engine", "wrong file kind"));
        };
        let LayerFile::Systems(s) = self.get(FileKind::Systems, &v.systems)? else {
            return Err(error("systems", "wrong file kind"));
        };
        let electronic = ["electronic_attitude", "electronic_hsi"];
        if p.instruments
            .iter()
            .any(|i| electronic.contains(&i.module.as_str()))
        {
            let d = s
                .definition
                .as_ref()
                .ok_or_else(|| error("panel", "electronic instruments require operational systems"))?;
            for module in electronic {
                require(
                    p.instruments.iter().any(|i| i.module == module),
                    "panel",
                    "install both electronic standby modules",
                )?;
                require(
                    d.electrical.loads.iter().any(|l| {
                        l.id == module && l.bus == crate::Bus::Essential && l.backup_capacity_ah == 0.
                    }),
                    "panel",
                    "standby modules require individual essential-bus loads; backup batteries belong to the units",
                )?;
            }
        }
        Ok(ResolvedAircraft {
            aircraft,
            variant: v.clone(),
            engine: e.clone(),
            installed_systems: s.clone(),
            panel: p.clone(),
            load: load.clone(),
            weight_balance: v.weight_balance(load)?,
        })
    }

    fn resolve_model(&self, v: &Variant, load: &Load) -> Result<Aircraft, AircraftError> {
        let LayerFile::Family(f) = self.get(FileKind::Family, &v.family)? else {
            return Err(error("family", "wrong file kind"));
        };
        let LayerFile::Engine(e) = self.get(FileKind::Engine, &v.engine)? else {
            return Err(error("engine", "wrong file kind"));
        };
        let LayerFile::Systems(s) = self.get(FileKind::Systems, &v.systems)? else {
            return Err(error("systems", "wrong file kind"));
        };
        if let Some(d) = &s.definition {
            for t in &d.tanks {
                require(
                    v.stations
                        .iter()
                        .any(|s| s.id == t.station && s.kind == StationKind::Fuel),
                    "tank.station",
                    "must reference a fuel station",
                )?;
            }
            require(
                (d.tanks.iter().map(|t| t.capacity_kg).sum::<f64>() - v.fuel_capacity_kg).abs() < 1e-6,
                "tanks",
                "capacity must match variant fuel capacity",
            )?;
            for station in v.stations.iter().filter(|s| s.kind == StationKind::Fuel) {
                require(
                    d.tanks.iter().any(|t| t.station == station.id),
                    "tank.station",
                    "every fuel station needs a tank",
                )?;
            }
        }
        let balance = v.weight_balance(load)?;
        let aircraft = Aircraft {
            format_version: 1,
            identity: v.identity.clone(),
            geometry: f.geometry.clone(),
            mass: v.mass(load, &balance),
            aerodynamics: f.aerodynamics.clone(),
            propulsion: e.propulsion.clone(),
            systems: f
                .controls
                .resolve(v.flap_stages.iter().map(|s| s.position).collect()),
        };
        aircraft.validate()?;
        Ok(aircraft)
    }

    /// All installed documents, including unused parts, pass the same physical checks as built-ins.
    pub fn validate(&self) -> Result<(), AircraftError> {
        let baseline = Aircraft::from_json(crate::CORVID_JSON)?;
        for (file, _) in self.files.values() {
            file.validate()?;
            match file {
                LayerFile::Aircraft(a) => {
                    self.resolve_definition(a)?;
                }
                LayerFile::Variant(v) => {
                    self.resolve_model(v, &Load::default())?;
                }
                LayerFile::Family(f) => {
                    let mut model = baseline.clone();
                    model.geometry = f.geometry.clone();
                    model.aerodynamics = f.aerodynamics.clone();
                    let [lo, hi] = f.controls.flaps;
                    model.systems = f.controls.resolve(vec![lo, hi]);
                    // A family has no CG. Put the validation body's CG above every contact.
                    model.mass.cg_m = [
                        0.0,
                        0.0,
                        f.geometry
                            .gear
                            .iter()
                            .map(|g| {
                                let [_, _, z] = g.point_m;
                                z
                            })
                            .fold(0.0, f64::min)
                            - 1.0,
                    ];
                    model.validate()?;
                }
                LayerFile::Engine(e) => {
                    let mut model = baseline.clone();
                    model.propulsion = e.propulsion.clone();
                    model.validate()?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Lossless flight-model migration. Version 1 did not describe occupants or equipment, so the
/// fixed mass becomes empty mass, with no load. Limits and equipment are explicit design defaults.
pub fn migrate_v1(text: &str) -> Result<Vec<LayerFile>, AircraftError> {
    require(
        crate::definition_version(text)? == 1,
        "format_version",
        "migration requires version 1",
    )?;
    let a = Aircraft::from_json(text)?;
    let ident = &a.identity;
    let make_identity = |suffix: &str| Identity {
        id: format!("{}-{suffix}", ident.id),
        ..ident.clone()
    };
    let s = &a.systems;
    let family = Family {
        format_version: 2,
        identity: make_identity("family"),
        geometry: a.geometry.clone(),
        aerodynamics: a.aerodynamics.clone(),
        controls: AirframeControls {
            elevator: s.elevator,
            aileron: s.aileron,
            rudder: s.rudder,
            throttle: s.throttle,
            flaps: s.flaps,
            brake: s.brake,
            gear_travel_m: s.gear_travel_m,
            max_normal_force_n: s.max_normal_force_n,
            hard_touchdown_mps: s.hard_touchdown_mps,
            rolling_pavement: s.rolling_pavement,
            rolling_other: s.rolling_other,
            brake_friction: s.brake_friction,
            lateral_friction: s.lateral_friction,
        },
    };
    let [x, _, _] = a.mass.cg_m;
    let low = a.mass.mass_kg * 0.5;
    let high = a.mass.mass_kg * 1.2;
    let variant = Variant {
        format_version: 2,
        identity: make_identity("standard"),
        family: family.identity.id.clone(),
        engine: make_identity("engine").id,
        systems: make_identity("systems").id,
        empty_mass: a.mass.clone(),
        min_mass_kg: low,
        max_mass_kg: high,
        cg_envelope: vec![[x - 0.5, low], [x + 0.5, low], [x + 0.5, high], [x - 0.5, high]],
        flap_stages: s
            .flap_stages
            .iter()
            .map(|position| FlapStage {
                position: *position,
                max_speed_mps: 45.0,
            })
            .collect(),
        fuel_capacity_kg: 0.0,
        stations: vec![],
        v_speeds: VSpeeds {
            vs_mps: 27.0,
            vso_mps: 23.0,
            vr_mps: 29.0,
            vx_mps: 33.0,
            vy_mps: 36.0,
            vfe_mps: 45.0,
            vno_mps: 55.0,
            vne_mps: 65.0,
        },
    };
    let engine = Engine {
        format_version: 2,
        identity: make_identity("engine"),
        propulsion: a.propulsion.clone(),
        induction: Induction::Carburettor,
        fuel_flow_kg_s: Table {
            points: vec![(0.0, 0.0), (1.0, 0.01)],
            source: "Original migration placeholder; no fuel burn yet.".into(),
        },
    };
    let systems = InstalledSystems {
        definition: None,
        format_version: 2,
        identity: make_identity("systems"),
        installed: vec![
            "electrical".into(),
            "vacuum".into(),
            "pitot_static".into(),
            "fuel".into(),
        ],
    };
    let panel = Panel {
        format_version: 2,
        identity: make_identity("six"),
        instruments: INSTRUMENT_MODULES
            .iter()
            .take(6)
            .zip(0_u32..)
            .map(|(module, n)| InstrumentInstallation {
                id: (*module).into(),
                module: (*module).into(),
                skin: default_skin(),
                position: [f64::from(n % 3), f64::from(n / 3)],
                size: [1.0, 1.0],
            })
            .collect(),
    };
    let definition = AircraftDefinition {
        format_version: 2,
        identity: ident.clone(),
        variant: variant.identity.id.clone(),
        panel: panel.identity.id.clone(),
        load: Load::default(),
    };
    Ok(vec![
        LayerFile::Family(Box::new(family)),
        LayerFile::Variant(variant),
        LayerFile::Engine(engine),
        LayerFile::Systems(systems),
        LayerFile::Panel(panel),
        LayerFile::Aircraft(definition),
    ])
}

const BUILTIN_LAYERS: &[&str] = &[
    include_str!("../../../aircraft/corvid/family.json"),
    include_str!("../../../aircraft/corvid/variant.json"),
    include_str!("../../../aircraft/corvid/engine.json"),
    include_str!("../../../aircraft/corvid/systems.json"),
    include_str!("../../../aircraft/corvid/panel.json"),
    include_str!("../../../aircraft/corvid/aircraft.json"),
    include_str!("../../../aircraft/corvid/standby-systems.json"),
    include_str!("../../../aircraft/corvid/standby-variant.json"),
    include_str!("../../../aircraft/corvid/standby-pair.json"),
    include_str!("../../../aircraft/corvid/standby-aircraft.json"),
    include_str!("../../../aircraft/larrow/family.json"),
    include_str!("../../../aircraft/larrow/variant.json"),
    include_str!("../../../aircraft/larrow/engine.json"),
    include_str!("../../../aircraft/larrow/systems.json"),
    include_str!("../../../aircraft/larrow/panel.json"),
    include_str!("../../../aircraft/larrow/aircraft.json"),
];

impl AircraftCatalog {
    pub(crate) fn pack_ids(&self) -> BTreeSet<&str> {
        self.files
            .values()
            .filter_map(|(_, source)| match source {
                DefinitionSource::Pack { id } => Some(id.as_str()),
                DefinitionSource::BuiltIn => None,
            })
            .collect()
    }
}
