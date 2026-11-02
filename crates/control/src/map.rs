//! Presentation state and Web Mercator tile coordinates shared by map hosts and importers.
use crate::{Command, CommandError, Session};
use flightcraft_world::GeoPoint;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BaseMap {
    #[default]
    Aero,
    Terrain,
    Sectional,
    Tac,
    Satellite,
    Street,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MapUp {
    #[default]
    NorthUp,
    TrackUp,
    HeadingUp,
}
impl From<crate::MapOrientation> for MapUp {
    fn from(o: crate::MapOrientation) -> Self {
        match o {
            crate::MapOrientation::NorthUp => Self::NorthUp,
            crate::MapOrientation::TrackUp => Self::TrackUp,
            crate::MapOrientation::HeadingUp => Self::HeadingUp,
        }
    }
}
impl From<MapUp> for crate::MapOrientation {
    fn from(o: MapUp) -> Self {
        match o {
            MapUp::NorthUp => Self::NorthUp,
            MapUp::TrackUp => Self::TrackUp,
            MapUp::HeadingUp => Self::HeadingUp,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MapOverlay {
    Route,
    Airspace,
    TerrainWarnings,
    Airports,
    Navaids,
    Fixes,
    Airways,
    Pins,
    Aircraft,
    Track,
    RangeRings,
    LiveWeather,
    Tfr,
    Plates,
}
impl MapOverlay {
    pub const AVAILABLE: [Self; 11] = [
        Self::Route,
        Self::Airspace,
        Self::TerrainWarnings,
        Self::Airports,
        Self::Navaids,
        Self::Fixes,
        Self::Airways,
        Self::Pins,
        Self::Aircraft,
        Self::Track,
        Self::RangeRings,
    ];
    pub fn available(self) -> bool {
        Self::AVAILABLE.contains(&self)
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OverlayStyle {
    pub enabled: bool,
    #[schemars(range(min = 0, max = 1))]
    pub opacity: f32,
}
/// Declutter distances are the visible span across the map's horizontal dimension, in nautical miles.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapDeclutter {
    #[schemars(range(min = 0, max = 400))]
    pub fixes_across_nm: f64,
    #[schemars(range(min = 0, max = 400))]
    pub airway_labels_across_nm: f64,
}
impl Default for MapDeclutter {
    fn default() -> Self {
        Self {
            fixes_across_nm: 15.0,
            airway_labels_across_nm: 20.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapState {
    #[serde(default)]
    pub declutter: MapDeclutter,
    pub base_map: BaseMap,
    pub overlays: BTreeMap<MapOverlay, OverlayStyle>,
    pub orientation: MapUp,
    #[schemars(range(min = 0.25, max = 200))]
    pub range_nm: f64,
    /// Null follows the aircraft; a position pins the centre in the active region's frame.
    pub centre: Option<GeoPoint>,
    pub night: bool,
}
impl Default for MapState {
    fn default() -> Self {
        Self {
            declutter: MapDeclutter::default(),
            base_map: BaseMap::Aero,
            overlays: MapOverlay::AVAILABLE
                .into_iter()
                .map(|o| {
                    (
                        o,
                        OverlayStyle {
                            enabled: o != MapOverlay::TerrainWarnings,
                            opacity: 1.0,
                        },
                    )
                })
                .collect(),
            orientation: MapUp::NorthUp,
            range_nm: 40.0,
            centre: None,
            night: false,
        }
    }
}
impl MapState {
    pub fn opacity(&self, overlay: MapOverlay) -> f32 {
        self.overlays
            .get(&overlay)
            .filter(|v| v.enabled)
            .map_or(0.0, |v| v.opacity)
    }
    fn validate(&self) -> Result<(), CommandError> {
        let bad = |s: &str| CommandError::Failed(s.into());
        if matches!(self.base_map, BaseMap::Satellite | BaseMap::Street) {
            return Err(bad("base map coming soon"));
        }
        if !self.range_nm.is_finite() || !(0.25..=200.0).contains(&self.range_nm) {
            return Err(bad("map range must be 0.25–200 nm"));
        }
        for threshold in [
            self.declutter.fixes_across_nm,
            self.declutter.airway_labels_across_nm,
        ] {
            if !threshold.is_finite() || !(0.0..=400.0).contains(&threshold) {
                return Err(bad("map declutter spans must be 0–400 nm"));
            }
        }
        if let Some(p) = self.centre {
            p.validate().map_err(|e| bad(&e.to_string()))?;
        }
        for (key, value) in &self.overlays {
            if !value.opacity.is_finite() || !(0.0..=1.0).contains(&value.opacity) {
                return Err(bad("overlay opacity must be 0–1"));
            }
            if value.enabled && !key.available() {
                return Err(bad("overlay coming soon"));
            }
        }
        Ok(())
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetMap {
    #[serde(default)]
    pub declutter: Option<MapDeclutter>,
    #[serde(default)]
    pub base_map: Option<BaseMap>,
    /// Supplied entries replace those overlays; omitted entries retain their values.
    #[serde(default)]
    pub overlays: Option<BTreeMap<MapOverlay, OverlayStyle>>,
    #[serde(default)]
    pub orientation: Option<MapUp>,
    #[serde(default)]
    pub range_nm: Option<f64>,
    #[serde(default)]
    pub centre: Option<GeoPoint>,
    #[serde(default)]
    pub follow_aircraft: Option<bool>,
    #[serde(default)]
    pub night: Option<bool>,
}
impl Command for SetMap {
    const NAME: &'static str = "map.set";
    const DESCRIPTION: &'static str =
        "Set base map, overlay opacity, orientation, range, centre, and night palette atomically.";
    type Output = MapState;
    fn run(self, s: &mut Session) -> Result<MapState, CommandError> {
        let mut state = s.map.clone();
        if let Some(v) = self.declutter {
            state.declutter = v;
        }
        if let Some(v) = self.base_map {
            state.base_map = v;
        }
        if let Some(v) = self.overlays {
            state.overlays.extend(v);
        }
        if let Some(v) = self.orientation {
            state.orientation = v;
        }
        if let Some(v) = self.range_nm {
            state.range_nm = v;
        }
        if let Some(v) = self.centre {
            state.centre = Some(v);
        }
        if self.follow_aircraft == Some(true) {
            state.centre = None;
        }
        if let Some(v) = self.night {
            state.night = v;
        }
        state.validate()?;
        if let (Some(centre), Some(region)) = (state.centre, s.sim().world().region()) {
            flightcraft_world::geo::LocalProjection::new(
                region.origin.latitude_deg,
                region.origin.longitude_deg,
            )
            .and_then(|projection| projection.project(centre.latitude_deg, centre.longitude_deg))
            .map_err(|_| CommandError::Failed("map centre is outside the supported region frame".into()))?;
        }
        s.map = state.clone();
        s.glass.range_nm = state.range_nm.max(1.0);
        s.glass.orientation = state.orientation.into();
        Ok(state)
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetMap {}
impl Command for GetMap {
    const NAME: &'static str = "map.state";
    const DESCRIPTION: &'static str = "Read map presentation state without advancing the flight.";
    type Output = MapState;
    fn run(self, s: &mut Session) -> Result<MapState, CommandError> {
        Ok(s.map.clone())
    }
}
impl Session {
    pub fn map_state(&self) -> &MapState {
        &self.map
    }
    /// Resolved active GPS waypoints, used to keep route fixes visible through declutter.
    pub fn gps_map_waypoints(&self) -> &[crate::GpsWaypoint] {
        if self.glass.generation == self.flight_generation {
            &self.glass.waypoints
        } else {
            &[]
        }
    }
    pub fn flight_plan(&self) -> &crate::FlightPlan {
        &self.navigation.plan
    }
}
/// Normalised Web Mercator coordinates. Longitude 180 is the eastern edge, not tile zero.
pub fn mercator(p: GeoPoint) -> Option<[f64; 2]> {
    p.validate().ok()?;
    if p.latitude_deg.abs() > 85.0511287798066 {
        return None;
    }
    let phi = p.latitude_deg.to_radians();
    Some([
        (p.longitude_deg + 180.0) / 360.0,
        (1.0 - libm::asinh(libm::tan(phi)) / core::f64::consts::PI) * 0.5,
    ])
}
pub fn inverse_mercator([x, y]: [f64; 2]) -> Option<GeoPoint> {
    if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return None;
    }
    Some(GeoPoint {
        latitude_deg: libm::atan(libm::sinh(core::f64::consts::PI * (1.0 - 2.0 * y))).to_degrees(),
        longitude_deg: x * 360.0 - 180.0,
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartTile {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}
impl ChartTile {
    pub fn at(p: GeoPoint, z: u8) -> Option<Self> {
        if z > 20 {
            return None;
        }
        let [x, y] = mercator(p)?;
        let n = 1_u32 << z;
        Some(Self {
            z,
            x: ((x * f64::from(n)).floor() as u32).min(n - 1),
            y: ((y * f64::from(n)).floor() as u32).min(n - 1),
        })
    }
    pub fn valid(self) -> bool {
        self.z <= 20 && self.x < (1_u32 << self.z) && self.y < (1_u32 << self.z)
    }
    pub fn path(self) -> String {
        format!("{}/{}/{}.png", self.z, self.x, self.y)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartIndex {
    #[serde(default)]
    #[schemars(range(min = 0, max = 80))]
    pub coverage_margin_nm: f64,
    #[schemars(range(min = 1, max = 2))]
    pub version: u8,
    pub kind: BaseMap,
    pub edition: String,
    pub expires: String,
    pub attribution: String,
    pub min_zoom: u8,
    pub max_zoom: u8,
    pub tile_size: u32,
    pub tiles: Vec<ChartTile>,
    pub bytes: u64,
}
impl ChartIndex {
    /// Version 2 uses lossless WebP; version 1 PNG indexes remain supported.
    pub fn tile_path(&self, tile: ChartTile) -> String {
        if self.version == 2 {
            format!("{}/{}/{}.webp", tile.z, tile.x, tile.y)
        } else {
            tile.path()
        }
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("chart index exceeds 4 MiB".into());
        }
        let index: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if !index.coverage_margin_nm.is_finite()
            || !(0.0..=80.0).contains(&index.coverage_margin_nm)
            || !matches!(index.version, 1 | 2)
            || !matches!(index.kind, BaseMap::Sectional | BaseMap::Tac)
            || index.min_zoom > index.max_zoom
            || index.max_zoom > 16
            || index.tile_size != 256
            || index.tiles.len() > 20000
            || index
                .tiles
                .iter()
                .any(|t| !t.valid() || t.z < index.min_zoom || t.z > index.max_zoom)
            || !flightcraft_world::navdata::iso_date(&index.edition)
            || !flightcraft_world::navdata::iso_date(&index.expires)
            || !flightcraft_world::navdata::text_ok(&index.attribution, 2048)
            || index
                .tiles
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != index.tiles.len()
        {
            return Err("invalid chart index".into());
        }
        Ok(index)
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn projection_and_tile_edges() {
        for latitude_deg in [-85.0, -40.0, 0.0, 40.97, 85.0] {
            for longitude_deg in [-180.0, -75.0, 0.0, 180.0] {
                let p = GeoPoint {
                    latitude_deg,
                    longitude_deg,
                };
                let q = inverse_mercator(mercator(p).unwrap()).unwrap();
                assert!((p.latitude_deg - q.latitude_deg).abs() < 1e-10);
                assert!((p.longitude_deg - q.longitude_deg).abs() < 1e-10);
            }
        }
        assert_eq!(
            ChartTile::at(
                GeoPoint {
                    latitude_deg: 0.0,
                    longitude_deg: 0.0
                },
                1
            ),
            Some(ChartTile { z: 1, x: 1, y: 1 })
        );
        assert!(
            ChartTile::at(
                GeoPoint {
                    latitude_deg: 90.0,
                    longitude_deg: 0.0
                },
                1
            )
            .is_none()
        );
        assert!(inverse_mercator([f64::NAN, 0.0]).is_none());
        assert!(!ChartTile { z: 255, x: 0, y: 0 }.valid());
    }
    #[test]
    fn settings_are_atomic_and_registered() {
        let mut s = Session::new().unwrap();
        s.execute(
            "map.set",
            serde_json::json!({"base_map":"tac","range_nm":12,"orientation":"heading_up"}),
        )
        .unwrap();
        s.execute(
            "map.set",
            serde_json::json!({"declutter":{"fixes_across_nm":12,"airway_labels_across_nm":16}}),
        )
        .unwrap();
        let before = s.execute("map.state", serde_json::json!({})).unwrap();
        assert!(
            s.execute(
                "map.set",
                serde_json::json!({"declutter":{"fixes_across_nm":-1,"airway_labels_across_nm":16}})
            )
            .is_err()
        );
        assert!(
            s.execute(
                "map.set",
                serde_json::json!({"base_map":"satellite","range_nm":20})
            )
            .is_err()
        );
        assert!(
            s.execute(
                "map.set",
                serde_json::json!({"overlays":{"tfr":{"enabled":true,"opacity":0.4}}})
            )
            .is_err()
        );
        assert_eq!(before, s.execute("map.state", serde_json::json!({})).unwrap());
    }
    #[test]
    fn active_map_fixes_clear_with_the_gps_on_reset() {
        let mut s = Session::new().unwrap();
        s.set_gps_waypoints(vec![crate::GpsWaypoint {
            ident: "FIX".into(),
            kind: crate::WaypointKind::Fix,
            position: GeoPoint {
                latitude_deg: 40.8,
                longitude_deg: -74.7,
            },
        }])
        .unwrap();
        assert_eq!(s.gps_map_waypoints().len(), 1);
        s.execute(
            "sim.reset",
            serde_json::json!({"start":{"kind":"runway"},"paused":true}),
        )
        .unwrap();
        assert!(s.gps_map_waypoints().is_empty());
    }
}
