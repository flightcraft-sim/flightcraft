#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use crate::modern::*;
use crate::systems::*;
use flightcraft_aero::modern::Severity;
use flightcraft_aero::{AircraftCatalog, DefinitionSource, LayerFile};
fn resolved() -> flightcraft_aero::ResolvedAircraft {
    let mut c = AircraftCatalog::builtins().unwrap();
    for json in [
        include_str!("../../../examples/packs/equipment-demo/engine.json"),
        include_str!("../../../examples/packs/equipment-demo/systems.json"),
        include_str!("../../../examples/packs/equipment-demo/panel.json"),
        include_str!("../../../examples/packs/equipment-demo/variant.json"),
        include_str!("../../../examples/packs/equipment-demo/aircraft.json"),
    ] {
        c.insert(LayerFile::from_json(json).unwrap(), DefinitionSource::BuiltIn)
            .unwrap();
    }
    c.validate().unwrap();
    c.resolve_aircraft("equipment-demonstrator").unwrap()
}
fn bench() -> (SystemsState, SystemsModel, Readout) {
    let r = Readout {
        engine_rpm: 4800.,
        altitude_msl_m: 1000.,
        controls: Controls {
            throttle: 0.6,
            ..Default::default()
        },
        ..Default::default()
    };
    let (s, m) = SystemsState::configure(&resolved(), r).unwrap();
    (s, m.unwrap(), r)
}
fn action(s: &mut SystemsState, m: &SystemsModel, setting: ModernSetting) {
    s.set(SystemSetting::Equipment { setting }, Some(m)).unwrap();
}
fn advance(s: &mut SystemsState, m: &SystemsModel, r: Readout, n: u32) {
    for _ in 0..n {
        s.step(m, r, Failures::default());
    }
}
#[test]
fn lanes_pumps_backup_key_and_gearing() {
    let (mut s, m, r) = bench();
    advance(&mut s, &m, r, 1);
    assert_eq!(s.engine_power_fraction, 1.);
    assert_eq!(s.modern.as_ref().unwrap().propeller_rpm, 2000.);
    action(
        &mut s,
        &m,
        ModernSetting::Failure {
            component: Component::LaneA,
            failed: true,
        },
    );
    advance(&mut s, &m, r, 1);
    assert!(s.engine_running);
    assert_eq!(s.engine_power_fraction, 0.96);
    assert!(s.modern.as_ref().unwrap().lane_a.light);
    action(
        &mut s,
        &m,
        ModernSetting::Failure {
            component: Component::Pump1,
            failed: true,
        },
    );
    advance(&mut s, &m, r, 1);
    assert!(s.engine_running);
    assert_eq!(s.modern.as_ref().unwrap().fuel_pressure_pa, 300000.);
    action(
        &mut s,
        &m,
        ModernSetting::Failure {
            component: Component::Pump2,
            failed: true,
        },
    );
    advance(&mut s, &m, r, 1);
    assert!(!s.engine_running);
    assert!(s.fuel_starved);
    action(
        &mut s,
        &m,
        ModernSetting::Failure {
            component: Component::Pump2,
            failed: false,
        },
    );
    action(
        &mut s,
        &m,
        ModernSetting::Key {
            position: KeyPosition::Start,
        },
    );
    advance(&mut s, &m, r, 250);
    assert!(s.engine_running);
    assert_eq!(s.carb_ice, 0.);
    assert_eq!(s.primer_charge, 0.);
    action(
        &mut s,
        &m,
        ModernSetting::Key {
            position: KeyPosition::On,
        },
    );
    s.set(
        SystemSetting::Switch {
            switch: SystemSwitch::Master,
            on: false,
        },
        Some(&m),
    )
    .unwrap();
    advance(&mut s, &m, r, 120);
    assert!(s.engine_running);
    assert!(s.electrical.loads.iter().any(|l| l.id == "lane_b" && l.on_backup));
    assert!(s.set(SystemSetting::Primer { charge: 1. }, Some(&m)).is_err());
    assert!(
        s.set(
            SystemSetting::Magnetos {
                position: Magnetos::Both
            },
            Some(&m)
        )
        .is_err()
    );
}
#[test]
fn dual_lane_loss_and_selector_off_stop_combustion() {
    let (mut s, m, r) = bench();
    for lane in [Lane::A, Lane::B] {
        action(&mut s, &m, ModernSetting::Lane { lane, on: false });
    }
    advance(&mut s, &m, r, 1);
    assert!(!s.engine_running);
    let (mut s, m, r) = bench();
    s.set(
        SystemSetting::FuelSelector {
            position: FuelSelector::Off,
        },
        Some(&m),
    )
    .unwrap();
    advance(&mut s, &m, r, 1);
    assert!(!s.engine_running);
    assert!(s.fuel_starved);
    assert_eq!(s.engine_power_fraction, 0.);
    assert!(
        s.set(
            SystemSetting::FuelSelector {
                position: FuelSelector::Both
            },
            Some(&m)
        )
        .is_err()
    );
}
#[test]
fn master_latches_until_push_and_rearms_on_new_edges() {
    let (mut s, m, r) = bench();
    action(&mut s, &m, ModernSetting::Door { open: true });
    advance(&mut s, &m, r, 1);
    assert!(s.modern.as_ref().unwrap().master_caution);
    action(&mut s, &m, ModernSetting::Door { open: false });
    advance(&mut s, &m, r, 1);
    assert!(s.modern.as_ref().unwrap().master_caution);
    action(
        &mut s,
        &m,
        ModernSetting::Acknowledge {
            severity: Severity::Caution,
        },
    );
    advance(&mut s, &m, r, 1);
    assert!(!s.modern.as_ref().unwrap().master_caution);
    action(&mut s, &m, ModernSetting::Door { open: true });
    advance(&mut s, &m, r, 1);
    assert!(s.modern.as_ref().unwrap().master_caution);
    action(
        &mut s,
        &m,
        ModernSetting::Acknowledge {
            severity: Severity::Caution,
        },
    );
    advance(&mut s, &m, r, 120);
    assert!(!s.modern.as_ref().unwrap().master_caution);
}
#[test]
fn breaker_pull_reset_and_pop() {
    let (mut s, m, r) = bench();
    s.set(
        SystemSetting::Breaker {
            load: "touch_left".into(),
            closed: false,
        },
        Some(&m),
    )
    .unwrap();
    advance(&mut s, &m, r, 1);
    assert!(
        !s.electrical
            .loads
            .iter()
            .find(|l| l.id == "touch_left")
            .unwrap()
            .powered
    );
    s.set(
        SystemSetting::Breaker {
            load: "touch_left".into(),
            closed: true,
        },
        Some(&m),
    )
    .unwrap();
    s.set(
        SystemSetting::LoadMultiplier {
            load: "touch_left".into(),
            value: 8.,
        },
        Some(&m),
    )
    .unwrap();
    advance(&mut s, &m, r, 120);
    let l = s.electrical.loads.iter().find(|l| l.id == "touch_left").unwrap();
    assert!(l.tripped);
    assert!(!l.breaker_closed);
}
#[test]
fn recovery_pin_hold_cancellation_delay_inflation_and_pilot_only() {
    let (mut s, m, r) = bench();
    assert!(
        s.set(
            SystemSetting::Equipment {
                setting: ModernSetting::ParachuteDeploy
            },
            Some(&m)
        )
        .is_err()
    );
    advance(&mut s, &m, r, 1000);
    assert_eq!(
        s.modern.as_ref().unwrap().recovery.as_ref().unwrap().phase,
        RecoveryPhase::Stowed
    );
    action(&mut s, &m, ModernSetting::ParachutePin { inserted: false });
    action(&mut s, &m, ModernSetting::ParachuteHandle { held: true });
    advance(&mut s, &m, r, 120);
    assert!(
        (s.modern
            .as_ref()
            .unwrap()
            .recovery
            .as_ref()
            .unwrap()
            .hold_progress
            - 0.5)
            .abs()
            < 1e-9
    );
    action(&mut s, &m, ModernSetting::ParachuteHandle { held: false });
    advance(&mut s, &m, r, 120);
    assert_eq!(
        s.modern
            .as_ref()
            .unwrap()
            .recovery
            .as_ref()
            .unwrap()
            .hold_progress,
        0.
    );
    action(&mut s, &m, ModernSetting::ParachuteHandle { held: true });
    advance(&mut s, &m, r, 240);
    assert_eq!(
        s.modern.as_ref().unwrap().recovery.as_ref().unwrap().phase,
        RecoveryPhase::Deploying
    );
    advance(&mut s, &m, r, 30);
    assert_eq!(
        s.modern.as_ref().unwrap().recovery.as_ref().unwrap().inflation,
        0.
    );
    advance(&mut s, &m, r, 150);
    let p = s.modern.as_ref().unwrap().recovery.as_ref().unwrap();
    assert_eq!(p.phase, RecoveryPhase::Inflating);
    assert!((p.inflation - 0.5).abs() < 1e-8);
    advance(&mut s, &m, r, 150);
    assert_eq!(
        s.modern.as_ref().unwrap().recovery.as_ref().unwrap().phase,
        RecoveryPhase::Descending
    );
}
#[test]
fn recovery_descent_attitude_impact_and_batching() {
    let c = resolved();
    let mut sim = Sim::new_with_aircraft(
        &Start::Air {
            height_m: 1000.,
            airspeed_mps: 40.,
            heading_rad: 0.,
        },
        c.aircraft.clone(),
    )
    .unwrap();
    sim.configure_systems(&c).unwrap();
    for setting in [
        ModernSetting::ParachutePin { inserted: false },
        ModernSetting::ParachuteDeploy,
    ] {
        sim.set_systems(SystemSetting::Equipment { setting }).unwrap();
    }
    let mut batched = sim.clone();
    sim.step(1000);
    for _ in 0..1000 {
        batched.step(1);
    }
    assert_eq!(sim.state(), batched.state());
    assert_eq!(sim.systems(), batched.systems());
    assert!(!sim.readout().crashed);
    assert_eq!(sim.state().velocity_mps, [0., 0., 7.]);
    assert!((sim.readout().pitch_rad + 0.3).abs() < 1e-12);
    assert!(
        (sim.readout().engine_rpm / sim.systems().modern.as_ref().unwrap().propeller_rpm - 2.4).abs() < 1e-12
    );
    sim.step(20000);
    let p = sim.systems().modern.as_ref().unwrap().recovery.as_ref().unwrap();
    assert_eq!(p.phase, RecoveryPhase::Impact);
    assert_eq!(p.impact_vertical_speed_mps, Some(7.));
}

#[test]
fn either_lane_runs_alone_and_key_off_requires_start() {
    for failed in [Component::LaneA, Component::LaneB] {
        let (mut s, m, r) = bench();
        action(
            &mut s,
            &m,
            ModernSetting::Failure {
                component: failed,
                failed: true,
            },
        );
        advance(&mut s, &m, r, 1);
        assert!(s.engine_running);
        assert_eq!(s.engine_power_fraction, 0.96);
        action(
            &mut s,
            &m,
            ModernSetting::Key {
                position: KeyPosition::Off,
            },
        );
        advance(&mut s, &m, r, 1);
        assert!(!s.engine_running);
        action(
            &mut s,
            &m,
            ModernSetting::Key {
                position: KeyPosition::On,
            },
        );
        advance(&mut s, &m, r, 300);
        assert!(!s.engine_running);
        action(
            &mut s,
            &m,
            ModernSetting::Key {
                position: KeyPosition::Start,
            },
        );
        advance(&mut s, &m, r, 250);
        assert!(s.engine_running);
    }
}

#[test]
fn declared_controls_guards_and_independent_lights_are_command_state() {
    let (mut s, m, r) = bench();
    assert!(
        s.set(
            SystemSetting::Equipment {
                setting: ModernSetting::PanelControl {
                    id: "toggle_aux".into(),
                    value: 1.
                }
            },
            Some(&m)
        )
        .is_err()
    );
    action(
        &mut s,
        &m,
        ModernSetting::PanelGuard {
            id: "toggle_aux".into(),
            open: true,
        },
    );
    action(
        &mut s,
        &m,
        ModernSetting::PanelControl {
            id: "toggle_aux".into(),
            value: 1.,
        },
    );
    assert_eq!(
        s.modern.as_ref().unwrap().panel_controls.get("toggle_aux"),
        Some(&1.)
    );
    assert!(
        s.set(
            SystemSetting::Equipment {
                setting: ModernSetting::PanelControl {
                    id: "undeclared".into(),
                    value: 1.
                }
            },
            Some(&m)
        )
        .is_err()
    );
    action(
        &mut s,
        &m,
        ModernSetting::Light {
            light: flightcraft_aero::LoadSwitch::Landing,
            on: true,
        },
    );
    advance(&mut s, &m, r, 1);
    assert!(s.electrical.loads.iter().any(|l| l.id == "landing" && l.powered));
    assert!(!s.electrical.loads.iter().any(|l| l.id == "taxi" && l.powered));
}

#[test]
fn single_message_acknowledge_keeps_the_message_and_clears_the_lamp_with_the_last() {
    let (mut s, m, r) = bench();
    action(&mut s, &m, ModernSetting::Door { open: true });
    advance(&mut s, &m, r, 1);
    let door = s
        .modern
        .as_ref()
        .unwrap()
        .annunciations
        .iter()
        .position(|a| a.active)
        .unwrap();
    assert!(!s.modern.as_ref().unwrap().annunciations[door].acknowledged);
    assert!(s.modern.as_ref().unwrap().annunciations[door].sequence > 0);
    action(&mut s, &m, ModernSetting::AcknowledgeMessage { index: door });
    let state = s.modern.as_ref().unwrap();
    assert!(state.annunciations[door].acknowledged && state.annunciations[door].active);
    assert!(!state.master_caution);
    // It clears, and the next time it appears it is unacknowledged again.
    action(&mut s, &m, ModernSetting::Door { open: false });
    advance(&mut s, &m, r, 1);
    assert!(!s.modern.as_ref().unwrap().annunciations[door].acknowledged);
    assert!(
        s.set(
            SystemSetting::Equipment {
                setting: ModernSetting::AcknowledgeMessage { index: door }
            },
            Some(&m)
        )
        .is_err()
    );
}
