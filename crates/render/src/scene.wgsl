// Uniforms, vertex buffers, and sampled textures only: WebGL2 / downlevel compatible.
struct Uniforms {
    view_projection: mat4x4<f32>,
    aircraft: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    ground: vec4<f32>,
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
}
@vertex fn scene_vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>, @location(3) kind: f32) -> Surface {
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
    o.position = u.view_projection * world;
    return o;
}
@fragment fn scene_fragment(i: Surface) -> @location(0) vec4<f32> {
    var color = i.color.rgb;
    if i.kind == 0.0 {
        let cell = i.world.xz / 25.0;
        let derivative = max(fwidth(cell), vec2<f32>(0.0001));
        let grid_distance = abs(fract(cell - 0.5) - 0.5) / derivative;
        let line = 1.0 - min(min(grid_distance.x, grid_distance.y), 1.0);
        let checker = 0.97 + 0.03 * sin(i.world.x * 0.08) * sin(i.world.z * 0.08);
        color = mix(u.ground.rgb * checker, u.grid.rgb, line * 0.65);
    }
    if i.kind != 1.0 && i.kind != 2.0 {
        // Soft footprint on both terrain and pavement, fading with aircraft height.
        let center = u.aircraft[3].xz;
        let offset = (i.world.xz - center) / vec2<f32>(5.5, 2.4);
        let shadow = exp(-dot(offset, offset)) * max(0.0, 1.0 - u.aircraft[3].y / 30.0);
        color *= 1.0 - shadow * 0.23;
    }
    if i.kind == 1.0 {
        let light = 0.62 + 0.38 * abs(dot(normalize(i.normal), normalize(vec3<f32>(0.4, 0.85, 0.3))));
        color *= light;
    }
    let distance = length(i.world - u.eye.xyz);
    let fog = 1.0 - exp(-distance / 6800.0);
    let alpha = select(1.0, i.color.a * u.eye.w, i.kind == 2.0);
    return vec4<f32>(mix(color, u.sky_horizon.rgb, fog), alpha);
}
