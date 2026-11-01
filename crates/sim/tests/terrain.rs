#![allow(clippy::unwrap_used)]
use flightcraft_sim::{Controls, Sim, Start, State, World, WorldChoice};
use flightcraft_world::{HeightTile, Surface};

#[test]
fn unpowered_glide_into_valley_hillside_crashes_above_sea_level() {
    let world = World::load(WorldChoice::TrainingValley).unwrap();
    let initial = State {
        position_m: [2200.0, 1400.0, -220.0],
        velocity_mps: [0.0, 45.0, 1.0],
        attitude: [libm::sqrt(0.5), 0.0, 0.0, libm::sqrt(0.5)],
        ..State::default()
    };
    let mut sim = Sim::from_state_in_world(initial, Controls::default(), world).unwrap();
    assert!(!sim.readout().crashed);
    assert!(sim.readout().height_agl_m < 220.0 && sim.readout().height_agl_m > 10.0);
    sim.step(120 * 60);
    assert!(sim.readout().crashed, "{:?}", sim.readout());
    assert!(sim.readout().altitude_msl_m > 100.0);
    let state = *sim.state();
    sim.step(120);
    assert_eq!(sim.state(), &state);
}

#[test]
fn sloping_contact_pushes_outward_and_counts_horizontal_arrival_speed() {
    // h = (north - 100) / 2, constant in east. Gear just penetrates this ramp.
    let tile = HeightTile::new(
        [100.0, 400.0],
        100.0,
        3,
        vec![0.0, 50.0, 100.0, 0.0, 50.0, 100.0, 0.0, 50.0, 100.0],
        vec![Surface::Grass; 4],
    )
    .unwrap();
    let world = World::from_tiles(vec![tile]).unwrap();
    let position = [200.0, 500.0, -51.20];
    // Pitch follows the ramp, avoiding the nose wheel penetrating far below its neighbours.
    let pitch = libm::atan(0.5);
    let state = State {
        position_m: position,
        attitude: [libm::cos(pitch / 2.0), 0.0, libm::sin(pitch / 2.0), 0.0],
        ..State::default()
    };
    let mut sim = Sim::from_state_in_world(state, Controls::default(), world.clone()).unwrap();
    assert!(!sim.readout().crashed, "{:?}", sim.readout());
    sim.step(1);
    let [north, _, _] = sim.state().velocity_mps;
    assert!(
        north < 0.0,
        "normal must push towards lower ground in NED: {:?}",
        sim.state()
    );
    let impact = State {
        position_m: [200.0, 500.0, -53.0],
        velocity_mps: [45.0, 0.0, 0.0],
        ..State::default()
    };
    let mut sim = Sim::from_state_in_world(impact, Controls::default(), world).unwrap();
    assert!(!sim.readout().crashed);
    sim.step(120);
    assert!(sim.readout().crashed);
}

#[test]
fn flat_start_remains_default_and_world_content_changes_replay_identity() {
    let flat = Sim::new(&Start::Runway).unwrap();
    let valley =
        Sim::new_with_world(&Start::Runway, World::load(WorldChoice::TrainingValley).unwrap()).unwrap();
    assert_eq!(flat.state(), valley.state());
    assert_ne!(flat.state_hash(), valley.state_hash());
}

#[test]
fn airframe_hits_a_hillside_while_wheels_are_clear() {
    // A steep face just ahead of the nose wheel catches the fuselage nose hard point.
    let samples = (0..41)
        .flat_map(|_| (0..41).map(|i| if i >= 20 { 5.0 } else { 0.0 }))
        .collect();
    let tile = HeightTile::new([200.0, 498.0], 0.1, 41, samples, vec![Surface::Grass; 1600]).unwrap();
    let world = World::from_tiles(vec![tile]).unwrap();
    let state = State {
        position_m: [200.0, 500.0, -3.0],
        ..State::default()
    };
    let sim = Sim::from_state_in_world(state, Controls::default(), world).unwrap();
    assert!(sim.readout().height_agl_m > 1.0);
    assert!(sim.readout().crashed);
}
