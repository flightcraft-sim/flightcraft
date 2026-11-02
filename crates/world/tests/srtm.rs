#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_world::{
    Region, RegionOrigin, Surface, World,
    srtm::{HgtTile, LocalProjection, Srtm, runway_height},
};

fn bytes(width: usize, height: impl Fn(usize, usize) -> i16) -> Vec<u8> {
    let mut data = Vec::new();
    for row in 0..width {
        for col in 0..width {
            data.extend_from_slice(&height(width - 1 - row, col).to_be_bytes());
        }
    }
    data
}
fn region() -> Region {
    Region {
        id: "synthetic".into(),
        name: "Synthetic field".into(),
        origin: RegionOrigin {
            latitude_deg: 0.5,
            longitude_deg: 0.5,
            field_elevation_m: 800.0,
            runway_heading_deg: 75.0,
            runway_length_m: 900.0,
            runway_width_m: 30.0,
        },
        southwest_m: [-2000.0, -2000.0],
        tiles_north: 2,
        tiles_east: 2,
        spacing_m: 31.25,
        tile_size: 65,
        runway_blend_m: 200.0,
        source: String::new(),
        licence: String::new(),
        attribution: String::new(),
        tiles: vec![],
        airports: None,
    }
}
#[test]
fn plane_both_resolutions_and_imported_known_coordinates() {
    for width in [1201, 3601] {
        let data = bytes(width, |n, e| (100 + n + e) as i16);
        let mut srtm = Srtm::default();
        srtm.insert(HgtTile::read("N00E000.hgt", &data).unwrap()).unwrap();
        for (lat, lon) in [(0.12345, 0.54321), (0.0, 0.0), (1.0, 1.0), (0.5, 0.5)] {
            assert!(
                (srtm.height(lat, lon).unwrap() - (100.0 + (lat + lon) * (width - 1) as f64)).abs() < 1e-6
            );
        }
        let r = region();
        let projection = LocalProjection::new(0.5, 0.5).unwrap();
        let (metadata, tiles) = srtm.import(&r).unwrap();
        let world = World::from_region(metadata, tiles).unwrap();
        for (n, e) in [(-1500.0, -1500.0), (1500.0, -1500.0), (-1000.0, 1500.0)] {
            let [lat, lon] = projection.unproject(n, e).unwrap();
            assert!(
                (world.ground(n, e).height_msl_m - (100.0 + (lat + lon) * (width - 1) as f64)).abs() < 1.0
            );
        }
        // Shared edges have exactly identical samples.
        let a = &world.tiles()[0];
        let b = &world.tiles()[1];
        for j in 0..65 {
            let e = -2000.0 + j as f64 * 31.25;
            assert_eq!(
                a.sample(0.0, e).unwrap().height_msl_m,
                b.sample(0.0, e).unwrap().height_msl_m
            );
        }
    }
}
#[test]
fn cone_interpolation_and_import() {
    let data = bytes(1201, |n, e| {
        (2000.0 - libm::hypot(n as f64 - 600.0, e as f64 - 600.0)).round() as i16
    });
    let mut srtm = Srtm::default();
    srtm.insert(HgtTile::read("N00E000.hgt", &data).unwrap()).unwrap();
    for (lat, lon) in [(0.25, 0.75), (0.5001, 0.6), (0.0, 1.0)] {
        let expected = 2000.0 - libm::hypot(lat * 1200.0 - 600.0, lon * 1200.0 - 600.0);
        assert!((srtm.height(lat, lon).unwrap() - expected).abs() < 1.0);
    }
    let (metadata, tiles) = srtm.import(&region()).unwrap();
    let world = World::from_region(metadata, tiles).unwrap();
    let projection = LocalProjection::new(0.5, 0.5).unwrap();
    for (n, e) in [(-1500.0, -1500.0), (1500.0, -1500.0)] {
        let [lat, lon] = projection.unproject(n, e).unwrap();
        assert!(
            (world.ground(n, e).height_msl_m
                - (2000.0 - libm::hypot(lat * 1200.0 - 600.0, lon * 1200.0 - 600.0)))
            .abs()
                < 1.0
        );
    }
}
#[test]
fn fills_voids_and_crosses_source_edges() {
    let data = bytes(1201, |n, e| {
        if (n == 600 && e == 600) || e == 1200 {
            i16::MIN
        } else {
            1000
        }
    });
    let mut srtm = Srtm::default();
    srtm.insert(HgtTile::read("N00E000.hgt", &data).unwrap()).unwrap();
    srtm.insert(HgtTile::read("N00E001.hgt", &bytes(1201, |_, _| 1000)).unwrap())
        .unwrap();
    assert_eq!(srtm.height(0.5, 0.5).unwrap(), 1000.0);
    assert_eq!(srtm.height(0.5, 1.0).unwrap(), 1000.0);
    assert!(srtm.height(2.0, 2.0).is_err());
    let mut empty = Srtm::default();
    empty
        .insert(HgtTile::read("N00E000.hgt", &bytes(1201, |_, _| i16::MIN)).unwrap())
        .unwrap();
    assert!(empty.height(0.5, 0.5).is_err());
}
#[test]
fn runway_flat_and_blend_smooth() {
    let r = region();
    let heading = r.origin.runway_heading_deg.to_radians();
    let (s, c) = libm::sincos(heading);
    let mut srtm = Srtm::default();
    srtm.insert(HgtTile::read("N00E000.hgt", &bytes(1201, |_, _| 1200)).unwrap())
        .unwrap();
    let (metadata, tiles) = srtm.import(&r).unwrap();
    let world = World::from_region(metadata, tiles).unwrap();
    for along in [0.0, 100.0, 899.0] {
        for across in [-15.0, 0.0, 15.0] {
            let (n, e) = (along * c - across * s, along * s + across * c);
            assert!((world.ground(n, e).height_msl_m - 800.0).abs() < 1e-4);
            // Check actual resampled grid, independently of world's runway override.
            let h = world
                .tiles()
                .iter()
                .find_map(|t| t.sample(n, e))
                .unwrap()
                .height_msl_m;
            assert!((h - 800.0).abs() < 1e-4);
        }
    }
    assert_eq!(world.ground(450.0 * c, 450.0 * s).surface, Surface::Pavement);
    let n = 450.0 * c - (-s) * 0.0;
    let across = 15.0 + 62.5;
    let evaluate = |d| runway_height(&r, n - s * d, 450.0 * s + c * d, 1200.0);
    assert!((evaluate(across + 0.01) - evaluate(across)).abs() < 0.001);
    assert!((evaluate(across + 200.0) - evaluate(across + 199.99)).abs() < 0.001);
    assert_eq!(evaluate(across + 100.0), 1000.0);
}
#[test]
fn projection_round_trip_within_fifty_km() {
    for (lat, lon) in [(0.0, 0.0), (37.0, -123.0), (-45.0, 179.9), (84.0, -179.9)] {
        let p = LocalProjection::new(lat, lon).unwrap();
        for n in [-50000.0, -20000.0, 0.0, 50000.0] {
            for e in [-50000.0, 0.0, 50000.0] {
                let [a, b] = p.unproject(n, e).unwrap();
                let [nn, ee] = p.project(a, b).unwrap();
                assert!(libm::hypot(n - nn, e - ee) < 0.5);
            }
        }
    }
    assert!(LocalProjection::new(f64::NAN, 0.0).is_err());
    assert!(LocalProjection::new(90.0, 0.0).is_err());
    let p = LocalProjection::new(0.0, 0.0).unwrap();
    assert!(p.unproject(f64::NAN, 0.0).is_err());
    assert!(p.project(0.0, 180.0).is_err());
}
#[test]
fn hostile_hgt_and_region_inputs() {
    for name in [
        "../N00E000.hgt",
        "N90E000.hgt",
        "N00E180.hgt",
        "N00X000.hgt",
        "N0E000.hgt",
        "éééé.hgt",
        "S00W000.hgt",
    ] {
        assert!(HgtTile::read(name, &[]).is_err());
    }
    for size in [
        0,
        1,
        1201 * 1201 * 2 - 1,
        1201 * 1201 * 2 + 1,
        3601 * 3601 * 2 + 1,
    ] {
        assert!(HgtTile::read("N00E000.hgt", &vec![0; size]).is_err());
    }
    assert!(Region::from_json("{}").is_err());
    let mut r = region();
    r.tiles_north = u32::MAX;
    assert!(r.validate().is_err());
    r = region();
    r.tile_size = u32::MAX;
    assert!(r.validate().is_err());
    r = region();
    r.tiles = vec!["../escape.fcht".into()];
    assert!(r.validate().is_err());
    r = region();
    r.spacing_m = f32::NAN;
    assert!(r.validate().is_err());
    r = region();
    r.id = "../escape".into();
    assert!(r.validate().is_err());
}

#[test]
fn southwest_names_signed_samples_and_duplicate_sources() {
    let data = bytes(1201, |n, e| (-400 + n as i32 + e as i32) as i16);
    let tile = HgtTile::read("S01W001.hgt", &data).unwrap();
    assert_eq!(tile.corner(), [-1, -1]);
    let mut source = Srtm::default();
    source.insert(tile).unwrap();
    assert_eq!(source.height(-1.0, -1.0).unwrap(), -400.0);
    assert!((source.height(-0.75, -0.5).unwrap() - 500.0).abs() < 1e-6);
    assert!(
        source
            .insert(HgtTile::read("S01W001.hgt", &data).unwrap())
            .is_err()
    );
    for name in ["N90E000.hgt", "N00E180.hgt", "../N00E000.hgt", "N00W000.hgt"] {
        assert!(HgtTile::read(name, &data).is_err());
    }
    let data = bytes(1201, |n, e| {
        if n == 600 && e == 600 {
            i16::MIN
        } else {
            (100 + n + e) as i16
        }
    });
    let mut source = Srtm::default();
    source
        .insert(HgtTile::read("N00E000.hgt", &data).unwrap())
        .unwrap();
    assert!((source.height(0.5, 0.5).unwrap() - 1300.0).abs() < 1e-6);
}
