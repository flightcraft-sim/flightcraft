struct Frame {matrix:mat4x4<f32>,aircraft:mat4x4<f32>,bounds:vec4<f32>}
@group(0) @binding(0) var<uniform> f:Frame;
@group(1) @binding(0) var albedo:texture_2d<f32>;
@group(1) @binding(1) var tex_sampler:sampler;
struct Output {@builtin(position) position:vec4<f32>,@location(0) world:vec3<f32>,@location(1) uv:vec2<f32>,@location(2) @interpolate(flat) kind:f32}
@vertex fn mesh_vertex(@location(0) position:vec3<f32>,@location(3) kind:f32)->Output {
    var p=vec4<f32>(position,1.0);if kind==1.0 || kind==-1.0 {p=f.aircraft*p;}
    var o:Output;o.position=f.matrix*p;o.world=p.xyz;o.uv=vec2<f32>(0.0);o.kind=kind;return o;
}
@vertex fn instance_vertex(@location(0) position:vec3<f32>,@location(2) uv:vec2<f32>,@location(4) c0:vec4<f32>,@location(5) c1:vec4<f32>,@location(6) c2:vec4<f32>,@location(7) c3:vec4<f32>)->Output {
    let p=mat4x4<f32>(c0,c1,c2,c3)*vec4<f32>(position,1.0);var o:Output;o.position=f.matrix*p;o.world=p.xyz;o.uv=uv;o.kind=0.0;return o;
}
@fragment fn mesh_fragment(i:Output) {
    if i.kind>=6.0 || i.kind==2.0 || (i.kind<0.0 && i.kind!=-1.0) {discard;}
    if i.kind!=1.0 && i.kind!=-1.0 && f.bounds.z>f.bounds.x && all(i.world.xz>=f.bounds.xy) && all(i.world.xz<=f.bounds.zw) {discard;}
}
@fragment fn instance_fragment(i:Output) {if textureSample(albedo,tex_sampler,i.uv).a<0.5 {discard;}}
