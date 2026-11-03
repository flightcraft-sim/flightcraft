//! WGS84 transverse Mercator, central scale one, with a region-local northing origin.
//! Krüger's third-flattening series through n⁴; heights remain orthometric MSL.
use crate::TerrainError;
const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_223_563;
pub const HORIZONTAL_BOUND_M: f64 = 250_000.0;

#[derive(Clone, Copy, Debug)]
pub struct LocalProjection {
    lon: f64,
    north0: f64,
}
impl LocalProjection {
    pub fn new(latitude_deg: f64, longitude_deg: f64) -> Result<Self, TerrainError> {
        if !latitude_deg.is_finite()
            || latitude_deg.abs() > 85.0
            || !longitude_deg.is_finite()
            || !(-180.0..180.0).contains(&longitude_deg)
        {
            return Err(TerrainError("projection origin outside supported bounds"));
        }
        let mut projection = Self {
            lon: longitude_deg.to_radians(),
            north0: 0.0,
        };
        let [north, _] = forward(latitude_deg.to_radians(), 0.0);
        projection.north0 = north;
        Ok(projection)
    }
    pub fn project(&self, latitude_deg: f64, longitude_deg: f64) -> Result<[f64; 2], TerrainError> {
        if !latitude_deg.is_finite()
            || latitude_deg.abs() >= 90.0
            || !longitude_deg.is_finite()
            || !(-180.0..=180.0).contains(&longitude_deg)
        {
            return Err(TerrainError("invalid geographic coordinate"));
        }
        let delta = (longitude_deg.to_radians() - self.lon + core::f64::consts::PI)
            .rem_euclid(core::f64::consts::TAU)
            - core::f64::consts::PI;
        if delta.abs() >= core::f64::consts::FRAC_PI_2 {
            return Err(TerrainError("coordinate outside transverse Mercator hemisphere"));
        }
        let [n, e] = forward(latitude_deg.to_radians(), delta);
        let result = [n - self.north0, e];
        bounded(result)?;
        Ok(result)
    }
    pub fn unproject(&self, north: f64, east: f64) -> Result<[f64; 2], TerrainError> {
        bounded([north, east])?;
        let (scale, _, beta) = series();
        let (xi, eta) = ((north + self.north0) / scale, east / scale);
        let (mut x, mut y) = (xi, eta);
        for (i, b) in beta.into_iter().enumerate() {
            let j = 2.0 * (i + 1) as f64;
            x -= b * libm::sin(j * xi) * libm::cosh(j * eta);
            y -= b * libm::cos(j * xi) * libm::sinh(j * eta);
        }
        let tau_prime = libm::sin(x) / libm::hypot(libm::sinh(y), libm::cos(x));
        let e2 = F * (2.0 - F);
        let e = libm::sqrt(e2);
        let mut tau = tau_prime / (1.0 - e2);
        for _ in 0..6 {
            let h = libm::hypot(1.0, tau);
            let sigma = libm::sinh(e * libm::atanh(e * tau / h));
            let tp = tau * libm::hypot(1.0, sigma) - sigma * h;
            tau +=
                (tau_prime - tp) * (1.0 + (1.0 - e2) * tau * tau) / ((1.0 - e2) * h * libm::hypot(1.0, tp));
        }
        let lat = libm::atan(tau).to_degrees();
        let lon = (self.lon + libm::atan2(libm::sinh(y), libm::cos(x))).to_degrees();
        if !lat.is_finite() || !lon.is_finite() {
            return Err(TerrainError("invalid inverse projection"));
        }
        Ok([lat, (lon + 180.0).rem_euclid(360.0) - 180.0])
    }
}
fn bounded(values: [f64; 2]) -> Result<(), TerrainError> {
    if values
        .iter()
        .any(|v| !v.is_finite() || v.abs() > HORIZONTAL_BOUND_M + 0.00001)
    {
        return Err(TerrainError("local coordinate outside 250 km bounds"));
    }
    Ok(())
}
fn series() -> (f64, [f64; 4], [f64; 4]) {
    let n = F / (2.0 - F);
    let (n2, n3, n4) = (n * n, n * n * n, n * n * n * n);
    (
        A / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0),
        [
            n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0,
            13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0,
            61.0 * n3 / 240.0 - 103.0 * n4 / 140.0,
            49561.0 * n4 / 161280.0,
        ],
        [
            n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0,
            n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0,
            17.0 * n3 / 480.0 - 37.0 * n4 / 840.0,
            4397.0 * n4 / 161280.0,
        ],
    )
}
fn forward(lat: f64, delta: f64) -> [f64; 2] {
    let e = libm::sqrt(F * (2.0 - F));
    let tau = libm::tan(lat);
    let sigma = libm::sinh(e * libm::atanh(e * tau / libm::hypot(1.0, tau)));
    let tp = tau * libm::hypot(1.0, sigma) - sigma * libm::hypot(1.0, tau);
    let xi = libm::atan2(tp, libm::cos(delta));
    let eta = libm::asinh(libm::sin(delta) / libm::hypot(tp, libm::cos(delta)));
    let (scale, alpha, _) = series();
    let (mut x, mut y) = (xi, eta);
    for (i, a) in alpha.into_iter().enumerate() {
        let j = 2.0 * (i + 1) as f64;
        x += a * libm::sin(j * xi) * libm::cosh(j * eta);
        y += a * libm::cos(j * xi) * libm::sinh(j * eta);
    }
    [scale * x, scale * y]
}

/// WGS84 UTM grid coordinates, independently using the same Krüger series as local TM.
/// Kept separate from the local 250 km coordinate bounds used by the simulation.
pub fn utm(latitude_deg: f64, longitude_deg: f64, zone: u8) -> Result<[f64; 2], TerrainError> {
    if !(1..=60).contains(&zone)
        || !latitude_deg.is_finite()
        || !longitude_deg.is_finite()
        || !(-80.0..=84.).contains(&latitude_deg)
        || !(-180.0..=180.).contains(&longitude_deg)
    {
        return Err(TerrainError("invalid UTM coordinate"));
    }
    let central = f64::from(zone) * 6. - 183.;
    let delta = (longitude_deg - central + 180.).rem_euclid(360.) - 180.;
    if delta.abs() > 6. {
        return Err(TerrainError("coordinate outside UTM zone margin"));
    }
    let [n, e] = forward(latitude_deg.to_radians(), delta.to_radians());
    Ok([
        n * 0.9996 + if latitude_deg < 0. { 10_000_000. } else { 0. },
        e * 0.9996 + 500_000.,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn millimetre_round_trips_across_region_square() {
        for (lat, lon) in [(0.0, 0.0), (40.97, -75.0), (-45.0, 179.9), (84.0, -179.9)] {
            let p = LocalProjection::new(lat, lon).unwrap();
            for n in [-250000.0, -125000.0, 0.0, 125000.0, 250000.0] {
                for e in [-250000.0, -125000.0, 0.0, 125000.0, 250000.0] {
                    let [lat, lon] = p.unproject(n, e).unwrap();
                    let [nn, ee] = p.project(lat, lon).unwrap();
                    assert!(libm::hypot(nn - n, ee - e) < 0.001, "{n} {e} {nn} {ee}");
                }
            }
        }
    }
    // Independently evaluate Snyder, USGS Professional Paper 1395, transverse Mercator
    // equations 8-9 through 8-15 in eccentricity and longitude (rather than Krüger n).
    fn snyder(lat: f64, lon: f64) -> [f64; 2] {
        let phi = lat.to_radians();
        let e2 = F * (2.0 - F);
        let e4 = e2 * e2;
        let e6 = e4 * e2;
        let e8 = e4 * e4;
        let m = A
            * ((1.0 - e2 / 4.0 - 3.0 * e4 / 64.0 - 5.0 * e6 / 256.0 - 175.0 * e8 / 16384.0) * phi
                - (3.0 * e2 / 8.0 + 3.0 * e4 / 32.0 + 45.0 * e6 / 1024.0 + 105.0 * e8 / 4096.0)
                    * libm::sin(2.0 * phi)
                + (15.0 * e4 / 256.0 + 45.0 * e6 / 1024.0 + 525.0 * e8 / 16384.0) * libm::sin(4.0 * phi)
                - (35.0 * e6 / 3072.0 + 175.0 * e8 / 12288.0) * libm::sin(6.0 * phi)
                + 315.0 * e8 / 131072.0 * libm::sin(8.0 * phi));
        let t = libm::tan(phi) * libm::tan(phi);
        let ep = e2 / (1.0 - e2);
        let c = ep * libm::cos(phi) * libm::cos(phi);
        let nu = A / libm::sqrt(1.0 - e2 * libm::sin(phi) * libm::sin(phi));
        let a = lon.to_radians() * libm::cos(phi);
        let a2 = a * a;
        let east = nu
            * a
            * (1.0
                + (1.0 - t + c) * a2 / 6.0
                + (5.0 - 18.0 * t + t * t + 72.0 * c - 58.0 * ep) * a2 * a2 / 120.0);
        let north = m + nu
            * libm::tan(phi)
            * a2
            * (0.5
                + (5.0 - t + 9.0 * c + 4.0 * c * c) * a2 / 24.0
                + (61.0 - 58.0 * t + t * t + 600.0 * c - 330.0 * ep) * a2 * a2 / 720.0);
        [north, east]
    }
    #[test]
    fn published_closed_form_equations_independently_evaluated() {
        for origin in [0.0, 40.0, 80.0] {
            let p = LocalProjection::new(origin, 0.0).unwrap();
            for lat in [origin, origin + 1.0] {
                for lon in [-1.0, 0.0, 1.0] {
                    let [n, e] = snyder(lat, lon);
                    let [n0, _] = snyder(origin, 0.0);
                    let [nn, ee] = p.project(lat, lon).unwrap();
                    assert!(
                        libm::hypot(nn - (n - n0), ee - e) < 0.001,
                        "{origin} {lat} {lon}: {nn} {ee} vs {} {e}",
                        n - n0
                    );
                }
            }
        }
    }
    #[test]
    fn utm_matches_independent_snyder_and_hemisphere_offsets() {
        for latitude in [0., 40.5, -45., 80.] {
            let [n, e] = snyder(latitude, 1.5);
            let [actual_n, actual_e] = utm(latitude, -73.5, 18).unwrap();
            let expected_n = n * 0.9996 + if latitude < 0. { 10_000_000. } else { 0. };
            assert!(libm::hypot(actual_n - expected_n, actual_e - (e * 0.9996 + 500_000.)) < 0.001);
        }
        assert!(utm(f64::NAN, -75., 18).is_err());
        assert!(utm(41., -75., 61).is_err());
    }
    #[test]
    fn usgs_forward_example_angles_on_wgs84() {
        // The USGS TM example uses 40°30′ N, 73°30′ W, and central meridian 75° W.
        // Re-evaluate its angles on WGS84 at scale one, with a local 40° N origin.
        let p = LocalProjection::new(40.0, -75.0).unwrap();
        let [n, e] = snyder(40.5, 1.5);
        let [origin, _] = snyder(40.0, 0.0);
        let [actual_n, actual_e] = p.project(40.5, -73.5).unwrap();
        assert!(libm::hypot(actual_n - (n - origin), actual_e - e) < 0.001);
    }
    #[test]
    fn hostile_coordinates() {
        let p = LocalProjection::new(40.0, -75.0).unwrap();
        for pair in [[f64::NAN, 0.0], [0.0, f64::INFINITY], [250001.0, 0.0]] {
            assert!(p.unproject(pair[0], pair[1]).is_err());
        }
        assert!(p.project(40.0, 105.0).is_err());
        assert!(p.project(90.0, 0.0).is_err());
    }
}
