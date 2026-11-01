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

#[cfg(test)]
mod tests {
    use super::*;

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
