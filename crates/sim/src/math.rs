//! Small allocation-free vector/quaternion operations with explicit components.
pub fn add([a, b, c]: [f64; 3], [d, e, f]: [f64; 3]) -> [f64; 3] {
    [a + d, b + e, c + f]
}
pub fn scale([a, b, c]: [f64; 3], s: f64) -> [f64; 3] {
    [a * s, b * s, c * s]
}
pub fn dot([a, b, c]: [f64; 3], [d, e, f]: [f64; 3]) -> f64 {
    a * d + b * e + c * f
}
pub fn cross([a, b, c]: [f64; 3], [d, e, f]: [f64; 3]) -> [f64; 3] {
    [b * f - c * e, c * d - a * f, a * e - b * d]
}
pub fn norm(v: [f64; 3]) -> f64 {
    libm::sqrt(dot(v, v))
}
pub fn mul([a, b, c, d]: [f64; 4], [e, f, g, h]: [f64; 4]) -> [f64; 4] {
    [
        a * e - b * f - c * g - d * h,
        a * f + b * e + c * h - d * g,
        a * g - b * h + c * e + d * f,
        a * h + b * g - c * f + d * e,
    ]
}
pub fn conjugate([w, x, y, z]: [f64; 4]) -> [f64; 4] {
    [w, -x, -y, -z]
}
pub fn rotate([w, x, y, z]: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let q = [x, y, z];
    let t = scale(cross(q, v), 2.0);
    add(v, add(scale(t, w), cross(q, t)))
}
pub fn normalise([w, x, y, z]: [f64; 4]) -> [f64; 4] {
    let n = libm::sqrt(w * w + x * x + y * y + z * z);
    if n.is_finite() && n > 1e-12 {
        [w / n, x / n, y / n, z / n]
    } else {
        [1.0, 0.0, 0.0, 0.0]
    }
}
pub fn attitude(pitch: f64, heading: f64) -> [f64; 4] {
    let (s, c) = (libm::sin(pitch * 0.5), libm::cos(pitch * 0.5));
    let (sh, ch) = (libm::sin(heading * 0.5), libm::cos(heading * 0.5));
    [ch * c, -sh * s, ch * s, sh * c]
}
pub fn euler([w, x, y, z]: [f64; 4]) -> [f64; 3] {
    [
        libm::atan2(2.0 * (w * x + y * z), 1.0 - 2.0 * (x * x + y * y)),
        libm::asin((2.0 * (w * y - z * x)).clamp(-1.0, 1.0)),
        libm::atan2(2.0 * (w * z + x * y), 1.0 - 2.0 * (y * y + z * z)).rem_euclid(core::f64::consts::TAU),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotation_inverse_and_norm() {
        let q = attitude(0.2, 1.1);
        let v = [3.0, 4.0, 5.0];
        assert!(norm(add(rotate(conjugate(q), rotate(q, v)), scale(v, -1.0))) < 1e-14);
        assert!((norm(rotate(q, v)) - norm(v)).abs() < 1e-14);
        let [r, p, h] = euler(q);
        assert!(r.abs() < 1e-15 && (p - 0.2).abs() < 1e-15 && (h - 1.1).abs() < 1e-15);
    }
}
