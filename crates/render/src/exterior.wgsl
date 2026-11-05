// Exterior-only PBR. No terrain/sky resources or simulation state are modified here.
struct Frame {
    view_projection: mat4x4<f32>,
    world_from_body: mat4x4<f32>,
    eye: vec4<f32>,
    sun_direction: vec4<f32>, // xyz points toward sun; w is direct-light visibility [0,1].
    sun_radiance: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    ground_radiance: vec4<f32>,
    environment: vec4<f32>, // x probe enabled, y maximum mip, z exposure, w ambient strength.
}
struct Material {
    base_color: vec4<f32>,
    emissive_normal: vec4<f32>,
    surface: vec4<f32>, // metallic, roughness, occlusion strength, alpha cutoff.
    finish: vec4<f32>, // alpha mode (0 opaque, 1 mask, 2 blend), double-sided, coat, coat roughness.
}
@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_map: texture_2d<f32>;
@group(1) @binding(2) var mr_map: texture_2d<f32>;
@group(1) @binding(3) var normal_map: texture_2d<f32>;
@group(1) @binding(4) var ao_map: texture_2d<f32>;
@group(1) @binding(5) var emissive_map: texture_2d<f32>;
@group(1) @binding(6) var surface_sampler: sampler;
// A scenery-owned, linear-radiance cubemap with roughness-prefiltered mip levels.
@group(2) @binding(0) var sky_probe: texture_cube<f32>;
@group(2) @binding(1) var probe_sampler: sampler;
struct Surface {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) emissive_scale: f32,
}
@vertex fn vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>,
                  @location(2) tangent: vec4<f32>, @location(3) uv: vec2<f32>,
                  @location(4) color: vec4<f32>, @location(5) emissive_scale: f32) -> Surface {
    var o: Surface;
    o.world = (frame.world_from_body * vec4<f32>(position, 1.0)).xyz;
    o.position = frame.view_projection * vec4<f32>(o.world, 1.0);
    // Host applies inverse-transpose node normals before upload; body-to-world is rigid.
    o.normal = normalize((frame.world_from_body * vec4<f32>(normal, 0.0)).xyz);
    o.tangent = vec4<f32>(normalize((frame.world_from_body * vec4<f32>(tangent.xyz, 0.0)).xyz), tangent.w);
    o.tangent.w *= sign(determinant(mat3x3<f32>(frame.world_from_body[0].xyz,frame.world_from_body[1].xyz,frame.world_from_body[2].xyz)));
    o.uv = uv; o.color = color; o.emissive_scale = emissive_scale;
    return o;
}
const PI: f32 = 3.14159265359;
// sin of the largest tangent-space normal tilt (about 20 degrees) the baked maps may apply.
const MAX_NORMAL_TILT: f32 = 0.35;
fn fresnel(f0: vec3<f32>, cosine: f32) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0-cosine, 0.0, 1.0), 5.0);
}
fn distribution(no_h: f32, roughness: f32) -> f32 {
    let a2 = pow(roughness, 4.0);
    let d = no_h*no_h*(a2-1.0)+1.0;
    return a2 / max(PI*d*d, 0.0000001);
}
fn visibility(no_v: f32, no_l: f32, roughness: f32) -> f32 {
    // Height-correlated Smith GGX, including the 1/(4 NoV NoL) denominator.
    let a2 = pow(roughness, 4.0);
    let gv = no_l*sqrt(max(no_v*no_v*(1.0-a2)+a2, 0.0));
    let gl = no_v*sqrt(max(no_l*no_l*(1.0-a2)+a2, 0.0));
    return 0.5/max(gv+gl, 0.00001);
}
fn procedural_environment(direction: vec3<f32>, roughness: f32) -> vec3<f32> {
    let y = direction.y*(1.0-0.65*roughness);
    return mix(frame.sky_horizon.rgb, select(frame.ground_radiance.rgb, frame.sky_zenith.rgb, y>=0.0),
               smoothstep(0.0, 0.8, abs(y)));
}
fn environment(direction: vec3<f32>, roughness: f32) -> vec3<f32> {
    if frame.environment.x > 0.5 {
        return textureSampleLevel(sky_probe, probe_sampler, direction, roughness*frame.environment.y).rgb;
    }
    return procedural_environment(direction, roughness);
}
// Analytic split-sum BRDF integration approximation; no LUT/compute/storage required on WebGL2.
fn environment_brdf(no_v: f32, roughness: f32) -> vec2<f32> {
    let r = roughness*vec4<f32>(-1.0,-0.0275,-0.572,0.022)+vec4<f32>(1.0,0.0425,1.04,-0.04);
    let a004 = min(r.x*r.x, exp2(-9.28*no_v))*r.x+r.y;
    return vec2<f32>(-1.04,1.04)*a004+r.zw;
}
fn tone_map(linear: vec3<f32>) -> vec3<f32> {
    let x = max(linear*frame.environment.z, vec3<f32>(0.0));
    let mapped = clamp((x*(2.51*x+0.03))/(x*(2.43*x+0.59)+0.14), vec3<f32>(0.0), vec3<f32>(1.0));
    // The scenery pass uses an unorm color target, so encode the display transfer here.
    return select(12.92*mapped, 1.055*pow(mapped,vec3<f32>(1.0/2.4))-0.055, mapped>vec3<f32>(0.0031308));
}
fn finish_lighting(color:vec3<f32>,world:vec3<f32>)->vec3<f32> {
    if light.settings.y>0.5 {return display_color(aerial(color,world,atmosphere_cloud_depth(normalize(world-light.eye.xyz),length(world-light.eye.xyz))));}
    return tone_map(color);
}
@fragment fn fragment(i: Surface, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // All derivative-using samples precede alpha discard and nonuniform branches.
    let base = textureSample(base_map, surface_sampler, i.uv)*material.base_color*i.color;
    let mr = textureSample(mr_map, surface_sampler, i.uv);
    let normal_texel = textureSample(normal_map, surface_sampler, i.uv).xyz*2.0-1.0;
    let ao_texel = textureSample(ao_map, surface_sampler, i.uv).r;
    let emissive = textureSample(emissive_map, surface_sampler, i.uv).rgb*material.emissive_normal.rgb*i.emissive_scale;
    if material.finish.x > 0.5 && material.finish.x < 1.5 && base.a < material.surface.w { discard; }
    if !front && material.finish.y < 0.5 { discard; }
    let face_sign = select(-1.0, 1.0, front);
    let geometric_normal = normalize(i.normal)*face_sign;
    let tangent = normalize(i.tangent.xyz-geometric_normal*dot(geometric_normal,i.tangent.xyz));
    let bitangent = cross(geometric_normal,tangent)*i.tangent.w;
    // Baked tangent normals are only trusted for shallow relief. A texel baked across a rim,
    // seam or the far side of a thin shell tilts far past any real surface detail, and lighting it
    // draws sawtooth white glints along the triangles that sample it; the tilt is capped.
    var tilt = normal_texel.xy*material.emissive_normal.w;
    let tilt_length = length(tilt);
    if tilt_length > MAX_NORMAL_TILT { tilt *= MAX_NORMAL_TILT/tilt_length; }
    let mapped = vec3<f32>(tilt, sqrt(max(1.0-dot(tilt,tilt), 0.0)));
    let n = normalize(mat3x3<f32>(tangent,bitangent,geometric_normal)*mapped);
    let v = normalize(frame.eye.xyz-i.world);
    let l = normalize(frame.sun_direction.xyz);
    let h = normalize(v+l+vec3<f32>(0.000001));
    let no_v = max(dot(n,v),0.0001); let no_l = max(dot(n,l),0.0);
    let no_h = max(dot(n,h),0.0); let vo_h = max(dot(v,h),0.0);
    let metallic = clamp(mr.b*material.surface.x,0.0,1.0);
    let roughness = clamp(mr.g*material.surface.y,0.045,1.0);
    let f0 = mix(vec3<f32>(0.04),base.rgb,metallic);
    let f = fresnel(f0,vo_h);
    let diffuse = base.rgb*(1.0-metallic)/PI;
    let specular = distribution(no_h,roughness)*visibility(no_v,no_l,roughness)*f;
    let coat_roughness = clamp(material.finish.w,0.045,1.0);
    let coat_f = fresnel(vec3<f32>(0.04),vo_h).r*material.finish.z;
    let coat = coat_f*distribution(no_h,coat_roughness)*visibility(no_v,no_l,coat_roughness);
    var direct = ((vec3<f32>(1.0)-f)*diffuse+specular)*(1.0-coat_f)+vec3<f32>(coat);
    direct *= frame.sun_radiance.rgb*no_l*frame.sun_direction.w*sun_shadow(i.world,n);
    let ao = mix(1.0,ao_texel,material.surface.z);
    let reflected = reflect(-v,n);
    let brdf = environment_brdf(no_v,roughness);
    let diffuse_ibl = environment(n,1.0)*base.rgb*(1.0-metallic)*(vec3<f32>(1.0)-fresnel(f0,no_v));
    let specular_ibl = environment(reflected,roughness)*(f0*brdf.x+brdf.y);
    let coat_ibl = environment(reflected,coat_roughness)*fresnel(vec3<f32>(0.04),no_v)*material.finish.z;
    var ambient = (diffuse_ibl+specular_ibl)*(1.0-material.finish.z*0.04)+coat_ibl;
    ambient *= ao*frame.environment.w;
    // Straight alpha lets the already rendered interior show through. Fresnel reflection grows
    // toward grazing angles; the shell uses depth testing, no depth writes, and sorted triangles.
    var alpha = 1.0;
    if material.finish.x > 1.5 {
        let grazing = fresnel(vec3<f32>(0.04),no_v).r;
        alpha = clamp(base.a+(1.0-base.a)*grazing,0.0,1.0);
        // Preserve reflected energy under straight-alpha blending, avoiding alpha-squared glass.
        let reflected_light = (specular_ibl+coat_ibl)*frame.environment.w;
        return vec4<f32>(finish_lighting(direct+ambient+emissive+reflected_light*(1.0/ max(alpha,0.04)-1.0),i.world),alpha);
    }
    return vec4<f32>(finish_lighting(direct+ambient+emissive,i.world),alpha);
}
