//! Read-only node animation and material-color GLB presentation, shared by desktop and web.
use flightcraft_aero::exterior::{ExteriorAsset, ExteriorModel, LightKind, Motion};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::sync::{Arc, OnceLock};

pub fn builtin(id: &str) -> Option<Arc<ExteriorAsset>> {
    static CORVID: OnceLock<Option<Arc<ExteriorAsset>>> = OnceLock::new();
    static LARROW: OnceLock<Option<Arc<ExteriorAsset>>> = OnceLock::new();
    let (slot, definition, lod0, lod1) = match id {
        "corvid" => (
            &CORVID,
            include_str!("../../../aircraft/corvid/exterior.json"),
            include_bytes!("../../../aircraft/corvid/exterior.glb").as_slice(),
            include_bytes!("../../../aircraft/corvid/exterior-lod1.glb").as_slice(),
        ),
        "larrow" => (
            &LARROW,
            include_str!("../../../aircraft/larrow/exterior.json"),
            include_bytes!("../../../aircraft/larrow/exterior.glb").as_slice(),
            include_bytes!("../../../aircraft/larrow/exterior-lod1.glb").as_slice(),
        ),
        _ => return None,
    };
    slot.get_or_init(|| {
        let definition = serde_json::from_str(definition).ok()?;
        Some(Arc::new(ExteriorAsset {
            lod0: Arc::new(flightcraft_aero::exterior::load_glb(lod0, &definition).ok()?),
            lod1: Arc::new(flightcraft_aero::exterior::load_glb(lod1, &definition).ok()?),
            definition,
        }))
    })
    .clone()
}

#[derive(Clone, Debug, Default)]
pub struct Pose {
    pub controls: [f32; 4],
    pub propeller_rpm: f32,
    pub propeller_phase: f32,
    pub wheel_phase: [f32; 16],
    pub compression_m: [f32; 16],
    pub cg_m: [f32; 3],
    pub time_s: f32,
    pub height_agl_m: f32,
    /// Navigation, strobe, beacon, landing, after switch and electrical power checks.
    pub lights: [bool; 4],
    pub powered_loads: std::collections::BTreeMap<String, bool>,
    previous_time_s: Option<f64>,
}
impl Pose {
    pub fn update(&mut self, sim: &flightcraft_sim::Sim) {
        let r = sim.readout();
        let s = sim.state();
        let a = sim.aircraft();
        let dt = self.previous_time_s.map_or(0., |t| (s.time_s - t).clamp(0., 1.)) as f32;
        self.previous_time_s = Some(s.time_s);
        let c = r.effective_controls;
        self.controls = [
            c.aileron as f32,
            c.elevator as f32,
            c.rudder as f32,
            c.flaps as f32,
        ];
        self.cg_m = a.mass.cg_m.map(|x| x as f32);
        self.time_s = s.time_s as f32;
        self.height_agl_m = r.height_agl_m as f32;
        self.propeller_rpm = sim
            .systems()
            .modern
            .as_ref()
            .map_or(r.engine_rpm as f32, |m| m.propeller_rpm as f32);
        self.propeller_phase = (self.propeller_phase + self.propeller_rpm * dt * std::f32::consts::TAU / 60.)
            .rem_euclid(std::f32::consts::TAU);
        let [w, x, y, z] = s.attitude.map(|v| v as f32);
        let attitude = Quat::from_xyzw(x, y, z, w);
        for (i, g) in a.geometry.gear.iter().enumerate() {
            let p = Vec3::from_array(a.relative_point(g.point_m).map(|x| x as f32));
            let point = Vec3::from_array(s.position_m.map(|x| x as f32)) + attitude * p;
            let ground = sim.world().ground(f64::from(point.x), f64::from(point.y));
            let compression =
                (point.z + ground.height_msl_m as f32).clamp(0., a.systems.gear_travel_m as f32);
            if let Some(value) = self.compression_m.get_mut(i) {
                *value = compression;
            }
            // Distance is integrated only in contact; airborne wheels retain their last phase.
            if compression > 0.
                && let Some(value) = self.wheel_phase.get_mut(i)
            {
                let velocity = Vec3::from_array(s.velocity_mps.map(|v| v as f32))
                    + attitude * Vec3::from_array(s.angular_rate_rps.map(|v| v as f32)).cross(p);
                let forward =
                    attitude * Quat::from_rotation_z(g.steering_rad as f32 * c.rudder as f32) * Vec3::X;
                *value += velocity.dot(forward) * dt;
            }
        }
        self.powered_loads = sim
            .systems()
            .electrical
            .loads
            .iter()
            .map(|l| (l.id.clone(), l.powered))
            .collect();
        use flightcraft_aero::LoadSwitch;
        for (out, switch) in self.lights.iter_mut().zip([
            LoadSwitch::Nav,
            LoadSwitch::Strobe,
            LoadSwitch::Beacon,
            LoadSwitch::Landing,
        ]) {
            *out = sim.light_powered(switch);
        }
    }
    fn light_on(&self, kind: LightKind, supply: Option<bool>) -> bool {
        let group = match kind {
            LightKind::NavRed | LightKind::NavGreen | LightKind::NavWhite => 0,
            LightKind::Strobe => 1,
            LightKind::Beacon => 2,
            LightKind::Landing => 3,
        };
        supply.unwrap_or(self.lights.get(group).copied().unwrap_or(false))
            && match kind {
                LightKind::Strobe => self.time_s.rem_euclid(1.2) < 0.09,
                LightKind::Beacon => self.time_s.rem_euclid(1.) < 0.16,
                _ => true,
            }
    }
    pub(crate) fn lamp_on(&self, light: &flightcraft_aero::exterior::ExteriorLight) -> bool {
        self.light_on(
            light.kind,
            light
                .power_load
                .as_ref()
                .and_then(|id| self.powered_loads.get(id))
                .copied(),
        )
    }
}

/// Absolute transforms in pack body coordinates; children inherit steering and spring movement.
pub fn transforms(asset: &ExteriorAsset, model: &ExteriorModel, pose: &Pose) -> Vec<Mat4> {
    let local: Vec<_> = model
        .nodes
        .iter()
        .map(|n| {
            let mut matrix = Mat4::from_cols_array_2d(&n.transform);
            if let Some(p) = asset.definition.moving_parts.iter().find(|p| p.node == n.name) {
                let [low, high] = p.travel_rad;
                let normalized = match p.motion {
                    Motion::Aileron => pose.controls.first(),
                    Motion::Elevator => pose.controls.get(1),
                    Motion::Rudder | Motion::Steering => pose.controls.get(2),
                    Motion::Flaps => pose.controls.get(3),
                    _ => None,
                }
                .copied()
                .unwrap_or(0.)
                .clamp(-1., 1.);
                let angle = match p.motion {
                    Motion::Propeller => pose.propeller_phase,
                    Motion::Wheel => {
                        -pose.wheel_phase.get(p.gear_index).copied().unwrap_or(0.) / p.wheel_radius_m
                    }
                    Motion::Flaps => low + (high - low) * normalized.max(0.),
                    _ => {
                        if normalized < 0. {
                            -normalized * low
                        } else {
                            normalized * high
                        }
                    }
                };
                if p.motion == Motion::Oleo {
                    matrix *= Mat4::from_translation(
                        Vec3::from_array(p.axis)
                            * pose.compression_m.get(p.gear_index).copied().unwrap_or(0.),
                    );
                } else {
                    matrix *= Mat4::from_quat(Quat::from_axis_angle(Vec3::from_array(p.axis), angle));
                }
            }
            matrix
        })
        .collect();
    let mut absolute = local.clone();
    for (i, n) in model.nodes.iter().enumerate() {
        let mut matrix = local.get(i).copied().unwrap_or(Mat4::IDENTITY);
        let mut parent = n.parent;
        // The loader rejects cycles and bounds the hierarchy to 128 nodes.
        for _ in 0..model.nodes.len() {
            let Some(p) = parent else {
                break;
            };
            let Some(node) = model.nodes.get(p) else {
                break;
            };
            matrix = local.get(p).copied().unwrap_or(Mat4::IDENTITY) * matrix;
            parent = node.parent;
        }
        if let Some(out) = absolute.get_mut(i) {
            *out = matrix;
        }
    }
    absolute
}

pub(crate) struct Frame {
    pub opaque: Vec<crate::mesh::Vertex>,
    pub transparent: Vec<crate::mesh::Vertex>,
}
pub(crate) fn frame(asset: &ExteriorAsset, pose: &Pose, distance: f32) -> Frame {
    let model = if distance > 150. { &asset.lod1 } else { &asset.lod0 };
    let matrices = transforms(asset, model, pose);
    let mut out = Frame {
        opaque: Vec::new(),
        transparent: Vec::new(),
    };
    for (i, n) in model.nodes.iter().enumerate() {
        let motion = asset
            .definition
            .moving_parts
            .iter()
            .find(|p| p.node == n.name)
            .map(|p| p.motion);
        let blur = pose.propeller_rpm > 650.;
        if (motion == Some(Motion::Propeller) && blur) || (motion == Some(Motion::PropBlur) && !blur) {
            continue;
        }
        let light = asset.definition.lights.iter().find(|l| l.node == n.name);
        let matrix = matrices.get(i).copied().unwrap_or(Mat4::IDENTITY);
        let normal_matrix = Mat3::from_mat4(matrix).inverse().transpose();
        let luminous = light.is_some_and(|l| pose.lamp_on(l));
        for v in &n.vertices {
            let mut color = v.color;
            if light.is_some() && !luminous {
                for c in color.iter_mut().take(3) {
                    *c *= 0.12;
                }
            }
            let vertex = crate::mesh::Vertex {
                position: (matrix.transform_point3(Vec3::from_array(v.position))
                    - Vec3::from_array(pose.cg_m))
                .to_array(),
                normal: (normal_matrix * Vec3::from_array(v.normal))
                    .normalize_or_zero()
                    .to_array(),
                color,
                kind: if luminous {
                    -2.
                } else if motion == Some(Motion::PropBlur) {
                    -3.
                } else {
                    1.
                },
                water_coverage: 0.,
            };
            if motion == Some(Motion::PropBlur) {
                out.transparent.push(vertex);
            } else {
                out.opaque.push(vertex);
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    fn models() -> Vec<Arc<ExteriorAsset>> {
        let mut models = vec![builtin("corvid").unwrap(), builtin("larrow").unwrap()];
        if let Ok(dir) = std::env::var("FLIGHTCRAFT_SLING_PACK") {
            let path = std::path::Path::new(&dir);
            let definition =
                serde_json::from_slice(&std::fs::read(path.join("exterior.json")).unwrap()).unwrap();
            let lod0 = Arc::new(
                flightcraft_aero::exterior::load_glb(
                    &std::fs::read(path.join("exterior.glb")).unwrap(),
                    &definition,
                )
                .unwrap(),
            );
            let lod1 = Arc::new(
                flightcraft_aero::exterior::load_glb(
                    &std::fs::read(path.join("exterior-lod1.glb")).unwrap(),
                    &definition,
                )
                .unwrap(),
            );
            models.push(Arc::new(ExteriorAsset {
                definition,
                lod0,
                lod1,
            }));
        }
        models
    }
    #[test]
    fn bounds_budget_and_every_moving_node_at_both_lods() {
        for asset in models() {
            assert!(["corvid", "larrow", "sling2"].contains(&asset.definition.aircraft.as_str()));
            if let Ok(configuration) = flightcraft_aero::AircraftCatalog::builtins()
                .unwrap()
                .resolve_aircraft(&asset.definition.aircraft)
            {
                assert!(
                    (asset.definition.span_m as f64 - configuration.aircraft.geometry.span_m).abs() < 0.001
                );
                assert!(
                    (asset.definition.prop_diameter_m as f64
                        - configuration.aircraft.propulsion.prop_diameter_m)
                        .abs()
                        < 0.001
                );
            }
            assert!(asset.lod1.triangles < asset.lod0.triangles);
            for model in [&asset.lod0, &asset.lod1] {
                let neutral = Pose::default();
                let base = transforms(&asset, model, &neutral);
                let mut low = Vec3::splat(f32::INFINITY);
                let mut high = Vec3::splat(f32::NEG_INFINITY);
                for (n, m) in model.nodes.iter().zip(&base) {
                    for v in &n.vertices {
                        let p = m.transform_point3(Vec3::from_array(v.position));
                        low = low.min(p);
                        high = high.max(p);
                    }
                }
                let size = high - low;
                assert!(
                    (size.y / asset.definition.span_m - 1.).abs() < 0.02,
                    "{} span {}",
                    asset.definition.aircraft,
                    size.y
                );
                assert!(
                    (size.x / asset.definition.length_m - 1.).abs() < 0.02,
                    "{} length {}",
                    asset.definition.aircraft,
                    size.x
                );
                assert!(
                    (size.z / asset.definition.height_m - 1.).abs() < 0.02,
                    "{} height {}",
                    asset.definition.aircraft,
                    size.z
                );
                for part in &asset.definition.moving_parts {
                    let i = model.nodes.iter().position(|n| n.name == part.node).unwrap();
                    let mut pose = Pose::default();
                    match part.motion {
                        Motion::Aileron => pose.controls[0] = 1.,
                        Motion::Elevator => pose.controls[1] = 1.,
                        Motion::Rudder | Motion::Steering => pose.controls[2] = 1.,
                        Motion::Flaps => pose.controls[3] = 1.,
                        Motion::Propeller => pose.propeller_phase = 1.,
                        Motion::Wheel => pose.wheel_phase[part.gear_index] = 0.2,
                        Motion::Oleo => pose.compression_m[part.gear_index] = 0.1,
                        Motion::PropBlur => continue,
                    }
                    let changed = transforms(&asset, model, &pose);
                    let before = base[i].transform_point3(Vec3::new(-0.2, 0., 0.));
                    let after = changed[i].transform_point3(Vec3::new(-0.2, 0., 0.));
                    match part.motion {
                        Motion::Elevator => assert!(
                            after.z < before.z,
                            "positive elevator must raise its trailing edge"
                        ),
                        Motion::Flaps => assert!(after.z > before.z, "flaps must lower their trailing edges"),
                        Motion::Rudder => assert!(
                            after.y > before.y,
                            "right rudder must move its trailing edge right"
                        ),
                        Motion::Oleo => assert!(after.z < before.z, "compression must raise the axle"),
                        _ => {}
                    }
                    assert_ne!(base[i], changed[i], "{} {}", asset.definition.aircraft, part.node);
                    let own = if model.nodes[i].vertices.is_empty() {
                        model.nodes.iter().position(|n| n.parent == Some(i)).unwrap()
                    } else {
                        i
                    };
                    assert!(model.nodes[own].vertices.iter().any(|v| {
                        base[own]
                            .transform_point3(Vec3::from_array(v.position))
                            .distance(changed[own].transform_point3(Vec3::from_array(v.position)))
                            > 0.0001
                    }));
                }
            }
        }
    }
    #[test]
    fn sim_effective_surfaces_and_contact_springs_drive_the_pose() {
        let mut sim = flightcraft_sim::Sim::new(&flightcraft_sim::Start::Runway).unwrap();
        sim.set_controls(flightcraft_sim::Controls {
            aileron: 0.3,
            elevator: 0.4,
            rudder: 0.2,
            flaps: 0.5,
            ..Default::default()
        });
        let mut failures = sim.failures();
        failures.elevator = Some(-0.25);
        sim.set_failures(failures).unwrap();
        sim.step(60);
        let mut pose = Pose::default();
        pose.update(&sim);
        assert_eq!(
            pose.controls[1], -0.25,
            "surface jams must override pilot demand visually"
        );
        assert_eq!(pose.controls[0], 0.3);
        assert!(
            pose.compression_m.iter().take(3).any(|c| *c > 0.001),
            "loaded contact springs must shorten the gear"
        );
    }

    #[test]
    fn blur_and_lights_follow_rpm_switches_and_simulation_time() {
        let asset = builtin("corvid").unwrap();
        let mut pose = Pose::default();
        let idle = frame(&asset, &pose, 20.);
        assert!(idle.transparent.is_empty());
        assert!(idle.opaque.iter().all(|v| v.kind != -2.));
        pose.propeller_rpm = 1200.;
        pose.lights = [true; 4];
        let lit = frame(&asset, &pose, 20.);
        assert!(!lit.transparent.is_empty());
        assert!(lit.opaque.iter().any(|v| v.kind == -2.));
        assert!(pose.light_on(LightKind::Strobe, None));
        pose.time_s = 0.5;
        assert!(!pose.light_on(LightKind::Strobe, None));
        assert!(!pose.light_on(LightKind::Beacon, None));
        assert!(pose.light_on(LightKind::NavRed, None));
        let mut sim = flightcraft_sim::Sim::new(&flightcraft_sim::Start::Runway).unwrap();
        pose.update(&sim);
        let before = pose.clone();
        pose.update(&sim);
        assert_eq!(before.propeller_phase, pose.propeller_phase);
        assert_eq!(before.wheel_phase, pose.wheel_phase);
        sim.step(1);
        pose.update(&sim);
    }
}
