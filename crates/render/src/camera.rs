//! Body-attached camera and perspective horizon geometry (body x forward, y right, z down).
use crate::drawing;
use glam::{Mat4, Quat, Vec3};

/// Corvid left-seat pilot eye, metres from the body reference point. Used by the Corvid cockpit view.
pub const CORVID_PILOT_EYE_M: Vec3 = Vec3::new(0.15, -0.28, -0.43);
pub const VERTICAL_FOV_RAD: f32 = 52.0 * std::f32::consts::PI / 180.0;

pub(crate) fn cockpit_view(position: Vec3, attitude: Quat, yaw: f32, pitch: f32, pilot_eye: Vec3) -> Mat4 {
    let head = Quat::from_rotation_z(yaw) * Quat::from_rotation_y(pitch);
    let eye = position + drawing(attitude * pilot_eye);
    let forward = drawing(attitude * head * Vec3::X);
    let up = drawing(attitude * head * -Vec3::Z);
    Mat4::look_to_rh(eye, forward, up)
}

/// Horizon half-plane in screen coordinates (right, down), measured in viewport half-heights.
/// Ground satisfies a*x + b*y + c >= 0. No division by cos(pitch), including vertical flight.
pub fn attitude_horizon(roll: f32, pitch: f32) -> [f32; 3] {
    [
        roll.sin() * pitch.cos(),
        roll.cos() * pitch.cos(),
        -pitch.sin() / (VERTICAL_FOV_RAD * 0.5).tan(),
    ]
}

/// Perspective projection with reversed depth: the near plane maps to 1 and the far plane to 0.
/// With a 0.015 m near plane a standard 24-bit depth buffer resolves only 4 m at 1 km and 36 m at
/// 3 km, so ground and water a metre apart fight; reversed 32-bit float depth resolves about a
/// millimetre at 6 km. Every scene pipeline therefore tests `GreaterEqual` and clears depth to 0.
pub fn perspective_reversed(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    Mat4::perspective_rh(fov_y, aspect, far, near)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ground and a water plane one metre apart, 6 km from a cockpit camera, must land on
    /// different depth values (they merged into one 24-bit value and fought, painting
    /// terrain-shaped patches through the water). Resolved against f32 depth spacing.
    #[test]
    fn reversed_depth_separates_a_metre_at_six_kilometres() {
        let p = perspective_reversed(1., 1.5, 0.015, 16000.);
        let depth = |z: f32| {
            let c = p * glam::Vec4::new(0., 0., -z, 1.);
            c.z / c.w
        };
        let (near, far) = (depth(6000.), depth(6001.));
        assert!(near > far);
        // At least ten times the spacing of adjacent f32 values at that depth.
        assert!(near - far > 10. * f32::EPSILON * near);
        // The standard (non-reversed) 24-bit mapping could not tell them apart.
        let standard = Mat4::perspective_rh(1., 1.5, 0.015, 16000.);
        let d = |z: f32| {
            let c = standard * glam::Vec4::new(0., 0., -z, 1.);
            c.z / c.w
        };
        assert!(d(6001.) - d(6000.) < 2. / 16_777_216.);
    }

    #[test]
    fn reversed_projection_maps_near_to_one_and_far_to_zero() {
        let p = perspective_reversed(1., 1.5, 0.015, 16000.);
        let depth = |z: f32| {
            let c = p * glam::Vec4::new(0., 0., -z, 1.);
            c.z / c.w
        };
        assert!((depth(0.015) - 1.).abs() < 1e-4);
        assert!(depth(16000.).abs() < 1e-4);
        assert!(depth(100.) > depth(1000.));
    }

    #[test]
    fn projected_world_horizon_matches_instrument_at_all_attitudes() {
        let projection = Mat4::perspective_rh(VERTICAL_FOV_RAD, 1.6, 0.05, 16000.0);
        for roll_deg in (-180..=180).step_by(15) {
            for pitch_deg in (-90..=90).step_by(10) {
                for heading_deg in [0.0_f32, 79.0, 230.0] {
                    let roll = (roll_deg as f32).to_radians();
                    let pitch = (pitch_deg as f32).to_radians();
                    let attitude = Quat::from_rotation_z(heading_deg.to_radians())
                        * Quat::from_rotation_y(pitch)
                        * Quat::from_rotation_x(roll);
                    let view = cockpit_view(Vec3::ZERO, attitude, 0.0, 0.0, CORVID_PILOT_EYE_M);
                    let [a, b, c] = attitude_horizon(roll, pitch);
                    // Homogeneous directions at infinity: the true horizon is independent of altitude.
                    for azimuth in (0..360).step_by(17) {
                        let t = (azimuth as f32).to_radians();
                        let clip = projection * view * Vec3::new(t.cos(), 0.0, t.sin()).extend(0.0);
                        assert!(
                            (a * clip.x * 1.6 - b * clip.y + c * clip.w).abs() < 2e-5,
                            "roll={roll_deg} pitch={pitch_deg} clip={clip:?}"
                        );
                    }
                    // Also verify which side is earth, even inverted or pointing vertically.
                    let clip = projection * view * (-Vec3::Y).extend(0.0);
                    assert!(a * clip.x * 1.6 - b * clip.y + c * clip.w > 1.0);
                    assert!(view.is_finite());
                }
            }
        }
    }

    #[test]
    fn head_look_keeps_eye_fixed_and_axes_orthogonal_even_at_vertical_pitch() {
        for yaw in [-2.1, 0.0, 2.1] {
            for pitch in [-1.4, 0.0, 1.4] {
                let attitude = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
                let position = Vec3::new(12.0, 34.0, 56.0);
                let view = cockpit_view(position, attitude, yaw, pitch, CORVID_PILOT_EYE_M);
                let eye = position + drawing(attitude * CORVID_PILOT_EYE_M);
                assert!(view.transform_point3(eye).length() < 1e-4);
                assert!((view.determinant() - 1.0).abs() < 1e-5);
            }
        }
    }
}
