#![allow(clippy::unwrap_used)]
use flightcraft_sim::nav_source::*;
#[test]
fn direct_to_cross_track_sign_to_from_and_hostile_inputs() {
    let source = DirectTo::new([0., 0.], [10000., 0.]).unwrap();
    let east = source.guidance(1000., 200.).unwrap();
    assert_eq!(east.cross_track_m, 200.);
    assert_eq!(east.to_from, ToFrom::To);
    assert_eq!(source.guidance(11000., 0.).unwrap().to_from, ToFrom::From);
    assert!(source.guidance(f64::NAN, 0.).is_none());
    assert!(DirectTo::new([0., 0.], [0., 0.]).is_err());
    assert!(DirectTo::new([f64::INFINITY, 0.], [10., 0.]).is_err());
    assert!(DirectTo::new([0., 0.], [250001., 0.]).is_err());
}

#[test]
fn shared_gps_adapter_converts_frames_units_and_loss_of_fix() {
    let gps = GpsNavSource::from_receiver(true, 90.0_f64.to_radians(), 0.1, ToFrom::To, 2.0_f64.to_radians())
        .unwrap();
    let guidance = gps.guidance(0.0, 0.0).unwrap();
    assert!((guidance.desired_track_rad.to_degrees() - 88.0).abs() < 1e-10);
    assert!((guidance.cross_track_m - 185.2).abs() < 1e-9);
    assert!(
        GpsNavSource::from_receiver(false, 0.0, 0.0, ToFrom::To, 0.0)
            .unwrap()
            .guidance(0.0, 0.0)
            .is_none()
    );
    assert!(GpsNavSource::from_receiver(true, 0.0, f64::NAN, ToFrom::To, 0.0).is_err());
    assert!(GpsNavSource::from_receiver(true, f64::NAN, 0.0, ToFrom::To, 0.0).is_err());
}
