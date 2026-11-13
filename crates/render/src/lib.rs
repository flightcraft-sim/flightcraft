//! Read-only 3D presentation. NED and body coordinates are converted here only.

mod camera;
pub mod exterior;
mod light_gun;
pub mod lighting;
mod registration;
pub use light_gun::{LightGun, LightGunPattern};
mod gpu;
mod mesh;
mod registration_atlas;
pub mod scenery;
mod shadows;
mod vision;
pub mod walkaround;
mod weather;
pub use gpu::{SyntheticVisionStats, initialize_synthetic_vision, synthetic_vision_stats};
pub use vision::{PFD_FOCAL_LENGTH, SyntheticVision, pfd_pitch_y};

pub use camera::{CORVID_PILOT_EYE_M, VERTICAL_FOV_RAD, attitude_horizon};
pub use gpu::{device_lost, initialize};

use eframe::egui;
use flightcraft_sim::State;
use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

/// Scene colours supplied by the UI's single theme module.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub airspace_b: [f32; 4],
    pub airspace_c: [f32; 4],
    pub airspace_d: [f32; 4],
    pub airspace_e: [f32; 4],
    pub airspace_mode_c: [f32; 4],
    pub terrain_caution: [f32; 4],
    pub terrain_warning: [f32; 4],
    pub sky_top: [f32; 4],
    pub sky_horizon: [f32; 4],
    pub ground: [f32; 4],
    pub earth: [f32; 4],
    pub dry_grass: [f32; 4],
    pub rock: [f32; 4],
    pub forest: [f32; 4],
    pub shoreline: [f32; 4],
    pub water: [f32; 4],
    pub grid: [f32; 4],
    pub runway: [f32; 4],
    pub marking: [f32; 4],
    pub light_red: [f32; 4],
    pub light_green: [f32; 4],
    pub light_amber: [f32; 4],
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
    Wing,
    Tower,
    Flyby,
    /// The session's pre-flight camera framing, from the walkaround scene.
    Walkaround,
}

/// The near plane: as far out as the view allows, since depth precision scales with it. Inside the
/// cabin and on the wing the panel and wing are centimetres away; the chase camera sits metres
/// behind the aircraft and the tower and fly-by cameras farther still. WebGL2 has no depth clip
/// control, so the reversed float buffer there gains little and the near plane carries the
/// precision that keeps water and the riverbed under it from speckling.
fn near_plane_m(camera: CameraMode, chase_distance_m: f32) -> f32 {
    match camera {
        CameraMode::Wing | CameraMode::Cockpit | CameraMode::Walkaround => 0.015,
        CameraMode::Chase => (chase_distance_m * 0.25).clamp(0.05, 1.0),
        CameraMode::Tower | CameraMode::Flyby => 1.0,
    }
}

pub struct Scene {
    scenery: Option<std::sync::Arc<std::sync::Mutex<scenery::Stream>>>,
    imagery: Option<std::sync::Arc<std::sync::Mutex<scenery::Stream>>>,
    pub scenery_quality: scenery::Quality,
    gpu_view: std::sync::Arc<std::sync::Mutex<Option<gpu::ViewGpu>>>,
    world: flightcraft_world::World,
    pub airspace_visible: bool,
    /// Read-only presentation switch for comparative lighting captures.
    pub lights_visible: bool,
    /// Comparative rendering switch; receivers and geometry remain unchanged.
    pub aircraft_shadow: bool,
    eye: Option<Vec3>,
    target: Option<Vec3>,
    chase_position: Option<Vec3>,
    pub camera: CameraMode,
    screen_view: Option<ScreenView>,
    chase_distance_m: f32,
    chase_height_m: f32,
    look_target: Vec2,
    look: Vec2,
    propeller_power: f32,
    exterior: Option<std::sync::Arc<flightcraft_aero::exterior::ExteriorAsset>>,
    exterior_pose: exterior::Pose,
    look_target_ned_m: Option<[f32; 3]>,
    registration: Option<registration::Override>,
    exterior_environment: Option<std::sync::Arc<exterior::ExteriorEnvironment>>,
    larrow: bool,
    sling: bool,
    conditions: flightcraft_world::Conditions,
    lighting: Option<lighting::LightingSettings>,
    hood: bool,
    light_gun: Option<LightGun>,
    director_runway: Option<([f64; 3], f64)>,
    previous_camera: Option<CameraMode>,
    canopy_latch_screen: Option<egui::Pos2>,
    canopy_drag_screen: Option<(egui::Pos2, egui::Vec2)>,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            scenery: None,
            imagery: None,
            scenery_quality: Default::default(),
            gpu_view: Default::default(),
            world: flightcraft_world::World::default(),
            airspace_visible: true,
            lights_visible: true,
            aircraft_shadow: true,
            eye: None,
            target: None,
            chase_position: None,
            camera: CameraMode::Chase,
            screen_view: None,
            chase_distance_m: 19.0,
            chase_height_m: 7.0,
            look_target: Vec2::ZERO,
            look: Vec2::ZERO,
            propeller_power: 0.0,
            exterior: exterior::builtin("corvid"),
            exterior_pose: Default::default(),
            look_target_ned_m: None,
            registration: None,
            exterior_environment: None,
            larrow: false,
            sling: false,
            conditions: Default::default(),
            lighting: None,
            hood: false,
            light_gun: None,
            director_runway: None,
            previous_camera: None,
            canopy_latch_screen: None,
            canopy_drag_screen: None,
        }
    }
}

fn drawing(v: Vec3) -> Vec3 {
    Vec3::new(v.y, -v.z, -v.x)
}

/// Independent camera settings for a page; never change the flight or another window.
#[derive(Clone, Copy, Debug)]
pub struct ScreenView {
    pub heading_offset_rad: f32,
    pub pitch_rad: f32,
    pub fov_rad: f32,
    pub fixed_eye_ned_m: Option<[f32; 3]>,
}
impl Scene {
    pub fn set_scenery(&mut self, stream: Option<std::sync::Arc<std::sync::Mutex<scenery::Stream>>>) {
        self.scenery = stream;
    }

    pub fn set_imagery(&mut self, stream: Option<std::sync::Arc<std::sync::Mutex<scenery::Stream>>>) {
        self.imagery = stream;
    }

    /// The terrain this scene draws.
    #[must_use]
    pub fn world(&self) -> &flightcraft_world::World {
        &self.world
    }

    /// The photoreal scenery and imagery streams this scene selects from.
    #[must_use]
    pub fn scenery_streams(&self) -> [Option<&std::sync::Arc<std::sync::Mutex<scenery::Stream>>>; 2] {
        [self.scenery.as_ref(), self.imagery.as_ref()]
    }

    /// The exterior model of the aircraft being drawn, for framing its moving parts.
    #[must_use]
    pub fn exterior_asset(&self) -> Option<&std::sync::Arc<flightcraft_aero::exterior::ExteriorAsset>> {
        self.exterior.as_ref()
    }

    /// With a fixed-eye screen view, look at this world point instead of the centre of gravity.
    pub fn set_look_target(&mut self, target_ned_m: Option<[f32; 3]>) {
        self.look_target_ned_m = target_ned_m;
    }

    pub fn set_screen_view(&mut self, view: Option<ScreenView>) {
        self.screen_view = view;
    }

    pub fn clear_lighting(&mut self) {
        self.lighting = None;
    }

    /// Validated UTC presentation clock; the flight trajectory is unchanged.
    pub fn set_lighting(&mut self, settings: lighting::LightingSettings) -> Result<(), String> {
        settings.validate()?;
        self.lighting = Some(settings);
        Ok(())
    }

    pub fn set_weather(&mut self, conditions: &flightcraft_world::Conditions, hood: bool) {
        if conditions.validate().is_ok() {
            self.conditions = conditions.clone();
        }
        self.hood = hood;
    }

    /// Select the built-in display mesh by the active definition's identity.
    /// Unknown identities use the default trainer outline.
    pub fn set_aircraft(&mut self, id: &str) {
        let id = match id {
            "corvid-standby" | "corvid-glass" => "corvid",
            id => id,
        };
        if self.exterior.as_ref().is_none_or(|a| a.definition.aircraft != id) {
            self.exterior = exterior::builtin(id);
        }
        let larrow = id == "larrow";
        let sling = id == "sling2";
        if self.larrow != larrow || self.sling != sling {
            self.sling = sling;
            self.larrow = larrow;
            self.reset_camera();
        }
    }

    /// Installed pack geometry overrides the embedded fictional model.
    /// Supply scenery lighting and an optional GGX-prefiltered sky probe to all exterior views.
    pub fn set_exterior_environment(
        &mut self,
        environment: Option<std::sync::Arc<exterior::ExteriorEnvironment>>,
    ) {
        self.exterior_environment = environment;
    }

    pub fn set_exterior(&mut self, asset: std::sync::Arc<flightcraft_aero::exterior::ExteriorAsset>) {
        self.exterior = Some(asset);
    }

    /// Paint an optional user registration on both pack surfaces. Cache until identity or pack changes.
    pub fn set_registration(&mut self, text: Option<&str>) {
        if let Some(cached) = &self.registration
            && self
                .exterior
                .as_ref()
                .is_some_and(|a| std::sync::Arc::ptr_eq(a, &cached.painted))
        {
            self.exterior = Some(cached.base.clone());
        }
        let Some(text) = text.filter(|s| {
            !s.is_empty()
                && s.len() <= 12
                && s.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        }) else {
            return;
        };
        let Some(base) = &self.exterior else { return };
        if let Some(cached) = &self.registration
            && std::sync::Arc::ptr_eq(base, &cached.base)
            && cached.text == text
        {
            self.exterior = Some(cached.painted.clone());
            return;
        }
        let painted = registration::paint(base, text);
        self.registration = Some(registration::Override {
            base: base.clone(),
            text: text.into(),
            painted: painted.clone(),
        });
        self.exterior = Some(painted);
    }

    /// Bounded presentation pose for replay and offscreen clients; never changes simulation mass or controls.
    pub fn set_exterior_pose(&mut self, pose: exterior::Pose) -> Result<(), String> {
        if pose
            .cg_m
            .iter()
            .chain(&pose.controls)
            .chain(&pose.wheel_phase)
            .chain(&pose.compression_m)
            .any(|v| !v.is_finite() || v.abs() > 100000.)
            || !pose.canopy_open_fraction.is_finite()
            || !pose.propeller_rpm.is_finite()
            || !pose.propeller_phase.is_finite()
            || !pose.time_s.is_finite()
            || !pose.height_agl_m.is_finite()
            || pose.powered_loads.len() > 4096
        {
            return Err("Invalid aircraft presentation pose".into());
        }
        self.exterior_pose = pose;
        Ok(())
    }

    /// The pre-flight figure, props, and close-ups to draw with the aircraft, or none.
    pub fn set_walkaround(&mut self, scene: Option<flightcraft_aero::walkaround::WalkaroundScene>) {
        self.exterior_pose.walkaround = scene.map(std::sync::Arc::new);
    }

    /// Read-only animation inputs; phases advance in simulation time, so pause freezes motion.
    pub fn set_exterior_state(&mut self, sim: &flightcraft_sim::Sim) {
        self.exterior_pose.update(sim);
    }

    /// Share the session's immutable terrain; GPU meshes are cached by content identity.
    pub fn set_world(&mut self, world: &flightcraft_world::World) {
        self.world = world.clone();
    }

    /// A virtual airport observer uses the assigned runway geometry, never aircraft-relative placement.
    pub fn set_director_runway(&mut self, runway: Option<([f64; 3], f64)>) {
        let runway = runway
            .filter(|(position, heading)| position.iter().all(|x| x.is_finite()) && heading.is_finite());
        if self.director_runway != runway {
            self.director_runway = runway;
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
            _ => CameraMode::Chase,
        };
        self.reset_camera();
    }

    pub fn reset_camera(&mut self) {
        self.eye = None;
        self.target = None;
        self.chase_position = None;
    }

    fn smooth_look(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        self.look = self.look.lerp(self.look_target, 1.0 - (-dt * 9.0).exp());
    }

    /// Projected centre of the authored inside latch, when visible in cockpit view.
    pub fn canopy_latch_screen(&self) -> Option<egui::Pos2> {
        self.canopy_latch_screen
    }

    /// Moving front-bow hit point and projected aft direction; UI owns the commands.
    pub fn canopy_drag_screen(&self) -> Option<(egui::Pos2, egui::Vec2)> {
        self.canopy_drag_screen
    }

    // Shared by the viewport and headless motion tests.
    fn camera_frame(&mut self, state: &State, dt: f32) -> (Vec3, Mat4, Mat4, f32) {
        if self.previous_camera != Some(self.camera) {
            self.reset_camera();
            self.previous_camera = Some(self.camera);
        }
        let mut vertical_fov = camera::VERTICAL_FOV_RAD;
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
        // Exponential smoothing is stable across long paint gaps; invalid time freezes the slew.
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let blend = 1.0 - (-dt * 7.0).exp();
        let pilot_eye = if let Some(interior) = self
            .exterior
            .as_ref()
            .and_then(|a| a.definition.interior.as_ref())
        {
            Vec3::from_array(interior.pilot_eye_m) - Vec3::from_array(self.exterior_pose.cg_m)
        } else if self.larrow {
            Vec3::new(-0.1, 0.0, -0.63)
        } else if self.sling {
            Vec3::new(-0.1, -0.25, -0.65)
        } else {
            CORVID_PILOT_EYE_M
        };
        let (eye, view) = match self.camera {
            CameraMode::Chase => {
                let desired_eye = position
                    + drawing(attitude * Vec3::new(-self.chase_distance_m, 0.0, -self.chase_height_m));
                let desired_target = position + drawing(attitude * Vec3::new(10.0, 0.0, -0.3));
                // Carry the camera with every aircraft translation, including sparse session
                // snapshots and zero-time repaints. Slew only the relative offset and aim;
                // smoothing world positions adds speed-dependent lag behind the aircraft.
                let translation = self
                    .chase_position
                    .map_or(Vec3::ZERO, |previous| position - previous);
                self.chase_position = Some(position);
                let eye = self
                    .eye
                    .map_or(desired_eye, |v| (v + translation).lerp(desired_eye, blend));
                let target = self
                    .target
                    .map_or(desired_target, |v| (v + translation).lerp(desired_target, blend));
                self.eye = Some(eye);
                self.target = Some(target);
                (eye, Mat4::look_at_rh(eye, target, Vec3::Y))
            }
            CameraMode::Wing => {
                let eye = position + drawing(attitude * Vec3::new(-1.0, -7.0, -1.4));
                (
                    eye,
                    Mat4::look_at_rh(
                        eye,
                        position + drawing(attitude * Vec3::new(3.0, 0.0, -0.2)),
                        Vec3::Y,
                    ),
                )
            }
            CameraMode::Tower => {
                let (threshold, heading) = self.director_runway.unwrap_or_else(|| {
                    let r = self.world.runway();
                    (r.threshold_ned_m, r.heading_deg.to_radians())
                });
                let [n, e, d] = threshold.map(|x| x as f32);
                let (sin, cos) = (heading as f32).sin_cos();
                let threshold = drawing(Vec3::new(n, e, d));
                let eye =
                    threshold + drawing(Vec3::new(-450. * cos + 30. * sin, -450. * sin - 30. * cos, -12.));
                let ground = self.world.ground(
                    f64::from(n - 450. * cos + 30. * sin),
                    f64::from(e - 450. * sin - 30. * cos),
                );
                let eye = Vec3::new(eye.x, eye.y.max(ground.height_msl_m as f32 + 12.), eye.z);
                // Pan from the fixed airport observer. Biasing aim toward the threshold
                // loses a climbing aircraft outside this deliberately narrow field of view.
                let target = position;
                vertical_fov = (2. * (2.3 / (2. * eye.distance(position).max(1.) * 0.065)).atan())
                    .clamp(4_f32.to_radians(), 35_f32.to_radians());
                self.eye = Some(eye);
                (eye, Mat4::look_at_rh(eye, target, Vec3::Y))
            }
            CameraMode::Flyby => {
                let [vn, ve, _] = state.velocity_mps.map(|x| x as f32);
                let track = Vec3::new(vn, ve, 0.).try_normalize().unwrap_or(Vec3::X);
                let right = Vec3::new(-track.y, track.x, 0.);
                let desired = position + drawing(track * 350. + right * 30. + Vec3::new(0., 0., -10.));
                let eye = self
                    .eye
                    .filter(|eye| eye.distance(position) < 800.)
                    .unwrap_or(desired);
                self.eye = Some(eye);
                vertical_fov = (2. * (2.3 / (2. * eye.distance(position).max(1.) * 0.07)).atan())
                    .clamp(5_f32.to_radians(), 60_f32.to_radians());
                (eye, Mat4::look_at_rh(eye, position, Vec3::Y))
            }
            CameraMode::Cockpit => {
                let eye = position + drawing(attitude * pilot_eye);
                (
                    eye,
                    camera::cockpit_view(position, attitude, self.look.x, self.look.y, pilot_eye),
                )
            }
            CameraMode::Walkaround => {
                let cg = Vec3::from_array(self.exterior_pose.cg_m);
                let frame = self.exterior_pose.walkaround.as_ref().and_then(|w| w.camera);
                let (eye_body, target_body, fov) = frame.map_or(
                    (Vec3::new(-1.0, -7.0, -1.4), Vec3::new(-1.6, 0., -0.15), 50_f32),
                    |c| (Vec3::from_array(c.eye_m), Vec3::from_array(c.target_m), c.fov_deg),
                );
                let eye = position + drawing(attitude * (eye_body - cg));
                let target = position + drawing(attitude * (target_body - cg));
                vertical_fov = fov.to_radians().clamp(10_f32.to_radians(), 100_f32.to_radians());
                (eye, Mat4::look_at_rh(eye, target, Vec3::Y))
            }
        };
        let (eye, view) = if let Some(settings) = self.screen_view {
            if let Some(fixed) = settings.fixed_eye_ned_m {
                let eye = drawing(Vec3::from_array(fixed));
                let target = self
                    .look_target_ned_m
                    .map_or(position, |t| drawing(Vec3::from_array(t)));
                let up = if (target - eye).normalize_or_zero().dot(Vec3::Y).abs() > 0.99 {
                    Vec3::NEG_Z
                } else {
                    Vec3::Y
                };
                (eye, Mat4::look_at_rh(eye, target, up))
            } else if self.camera != CameraMode::Cockpit {
                (eye, view)
            } else {
                let eye = position + drawing(attitude * pilot_eye);
                (
                    eye,
                    camera::cockpit_view(
                        position,
                        attitude,
                        settings.heading_offset_rad,
                        settings.pitch_rad,
                        pilot_eye,
                    ),
                )
            }
        } else {
            (eye, view)
        };
        (eye, view, model, vertical_fov)
    }

    /// Enqueue the wgpu callback. The camera follows the body's quaternion, not UI readouts.
    pub fn show(&mut self, ui: &mut egui::Ui, state: &State, dt: f32, palette: Palette) {
        let rect = ui.max_rect();
        let (eye, view, model, vertical_fov) = self.camera_frame(state, dt);
        let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
        let projection = camera::perspective_reversed(
            self.screen_view.map_or(vertical_fov, |s| {
                s.fov_rad.clamp(15_f32.to_radians(), 150_f32.to_radians())
            }),
            aspect,
            near_plane_m(self.camera, self.chase_distance_m),
            16000.0,
        );
        let world_from_clip = (projection * Mat4::from_mat3(Mat3::from_mat4(view))).inverse();
        let mut uniform = gpu::Uniform {
            scenery_bounds: [0.; 4],
            landing_light: [0.; 4],
            atmosphere: [0.; 4],
            fog_color: [0.; 4],
            clouds: [[0.; 4]; 3],
            light_red: palette.light_red,
            viewport: [
                rect.width().max(1.),
                rect.height().max(1.),
                self.exterior_pose.height_agl_m,
                0.,
            ],
            vision: [0.; 4],
            caution: palette.terrain_caution,
            warning: palette.terrain_warning,
            view_projection: (projection * view).to_cols_array_2d(),
            aircraft: model.to_cols_array_2d(),
            world_from_clip: world_from_clip.to_cols_array_2d(),
            sky_top: palette.sky_top,
            sky_horizon: palette.sky_horizon,
            ground: palette.ground,
            dry_grass: palette.dry_grass,
            rock: palette.rock,
            forest: palette.forest,
            shoreline: palette.shoreline,
            water: palette.water,
            grid: palette.grid,
            eye: Vec4::new(eye.x, eye.y, eye.z, self.propeller_power).to_array(),
        };
        weather::apply(&mut uniform, &self.conditions, palette);
        let mut scenery = scenery::Selection::default();
        for (index, source) in [&self.scenery, &self.imagery].into_iter().enumerate() {
            let selection = source
                .as_ref()
                .and_then(|stream| stream.try_lock().ok())
                .map(|mut stream| {
                    stream.select(
                        [-f64::from(eye.z), f64::from(eye.x), -f64::from(eye.y)],
                        f64::from(rect.height()),
                        f64::from(self.screen_view.map_or(vertical_fov, |s| {
                            s.fov_rad.clamp(15_f32.to_radians(), 150_f32.to_radians())
                        })),
                        self.scenery_quality,
                    )
                })
                .unwrap_or_default();
            if index == 0 {
                scenery = selection;
            } else {
                scenery.overlay(selection);
            }
        }
        let scenery_attribution = scenery.attribution.clone();
        let scenery_unavailable = scenery.stats.failed_tiles > 0;
        let scenery_message = scenery.message.clone();
        if scenery.stats.pending_tiles > 0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        if self.lights_visible
            && self.exterior.as_ref().is_some_and(|asset| {
                asset.definition.lights.iter().any(|l| {
                    l.kind == flightcraft_aero::exterior::LightKind::Landing && self.exterior_pose.lamp_on(l)
                })
            })
        {
            uniform.landing_light = (model
                .transform_vector3(Vec3::new(1., 0., 0.12))
                .normalize_or_zero())
            .extend(1.)
            .to_array();
        }
        if self.hood && self.camera == CameraMode::Cockpit {
            uniform.atmosphere[3] = 1.;
            uniform.fog_color = palette.glare_shield;
        }
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            gpu::SceneCallback {
                scenery,
                scenery_quality: self.scenery_quality,
                resources: self.gpu_view.clone(),
                uniform,
                show_airspace: self.airspace_visible,
                show_lights: self.lights_visible,
                aircraft_shadow: self.aircraft_shadow,
                world: self.world.clone(),
                size: [rect.width(), rect.height()],
                show_aircraft: self.camera != CameraMode::Cockpit
                    || self
                        .exterior
                        .as_ref()
                        .is_some_and(|a| a.definition.interior.is_some()),
                larrow: self.larrow,
                sling: self.sling,
                exterior: self.exterior.clone(),
                exterior_pose: self.exterior_pose.clone(),
                exterior_environment: self.exterior_environment.clone(),
                lighting: {
                    let origin = self
                        .world
                        .region()
                        .map(|r| [r.origin.latitude_deg, r.origin.longitude_deg])
                        .unwrap_or([40.97113, -74.99750]);
                    let location = flightcraft_world::geo::LocalProjection::new(origin[0], origin[1])
                        .ok()
                        .and_then(|p| p.unproject(state.position_m[0], state.position_m[1]).ok())
                        .unwrap_or(origin);
                    let settings = self.lighting.unwrap_or_else(|| {
                        let hour = match self.conditions.time_of_day {
                            flightcraft_world::TimeOfDay::Day => 12.,
                            flightcraft_world::TimeOfDay::Dawn => 7.,
                            flightcraft_world::TimeOfDay::Dusk => 17.,
                            flightcraft_world::TimeOfDay::Night => 0.,
                        };
                        lighting::LightingSettings {
                            utc_hour: (hour - location[1] / 15.).rem_euclid(24.),
                            ..Default::default()
                        }
                    });
                    let sun = lighting::sun_direction(settings, location[0], location[1], state.time_s)
                        .unwrap_or([0., 1., 0.]);
                    let cloud = self
                        .conditions
                        .layers
                        .iter()
                        .filter(|l| l.base_ft_msl * 0.3048 > f64::from(eye.y))
                        .map(|l| 1. - 0.78 * l.coverage.fraction() as f32)
                        .product::<f32>();
                    lighting::Frame::new(
                        sun,
                        eye.to_array(),
                        self.conditions.effective_visibility_m() as f32,
                        cloud,
                        settings.exposure_ev,
                    )
                },
            },
        ));
        if !scenery_attribution.is_empty() {
            let color = egui::Color32::from_rgba_unmultiplied(
                (palette.marking.first().copied().unwrap_or(1.) * 255.) as u8,
                (palette.marking.get(1).copied().unwrap_or(1.) * 255.) as u8,
                (palette.marking.get(2).copied().unwrap_or(1.) * 255.) as u8,
                220,
            );
            let galley = ui.painter().layout(
                scenery_attribution,
                egui::FontId::proportional(10.),
                color,
                (rect.width() - 16.).max(1.),
            );
            let position = rect.left_bottom() + egui::vec2(8., -8. - galley.size().y);
            ui.painter().galley(position, galley, color);
        }
        if scenery_unavailable {
            ui.painter().text(
                rect.left_top() + egui::vec2(8., 8.),
                egui::Align2::LEFT_TOP,
                scenery_message
                    .as_deref()
                    .unwrap_or("Some scenery is unavailable"),
                egui::FontId::proportional(12.),
                egui::Color32::from_rgba_unmultiplied(
                    (palette.marking.first().copied().unwrap_or(1.) * 255.) as u8,
                    (palette.marking.get(1).copied().unwrap_or(1.) * 255.) as u8,
                    (palette.marking.get(2).copied().unwrap_or(1.) * 255.) as u8,
                    255,
                ),
            );
        }
        self.canopy_latch_screen = None;
        self.canopy_drag_screen = None;
        if self.camera == CameraMode::Cockpit
            && let Some(asset) = &self.exterior
        {
            let matrices = exterior::transforms(asset, &asset.lod0, &self.exterior_pose);
            let project = |point: Vec3| -> Option<egui::Pos2> {
                let clip = projection * view * model * point.extend(1.);
                if clip.w <= 0. {
                    return None;
                }
                let ndc = clip.truncate() / clip.w;
                Some(egui::pos2(
                    rect.center().x + ndc.x * rect.width() / 2.,
                    rect.center().y - ndc.y * rect.height() / 2.,
                ))
            };
            for (index, node) in asset
                .lod0
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| matches!(n.name.as_str(), "canopy_inside_latch" | "canopy_bow_front"))
            {
                if node.vertices.is_empty() {
                    continue;
                }
                let centre = node
                    .vertices
                    .iter()
                    .map(|v| Vec3::from_array(v.position))
                    .sum::<Vec3>()
                    / node.vertices.len() as f32;
                let point = matrices
                    .get(index)
                    .copied()
                    .unwrap_or(Mat4::IDENTITY)
                    .transform_point3(centre)
                    - Vec3::from_array(self.exterior_pose.cg_m);
                if let Some(screen) = project(point).filter(|p| rect.contains(*p)) {
                    if node.name == "canopy_inside_latch" {
                        self.canopy_latch_screen = Some(screen);
                    } else if let Some(aft) = project(point - Vec3::X * 0.01) {
                        let delta = aft - screen;
                        if delta.length_sq() > 0.001 {
                            self.canopy_drag_screen = Some((screen, delta.normalized()));
                        }
                    }
                }
            }
        }
        self.draw_light_gun(ui, rect, eye, projection * view, palette);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Views with nothing within a metre push the near plane out for depth precision; the cabin
    /// and wing views keep the centimetre plane the panel and wing need.
    #[test]
    fn outside_views_push_the_near_plane_out_and_the_cabin_keeps_it_close() {
        assert_eq!(near_plane_m(CameraMode::Cockpit, 19.), 0.015);
        assert_eq!(near_plane_m(CameraMode::Wing, 19.), 0.015);
        assert_eq!(near_plane_m(CameraMode::Chase, 19.), 1.0);
        assert_eq!(near_plane_m(CameraMode::Chase, 2.), 0.5);
        assert_eq!(near_plane_m(CameraMode::Tower, 19.), 1.0);
    }

    #[test]
    fn north_east_down_maps_to_right_up_back() {
        assert_eq!(drawing(Vec3::X), -Vec3::Z);
        assert_eq!(drawing(Vec3::Y), Vec3::X);
        assert_eq!(drawing(Vec3::Z), -Vec3::Y);
    }

    #[test]
    fn authored_pilot_eye_tracks_the_cabin_for_different_cg_and_looks() {
        let mut asset = (*exterior::builtin("corvid").unwrap()).clone();
        let pilot = [-2.30, -0.205, -0.72];
        asset.definition.interior = Some(flightcraft_aero::exterior::Interior { pilot_eye_m: pilot });
        let mut scene = Scene::default();
        scene.set_exterior(std::sync::Arc::new(asset));
        scene.camera = CameraMode::Cockpit;
        for cg in [[-1.65, 0., 0.], [-1.85, 0.03, 0.02]] {
            scene.exterior_pose.cg_m = cg;
            for yaw in [-2.1, 0., 1.25] {
                scene.set_cockpit_look(yaw, -0.4);
                let (eye, view, model, _) = scene.camera_frame(&State::default(), 1.);
                let expected = model.transform_point3(Vec3::from_array(pilot) - Vec3::from_array(cg));
                assert!((eye - expected).length() < 1e-5);
                assert!(view.transform_point3(eye).length() < 1e-5);
            }
        }
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

#[cfg(test)]
mod exterior_proof;

#[cfg(test)]
mod camera_motion_tests;
