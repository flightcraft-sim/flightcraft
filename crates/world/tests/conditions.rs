#![allow(clippy::unwrap_used)]
use flightcraft_world::*;
#[test]
fn finite_bounded_layers_and_visibility() {
    let valid = Conditions::preset(WeatherPreset::Ifr);
    assert!(valid.validate().is_ok());
    for visibility_sm in [f64::NAN, f64::INFINITY, 0., 0.124, 100.1] {
        assert!(
            Conditions {
                visibility_sm,
                ..valid.clone()
            }
            .validate()
            .is_err()
        );
    }
    for (base, top) in [
        (600., 600.),
        (700., 600.),
        (f64::NAN, 1000.),
        (-1501., 1000.),
        (0., 60001.),
    ] {
        let c = Conditions {
            layers: vec![CloudLayer {
                base_ft_msl: base,
                top_ft_msl: top,
                coverage: Coverage::Ovc,
            }],
            ..valid.clone()
        };
        assert!(c.validate().is_err());
    }
    let mut too_many = valid.clone();
    too_many.layers = vec![valid.layers.first().copied().unwrap(); 4];
    assert!(too_many.validate().is_err());
}
#[test]
fn cloud_base_is_a_real_boundary_and_precipitation_reduces_visibility() {
    let mut c = Conditions::preset(WeatherPreset::Ifr);
    assert!(c.in_cloud([0., 0., -700. * 0.3048]));
    assert!(c.in_cloud([0., 0., -600. * 0.3048]));
    assert!(!c.in_cloud([0., 0., -599.99 * 0.3048]));
    assert!(!c.in_cloud([0., 0., -3601. * 0.3048]));
    let visibility = c.effective_visibility_m();
    c.precipitation = Precipitation::Rain;
    assert_eq!(c.effective_visibility_m(), visibility * 0.75);
    c.precipitation = Precipitation::Snow;
    assert_eq!(c.effective_visibility_m(), visibility * 0.5);
}
