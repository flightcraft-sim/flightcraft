#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use super::*;
#[test]
fn terrain_clearance_thresholds() {
    assert_eq!(terrain_band(3000. * 0.3048, 1999. * 0.3048), TerrainBand::Clear);
    assert_eq!(terrain_band(3000. * 0.3048, 2000. * 0.3048), TerrainBand::Caution);
    assert_eq!(terrain_band(3000. * 0.3048, 2900. * 0.3048), TerrainBand::Warning);
    assert_eq!(terrain_band(3000. * 0.3048, 4000. * 0.3048), TerrainBand::Warning);
}
#[test]
fn commands_validate_atomically_and_install_glass() {
    let mut session = Session::new().unwrap();
    session
        .execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
        )
        .unwrap();
    assert!(!session.sim.systems().vacuum_installed);
    assert!(
        session
            .set_glass(GlassSetting::Transponder {
                code: "1280".into(),
                mode: TransponderMode::Alt
            })
            .is_err()
    );
    assert_eq!(session.glass_state().settings.transponder_code, "1200");
    session
        .set_glass(GlassSetting::Transponder {
            code: "0456".into(),
            mode: TransponderMode::On,
        })
        .unwrap();
    assert_eq!(session.glass_state().settings.transponder_code, "0456");
    assert!(
        session
            .set_glass(GlassSetting::MapRange { range_nm: f64::NAN })
            .is_err()
    );
}
fn regional_session() -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(crate::read_region(&directory).unwrap()).unwrap();
    s.execute("sim.reset",serde_json::json!({"world":"new-york","start":{"kind":"air","height_m":914.4,"airspeed_mps":50.0,"heading_rad":1.0}})).unwrap();
    s
}
// Independent vector central-angle oracle; no production geodesy is called here.
fn oracle(a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let [a, b1] = a.map(f64::to_radians);
    let [c, d] = b.map(f64::to_radians);
    let dot = a.sin() * c.sin() + a.cos() * c.cos() * (d - b1).cos();
    let distance = dot.clamp(-1., 1.).acos() * 6_371_008.8 / 1852.;
    let bearing = ((d - b1).sin() * c.cos())
        .atan2(a.cos() * c.sin() - a.sin() * c.cos() * (d - b1).cos())
        .rem_euclid(std::f64::consts::TAU);
    (distance, bearing)
}
#[test]
fn new_york_leg_math_scaling_airspace_and_snapshot_schema() {
    require_region_data!();
    let mut s = regional_session();
    let world = s.sim.world();
    let a = crate::regions::airport(world, "1N7").unwrap();
    let b = crate::regions::airport(world, "KMMU").unwrap();
    let start = [a.latitude_deg, a.longitude_deg];
    let end = [b.latitude_deg, b.longitude_deg];
    let expected = oracle(start, end);
    s.set_glass(GlassSetting::FlightPlan {
        airports: vec!["1N7".into(), "KMMU".into()],
    })
    .unwrap();
    s.set_glass(GlassSetting::ActiveLeg { index: 1 }).unwrap();
    let state = s.glass_state();
    let leg = state.gps.legs.get(1).unwrap();
    assert!((leg.distance_nm - expected.0).abs() < 0.1);
    assert!(
        (leg.bearing_rad - s.sim.heading_reference().true_to_magnetic(expected.1))
            .to_degrees()
            .abs()
            < 1.
    );
    assert!(state.outlines.iter().any(
        |o| o.class == Some(flightcraft_world::AirspaceClass::B) && o.rings.iter().any(|r| r.len() > 10)
    ));
    assert!(state.terrain.len() > 16000);
    assert_eq!(state.airports.len(), 26);
    let position = s
        .sim
        .world()
        .geodetic(s.sim.state().position_m[0], s.sim.state().position_m[1])
        .unwrap();
    let distance = oracle(position, end).0;
    assert!((state.gps.distance_nm - distance).abs() < 0.1);
    assert_eq!(state.gps.cdi_full_scale_nm, if distance < 30. { 1. } else { 2. });
    // Right-of-track case: latitude shifted south of an eastward leg gives a left correction.
    let (xt, _) = track(start, end, [start[0] - 0.01, start[1]]);
    let angular = oracle(start, [start[0] - 0.01, start[1]]).0 * 1852. / 6_371_008.8;
    let bearing = oracle(start, [start[0] - 0.01, start[1]]).1;
    let expected_xt = (angular.sin() * (bearing - expected.1).sin()).asin() * 6_371_008.8 / 1852.;
    assert!((xt / 1852. - expected_xt).abs() < 0.001);
    let info = Session::commands()
        .into_iter()
        .find(|c| c.name == "avionics.state")
        .unwrap();
    let out = s.execute("avionics.state", serde_json::json!({})).unwrap();
    assert!(
        jsonschema::validator_for(&info.output_schema)
            .unwrap()
            .is_valid(&out)
    );
}
#[test]
fn independent_failure_reversion_and_backup_endurance() {
    require_region_data!();
    let mut s = regional_session();
    s.sim.step(5401);
    assert!(s.glass_state().air_data.attitude_valid);
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Ahrs,
        failed: true,
    })
    .unwrap();
    let state = s.glass_state();
    assert!(!state.air_data.attitude_valid && state.air_data.air_data_valid);
    assert!(state.standby.attitude_valid);
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Adc,
        failed: true,
    })
    .unwrap();
    assert!(!s.glass_state().air_data.air_data_valid);
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Mfd,
        failed: true,
    })
    .unwrap();
    assert!(s.glass_state().reversion);
    s.sim
        .set_systems(flightcraft_sim::systems::SystemSetting::Switch {
            switch: flightcraft_sim::systems::SystemSwitch::Master,
            on: false,
        })
        .unwrap();
    // Electrical state changes on a fixed step, including independent standby power.
    s.sim.step(1);
    let standby = s
        .sim
        .systems()
        .electrical
        .loads
        .iter()
        .find(|l| l.id == "glass_standby")
        .unwrap();
    assert!(standby.on_backup && standby.powered);
    assert!(s.glass_state().standby.attitude_valid);
}
#[test]
fn blocked_ports_remain_unflagged_and_freeze_when_both_are_sealed() {
    require_region_data!();
    let mut s = regional_session();
    s.sim.step(10);
    s.execute(
        "failure.set",
        serde_json::json!({"failure":{"kind":"pitot_blocked"}}),
    )
    .unwrap();
    s.execute(
        "failure.set",
        serde_json::json!({"failure":{"kind":"static_blocked"}}),
    )
    .unwrap();
    let before = s.sim.systems().sensed;
    s.sim.step(120);
    let state = s.glass_state();
    assert!(state.air_data.air_data_valid);
    assert!((state.air_data.indicated_airspeed_mps - before.indicated_airspeed_mps).abs() < 1e-9);
    assert!((state.air_data.altitude_msl_m - before.altitude_msl_m).abs() < 1e-9);
    assert_eq!(state.air_data.vertical_speed_mps, 0.);
}
#[test]
fn actual_cdi_matches_signed_geodesic_and_terminal_transition() {
    require_region_data!();
    let mut s = regional_session();
    s.set_glass(GlassSetting::FlightPlan {
        airports: vec!["1N7".into(), "KMMU".into()],
    })
    .unwrap();
    s.set_glass(GlassSetting::ActiveLeg { index: 1 }).unwrap();
    let a = crate::regions::airport(s.sim.world(), "1N7").unwrap();
    let b = crate::regions::airport(s.sim.world(), "KMMU").unwrap();
    let start = [a.latitude_deg, a.longitude_deg];
    let end = [b.latitude_deg, b.longitude_deg];
    let course = oracle(start, end).1;
    let origin = s.sim.world().region().unwrap().origin.clone();
    let projection =
        flightcraft_world::geo::LocalProjection::new(origin.latitude_deg, origin.longitude_deg).unwrap();
    for position in [[40.90, -74.95], [40.82, -74.60], [40.92, -74.60]] {
        let mut state = *s.sim.state();
        let [n, e] = projection.project(position[0], position[1]).unwrap();
        state.position_m = [n, e, -914.4];
        s.sim = flightcraft_sim::Sim::from_state_in_world(state, s.sim.controls(), s.sim.world().clone())
            .unwrap();
        let result = s.glass_state().gps;
        let (distance, bearing) = oracle(start, position);
        let expected_xt = (distance * 1852. / 6_371_008.8)
            .sin()
            .mul_add((bearing - course).sin(), 0.)
            .asin()
            * 6_371_008.8
            / 1852.;
        let full_scale = if oracle(position, end).0 < 30. { 1. } else { 2. };
        assert_eq!(result.cdi_full_scale_nm, full_scale);
        assert!((result.cross_track_nm - expected_xt).abs() < 0.001);
        assert!((result.cdi_deviation - (-expected_xt / full_scale).clamp(-1., 1.)).abs() < 0.001);
    }
}
#[test]
fn glass_settings_validate_against_schemas_and_queries_do_not_advance_sensors() {
    require_region_data!();
    let mut s = regional_session();
    let before = s.sim.glass_computers().clone();
    let hash = s.sim.state_hash();
    for _ in 0..4 {
        s.execute("avionics.state", serde_json::json!({})).unwrap();
    }
    assert_eq!(s.sim.glass_computers(), &before);
    assert_eq!(s.sim.state_hash(), hash);
    s.set_glass(GlassSetting::Failure {
        target: GlassFailure::Ahrs,
        failed: true,
    })
    .unwrap();
    assert_ne!(s.sim.state_hash(), hash);
    let info = Session::commands()
        .into_iter()
        .find(|c| c.name == "avionics.set")
        .unwrap();
    let validator = jsonschema::validator_for(&info.input_schema).unwrap();
    for setting in [
        serde_json::json!({"kind":"map_range","range_nm":0}),
        serde_json::json!({"kind":"transponder","code":"1280","mode":"alt"}),
    ] {
        assert!(!validator.is_valid(&serde_json::json!({"setting":{"kind":"glass","setting":setting}})));
    }
}
fn glass_session() -> Session {
    let mut s = Session::new().unwrap();
    s.execute(
        "aircraft.load",
        serde_json::json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    )
    .unwrap();
    s
}
#[test]
fn minimums_are_validated_stored_and_removed() {
    let mut s = glass_session();
    assert_eq!(s.glass_state().settings.minimums_m, None);
    assert!(
        s.set_glass(GlassSetting::Minimums { altitude_m: f64::NAN })
            .is_err()
    );
    assert!(
        s.set_glass(GlassSetting::Minimums { altitude_m: 20000. })
            .is_err()
    );
    assert_eq!(s.glass_state().settings.minimums_m, None);
    s.set_glass(GlassSetting::Minimums { altitude_m: 76.2 }).unwrap();
    assert_eq!(s.glass_state().settings.minimums_m, Some(76.2));
    s.set_glass(GlassSetting::MinimumsOff {}).unwrap();
    assert_eq!(s.glass_state().settings.minimums_m, None);
    // The tagged JSON a client sends.
    s.execute(
        "avionics.set",
        serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"minimums","altitude_m":300.0}}}),
    )
    .unwrap();
    assert_eq!(s.glass_state().settings.minimums_m, Some(300.));
}
#[test]
fn the_minimums_callout_is_spoken_once_when_the_altitude_is_crossed() {
    let mut s = glass_session();
    s.sim.step(1);
    let altitude = s.sim.glass_computers().primary.altitude_msl_m;
    let spoken = |s: &Session| {
        s.comms
            .record
            .events
            .iter()
            .filter(|e| format!("{e:?}").contains("Minimums."))
            .count()
    };
    // Minimums well below the aircraft: watching, silent.
    s.set_glass(GlassSetting::Minimums {
        altitude_m: altitude - 200.,
    })
    .unwrap();
    for _ in 0..3 {
        s.sequence_minimums();
    }
    assert_eq!(spoken(&s), 0);
    // The aircraft is now below its minimums: one callout, not one per step.
    s.glass.minimums_m = Some(altitude + 20.);
    for _ in 0..5 {
        s.sequence_minimums();
    }
    assert_eq!(spoken(&s), 1);
    // Setting minimums while already below them calls for nothing.
    s.set_glass(GlassSetting::Minimums {
        altitude_m: altitude + 50.,
    })
    .unwrap();
    s.sequence_minimums();
    assert_eq!(spoken(&s), 1);
}
#[test]
fn vspeed_defaults_come_from_the_packs_vspeeds_glide_speed_and_approach_speed() {
    use flightcraft_aero::emergencies::EmergencyProcedure;
    let s = glass_session();
    let mut aircraft = s.aircraft_configuration().unwrap().clone();
    let v = aircraft.variant.v_speeds.clone();
    let defaults = |a: &flightcraft_aero::ResolvedAircraft| vspeed_pack_defaults_mps(a);
    let d = defaults(&aircraft);
    assert_eq!(
        (d[0], d[1], d[2]),
        (Some(v.vr_mps), Some(v.vx_mps), Some(v.vy_mps))
    );
    // Vapp is the approach speed the crew pilot flies: 1.3 Vso.
    assert_eq!(d[4], Some(1.3 * v.vso_mps));
    // Vg is the engine-failure procedure's best glide, else the first procedure's, else none.
    let procedure = |id: &str, speed: f64| EmergencyProcedure {
        id: id.into(),
        title: id.into(),
        best_glide_mps: Some(speed),
        glide_ratio: Some(9.0),
        speed_tolerance_mps: Some(2.0),
        items: Vec::new(),
        note: None,
    };
    aircraft.installed_systems.emergencies.clear();
    assert_eq!(defaults(&aircraft)[3], None);
    aircraft.installed_systems.emergencies = vec![procedure("fire", 40.0), procedure("other", 41.0)];
    assert_eq!(defaults(&aircraft)[3], Some(40.0));
    // A procedure without glide numbers (a ground fire) is skipped.
    let mut ground = procedure("ground_fire", 0.);
    ground.best_glide_mps = None;
    aircraft.installed_systems.emergencies.insert(0, ground);
    assert_eq!(defaults(&aircraft)[3], Some(40.0));
    aircraft.installed_systems.emergencies.remove(0);
    aircraft
        .installed_systems
        .emergencies
        .push(procedure("engine_failure", 32.0));
    assert_eq!(defaults(&aircraft)[3], Some(32.0));
    // With no pack speed the bug is on but draws nothing; the pilot's speed still shows.
    let mut config = flightcraft_sim::avionics::glass_aids::VspeedConfig::default();
    assert_eq!(config.shown_kt(VspeedBug::G, None), None);
    config.entry_mut(VspeedBug::G).speed_kt = Some(68.);
    assert_eq!(config.shown_kt(VspeedBug::G, None), Some(68.));
}
#[test]
fn the_second_bearing_pointer_is_a_gps_or_off_setting() {
    let mut s = glass_session();
    assert_eq!(s.glass_state().settings.bearing_pointer2, BearingPointer::Off);
    s.set_glass(GlassSetting::BearingPointer2 {
        source: BearingPointer::Gps,
    })
    .unwrap();
    let g = s.glass_state().settings;
    assert_eq!(
        (g.bearing_pointer, g.bearing_pointer2),
        (BearingPointer::Off, BearingPointer::Gps)
    );
    // A radio is not an option: no NAV2 is installed, and the setting only knows off and gps.
    assert!(
        s.execute(
            "avionics.set",
            serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"bearing_pointer2","source":"nav2"}}}),
        )
        .is_err()
    );
    s.execute(
        "avionics.set",
        serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"bearing_pointer2","source":"off"}}}),
    )
    .unwrap();
    assert_eq!(s.glass_state().settings.bearing_pointer2, BearingPointer::Off);
}
#[test]
fn the_pitch_offset_is_a_validated_setting_limited_to_two_and_a_half_degrees() {
    let mut s = glass_session();
    assert_eq!(s.glass_state().settings.pitch_offset_deg, 0.);
    for bad in [-2.6, 2.6, f64::NAN] {
        assert!(s.set_glass(GlassSetting::PitchOffset { degrees: bad }).is_err());
    }
    s.set_glass(GlassSetting::PitchOffset { degrees: 1.23 }).unwrap();
    assert_eq!(s.glass_state().settings.pitch_offset_deg, 1.2);
    s.set_glass(GlassSetting::PitchOffset { degrees: -2.5 }).unwrap();
    assert_eq!(s.glass_state().settings.pitch_offset_deg, -2.5);
    s.set_glass(GlassSetting::PitchOffset { degrees: 0. }).unwrap();
    assert_eq!(s.glass_state().settings.pitch_offset_deg, 0.);
}
#[test]
fn the_g_meter_samples_the_load_factor_keeps_extremes_and_resets() {
    let mut s = glass_session();
    s.sim.step(600);
    let g = s.glass_state().g_meter;
    assert!(!g.shown, "off until the pilot turns it on");
    assert!(
        g.current_g.is_finite() && g.min_g <= g.current_g && g.current_g <= g.max_g,
        "{g:?}"
    );
    s.set_glass(GlassSetting::GMeter { shown: true }).unwrap();
    assert!(s.glass_state().g_meter.shown);
    // Sampling folds the load factor in at 10 Hz of simulation time; an excursion stays in the record.
    s.set_glass(GlassSetting::GMeterReset {}).unwrap();
    s.glass.g_track.observe(2.7);
    s.glass.g_track.observe(-0.4);
    s.sim.step(300);
    s.sequence_gmeter();
    let m = s.glass_state().g_meter;
    assert!(m.max_g >= 2.7 && m.min_g <= -0.4);
    // Reset starts over from the current load factor.
    s.set_glass(GlassSetting::GMeterReset {}).unwrap();
    let m = s.glass_state().g_meter;
    assert!(m.max_g < 2. && m.min_g > -0.5, "{m:?}");
    assert!((m.max_g - m.min_g).abs() < 0.2);
}
#[test]
fn the_clock_runs_on_simulation_time_and_can_be_set_and_changed_to_local() {
    let mut s = glass_session();
    let t0 = s.sim.state().time_s;
    let g = s.glass_state();
    assert_eq!(g.clock.label, "UTC");
    assert!((g.clock.seconds - (17. * 3600. + t0)).abs() < 1e-6);
    // Set the clock to 08:30:00 UTC; it then follows simulation time.
    s.set_glass(GlassSetting::ClockSet {
        utc_seconds: 8.5 * 3600.,
    })
    .unwrap();
    assert!((s.glass_state().clock.seconds - 8.5 * 3600.).abs() < 1e-6);
    s.sim.step(600);
    let elapsed = s.sim.state().time_s - t0;
    assert!(elapsed > 1.);
    assert!((s.glass_state().clock.seconds - (8.5 * 3600. + elapsed)).abs() < 1e-6);
    // A bad time is refused and leaves the clock alone.
    for bad in [f64::NAN, -1., 90_000.] {
        assert!(s.set_glass(GlassSetting::ClockSet { utc_seconds: bad }).is_err());
    }
    // Local time differs from UTC by a whole number of hours (the aircraft's time zone).
    let utc = s.glass_state().clock.seconds;
    s.set_glass(GlassSetting::ClockFormat {
        format: ClockFormat::Local,
    })
    .unwrap();
    let local = s.glass_state().clock;
    assert_eq!(local.label, "LCL");
    let hours = (local.seconds - utc) / 3600.;
    assert!((hours - hours.round()).abs() < 1e-6, "{hours}");
}
#[test]
fn the_flight_timer_is_a_glass_setting_that_follows_simulation_time() {
    let mut s = glass_session();
    let timer = |s: &Session| s.glass_state().clock;
    assert!(!timer(&s).timer_running);
    assert_eq!(timer(&s).timer_s, 0.);
    s.execute(
        "avionics.set",
        serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"timer","operation":{"kind":"start"}}}}),
    )
    .unwrap();
    let t0 = s.sim.state().time_s;
    s.sim.step(1200);
    let elapsed = s.sim.state().time_s - t0;
    assert!(timer(&s).timer_running && (timer(&s).timer_s - elapsed).abs() < 1e-6);
    // Stopped, it holds while the simulation runs on.
    s.set_glass(GlassSetting::Timer {
        operation: TimerAction::Stop {},
    })
    .unwrap();
    let held = timer(&s).timer_s;
    s.sim.step(600);
    assert_eq!(timer(&s).timer_s, held);
    s.set_glass(GlassSetting::Timer {
        operation: TimerAction::Reset {},
    })
    .unwrap();
    assert_eq!(timer(&s).timer_s, 0.);
    // Counting down from a preset, and the preset is validated.
    assert!(
        s.set_glass(GlassSetting::Timer {
            operation: TimerAction::Preset { seconds: 0. }
        })
        .is_err()
    );
    s.set_glass(GlassSetting::Timer {
        operation: TimerAction::Preset { seconds: 3. },
    })
    .unwrap();
    s.set_glass(GlassSetting::Timer {
        operation: TimerAction::Direction {
            direction: TimerDirection::Down,
        },
    })
    .unwrap();
    assert_eq!(timer(&s).timer_s, 3.);
    s.set_glass(GlassSetting::Timer {
        operation: TimerAction::Start {},
    })
    .unwrap();
    s.sim.step(120 * 10);
    let c = timer(&s);
    assert!(c.timer_down && c.timer_expired && c.timer_s == 0.);
    // The data bar field choice is a setting too.
    s.set_glass(GlassSetting::DataBarField {
        field: DataBarField::Timer,
    })
    .unwrap();
    assert_eq!(s.glass_state().settings.data_bar_field, DataBarField::Timer);
}

fn vnav_session(constraints: [Option<AltitudeConstraint>; 2]) -> Session {
    let mut s = regional_session();
    s.sim.step(600);
    let [n, e, _] = s.sim.state().position_m;
    let [lat, lon] = s.sim.world().geodetic(n, e).unwrap();
    // Two waypoints due north, 6 and 12 NM out (a degree of latitude is 60 NM).
    let waypoint = |ident: &str, miles: f64, altitude_constraint| GpsWaypoint {
        ident: ident.into(),
        kind: WaypointKind::Fix,
        position: flightcraft_world::GeoPoint {
            latitude_deg: lat + miles / 60.,
            longitude_deg: lon,
        },
        altitude_constraint,
    };
    s.set_gps_waypoints(vec![
        waypoint("ALPHA", 6., constraints[0]),
        waypoint("BRAVO", 12., constraints[1]),
    ])
    .unwrap();
    // Every command and display frame sequences the plan, which hands the receiver its leg.
    s.sequence_glass();
    s
}
fn feet(f: f64) -> f64 {
    f * 0.3048
}
#[test]
fn the_vertical_path_is_computed_back_from_the_constraint_with_its_deviation_and_required_speed() {
    require_region_data!();
    let at = |kind, ft| Some(AltitudeConstraint::new(kind, ft));
    // 500 ft at ALPHA, 6 NM out; the aircraft is at about 3,000 ft.
    let s = vnav_session([at(ConstraintKind::At, 500.), None]);
    let g = s.glass_state();
    let v = g.vnav.expect("a constraint ahead gives a path");
    assert_eq!(v.target_ident, "ALPHA");
    assert!(
        (v.target_distance_nm - 6.).abs() < 0.05,
        "{}",
        v.target_distance_nm
    );
    let altitude = g.air_data.altitude_msl_m;
    assert!((altitude / 0.3048 - 3000.).abs() < 150., "{}", altitude / 0.3048);
    // A 3 degree path, 6 NM from a 500 ft constraint, is at about 2,410 ft.
    let want = feet(500.) + v.target_distance_nm * 1852. * 3f64.to_radians().tan();
    assert!((v.path_altitude_m - want).abs() < 1.0);
    assert!((v.deviation_m - (altitude - want)).abs() < 1.0);
    assert!(v.dots.abs() <= 2.5);
    // Above the path is positive, so the indicator reads high; the data bar says to descend.
    assert!(v.deviation_m > 0. && v.dots > 0., "{v:?}");
    assert!(v.required_vs_fpm.is_some_and(|f| f < 0.), "{v:?}");
    assert!(v.top_of_descent_nm.is_some_and(|t| t < 0.1), "{v:?}");
    assert!((v.angle_deg - 3.).abs() < 1e-12 && !v.tracking);
    // The same aircraft below the path reads negative: constrain BRAVO high enough that the
    // path to it is far above the aircraft.
    let s = vnav_session([None, at(ConstraintKind::At, 4000.)]);
    let v = s.glass_state().vnav.unwrap();
    assert_eq!(v.target_ident, "BRAVO");
    assert!(v.deviation_m < 0. && v.dots < 0., "{v:?}");
    // A climb is needed to be at 4,000 ft there.
    assert!(v.required_vs_fpm.is_some_and(|f| f > 0.), "{v:?}");
    // No constraint, no path.
    assert!(vnav_session([None, None]).glass_state().vnav.is_none());
}
#[test]
fn the_descent_angle_is_a_validated_setting_that_moves_the_path() {
    require_region_data!();
    let mut s = vnav_session([Some(AltitudeConstraint::new(ConstraintKind::At, 500.)), None]);
    let path = |s: &Session| s.glass_state().vnav.unwrap().path_altitude_m;
    let three = path(&s);
    for bad in [0.5, 6.5, f64::NAN] {
        assert!(s.set_glass(GlassSetting::VnavAngle { degrees: bad }).is_err());
    }
    assert_eq!(path(&s), three);
    s.set_glass(GlassSetting::VnavAngle { degrees: 5. }).unwrap();
    assert!(path(&s) > three, "a steeper path is higher this far out");
    assert_eq!(s.glass_state().settings.vnav_angle_deg, 5.);
}
#[test]
fn the_receiver_leg_carries_the_path_and_the_autopilot_sees_the_deviation() {
    require_region_data!();
    let s = vnav_session([Some(AltitudeConstraint::new(ConstraintKind::At, 500.)), None]);
    let leg = s.sim.receiver_vnav().expect("the active leg has a path");
    assert!((leg.target_altitude_m - feet(500.)).abs() < 1e-9);
    assert!(leg.beyond_m.abs() < 1e-6, "the leg ends at the constraint");
    // A plan with no constraint hands the autopilot no vertical guidance at all.
    let plain = vnav_session([None, None]);
    assert!(plain.sim.receiver_vnav().is_none());
}
#[test]
fn constraints_are_edited_on_the_fpl_page_validated_and_survive_other_edits() {
    require_region_data!();
    use crate::touch_fpl::FplEdit;
    let mut s = vnav_session([None, None]);
    let constraint = |s: &Session, i: usize| s.glass_state().gps.legs[i].constraint.clone();
    // Nothing selected past the plan, and a bad constraint, are refused.
    s.edit_flight_plan(FplEdit::Select { index: 1 }).unwrap();
    assert!(
        s.edit_flight_plan(FplEdit::Altitude {
            constraint: Some(AltitudeConstraint {
                kind: ConstraintKind::At,
                altitude_m: f64::NAN
            })
        })
        .is_err()
    );
    assert!(
        s.set_glass(GlassSetting::WaypointAltitude {
            index: 7,
            constraint: None
        })
        .is_err()
    );
    // Set BRAVO at or above 2,500 ft, as the keypad does.
    s.edit_flight_plan(FplEdit::Altitude {
        constraint: Some(AltitudeConstraint::new(ConstraintKind::AtOrAbove, 2500.)),
    })
    .unwrap();
    assert_eq!(constraint(&s, 1).as_deref(), Some("2500FT A"));
    assert_eq!(constraint(&s, 0), None);
    // Removing ALPHA keeps the constraint on BRAVO, now the only waypoint.
    s.edit_flight_plan(FplEdit::Select { index: 0 }).unwrap();
    s.edit_flight_plan(FplEdit::Remove {}).unwrap();
    assert_eq!(s.glass_state().gps.legs.len(), 1);
    assert_eq!(constraint(&s, 0).as_deref(), Some("2500FT A"));
    // Clear it.
    s.edit_flight_plan(FplEdit::Select { index: 0 }).unwrap();
    s.edit_flight_plan(FplEdit::Altitude { constraint: None })
        .unwrap();
    assert_eq!(constraint(&s, 0), None);
    // The tagged JSON a client sends.
    s.execute(
        "avionics.set",
        serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"waypoint_altitude","index":0,
            "constraint":{"kind":"at_or_below","altitude_m":900.0}}}}),
    )
    .unwrap();
    assert_eq!(constraint(&s, 0).as_deref(), Some("2953FT B"));
}
