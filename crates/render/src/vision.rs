//! Pilot-eye terrain presentation shared by every synthetic-vision display.
use crate::{CORVID_PILOT_EYE_M, Palette, drawing, gpu};
use eframe::egui;
use flightcraft_sim::State;
use flightcraft_world::World;
use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

/// Local PFD pixels per radian at the optical centre (2.5 pixels per degree).
pub const PFD_FOCAL_LENGTH: f32 = 2.5 * 180.0 / std::f32::consts::PI;
/// Perspective ordinate, shared by the ladder and the terrain camera.
pub fn pfd_pitch_y(pitch_rad: f64) -> f32 {
    PFD_FOCAL_LENGTH * pitch_rad.tan() as f32
}

#[derive(Clone, Copy)]
struct VisionImage {
    texture: egui::TextureId,
    prepared: Option<(u64, u64)>,
}
fn image_key() -> egui::Id {
    egui::Id::new("flightcraft-synthetic-vision-image")
}
pub(crate) fn bind_texture(context: &egui::Context, texture: egui::TextureId) {
    context.data_mut(|data| {
        data.insert_temp(
            image_key(),
            VisionImage {
                texture,
                prepared: None,
            },
        )
    });
}

#[derive(Clone, Debug)]
pub struct SyntheticVision {
    pub(crate) world: World,
    position: Vec3,
    attitude: Quat,
    velocity: Vec3,
    altitude_m: f32,
    viewport: [f32; 4],
}
impl SyntheticVision {
    /// Sensor attitude drives the view; indicated altitude drives awareness bands.
    /// The immutable simulation snapshot supplies position, grid heading, and ground velocity.
    pub fn new(world: &World, state: &State, roll: f64, pitch: f64, altitude_m: f64) -> Option<Self> {
        let [w, x, y, z] = state.attitude.map(|v| v as f32);
        let raw = Quat::from_xyzw(x, y, z, w);
        let position = Vec3::from_array(state.position_m.map(|v| v as f32));
        let velocity = Vec3::from_array(state.velocity_mps.map(|v| v as f32));
        if !raw.is_finite()
            || !raw.length_squared().is_finite()
            || raw.length_squared() < 0.001
            || !position.is_finite()
            || !velocity.is_finite()
            || !roll.is_finite()
            || !pitch.is_finite()
            || !altitude_m.is_finite()
            || roll.abs() > std::f64::consts::TAU
            || pitch.abs() > std::f64::consts::PI
            || altitude_m.abs() > 100000.
        {
            return None;
        }
        let forward = raw.normalize() * Vec3::X;
        let yaw = forward.y.atan2(forward.x);
        let attitude = Quat::from_rotation_z(yaw)
            * Quat::from_rotation_y(pitch as f32)
            * Quat::from_rotation_x(roll as f32);
        Some(Self {
            world: world.clone(),
            position,
            attitude,
            velocity,
            altitude_m: altitude_m as f32,
            viewport: [0., 30., 500., 190.],
        })
    }
    /// Extend the PFD background without moving its optical centre or changing its focal scale.
    /// Invalid bounds retain the previous viewport.
    pub fn with_pfd_viewport(mut self, viewport: [f32; 4]) -> Self {
        let [_, _, w, h] = viewport;
        if viewport.iter().all(|v| v.is_finite() && v.abs() <= 10000.) && w > 0. && h > 0. {
            self.viewport = viewport;
        }
        self
    }
    /// Advisory awareness bands are inhibited below 500 ft AGL, and below 1,000 ft
    /// AGL within 1 NM of a runway. Terrain, runway outlines, and SVT remain visible.
    pub fn terrain_awareness_enabled(&self) -> bool {
        let north = f64::from(self.position.x);
        let east = f64::from(self.position.y);
        let agl = -f64::from(self.position.z) - self.world.ground(north, east).height_msl_m;
        if agl < 500. * 0.3048 {
            return false;
        }
        if agl >= 1000. * 0.3048 {
            return true;
        }
        let near = |a: [f64; 2], b: [f64; 2]| {
            let [dn, de] = [b[0] - a[0], b[1] - a[1]];
            let length2 = dn * dn + de * de;
            let along = if length2 > 0. {
                ((north - a[0]) * dn + (east - a[1]) * de) / length2
            } else {
                0.
            }
            .clamp(0., 1.);
            let n = north - a[0] - along * dn;
            let e = east - a[1] - along * de;
            n * n + e * e <= 1852. * 1852.
        };
        if self.world.runway_strips().is_empty() {
            let runway = self.world.runway();
            let [n, e, _] = runway.threshold_ned_m;
            let heading = runway.heading_deg.to_radians();
            !near(
                [n, e],
                [
                    n + runway.length_m * heading.cos(),
                    e + runway.length_m * heading.sin(),
                ],
            )
        } else {
            !self.world.runway_strips().iter().any(|r| {
                let [a, b] = r.ends_m;
                near(a, b)
            })
        }
    }
    fn eye(&self) -> Vec3 {
        self.position + self.attitude * CORVID_PILOT_EYE_M
    }
    pub(crate) fn tile_in_range(&self, center: [f32; 3], half_extent: f32) -> bool {
        let [east, _, back] = center;
        let eye = drawing(self.eye());
        let dx = ((eye.x - east).abs() - half_extent).max(0.);
        let dz = ((eye.z - back).abs() - half_extent).max(0.);
        dx * dx + dz * dz <= 24000. * 24000.
    }
    /// Project a NED direction to the PFD's local attitude canvas, with no edge clamping.
    fn direction(&self, direction: Vec3) -> Option<[f32; 2]> {
        let body = self.attitude.conjugate() * direction;
        (body.is_finite() && body.x > 0.01).then(|| {
            [
                250. + PFD_FOCAL_LENGTH * body.y / body.x,
                125. + PFD_FOCAL_LENGTH * body.z / body.x,
            ]
        })
    }
    /// Ground-relative velocity, including drift in a crosswind; hidden below taxi speed.
    pub fn flight_path_marker(&self) -> Option<[f32; 2]> {
        (self.velocity.length() >= 5.)
            .then(|| self.direction(self.velocity))
            .flatten()
    }
    /// Airports and runway thresholds in front of the aircraft, within ten nautical miles.
    pub fn labels(&self) -> Vec<([f32; 2], String)> {
        let mut candidates = Vec::new();
        let Some(airports) = self.world.airports() else {
            return Vec::new();
        };
        let Some(region) = self.world.region() else {
            return Vec::new();
        };
        let Ok(projection) = flightcraft_world::geo::LocalProjection::new(
            region.origin.latitude_deg,
            region.origin.longitude_deg,
        ) else {
            return Vec::new();
        };
        let mut project = |north: f64, east: f64, elevation: f64, offset: f32, label: String| {
            let delta = Vec3::new(north as f32, east as f32, -elevation as f32) - self.eye();
            if delta.truncate().length() <= 10. * 1852.
                && let Some([x, y]) = self.direction(delta)
            {
                candidates.push((delta.length_squared(), [x, y + offset], label));
            }
        };
        // Runway strips already contain the validated geographic projection, in catalogue order.
        let mut strips = self.world.runway_strips().iter();
        for airport in &airports.airports {
            let ident = airport.icao.as_ref().unwrap_or(&airport.ident);
            if let Ok([n, e]) = projection.project(airport.latitude_deg, airport.longitude_deg) {
                project(n, e, airport.elevation_m, -24., ident.clone());
            }
            for runway in &airport.runways {
                let Some(strip) = strips.next() else {
                    break;
                };
                for ((end, [n, e]), elevation) in runway.ends.iter().zip(strip.ends_m).zip(strip.elevations_m)
                {
                    project(n, e, elevation, 15., format!("{} {}", ident, end.ident));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        candidates.into_iter().map(|(_, at, label)| (at, label)).collect()
    }
    pub(crate) fn uniform(&self, palette: Palette) -> gpu::Uniform {
        let position = drawing(self.position);
        let eye = drawing(self.eye());
        let view = crate::camera::cockpit_view(position, self.attitude, 0., 0., CORVID_PILOT_EYE_M);
        let [x, y, w, h] = self.viewport;
        let fov = 2. * (h * 0.5 / PFD_FOCAL_LENGTH).atan();
        let mut projection = Mat4::perspective_rh(fov, w / h, 0.5, 24000.);
        projection.z_axis.x = 1. - 2. * (250. - x) / w;
        projection.z_axis.y = 2. * (125. - y) / h - 1.;
        gpu::Uniform {
            scenery_bounds: [0.; 4],
            landing_light: [0.; 4],
            atmosphere: [0., 1., 0., 0.],
            fog_color: palette.sky_horizon,
            clouds: [[0.; 4]; 3],
            light_red: palette.light_red,
            viewport: [750., 285., 0., 0.],
            view_projection: (projection * view).to_cols_array_2d(),
            aircraft: Mat4::IDENTITY.to_cols_array_2d(),
            world_from_clip: (projection * Mat4::from_mat3(Mat3::from_mat4(view)))
                .inverse()
                .to_cols_array_2d(),
            sky_top: palette.sky_top,
            sky_horizon: palette.sky_horizon,
            ground: palette.ground,
            dry_grass: palette.dry_grass,
            rock: palette.rock,
            forest: palette.forest,
            shoreline: palette.shoreline,
            water: palette.water,
            grid: palette.grid,
            eye: Vec4::new(eye.x, eye.y, eye.z, 0.).to_array(),
            vision: [
                self.altitude_m,
                1.,
                if self.terrain_awareness_enabled() { 1. } else { 0. },
                0.,
            ],
            caution: palette.terrain_caution,
            warning: palette.terrain_warning,
        }
    }
    /// Enqueue only for an enabled, visible, valid display. The callback shares a bounded-rate
    /// terrain texture, depth target, and the main renderer's immutable terrain buffers.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect) {
        if !rect.is_positive() || !painter.clip_rect().intersects(rect) {
            return;
        }
        let frame = painter.ctx().cumulative_frame_nr();
        let image = painter.ctx().data_mut(|data| {
            let mut image = data.get_temp::<VisionImage>(image_key())?;
            let key = (frame, self.world.fingerprint());
            let prepare = image.prepared != Some(key);
            image.prepared = Some(key);
            data.insert_temp(image_key(), image);
            Some((image.texture, prepare))
        });
        if image.is_none_or(|(_, prepare)| prepare) {
            painter.add(egui_wgpu::Callback::new_paint_callback(
                rect,
                gpu::VisionCallback {
                    composite: image.is_none(),
                    view: self.clone(),
                    frame,
                    time: painter.ctx().input(|i| i.time),
                },
            ));
        }
        if let Some((texture, _)) = image {
            painter.image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                egui::Color32::WHITE,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    #[test]
    fn perspective_horizon_and_ladder_agree_at_level_and_ten_degrees() {
        for pitch in [-10_f64, 0., 10.] {
            for bank in [-30_f64, 0., 30.] {
                let view = SyntheticVision::new(
                    &World::default(),
                    &State::default(),
                    bank.to_radians(),
                    pitch.to_radians(),
                    1000.,
                )
                .unwrap();
                for viewport in [[0., 30., 500., 190.], [0., 0., 500., 440.]] {
                    let view = view.clone().with_pfd_viewport(viewport);
                    let u = view.uniform(crate::mesh::terrain_tests::palette());
                    let matrix = Mat4::from_cols_array_2d(&u.view_projection);
                    let clip = matrix * drawing(Vec3::X).extend(0.);
                    let [x, y, w, h] = viewport;
                    let actual = [
                        x + w * 0.5 * (1. + clip.x / clip.w),
                        y + h * 0.5 * (1. - clip.y / clip.w),
                    ];
                    let y = pfd_pitch_y(pitch.to_radians());
                    let expected = [
                        250. + y * (bank as f32).to_radians().sin(),
                        125. + y * (bank as f32).to_radians().cos(),
                    ];
                    assert!((actual[0] - expected[0]).abs() < 0.001);
                    assert!((actual[1] - expected[1]).abs() < 0.001);
                }
            }
        }
    }
    #[test]
    fn awareness_bands_are_inhibited_on_ground_low_and_near_runways() {
        let world = World::default();
        for (north, east, feet, enabled) in [
            (0., 0., 0., false),
            (4000., 4000., 499., false),
            (0., 0., 750., false),
            (0., 1851., 750., false),
            (0., 1900., 750., true),
            (4000., 4000., 750., true),
            (0., 0., 1001., true),
        ] {
            let state = State {
                position_m: [north, east, -feet * 0.3048],
                ..State::default()
            };
            let view = SyntheticVision::new(&world, &state, 0., 0., feet * 0.3048).unwrap();
            assert_eq!(
                view.terrain_awareness_enabled(),
                enabled,
                "{north},{east} {feet}ft"
            );
            let uniform = view.uniform(crate::mesh::terrain_tests::palette());
            assert_eq!(uniform.vision[1], 1., "SVT itself must stay on");
            assert_eq!(uniform.vision[2], if enabled { 1. } else { 0. });
        }
    }
    #[test]
    fn vertical_pitch_keeps_the_correct_sky_and_ground_side() {
        assert!(pfd_pitch_y(std::f64::consts::FRAC_PI_2) > 1e8);
        assert!(pfd_pitch_y(-std::f64::consts::FRAC_PI_2) < -1e8);
    }
    #[test]
    fn marker_follows_ground_velocity_in_crosswind_and_descent() {
        let state = State {
            velocity_mps: [50., 10., 3.],
            ..State::default()
        };
        let view = SyntheticVision::new(&World::default(), &state, 0., 0., 1000.).unwrap();
        let [x, y] = view.flight_path_marker().unwrap();
        assert!((x - 250. - PFD_FOCAL_LENGTH * 0.2).abs() < 0.001);
        assert!((y - 125. - PFD_FOCAL_LENGTH * 0.06).abs() < 0.001);
        let stopped = SyntheticVision::new(&World::default(), &State::default(), 0., 0., 1000.).unwrap();
        assert!(stopped.flight_path_marker().is_none());
        assert!(SyntheticVision::new(&World::default(), &state, f64::NAN, 0., 1000.).is_none());
    }
}
