// The same weather uniforms as the main scene. glTF material data stays on the GPU.
struct Uniforms {
    view_projection: mat4x4<f32>, aircraft: mat4x4<f32>, world_from_clip: mat4x4<f32>,
    sky_top: vec4<f32>, sky_horizon: vec4<f32>, ground: vec4<f32>, dry_grass: vec4<f32>,
    rock: vec4<f32>, forest: vec4<f32>, shoreline: vec4<f32>, water: vec4<f32>, grid: vec4<f32>,
    eye: vec4<f32>, vision: vec4<f32>, caution: vec4<f32>, warning: vec4<f32>,
    atmosphere: vec4<f32>, fog_color: vec4<f32>, clouds: array<vec4<f32>,3>, light_red: vec4<f32>,
    viewport: vec4<f32>, landing_light: vec4<f32>, scenery_bounds: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var albedo: texture_2d<f32>;
@group(1) @binding(1) var texture_sampler: sampler;
struct Surface {
    @builtin(position) position:vec4<f32>, @location(0) world:vec3<f32>,
    @location(1) normal:vec3<f32>, @location(2) uv:vec2<f32>, @location(3) color:vec4<f32>,
}
@vertex fn vertex(@location(0) position:vec3<f32>, @location(1) normal:vec3<f32>, @location(2) uv:vec2<f32>, @location(3) color:vec4<f32>, @location(4) c0:vec4<f32>, @location(5) c1:vec4<f32>, @location(6) c2:vec4<f32>, @location(7) c3:vec4<f32>)->Surface {
    var o:Surface;
    let model=mat4x4<f32>(c0,c1,c2,c3);
    o.world=(model*vec4<f32>(position,1.0)).xyz;
    o.position=u.view_projection*vec4<f32>(o.world,1.0);
    // Cofactor normal transform supports nonuniform measured crown/height scales.
    o.normal=normalize(cross(c1.xyz,c2.xyz)*normal.x+cross(c2.xyz,c0.xyz)*normal.y+cross(c0.xyz,c1.xyz)*normal.z);o.uv=uv;o.color=color;
    return o;
}
// The same analytic cloud integration as the main scene, evaluated along this surface ray.
fn cloud_optical_depth(ray:vec3<f32>,distance:f32)->f32 {
    var optical=0.0;
    for (var layer=0;layer<3;layer+=1) {
        let slab=u.clouds[layer];if slab.w<0.5 {continue;}
        let dy=select(ray.y,select(-0.00001,0.00001,ray.y>=0.0),abs(ray.y)<0.00001);
        let a=(slab.x-u.eye.y)/dy;let b=(slab.y-u.eye.y)/dy;
        let enter=max(min(a,b),0.0);let exit=min(max(a,b),distance);
        let hit=u.eye.xyz+ray*(enter+max(exit-enter,0.0)*0.5);
        let density=0.5+0.25*sin(-hit.z*0.0017)+0.25*sin(hit.x*0.0023);
        let coverage=select(1.0-smoothstep(slab.z-0.04,slab.z,density),1.0,slab.z>0.99);
        let detail=0.75+0.15*sin(hit.x*0.019+sin(hit.z*0.011))+0.10*sin(hit.z*0.037);
        optical+=max(exit-enter,0.0)*0.055*coverage*detail;
    }
    return optical;
}
@fragment fn fragment(i:Surface)->@location(0) vec4<f32> {
    let albedo_color=textureSample(albedo,texture_sampler,i.uv)*i.color;
    if albedo_color.a<0.5 {discard;}
    if u.atmosphere.z>0.5 || u.atmosphere.w>0.5 {return u.fog_color;}
    let distance=length(i.world-u.eye.xyz);
    let sun=normalize(vec3<f32>(-0.58,0.70,-0.42));
    let normal=normalize(i.normal);
    // Conservative ambient fill while the reference imagery still contains baked lighting.
    var color=albedo_color.rgb*(0.65+0.35*max(dot(normal,sun),0.0))*u.atmosphere.y;
    let center=u.aircraft[3].xz;
    let offset=(i.world.xz-center)/vec2<f32>(5.5,2.4);
    let shadow=exp(-dot(offset,offset))*max(0.0,1.0-u.viewport.z/30.0);
    color*=1.0-shadow*0.23;
    let cloud=cloud_optical_depth(normalize(i.world-u.eye.xyz),distance);
    let transmission=exp(-distance*u.atmosphere.x-cloud);
    return vec4<f32>(mix(u.fog_color.rgb,color,transmission),1.0);
}
