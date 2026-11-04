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
    let center=textureSample(scene,scene_sampler,i.uv);
    let texel=vec2<f32>(1.0)/vec2<f32>(textureDimensions(scene));
    let a=textureSample(scene,scene_sampler,i.uv+vec2<f32>(4.0,0.0)*texel).rgb;
    let b=textureSample(scene,scene_sampler,i.uv-vec2<f32>(4.0,0.0)*texel).rgb;
    let c=textureSample(scene,scene_sampler,i.uv+vec2<f32>(0.0,4.0)*texel).rgb;
    let d=textureSample(scene,scene_sampler,i.uv-vec2<f32>(0.0,4.0)*texel).rgb;
    let bloom=max(a-vec3<f32>(0.985),vec3<f32>(0.0))+max(b-vec3<f32>(0.985),vec3<f32>(0.0))+max(c-vec3<f32>(0.985),vec3<f32>(0.0))+max(d-vec3<f32>(0.985),vec3<f32>(0.0));
    return vec4<f32>(min(center.rgb+bloom*2.0,vec3<f32>(1.0)),center.a);
}
