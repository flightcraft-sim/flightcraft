//! Controlled bench inputs isolate sensor dynamics from changes in the flight trajectory.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::systems::*;
use super::{Controls, DT_S, Failures, Readout};
use flightcraft_aero::{AircraftCatalog, propulsion_with_power};

fn bench(name: &str) -> (SystemsState, SystemsModel, Readout) {
    let r = Readout {
        indicated_airspeed_mps: 45.0,
        altitude_msl_m: 1000.0,
        engine_rpm: 2000.0,
        controls: Controls {
            throttle: 0.65,
            ..Controls::default()
        },
        ..Readout::default()
    };
    let resolved = AircraftCatalog::builtins()
        .unwrap()
        .resolve_aircraft(name)
        .unwrap();
    let (s, m) = SystemsState::configure(&resolved, r).unwrap();
    (s, m.unwrap(), r)
}
fn seconds(s: &mut SystemsState, m: &SystemsModel, r: Readout, f: Failures, n: u32) {
    for _ in 0..n * 120 {
        s.step(m, r, f);
    }
}
fn set(s: &mut SystemsState, m: &SystemsModel, setting: SystemSetting) {
    s.set(setting, Some(m)).unwrap();
}

#[test]
fn failed_vacuum_spins_down_and_precesses_over_minutes_then_recovers() {
    let (mut healthy, m, r) = bench("corvid");
    let mut failed = healthy.clone();
    seconds(&mut healthy, &m, r, Failures::default(), 300);
    seconds(
        &mut failed,
        &m,
        r,
        Failures {
            vacuum_pump: true,
            ..Failures::default()
        },
        300,
    );
    assert!(healthy.gyro_spin > 0.95);
    assert!(healthy.sensed.roll_rad.abs() < 0.01);
    assert_eq!(failed.suction_pa, 0.0);
    assert!(failed.gyro_spin < 0.04);
    assert!(failed.sensed.roll_rad > 0.8);
    assert!(failed.sensed.heading_rad > 1.0);
    assert!(!failed.sensed.attitude_valid);
    let error = failed.gyro_error_rad[0];
    seconds(&mut failed, &m, r, Failures::default(), 300);
    assert!(failed.gyro_spin > 0.95);
    assert!(failed.gyro_error_rad[0] < error);
}

#[test]
fn battery_only_endurance_matches_declared_capacity_and_sheds_loads() {
    let (mut s, m, _) = bench("corvid");
    let f = Failures {
        alternator: true,
        ..Failures::default()
    };
    // Avionics (5 A) plus the electric turn coordinator (0.8 A): about 4.14 hours.
    for _ in 0..(4 * 3600 * 120) {
        s.electrical_step(&m, 2000.0, f);
    }
    assert!((s.electrical.battery_soc - 1.0 / 30.0).abs() < 0.001);
    assert!(s.electrical.main_bus_voltage > 9.0);
    for _ in 0..(3600 * 120) {
        s.electrical_step(&m, 2000.0, f);
    }
    assert!(s.electrical.battery_soc <= 0.001);
    assert_eq!(s.electrical.main_bus_voltage, 0.0);
    assert!(!s.electrical.loads.iter().any(|l| l.powered));
    // Restoring the alternator recharges even a fully discharged battery.
    for _ in 0..1200 {
        s.electrical_step(&m, 2000.0, Failures::default());
    }
    assert!(s.electrical.battery_soc > 0.001);
    assert!(s.electrical.battery_current_a < 0.0);
}

#[test]
fn sealed_pitot_climb_increases_airspeed_and_descent_reverses_it() {
    let (mut s, m, mut r) = bench("corvid");
    let start = s.sensed.indicated_airspeed_mps;
    let total = s.pitot_total_pressure_pa;
    let f = Failures {
        pitot_blocked: true,
        ..Failures::default()
    };
    for _ in 0..1200 {
        r.altitude_msl_m += DT_S * 3.0;
        s.step(&m, r, f);
    }
    assert_eq!(s.pitot_total_pressure_pa, total);
    assert!(s.sensed.indicated_airspeed_mps > start + 5.0);
    for _ in 0..1200 {
        r.altitude_msl_m -= DT_S * 3.0;
        s.step(&m, r, f);
    }
    assert!((s.sensed.indicated_airspeed_mps - start).abs() < 1e-8);
}

#[test]
fn blocked_static_freezes_altitude_and_vsi_alternate_source_restores_them() {
    let (mut s, m, mut r) = bench("corvid");
    let initial = s.sensed.altitude_msl_m;
    let f = Failures {
        static_blocked: true,
        ..Failures::default()
    };
    for _ in 0..1200 {
        r.altitude_msl_m += DT_S * 3.0;
        s.step(&m, r, f);
    }
    assert_eq!(s.sensed.altitude_msl_m, initial);
    assert_eq!(s.sensed.vertical_speed_mps, 0.0);
    assert!(s.sensed.indicated_airspeed_mps < 45.0);
    set(
        &mut s,
        &m,
        SystemSetting::Switch {
            switch: SystemSwitch::AlternateStatic,
            on: true,
        },
    );
    s.step(&m, r, f);
    assert!(s.sensed.altitude_msl_m > r.altitude_msl_m);
    assert!(s.sensed.vertical_speed_mps > 0.0);
    let altitude = s.sensed.altitude_msl_m;
    set(
        &mut s,
        &m,
        SystemSetting::Altimeter {
            pressure_pa: 102325.0,
        },
    );
    assert!(s.sensed.altitude_msl_m > altitude + 70.0);
}

#[test]
fn fuel_starvation_ignores_fuel_in_unselected_tank_and_requires_restart() {
    let (mut s, m, r) = bench("corvid");
    set(
        &mut s,
        &m,
        SystemSetting::TankQuantity {
            tank: "left".into(),
            mass_kg: 0.0,
        },
    );
    set(
        &mut s,
        &m,
        SystemSetting::FuelSelector {
            position: FuelSelector::Left,
        },
    );
    s.step(&m, r, Failures::default());
    assert!(s.fuel_starved);
    assert!(!s.engine_running);
    assert_eq!(s.engine_power_fraction, 0.0);
    assert_eq!(s.fuel_flow_kg_s, 0.0);
    assert!(s.tanks[1].quantity_kg > 10.0);
    set(
        &mut s,
        &m,
        SystemSetting::FuelSelector {
            position: FuelSelector::Right,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 1);
    assert!(!s.engine_running);
    set(
        &mut s,
        &m,
        SystemSetting::Switch {
            switch: SystemSwitch::Starter,
            on: true,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 3);
    assert!(s.engine_running);
    assert!(s.fuel_flow_kg_s > 0.0);
}

#[test]
fn single_mag_drop_is_in_light_piston_runup_range() {
    let (mut s, m, _) = bench("corvid");
    let aircraft = AircraftCatalog::builtins()
        .unwrap()
        .resolve_aircraft("corvid")
        .unwrap()
        .aircraft;
    // Set throttle for a stationary 1700 RPM run-up, independently using the propeller solver.
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..40 {
        let throttle = (lo + hi) * 0.5;
        if propulsion_with_power(&aircraft, 0.0, throttle, 1.225, 1.0).1 < 1700.0 {
            lo = throttle;
        } else {
            hi = throttle;
        }
    }
    let throttle = (lo + hi) * 0.5;
    for position in [Magnetos::Left, Magnetos::Right] {
        set(&mut s, &m, SystemSetting::Magnetos { position });
        let rpm = propulsion_with_power(&aircraft, 0.0, throttle, 1.225, s.engine_power_fraction).1;
        // Original fictional acceptance target: a roughly 75–175 RPM single-mag drop.
        assert!((75.0..175.0).contains(&(1700.0 - rpm)), "{rpm}");
    }
    set(
        &mut s,
        &m,
        SystemSetting::Magnetos {
            position: Magnetos::Both,
        },
    );
    assert_eq!(s.engine_power_fraction, 1.0);
}

#[test]
fn humid_carb_ice_reduces_power_and_heat_clears_it_injected_engine_immune() {
    for name in ["corvid", "larrow"] {
        let (mut s, m, r) = bench(name);
        set(
            &mut s,
            &m,
            SystemSetting::Environment {
                temperature_c: 15.0,
                relative_humidity: 1.0,
            },
        );
        seconds(&mut s, &m, r, Failures::default(), 300);
        if name == "corvid" {
            assert!(s.carb_ice > 0.15);
            assert!(s.engine_power_fraction < 0.9);
            set(
                &mut s,
                &m,
                SystemSetting::Switch {
                    switch: SystemSwitch::CarbHeat,
                    on: true,
                },
            );
            seconds(&mut s, &m, r, Failures::default(), 120);
            assert_eq!(s.carb_ice, 0.0);
            assert_eq!(
                s.engine_power_fraction,
                m.definition.engine.carb_heat_power_fraction
            );
            set(
                &mut s,
                &m,
                SystemSetting::Switch {
                    switch: SystemSwitch::CarbHeat,
                    on: false,
                },
            );
            assert_eq!(s.engine_power_fraction, 1.0);
        } else {
            assert_eq!(s.carb_ice, 0.0);
            assert_eq!(s.engine_power_fraction, 1.0);
        }
    }
}

#[test]
fn breaker_trips_resets_and_retrips_until_fault_removed() {
    let (mut s, m, r) = bench("corvid");
    set(
        &mut s,
        &m,
        SystemSetting::LoadMultiplier {
            load: "avionics".into(),
            value: 4.0,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 1);
    assert!(s.electrical.loads[0].tripped);
    assert!(!s.electrical.loads[0].powered);
    set(
        &mut s,
        &m,
        SystemSetting::Breaker {
            load: "avionics".into(),
            closed: true,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 1);
    assert!(s.electrical.loads[0].tripped);
    set(
        &mut s,
        &m,
        SystemSetting::LoadMultiplier {
            load: "avionics".into(),
            value: 1.0,
        },
    );
    set(
        &mut s,
        &m,
        SystemSetting::Breaker {
            load: "avionics".into(),
            closed: true,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 1);
    assert!(s.electrical.loads[0].powered);
    assert!(!s.electrical.loads[0].tripped);
}

#[test]
fn electronic_gyro_uses_its_backup_after_master_off_then_spins_down() {
    let (mut s, m, r) = bench("larrow");
    set(
        &mut s,
        &m,
        SystemSetting::Switch {
            switch: SystemSwitch::Master,
            on: false,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 60);
    let l = s
        .electrical
        .loads
        .iter()
        .find(|l| l.id == "electric_gyros")
        .unwrap();
    assert!(l.powered && l.on_backup);
    assert!(l.backup_remaining_ah < 1.0);
    assert!(s.sensed.attitude_valid);
    seconds(&mut s, &m, r, Failures::default(), 1900);
    assert!(!s.sensed.attitude_valid);
    assert!(s.gyro_spin < 0.1);
}

#[test]
fn mixture_leaning_heats_to_peak_then_cools_and_cutoff_stops_combustion() {
    let (mut rich, m, r) = bench("corvid");
    let mut lean = rich.clone();
    set(&mut lean, &m, SystemSetting::Mixture { value: 0.72 });
    seconds(&mut rich, &m, r, Failures::default(), 120);
    seconds(&mut lean, &m, r, Failures::default(), 120);
    assert!(lean.egt_c > rich.egt_c);
    assert!(lean.fuel_flow_kg_s < rich.fuel_flow_kg_s);
    assert!(lean.cht_c > rich.cht_c);
    set(&mut lean, &m, SystemSetting::Mixture { value: 0.0 });
    seconds(&mut lean, &m, r, Failures::default(), 120);
    assert!(!lean.engine_running);
    assert_eq!(lean.fuel_flow_kg_s, 0.0);
    assert!(lean.egt_c < 100.0);
}

#[test]
fn cold_carburetted_start_requires_fuel_mags_master_primer_and_cranking_time() {
    let (mut s, m, r) = bench("corvid");
    set(
        &mut s,
        &m,
        SystemSetting::Magnetos {
            position: Magnetos::Off,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 600);
    assert!(s.cht_c < 60.0);
    set(
        &mut s,
        &m,
        SystemSetting::Magnetos {
            position: Magnetos::Both,
        },
    );
    set(
        &mut s,
        &m,
        SystemSetting::Switch {
            switch: SystemSwitch::Starter,
            on: true,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 3);
    assert!(!s.engine_running);
    set(&mut s, &m, SystemSetting::Primer { charge: 0.6 });
    set(
        &mut s,
        &m,
        SystemSetting::Switch {
            switch: SystemSwitch::Master,
            on: false,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 3);
    assert!(!s.engine_running);
    set(
        &mut s,
        &m,
        SystemSetting::Switch {
            switch: SystemSwitch::Master,
            on: true,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 1);
    assert!(!s.engine_running);
    seconds(&mut s, &m, r, Failures::default(), 2);
    assert!(s.engine_running);
    assert!(s.primer_charge < 0.6);
}

/// A cold carburetted engine, magnetos and master on, mixture rich, starter off.
fn cold_ready(aircraft: &str) -> (SystemsState, SystemsModel, Readout) {
    let (mut s, m, r) = bench(aircraft);
    set(
        &mut s,
        &m,
        SystemSetting::Magnetos {
            position: Magnetos::Off,
        },
    );
    seconds(&mut s, &m, r, Failures::default(), 600);
    assert!(s.cht_c < 60.0 && !s.engine_running);
    set(
        &mut s,
        &m,
        SystemSetting::Magnetos {
            position: Magnetos::Both,
        },
    );
    (s, m, r)
}

fn starter(s: &mut SystemsState, m: &SystemsModel, on: bool) {
    set(
        s,
        m,
        SystemSetting::Switch {
            switch: SystemSwitch::Starter,
            on,
        },
    );
}

#[test]
fn primed_fuel_evaporates_when_the_engine_is_not_cranked() {
    let (mut s, m, r) = cold_ready("corvid");
    set(&mut s, &m, SystemSetting::Primer { charge: 0.6 });
    seconds(&mut s, &m, r, Failures::default(), 180);
    assert!(
        s.primer_charge < 0.25 && s.primer_charge > 0.15,
        "{}",
        s.primer_charge
    );
    seconds(&mut s, &m, r, Failures::default(), 1200);
    assert_eq!(s.primer_charge, 0.0);
}

#[test]
fn over_priming_floods_the_engine_until_cranking_clears_it() {
    let (mut s, m, r) = cold_ready("corvid");
    set(&mut s, &m, SystemSetting::Primer { charge: 1.0 });
    assert!(s.flooded(&m));
    starter(&mut s, &m, true);
    seconds(&mut s, &m, r, Failures::default(), 2);
    assert!(!s.engine_running, "a flooded engine does not fire");
    seconds(&mut s, &m, r, Failures::default(), 6);
    assert!(!s.flooded(&m));
    assert!(
        s.engine_running,
        "cranking drew the excess through and it started"
    );
}

#[test]
fn clearing_procedure_purges_a_flooded_engine_faster() {
    let (mut normal, m, r) = cold_ready("corvid");
    let mut clearing = normal.clone();
    for s in [&mut normal, &mut clearing] {
        set(s, &m, SystemSetting::Primer { charge: 1.0 });
    }
    set(&mut clearing, &m, SystemSetting::Mixture { value: 0.0 });
    let mut wide_open = r;
    wide_open.controls.throttle = 1.0;
    starter(&mut normal, &m, true);
    starter(&mut clearing, &m, true);
    seconds(&mut normal, &m, wide_open, Failures::default(), 2);
    seconds(&mut clearing, &m, wide_open, Failures::default(), 2);
    assert!(!clearing.engine_running, "idle cut-off never fires");
    assert!(clearing.primer_charge < normal.primer_charge - 0.2);
}

#[test]
fn depleted_battery_sheds_main_bus_before_essential_fuel_pump() {
    let (mut s, m, _) = bench("corvid");
    s.electrical.battery_soc = 0.0;
    for switch in [SystemSwitch::FuelPump, SystemSwitch::Lights] {
        set(&mut s, &m, SystemSetting::Switch { switch, on: true });
    }
    // Idle alternator supplies 8 A: essential pump needs 3 A, main loads need 13 A.
    s.electrical_step(&m, 600.0, Failures::default());
    assert_eq!(s.electrical.main_bus_voltage, 0.0);
    assert!(s.electrical.essential_bus_voltage > 12.0);
    assert!(
        s.electrical
            .loads
            .iter()
            .find(|l| l.id == "fuel_pump")
            .unwrap()
            .powered
    );
    assert!(
        !s.electrical
            .loads
            .iter()
            .find(|l| l.id == "avionics")
            .unwrap()
            .powered
    );
    assert!(
        !s.electrical
            .loads
            .iter()
            .find(|l| l.id == "lights")
            .unwrap()
            .powered
    );
}

#[test]
fn exterior_lights_follow_common_switch_breaker_and_master_power() {
    use crate::{Sim, Start};
    use flightcraft_aero::LoadSwitch;
    let mut sim = Sim::new(&Start::Runway).unwrap();
    assert!(!sim.light_powered(LoadSwitch::Nav));
    sim.set_systems(SystemSetting::Switch {
        switch: SystemSwitch::Lights,
        on: true,
    })
    .unwrap();
    sim.step(1);
    assert!(sim.light_powered(LoadSwitch::Nav));
    assert!(sim.light_powered(LoadSwitch::Landing));
    sim.set_systems(SystemSetting::Breaker {
        load: "lights".into(),
        closed: false,
    })
    .unwrap();
    sim.step(1);
    assert!(!sim.light_powered(LoadSwitch::Nav));
    sim.set_systems(SystemSetting::Breaker {
        load: "lights".into(),
        closed: true,
    })
    .unwrap();
    sim.set_systems(SystemSetting::Switch {
        switch: SystemSwitch::Master,
        on: false,
    })
    .unwrap();
    sim.step(1);
    assert!(!sim.light_powered(LoadSwitch::Landing));
}

/// A normal idle gives low suction, not a failed gyro.
#[test]
fn vacuum_gyros_stay_valid_at_idle_and_idle_suction_is_not_annunciated_low() {
    let (mut s, m, mut r) = bench("corvid");
    r.engine_rpm = 615.;
    r.indicated_airspeed_mps = 0.;
    seconds(&mut s, &m, r, Failures::default(), 300);
    assert!(s.gyro_spin > 0.95, "spin {}", s.gyro_spin);
    assert!(s.sensed.attitude_valid);
    assert!(s.suction_pa >= 10_000., "suction {}", s.suction_pa);
    // Well under idle the rotor does slow: a pump barely turning is not enough.
    r.engine_rpm = 100.;
    seconds(&mut s, &m, r, Failures::default(), 300);
    assert!(!s.sensed.attitude_valid, "spin {}", s.gyro_spin);
}
