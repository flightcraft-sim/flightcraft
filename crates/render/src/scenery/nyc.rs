//! Fly NYC consumes reconstructed USGS buildings as OGC 3D Tiles. Reconstruction belongs to
//! fc-scenery; these types define the area, provenance, and static-provider handoff only.
use flightcraft_world::tiles3d::Tileset;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const NYC_BOUNDS: [f64; 4] = [-74.04, 40.66, -73.90, 40.84];
pub const NYC_BUILDING_BUDGET: u64 = 250_000_000;
pub const WEB_BUDGET: u64 = 1_000_000_000;
pub const NYC_EPT: &str = "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/NY_NewYorkCity/ept.json";
const EPT_PREFIX: &str = "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/NY_NewYorkCity/";
pub const NYC_ATTRIBUTION: &str = "Buildings: USGS 3DEP, NY_NewYorkCity classified lidar, public domain (US government data). Reconstructed by FlightCraft; original procedural materials. For simulation only.";

/// Geographic order is west, south, east, north, in degrees. Source cube order is
/// xmin, ymin, zmin, xmax, ymax, zmax, in the EPT source frame; padding is not ground elevation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlyNycArea {
    pub version: u8,
    pub region: String,
    pub area: String,
    pub bounds: [f64; 4],
    pub building_budget_bytes: u64,
    pub total_web_budget_bytes: u64,
    pub ept: String,
    pub source_crs: String,
    pub source_cube_bounds: [f64; 6],
    pub source_points: u64,
    pub classification: u8,
}
pub fn fly_nyc_area() -> FlyNycArea {
    FlyNycArea {
        version: 1,
        region: "new-york".into(),
        area: "nyc-lidar".into(),
        bounds: NYC_BOUNDS,
        building_budget_bytes: NYC_BUILDING_BUDGET,
        total_web_budget_bytes: WEB_BUDGET,
        ept: NYC_EPT.into(),
        source_crs: "EPSG:3857".into(),
        source_cube_bounds: [
            -8_267_099.,
            4_937_860.,
            -31_153.,
            -8_204_119.,
            5_000_840.,
            31_827.,
        ],
        source_points: 4_755_025_996,
        classification: 6,
    }
}
/// Same URL/edition/date/byte-count/SHA-256 fields as acquisition receipts. No credentials.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NycLidarReceipt {
    pub url: String,
    pub edition: Option<String>,
    pub downloaded_at: String,
    pub bytes: u64,
    pub sha256: String,
}
/// Lives at tileset.extras.flightcraft.nycLidar. Input leaves each contain one complete
/// building GLB in an explicit ECEF transform; no Socrata, TUM, or external footprint input.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NycLidarSource {
    pub selection: FlyNycArea,
    pub reconstruction_buildings: u64,
    pub reconstruction_leaves_are_buildings: bool,
    pub uses_external_footprints: bool,
    pub receipts: Vec<NycLidarReceipt>,
}
impl NycLidarSource {
    pub fn validate(&self) -> Result<(), String> {
        if self.selection != fly_nyc_area()
            || self.reconstruction_buildings == 0
            || self.reconstruction_buildings > 100_000
            || !self.reconstruction_leaves_are_buildings
            || self.uses_external_footprints
            || self.receipts.is_empty()
            || self.receipts.len() > 50_000
        {
            return Err(
                "Fly NYC requires classified 3DEP-only building reconstruction for its fixed showcase area"
                    .into(),
            );
        }
        let mut metadata = false;
        let mut points = false;
        for source in &self.receipts {
            let path = source
                .url
                .strip_prefix(EPT_PREFIX)
                .filter(|p| flightcraft_world::tiles3d::local_uri(p))
                .ok_or("Fly NYC provenance contains a source outside the reviewed USGS NYC EPT project")?;
            if source.bytes == 0
                || source.sha256.len() != 64
                || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || source.downloaded_at.is_empty()
                || source.downloaded_at.len() > 128
            {
                return Err("Fly NYC reconstruction needs complete source acquisition receipts".into());
            }
            metadata |= source.url == NYC_EPT;
            points |= path.starts_with("ept-data/") && path.ends_with(".laz");
        }
        if !metadata || !points {
            return Err("Fly NYC needs receipts for EPT metadata and acquired classified LAZ nodes".into());
        }
        Ok(())
    }
}
/// Permanent source restrictions also reject stale builds made before the NYC terms correction.
/// Geometric acceptance flags can never authorize the NYC portal or TUM for redistribution.
pub fn validate_static_scenery(set: &Tileset) -> Result<(), String> {
    if let Some(flags) = set.extras.as_ref().and_then(|v| v.get("flightcraft")) {
        validate_redistribution(flags)?;
    }
    Ok(())
}
pub fn validate_redistribution(flags: &Value) -> Result<(), String> {
    if flags.get("localOnly").and_then(Value::as_bool) == Some(true)
        || flags.get("redistributionAllowed").and_then(Value::as_bool) == Some(false)
        || matches!(
            flags.get("area").and_then(Value::as_str),
            Some("nyc-footprints" | "nyc-lod2" | "nyc-reviewed-glb")
        )
    {
        return Err(
            "Local-only NYC portal and TUM providers cannot be redistributed or loaded as website data"
                .into(),
        );
    }
    if flags.get("area").and_then(Value::as_str) == Some("nyc-lidar") {
        nyc_lidar_source(flags)?;
    }
    Ok(())
}
pub fn nyc_lidar_source(flags: &Value) -> Result<NycLidarSource, String> {
    if flags.get("region").and_then(Value::as_str) != Some("new-york")
        || flags.get("area").and_then(Value::as_str) != Some("nyc-lidar")
        || flags.get("sourceKind").and_then(Value::as_str) != Some("usgs-3dep-lidar")
        || flags.get("roofPolicy").and_then(Value::as_str) != Some("lidar-only")
        || flags.get("lidarBuilt").and_then(Value::as_bool) != Some(true)
        || flags.get("localOnly").and_then(Value::as_bool) != Some(false)
        || flags.get("redistributionAllowed").and_then(Value::as_bool) != Some(true)
    {
        return Err("Fly NYC ships lidar-derived buildings only; portal footprints and flat-roof substitutions are forbidden".into());
    }
    let source: NycLidarSource = serde_json::from_value(
        flags
            .get("nycLidar")
            .ok_or("Fly NYC lidar provenance is missing")?
            .clone(),
    )
    .map_err(|_| "Invalid Fly NYC lidar provenance")?;
    source.validate()?;
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn committed_nyc_area_matches_provider_contract() {
        let area: FlyNycArea =
            serde_json::from_str(include_str!("../../../../assets/scenery/fly-nyc.json")).unwrap();
        assert_eq!(area, fly_nyc_area());
    }
    #[test]
    fn stale_portal_and_tum_flags_cannot_authorize_website_use() {
        for area in ["nyc-footprints", "nyc-lod2", "nyc-reviewed-glb"] {
            let set: Tileset = serde_json::from_value(serde_json::json!({"asset":{"version":"1.1"},"geometricError":0,"root":{"boundingVolume":{"region":[-1.3,0.7,-1.2,0.8,0,100]},"geometricError":0,"content":{"uri":"test.glb"}},"extras":{"flightcraft":{"area":area,"localOnly":false,"redistributionAllowed":true,"previewOnly":false,"lidarBuilt":true,"verticalDatumVerified":true,"runwayAlignmentVerified":true}}})).unwrap();
            assert!(validate_static_scenery(&set).is_err());
        }
    }
}
