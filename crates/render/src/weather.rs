//! Bounded, texture-free weather uniforms. Synthetic vision uses disabled atmosphere uniforms.
use crate::{Palette, gpu::Uniform};
use flightcraft_world::{Conditions, TimeOfDay};
pub(crate) fn apply(u: &mut Uniform, conditions: &Conditions, palette: Palette) {
    let [east, altitude, back, _] = u.eye;
    let in_cloud = conditions.in_cloud([-f64::from(back), f64::from(east), -f64::from(altitude)]);
    let daylight = match conditions.time_of_day {
        TimeOfDay::Day => 1.,
        TimeOfDay::Dawn | TimeOfDay::Dusk => 0.3,
        TimeOfDay::Night => 0.045,
    };
    u.atmosphere = [
        3.912 / conditions.effective_visibility_m() as f32,
        daylight,
        f32::from(in_cloud),
        0.,
    ];
    let grey = match conditions.obscuration {
        flightcraft_world::Obscuration::None => 0.5,
        flightcraft_world::Obscuration::Haze => 0.55,
        flightcraft_world::Obscuration::Mist => 0.65,
        flightcraft_world::Obscuration::Fog => 0.8,
    };
    for ((out, horizon), marking) in u
        .fog_color
        .iter_mut()
        .zip(palette.sky_horizon)
        .zip(palette.marking)
    {
        *out = (horizon * (1. - grey) + marking * grey) * daylight.max(0.18);
    }
    u.fog_color[3] = 1.;
    for (out, layer) in u.clouds.iter_mut().zip(&conditions.layers) {
        *out = [
            layer.base_ft_msl as f32 * 0.3048,
            layer.top_ft_msl as f32 * 0.3048,
            layer.coverage.fraction() as f32,
            1.,
        ];
    }
}
