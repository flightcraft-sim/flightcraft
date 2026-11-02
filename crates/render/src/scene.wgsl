// Uniforms, vertex buffers, and sampled textures only: WebGL2 / downlevel compatible.
struct Uniforms {
    view_projection: mat4x4<f32>,
    aircraft: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    ground: vec4<f32>,
    dry_grass: vec4<f32>,
    rock: vec4<f32>,
    forest: vec4<f32>,
    shoreline: vec4<f32>,
    water: vec4<f32>,
    grid: vec4<f32>,
    eye: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
struct Sky { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn sky_vertex(@builtin(vertex_index) index: u32) -> Sky {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var o: Sky;
    o.uv = uv;
    o.position = vec4<f32>(uv * 2.0 - 1.0, 0.9999, 1.0);
    return o;
}
@fragment fn sky_fragment(i: Sky) -> @location(0) vec4<f32> {
    let ray = normalize((u.world_from_clip * vec4<f32>(i.uv * 2.0 - 1.0, 1.0, 1.0)).xyz);
    let far_color = select(u.ground, u.sky_top, ray.y >= 0.0);
    return mix(u.sky_horizon, far_color, smoothstep(0.0, 0.65, abs(ray.y)));
}
struct Surface {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) kind: f32,
    @location(4) water_coverage: f32,
}
@vertex fn scene_vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>, @location(3) kind: f32, @location(4) water_coverage: f32) -> Surface {
    var o: Surface;
    var world = vec4<f32>(position, 1.0);
    var n = normal;
    if kind == 1.0 || kind == 2.0 {
        world = u.aircraft * world;
        n = (u.aircraft * vec4<f32>(normal, 0.0)).xyz;
    }
    o.world = world.xyz;
    o.normal = n;
    o.color = color;
    o.kind = kind;
    o.water_coverage = water_coverage;
    o.position = u.view_projection * world;
    return o;
}
// Fixed world-space sunlight, with enough ambient fill to keep the unlit sides readable.
const SUN: vec3<f32> = vec3<f32>(-0.58, 0.70, -0.42);

fn detail_lines(world: vec2<f32>, spacing: f32) -> f32 {
    let cell = world / spacing;
    let footprint = max(fwidth(cell), vec2<f32>(0.0001));
    let distance = abs(fract(cell - 0.5) - 0.5) / footprint;
    let line = 1.0 - min(min(distance.x, distance.y), 1.0);
    // Remove unresolved detail before it aliases, including at grazing cockpit angles.
    return line * (1.0 - smoothstep(0.15, 0.6, max(footprint.x, footprint.y)));
}

@fragment fn scene_fragment(i: Surface) -> @location(0) vec4<f32> {
    let distance = length(i.world - u.eye.xyz);
    let normal = normalize(i.normal);
    let sun = normalize(SUN);
    // Derivatives are evaluated outside material branches for downlevel shader validation.
    let fine = detail_lines(i.world.xz, 12.5);
    let coarse = detail_lines(i.world.xz, 100.0);
    let shore = 1.0 - smoothstep(0.0, 0.18, abs(i.water_coverage - 0.5));
    var color = i.color.rgb;
    if i.kind == 0.0 || i.kind == 4.0 {
        let height = i.world.y;
        let rock = smoothstep(0.045, 0.15, 1.0 - normal.y);
        let band_height = height + 22.0 * sin(i.world.x * 0.003) + 18.0 * sin(i.world.z * 0.004);
        let forest = smoothstep(40.0, 100.0, band_height) * (1.0 - smoothstep(190.0, 290.0, band_height));
        color = mix(u.ground.rgb, u.dry_grass.rgb, smoothstep(180.0, 460.0, height));
        color = mix(color, u.forest.rgb, forest * 0.78);
        color = mix(color, u.rock.rgb, rock);
        color = mix(color, u.shoreline.rgb, shore);
        let detail = fine * 0.10 * (1.0 - smoothstep(100.0, 650.0, distance))
            + coarse * 0.07 * (1.0 - smoothstep(400.0, 1800.0, distance));
        color = mix(color, u.grid.rgb, detail);
        color *= 0.46 + 0.54 * max(dot(normal, sun), 0.0);
    }
    if i.kind == 5.0 {
        // Flat geometry, with a broad sky reflection and a small, soft sun sheen. No wave mesh.
        let view = normalize(u.eye.xyz - i.world);
        let grazing = 1.0 - clamp(abs(view.y), 0.0, 1.0);
        let fresnel = grazing * grazing * grazing * grazing * grazing;
        let half_vector = normalize(sun + view);
        let sheen = pow(max(half_vector.y, 0.0), 64.0) * 0.14;
        color = mix(u.water.rgb, u.sky_horizon.rgb, 0.18 * fresnel + sheen);
        color = mix(color, u.shoreline.rgb, shore * 0.7);
    }
    if i.kind != 1.0 && i.kind != 2.0 {
        // Soft footprint on both terrain and pavement, fading with aircraft height.
        let center = u.aircraft[3].xz;
        let offset = (i.world.xz - center) / vec2<f32>(5.5, 2.4);
        let shadow = exp(-dot(offset, offset)) * max(0.0, 1.0 - u.aircraft[3].y / 30.0);
        color *= 1.0 - shadow * 0.23;
    }
    if i.kind == 1.0 {
        // Preserve the aircraft and cockpit's existing two-sided lighting.
        let light = 0.62 + 0.38 * abs(dot(normal, normalize(vec3<f32>(0.4, 0.85, 0.3))));
        color *= light;
    }
    // Clear foreground preserves markings and cockpit sight lines. The far terrain converges
    // to the exact sky horizon token before the far clip, hiding the edge of the training field.
    let haze_distance = max(distance - 180.0, 0.0) / 5800.0;
    var fog = 1.0 - exp(-haze_distance * (1.0 + haze_distance * 0.4));
    if i.kind == 1.0 || i.kind == 2.0 {
        fog = 1.0 - exp(-distance / 6800.0);
    }
    if i.kind == 9.0 && fract(i.water_coverage / 300.0) > 0.55 { discard; }
    var alpha = select(1.0, i.color.a * u.eye.w, i.kind == 2.0);
    if i.kind >= 6.0 {
        // Keep chart colours readable at distance, independently of engine power and ground haze.
        color = i.color.rgb;
        fog = 0.0;
        alpha = i.color.a;
    }
    return vec4<f32>(mix(color, u.sky_horizon.rgb, fog), alpha);
}
