#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::{Session, read_region};
use flightcraft_sim::{
    Sim, Start,
    autopilot::wrap,
    equipment::{EquipmentFailure, LateralMode, PanelSetting, VerticalMode},
    nav_source::{DirectTo, NavSource},
    systems::SystemSetting,
};
use flightcraft_world::{World, geo::LocalProjection};
use serde_json::json;
use std::sync::OnceLock;

fn world() -> World {
    static WORLD: OnceLock<World> = OnceLock::new();
    WORLD
        .get_or_init(|| {
            read_region(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york"),
            )
            .unwrap()
        })
        .clone()
}
fn flight(panel: &str) -> Sim {
    let catalog = flightcraft_aero::AircraftCatalog::builtins().unwrap();
    let load = catalog.resolve_aircraft("corvid").unwrap().load;
    let configuration = catalog.resolve("corvid-standard", panel, &load).unwrap();
    let world = world();
    let heading = flightcraft_sim::magnetic::reference_at(&world, [0., 0., -1000.], Default::default())
        .unwrap()
        .magnetic_to_grid(0.);
    let mut sim = Sim::new_with_aircraft_weather_world(
        &Start::AirAt {
            position_m: [0.0, 0.0, -1000.0],
            airspeed_mps: 40.0,
            heading_rad: heading,
        },
        configuration.aircraft.clone(),
        Default::default(),
        world,
    )
    .unwrap();
    sim.configure_systems(&configuration).unwrap();
    sim
}
fn set(sim: &mut Sim, setting: PanelSetting) {
    sim.set_autopilot_panel(setting).unwrap();
}
fn engage(sim: &mut Sim) {
    set(sim, PanelSetting::Engage { on: true });
}
fn step(sim: &mut Sim, seconds: u32) {
    sim.step(seconds * 120);
    assert!(!sim.readout().crashed, "{:?}", sim.readout());
}

#[test]
fn shared_heading_bug_moves_short_way_at_bank_limit_and_settles() {
    require_region_data!();
    for panel in ["corvid-legacy", "corvid-digital"] {
        let mut sim = flight(panel);
        engage(&mut sim);
        set(
            &mut sim,
            PanelSetting::Lateral {
                mode: LateralMode::Hdg,
            },
        );
        set(
            &mut sim,
            PanelSetting::Vertical {
                mode: VerticalMode::Alt,
            },
        );
        for bug in [90.0_f64.to_radians(), 270.0_f64.to_radians()] {
            let expected = wrap(bug - sim.systems().sensed.heading_rad).signum();
            sim.set_systems(SystemSetting::HeadingBug { heading_rad: bug })
                .unwrap();
            let previous = sim.readout().heading_rad;
            step(&mut sim, 12);
            let state = sim.installed_autopilot().unwrap();
            assert_eq!(state.command_bank_rad.signum(), expected, "{panel}: {state:?}");
            assert!(
                wrap(sim.readout().heading_rad - previous) * expected > 0.1,
                "{panel}: {:?}",
                sim.readout()
            );
            assert!(
                (sim.readout().roll_rad.abs() - 25.0_f64.to_radians()).abs() < 6.0_f64.to_radians(),
                "{panel}: {:?}",
                sim.readout()
            );
            for _ in 0..108 {
                step(&mut sim, 1);
            }
            let r = sim.readout();
            println!(
                "{panel} heading bug {bug}: hdg={} alt={} roll={}",
                r.heading_rad, r.altitude_msl_m, r.roll_rad
            );
            assert!(
                wrap(r.heading_rad - bug).abs() < 3.0_f64.to_radians(),
                "{panel}: {r:?}"
            );
            assert!(
                (r.altitude_msl_m - 1000.0).abs() < 50.0 * 0.3048,
                "{panel}: {r:?}"
            );
        }
    }
}
#[test]
fn altitude_capture_and_vs_track() {
    require_region_data!();
    for panel in ["corvid-legacy", "corvid-digital"] {
        let mut sim = flight(panel);
        engage(&mut sim);
        let mut controls = sim.controls();
        controls.throttle = 0.9;
        sim.set_controls(controls);
        set(
            &mut sim,
            PanelSetting::Vertical {
                mode: VerticalMode::Vs,
            },
        );
        set(
            &mut sim,
            PanelSetting::VerticalSpeed {
                vertical_speed_mps: 2.0,
            },
        );
        for second in 0..70 {
            step(&mut sim, 1);
            if second >= 50 {
                assert!(
                    (sim.readout().vertical_speed_mps - 2.0).abs() < 100.0 * 0.00508,
                    "{panel}: {:?}",
                    sim.readout()
                );
            }
        }
        assert!(
            (sim.readout().vertical_speed_mps - 2.0).abs() < 100.0 * 0.00508,
            "{panel}: {:?}",
            sim.readout()
        );
        let target = sim.readout().altitude_msl_m + 100.0;
        set(&mut sim, PanelSetting::Altitude { altitude_m: target });
        let mut overshoot = 0.0_f64;
        for _ in 0..150 {
            step(&mut sim, 1);
            overshoot = overshoot.max(sim.readout().altitude_msl_m - target);
        }
        println!(
            "{panel} capture target={target}, alt={}, overshoot={overshoot}",
            sim.readout().altitude_msl_m
        );
        assert_eq!(
            sim.installed_autopilot().unwrap().active_vertical,
            Some(VerticalMode::Alt)
        );
        assert!(overshoot < 100.0 * 0.3048);
        assert!(
            (sim.readout().altitude_msl_m - target).abs() < 50.0 * 0.3048,
            "{panel}: {:?}",
            sim.readout()
        );
    }
}
#[test]
fn ias_climb_uses_pitch_and_pilot_power() {
    require_region_data!();
    let mut sim = flight("corvid-digital");
    engage(&mut sim);
    let mut controls = sim.controls();
    controls.throttle = 1.0;
    sim.set_controls(controls);
    set(
        &mut sim,
        PanelSetting::Vertical {
            mode: VerticalMode::Ias,
        },
    );
    set(&mut sim, PanelSetting::Airspeed { airspeed_mps: 36.0 });
    for second in 0..150 {
        step(&mut sim, 1);
        if second > 90 {
            assert!(
                (sim.readout().indicated_airspeed_mps - 36.0).abs() < 5.0 * 0.514444,
                "{:?}",
                sim.readout()
            );
            assert!(sim.readout().vertical_speed_mps > 0.0);
            assert_eq!(sim.readout().controls.throttle, 1.0);
        }
    }
}
#[test]
fn gps_direct_to_kmmu_tracks_inside_tenth_mile() {
    require_region_data!();
    let mut sim = flight("corvid-digital");
    let world = world();
    let region = world.region().unwrap();
    let airport = world
        .airports()
        .unwrap()
        .airports
        .iter()
        .find(|a| a.icao.as_deref() == Some("KMMU"))
        .unwrap();
    let destination = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
        .unwrap()
        .project(airport.latitude_deg, airport.longitude_deg)
        .unwrap();
    let source = DirectTo::new([0.0, 0.0], destination).unwrap();
    sim.set_direct_to(destination).unwrap();
    engage(&mut sim);
    set(
        &mut sim,
        PanelSetting::Vertical {
            mode: VerticalMode::Alt,
        },
    );
    set(
        &mut sim,
        PanelSetting::Lateral {
            mode: LateralMode::Nav,
        },
    );
    for second in 0..300 {
        step(&mut sim, 1);
        let [n, e, _] = sim.state().position_m;
        let nav = source.guidance(n, e).unwrap();
        if second > 150 {
            assert!(
                nav.cross_track_m.abs() < 185.2,
                "xtk={} {:?}",
                nav.cross_track_m,
                sim.readout()
            );
        }
    }
}
#[test]
fn failures_disconnect_and_legacy_roll_dies_with_turn_sensor() {
    require_region_data!();
    for panel in ["corvid-leveler", "corvid-legacy", "corvid-digital"] {
        for failure in [EquipmentFailure::RollServo, EquipmentFailure::RollSensor] {
            let mut sim = flight(panel);
            engage(&mut sim);
            set(
                &mut sim,
                PanelSetting::Failure {
                    failure,
                    failed: true,
                },
            );
            step(&mut sim, 1);
            let s = sim.installed_autopilot().unwrap();
            assert!(!s.engaged && s.active_lateral.is_none() && !s.roll_servo_engaged);
            assert!(s.ap_flash && s.disconnect_tone);
        }
    }
    let mut sim = flight("corvid-legacy");
    engage(&mut sim);
    sim.set_systems(SystemSetting::Breaker {
        load: "turn_coordinator".into(),
        closed: false,
    })
    .unwrap();
    step(&mut sim, 1);
    let state = sim.installed_autopilot().unwrap();
    assert!(!state.engaged && state.active_lateral.is_none());
}
#[test]
fn pilot_overpowers_clutch_and_trim_runs_or_warns_when_failed() {
    require_region_data!();
    let mut sim = flight("corvid-digital");
    engage(&mut sim);
    set(
        &mut sim,
        PanelSetting::Vertical {
            mode: VerticalMode::Vs,
        },
    );
    set(
        &mut sim,
        PanelSetting::VerticalSpeed {
            vertical_speed_mps: 3.0,
        },
    );
    let mut controls = sim.controls();
    controls.throttle = 1.0;
    sim.set_controls(controls);
    let mut trimmed = false;
    for _ in 0..30 {
        step(&mut sim, 1);
        trimmed |= sim.installed_autopilot().unwrap().trim_in_motion;
    }
    assert!(trimmed);
    let mut jammed = flight("corvid-digital");
    engage(&mut jammed);
    set(
        &mut jammed,
        PanelSetting::Failure {
            failure: EquipmentFailure::Trim,
            failed: true,
        },
    );
    set(
        &mut jammed,
        PanelSetting::Vertical {
            mode: VerticalMode::Vs,
        },
    );
    set(
        &mut jammed,
        PanelSetting::VerticalSpeed {
            vertical_speed_mps: 5.0,
        },
    );
    let mut warned = false;
    for _ in 0..20 {
        step(&mut jammed, 1);
        warned |= jammed.installed_autopilot().unwrap().mistrim;
    }
    assert!(warned, "{:?}", jammed.installed_autopilot());
    let mut controls = sim.controls();
    controls.aileron = 1.0;
    sim.set_controls(controls);
    sim.step(1);
    let state = sim.installed_autopilot().unwrap();
    assert!(state.pilot_overpower && !state.roll_servo_engaged);
    assert_eq!(sim.readout().controls.aileron, 1.0);
}
#[test]
fn panel_commands_schemas_aliases_and_sync_use_one_bug() {
    let mut s = Session::new().unwrap();
    let load = s.aircraft_configuration().unwrap().load.clone();
    s.execute(
        "aircraft.load",
        json!({"variant":"corvid-standard","panel":"corvid-digital","load":load,"reset":true}),
    )
    .unwrap();
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"air","height_m":1000.0,"airspeed_mps":40.0,"heading_rad":1.4}}),
    )
    .unwrap();
    for input in [
        json!({"setting":{"kind":"heading_bug_sync"}}),
        json!({"setting":{"kind":"autopilot","setting":{"kind":"engage","on":true}}}),
    ] {
        let info = Session::commands()
            .into_iter()
            .find(|c| c.name == "avionics.set")
            .unwrap();
        assert!(
            jsonschema::validator_for(&info.input_schema)
                .unwrap()
                .is_valid(&input)
        );
        let out = s.execute("avionics.set", input).unwrap();
        assert!(
            jsonschema::validator_for(&info.output_schema)
                .unwrap()
                .is_valid(&out)
        );
    }
    assert!((s.sim().systems().controls.heading_bug_rad - 1.4).abs() < 0.5_f64.to_radians());
    let before = s.execute("autopilot.state", json!({})).unwrap();
    for input in [
        json!({"setting":{"kind":"autopilot","setting":{"kind":"altitude","altitude_m":-1.0}}}),
        json!({"setting":{"kind":"autopilot","setting":{"kind":"lateral","mode":"made_up"}}}),
        json!({"setting":{"kind":"autopilot","setting":{"kind":"engage","on":true,"extra":1}}}),
    ] {
        assert!(s.execute("avionics.set", input).is_err());
        assert_eq!(s.execute("autopilot.state", json!({})).unwrap(), before);
    }
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"autopilot","setting":{"kind":"disconnect"}}}),
    )
    .unwrap();
    assert_eq!(
        s.execute("assist.set", json!({"heading_rad":0.5})).unwrap(),
        s.execute("autopilot.set", json!({"heading_rad":0.5})).unwrap()
    );
    assert_eq!(
        s.execute("assist.off", json!({})).unwrap(),
        s.execute("autopilot.off", json!({})).unwrap()
    );
}

#[test]
fn lvl_recovers_forty_bank_and_ten_down_in_ten_seconds() {
    require_region_data!();
    let initial = flight("corvid-digital");
    let mut state = *initial.state();
    let (roll, pitch) = (40.0_f64.to_radians(), -10.0_f64.to_radians());
    state.attitude = [
        libm::cos(roll / 2.0) * libm::cos(pitch / 2.0),
        libm::sin(roll / 2.0) * libm::cos(pitch / 2.0),
        libm::cos(roll / 2.0) * libm::sin(pitch / 2.0),
        -libm::sin(roll / 2.0) * libm::sin(pitch / 2.0),
    ];
    let catalog = flightcraft_aero::AircraftCatalog::builtins().unwrap();
    let load = catalog.resolve_aircraft("corvid").unwrap().load;
    let config = catalog
        .resolve("corvid-standard", "corvid-digital", &load)
        .unwrap();
    let mut sim = Sim::from_state_in_world(state, initial.controls(), world()).unwrap();
    sim.configure_systems(&config).unwrap();
    set(&mut sim, PanelSetting::Level);
    step(&mut sim, 10);
    assert!(
        sim.readout().roll_rad.abs() < 3.0_f64.to_radians(),
        "{:?}",
        sim.readout()
    );
    assert!(
        sim.readout().vertical_speed_mps.abs() < 100.0 * 0.00508,
        "{:?}",
        sim.readout()
    );
}

#[test]
fn equipment_memory_hashes_and_fixed_step_batching_match() {
    require_region_data!();
    let mut a = flight("corvid-digital");
    let original = a.state_hash();
    engage(&mut a);
    assert_ne!(a.state_hash(), original);
    set(
        &mut a,
        PanelSetting::Vertical {
            mode: VerticalMode::Alt,
        },
    );
    let mut b = a.clone();
    a.step(1200);
    for _ in 0..1200 {
        b.step(1);
    }
    assert_eq!(a.readout(), b.readout());
    assert_eq!(a.installed_autopilot(), b.installed_autopilot());
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn wing_leveler_turns_without_taking_pitch_or_throttle() {
    require_region_data!();
    let mut sim = flight("corvid-leveler");
    let pilot = sim.controls();
    engage(&mut sim);
    set(&mut sim, PanelSetting::Turn { rate_rps: 0.03 });
    step(&mut sim, 40);
    let r = sim.readout();
    assert!((r.turn_rate_rps - 0.03).abs() < 0.01, "{r:?}");
    assert_eq!(r.controls.elevator, pilot.elevator);
    assert_eq!(r.controls.throttle, pilot.throttle);
    assert!(!sim.installed_autopilot().unwrap().pitch_servo_engaged);
}

#[test]
fn receiver_test_double_arms_captures_and_disconnects_on_lost_signal() {
    use flightcraft_sim::nav_source::{NavGuidance, NavKind, ToFrom};
    struct Receiver(Option<NavGuidance>);
    impl NavSource for Receiver {
        fn guidance(&self, _north: f64, _east: f64) -> Option<NavGuidance> {
            self.0
        }
    }
    let mut sim = flight("corvid-digital");
    engage(&mut sim);
    set(
        &mut sim,
        PanelSetting::Lateral {
            mode: LateralMode::Nav,
        },
    );
    sim.step_with_nav_source(1, &Receiver(None));
    assert_eq!(
        sim.installed_autopilot().unwrap().armed_lateral,
        Some(LateralMode::Nav)
    );
    let receiver = Receiver(Some(NavGuidance {
        desired_track_rad: 0.0,
        cross_track_m: 10.0,
        to_from: ToFrom::To,
        kind: NavKind::Gps,
        vertical_deviation_m: None,
        path_angle_rad: None,
    }));
    sim.step_with_nav_source(1, &receiver);
    assert_eq!(
        sim.installed_autopilot().unwrap().active_lateral,
        Some(LateralMode::Nav)
    );
    sim.step_with_nav_source(1, &Receiver(None));
    let state = sim.installed_autopilot().unwrap();
    assert!(!state.engaged && state.ap_flash && state.disconnect_tone);
}
