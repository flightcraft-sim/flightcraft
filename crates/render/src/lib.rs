//! Read-only 3D presentation. NED and body coordinates are converted here only.

mod gpu;
mod mesh;

pub use gpu::initialize;

use eframe::egui;
use flightcraft_sim::State;
use glam::{Mat4, Quat, Vec3, Vec4};

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
}

/// Views share the same state, with no simulation side effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraMode {
    #[default]
    Chase,
    Cockpit,
}

#[derive(Default)]
pub struct Scene {
    eye: Option<Vec3>,
    target: Option<Vec3>,
    pub camera: CameraMode,
}

fn drawing(v: Vec3) -> Vec3 {
    Vec3::new(v.y, -v.z, -v.x)
}

impl Scene {
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
        let position =
            drawing(Vec3::new(north, east, down)).max(Vec3::new(f32::NEG_INFINITY, 1.15, f32::NEG_INFINITY));
        let axis = |v| drawing(attitude * v).extend(0.0);
        let model = Mat4::from_cols(axis(Vec3::X), axis(Vec3::Y), axis(Vec3::Z), position.extend(1.0));
        let (desired_eye, desired_target) = match self.camera {
            CameraMode::Chase => (
                position + drawing(attitude * Vec3::new(-19.0, 0.0, -7.0)),
                position + drawing(attitude * Vec3::new(10.0, 0.0, -0.3)),
            ),
            CameraMode::Cockpit => (
                position + drawing(attitude * Vec3::new(0.7, 0.0, -0.45)),
                position + drawing(attitude * Vec3::new(100.0, 0.0, -0.45)),
            ),
        };
        let blend = 1.0 - (-dt.clamp(0.0, 0.25) * 7.0).exp();
        let eye = self.eye.map_or(desired_eye, |v| v.lerp(desired_eye, blend));
        let target = self
            .target
            .map_or(desired_target, |v| v.lerp(desired_target, blend));
        self.eye = Some(eye);
        self.target = Some(target);
        let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
        let view = Mat4::look_at_rh(eye, target, Vec3::Y);
        let projection = Mat4::perspective_rh(52_f32.to_radians(), aspect, 0.15, 16000.0);
        let uniform = gpu::Uniform {
            view_projection: (projection * view).to_cols_array_2d(),
            aircraft: model.to_cols_array_2d(),
            sky_top: palette.sky_top,
            sky_horizon: palette.sky_horizon,
            ground: palette.ground,
            grid: palette.grid,
            eye: Vec4::new(eye.x, eye.y, eye.z, 0.0).to_array(),
        };
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            gpu::SceneCallback {
                uniform,
                size: [rect.width(), rect.height()],
                show_aircraft: self.camera == CameraMode::Chase,
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
}
