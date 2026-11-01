#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_world::{TurbulenceLevel, Weather, Wind};

#[test]
fn meteorological_wind_and_surface_shear() {
    for (from, expected) in [
        (0.0, [-10.0, 0.0, 0.0]),
        (90.0, [0.0, -10.0, 0.0]),
        (180.0, [10.0, 0.0, 0.0]),
        (270.0, [0.0, 10.0, 0.0]),
    ] {
        let w = Weather {
            wind_from_deg: from,
            wind_speed_mps: 10.0,
            ..Weather::default()
        };
        let actual = w.mean_wind_mps([0.0, 0.0, -10.0]);
        for (a, b) in actual.into_iter().zip(expected) {
            assert!((a - b).abs() < 1e-12);
        }
    }
    let w = Weather {
        wind_speed_mps: 10.0,
        shear: true,
        ..Weather::default()
    };
    assert_eq!(w.mean_wind_mps([0.0; 3]), [0.0; 3]);
    assert_eq!(w.mean_wind_mps([0.0, 0.0, -10.0]), [-10.0, 0.0, 0.0]);
    assert!((w.mean_wind_mps([0.0, 0.0, -100.0])[0] + 13.894954943731376).abs() < 1e-12);
    assert_eq!(
        w.mean_wind_mps([0.0, 0.0, -1000.0]),
        w.mean_wind_mps([0.0, 0.0, -5000.0])
    );
}

#[test]
fn dryden_long_run_rms_and_autocorrelation() {
    // Four hours, 50 m/s, 120 Hz, after 100 s burn-in. Lag τ=L/V yields
    // R_u(τ)/σ²=e^-1 and R_v,w(τ)/σ²=0.5 e^-1 from the published spectra.
    for level in [
        TurbulenceLevel::Light,
        TurbulenceLevel::Moderate,
        TurbulenceLevel::Severe,
    ] {
        let mut wind = Wind::new(Weather {
            turbulence: level,
            seed: 98317,
            ..Weather::default()
        })
        .unwrap();
        let lag = (533.4_f64 / 50.0 * 120.0).round() as usize;
        let mut history = vec![[0.0; 3]; lag];
        let mut sum = [0.0; 3];
        let mut squares = [0.0; 3];
        let mut correlation = [0.0; 3];
        let samples = 120 * 14_400;
        for step in 0..samples + 12_000 {
            wind.advance(1.0 / 120.0, [50.0, 0.0, 0.0]).unwrap();
            let gust = wind.gust_mps();
            if step >= 12_000 {
                for axis in 0..3 {
                    sum[axis] += gust[axis];
                    squares[axis] += gust[axis] * gust[axis];
                    correlation[axis] += gust[axis] * history[step % lag][axis];
                }
            }
            history[step % lag] = gust;
        }
        for axis in 0..3 {
            let mean = sum[axis] / samples as f64;
            let variance = squares[axis] / samples as f64 - mean * mean;
            let sigma = libm::sqrt(variance);
            let expected = level.intensity_mps();
            assert!(
                (sigma / expected - 1.0).abs() < 0.15,
                "{level:?} axis {axis}: sigma {sigma}"
            );
            assert!(mean.abs() < 0.15 * expected);
            let measured = (correlation[axis] / samples as f64 - mean * mean) / variance;
            let expected_correlation = libm::exp(-1.0) * if axis == 0 { 1.0 } else { 0.5 };
            assert!(
                (measured - expected_correlation).abs() < 0.08,
                "axis {axis}: R {measured}"
            );
        }
    }
}

#[test]
fn hostile_weather_and_steps_are_atomic_errors() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 1e100] {
        assert!(
            Wind::new(Weather {
                wind_speed_mps: value,
                ..Weather::default()
            })
            .is_err()
        );
        assert!(
            Wind::new(Weather {
                wind_from_deg: value,
                ..Weather::default()
            })
            .is_err()
        );
    }
    let mut wind = Wind::new(Weather {
        turbulence: TurbulenceLevel::Light,
        seed: 1,
        ..Weather::default()
    })
    .unwrap();
    let before: Vec<_> = wind.hash_words().collect();
    for (dt, speed) in [
        (f64::NAN, 40.0),
        (0.01, f64::NAN),
        (-1.0, 40.0),
        (2.0, 40.0),
        (0.01, 1001.0),
    ] {
        assert!(wind.advance(dt, [speed, 0.0, 0.0]).is_err());
        assert_eq!(wind.hash_words().collect::<Vec<_>>(), before);
    }
}

#[test]
fn gust_filter_axes_follow_air_relative_flight_path() {
    let weather = Weather {
        turbulence: TurbulenceLevel::Moderate,
        seed: 7,
        ..Weather::default()
    };
    let mut north = Wind::new(weather).unwrap();
    let mut east = north.clone();
    for _ in 0..1200 {
        north.advance(1.0 / 120.0, [50.0, 0.0, 0.0]).unwrap();
        east.advance(1.0 / 120.0, [0.0, 50.0, 0.0]).unwrap();
        let [u, v, w] = north.gust_mps();
        let [n, e, d] = east.gust_mps();
        assert!((n + v).abs() < 1e-12 && (e - u).abs() < 1e-12 && d == w);
    }
    let before = north.gust_mps();
    north.advance(f64::from_bits(1), [0.0; 3]).unwrap();
    assert_eq!(north.gust_mps(), before);
}
