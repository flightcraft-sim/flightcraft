#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_sim::{Aircraft, Controls, Sim, Start, Weather};

#[test]
fn custom_aircraft_drives_trim_dynamics_and_control_limits() {
    let start = Start::Air {
        height_m: 1000.0,
        airspeed_mps: 40.0,
        heading_rad: 0.0,
    };
    let mut original = Sim::new(&start).unwrap();
    let mut aircraft = Aircraft::corvid().unwrap();
    aircraft.identity.id = "custom-trainer".into();
    aircraft.mass.mass_kg = 1200.0;
    aircraft.mass.inertia_kg_m2[0][1] = 40.0;
    aircraft.mass.inertia_kg_m2[1][0] = 40.0;
    aircraft.geometry.wing_area_m2 = 16.0;
    aircraft.propulsion.max_power_w = 140000.0;
    aircraft.systems.elevator = [-0.5, 0.5];
    aircraft.systems.brake = [0.0, 0.5];
    let mut custom =
        Sim::new_with_aircraft_and_weather(&start, aircraft.clone(), Weather::default()).unwrap();
    assert_eq!(custom.aircraft(), &aircraft);
    assert_ne!(original.state().attitude, custom.state().attitude);
    let controls = Controls {
        elevator: 1.0,
        brake: 1.0,
        aileron: 0.2,
        throttle: 0.7,
        ..Controls::default()
    };
    original.set_controls(controls);
    custom.set_controls(controls);
    assert_eq!(custom.controls().elevator, 0.5);
    assert_eq!(custom.controls().brake, 0.5);
    original.step(12);
    custom.step(12);
    assert_ne!(original.state(), custom.state());
    assert!(!custom.readout().crashed);
    assert!(custom.state().angular_rate_rps.iter().all(|v| v.is_finite()));
}

#[test]
fn custom_gear_and_cg_define_runway_start_and_height() {
    let original = Sim::new(&Start::Runway).unwrap();
    let mut aircraft = Aircraft::corvid().unwrap();
    aircraft.mass.cg_m = [0.0, 0.0, 0.1];
    for gear in &mut aircraft.geometry.gear {
        gear.point_m[2] += 0.3;
    }
    let mut custom = Sim::new_with_aircraft(&Start::Runway, aircraft).unwrap();
    assert!((custom.state().position_m[2] - original.state().position_m[2] + 0.2).abs() < 1e-12);
    assert!(custom.readout().on_ground);
    custom.step(120);
    assert!(!custom.readout().crashed);
    assert!(custom.readout().on_ground);
}

#[test]
fn invalid_rust_aircraft_is_rejected_before_simulation() {
    let mut aircraft = Aircraft::corvid().unwrap();
    aircraft.systems.elevator = [1.0, -1.0];
    let error = Sim::new_with_aircraft(&Start::Runway, aircraft).unwrap_err();
    assert!(error.to_string().contains("systems.elevator"));
}

#[test]
fn extreme_valid_aircraft_freezes_before_intermediate_overflow() {
    let mut aircraft = Aircraft::corvid().unwrap();
    aircraft.mass.mass_kg = 1e-6;
    aircraft.geometry.wing_area_m2 = 1e12;
    aircraft.mass.inertia_kg_m2 = [[1e-5, 0.0, 0.0], [0.0, 1e-5, 0.0], [0.0, 0.0, 1e-5]];
    aircraft.validate().unwrap();
    let mut sim = Sim::new_with_aircraft(
        &Start::Air {
            height_m: 1000.0,
            airspeed_mps: 40.0,
            heading_rad: 0.0,
        },
        aircraft,
    )
    .unwrap();
    let before = *sim.state();
    sim.set_controls(Controls {
        elevator: 1.0,
        throttle: 1.0,
        ..Controls::default()
    });
    sim.step(120);
    assert!(sim.readout().crashed);
    assert_eq!(sim.state(), &before);
}

#[test]
fn layered_station_load_reaches_simulation_without_changing_its_interface() {
    let catalog = flightcraft_aero::AircraftCatalog::builtins().unwrap();
    let load = flightcraft_aero::Load {
        stations: vec![flightcraft_aero::StationLoad {
            station: "pilot".into(),
            mass_kg: 80.0,
        }],
    };
    let resolved = catalog.resolve("corvid-standard", "corvid-six", &load).unwrap();
    assert_eq!(resolved.aircraft.mass.mass_kg, 880.0);
    let start = Start::Air {
        height_m: 500.0,
        airspeed_mps: 40.0,
        heading_rad: 0.0,
    };
    let sim = Sim::new_with_aircraft(&start, resolved.aircraft).unwrap();
    let default_sim = Sim::new(&start).unwrap();
    assert_eq!(sim.aircraft().mass.mass_kg, 880.0);
    assert_ne!(sim.state().attitude, default_sim.state().attitude);
}
