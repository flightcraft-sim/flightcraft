//! Shared deterministic GPS geodesy: WGS84 inverse and great-circle leg tracking.
const EARTH_NM: f64 = 6_371_008.8 / 1852.;
/// Signed great-circle cross-track distance; positive is to the right of the outbound course.
pub fn cross_track(origin: [f64; 2], position: [f64; 2], course_deg: f64) -> f64 {
    let (distance, bearing) = spherical_inverse_nm(origin, position);
    libm::asin(
        (libm::sin(distance / EARTH_NM) * libm::sin((bearing - course_deg).to_radians())).clamp(-1., 1.),
    ) * EARTH_NM
}
fn spherical_inverse_nm([lat1, lon1]: [f64; 2], [lat2, lon2]: [f64; 2]) -> (f64, f64) {
    let (a, b, d) = (lat1.to_radians(), lat2.to_radians(), (lon2 - lon1).to_radians());
    let y = libm::sin(d) * libm::cos(b);
    let x = libm::cos(a) * libm::sin(b) - libm::sin(a) * libm::cos(b) * libm::cos(d);
    let central = libm::atan2(
        libm::hypot(x, y),
        libm::sin(a) * libm::sin(b) + libm::cos(a) * libm::cos(b) * libm::cos(d),
    );
    (
        central * EARTH_NM,
        libm::atan2(y, x).to_degrees().rem_euclid(360.),
    )
}
/// WGS84 inverse geodesic using a bounded reduced-latitude iteration, in nm and true degrees.
pub fn inverse_nm(from: [f64; 2], to: [f64; 2]) -> (f64, f64) {
    let [lat1, lon1] = from;
    let [lat2, lon2] = to;
    let f = 1. / 298.257223563;
    let b = 6378137. * (1. - f);
    let u1 = libm::atan((1. - f) * libm::tan(lat1.to_radians()));
    let u2 = libm::atan((1. - f) * libm::tan(lat2.to_radians()));
    let (s1, c1, s2, c2) = (libm::sin(u1), libm::cos(u1), libm::sin(u2), libm::cos(u2));
    let l = (lon2 - lon1).to_radians();
    let mut lambda = l;
    for _ in 0..64 {
        let (sl, cl) = (libm::sin(lambda), libm::cos(lambda));
        let sin_sigma = libm::hypot(c2 * sl, c1 * s2 - s1 * c2 * cl);
        if sin_sigma < 1e-15 {
            return (0., 0.);
        }
        let cos_sigma = s1 * s2 + c1 * c2 * cl;
        let sigma = libm::atan2(sin_sigma, cos_sigma);
        let sin_alpha = c1 * c2 * sl / sin_sigma;
        let cos2_alpha = 1. - sin_alpha * sin_alpha;
        let cos2_mid = if cos2_alpha > 1e-15 {
            cos_sigma - 2. * s1 * s2 / cos2_alpha
        } else {
            0.
        };
        let c = f / 16. * cos2_alpha * (4. + f * (4. - 3. * cos2_alpha));
        let next = l
            + (1. - c)
                * f
                * sin_alpha
                * (sigma + c * sin_sigma * (cos2_mid + c * cos_sigma * (-1. + 2. * cos2_mid * cos2_mid)));
        if (next - lambda).abs() < 1e-12 {
            let u = cos2_alpha * (6378137_f64 * 6378137. - b * b) / (b * b);
            let a = 1. + u / 16384. * (4096. + u * (-768. + u * (320. - 175. * u)));
            let bb = u / 1024. * (256. + u * (-128. + u * (74. - 47. * u)));
            let delta = bb
                * sin_sigma
                * (cos2_mid
                    + bb / 4.
                        * (cos_sigma * (-1. + 2. * cos2_mid * cos2_mid)
                            - bb / 6.
                                * cos2_mid
                                * (-3. + 4. * sin_sigma * sin_sigma)
                                * (-3. + 4. * cos2_mid * cos2_mid)));
            return (
                b * a * (sigma - delta) / 1852.,
                libm::atan2(c2 * sl, c1 * s2 - s1 * c2 * cl)
                    .to_degrees()
                    .rem_euclid(360.),
            );
        }
        lambda = next;
    }
    spherical_inverse_nm(from, to)
}

/// WGS84 distance in metres and initial true bearing in radians.
pub fn inverse(from: [f64; 2], to: [f64; 2]) -> (f64, f64) {
    let (nm, deg) = inverse_nm(from, to);
    (nm * 1852., deg.to_radians())
}
/// Signed great-circle cross track (right positive) and along track, in metres.
pub fn track(start: [f64; 2], end: [f64; 2], position: [f64; 2]) -> (f64, f64) {
    let (_, course) = spherical_inverse_nm(start, end);
    let (distance, bearing) = spherical_inverse_nm(start, position);
    let angular = distance / EARTH_NM;
    let along = libm::atan2(
        libm::sin(angular) * libm::cos((bearing - course).to_radians()),
        libm::cos(angular),
    ) * EARTH_NM
        * 1852.;
    (cross_track(start, position, course) * 1852., along)
}
/// Local forward tangent to the leg's great circle, including convergence along the route.
pub fn desired_track(start: [f64; 2], end: [f64; 2], position: [f64; 2]) -> f64 {
    let vector = |[lat, lon]: [f64; 2]| {
        let (lat, lon) = (lat.to_radians(), lon.to_radians());
        [
            libm::cos(lat) * libm::cos(lon),
            libm::cos(lat) * libm::sin(lon),
            libm::sin(lat),
        ]
    };
    let cross = |[ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3]| {
        [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
    };
    let normal = cross(vector(start), vector(end));
    let [tx, ty, tz] = cross(normal, vector(position));
    let [lat, lon] = position.map(f64::to_radians);
    let east = -libm::sin(lon) * tx + libm::cos(lon) * ty;
    let north =
        -libm::sin(lat) * libm::cos(lon) * tx - libm::sin(lat) * libm::sin(lon) * ty + libm::cos(lat) * tz;
    libm::atan2(east, north).rem_euclid(core::f64::consts::TAU)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wgs84_equator_and_hand_computed_great_circle_tracking() {
        let (distance, bearing) = inverse([0., 0.], [0., 1.]);
        assert!((distance - 111319.490793).abs() < 0.1);
        assert!((bearing.to_degrees() - 90.).abs() < 1e-10);
        let (xt, along) = track([0., 0.], [0., 1.], [-0.01, 0.5]);
        assert!((xt - 1111.95).abs() < 0.1);
        assert!((along - 55597.54).abs() < 0.1);
    }
}
