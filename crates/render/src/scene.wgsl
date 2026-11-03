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
    vision: vec4<f32>,
    caution: vec4<f32>,
    warning: vec4<f32>,
    atmosphere: vec4<f32>,
    fog_color: vec4<f32>,
    clouds: array<vec4<f32>, 3>,
    light_red: vec4<f32>,
    viewport: vec4<f32>,
    landing_light: vec4<f32>,
    scenery_bounds: vec4<f32>,
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
// Integrate three analytic noise slabs along the actual viewing segment. No 3D texture or loop march.
fn cloud_optical_depth(ray: vec3<f32>, distance: f32) -> f32 {
    var optical = 0.0;
    for (var layer = 0; layer < 3; layer += 1) {
        let slab = u.clouds[layer];
        if slab.w < 0.5 { continue; }
        let dy = select(ray.y, select(-0.00001, 0.00001, ray.y >= 0.0), abs(ray.y) < 0.00001);
        let a = (slab.x - u.eye.y) / dy;
        let b = (slab.y - u.eye.y) / dy;
        let enter = max(min(a, b), 0.0);
        let exit = min(max(a, b), distance);
        let hit = u.eye.xyz + ray * (enter + max(exit - enter, 0.0) * 0.5);
        let density = 0.5 + 0.25 * sin(-hit.z * 0.0017) + 0.25 * sin(hit.x * 0.0023);
        let coverage = select(1.0 - smoothstep(slab.z - 0.04, slab.z, density), 1.0, slab.z > 0.99);
        let detail = 0.75 + 0.15 * sin(hit.x * 0.019 + sin(hit.z * 0.011)) + 0.10 * sin(hit.z * 0.037);
        optical += max(exit - enter, 0.0) * 0.055 * coverage * detail;
    }
    return optical;
}
@fragment fn sky_fragment(i: Sky) -> @location(0) vec4<f32> {
    if u.atmosphere.w > 0.5 || (u.vision.y < 0.5 && u.atmosphere.z > 0.5) { return u.fog_color; }
    let ray = normalize((u.world_from_clip * vec4<f32>(i.uv * 2.0 - 1.0, 1.0, 1.0)).xyz);
    let far_color = select(u.ground, u.sky_top, ray.y >= 0.0);
    var sky = mix(u.sky_horizon, far_color, smoothstep(0.0, 0.65, abs(ray.y)));
    let horizon = (1.0 - smoothstep(0.001, 0.004, abs(ray.y))) * u.vision.y;
    sky = mix(sky, u.grid, horizon);
    if u.vision.y < 0.5 {
        sky = vec4<f32>(sky.rgb * u.atmosphere.y, 1.0);
        let cloud = 1.0 - exp(-cloud_optical_depth(ray, 16000.0));
        let haze = (1.0 - smoothstep(0.0, 0.16, abs(ray.y))) * 0.65;
        sky = mix(sky, u.fog_color, haze);
        // Broad billows and finer relief remain visible even in an optically thick overcast.
        let dy = select(ray.y, select(-0.01, 0.01, ray.y >= 0.0), abs(ray.y) < 0.01);
        let p = u.eye.xyz + ray * clamp((u.clouds[0].x - u.eye.y) / dy, 0.0, 16000.0);
        let relief = 0.86 + 0.08 * sin(p.x * 0.008 + sin(p.z * 0.006)) + 0.06 * sin(p.z * 0.019);
        sky = mix(sky, vec4<f32>(u.fog_color.rgb * mix(0.9, relief, smoothstep(0.025, 0.12, abs(ray.y))), 1.0), cloud);
        if u.atmosphere.z > 0.5 { return u.fog_color; }
    }
    return sky;
}
struct Surface {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) kind: f32,
    @location(4) water_coverage: f32,
    @location(5) light_angle: f32,
    @location(6) light_facing: f32,
}
@vertex fn scene_vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>, @location(3) kind: f32, @location(4) water_coverage: f32) -> Surface {
    var o: Surface;
    var world = vec4<f32>(position, 1.0);
    var n = normal;
    if kind == 1.0 || kind == 2.0 || kind == -1.0 || kind == -2.0 || kind == -3.0 {
        world = u.aircraft * world;
        n = (u.aircraft * vec4<f32>(normal, 0.0)).xyz;
    }
    o.light_angle = 0.0;
    o.light_facing = 0.0;
    o.world = world.xyz;
    o.normal = n;
    o.color = color;
    o.kind = kind;
    o.water_coverage = water_coverage;
    o.position = u.view_projection * world;
    if kind == -2.0 {
        // Aircraft lamps retain a luminous footprint when their physical lens is subpixel.
        let projected = (u.view_projection * vec4<f32>(n, 0.0)).xy;
        let direction = projected / max(length(projected), 0.0001);
        o.position.x += direction.x * 2.0 / u.viewport.x * o.position.w;
        o.position.y += direction.y * 2.0 / u.viewport.y * o.position.w;
    }
    if kind >= 11.0 {
        // Subpixel fixtures retain a small luminous footprint, with geometric depth occlusion.
        let pixels = clamp(700.0 / max(o.position.w, 1.0), 1.1, 5.0);
        o.position.x += normal.x * pixels * 2.0 / u.viewport.x * o.position.w;
        o.position.y += normal.y * pixels * 2.0 / u.viewport.y * o.position.w;
        let delta = u.eye.xyz - position;
        o.light_facing = dot(delta.xz, vec2<f32>(sin(normal.z), cos(normal.z)));
        o.light_angle = atan2(delta.y, length(delta.xz));
        o.normal = vec3<f32>(0.0, 1.0, 0.0);
    }
    if kind == 10.0 {
        // Expand outline edges in clip space to retain a legible width at PFD resolution.
        let next = u.view_projection * vec4<f32>(position + normal, 1.0);
        let delta = (next.xy / max(abs(next.w), 0.001) - o.position.xy / max(abs(o.position.w), 0.001)) * vec2<f32>(750.0, 285.0);
        let side = vec2<f32>(-delta.y, delta.x) / max(length(delta), 0.00001);
        o.position.x += side.x * 2.0 / 750.0 * o.position.w * water_coverage * 0.9;
        o.position.y += side.y * 2.0 / 285.0 * o.position.w * water_coverage * 0.9;
        o.normal = vec3<f32>(0.0, 1.0, 0.0);
    }
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
    if i.kind == 3.0 && u.vision.y < 0.5 && u.scenery_bounds.z > u.scenery_bounds.x && all(i.world.xz >= u.scenery_bounds.xy) && all(i.world.xz <= u.scenery_bounds.zw) { discard; }
    let distance = length(i.world - u.eye.xyz);
    let normal = normalize(i.normal);
    let sun = normalize(SUN);
    // Derivatives are evaluated outside material branches for downlevel shader validation.
    let fine = detail_lines(i.world.xz, 12.5);
    let coarse = detail_lines(i.world.xz, 100.0);
    let shore = 1.0 - smoothstep(0.0, 0.18, abs(i.water_coverage - 0.5));
    if i.kind == -2.0 {
        if u.atmosphere.z > 0.5 || u.atmosphere.w > 0.5 { return u.fog_color; }
        let cloud = cloud_optical_depth(normalize(i.world - u.eye.xyz), distance);
        return vec4<f32>(u.fog_color.rgb + i.color.rgb * exp(-distance * u.atmosphere.x * 0.45 - cloud), 1.0);
    }
    if (i.kind == 12.0 || i.kind == 14.0) && i.light_facing > 0.0 { discard; }
    if u.vision.y < 0.5 && i.kind != 1.0 && i.kind != 2.0 && (u.atmosphere.w > 0.5 || u.atmosphere.z > 0.5) { return u.fog_color; }
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
    if i.kind != 1.0 && i.kind != 2.0 && i.kind != -1.0 {
        // Soft footprint on both terrain and pavement, fading with aircraft height.
        let center = u.aircraft[3].xz;
        let offset = (i.world.xz - center) / vec2<f32>(5.5, 2.4);
        let shadow = exp(-dot(offset, offset)) * max(0.0, 1.0 - u.viewport.z / 30.0);
        color *= 1.0 - shadow * 0.23;
    }
    if i.kind == 1.0 || i.kind == -1.0 {
        // Preserve the aircraft and cockpit's existing two-sided lighting.
        let light = 0.62 + 0.38 * abs(dot(normal, normalize(vec3<f32>(0.4, 0.85, 0.3))));
        color *= light * select(max(0.12, u.atmosphere.y), 1.0, u.vision.y > 0.5);
    }
    // Clear foreground preserves markings and cockpit sight lines. The far terrain converges
    // to the exact sky horizon token before the far clip, hiding the edge of the training field.
    let visibility = select(5800.0, 24000.0, u.vision.y > 0.5);
    let haze_distance = max(distance - 180.0, 0.0) / visibility;
    var fog = 1.0 - exp(-haze_distance * (1.0 + haze_distance * 0.4));
    if i.kind == 1.0 || i.kind == 2.0 {
        fog = 1.0 - exp(-distance / 6800.0);
    }
    if u.vision.y > 0.5 && u.vision.z > 0.5 && (i.kind == 0.0 || i.kind == 3.0 || i.kind == 4.0 || i.kind == 5.0) {
        let clearance = u.vision.x - i.world.y;
        let relief = 0.72 + 0.28 * max(dot(normal, sun), 0.0);
        if clearance <= 304.8 { color = u.caution.rgb * relief; }
        if clearance <= 30.48 { color = u.warning.rgb * relief; }
        // Awareness bands stay readable in distant haze, as on the map.
        if clearance <= 304.8 { fog = 0.0; }
    }
    if i.kind == 9.0 && fract(i.water_coverage / 300.0) > 0.55 { discard; }
    var alpha = select(1.0, i.color.a * u.eye.w, i.kind == 2.0);
    if i.kind == -3.0 { alpha = i.color.a; }
    if i.kind >= 6.0 {
        // Keep chart colours readable at distance, independently of engine power and ground haze.
        color = i.color.rgb;
        fog = 0.0;
        alpha = i.color.a;
    }
    if u.vision.y < 0.5 {
        let cockpit = i.kind == 1.0 || i.kind == 2.0;
        if !cockpit {
            if u.atmosphere.w > 0.5 { return u.fog_color; }
            color *= u.atmosphere.y;
            if u.landing_light.w > 0.5 && (i.kind == 0.0 || i.kind == 3.0 || i.kind == 4.0) {
                let delta = i.world - u.aircraft[3].xyz;
                let beam = smoothstep(0.94, 0.995, dot(normalize(delta), u.landing_light.xyz));
                let falloff = 1.0 / (1.0 + dot(delta, delta) * 0.0025);
                color += i.color.rgb * vec3<f32>(1.0, 0.94, 0.78) * beam * falloff * (1.0 - u.atmosphere.y);
            }
            let cloud = cloud_optical_depth(normalize(i.world - u.eye.xyz), distance);
            var transmission = exp(-distance * u.atmosphere.x - cloud);
            if u.atmosphere.z > 0.5 { transmission = 0.0; }
            if i.kind >= 11.0 {
                // Emissive lights penetrate haze farther than unlit pavement; cloud remains opaque.
                transmission = exp(-distance * u.atmosphere.x * 0.45 - cloud);
                if u.atmosphere.z > 0.5 { transmission = 0.0; }
                color = i.color.rgb;
                if i.kind == 12.0 {
                    color = select(u.light_red.rgb, i.color.rgb, i.light_angle > i.water_coverage);
                }
                if i.kind == 13.0 { color = select(i.color.rgb, u.light_red.rgb, i.light_facing > 0.0); }
                // Add fixture radiance to the scattered background. Mixing with fog would make
                // white lights darker than a bright daytime overcast.
                let range_gain = min(1.0, 9000000.0 / max(distance * distance, 1.0));
                let intensity = (1.5 + 1.5 * (1.0 - u.atmosphere.y)) * range_gain;
                return vec4<f32>(u.fog_color.rgb + color * transmission * intensity, alpha);
            }
            return vec4<f32>(mix(u.fog_color.rgb, color, transmission), alpha);
        }
    }
    return vec4<f32>(mix(color, u.sky_horizon.rgb, fog), alpha);
}
