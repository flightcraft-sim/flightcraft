//! Read-only exterior animation and bounded PBR drawing, shared by desktop and web.
use flightcraft_aero::exterior::{ExteriorAsset, ExteriorModel, LightKind, Motion};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::sync::{Arc, OnceLock};

pub fn builtin(id: &str) -> Option<Arc<ExteriorAsset>> {
    static CORVID: OnceLock<Option<Arc<ExteriorAsset>>> = OnceLock::new();
    static LARROW: OnceLock<Option<Arc<ExteriorAsset>>> = OnceLock::new();
    let (slot, definition, lod0, lod1) = match id {
        "corvid" | "corvid-standby" | "corvid-glass" => (
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
            lod2: None,
        }))
    })
    .clone()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    pub controls: [f32; 4],
    pub canopy_open_fraction: f32,
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
    /// A pre-flight walkaround: the figure, props, hand-held surfaces, and close-ups.
    pub walkaround: Option<Arc<flightcraft_aero::walkaround::WalkaroundScene>>,
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
        self.canopy_open_fraction = sim.canopy().open_fraction as f32;
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
                // A surface moved by hand overrides the flight controls.
                let held = pose
                    .walkaround
                    .as_ref()
                    .and_then(|w| w.surfaces.iter().find(|s| s.node == n.name))
                    .map(|s| s.value);
                let normalized = held
                    .or_else(|| {
                        match p.motion {
                            Motion::Aileron => pose.controls.first(),
                            Motion::Elevator => pose.controls.get(1),
                            Motion::Rudder | Motion::Steering => pose.controls.get(2),
                            Motion::Flaps => pose.controls.get(3),
                            _ => None,
                        }
                        .copied()
                    })
                    .unwrap_or(0.)
                    .clamp(-1., 1.);
                let angle = match p.motion {
                    Motion::Propeller => pose
                        .walkaround
                        .as_ref()
                        .and_then(|w| w.propeller_phase)
                        .unwrap_or(pose.propeller_phase),
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
                if p.motion == Motion::Slide {
                    let fraction = if pose.canopy_open_fraction.is_finite() {
                        pose.canopy_open_fraction.clamp(0., 1.)
                    } else {
                        0.
                    };
                    matrix *= Mat4::from_translation(
                        Vec3::from_array(p.axis) * p.travel_m.unwrap_or(0.) * fraction,
                    );
                } else if p.motion == Motion::Oleo {
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
    // The figure and props cast shadows with the aircraft.
    if let Some(scene) = &pose.walkaround {
        for primitive in crate::walkaround::primitives(scene, pose.cg_m, 0) {
            out.opaque
                .extend(primitive.vertices.iter().map(|v| crate::mesh::Vertex {
                    position: v.position,
                    normal: v.normal,
                    color: [0.6, 0.6, 0.6, 1.],
                    kind: 1.,
                    water_coverage: 0.,
                }));
        }
    }
    out
}

/// Select a validated pack LOD without changing simulation state.
pub fn model_at_distance(asset: &ExteriorAsset, distance: f32) -> &Arc<ExteriorModel> {
    if distance > 500. {
        asset.lod2.as_ref().unwrap_or(&asset.lod1)
    } else if distance > 150. {
        &asset.lod1
    } else {
        &asset.lod0
    }
}
pub fn pbr_materials(model: &ExteriorModel) -> Vec<PbrMaterial> {
    use flightcraft_aero::exterior::MaterialAlpha;
    let mut materials: Vec<PbrMaterial> = model
        .materials
        .iter()
        .map(|m| PbrMaterial {
            base_color: m.base_color,
            emissive: m.emissive,
            metallic: m.metallic,
            roughness: m.roughness,
            normal_scale: m.normal_scale,
            occlusion_strength: m.occlusion_strength,
            alpha_cutoff: m.alpha_cutoff,
            alpha_mode: match m.alpha_mode {
                MaterialAlpha::Opaque => AlphaMode::Opaque,
                MaterialAlpha::Mask => AlphaMode::Mask,
                MaterialAlpha::Blend => AlphaMode::Blend,
            },
            double_sided: m.double_sided,
            clearcoat: m.clearcoat,
            clearcoat_roughness: m.clearcoat_roughness,
            images: m.images.map(|i| {
                i.and_then(|i| model.images.get(i)).map(|image| PbrImage {
                    size: image.size,
                    rgba: image.rgba.clone(),
                })
            }),
        })
        .collect();
    // The walkaround's own materials follow the model's, whether or not one is drawn.
    materials.extend(crate::walkaround::materials());
    materials
}
/// Apply named part animations, CG offset, and lamp gates to the textured game geometry.
pub fn pbr_primitives(asset: &ExteriorAsset, model: &ExteriorModel, pose: &Pose) -> Vec<PbrPrimitive> {
    let matrices = transforms(asset, model, pose);
    let mut groups: Vec<Vec<PbrVertex>> = (0..model.materials.len()).map(|_| Vec::new()).collect();
    for (i, node) in model.nodes.iter().enumerate() {
        let motion = asset
            .definition
            .moving_parts
            .iter()
            .find(|p| p.node == node.name)
            .map(|p| p.motion);
        let blur = pose.propeller_rpm > 650.;
        if (motion == Some(Motion::Propeller) && blur) || (motion == Some(Motion::PropBlur) && !blur) {
            continue;
        }
        let light = asset.definition.lights.iter().find(|l| l.node == node.name);
        let luminous = light.is_none_or(|l| pose.lamp_on(l));
        let matrix = matrices.get(i).copied().unwrap_or(Mat4::IDENTITY);
        let normal_matrix = Mat3::from_mat4(matrix).inverse().transpose();
        let sign = Mat3::from_mat4(matrix).determinant().signum();
        for v in &node.vertices {
            let normal = (normal_matrix * Vec3::from_array(v.normal)).normalize_or_zero();
            let [tx, ty, tz, tw] = v.tangent;
            let tangent = matrix.transform_vector3(Vec3::new(tx, ty, tz));
            let tangent = tangent - normal * tangent.dot(normal);
            // Blender can export zero/parallel tangents at collapsed UV tips and planar caps.
            // Use a stable orthogonal basis there; never upload a zero normal-map frame.
            let tangent = if tangent.length_squared() > 1e-8 {
                tangent.normalize()
            } else {
                normal
                    .cross(if normal.x.abs() < 0.9 { Vec3::X } else { Vec3::Y })
                    .normalize_or_zero()
            };
            if let Some(vertices) = groups.get_mut(v.material) {
                vertices.push(PbrVertex {
                    position: (matrix.transform_point3(Vec3::from_array(v.position))
                        - Vec3::from_array(pose.cg_m))
                    .to_array(),
                    normal: normal.to_array(),
                    tangent: [tangent.x, tangent.y, tangent.z, tw * sign],
                    uv: v.uv,
                    color: if luminous { [1.; 4] } else { [0.12, 0.12, 0.12, 1.] },
                    emissive_scale: if luminous { 1. } else { 0. },
                });
            }
        }
    }
    let mut primitives: Vec<PbrPrimitive> = groups
        .into_iter()
        .enumerate()
        .filter(|(_, v)| !v.is_empty())
        .map(|(material, vertices)| PbrPrimitive { material, vertices })
        .collect();
    if let Some(scene) = &pose.walkaround {
        primitives.extend(crate::walkaround::primitives(
            scene,
            pose.cg_m,
            model.materials.len(),
        ));
    }
    primitives
}

/// Exterior GPU implementation is independent of the scenery pipeline. Its public input types
/// are a handoff contract for the bounded pack decoder and the scenery-owned render pass.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    fn models() -> Vec<Arc<ExteriorAsset>> {
        let mut models = vec![builtin("corvid").unwrap(), builtin("larrow").unwrap()];
        if let Ok(dir) = std::env::var("FLIGHTCRAFT_SLING_PACK") {
            let path = std::path::Path::new(&dir);
            let definition: flightcraft_aero::exterior::Exterior =
                serde_json::from_slice(&std::fs::read(path.join("exterior.json")).unwrap()).unwrap();
            let assets = definition
                .paths()
                .into_iter()
                .map(|name| (name.to_owned(), std::fs::read(path.join(name)).unwrap()))
                .collect();
            let mut cache = std::collections::BTreeMap::new();
            let mut load = |name: &str| {
                Arc::new(
                    flightcraft_aero::exterior::load_glb_with_assets(
                        &std::fs::read(path.join(name)).unwrap(),
                        &definition,
                        name,
                        &assets,
                        &mut cache,
                    )
                    .unwrap(),
                )
            };
            let lod0 = load(&definition.model);
            let lod1 = load(&definition.lod1);
            let lod2 = definition.lod2.as_deref().map(load);
            models.push(Arc::new(ExteriorAsset {
                definition,
                lod0,
                lod1,
                lod2,
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
            if let Some(lod2) = &asset.lod2 {
                assert!(lod2.triangles < asset.lod1.triangles);
            }
            for model in [&asset.lod0, &asset.lod1].into_iter().chain(asset.lod2.as_ref()) {
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
                let tolerance = if asset.definition.aircraft == "sling2" {
                    0.01
                } else {
                    0.02
                };
                assert!(
                    (size.y / asset.definition.span_m - 1.).abs() < tolerance,
                    "{} span {}",
                    asset.definition.aircraft,
                    size.y
                );
                assert!(
                    (size.x / asset.definition.length_m - 1.).abs() < tolerance,
                    "{} length {}",
                    asset.definition.aircraft,
                    size.x
                );
                assert!(
                    (size.z / asset.definition.height_m - 1.).abs() < tolerance,
                    "{} height {}",
                    asset.definition.aircraft,
                    size.z
                );
                let mut aileron_sides = [false; 2];
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
                        Motion::Slide => pose.canopy_open_fraction = 1.,
                        Motion::PropBlur => continue,
                    }
                    let changed = transforms(&asset, model, &pose);
                    // Probe a real trailing-edge vertex for ailerons. Identify the side
                    // in neutral body coordinates (y right, z down), independently of
                    // the node name or the binding's chosen rotation axis.
                    let probe = if part.motion == Motion::Aileron {
                        model.nodes[i]
                            .vertices
                            .iter()
                            .map(|v| Vec3::from_array(v.position))
                            .min_by(|a, b| {
                                base[i]
                                    .transform_point3(*a)
                                    .x
                                    .total_cmp(&base[i].transform_point3(*b).x)
                            })
                            .unwrap()
                    } else {
                        Vec3::new(-0.2, 0., 0.)
                    };
                    let before = base[i].transform_point3(probe);
                    let after = changed[i].transform_point3(probe);
                    match part.motion {
                        Motion::Aileron => {
                            assert!(before.y.abs() > 0.1, "aileron must be on a physical side");
                            let right = before.y > 0.;
                            aileron_sides[usize::from(right)] = true;
                            assert!(
                                if right {
                                    after.z < before.z - 0.001
                                } else {
                                    after.z > before.z + 0.001
                                },
                                "{} {} at {} triangles: positive right roll must raise RIGHT / lower LEFT trailing edge; {before:?} -> {after:?}",
                                asset.definition.aircraft,
                                part.node,
                                model.triangles
                            );
                            pose.controls[0] = -1.;
                            let reverse = transforms(&asset, model, &pose)[i].transform_point3(probe);
                            assert!(
                                if right {
                                    reverse.z > before.z + 0.001
                                } else {
                                    reverse.z < before.z - 0.001
                                },
                                "negative roll must reverse the aileron trailing-edge directions"
                            );
                        }
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
                    // Sling's oleo -> steering/wheel -> tire/hub hierarchy can have
                    // multiple empty groups before any visible geometry.
                    let in_group = |index: usize| {
                        let mut parent = Some(index);
                        for _ in 0..model.nodes.len() {
                            let Some(p) = parent else { break };
                            if p == i {
                                return true;
                            }
                            parent = model.nodes[p].parent;
                        }
                        false
                    };
                    assert!(
                        model
                            .nodes
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| in_group(*index))
                            .any(|(index, node)| node.vertices.iter().any(|v| {
                                base[index]
                                    .transform_point3(Vec3::from_array(v.position))
                                    .distance(changed[index].transform_point3(Vec3::from_array(v.position)))
                                    > 0.0001
                            })),
                        "{} {} at {} triangles must move visible geometry",
                        asset.definition.aircraft,
                        part.node,
                        model.triangles
                    );
                }
                assert!(
                    aileron_sides.into_iter().all(|tested| tested),
                    "both ailerons must be tested at every LOD"
                );
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

/// Texture/geometry limits are checked before any GPU allocation. The pack decoder must also
/// bound encoded bytes and decompression before constructing these RGBA inputs.
pub const MAX_PBR_TEXTURE_DIMENSION: u32 = 4096;
pub const MAX_PBR_TEXTURE_BYTES: usize = 5 * 4096 * 4096 * 4;
pub const MAX_PBR_TRIANGLES: usize = 120_000;
pub const MAX_PBR_MATERIALS: usize = 32;
pub const MAX_PBR_IMAGES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

/// Already decoded, tightly packed RGBA8. Only base color and emissive are sRGB.
#[derive(Clone, Debug)]
pub struct PbrImage {
    pub size: [u32; 2],
    pub rgba: Arc<[u8]>,
}
impl PbrImage {
    pub fn validate(&self) -> Result<(), String> {
        let [w, h] = self.size;
        if w == 0 || h == 0 || w > MAX_PBR_TEXTURE_DIMENSION || h > MAX_PBR_TEXTURE_DIMENSION {
            return Err("exterior texture dimensions exceed the supported limit".into());
        }
        let bytes = u64::from(w) * u64::from(h) * 4;
        if bytes != self.rgba.len() as u64 {
            return Err("exterior texture storage does not match its dimensions".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct PbrMaterial {
    pub base_color: [f32; 4],
    pub emissive: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub normal_scale: f32,
    pub occlusion_strength: f32,
    pub alpha_cutoff: f32,
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    /// Base color, metallic-roughness (G/B), tangent normal, occlusion (R), emissive.
    /// Missing maps use neutral 1x1 textures. No filename, URI, or decoder enters the renderer.
    pub images: [Option<PbrImage>; 5],
}
impl Default for PbrMaterial {
    fn default() -> Self {
        Self {
            base_color: [1.; 4],
            emissive: [0.; 3],
            metallic: 0.,
            roughness: 0.5,
            normal_scale: 1.,
            occlusion_strength: 1.,
            alpha_cutoff: 0.5,
            alpha_mode: AlphaMode::Opaque,
            double_sided: false,
            clearcoat: 0.,
            clearcoat_roughness: 0.1,
            images: std::array::from_fn(|_| None),
        }
    }
}
impl PbrMaterial {
    pub fn validate(&self) -> Result<(), String> {
        if !self
            .base_color
            .iter()
            .all(|x| x.is_finite() && (0.0..=1.).contains(x))
            || !self
                .emissive
                .iter()
                .all(|x| x.is_finite() && (0.0..=100.).contains(x))
            || ![
                self.metallic,
                self.roughness,
                self.occlusion_strength,
                self.alpha_cutoff,
                self.clearcoat,
                self.clearcoat_roughness,
            ]
            .iter()
            .all(|x| x.is_finite() && (0.0..=1.).contains(x))
            || !self.normal_scale.is_finite()
            || !(0.0..=4.).contains(&self.normal_scale)
        {
            return Err("invalid exterior PBR material factors".into());
        }
        let mut bytes = 0usize;
        for image in self.images.iter().flatten() {
            image.validate()?;
            bytes = bytes.saturating_add(image.rgba.len());
        }
        if bytes > MAX_PBR_TEXTURE_BYTES {
            return Err("exterior texture byte budget exceeded".into());
        }
        Ok(())
    }
}

/// Vertices are expanded triangles, already posed into CG-relative body coordinates.
/// Node normals use inverse transpose; tangents use the forward matrix, orthogonalized to the
/// normal, and tangent.w changes sign for a reflected transform. UVs are glTF TEXCOORD_0.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PbrVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// Animation gates lamps without rebuilding their material textures; ordinary surfaces use 1.
    pub emissive_scale: f32,
}
#[derive(Clone, Debug)]
pub struct PbrPrimitive {
    pub material: usize,
    pub vertices: Vec<PbrVertex>,
}

/// Per-view exterior lighting, in renderer world coordinates (Y up). All colors are linear
/// radiance. Scenery may set direct_visibility for its sun/shadow visibility at the aircraft.
/// A prefiltered sky probe is bound separately; no probe uses the procedural hemisphere below.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PbrFrame {
    pub view_projection: [[f32; 4]; 4],
    pub world_from_body: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub sun_direction: [f32; 4],
    pub sun_radiance: [f32; 4],
    pub sky_zenith: [f32; 4],
    pub sky_horizon: [f32; 4],
    pub ground_radiance: [f32; 4],
    /// Set by `prepare`: probe enabled, maximum mip, exposure, ambient strength.
    pub environment: [f32; 4],
}
impl Default for PbrFrame {
    fn default() -> Self {
        Self {
            view_projection: Mat4::IDENTITY.to_cols_array_2d(),
            world_from_body: Mat4::IDENTITY.to_cols_array_2d(),
            eye: [0., 0., 10., 1.],
            sun_direction: [0.35, 0.8, 0.4, 1.],
            sun_radiance: [3.0, 2.8, 2.5, 0.],
            sky_zenith: [0.15, 0.30, 0.55, 0.],
            sky_horizon: [0.55, 0.65, 0.75, 0.],
            ground_radiance: [0.12, 0.10, 0.08, 0.],
            environment: [0., 0., 1., 1.],
        }
    }
}

/// Scenery supplies a filterable cubemap of linear radiance, with GGX-prefiltered mip levels
/// from sharp (level zero) to rough (last level). The caller keeps the texture alive. Maximum
/// 13 levels, and no texture arrays, float filtering extension, or compute support are required.
pub struct SkyProbe<'a> {
    pub view: &'a wgpu::TextureView,
    pub mip_count: u32,
}

/// Scenery-owned lighting for scene viewports. Camera matrices are supplied by the viewport;
/// the lighting fields in `frame` are used verbatim. With no probe, procedural sky is reflected.
#[derive(Clone, Default)]
pub struct ExteriorEnvironment {
    pub frame: PbrFrame,
    pub probe: Option<Arc<EnvironmentProbe>>,
}
pub struct EnvironmentProbe {
    pub view: wgpu::TextureView,
    pub mip_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    emissive_normal: [f32; 4],
    surface: [f32; 4],
    finish: [f32; 4],
}

struct PbrGpuMaterial {
    bind: wgpu::BindGroup,
    alpha_mode: AlphaMode,
}
struct PbrDraw {
    range: std::ops::Range<u32>,
    material: usize,
}

/// One instance per scene viewport. Upload materials only on an asset change; prepare posed
/// triangles each frame; draw into the scenery's color/depth pass after opaque terrain. The
/// pipeline uses straight-alpha blending, no transparent depth writes, and back-to-front
/// triangle sorting in view space. Convex glass renders both surfaces in their correct order.
pub struct PbrRenderer {
    scene_lighting: crate::lighting::GpuLighting,
    scene_bind: Option<wgpu::BindGroup>,
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    material_layout: wgpu::BindGroupLayout,
    probe_layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    frame_bind: wgpu::BindGroup,
    probe_bind: wgpu::BindGroup,
    probe_sampler: wgpu::Sampler,
    fallback_probe: wgpu::TextureView,
    vertices: wgpu::Buffer,
    image_cache: SharedPbrImages,
    materials: Vec<PbrGpuMaterial>,
    draws: Vec<PbrDraw>,
}
impl Drop for PbrRenderer {
    fn drop(&mut self) {
        // Dropping a wgpu resource on the web waits for the browser's garbage collector.
        self.uniform.destroy();
        self.vertices.destroy();
        self.fallback_probe.texture().destroy();
    }
}
/// Uploaded exterior texture maps (image, sRGB, normal map, upload size limit, view). Every view of
/// the aircraft (the main outside view and the exterior inset) shares one set, so a second
/// view costs a few kilobytes of bind groups rather than another few hundred megabytes of mip
/// chains. Uses a plain `Arc<Mutex>`: all access is from the render thread, briefly.
pub type SharedPbrImages = Arc<std::sync::Mutex<Vec<(PbrImage, bool, bool, u32, wgpu::TextureView)>>>;

/// Source bytes of cached maps above which the shared set is dropped and rebuilt on demand. Large
/// enough for the five 4096 maps of the largest allowed aircraft plus its lower detail levels.
pub const PBR_IMAGE_CACHE_BYTES: usize = 3 * MAX_PBR_TEXTURE_BYTES / 2;

/// How many cached texture maps the shared set holds; for the guard test and diagnostics.
#[must_use]
pub fn shared_image_count(images: &SharedPbrImages) -> usize {
    images.lock().map_or(0, |cache| cache.len())
}

impl PbrRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        Self::with_shared_images(device, SharedPbrImages::default())
    }
    /// A renderer that uploads its texture maps into, and reuses them from, `images`.
    pub fn with_shared_images(device: &wgpu::Device, images: SharedPbrImages) -> Self {
        Self::with_samples(device, images, 1)
    }
    /// A renderer whose pipelines draw into a target with `samples` samples per pixel.
    pub fn with_samples(device: &wgpu::Device, images: SharedPbrImages, samples: u32) -> Self {
        use wgpu::util::DeviceExt;
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("exterior PBR frame"),
            contents: bytemuck::bytes_of(&PbrFrame::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let buffer_entry = |size| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("exterior frame layout"),
            entries: &[buffer_entry(std::mem::size_of::<PbrFrame>() as u64)],
        });
        let frame_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exterior frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let texture_entry = |binding, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let mut entries = vec![buffer_entry(std::mem::size_of::<MaterialUniform>() as u64)];
        entries.extend((1..=5).map(|binding| texture_entry(binding, wgpu::TextureViewDimension::D2)));
        entries.push(sampler_entry(6));
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("exterior material layout"),
            entries: &entries,
        });
        let probe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("exterior sky probe layout"),
            entries: &[
                texture_entry(0, wgpu::TextureViewDimension::Cube),
                sampler_entry(1),
            ],
        });
        let probe_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("exterior environment sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // The shader does not sample this zero-initialized placeholder in procedural mode.
        let fallback_probe = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("exterior fallback sky binding"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 6,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            });
        let probe_bind = Self::bind_probe(device, &probe_layout, &probe_sampler, &fallback_probe);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("exterior PBR WGSL"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("lighting.wgsl"),
                    include_str!("exterior.wgsl")
                )
                .into(),
            ),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("exterior PBR pipeline layout"),
            bind_group_layouts: &[
                &frame_layout,
                &material_layout,
                &probe_layout,
                &crate::lighting::layout(device),
            ],
            push_constant_ranges: &[],
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x4,3=>Float32x2,4=>Float32x4,5=>Float32];
        let pipeline = |transparent| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("exterior PBR"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<PbrVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: if transparent {
                            Some(wgpu::BlendState::ALPHA_BLENDING)
                        } else {
                            None
                        },
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24Plus,
                    depth_write_enabled: !transparent,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview: None,
                cache: None,
            })
        };
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("posed exterior PBR vertices"),
            size: (MAX_PBR_TRIANGLES * 3 * std::mem::size_of::<PbrVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            scene_lighting: crate::lighting::GpuLighting::new(device, crate::scenery::Quality::Low),
            scene_bind: None,
            opaque: pipeline(false),
            transparent: pipeline(true),
            material_layout,
            probe_layout,
            uniform,
            frame_bind,
            probe_bind,
            probe_sampler,
            fallback_probe,
            vertices,
            image_cache: images,
            materials: Vec::new(),
            draws: Vec::new(),
        }
    }
    fn bind_probe(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exterior sky probe"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }
    /// Validate the complete material set before replacing any existing GPU resources.
    pub fn set_materials(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        materials: &[PbrMaterial],
    ) -> Result<(), String> {
        self.set_materials_with_limit(device, queue, materials, device.limits().max_texture_dimension_2d)
    }
    /// Distant LODs start at smaller, correctly filtered levels of the shared atlas.
    pub fn set_materials_with_limit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        materials: &[PbrMaterial],
        max_dimension: u32,
    ) -> Result<(), String> {
        use wgpu::util::DeviceExt;
        let max_dimension = max_dimension.clamp(1, device.limits().max_texture_dimension_2d);
        if materials.is_empty() || materials.len() > MAX_PBR_MATERIALS {
            return Err("exterior material count exceeds the supported limit".into());
        }
        let mut total_bytes = 0usize;
        let mut unique_images: Vec<&PbrImage> = Vec::new();
        for material in materials {
            material.validate()?;
            for image in material.images.iter().flatten() {
                if !unique_images.iter().any(|seen| {
                    seen.size == image.size
                        && (Arc::ptr_eq(&seen.rgba, &image.rgba) || seen.rgba == image.rgba)
                }) {
                    total_bytes = total_bytes.saturating_add(image.rgba.len());
                    unique_images.push(image);
                }
            }
        }
        if total_bytes > MAX_PBR_TEXTURE_BYTES || unique_images.len() > MAX_PBR_IMAGES {
            return Err("exterior texture byte budget exceeded".into());
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("exterior material sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let defaults = [[255; 4], [255; 4], [128, 128, 255, 255], [255; 4], [255; 4]];
        let mut gpu_materials = Vec::with_capacity(materials.len());
        let mut cached_views = self
            .image_cache
            .lock()
            .map_err(|_| "exterior texture cache is poisoned".to_string())?;
        // Preserve immutable texture mip chains across aircraft LOD changes and between views,
        // bounded in total.
        if cached_views
            .iter()
            .map(|(image, _, _, _, _)| image.rgba.len())
            .sum::<usize>()
            > PBR_IMAGE_CACHE_BYTES
        {
            cached_views.clear();
        }
        for material in materials {
            let [er, eg, eb] = material.emissive;
            let uniform = MaterialUniform {
                base_color: material.base_color,
                emissive_normal: [er, eg, eb, material.normal_scale],
                surface: [
                    material.metallic,
                    material.roughness,
                    material.occlusion_strength,
                    material.alpha_cutoff,
                ],
                finish: [
                    match material.alpha_mode {
                        AlphaMode::Opaque => 0.,
                        AlphaMode::Mask => 1.,
                        AlphaMode::Blend => 2.,
                    },
                    if material.double_sided { 1. } else { 0. },
                    material.clearcoat,
                    material.clearcoat_roughness,
                ],
            };
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("exterior PBR material"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let views: Vec<_> = material
                .images
                .iter()
                .zip(defaults)
                .enumerate()
                .map(|(i, (image, neutral))| {
                    let (size, rgba) = image
                        .as_ref()
                        .map_or(([1, 1], neutral.as_slice()), |x| (x.size, x.rgba.as_ref()));
                    let srgb = i == 0 || i == 4;
                    if let Some(image) = image
                        && let Some((_, _, _, _, view)) =
                            cached_views.iter().find(|(seen, space, normal, limit, _)| {
                                *space == srgb
                                    && *normal == (i == 2)
                                    && *limit == max_dimension
                                    && seen.size == image.size
                                    && (Arc::ptr_eq(&seen.rgba, &image.rgba) || seen.rgba == image.rgba)
                            })
                    {
                        return view.clone();
                    }
                    // WebGL2's guaranteed limit can be 2048. Keep source limits at 4096,
                    // but upload a correctly filtered mip when a device needs smaller maps.
                    let mut upload_size = size;
                    let mut upload_rgba = rgba.to_vec();
                    while upload_size.iter().any(|n| *n > max_dimension) {
                        (upload_size, upload_rgba) = downsample_pbr(upload_size, &upload_rgba, srgb, i == 2);
                    }
                    let [width, height] = upload_size;
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("bounded exterior PBR map"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: width.max(height).ilog2() + 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: if i == 0 || i == 4 {
                            wgpu::TextureFormat::Rgba8UnormSrgb
                        } else {
                            wgpu::TextureFormat::Rgba8Unorm
                        },
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    let mut level = 0;
                    let mut mip_size = upload_size;
                    let mut mip_rgba = upload_rgba;
                    loop {
                        let [mw, mh] = mip_size;
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &texture,
                                mip_level: level,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            &mip_rgba,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(mw * 4),
                                rows_per_image: Some(mh),
                            },
                            wgpu::Extent3d {
                                width: mw,
                                height: mh,
                                depth_or_array_layers: 1,
                            },
                        );
                        if mw == 1 && mh == 1 {
                            break;
                        }
                        (mip_size, mip_rgba) = downsample_pbr(mip_size, &mip_rgba, srgb, i == 2);
                        level += 1;
                    }
                    let view = texture.create_view(&Default::default());
                    if let Some(image) = image {
                        cached_views.push((image.clone(), srgb, i == 2, max_dimension, view.clone()));
                    }
                    view
                })
                .collect();
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }];
            entries.extend(views.iter().enumerate().map(|(i, view)| wgpu::BindGroupEntry {
                binding: i as u32 + 1,
                resource: wgpu::BindingResource::TextureView(view),
            }));
            entries.push(wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(&sampler),
            });
            gpu_materials.push(PbrGpuMaterial {
                bind: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("exterior PBR material"),
                    layout: &self.material_layout,
                    entries: &entries,
                }),
                alpha_mode: material.alpha_mode,
            });
        }
        drop(cached_views);
        self.materials = gpu_materials;
        self.draws.clear();
        Ok(())
    }

    /// Validation and sorting finish before this view's state is changed. Body-to-world must be
    /// rigid (reflection is allowed); node transforms have already been applied by the animator.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: PbrFrame,
        primitives: &[PbrPrimitive],
        probe: Option<SkyProbe<'_>>,
    ) -> Result<(), String> {
        let prepared = prepare_pbr(
            &frame,
            primitives,
            &self.materials.iter().map(|m| m.alpha_mode).collect::<Vec<_>>(),
        )?;
        self.update_frame(device, queue, frame, probe)?;
        if !prepared.0.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&prepared.0));
        }
        self.draws = prepared.1;
        Ok(())
    }
    /// Reuse unchanged body geometry while still validating and updating the camera and lighting.
    pub(crate) fn update_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut frame: PbrFrame,
        probe: Option<SkyProbe<'_>>,
    ) -> Result<(), String> {
        prepare_pbr(&frame, &[], &[])?;
        if probe.as_ref().is_some_and(|p| !(1..=13).contains(&p.mip_count)) {
            return Err("invalid exterior sky probe mip count".into());
        }
        let [_, _, exposure, ambient] = frame.environment;
        frame.environment = [
            if probe.is_some() { 1. } else { 0. },
            probe
                .as_ref()
                .map_or(0., |p| p.mip_count.saturating_sub(1) as f32),
            exposure,
            ambient,
        ];
        let probe_view = probe.map_or(&self.fallback_probe, |p| p.view);
        self.probe_bind = Self::bind_probe(device, &self.probe_layout, &self.probe_sampler, probe_view);
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&frame));
        Ok(())
    }
    pub(crate) fn set_scene_lighting(&mut self, bind: &wgpu::BindGroup) {
        self.scene_bind = Some(bind.clone());
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_opaque(pass);
        self.draw_transparent(pass);
    }
    pub fn draw_opaque(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_group(pass, false);
    }
    pub fn draw_transparent(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_group(pass, true);
    }
    fn draw_group(&self, pass: &mut wgpu::RenderPass<'_>, transparent: bool) {
        pass.set_bind_group(
            3,
            self.scene_bind.as_ref().unwrap_or(&self.scene_lighting.bind),
            &[],
        );
        pass.set_bind_group(0, &self.frame_bind, &[]);
        pass.set_bind_group(2, &self.probe_bind, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for draw in &self.draws {
            if let Some(material) = self.materials.get(draw.material) {
                if (material.alpha_mode == AlphaMode::Blend) != transparent {
                    continue;
                }
                pass.set_pipeline(if material.alpha_mode == AlphaMode::Blend {
                    &self.transparent
                } else {
                    &self.opaque
                });
                pass.set_bind_group(1, &material.bind, &[]);
                pass.draw(draw.range.clone(), 0..1);
            }
        }
    }
}

fn prepare_pbr(
    frame: &PbrFrame,
    primitives: &[PbrPrimitive],
    materials: &[AlphaMode],
) -> Result<(Vec<PbrVertex>, Vec<PbrDraw>), String> {
    if bytemuck::cast_slice::<PbrFrame, f32>(std::slice::from_ref(frame))
        .iter()
        .any(|x| !x.is_finite())
    {
        return Err("non-finite exterior frame".into());
    }
    let world = Mat4::from_cols_array_2d(&frame.world_from_body);
    let rotation = Mat3::from_mat4(world);
    let gram = rotation.transpose() * rotation;
    let [sx, sy, sz, direct_visibility] = frame.sun_direction;
    let [_, _, exposure, ambient] = frame.environment;
    if (gram - Mat3::IDENTITY)
        .to_cols_array()
        .iter()
        .any(|x| x.abs() > 0.001)
        || (world.w_axis.w - 1.).abs() > 0.001
        || world.x_axis.w.abs() + world.y_axis.w.abs() + world.z_axis.w.abs() > 0.001
        || Vec3::new(sx, sy, sz).length_squared() < 0.01
        || !(0.0..=1.).contains(&direct_visibility)
        || !(0.0..=32.).contains(&exposure)
        || !(0.0..=16.).contains(&ambient)
    {
        return Err("invalid exterior frame transform or lighting".into());
    }
    if primitives.len() > 4096
        || frame
            .sun_radiance
            .iter()
            .chain(&frame.sky_zenith)
            .chain(&frame.sky_horizon)
            .chain(&frame.ground_radiance)
            .any(|x| !(0.0..=10000.).contains(x))
    {
        return Err("exterior primitive count or radiance exceeds its limit".into());
    }
    let mut count = 0usize;
    for primitive in primitives {
        count = count.saturating_add(primitive.vertices.len());
        if count > MAX_PBR_TRIANGLES * 3
            || !primitive.vertices.len().is_multiple_of(3)
            || primitive.material >= materials.len()
        {
            return Err("exterior geometry budget or material binding is invalid".into());
        }
        for v in &primitive.vertices {
            let [tx, ty, tz, handedness] = v.tangent;
            if bytemuck::cast_slice::<PbrVertex, f32>(std::slice::from_ref(v))
                .iter()
                .any(|x| !x.is_finite())
                || v.position.iter().any(|x| x.abs() > 200.)
                || v.uv.iter().any(|x| x.abs() > 65536.)
                || v.color.iter().any(|x| !(0.0..=1.).contains(x))
                || !(0.0..=1.).contains(&v.emissive_scale)
                || !(0.99..=1.01).contains(&Vec3::from_array(v.normal).length_squared())
                || !(0.99..=1.01).contains(&Vec3::new(tx, ty, tz).length_squared())
                || (handedness.abs() - 1.).abs() > 0.001
                || Vec3::from_array(v.normal).dot(Vec3::new(tx, ty, tz)).abs() > 0.01
            {
                return Err("invalid exterior vertex, normal, or tangent".into());
            }
        }
    }
    let [ex, ey, ez, _] = frame.eye;
    let eye = Vec3::new(ex, ey, ez);
    // Clip z increases with forward depth for both standard perspective and orthographic
    // projections. Clip w is constant for the orthographic profile/plan proof cameras.
    let clip = Mat4::from_cols_array_2d(&frame.view_projection) * world;
    let mut opaque = Vec::new();
    let mut transparent = Vec::new();
    for primitive in primitives {
        for triangle in primitive.vertices.as_chunks::<3>().0 {
            let [a, b, c] = triangle;
            let center =
                (Vec3::from_array(a.position) + Vec3::from_array(b.position) + Vec3::from_array(c.position))
                    / 3.;
            let depth = (clip * center.extend(1.)).z;
            let distance = world.transform_point3(center).distance_squared(eye);
            let item = (primitive.material, [*a, *b, *c], depth, distance);
            if materials.get(primitive.material) == Some(&AlphaMode::Blend) {
                transparent.push(item);
            } else {
                opaque.push(item);
            }
        }
    }
    transparent.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| b.3.total_cmp(&a.3)));
    let mut vertices = Vec::with_capacity(count);
    let mut draws: Vec<PbrDraw> = Vec::new();
    for (material, triangle, _, _) in opaque.into_iter().chain(transparent) {
        let start = vertices.len() as u32;
        vertices.extend(triangle);
        if let Some(last) = draws.last_mut().filter(|last| last.material == material) {
            last.range.end += 3;
        } else {
            draws.push(PbrDraw {
                range: start..start + 3,
                material,
            });
        }
    }
    Ok((vertices, draws))
}

// Filter color in linear space and renormalize tangent normals at every mip. This avoids
// gamma-dark seams and shrinking normal vectors without requiring compute or render-to-float.
fn downsample_pbr(size: [u32; 2], rgba: &[u8], srgb: bool, normal: bool) -> ([u32; 2], Vec<u8>) {
    let [width, height] = size;
    let next = [(width / 2).max(1), (height / 2).max(1)];
    let [nw, nh] = next;
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        for x in 0..nw {
            let mut total = [0_f32; 4];
            let mut count = 0_f32;
            for sy in y * height / nh..(y + 1) * height / nh {
                for sx in x * width / nw..(x + 1) * width / nw {
                    let offset = ((sy * width + sx) * 4) as usize;
                    let Some(pixel) = rgba.get(offset..offset + 4) else {
                        continue;
                    };
                    for (index, (sum, byte)) in total.iter_mut().zip(pixel).enumerate() {
                        let value = f32::from(*byte) / 255.;
                        *sum += if srgb && index < 3 {
                            srgb_linear(value)
                        } else {
                            value
                        };
                    }
                    count += 1.;
                }
            }
            let [r, g, b, a] = total.map(|v| v / count.max(1.));
            let rgb = if normal {
                ((Vec3::new(r, g, b) * 2. - Vec3::ONE)
                    .try_normalize()
                    .unwrap_or(Vec3::Z)
                    + Vec3::ONE)
                    * 0.5
            } else {
                Vec3::new(r, g, b)
            };
            for value in rgb.to_array() {
                let value = if srgb { linear_srgb(value) } else { value };
                out.push((value.clamp(0., 1.) * 255.).round() as u8);
            }
            out.push((a.clamp(0., 1.) * 255.).round() as u8);
        }
    }
    (next, out)
}
fn srgb_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}
fn linear_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod pbr_tests {
    use super::*;
    #[test]
    fn pbr_shader_validates_translates_to_webgl2_and_matches_host_layout()
    -> Result<(), Box<dyn std::error::Error>> {
        use naga::{back::glsl, valid};
        let module = naga::front::wgsl::parse_str(&format!(
            "{}\n{}",
            include_str!("lighting.wgsl"),
            include_str!("exterior.wgsl")
        ))?;
        let info = valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::empty())
            .validate(&module)?;
        for entry in &module.entry_points {
            let options = glsl::Options {
                version: glsl::Version::Embedded {
                    version: 300,
                    is_webgl: true,
                },
                ..Default::default()
            };
            let pipeline = glsl::PipelineOptions {
                shader_stage: entry.stage,
                entry_point: entry.name.clone(),
                multiview: None,
            };
            let mut output = String::new();
            glsl::Writer::new(
                &mut output,
                &module,
                &info,
                &options,
                &pipeline,
                Default::default(),
            )?
            .write()?;
            assert!(output.starts_with("#version 300 es"));
        }
        for (name, size) in [
            ("Frame", std::mem::size_of::<PbrFrame>()),
            ("Material", std::mem::size_of::<MaterialUniform>()),
        ] {
            let span = module
                .types
                .iter()
                .find_map(|(_, ty)| match &ty.inner {
                    naga::TypeInner::Struct { span, .. } if ty.name.as_deref() == Some(name) => Some(*span),
                    _ => None,
                })
                .unwrap();
            assert_eq!(span as usize, size);
        }
        Ok(())
    }
    fn triangle(z: f32, material: usize) -> PbrPrimitive {
        PbrPrimitive {
            material,
            vertices: [[-0.9, -0.9, z], [0.9, -0.9, z], [0., 0.9, z]]
                .map(|position| PbrVertex {
                    position,
                    normal: [0., 0., 1.],
                    tangent: [1., 0., 0., 1.],
                    uv: [0.5, 0.5],
                    color: [1.; 4],
                    emissive_scale: 1.,
                })
                .to_vec(),
        }
    }
    #[test]
    fn pbr_rejects_bad_storage_factors_geometry_and_transforms() {
        for size in [[0, 1], [4097, 1], [u32::MAX, u32::MAX], [4096, 4096]] {
            assert!(
                PbrImage {
                    size,
                    rgba: Arc::from([0u8; 4])
                }
                .validate()
                .is_err()
            );
        }
        let material = PbrMaterial {
            roughness: f32::NAN,
            ..Default::default()
        };
        assert!(material.validate().is_err());
        let mut primitive = triangle(0., 0);
        primitive.vertices[0].uv[0] = f32::INFINITY;
        assert!(prepare_pbr(&PbrFrame::default(), &[primitive], &[AlphaMode::Opaque]).is_err());
        assert!(prepare_pbr(&PbrFrame::default(), &[triangle(0., 1)], &[AlphaMode::Opaque]).is_err());
        let mut primitive = triangle(0., 0);
        primitive.vertices.pop();
        assert!(prepare_pbr(&PbrFrame::default(), &[primitive], &[AlphaMode::Opaque]).is_err());
        let frame = PbrFrame {
            world_from_body: Mat4::from_scale(Vec3::new(1., 2., 1.)).to_cols_array_2d(),
            ..Default::default()
        };
        assert!(prepare_pbr(&frame, &[triangle(0., 0)], &[AlphaMode::Opaque]).is_err());
        let mut primitive = triangle(0., 0);
        primitive.vertices[0].tangent = [0., 0., 1., 1.];
        assert!(prepare_pbr(&PbrFrame::default(), &[primitive], &[AlphaMode::Opaque]).is_err());
    }
    #[test]
    fn mip_filter_uses_linear_color_and_unit_normals() {
        let (size, color) = downsample_pbr([2, 1], &[0, 0, 0, 255, 255, 255, 255, 255], true, false);
        assert_eq!(size, [1, 1]);
        assert!(
            (186..=189).contains(&color[0]),
            "sRGB average must be linear-light middle gray"
        );
        let (_, normal) = downsample_pbr([2, 1], &[255, 128, 128, 255, 128, 128, 255, 255], false, true);
        let vector = Vec3::new(normal[0] as f32, normal[1] as f32, normal[2] as f32) / 127.5 - Vec3::ONE;
        assert!((vector.length() - 1.).abs() < 0.01);
        let (size, odd) = downsample_pbr(
            [3, 1],
            &[0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255],
            false,
            false,
        );
        assert_eq!(size, [1, 1]);
        assert_eq!(odd, [85, 85, 85, 255]);
    }
    #[test]
    fn transparent_triangles_sort_along_view_after_all_opaque_triangles() {
        let frame = PbrFrame {
            view_projection: (Mat4::perspective_rh(1., 1., 0.1, 100.)
                * Mat4::look_at_rh(Vec3::new(0., 0., 5.), Vec3::ZERO, Vec3::Y))
            .to_cols_array_2d(),
            ..Default::default()
        };
        let (vertices, draws) = prepare_pbr(
            &frame,
            &[triangle(2., 1), triangle(1., 0), triangle(-1., 1)],
            &[AlphaMode::Opaque, AlphaMode::Blend],
        )
        .unwrap();
        assert_eq!(vertices[0].position[2], 1.);
        assert_eq!(vertices[3].position[2], -1.);
        assert_eq!(vertices[6].position[2], 2.);
        assert_eq!(draws.len(), 2);
        assert_eq!(draws[1].range, 3..9);
        let mut reversed = frame;
        reversed.view_projection = (Mat4::perspective_rh(1., 1., 0.1, 100.)
            * Mat4::look_at_rh(Vec3::new(0., 0., -5.), Vec3::ZERO, Vec3::Y))
        .to_cols_array_2d();
        let (vertices, _) = prepare_pbr(
            &reversed,
            &[triangle(-1., 1), triangle(2., 1)],
            &[AlphaMode::Opaque, AlphaMode::Blend],
        )
        .unwrap();
        assert_eq!(vertices[0].position[2], 2.);
    }
    #[test]
    fn transparent_orthographic_triangles_sort_by_view_depth() {
        let frame = PbrFrame {
            view_projection: (Mat4::orthographic_rh(-2., 2., -2., 2., 0.1, 100.)
                * Mat4::look_at_rh(Vec3::new(0., 0., 5.), Vec3::ZERO, Vec3::Y))
            .to_cols_array_2d(),
            // Deliberately misleading radial distances: sorting must use view depth.
            eye: [0., 0., 0., 1.],
            ..Default::default()
        };
        let (vertices, _) =
            prepare_pbr(&frame, &[triangle(2., 0), triangle(-1., 0)], &[AlphaMode::Blend]).unwrap();
        assert_eq!(vertices[0].position[2], -1.);
        assert_eq!(vertices[3].position[2], 2.);
    }
    /// A second view of the same aircraft (the exterior inset) must not hold a second copy of the
    /// texture mip chains: that doubled GPU memory and made the WebGPU smoke run out of it.
    #[test]
    fn shared_image_budget_holds_the_largest_aircraft_without_eviction() {
        // A budget smaller than one aircraft's maps would make every view clear the shared set
        // and re-upload everything, which is the duplication again.
        const { assert!(PBR_IMAGE_CACHE_BYTES >= MAX_PBR_TEXTURE_BYTES) };
    }
    #[test]
    #[ignore = "requires an offscreen GPU"]
    fn two_views_sharing_images_upload_each_map_once() {
        block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let adapter = instance.request_adapter(&Default::default()).await.unwrap();
            let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
            let shared = SharedPbrImages::default();
            let mut main = PbrRenderer::with_shared_images(&device, shared.clone());
            let mut inset = PbrRenderer::with_shared_images(&device, shared.clone());
            let image = PbrImage {
                size: [8, 8],
                rgba: vec![200u8; 8 * 8 * 4].into(),
            };
            let material = PbrMaterial {
                images: [Some(image), None, None, None, None],
                ..Default::default()
            };
            main.set_materials_with_limit(&device, &queue, std::slice::from_ref(&material), 4096)
                .unwrap();
            let after_main = shared_image_count(&shared);
            inset
                .set_materials_with_limit(&device, &queue, std::slice::from_ref(&material), 4096)
                .unwrap();
            assert_eq!(after_main, 1);
            assert_eq!(shared_image_count(&shared), after_main);
        });
    }
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        struct Wake(std::thread::Thread);
        impl std::task::Wake for Wake {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(Arc::new(Wake(std::thread::current())));
        let mut context = std::task::Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(value) => return value,
                std::task::Poll::Pending => std::thread::park(),
            }
        }
    }
    #[test]
    #[ignore = "requires an offscreen GPU; exercises the actual exterior shader with WebGL2 limits"]
    fn pbr_offscreen_material_maps_probe_and_transparency() {
        block_on(async {
            let backend = if std::env::var("FLIGHTCRAFT_TEST_BACKEND").as_deref() == Ok("gl") {
                wgpu::Backends::GL
            } else {
                wgpu::Backends::VULKAN
            };
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: backend,
                ..Default::default()
            });
            let adapter = instance.request_adapter(&Default::default()).await.unwrap();
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                    ..Default::default()
                })
                .await
                .unwrap();
            device.push_error_scope(wgpu::ErrorFilter::Validation);
            let mut renderer = PbrRenderer::new(&device);
            let texture = |format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("PBR proof"),
                    size: wgpu::Extent3d {
                        width: 64,
                        height: 64,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let color = texture(
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let color_view = color.create_view(&Default::default());
            let depth = texture(
                wgpu::TextureFormat::Depth24Plus,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            let depth_view = depth.create_view(&Default::default());
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("PBR readback"),
                size: 64 * 64 * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let frame = PbrFrame {
                eye: [0., 0., 3., 1.],
                view_projection: (Mat4::perspective_rh(1., 1., 0.1, 100.)
                    * Mat4::look_at_rh(Vec3::new(0., 0., 3.), Vec3::ZERO, Vec3::Y))
                .to_cols_array_2d(),
                sun_direction: [0.4, 0.6, 0.8, 1.],
                ..Default::default()
            };
            let mut render = |materials: &[PbrMaterial],
                              primitives: &[PbrPrimitive],
                              frame: PbrFrame,
                              probe: Option<SkyProbe<'_>>| {
                renderer.set_materials(&device, &queue, materials).unwrap();
                renderer
                    .prepare(&device, &queue, frame, primitives, probe)
                    .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &color_view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.),
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    renderer.draw(&mut pass);
                }
                encoder.copy_texture_to_buffer(
                    color.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(64),
                        },
                    },
                    wgpu::Extent3d {
                        width: 64,
                        height: 64,
                        depth_or_array_layers: 1,
                    },
                );
                queue.submit([encoder.finish()]);
                let (tx, rx) = std::sync::mpsc::channel();
                readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                rx.recv().unwrap().unwrap();
                let data = readback.slice(..).get_mapped_range().to_vec();
                readback.unmap();
                data
            };
            let solid = PbrMaterial {
                base_color: [0.7, 0.5, 0.3, 1.],
                clearcoat: 0.5,
                ..Default::default()
            };
            let baseline = render(std::slice::from_ref(&solid), &[triangle(0., 0)], frame, None);
            let pixel = |bytes: &[u8]| bytes[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3].to_vec();
            assert!(pixel(&baseline).iter().any(|x| *x > 60));
            // Each material map must affect actual rendered pixels, including correct channels.
            let samples = [
                (0, [40, 180, 60, 255]),
                (1, [255, 8, 255, 255]),
                (2, [240, 128, 160, 255]),
                (3, [0, 0, 0, 255]),
                (4, [0, 0, 255, 255]),
            ];
            for (index, rgba) in samples {
                let mut mapped = solid.clone();
                mapped.images[index] = Some(PbrImage {
                    size: [1, 1],
                    rgba: Arc::from(rgba),
                });
                if index == 1 {
                    mapped.metallic = 1.;
                }
                if index == 4 {
                    mapped.emissive = [1.; 3];
                }
                let changed = render(&[mapped], &[triangle(0., 0)], frame, None);
                assert_ne!(
                    pixel(&baseline),
                    pixel(&changed),
                    "map {index} must affect pixels"
                );
            }
            let red = PbrMaterial {
                base_color: [0.6, 0.05, 0.02, 0.22],
                alpha_mode: AlphaMode::Blend,
                double_sided: true,
                ..Default::default()
            };
            let blue = PbrMaterial {
                base_color: [0.02, 0.05, 0.6, 0.22],
                alpha_mode: AlphaMode::Blend,
                double_sided: true,
                ..Default::default()
            };
            let ordered = render(
                &[solid.clone(), red.clone(), blue.clone()],
                &[triangle(0., 0), triangle(0.3, 1), triangle(0.6, 2)],
                frame,
                None,
            );
            let shuffled = render(
                &[solid, red, blue],
                &[triangle(0.6, 2), triangle(0., 0), triangle(0.3, 1)],
                frame,
                None,
            );
            assert_eq!(
                ordered, shuffled,
                "transparent sort must be independent of input order"
            );
            let cube = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("scenery probe fixture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 6,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                cube.as_image_copy(),
                &[255, 0, 0, 255].repeat(6),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 6,
                },
            );
            let cube_view = cube.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            });
            let metal = PbrMaterial {
                metallic: 1.,
                roughness: 0.12,
                ..Default::default()
            };
            let mut ambient_frame = frame;
            ambient_frame.sun_radiance = [0.; 4];
            let procedural = render(
                std::slice::from_ref(&metal),
                &[triangle(0., 0)],
                ambient_frame,
                None,
            );
            let probed = render(
                &[metal],
                &[triangle(0., 0)],
                ambient_frame,
                Some(SkyProbe {
                    view: &cube_view,
                    mip_count: 1,
                }),
            );
            assert_ne!(pixel(&procedural), pixel(&probed));
            assert!(pixel(&probed)[0] > pixel(&probed)[2] + 30);
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            assert!(device.pop_error_scope().await.is_none());
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod canopy_tests {
    use super::*;
    use flightcraft_aero::exterior::{ModelNode, MovingPart};
    #[test]
    fn canopy_slide_moves_only_its_descendants_aft_at_every_fraction() {
        let mut asset = (*builtin("corvid").unwrap()).clone();
        asset.definition.moving_parts = vec![MovingPart {
            node: "canopy_slide".into(),
            motion: Motion::Slide,
            axis: [-1., 0., 0.],
            travel_rad: [0., 0.],
            travel_m: Some(1.1),
            gear_index: 0,
            wheel_radius_m: 0.19,
        }];
        let mut model = (*asset.lod0).clone();
        model.nodes = [
            ("airframe", None),
            ("windshield", None),
            ("roll_over_bar", None),
            ("canopy_slide", None),
            ("canopy_glass", Some(3)),
            ("canopy_bow_front", Some(3)),
            ("canopy_outside_latch", Some(3)),
            ("canopy_inside_latch", Some(2)),
            ("seat_back", None),
        ]
        .into_iter()
        .map(|(name, parent)| ModelNode {
            name: name.into(),
            parent,
            transform: Mat4::from_translation(Vec3::new(-2., 0., -0.5)).to_cols_array_2d(),
            vertices: vec![],
        })
        .collect();
        let neutral = transforms(&asset, &model, &Pose::default());
        for fraction in [0., 0.25, 0.5, 0.75, 1.] {
            let pose = Pose {
                canopy_open_fraction: fraction,
                ..Default::default()
            };
            let actual = transforms(&asset, &model, &pose);
            for (i, node) in model.nodes.iter().enumerate() {
                let delta = actual[i].w_axis.truncate() - neutral[i].w_axis.truncate();
                if node.name == "canopy_slide" || node.parent == Some(3) {
                    assert!((delta - Vec3::new(-1.1 * fraction, 0., 0.)).length() < 1e-6);
                } else {
                    assert_eq!(actual[i], neutral[i], "{} must stay fixed", node.name);
                }
                assert_eq!(Mat3::from_mat4(actual[i]), Mat3::from_mat4(neutral[i]));
            }
        }
    }
    #[test]
    fn exterior_pose_reads_live_canopy_state_without_advancing_simulation() {
        let mut config = flightcraft_aero::AircraftCatalog::builtins()
            .unwrap()
            .resolve_aircraft("corvid")
            .unwrap();
        config
            .installed_systems
            .definition
            .as_mut()
            .unwrap()
            .sliding_canopy = true;
        let mut sim = flightcraft_sim::Sim::new(&flightcraft_sim::Start::ParkedAt {
            position_m: [0.; 3],
            heading_rad: 0.,
        })
        .unwrap();
        sim.configure_systems(&config).unwrap();
        let before = sim.state_hash();
        let mut pose = Pose::default();
        pose.update(&sim);
        assert_eq!(pose.canopy_open_fraction, 1.);
        assert_eq!(sim.state_hash(), before);
        sim.set_canopy(flightcraft_sim::canopy::CanopyAction::Open, Some(0.4))
            .unwrap();
        pose.update(&sim);
        assert_eq!(pose.canopy_open_fraction, 0.4);
    }
}
