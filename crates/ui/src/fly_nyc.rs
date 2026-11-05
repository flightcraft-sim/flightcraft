//! The website's default flight: "Fly NYC".
//!
//! The site ships one polished showcase (docs/scenery-sources.md): the Hudson River corridor and
//! the Manhattan skyline, built from public 3DEP lidar, NAIP and NHD (docs/scenery.md, "Hudson
//! corridor"). With no query the New York region opens at the George Washington Bridge at about
//! 1,000 ft, southbound down the river with the whole skyline ahead.
//!
//! Why the air start and not Teterboro: KTEB lies about 9 km west of the Hudson and outside the
//! lidar envelope, so a departure spends its first minutes over New Jersey with flat NAIP and no
//! buildings. The bridge start puts the measured skyline in the first frame. Departing KTEB stays
//! one query away (`?start=kteb`), and `?start=runway` keeps the original 1N7 runway start.
use serde_json::{Value, json};

/// Same-origin path of the accepted Hudson corridor tileset on the website.
pub const FLY_NYC_TILESET: &str = "terrain/new-york/scenery/hudson-corridor/tileset.json";
/// George Washington Bridge, mid-span (latitude, longitude).
pub const GWB: [f64; 2] = [40.8500, -73.9519];
/// Start about 0.9 km north of the span so the bridge is in the opening view.
const START: [f64; 2] = [40.8585, -73.9519];
const AGL_M: f64 = 300.;
const AIRSPEED_MPS: f64 = 40.;

/// The `sim.reset` start for a region and an optional `?start=` request.
pub fn start(region: &str, request: Option<&str>) -> Value {
    match (region, request) {
        (_, Some("runway")) => json!({"kind":"runway"}),
        ("new-york", Some("kteb" | "teterboro")) => {
            json!({"kind":"airport_runway","airport":"KTEB","end":"06"})
        }
        ("new-york", _) => json!({
            "kind":"geographic_air",
            "position":{"latitude_deg":START[0],"longitude_deg":START[1]},
            "height_agl_m":AGL_M,
            "airspeed_mps":AIRSPEED_MPS,
            // South, down the river.
            "heading_rad":std::f64::consts::PI
        }),
        _ => json!({"kind":"runway"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_york_opens_southbound_at_the_bridge_and_other_regions_keep_the_runway() {
        let v = start("new-york", None);
        assert_eq!(v["kind"], "geographic_air");
        assert!((v["heading_rad"].as_f64().unwrap() - std::f64::consts::PI).abs() < 1e-12);
        let (lat, lon) = (
            v["position"]["latitude_deg"].as_f64().unwrap(),
            v["position"]["longitude_deg"].as_f64().unwrap(),
        );
        // Within a kilometre of the bridge and north of it.
        assert!(lat > GWB[0] && lat - GWB[0] < 0.01 && (lon - GWB[1]).abs() < 0.01);
        assert_eq!(start("new-york", Some("kteb"))["airport"], "KTEB");
        assert_eq!(start("new-york", Some("runway"))["kind"], "runway");
        assert_eq!(start("eastern-pa", None)["kind"], "runway");
        assert_eq!(start("eastern-pa", Some("kteb"))["kind"], "runway");
    }
    #[test]
    fn the_tileset_path_is_the_one_the_website_validator_accepts() {
        assert!(FLY_NYC_TILESET.ends_with("scenery/hudson-corridor/tileset.json"));
        assert!(flightcraft_world::tiles3d::local_uri(FLY_NYC_TILESET));
    }
}
