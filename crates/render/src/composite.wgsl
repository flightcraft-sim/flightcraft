@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
struct Output { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vertex(@builtin(vertex_index) index: u32) -> Output {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var o: Output;
    o.position = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return o;
}
@fragment fn fragment(i: Output) -> @location(0) vec4<f32> {
    return textureSample(scene, scene_sampler, i.uv);
}
