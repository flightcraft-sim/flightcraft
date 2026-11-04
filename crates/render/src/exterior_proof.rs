//! Matched-camera proof using the same PBR loader, posing, and draw path as the scene viewport.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use crate::exterior::*;
use flightcraft_aero::exterior::{Exterior, ExteriorAsset};
use glam::{Mat4, Vec2, Vec3, Vec4};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    struct Wake(std::thread::Thread);
    impl std::task::Wake for Wake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Wake(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(v) => return v,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}

fn load_pack() -> Arc<ExteriorAsset> {
    let pack = PathBuf::from(std::env::var("FLIGHTCRAFT_SLING_PACK").unwrap());
    let definition: Exterior =
        serde_json::from_slice(&std::fs::read(pack.join("exterior.json")).unwrap()).unwrap();
    let assets = definition
        .paths()
        .into_iter()
        .map(|name| (name.to_owned(), std::fs::read(pack.join(name)).unwrap()))
        .collect();
    let mut cache = std::collections::BTreeMap::new();
    let mut load = |name: &str| {
        Arc::new(
            flightcraft_aero::exterior::load_glb_with_assets(
                &std::fs::read(pack.join(name)).unwrap(),
                &definition,
                name,
                &assets,
                &mut cache,
            )
            .unwrap(),
        )
    };
    Arc::new(ExteriorAsset {
        lod0: load("exterior.glb"),
        lod1: load("exterior-lod1.glb"),
        lod2: Some(load("exterior-lod2.glb")),
        definition,
    })
}

// The same single-sample PBR render pass and readback for matched stills and motion frames.
fn capture_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut PbrRenderer,
    frame: PbrFrame,
    primitives: &[PbrPrimitive],
    size: [u32; 2],
) -> image::RgbaImage {
    let [w, h] = size;
    // Ground is a proof-only apron; loaded aircraft geometry retains the normal 120k gate.
    renderer.prepare(device, queue, frame, primitives, None).unwrap();
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("matched exterior proof"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
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
    let row = (w * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("matched exterior readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("matched aircraft exterior"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.78,
                        g: 0.80,
                        b: 0.82,
                        a: 1.,
                    }),
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
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        renderer.draw(&mut pass);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let pixels = {
        let mapped = readback.slice(..).get_mapped_range();
        mapped
            .chunks(row as usize)
            .flat_map(|r| r[..w as usize * 4].iter().copied())
            .collect::<Vec<_>>()
    };
    readback.unmap();
    image::RgbaImage::from_raw(w, h, pixels).unwrap()
}

#[test]
#[ignore = "requires FLIGHTCRAFT_SLING_PACK with Blender GLBs and a registration surface"]
fn sling_baked_registration_replaced_on_both_sides_at_every_lod() {
    let base = load_pack();
    let surface = base.definition.registration.surface.as_ref().unwrap();
    let mut scene = crate::Scene::default();
    scene.set_exterior(base.clone());
    scene.set_registration(Some("N824RX"));
    let painted = scene.exterior.as_ref().unwrap();
    for (before, after) in [
        (&base.lod0, &painted.lod0),
        (&base.lod1, &painted.lod1),
        (base.lod2.as_ref().unwrap(), painted.lod2.as_ref().unwrap()),
    ] {
        let matrices = transforms(&base, before, &Default::default());
        for (index, (old, new)) in before.nodes.iter().zip(&after.nodes).enumerate() {
            assert_eq!(old.vertices.len(), new.vertices.len());
            for (a, b) in old.vertices.iter().zip(&new.vertices) {
                assert_eq!(a.position, b.position);
                assert_eq!(a.uv, b.uv);
                assert_eq!(a.normal, b.normal);
            }
            if old.name != surface.node {
                continue;
            }
            let mut erased = [0; 2];
            let mut ink = [0; 2];
            for triangle in old.vertices.as_chunks::<3>().0 {
                let [a, b, c] = triangle;
                let [pa, pb, pc] =
                    [a, b, c].map(|v| matrices[index].transform_point3(Vec3::from_array(v.position)));
                let side = usize::from(pa.y + pb.y + pc.y > 0.);
                let [a2, b2, c2] = [pa, pb, pc].map(|p| Vec2::new(p.x, p.z));
                let determinant = (b2 - a2).perp_dot(c2 - a2);
                if determinant.abs() < 1e-12 {
                    continue;
                }
                let image_index = before.materials[a.material].images[0].unwrap();
                let old_image = &before.images[image_index];
                let new_image = &after.images[image_index];
                for image in before.materials[a.material].images[1..].iter().flatten() {
                    assert!(Arc::ptr_eq(
                        &before.images[*image].rgba,
                        &after.images[*image].rgba
                    ));
                }
                for ix in 0..50 {
                    for iz in 0..20 {
                        let p = Vec2::new(
                            surface.x_m[0] + (ix as f32 + 0.5) / 50. * (surface.x_m[1] - surface.x_m[0]),
                            surface.z_m[0] + (iz as f32 + 0.5) / 20. * (surface.z_m[1] - surface.z_m[0]),
                        );
                        let beta = (p - a2).perp_dot(c2 - a2) / determinant;
                        let gamma = (b2 - a2).perp_dot(p - a2) / determinant;
                        if beta < 0. || gamma < 0. || beta + gamma > 1. {
                            continue;
                        }
                        let uv = Vec2::from_array(a.uv) * (1. - beta - gamma)
                            + Vec2::from_array(b.uv) * beta
                            + Vec2::from_array(c.uv) * gamma;
                        let [w, h] = old_image.size;
                        let x = (uv.x * w as f32).floor().clamp(0., w.saturating_sub(1) as f32) as usize;
                        let y = (uv.y * h as f32).floor().clamp(0., h.saturating_sub(1) as f32) as usize;
                        let offset = (y * w as usize + x) * 4;
                        let old_pixel = &old_image.rgba[offset..offset + 4];
                        let new_pixel = &new_image.rgba[offset..offset + 4];
                        if old_pixel[0] < 160 && new_pixel[0] > 200 {
                            erased[side] += 1;
                        }
                        if new_pixel[0] < 120 {
                            ink[side] += 1;
                        }
                    }
                }
            }
            assert!(
                erased.iter().all(|n| *n > 15),
                "old N371FC must be erased on both sides: {erased:?}"
            );
            assert!(
                ink.iter().all(|n| *n > 15),
                "new number must appear on both sides: {ink:?}"
            );
        }
    }
    scene.set_registration(None);
    assert!(Arc::ptr_eq(scene.exterior.as_ref().unwrap(), &base));
}

#[test]
#[ignore = "requires an offscreen GPU, FLIGHTCRAFT_SLING_PACK, and FLIGHTCRAFT_CYCLES_OUTPUT"]
fn matched_sling_exterior_cameras() {
    block_on(async {
        let source = PathBuf::from(std::env::var("FLIGHTCRAFT_CYCLES_OUTPUT").unwrap());
        let out = PathBuf::from(
            std::env::var("FLIGHTCRAFT_EXTERIOR_PROOF").unwrap_or_else(|_| "out/exterior-pbr".into()),
        );
        std::fs::create_dir_all(&out).unwrap();
        let mut scene = crate::Scene::default();
        scene.set_exterior(load_pack());
        let registration = std::env::var("FLIGHTCRAFT_PROOF_REGISTRATION").ok();
        scene.set_registration(registration.as_deref());
        let asset = scene.exterior.as_ref().unwrap();
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
        let mut limits = wgpu::Limits::downlevel_webgl2_defaults();
        limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d.min(4096);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            })
            .await
            .unwrap();
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut renderer = PbrRenderer::new(&device);
        let mut materials = pbr_materials(&asset.lod0);
        let floor_material = materials.len();
        materials.push(PbrMaterial {
            base_color: [0.26, 0.28, 0.29, 1.],
            roughness: 0.86,
            ..Default::default()
        });
        renderer.set_materials(&device, &queue, &materials).unwrap();
        let mut pose = Pose::default();
        pose.lights = [true; 4];
        let mut primitives = pbr_primitives(asset, &asset.lod0, &pose);
        let ground = |position| PbrVertex {
            position,
            normal: [0., 0., -1.],
            tangent: [1., 0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
            emissive_scale: 1.,
        };
        primitives.push(PbrPrimitive {
            material: floor_material,
            vertices: [
                [-180., -180., 0.855],
                [180., 180., 0.855],
                [180., -180., 0.855],
                [-180., -180., 0.855],
                [-180., 180., 0.855],
                [180., 180., 0.855],
            ]
            .map(ground)
            .to_vec(),
        });
        for primitive in &primitives {
            for v in &primitive.vertices {
                let n = Vec3::from_array(v.normal);
                let [x, y, z, w] = v.tangent;
                let t = Vec3::new(x, y, z);
                assert!(
                    (0.99..=1.01).contains(&n.length_squared())
                        && (0.99..=1.01).contains(&t.length_squared())
                        && n.dot(t).abs() < 0.01
                        && (w.abs() - 1.).abs() < 0.001,
                    "material {} position {:?} normal {:?} tangent {:?}",
                    primitive.material,
                    v.position,
                    v.normal,
                    v.tangent
                );
            }
        }
        let manifest = std::fs::read(source.join("cameras.json")).unwrap();
        let cameras: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        let world = Mat4::from_cols(
            Vec4::new(0., 0., -1., 0.),
            Vec4::new(1., 0., 0., 0.),
            Vec4::new(0., -1., 0., 0.),
            Vec4::W,
        );
        let vector = |v: &serde_json::Value| {
            Vec3::new(
                v[0].as_f64().unwrap() as f32,
                v[1].as_f64().unwrap() as f32,
                v[2].as_f64().unwrap() as f32,
            )
        };
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(source.join("validation.json")).unwrap()).unwrap();
        let sunlight = world.transform_vector3(vector(&receipt["lighting"]["sun_direction_body"]));
        let sun_rgb = vector(&receipt["lighting"]["sun_rgb"]);
        let exposure = 2_f32.powf(receipt["lighting"]["exposure_ev"].as_f64().unwrap() as f32);
        let mut images = Vec::new();
        for label in [
            "director-front",
            "director-rear",
            "director-left",
            "director-top",
            "beauty",
            "elevation-side",
            "plan-top",
        ]
        .into_iter()
        .chain(
            ["beauty-open", "elevation-side-open"]
                .into_iter()
                .filter(|label| cameras.get(*label).is_some()),
        ) {
            let camera = &cameras[label];
            let mut current_pose = pose.clone();
            current_pose.canopy_open_fraction = camera["canopy_open_fraction"].as_f64().unwrap_or(0.) as f32;
            let mut current_primitives = pbr_primitives(asset, &asset.lod0, &current_pose);
            current_primitives.push(primitives.last().unwrap().clone());
            let w = camera["size"][0].as_u64().unwrap() as u32;
            let h = camera["size"][1].as_u64().unwrap() as u32;
            let eye = world.transform_point3(vector(&camera["eye_body_m"]));
            let target = world.transform_point3(vector(&camera["target_body_m"]));
            let up = world.transform_vector3(vector(&camera["up_body"]));
            let projection = if camera["projection"] == "orthographic" {
                let width = camera["ortho_width_m"].as_f64().unwrap() as f32;
                let height = width * h as f32 / w as f32;
                Mat4::orthographic_rh(-width / 2., width / 2., -height / 2., height / 2., 0.05, 10000.)
            } else {
                Mat4::perspective_rh(
                    camera["vertical_fov_rad"].as_f64().unwrap() as f32,
                    w as f32 / h as f32,
                    0.05,
                    10000.,
                )
            };
            let frame = PbrFrame {
                view_projection: (projection * Mat4::look_at_rh(eye, target, up)).to_cols_array_2d(),
                world_from_body: world.to_cols_array_2d(),
                eye: eye.extend(1.).to_array(),
                sun_direction: sunlight.extend(1.).to_array(),
                sun_radiance: sun_rgb.extend(0.).to_array(),
                environment: [0., 0., exposure, 0.6],
                ..Default::default()
            };
            let pixels =
                capture_frame(&device, &queue, &mut renderer, frame, &current_primitives, [w, h]).into_raw();
            image::save_buffer(
                out.join(format!("engine-{label}.png")),
                &pixels,
                w,
                h,
                image::ColorType::Rgba8,
            )
            .unwrap();
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) < 150)
                    .count()
                    > 100,
                "aircraft must be visible at {label}"
            );
            images.push((label, pixels));
        }
        std::fs::write(out.join("cameras.json"), manifest).unwrap();
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(device.pop_error_scope().await.is_none());
        assert_eq!(images.len(), cameras.as_object().unwrap().len());
    });
}

// Reuse our original registration strokes for contact-sheet captions. Captions are
// outside the images; individual proof frames remain unmodified GPU readbacks.
fn caption(image: &mut image::RgbaImage, text: &str, origin: [u32; 2], height: f32) {
    for (index, c) in text.chars().enumerate() {
        let offset = Vec2::new(origin[0] as f32 + index as f32 * height * 0.8, origin[1] as f32);
        for path in crate::registration::glyph(c) {
            for pair in path.windows(2) {
                let a = offset + pair[0] * Vec2::new(height * 0.65, height);
                let b = offset + pair[1] * Vec2::new(height * 0.65, height);
                let low = a.min(b) - Vec2::splat(2.);
                let high = a.max(b) + Vec2::splat(2.);
                for y in (low.y.max(0.) as u32)..=(high.y as u32).min(image.height() - 1) {
                    for x in (low.x.max(0.) as u32)..=(high.x as u32).min(image.width() - 1) {
                        let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-8)).clamp(0., 1.);
                        if p.distance(a.lerp(b, t)) < 1.15 {
                            image.put_pixel(x, y, image::Rgba([24, 32, 42, 255]));
                        }
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires an offscreen GPU and FLIGHTCRAFT_SLING_PACK with installed Blender GLBs"]
fn sling_exterior_motion_proof() {
    block_on(async {
        let asset = load_pack();
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/plane/motion");
        std::fs::create_dir_all(&out).unwrap();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let mut limits = wgpu::Limits::downlevel_webgl2_defaults();
        limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d.min(4096);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            })
            .await
            .unwrap();
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut renderer = PbrRenderer::new(&device);
        renderer
            .set_materials(&device, &queue, &pbr_materials(&asset.lod0))
            .unwrap();
        let world = Mat4::from_cols(
            Vec4::new(0., 0., -1., 0.),
            Vec4::new(1., 0., 0., 0.),
            Vec4::new(0., -1., 0., 0.),
            Vec4::W,
        );
        // Director Wing's actual eye position. Aim at the aircraft centre for
        // this proof so the trailing edges and tail remain in the fixed frame.
        let eye_body = Vec3::new(-1., -7., -1.4);
        let target_body = Vec3::new(-1.6, 0., -0.15);
        let eye = world.transform_point3(eye_body);
        let target = world.transform_point3(target_body);
        let size = [1280, 800];
        let fov = 50_f32.to_radians();
        let frame = PbrFrame {
            view_projection: (Mat4::perspective_rh(fov, size[0] as f32 / size[1] as f32, 0.015, 1000.)
                * Mat4::look_at_rh(eye, target, Vec3::Y))
            .to_cols_array_2d(),
            world_from_body: world.to_cols_array_2d(),
            eye: eye.extend(1.).to_array(),
            sun_direction: world
                .transform_vector3(Vec3::new(0.4, -0.6, -0.7).normalize())
                .extend(1.)
                .to_array(),
            sun_radiance: [3., 2.9, 2.7, 0.],
            environment: [0., 0., 2_f32.powf(-0.7), 0.6],
            ..Default::default()
        };
        let mut poses = Vec::new();
        for (control, name) in ["aileron", "elevator", "rudder", "flaps"].into_iter().enumerate() {
            for index in 0..12 {
                let demand = if control == 3 {
                    index as f32 / 11.
                } else {
                    -1. + 2. * index as f32 / 11.
                };
                let mut pose = Pose::default();
                pose.controls[control] = demand;
                poses.push((
                    format!("{name}-{index:02}"),
                    format!("{} {:+04}", name.to_uppercase(), (demand * 100.).round() as i32),
                    pose,
                ));
            }
        }
        // Stopped phases are identical across timestamps; 300 RPM advances 15
        // degrees per 1/120 second (8 distinct orientations of a 3-blade prop).
        for (rpm, count) in [(0., 2), (300., 8), (2000., 2)] {
            for index in 0..count {
                let time_s = index as f32 / 120.;
                let mut pose = Pose::default();
                pose.time_s = time_s;
                pose.propeller_rpm = rpm;
                pose.propeller_phase = rpm * time_s * std::f32::consts::TAU / 60.;
                poses.push((
                    format!("prop-{:04}-{index:02}", rpm as u32),
                    format!(
                        "PROP {} RPM {}",
                        rpm as u32,
                        if rpm > 650. {
                            "BLUR".into()
                        } else {
                            format!("{:03} DEG", pose.propeller_phase.to_degrees().round() as u32)
                        }
                    ),
                    pose,
                ));
            }
        }
        let has_canopy = asset
            .definition
            .moving_parts
            .iter()
            .any(|p| p.node == "canopy_slide");
        if has_canopy {
            for index in 0..8 {
                let mut pose = Pose::default();
                pose.canopy_open_fraction = index as f32 / 7.;
                poses.push((
                    format!("canopy-{index:02}"),
                    format!(
                        "CANOPY {:03} OPEN",
                        (pose.canopy_open_fraction * 100.).round() as u32
                    ),
                    pose,
                ));
            }
        }
        assert_eq!(poses.len(), if has_canopy { 68 } else { 60 });
        let cell = [400_u32, 286_u32];
        let mut sheet = image::RgbaImage::from_pixel(
            cell[0] * 12,
            cell[1] * if has_canopy { 6 } else { 5 } + 48,
            image::Rgba([242, 245, 248, 255]),
        );
        caption(
            &mut sheet,
            "SLING - ENGINE DIRECTOR WING - CONTROLS, PROP, CANOPY - LEFT TO RIGHT",
            [12, 10],
            24.,
        );
        let mut receipt = Vec::new();
        let mut previous: Option<image::RgbaImage> = None;
        let mut still = None;
        let mut solid_count = None;
        for (index, (name, label, pose)) in poses.iter().enumerate() {
            let primitives = pbr_primitives(&asset, &asset.lod0, pose);
            let vertices: usize = primitives.iter().map(|p| p.vertices.len()).sum();
            if pose.propeller_rpm <= 650. {
                assert_eq!(*solid_count.get_or_insert(vertices), vertices);
            } else {
                assert_ne!(solid_count.unwrap(), vertices, "blur must replace solid blades");
            }
            let image = capture_frame(&device, &queue, &mut renderer, frame, &primitives, size);
            let changed_pixels = previous.as_ref().map_or(0, |p| {
                p.pixels().zip(image.pixels()).filter(|(a, b)| a != b).count()
            });
            if index > 0 && index < 48 {
                assert!(
                    changed_pixels > 20,
                    "visible surface must move between frames: {name}"
                );
            }
            if name == "prop-0000-00" {
                still = Some(image.clone());
            } else if name == "prop-0000-01" {
                assert_eq!(still.as_ref().unwrap(), &image, "zero RPM must remain still");
            } else if name.starts_with("prop-0300-") && !name.ends_with("-00") {
                assert!(changed_pixels > 20, "300 RPM blades must visibly rotate");
            }
            if name.starts_with("canopy-") && name != "canopy-00" {
                assert!(changed_pixels > 20, "canopy must visibly slide at {name}");
            }
            image.save(out.join(format!("{name}.png"))).unwrap();
            let column = index as u32 % 12;
            let row = index as u32 / 12;
            let thumb = image::imageops::resize(&image, cell[0], 250, image::imageops::FilterType::Lanczos3);
            image::imageops::overlay(
                &mut sheet,
                &thumb,
                i64::from(column * cell[0]),
                i64::from(48 + row * cell[1]),
            );
            caption(
                &mut sheet,
                label,
                [column * cell[0] + 8, 48 + row * cell[1] + 256],
                18.,
            );
            // Record actual trailing-edge positions, independently of the screen
            // result, so direction and the fixed camera are reviewable.
            let matrices = transforms(&asset, &asset.lod0, pose);
            let neutral = transforms(&asset, &asset.lod0, &Pose::default());
            let mut trailing_edges = serde_json::Map::new();
            for part in &asset.definition.moving_parts {
                if !matches!(
                    part.motion,
                    flightcraft_aero::exterior::Motion::Aileron
                        | flightcraft_aero::exterior::Motion::Elevator
                        | flightcraft_aero::exterior::Motion::Rudder
                        | flightcraft_aero::exterior::Motion::Flaps
                ) {
                    continue;
                }
                let node = asset.lod0.nodes.iter().position(|n| n.name == part.node).unwrap();
                let vertex = asset.lod0.nodes[node]
                    .vertices
                    .iter()
                    .map(|v| Vec3::from_array(v.position))
                    .min_by(|a, b| {
                        neutral[node]
                            .transform_point3(*a)
                            .x
                            .total_cmp(&neutral[node].transform_point3(*b).x)
                    })
                    .unwrap();
                trailing_edges.insert(
                    part.node.clone(),
                    serde_json::json!(matrices[node].transform_point3(vertex).to_array()),
                );
            }
            receipt.push(serde_json::json!({"file": format!("{name}.png"), "controls": pose.controls, "canopy_slide_body_m": asset.lod0.nodes.iter().position(|n| n.name == "canopy_slide").map(|i| matrices[i].w_axis.truncate().to_array()), "canopy_open_fraction": pose.canopy_open_fraction, "propeller_rpm": pose.propeller_rpm, "time_s": pose.time_s, "phase_rad": pose.propeller_phase, "visible_vertices": vertices, "pixels_changed_from_previous": changed_pixels, "trailing_edges_body_m": trailing_edges}));
            eprintln!(
                "motion proof {}: {name} ({changed_pixels} changed pixels)",
                index + 1
            );
            previous = Some(image);
        }
        sheet.save(out.join("contact-sheet.png")).unwrap();
        std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "aircraft": asset.definition.aircraft, "model": "installed Sling LOD0", "adapter": format!("{:?}", adapter.get_info()),
            "camera": {"name": "director-wing", "eye_body_m": eye_body.to_array(), "target_body_m": target_body.to_array(), "up_body": [0,0,-1], "vertical_fov_rad": fov, "size": size, "note": "Director Wing eye; aim centred to include the full tail. Fixed camera, lighting and all other controls."},
            "contact_sheet": "contact-sheet.png", "frames": receipt
        })).unwrap()).unwrap();
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(device.pop_error_scope().await.is_none());
    });
}

#[test]
#[ignore = "requires FLIGHTCRAFT_SLING_PACK with installed V9 GLBs"]
fn sling_canopy_moves_only_its_descendants_at_every_lod() {
    let asset = load_pack();
    let part = asset
        .definition
        .moving_parts
        .iter()
        .find(|p| p.node == "canopy_slide")
        .expect("install V9 before running canopy proof");
    assert_eq!(part.motion, flightcraft_aero::exterior::Motion::Slide);
    assert_eq!(part.axis, [-1., 0., 0.]);
    let travel = part.travel_m.unwrap();
    for model in std::iter::once(&asset.lod0)
        .chain(std::iter::once(&asset.lod1))
        .chain(asset.lod2.as_ref())
    {
        let canopy = model.nodes.iter().position(|n| n.name == "canopy_slide").unwrap();
        let neutral = transforms(&asset, model, &Pose::default());
        for index in 0..8 {
            let mut pose = Pose::default();
            pose.canopy_open_fraction = index as f32 / 7.;
            let actual = transforms(&asset, model, &pose);
            for (i, node) in model.nodes.iter().enumerate() {
                let mut parent = Some(i);
                let mut moving = false;
                while let Some(p) = parent {
                    if p == canopy {
                        moving = true;
                        break;
                    }
                    parent = model.nodes[p].parent;
                }
                if moving {
                    let delta = actual[i].w_axis.truncate() - neutral[i].w_axis.truncate();
                    assert!(
                        (delta - Vec3::new(-travel * pose.canopy_open_fraction, 0., 0.)).length() < 1e-5,
                        "{}",
                        node.name
                    );
                    assert_eq!(
                        glam::Mat3::from_mat4(actual[i]),
                        glam::Mat3::from_mat4(neutral[i])
                    );
                } else {
                    assert_eq!(actual[i], neutral[i], "{} moved with canopy", node.name);
                }
            }
        }
    }
}
