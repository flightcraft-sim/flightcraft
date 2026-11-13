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
const AIRSPEED_MPS: f64 = 52.;
/// Magnetic heading down the river at the bridge: about 205 degrees true, 12.5 degrees west variation.
const HEADING_MAGNETIC_DEG: f64 = 217.;

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
            "heading_rad":HEADING_MAGNETIC_DEG.to_radians()
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
        assert!((v["heading_rad"].as_f64().unwrap() - 217_f64.to_radians()).abs() < 1e-12);
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
    /// The opening flies itself while the player finds the controls: the elevator sits in the pitch
    /// trim, and the keyboard's stick-centering each frame leaves it level.
    #[test]
    fn the_opening_stays_level_hands_off_with_the_keyboard_centering_the_stick() {
        let mut s = flightcraft_control::Session::new().unwrap();
        s.execute(
            "sim.reset",
            json!({"start":{"kind":"air","height_m":AGL_M,"airspeed_mps":AIRSPEED_MPS,"heading_rad":3.},"paused":false}),
        )
        .unwrap();
        assert!(s.sim().systems().controls.trim != 0.);
        // The heading bug starts on the heading flown, not 000.
        let bug = s.sim().systems().controls.heading_bug_rad.to_degrees();
        let heading = s.sim().readout().heading_rad.to_degrees();
        let off = (bug - heading + 180.).rem_euclid(360.) - 180.;
        assert!(off.abs() < 1., "bug {bug} heading {heading}");
        let start = s.sim().readout().altitude_msl_m;
        for _ in 0..(120 * 120) {
            // What the web and desktop input does every frame with no key held.
            s.execute(
                "sim.set_controls",
                json!({"elevator":0.,"aileron":0.,"rudder":0.}),
            )
            .unwrap();
            s.execute("sim.step", json!({"steps":1})).unwrap();
            let r = s.sim().readout();
            assert!(!r.crashed);
            assert!(
                (r.altitude_msl_m - start).abs() < 30.,
                "{} m from level",
                r.altitude_msl_m - start
            );
        }
    }

    /// Fly me from the opening: the nearest airport must be a flight the planner accepts. Runs where
    /// the region data is installed, which is every CI job.
    #[test]
    fn fly_me_can_plan_to_teterboro_from_the_opening() {
        let Ok(region) =
            flightcraft_control::read_region(std::path::Path::new("../../assets/terrain/new-york"))
        else {
            eprintln!("skipped: the New York region is not installed");
            return;
        };
        let mut s = flightcraft_control::Session::new().unwrap();
        s.install_region(region).unwrap();
        s.execute(
            "sim.reset",
            json!({"world":"new-york","start":start("new-york",None),"paused":false}),
        )
        .unwrap();
        s.run(flightcraft_control::SetCrewMode {
            mode: flightcraft_control::CrewMode::Watching,
            director: false,
        })
        .unwrap();
        let plan = s
            .execute(
                "crew.intent",
                json!({"intent":{"kind":"fly_to_airport","airport":"KTEB","runway":"01","altitude_m":4500.*0.3048}}),
            )
            .unwrap();
        assert_eq!(plan["accepted"], true, "{plan}");
    }
}
