#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_world::{HeightTile, Surface, TRAINING_SEED, World, WorldChoice, generate_training_valley};

fn plane() -> HeightTile {
    HeightTile::new(
        [100.0, 200.0],
        10.0,
        3,
        vec![0.0, 10.0, 20.0, 20.0, 30.0, 40.0, 40.0, 50.0, 60.0],
        vec![Surface::Grass; 4],
    )
    .unwrap()
}

#[test]
fn round_trip_and_bilinear_plane() {
    let tile = plane();
    assert_eq!(HeightTile::read(&tile.write()).unwrap(), tile);
    for j in 0..3 {
        for i in 0..3 {
            let g = tile
                .sample(100.0 + i as f64 * 10.0, 200.0 + j as f64 * 10.0)
                .unwrap();
            assert_eq!(g.height_msl_m, i as f64 * 10.0 + j as f64 * 20.0);
        }
    }
    let g = tile.sample(102.5, 207.5).unwrap();
    assert_eq!(g.height_msl_m, 17.5);
    let length = libm::sqrt(6.0);
    assert_eq!(g.normal_ned, [-1.0 / length, -2.0 / length, -1.0 / length]);
    for (n, e) in [
        (99.0, 200.0),
        (100.0, 221.0),
        (f64::NAN, 200.0),
        (100.0, f64::INFINITY),
    ] {
        assert!(tile.sample(n, e).is_none());
    }
    // Cross term distinguishes bilinear interpolation from triangular interpolation.
    let saddle = HeightTile::new([0.0; 2], 1.0, 2, vec![0.0, 0.0, 0.0, 4.0], vec![Surface::Grass]).unwrap();
    assert_eq!(saddle.sample(0.25, 0.75).unwrap().height_msl_m, 0.75);
}

#[test]
fn hostile_bytes_are_bounded_and_never_panic() {
    let bytes = plane().write();
    for length in 0..bytes.len() {
        assert!(HeightTile::read(&bytes[..length]).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(HeightTile::read(&extra).is_err());
    for (offset, value) in [(0, 255), (4, 255), (6, 1), (28, 255), (bytes.len() - 1, 255)] {
        let mut invalid = bytes.clone();
        invalid[offset] = value;
        assert!(HeightTile::read(&invalid).is_err());
    }
    for (offset, value) in [(8, f64::NAN.to_le_bytes()), (16, f64::INFINITY.to_le_bytes())] {
        let mut invalid = bytes.clone();
        invalid[offset..offset + 8].copy_from_slice(&value);
        assert!(HeightTile::read(&invalid).is_err());
    }
    for offset in [24, 32] {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut invalid = bytes.clone();
            invalid[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(HeightTile::read(&invalid).is_err());
        }
    }
    let mut bits = 1_u64;
    for length in 0..2048 {
        let hostile: Vec<u8> = (0..length)
            .map(|_| {
                bits = bits.wrapping_mul(6364136223846793005).wrapping_add(1);
                (bits >> 32) as u8
            })
            .collect();
        assert!(HeightTile::read(&hostile).is_err());
    }
    for index in 0..bytes.len() {
        for value in 0..=255 {
            let mut mutated = bytes.clone();
            mutated[index] = value;
            if let Ok(tile) = HeightTile::read(&mutated) {
                assert!(
                    tile.sample(tile.origin_m()[0], tile.origin_m()[1])
                        .unwrap()
                        .height_msl_m
                        .is_finite()
                );
            }
        }
    }
    assert!(HeightTile::new([0.0; 2], 1.0, u32::MAX, vec![], vec![]).is_err());
}

#[test]
fn bundled_valley_is_reproducible_flat_at_airfield_and_continuous() {
    let world = World::load(WorldChoice::TrainingValley).unwrap();
    let generated = generate_training_valley(TRAINING_SEED).unwrap();
    assert_eq!(world.tiles(), generated);
    assert_ne!(generate_training_valley(TRAINING_SEED + 1).unwrap(), generated);
    assert!(generated.iter().map(|t| t.write().len()).sum::<usize>() < 4_000_000);
    for n in [-500.0, 0.0, 450.0, 900.0, 1400.0] {
        for e in [-100.0, 0.0, 100.0] {
            let g = world.ground(n, e);
            assert_eq!(g.height_msl_m, 0.0);
            assert_eq!(g.normal_ned, [0.0, 0.0, -1.0]);
        }
    }
    assert_eq!(world.ground(450.0, 0.0).surface, Surface::Pavement);
    let lake = world.ground(-3250.0, 3000.0);
    assert_eq!(lake.surface, Surface::Water);
    assert_eq!(lake.height_msl_m, 35.0);
    assert_eq!(lake.normal_ned, [0.0, 0.0, -1.0]);
    assert!(world.ground(2200.0, 3000.0).height_msl_m > 500.0);
    for a in world.tiles() {
        for b in world.tiles() {
            let [n, e] = a.origin_m();
            let [nn, ee] = b.origin_m();
            if n + a.extent_m() == nn && e == ee {
                for k in 0..=64 {
                    let east = e + k as f64 * a.spacing_m();
                    assert_eq!(
                        a.sample(nn, east).unwrap().height_msl_m,
                        b.sample(nn, east).unwrap().height_msl_m
                    );
                }
            }
        }
    }
    assert_eq!(world.ground(7000.0, 0.0).height_msl_m, 0.0);
    assert_eq!(world.ground(f64::NAN, 0.0).height_msl_m, 0.0);
}

#[test]
fn nonflat_water_is_rejected_and_surfaces_round_trip() {
    assert!(HeightTile::new([0.0; 2], 1.0, 2, vec![0.0, 1.0, 0.0, 1.0], vec![Surface::Water]).is_err());
    for surface in [Surface::Pavement, Surface::Grass, Surface::Water] {
        let tile = HeightTile::new([0.0; 2], 1.0, 2, vec![35.0; 4], vec![surface]).unwrap();
        assert_eq!(
            HeightTile::read(&tile.write())
                .unwrap()
                .sample(0.5, 0.5)
                .unwrap()
                .surface,
            surface
        );
    }
}

#[test]
fn compact_tiles_are_bounded_quantized_and_backward_compatible() {
    let original = HeightTile::new(
        [0.; 2],
        60.,
        3,
        vec![-123.12, -123.0, 0., 0.12, 0.13, 123.125, 123.13, 500., 500.1],
        vec![Surface::Grass; 4],
    )
    .unwrap();
    let bytes = original.write_compact().unwrap();
    let decoded = HeightTile::read(&bytes).unwrap();
    for j in 0..3 {
        for i in 0..3 {
            let a = original.sample(f64::from(i) * 60., f64::from(j) * 60.).unwrap();
            let b = decoded.sample(f64::from(i) * 60., f64::from(j) * 60.).unwrap();
            assert!((a.height_msl_m - b.height_msl_m).abs() <= 0.125);
        }
    }
    for length in 0..bytes.len() {
        assert!(HeightTile::read(&bytes[..length]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(HeightTile::read(&trailing).is_err());
    let mut stream_trailing = bytes.clone();
    stream_trailing.push(0);
    let len = (stream_trailing.len() - 40) as u32;
    stream_trailing[36..40].copy_from_slice(&len.to_le_bytes());
    assert!(HeightTile::read(&stream_trailing).is_err());
    let mut invalid = bytes.clone();
    invalid[32..36].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(HeightTile::read(&invalid).is_err());
    let mut reserved = bytes.clone();
    reserved[6] = 1;
    assert!(HeightTile::read(&reserved).is_err());
    // A tiny header cannot ask the inflater to allocate outside the declared grid.
    let mut bomb = bytes.clone();
    bomb[28..32].copy_from_slice(&2_u32.to_le_bytes());
    assert!(HeightTile::read(&bomb).is_err());
    for tile in generate_training_valley(TRAINING_SEED).unwrap() {
        assert_eq!(HeightTile::read(&tile.write()).unwrap(), tile);
        let compact = HeightTile::read(&tile.write_compact().unwrap()).unwrap();
        assert_eq!(compact.origin_m(), tile.origin_m());
    }
}
