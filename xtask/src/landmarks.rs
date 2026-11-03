//! GNIS Domestic Names importer. Raw state files remain outside the repository.
use flightcraft_world::{GeoPoint, Landmark, LandmarkSource, Landmarks, Region};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
// GNIS supplies point features, not stream lengths. Keep a deliberately short major-river list.
const STREAMS: &[&str] = &[
    "Delaware River",
    "Hudson River",
    "Lehigh River",
    "Susquehanna River",
    "Schuylkill River",
    "Lackawanna River",
    "Passaic River",
    "Raritan River",
    "Musconetcong River",
    "Pequest River",
    "Paulins Kill",
    "Wallkill River",
];
// GNIS has no lake area or population field. This explicit selection avoids guessing size
// from a name, while keeping minor ponds and hamlets in the reference data only.
fn prominence(name: &str, class: &str) -> Option<u8> {
    match class {
        "Gap" => Some(0),
        "Summit" if name.starts_with("Mount ") || name.ends_with(" Mountain") => Some(1),
        "Ridge" => Some(1),
        "Stream" if STREAMS.contains(&name) => Some(2),
        "Lake" | "Reservoir"
            if [
                "Lake Wallenpaupack",
                "Lake Hopatcong",
                "Round Valley Reservoir",
                "Merrill Creek Reservoir",
                "Lake Nockamixon",
                "Beltzville Lake",
                "Green Lane Reservoir",
                "Lake Harmony",
                "Blue Marsh Lake",
                "Spruce Run Reservoir",
                "Tobyhanna Lake",
                "Lake Naomi",
                "Stillwater Lake",
                "Culvers Lake",
                "Swartswood Lake",
            ]
            .contains(&name) =>
        {
            Some(3)
        }
        "Populated Place"
            if [
                "Blairstown",
                "Stroudsburg",
                "East Stroudsburg",
                "Allentown",
                "Bethlehem",
                "Easton",
                "Jim Thorpe",
                "Lehighton",
                "Palmerton",
                "Washington",
                "Hackettstown",
                "Newton",
                "Milford",
                "Port Jervis",
                "Mount Pocono",
                "Scranton",
                "Wilkes-Barre",
                "New York",
                "Jersey City",
                "Newark",
                "Trenton",
                "Doylestown",
                "Quakertown",
                "Phillipsburg",
                "Nazareth",
            ]
            .contains(&name) =>
        {
            Some(4)
        }
        _ => None,
    }
}
pub fn extract(region: &Region, directory: &Path, out: &Path) -> Result<(), String> {
    let [west, south, east, north] = super::region::grid_bounds(region)?;
    let mut paths: Vec<PathBuf> = std::fs::read_dir(directory)
        .map_err(|e| e.to_string())?
        .map(|e| e.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    paths.retain(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("DomesticNames_") && n.ends_with(".txt"))
    });
    paths.sort();
    if paths.is_empty() || paths.len() > 60 {
        return Err("GNIS requires 1–60 DomesticNames state files".into());
    }
    let mut features = BTreeMap::new();
    let mut sources = Vec::new();
    for path in paths {
        let bytes = super::download::read(&path, 256 * 1024 * 1024)?;
        sources.push(LandmarkSource {
            file: path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("GNIS filename")?
                .into(),
            sha256: super::download::sha256(&bytes),
            bytes: bytes.len() as u64,
        });
        let mut lines = BufReader::new(bytes.as_slice()).lines();
        let header = lines
            .next()
            .ok_or("GNIS header missing")?
            .map_err(|e| e.to_string())?;
        let columns: Vec<_> = header.trim_start_matches('\u{feff}').split('|').collect();
        let column = |name| {
            columns
                .iter()
                .position(|c| *c == name)
                .ok_or_else(|| format!("GNIS column missing: {name}"))
        };
        let (id, name, class, state, lat, lon) = (
            column("feature_id")?,
            column("feature_name")?,
            column("feature_class")?,
            column("state_name")?,
            column("prim_lat_dec")?,
            column("prim_long_dec")?,
        );
        for line in lines {
            let line = line.map_err(|e| e.to_string())?;
            if line.len() > 8192 {
                return Err("GNIS record too long".into());
            }
            let fields: Vec<_> = line.split('|').collect();
            let field = |i| fields.get(i).copied().ok_or("truncated GNIS record");
            let (name, class) = (field(name)?, field(class)?);
            if ![
                "Summit",
                "Gap",
                "Ridge",
                "Lake",
                "Reservoir",
                "Stream",
                "Populated Place",
            ]
            .contains(&class)
                || name.contains("(historical)")
                || class == "Stream" && !STREAMS.contains(&name)
            {
                continue;
            }
            let latitude_deg = field(lat)?.parse::<f64>().map_err(|e| e.to_string())?;
            let longitude_deg = field(lon)?.parse::<f64>().map_err(|e| e.to_string())?;
            // Retain the Delaware River's GNIS primary (mouth) point as context for the
            // Water Gap description; never move a river's point onto the flight route.
            let feature_id = field(id)?.parse::<u32>().map_err(|e| e.to_string())?;
            if feature_id != 217887
                && (!(south..=north).contains(&latitude_deg) || !(west..=east).contains(&longitude_deg))
            {
                continue;
            }
            let feature_id = field(id)?.parse::<u32>().map_err(|e| e.to_string())?;
            features.entry(feature_id).or_insert(Landmark {
                feature_id,
                prominence: prominence(name, class),
                description: None,
                name: name.into(),
                feature_class: class.into(),
                state: field(state)?.into(),
                position: GeoPoint {
                    latitude_deg,
                    longitude_deg,
                },
            });
        }
    }
    // Original summaries use GNIS names and primary positions from the same output.
    if [217887, 1173129, 881083, 1213628]
        .iter()
        .all(|id| features.contains_key(id))
        && let Some(gap) = features.get_mut(&1173129)
    {
        gap.description = Some("The Delaware River cuts through Kittatinny Mountain here. Mount Tammany is on the New Jersey side.".into());
    }
    let data = Landmarks {version:1,attribution:"USGS Geographic Names Information System, Domestic Names. U.S. government public domain. Stream selection: documented major-river name list; GNIS primary point positions, not river geometry. Prominence: gaps, mountains/ridges, major-river list, curated large lakes/reservoirs and towns. Original geographic summaries; the Delaware River mouth point is retained for Water Gap context.".into(),sources,features:features.into_values().collect()};
    data.validate().map_err(|e| e.to_string())?;
    super::download::write_json(out, &data)
}
pub fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(2);
    let mut values = BTreeMap::new();
    while let Some(key) = args.next() {
        if !["--region", "--gnis", "--out"].contains(&key.as_str()) || values.contains_key(&key) {
            return Err("landmarks --region region.json --gnis DIRECTORY --out landmarks.json".into());
        }
        values.insert(key, args.next().ok_or("missing landmarks argument")?);
    }
    let required = |name| {
        values
            .get(name)
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing {name}"))
    };
    let region: Region = super::download::json(&required("--region")?)?;
    extract(&region, &required("--gnis")?, &required("--out")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gnis_extraction_filters_classes_bounds_and_retains_source_ids() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let region: Region =
            super::super::download::json(&root.join("assets/terrain/eastern-pa/region.json")).unwrap();
        let tmp = std::env::temp_dir().join(format!("gnis-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let text = "feature_id|feature_name|feature_class|state_name|prim_lat_dec|prim_long_dec\n1|Test Mountain|Summit|New Jersey|40.98|-75.1\n2|Minor Brook|Stream|New Jersey|40.98|-75.1\n3|Hudson River|Stream|New York|40.98|-75.1\n4|Distant Summit|Summit|New Jersey|30|-90\n5|Test Church|Church|New Jersey|40.98|-75.1\n";
        std::fs::write(tmp.join("DomesticNames_NJ.txt"), text).unwrap();
        extract(&region, &tmp, &tmp.join("landmarks.json")).unwrap();
        let data: Landmarks = super::super::download::json(&tmp.join("landmarks.json")).unwrap();
        assert_eq!(
            data.features.iter().map(|f| f.feature_id).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(
            data.sources[0].sha256,
            super::super::download::sha256(text.as_bytes())
        );
        std::fs::remove_dir_all(tmp).unwrap();
    }
}

#[cfg(test)]
mod prominence_tests {
    use super::prominence;
    #[test]
    fn scenic_selection_orders_notable_features_and_excludes_minor_water() {
        assert_eq!(prominence("Delaware Water Gap", "Gap"), Some(0));
        assert_eq!(prominence("Mount Tammany", "Summit"), Some(1));
        assert_eq!(prominence("Lehigh River", "Stream"), Some(2));
        assert_eq!(prominence("Lake Wallenpaupack", "Reservoir"), Some(3));
        assert_eq!(prominence("Stroudsburg", "Populated Place"), Some(4));
        assert_eq!(prominence("Zacharias Pond", "Lake"), None);
        assert_eq!(prominence("Stoefflet Pond", "Reservoir"), None);
        assert_eq!(prominence("Minor Brook", "Stream"), None);
    }
}
