#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_world::geomag::{MagneticModel, SimulationDate};
#[test]
fn every_official_wmm2025_test_vector() {
    let model = MagneticModel::wmm2025().unwrap();
    let mut count = 0;
    for line in include_str!("fixtures/WMM2025_TestValues.txt")
        .lines()
        .filter(|s| !s.starts_with('#') && !s.trim().is_empty())
    {
        let v: Vec<f64> = line.split_whitespace().map(|s| s.parse().unwrap()).collect();
        let f = model.field(v[2], v[3], v[1] * 1000.0, v[0]).unwrap();
        for (actual, expected) in [
            (f.north_nt, v[7]),
            (f.east_nt, v[8]),
            (f.down_nt, v[9]),
            (f.horizontal_nt, v[6]),
            (f.total_nt, v[10]),
        ] {
            assert!((actual - expected).abs() < 0.02, "{line}: {actual} != {expected}");
        }
        assert!((f.declination_deg - v[4]).abs() < 0.011);
        assert!((f.inclination_deg - v[5]).abs() < 0.011);
        // Linear secular coefficients also reproduce the published vector derivatives.
        let delta = 0.0001;
        let later = model.field(v[2], v[3], v[1] * 1000.0, v[0] + delta).unwrap();
        for (actual, expected) in [
            ((later.north_nt - f.north_nt) / delta, v[14]),
            ((later.east_nt - f.east_nt) / delta, v[15]),
            ((later.down_nt - f.down_nt) / delta, v[16]),
            ((later.horizontal_nt - f.horizontal_nt) / delta, v[13]),
            ((later.total_nt - f.total_nt) / delta, v[17]),
            ((later.declination_deg - f.declination_deg) / delta, v[11]),
            ((later.inclination_deg - f.inclination_deg) / delta, v[12]),
        ] {
            assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
        }
        count += 1;
    }
    assert_eq!(count, 100);
}
#[test]
fn model_bounds_and_calendar() {
    let m = MagneticModel::wmm2025().unwrap();
    assert!(m.field(f64::NAN, 0.0, 0.0, 2026.0).is_err());
    assert!(m.field(0.0, 0.0, 0.0, 2030.0).is_err());
    for lat in [-90.0, 90.0] {
        assert!(m.field(lat, 0.0, 0.0, 2026.0).unwrap().total_nt.is_finite());
    }
    assert_eq!(SimulationDate::default().decimal_year().unwrap(), 2026.0);
    assert!(
        SimulationDate {
            year: 2026,
            month: 2,
            day: 29
        }
        .decimal_year()
        .is_err()
    );
}
