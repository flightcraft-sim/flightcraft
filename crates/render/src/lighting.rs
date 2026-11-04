//! Original single-scattering atmosphere and astronomical sun. Presentation only; no wall clock.
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
/// UTC presentation clock, independent of the deterministic flight state. Simulation seconds advance it.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LightingSettings {
    /// Gregorian calendar, 1900–2100.
    #[schemars(range(min = 1900, max = 2100))]
    pub year: i32,
    #[schemars(range(min = 1, max = 12))]
    pub month: u8,
    #[schemars(range(min = 1, max = 31))]
    pub day: u8,
    #[schemars(range(min = 0, max = 24))]
    pub utc_hour: f64,
    #[schemars(range(min = -6, max = 6))]
    pub exposure_ev: f32,
}
impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            year: 2026,
            month: 11,
            day: 3,
            utc_hour: 17.,
            exposure_ev: 0.,
        }
    }
}
impl LightingSettings {
    pub fn validate(&self) -> Result<(), String> {
        let leap = self.year % 4 == 0 && (self.year % 100 != 0 || self.year % 400 == 0);
        let days = match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if leap {
                    29
                } else {
                    28
                }
            }
            _ => 0,
        };
        if !(1900..=2100).contains(&self.year)
            || self.day == 0
            || self.day > days
            || !self.utc_hour.is_finite()
            || !(0.0..24.).contains(&self.utc_hour)
            || !self.exposure_ev.is_finite()
            || !(-6.0..=6.).contains(&self.exposure_ev)
        {
            return Err("Invalid scenery date, UTC time, or exposure".into());
        }
        Ok(())
    }
}
/// Low-order solar ephemeris from orbital longitude/obliquity, transformed through local sidereal time.
/// Returns the direction toward the sun in rendering coordinates (east, up, south).
pub fn sun_direction(
    settings: LightingSettings,
    latitude: f64,
    longitude: f64,
    seconds: f64,
) -> Result<[f32; 3], String> {
    settings.validate()?;
    if !latitude.is_finite()
        || latitude.abs() > 90.
        || !longitude.is_finite()
        || longitude.abs() > 180.
        || !seconds.is_finite()
        || seconds.abs() > 1e12
    {
        return Err("Invalid solar location or elapsed time".into());
    }
    let (mut y, mut m) = (settings.year, i32::from(settings.month));
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let a = (f64::from(y) / 100.).floor();
    let jd = (365.25 * f64::from(y + 4716)).floor()
        + (30.6001 * f64::from(m + 1)).floor()
        + f64::from(settings.day)
        + 2.
        - a
        + (a / 4.).floor()
        - 1524.5
        + settings.utc_hour / 24.
        + seconds / 86400.;
    let d = jd - 2451545.;
    let g = (357.529 + 0.98560028 * d).to_radians();
    let lambda = (280.459 + 0.98564736 * d + 1.915 * g.sin() + 0.020 * (2. * g).sin()).to_radians();
    let epsilon = (23.439 - 0.00000036 * d).to_radians();
    let ra = (epsilon.cos() * lambda.sin()).atan2(lambda.cos());
    let dec = (epsilon.sin() * lambda.sin()).asin();
    let h = (280.46061837 + 360.98564736629 * d + longitude).to_radians() - ra;
    let lat = latitude.to_radians();
    Ok([
        (-dec.cos() * h.sin()) as f32,
        (lat.sin() * dec.sin() + lat.cos() * dec.cos() * h.cos()) as f32,
        (lat.sin() * dec.cos() * h.cos() - lat.cos() * dec.sin()) as f32,
    ])
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Frame {
    pub shadow: [[[f32; 4]; 4]; 3],
    pub sun: [f32; 4],
    pub radiance: [f32; 4],
    /// RGB Rayleigh optical depths at observer, aerosol optical depth.
    pub optical: [f32; 4],
    /// Eye xyz; extinction in inverse meters (Koschmieder visibility).
    pub eye: [f32; 4],
    /// Split distances, number of active cascades.
    pub splits: [f32; 4],
    /// Exposure, enabled, cloud direct transmittance, atlas texel size.
    pub settings: [f32; 4],
    pub clouds: [[f32; 4]; 3],
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            shadow: [Mat4::IDENTITY.to_cols_array_2d(); 3],
            sun: [0., 1., 0., 1.],
            radiance: [2.8, 2.65, 2.4, 1.],
            optical: [0.044, 0.105, 0.255, 0.06],
            eye: [0.; 4],
            splits: [180., 1800., 3600., 0.],
            settings: [0.8, 0., 1., 1. / 1024.],
            clouds: [[0.; 4]; 3],
        }
    }
}
impl Frame {
    pub fn new(sun: [f32; 3], eye: [f32; 3], visibility: f32, cloud: f32, ev: f32) -> Self {
        let height = eye[1].max(0.);
        let density = (-height / 8000.).exp();
        let optical = [
            0.044 * density,
            0.105 * density,
            0.255 * density,
            (3.912 / visibility.clamp(100., 160934.) * 600. * (-height / 1200.).exp()).clamp(0.012, 2.5),
        ];
        let direct = solar_transmission(sun[1], optical);
        Self {
            sun: [sun[0], sun[1], sun[2], 1.],
            radiance: [direct[0] * 4., direct[1] * 3.9, direct[2] * 3.7, 1.],
            optical,
            eye: [eye[0], eye[1], eye[2], 3.912 / visibility.clamp(100., 160934.)],
            settings: [(0.8 * 2_f32.powf(ev)).min(32.), 1., cloud, 1. / 1024.],
            ..Self::default()
        }
    }
    pub fn display(&self, color: Vec3) -> [f32; 4] {
        let x = color * self.settings[0];
        let c = ((x * (2.51 * x + Vec3::splat(0.03)))
            / (x * (2.43 * x + Vec3::splat(0.59)) + Vec3::splat(0.14)))
        .clamp(Vec3::ZERO, Vec3::ONE);
        let rgb = c.to_array().map(|c| {
            if c > 0.0031308 {
                1.055 * c.powf(1. / 2.4) - 0.055
            } else {
                12.92 * c
            }
        });
        [rgb[0], rgb[1], rgb[2], 1.]
    }
    pub fn sky(&self, ray: Vec3) -> Vec3 {
        let sun = Vec3::from_slice(&self.sun);
        let mu = ray.dot(sun).clamp(-1., 1.);
        let air = airmass(ray.y.max(0.));
        let beta = Vec3::from_slice(&self.optical);
        let mie = self.optical[3];
        let tau = (beta + Vec3::splat(mie)) * air;
        let transmission = Vec3::new((-tau.x).exp(), (-tau.y).exp(), (-tau.z).exp());
        let phase_r = 0.75 * (1. + mu * mu);
        let phase_m = (1. - 0.76_f32.powi(2)) / (1. + 0.76_f32.powi(2) - 1.52 * mu).max(0.02).powf(1.5);
        let scatter = (beta * phase_r + Vec3::splat(mie * phase_m)) / (beta + Vec3::splat(mie));
        let twilight = ((sun.y + 0.12) / 0.16).clamp(0., 1.);
        let mean_depth = [
            self.optical[0] * 0.45,
            self.optical[1] * 0.45,
            self.optical[2] * 0.45,
            self.optical[3] * 0.25,
        ];
        let lit = Vec3::from_slice(&solar_transmission(sun.y.max(0.025), mean_depth));
        let sky =
            (Vec3::ONE - transmission) * scatter * lit * 1.2 * twilight + Vec3::new(0.001, 0.002, 0.005);
        if ray.y < 0. {
            sky.lerp(
                Vec3::new(0.055, 0.065, 0.045) * twilight,
                (-ray.y * 4.).clamp(0., 1.),
            )
        } else {
            sky
        }
    }
    pub fn cascades(&mut self, forward: Vec3, quality: crate::scenery::Quality) {
        let sun = Vec3::from_slice(&self.sun).normalize_or_zero();
        let eye = Vec3::from_slice(&self.eye);
        let count = if quality == crate::scenery::Quality::Low {
            2
        } else {
            3
        };
        self.splits[3] = count as f32;
        let distances = if quality == crate::scenery::Quality::Low {
            [180., 1800., 5000.]
        } else {
            [400., 1600., 5000.]
        };
        self.splits = [distances[0], distances[1], distances[2], count as f32];
        for (i, far) in distances.into_iter().enumerate() {
            let radius = far * 1.05;
            let center = eye + forward * far * 0.25;
            let up = if sun.y.abs() > 0.97 { Vec3::Z } else { Vec3::Y };
            let view = Mat4::look_at_rh(center + sun * (radius + 1800.), center, up);
            // Anchor light-space translation to a shadow texel to suppress crawling on static ground.
            let texel = 2. * radius
                / (if quality == crate::scenery::Quality::Low {
                    512.
                } else {
                    1024.
                });
            let origin = view.transform_point3(Vec3::ZERO);
            let shift = (origin / texel).round() * texel - origin;
            let view = Mat4::from_translation(Vec3::new(shift.x, shift.y, 0.)) * view;
            if let Some(matrix) = self.shadow.get_mut(i) {
                *matrix = (Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.1, 2. * radius + 3600.)
                    * view)
                    .to_cols_array_2d();
            }
        }
    }
}
fn airmass(y: f32) -> f32 {
    1. / (y.max(0.) + 0.025 * (-11. * y.max(0.)).exp()).max(0.025)
}
fn solar_transmission(y: f32, optical: [f32; 4]) -> [f32; 3] {
    let air = airmass(y);
    let above = ((y + 0.009) / 0.018).clamp(0., 1.);
    [optical[0], optical[1], optical[2]].map(|r| (-(r + optical[3]) * air).exp() * above)
}
pub(crate) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("physical atmosphere and cascades"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Frame>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ],
    })
}
pub(crate) struct GpuLighting {
    pub bind: wgpu::BindGroup,
    pub uniform: wgpu::Buffer,
    pub atlas: wgpu::TextureView,
    pub size: u32,
    pub probe: crate::exterior::EnvironmentProbe,
    texture: wgpu::Texture,
    key: Option<[u32; 8]>,
}
impl GpuLighting {
    pub fn new(device: &wgpu::Device, quality: crate::scenery::Quality) -> Self {
        use wgpu::util::DeviceExt;
        let size = if quality == crate::scenery::Quality::Low {
            1024
        } else {
            2048
        };
        let atlas = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("cascaded sun depth atlas"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("physical atmosphere"),
            contents: bytemuck::bytes_of(&Frame::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sun PCF"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun and atmosphere"),
            layout: &layout(device),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("single-scattering sky GGX probe"),
            size: wgpu::Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 6,
            },
            mip_level_count: 6,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        Self {
            bind,
            uniform,
            atlas,
            size,
            probe: crate::exterior::EnvironmentProbe { view, mip_count: 6 },
            texture,
            key: None,
        }
    }
    pub fn update(&mut self, queue: &wgpu::Queue, frame: &Frame) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(frame));
        let key = [
            frame.sun[0].to_bits() & !0xfff,
            frame.sun[1].to_bits() & !0xfff,
            frame.sun[2].to_bits() & !0xfff,
            frame.optical[0].to_bits() & !0xffff,
            frame.optical[1].to_bits() & !0xffff,
            frame.optical[2].to_bits() & !0xffff,
            frame.optical[3].to_bits() & !0xffff,
            frame.settings[2].to_bits(),
        ];
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        for level in 0..6 {
            let side = 32 >> level;
            let rough = level as f32 / 5.;
            for face in 0..6 {
                let mut rgba = Vec::with_capacity((side * side * 4) as usize);
                for y in 0..side {
                    for x in 0..side {
                        let u = 2. * (x as f32 + 0.5) / side as f32 - 1.;
                        let v = 2. * (y as f32 + 0.5) / side as f32 - 1.;
                        let dir = match face {
                            0 => Vec3::new(1., -v, -u),
                            1 => Vec3::new(-1., -v, u),
                            2 => Vec3::new(u, 1., v),
                            3 => Vec3::new(u, -1., -v),
                            4 => Vec3::new(u, -v, 1.),
                            _ => Vec3::new(-u, -v, -1.),
                        }
                        .normalize();
                        let t = dir
                            .cross(if dir.y.abs() > 0.9 { Vec3::X } else { Vec3::Y })
                            .normalize();
                        let b = dir.cross(t);
                        let mut color = Vec3::ZERO;
                        let mut weight = 0.;
                        for k in 0..8 {
                            let a = k as f32 * 2.399963;
                            let xi = (k as f32 + 0.5) / 8.;
                            let alpha = rough.max(0.01).powi(2);
                            let cos = ((1. - xi) / (1. + (alpha * alpha - 1.) * xi)).sqrt();
                            let sin = (1. - cos * cos).sqrt();
                            let h = (dir * cos + (t * a.cos() + b * a.sin()) * sin).normalize();
                            let l = (2. * dir.dot(h) * h - dir).normalize();
                            let w = dir.dot(l).max(0.);
                            color += frame.sky(l) * w;
                            weight += w;
                        }
                        color /= weight.max(0.001);
                        color = color.lerp(Vec3::splat(0.3), 1. - frame.settings[2]);
                        rgba.extend(color.to_array().map(|v| (v.clamp(0., 1.) * 255.).round() as u8));
                        rgba.push(255);
                    }
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.texture,
                        mip_level: level,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: face },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &rgba,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(side * 4),
                        rows_per_image: Some(side),
                    },
                    wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ephemeris_matches_independent_equinox_and_solstice_controls_and_rejects_bad_dates() {
        // Equinox Greenwich apparent noon: near zenith at equator. June solstice: 61.96 degrees at Greenwich.
        let mut s = LightingSettings {
            year: 2026,
            month: 3,
            day: 20,
            utc_hour: 12.12,
            exposure_ev: 0.,
        };
        let a = sun_direction(s, 0., 0., 0.).unwrap();
        assert!(a[1] > 0.999);
        s.month = 6;
        s.day = 21;
        s.utc_hour = 12.03;
        let b = sun_direction(s, 51.4779, 0., 0.).unwrap();
        assert!((b[1].asin().to_degrees() - 61.96).abs() < 0.2);
        assert!(sun_direction(s, 51.4779, 0., 43200.).unwrap()[1] < 0.);
        s.month = 2;
        s.day = 30;
        assert!(s.validate().is_err());
        assert!(sun_direction(s, f64::NAN, 0., 0.).is_err());
    }
    #[test]
    fn scattering_and_cascades_are_finite_and_visibility_drives_extinction() {
        let mut f = Frame::new([0., 0.8, 0.6], [0., 113., 0.], 16000., 1., 0.);
        let clear = f.eye[3];
        assert!(f.sky(Vec3::Y).z > f.sky(Vec3::Y).x);
        f.cascades(Vec3::NEG_Z, crate::scenery::Quality::Low);
        assert_eq!(f.splits[3], 2.);
        assert!(f.shadow.iter().flatten().flatten().all(|v| v.is_finite()));
        assert!(Frame::new([0., 0.8, 0.6], [0., 113., 0.], 800., 1., 0.).eye[3] > clear * 10.);
    }
}
