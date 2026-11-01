//! Read-only 3D presentation. NED and body coordinates are converted here only.

mod camera;
mod gpu;
mod mesh;

pub use camera::{CORVID_PILOT_EYE_M, VERTICAL_FOV_RAD, attitude_horizon};
pub use gpu::initialize;

use eframe::egui;
use flightcraft_sim::State;
use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

/// Scene colours supplied by the UI's single theme module.
#[derive(Clone, Copy)]
pub struct Palette {
    pub sky_top: [f32; 4],
    pub sky_horizon: [f32; 4],
    pub ground: [f32; 4],
    pub grid: [f32; 4],
    pub runway: [f32; 4],
    pub marking: [f32; 4],
    pub aircraft: [f32; 4],
    pub accent: [f32; 4],
    pub glass: [f32; 4],
    pub rubber: [f32; 4],
    pub shadow: [f32; 4],
    pub cockpit_frame: [f32; 4],
    pub glare_shield: [f32; 4],
    pub propeller: [f32; 4],
}

/// Views share the same state, with no simulation side effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraMode {
    #[default]
    Chase,
    Cockpit,
}

pub struct Scene {
    eye: Option<Vec3>,
    target: Option<Vec3>,
    pub camera: CameraMode,
    chase_distance_m: f32,
    chase_height_m: f32,
    look_target: Vec2,
    look: Vec2,
    propeller_power: f32,
    larrow: bool,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            eye: None,
            target: None,
            camera: CameraMode::Chase,
            chase_distance_m: 19.0,
            chase_height_m: 7.0,
            look_target: Vec2::ZERO,
            look: Vec2::ZERO,
            propeller_power: 0.0,
            larrow: false,
        }
    }
}

fn drawing(v: Vec3) -> Vec3 {
    Vec3::new(v.y, -v.z, -v.x)
}

impl Scene {
    /// Select the built-in display mesh by the active definition's identity.
    /// Unknown identities use the default trainer outline.
    pub fn set_aircraft(&mut self, id: &str) {
        let larrow = id == "larrow";
        if self.larrow != larrow {
            self.larrow = larrow;
            self.reset_camera();
        }
    }

    /// Presentation offsets supplied by the app's validated camera command.
    pub fn set_chase_offsets(&mut self, distance_m: f32, height_m: f32) {
        if distance_m.is_finite() && height_m.is_finite() {
            self.chase_distance_m = distance_m.clamp(2.0, 200.0);
            self.chase_height_m = height_m.clamp(0.0, 100.0);
        }
    }

    /// Cockpit yaw is positive right; pitch is positive up, in radians.
    pub fn set_cockpit_look(&mut self, yaw_rad: f32, pitch_rad: f32) {
        if yaw_rad.is_finite() && pitch_rad.is_finite() {
            self.look_target = Vec2::new(yaw_rad.clamp(-2.1, 2.1), pitch_rad.clamp(-1.4, 1.4));
        }
    }

    pub fn set_propeller_power(&mut self, power: f32) {
        self.propeller_power = if power.is_finite() {
            power.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    pub fn toggle_camera(&mut self) {
        self.camera = match self.camera {
            CameraMode::Chase => CameraMode::Cockpit,
            CameraMode::Cockpit => CameraMode::Chase,
        };
        self.reset_camera();
    }

    pub fn reset_camera(&mut self) {
        self.eye = None;
        self.target = None;
    }

    fn smooth_look(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        self.look = self.look.lerp(self.look_target, 1.0 - (-dt * 9.0).exp());
    }

    /// Enqueue the wgpu callback. The camera follows the body's quaternion, not UI readouts.
    pub fn show(&mut self, ui: &mut egui::Ui, state: &State, dt: f32, palette: Palette) {
        let rect = ui.max_rect();
        let [north, east, down] = state.position_m.map(|v| v as f32);
        let [w, x, y, z] = state.attitude.map(|v| v as f32);
        let raw = Quat::from_xyzw(x, y, z, w);
        let attitude = if raw.is_finite() && raw.length_squared() > 0.001 {
            raw.normalize()
        } else {
            Quat::IDENTITY
        };
        // The bootstrap state has its CG at ground height. Keep the visual gear above the plane.
        let position = drawing(Vec3::new(north, east, down));
        let position = if self.larrow {
            position
        } else {
            position.max(Vec3::new(f32::NEG_INFINITY, 1.15, f32::NEG_INFINITY))
        };
        let axis = |v| drawing(attitude * v).extend(0.0);
        let model = Mat4::from_cols(axis(Vec3::X), axis(Vec3::Y), axis(Vec3::Z), position.extend(1.0));
        self.smooth_look(dt);
        let blend = 1.0 - (-dt.clamp(0.0, 0.25) * 7.0).exp();
        let (eye, view) = match self.camera {
            CameraMode::Chase => {
                let desired_eye = position
                    + drawing(attitude * Vec3::new(-self.chase_distance_m, 0.0, -self.chase_height_m));
                let desired_target = position + drawing(attitude * Vec3::new(10.0, 0.0, -0.3));
                let eye = self.eye.map_or(desired_eye, |v| v.lerp(desired_eye, blend));
                let target = self
                    .target
                    .map_or(desired_target, |v| v.lerp(desired_target, blend));
                self.eye = Some(eye);
                self.target = Some(target);
                (eye, Mat4::look_at_rh(eye, target, Vec3::Y))
            }
            CameraMode::Cockpit => {
                let pilot_eye = if self.larrow {
                    Vec3::new(-0.1, 0.0, -0.63)
                } else {
                    CORVID_PILOT_EYE_M
                };
                let eye = position + drawing(attitude * pilot_eye);
                (
                    eye,
                    camera::cockpit_view(position, attitude, self.look.x, self.look.y, pilot_eye),
                )
            }
        };
        let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
        let projection = Mat4::perspective_rh(camera::VERTICAL_FOV_RAD, aspect, 0.05, 16000.0);
        let world_from_clip = (projection * Mat4::from_mat3(Mat3::from_mat4(view))).inverse();
        let uniform = gpu::Uniform {
            view_projection: (projection * view).to_cols_array_2d(),
            aircraft: model.to_cols_array_2d(),
            world_from_clip: world_from_clip.to_cols_array_2d(),
            sky_top: palette.sky_top,
            sky_horizon: palette.sky_horizon,
            ground: palette.ground,
            grid: palette.grid,
            eye: Vec4::new(eye.x, eye.y, eye.z, self.propeller_power).to_array(),
        };
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            gpu::SceneCallback {
                uniform,
                size: [rect.width(), rect.height()],
                show_aircraft: self.camera == CameraMode::Chase,
                larrow: self.larrow,
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn north_east_down_maps_to_right_up_back() {
        assert_eq!(drawing(Vec3::X), -Vec3::Z);
        assert_eq!(drawing(Vec3::Y), Vec3::X);
        assert_eq!(drawing(Vec3::Z), -Vec3::Y);
    }

    #[test]
    fn head_look_is_smooth_frame_rate_independent_and_returns_on_release() {
        let mut fast = Scene::default();
        let mut slow = Scene::default();
        fast.set_cockpit_look(-1.35, 0.9);
        slow.set_cockpit_look(-1.35, 0.9);
        fast.smooth_look(1.0 / 120.0);
        assert!(fast.look.x < 0.0 && fast.look.x > -1.35);
        for _ in 1..120 {
            fast.smooth_look(1.0 / 120.0);
        }
        for _ in 0..30 {
            slow.smooth_look(1.0 / 30.0);
        }
        assert!((fast.look - slow.look).length() < 1e-5);
        fast.set_cockpit_look(0.0, 0.0);
        for _ in 0..120 {
            fast.smooth_look(1.0 / 120.0);
        }
        assert!(fast.look.length() < 0.001);
        fast.set_cockpit_look(f32::NAN, f32::INFINITY);
        fast.smooth_look(f32::NAN);
        assert!(fast.look.is_finite());
    }
}

#[cfg(test)]
mod aircraft_tests {
    use super::*;

    #[test]
    fn mesh_follows_identity_and_clears_the_chase_camera_on_change() {
        let mut scene = Scene {
            eye: Some(Vec3::ONE),
            ..Default::default()
        };
        scene.set_aircraft("larrow");
        assert!(scene.larrow && scene.eye.is_none());
        scene.eye = Some(Vec3::ONE);
        scene.set_aircraft("larrow");
        assert_eq!(scene.eye, Some(Vec3::ONE));
        scene.set_aircraft("custom");
        assert!(!scene.larrow && scene.eye.is_none());
    }
}
