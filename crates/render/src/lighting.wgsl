// Original single-scattering approximation, in linear relative solar radiance.
struct Lighting {
    shadow: array<mat4x4<f32>,3>, sun:vec4<f32>, radiance:vec4<f32>, optical:vec4<f32>,
    eye:vec4<f32>, splits:vec4<f32>, settings:vec4<f32>, clouds:array<vec4<f32>,3>,
}
@group(3) @binding(0) var<uniform> light:Lighting;
@group(3) @binding(1) var sun_depth:texture_depth_2d;
@group(3) @binding(2) var sun_sampler:sampler_comparison;
fn air_mass(y:f32)->f32 {return 1.0/max(max(y,0.0)+0.025*exp(-11.0*max(y,0.0)),0.025);}
fn physical_sky(ray:vec3<f32>)->vec3<f32> {
    let mu=clamp(dot(ray,light.sun.xyz),-1.0,1.0);
    let beta=light.optical.rgb;let mie=light.optical.w;
    let transmission=exp(-(beta+vec3<f32>(mie))*air_mass(max(ray.y,0.0)));
    let phase_r=0.75*(1.0+mu*mu);
    let phase_m=(1.0-0.76*0.76)/pow(max(1.0+0.76*0.76-1.52*mu,0.02),1.5);
    let scatter=(beta*phase_r+vec3<f32>(mie*phase_m))/(beta+vec3<f32>(mie));
    let twilight=clamp((light.sun.y+0.12)/0.16,0.0,1.0);
    let lit=exp(-(beta*0.45+vec3<f32>(mie*0.25))*air_mass(max(light.sun.y,0.025)));
    let sky=(vec3<f32>(1.0)-transmission)*scatter*lit*1.2*twilight+vec3<f32>(0.001,0.002,0.005);
    return mix(sky,vec3<f32>(0.055,0.065,0.045)*twilight,clamp(-ray.y*4.0,0.0,1.0));
}
fn display_color(linear:vec3<f32>)->vec3<f32> {
    let x=max(linear*light.settings.x,vec3<f32>(0.0));
    let c=clamp((x*(2.51*x+0.03))/(x*(2.43*x+0.59)+0.14),vec3<f32>(0.0),vec3<f32>(1.0));
    return select(12.92*c,1.055*pow(c,vec3<f32>(1.0/2.4))-0.055,c>vec3<f32>(0.0031308));
}
fn linear_color(color:vec3<f32>)->vec3<f32> {return select(color/12.92,pow((color+0.055)/1.055,vec3<f32>(2.4)),color>vec3<f32>(0.04045));}
fn cascade_shadow(world:vec3<f32>,normal:vec3<f32>,cascade:u32)->f32 {
    let p=light.shadow[cascade]*vec4<f32>(world+normal*0.12,1.0);
    let ndc=p.xyz/p.w;
    if any(abs(ndc.xy)>vec2<f32>(0.995)) || ndc.z<0.0 || ndc.z>1.0 {return 1.0;}
    let corner=vec2<f32>(f32(cascade%2u),f32(cascade/2u));
    let uv=(corner+vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5))*0.5;
    let bias=0.000035+0.00008*(1.0-max(dot(normal,light.sun.xyz),0.0));
    let t=light.settings.w*0.6;
    return 0.25*(textureSampleCompareLevel(sun_depth,sun_sampler,uv+vec2<f32>(-t,-t),ndc.z-bias)
        +textureSampleCompareLevel(sun_depth,sun_sampler,uv+vec2<f32>(t,-t),ndc.z-bias)
        +textureSampleCompareLevel(sun_depth,sun_sampler,uv+vec2<f32>(-t,t),ndc.z-bias)
        +textureSampleCompareLevel(sun_depth,sun_sampler,uv+vec2<f32>(t,t),ndc.z-bias));
}
fn sun_shadow(world:vec3<f32>,normal:vec3<f32>)->f32 {
    if light.settings.y<0.5 || light.splits.w<0.5 || light.sun.y<=0.0 {return 1.0;}
    let distance=length(world-light.eye.xyz);var cascade=0u;
    if distance>light.splits.x {cascade=1u;}if distance>light.splits.y && light.splits.w>2.5 {cascade=2u;}
    let value=cascade_shadow(world,normal,cascade);
    let edge=select(select(light.splits.x,light.splits.y,cascade==1u),light.splits.z,cascade==2u);
    let fade=smoothstep(edge*0.88,edge,distance);
    if fade>0.0 {if f32(cascade+1u)<light.splits.w {return mix(value,cascade_shadow(world,normal,cascade+1u),fade);}return mix(value,1.0,fade);}
    return value;
}
fn surface_light(world:vec3<f32>,normal:vec3<f32>)->vec3<f32> {
    let direct=light.radiance.rgb*max(dot(normal,light.sun.xyz),0.0)*sun_shadow(world,normal)*light.settings.z;
    let ambient=physical_sky(normal)*0.65+physical_sky(vec3<f32>(0.0,1.0,0.0))*0.65+vec3<f32>(0.025);
    return ambient+direct*0.45;
}
fn atmosphere_cloud_depth(ray: vec3<f32>, distance: f32) -> f32 {
    var optical = 0.0;
    for (var layer = 0; layer < 3; layer += 1) {
        let slab = light.clouds[layer];
        if slab.w < 0.5 { continue; }
        let dy = select(ray.y, select(-0.00001, 0.00001, ray.y >= 0.0), abs(ray.y) < 0.00001);
        let a = (slab.x - light.eye.y) / dy;
        let b = (slab.y - light.eye.y) / dy;
        let enter = max(min(a, b), 0.0);
        let exit = min(max(a, b), distance);
        let hit = light.eye.xyz + ray * (enter + max(exit - enter, 0.0) * 0.5);
        let density = 0.5 + 0.25 * sin(-hit.z * 0.0017) + 0.25 * sin(hit.x * 0.0023);
        let coverage = select(1.0 - smoothstep(slab.z - 0.04, slab.z, density), 1.0, slab.z > 0.99);
        let detail = 0.75 + 0.15 * sin(hit.x * 0.019 + sin(hit.z * 0.011)) + 0.10 * sin(hit.z * 0.037);
        optical += max(exit - enter, 0.0) * 0.055 * coverage * detail;
    }
    return optical;
}
fn aerial(color:vec3<f32>,world:vec3<f32>,cloud:f32)->vec3<f32> {
    let delta=world-light.eye.xyz;let distance=length(delta);let ray=delta/max(distance,0.001);
    var transmit=exp(-distance*light.eye.w-cloud);
    if light.radiance.w>1.5 {return physical_sky(normalize(vec3<f32>(0.0,0.05,1.0)));}
    let fog=mix(physical_sky(ray),physical_sky(vec3<f32>(0.0,0.05,1.0)),clamp(1.0-light.settings.z,0.0,1.0));
    return mix(fog,color,transmit);
}
