#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_sim::systems::{FuelSelector, Magnetos, SystemSetting, SystemSwitch};
use flightcraft_sim::{Controls, Failures, Sim, Start};

#[test]
fn systems_step_with_flight_and_grouped_steps_replay_exactly() {
    let mut a = Sim::new(&Start::Air {
        height_m: 1000.0,
        airspeed_mps: 40.0,
        heading_rad: 0.0,
    })
    .unwrap();
    a.set_failures(Failures {
        vacuum_pump: true,
        alternator: true,
        ..Failures::default()
    })
    .unwrap();
    a.set_systems(SystemSetting::Switch {
        switch: SystemSwitch::Lights,
        on: true,
    })
    .unwrap();
    let mut b = a.clone();
    a.step(1200);
    for _ in 0..1200 {
        b.step(1);
    }
    assert!(!a.readout().crashed);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(a.systems(), b.systems());
    assert!(a.systems().electrical.battery_soc < 1.0);
    assert!(a.systems().tanks[0].quantity_kg < 20.0);
    let before = a.systems().clone();
    for _ in 0..3 {
        let _ = a.readout();
        let _ = a.state_hash();
    }
    assert_eq!(*a.systems(), before);
}

#[test]
fn engine_switches_affect_actual_propeller_loads_and_unselected_fuel_starves() {
    let mut sim = Sim::new(&Start::Runway).unwrap();
    sim.set_controls(Controls {
        throttle: 0.5,
        brake: 1.0,
        ..Controls::default()
    });
    let both = sim.readout().engine_rpm;
    sim.set_systems(SystemSetting::Magnetos {
        position: Magnetos::Left,
    })
    .unwrap();
    assert!(sim.readout().engine_rpm < both);
    sim.set_systems(SystemSetting::Magnetos {
        position: Magnetos::Both,
    })
    .unwrap();
    assert_eq!(sim.readout().engine_rpm, both);
    sim.set_systems(SystemSetting::TankQuantity {
        tank: "left".into(),
        mass_kg: 0.0,
    })
    .unwrap();
    sim.set_systems(SystemSetting::FuelSelector {
        position: FuelSelector::Left,
    })
    .unwrap();
    assert_eq!(sim.readout().engine_rpm, 0.0);
    sim.step(1);
    assert!(sim.systems().fuel_starved);
    assert!(!sim.systems().engine_running);
    assert!(sim.systems().tanks[1].quantity_kg > 0.0);
}

#[test]
fn direct_nonfinite_settings_are_atomic_and_hash_includes_sensor_memory() {
    let mut sim = Sim::new(&Start::Runway).unwrap();
    let before = sim.systems().clone();
    let initial_hash = sim.state_hash();
    sim.set_systems(SystemSetting::Magnetos {
        position: Magnetos::Both,
    })
    .unwrap();
    sim.set_systems(SystemSetting::Switch {
        switch: SystemSwitch::Master,
        on: true,
    })
    .unwrap();
    assert_eq!(sim.state_hash(), initial_hash);
    for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert!(sim.set_systems(SystemSetting::Mixture { value }).is_err());
        assert_eq!(*sim.systems(), before);
    }
    let initial = sim.state_hash();
    sim.set_systems(SystemSetting::Altimeter {
        pressure_pa: 102000.0,
    })
    .unwrap();
    assert_ne!(sim.state_hash(), initial);
    assert_eq!(sim.state().time_s, 0.0);
}

#[test]
fn mutated_resolved_metadata_cannot_install_nonfinite_or_invalid_engine_state() {
    let original = flightcraft_aero::AircraftCatalog::builtins()
        .unwrap()
        .resolve_aircraft("corvid")
        .unwrap();
    let mut sim = Sim::new(&Start::Runway).unwrap();
    let before = sim.systems().clone();
    let mut bad = original.clone();
    bad.load.stations[0].mass_kg = f64::NAN;
    assert!(sim.configure_systems(&bad).is_err());
    assert_eq!(*sim.systems(), before);
    let mut bad = original;
    bad.engine.propulsion.max_power_w = 0.0;
    assert!(sim.configure_systems(&bad).is_err());
    assert_eq!(*sim.systems(), before);
}

#[test]
fn draining_and_servicing_recompute_mass_cg_and_inertia_once_per_step() {
    use flightcraft_aero::{AircraftCatalog, StationKind, StationLoad};
    let configuration = AircraftCatalog::builtins()
        .unwrap()
        .resolve_aircraft("larrow")
        .unwrap();
    let mut sim = Sim::new_with_aircraft(
        &Start::Air {
            height_m: 1000.,
            airspeed_mps: 40.,
            heading_rad: 0.,
        },
        configuration.aircraft.clone(),
    )
    .unwrap();
    sim.set_controls(Controls {
        throttle: 0.5,
        brake: 1.0,
        ..Controls::default()
    });
    let initial = sim.aircraft().mass.clone();
    sim.step(1200);
    assert!(!sim.readout().crashed);
    assert!(sim.aircraft().mass.mass_kg < initial.mass_kg);
    assert_ne!(sim.aircraft().mass.cg_m, initial.cg_m);
    assert_ne!(sim.aircraft().mass.inertia_kg_m2, initial.inertia_kg_m2);
    let burned = 40. - sim.systems().tanks.iter().map(|t| t.quantity_kg).sum::<f64>();
    assert!((initial.mass_kg - sim.aircraft().mass.mass_kg - burned).abs() < 1e-10);
    let mut load = configuration.load.clone();
    for station in configuration
        .variant
        .stations
        .iter()
        .filter(|s| s.kind == StationKind::Fuel)
    {
        let remaining = sim.systems().tanks.iter().map(|t| t.quantity_kg).sum();
        *load
            .stations
            .iter_mut()
            .find(|e| e.station == station.id)
            .unwrap() = StationLoad {
            station: station.id.clone(),
            mass_kg: remaining,
        };
    }
    assert_eq!(
        sim.aircraft().mass,
        configuration.variant.loaded_mass(&load).unwrap()
    );
    // Servicing takes effect at a fixed-step boundary, and adds no time itself.
    let before = sim.aircraft().mass.clone();
    let time = sim.state().time_s;
    sim.set_systems(SystemSetting::TankQuantity {
        tank: "left".into(),
        mass_kg: 35.,
    })
    .unwrap();
    assert_eq!(sim.state().time_s, time);
    assert_eq!(sim.aircraft().mass, before);
    sim.step(1);
    assert!(sim.aircraft().mass.mass_kg > before.mass_kg);
}

#[test]
fn oil_cools_and_loses_pressure_without_combustion() {
    let mut sim = Sim::new(&Start::Runway).unwrap();
    sim.set_controls(Controls {
        brake: 1.0,
        ..Controls::default()
    });
    let initial = sim.systems().oil_temperature_c;
    sim.set_systems(SystemSetting::Magnetos {
        position: Magnetos::Off,
    })
    .unwrap();
    sim.step(12000);
    assert!(!sim.readout().crashed);
    assert!(sim.systems().oil_temperature_c < initial);
    assert!(sim.systems().oil_pressure_pa < 1.);
}
