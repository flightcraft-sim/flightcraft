#![allow(clippy::unwrap_used)]
use flightcraft_aero::{
    Airflow, ControlSurface, Controls, Failures, forces, forces_with_failures, propulsion_with_power,
};

#[test]
fn failed_engine_has_no_idle_power_and_windmills() {
    assert_eq!(propulsion_with_power(0.0, 1.0, 1.225, 0.0), (0.0, 0.0));
    for speed in [10.0, 30.0, 50.0] {
        let failed = propulsion_with_power(speed, 1.0, 1.225, 0.0);
        assert!(failed.0 < 0.0 && failed.1 > 0.0);
        assert_eq!(failed, propulsion_with_power(speed, 0.0, 1.225, 0.0));
        assert!(
            propulsion_with_power(speed, 1.0, 1.225, 0.4).0 < propulsion_with_power(speed, 1.0, 1.225, 1.0).0
        );
    }
}

#[test]
fn jams_override_demands_before_forces_and_moments() {
    let flow = Airflow {
        velocity_body_mps: [35.0, 1.0, 2.0],
        ..Airflow::default()
    };
    for surface in [
        ControlSurface::Elevator,
        ControlSurface::Aileron,
        ControlSurface::Rudder,
        ControlSurface::Flaps,
    ] {
        let mut failures = Failures::default();
        failures.jam(surface, 0.3).unwrap();
        let demanded = Controls {
            elevator: -0.4,
            aileron: -0.2,
            rudder: 0.8,
            flaps: 0.0,
            throttle: 0.7,
        };
        let actual = failures.apply(demanded);
        let with_failure = forces_with_failures(flow, demanded, 1.225, failures);
        let explicit = forces(flow, actual, 1.225);
        assert_eq!(with_failure.force_n, explicit.force_n);
        assert_eq!(with_failure.moment_nm, explicit.moment_nm);
        let healthy = forces(flow, demanded, 1.225);
        assert!(with_failure.force_n != healthy.force_n || with_failure.moment_nm != healthy.moment_nm);
        for position in [f64::NAN, f64::INFINITY, -2.0, 2.0] {
            let before = failures;
            assert!(failures.jam(surface, position).is_err());
            assert_eq!(failures, before);
        }
    }
    assert!(Failures::default().jam(ControlSurface::Flaps, -0.1).is_err());
}
