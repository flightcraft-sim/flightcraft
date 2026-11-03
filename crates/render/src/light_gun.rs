//! Read-only tower light-gun billboard, projected from its world position.
use crate::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightGunPattern {
    SteadyGreen,
    FlashingGreen,
    SteadyRed,
    FlashingRed,
    FlashingWhite,
    AlternatingRedGreen,
}
#[derive(Clone, Copy, Debug)]
pub struct LightGun {
    pub position_m: [f64; 3],
    pub pattern: LightGunPattern,
    pub elapsed_s: f64,
}
impl LightGun {
    pub fn color(self, p: Palette) -> Option<[f32; 4]> {
        if !self.elapsed_s.is_finite() || !self.position_m.iter().all(|v| v.is_finite()) {
            return None;
        }
        let phase = (self.elapsed_s.max(0.) * 2.).floor() as u64;
        match self.pattern {
            LightGunPattern::SteadyGreen => Some(p.light_green),
            LightGunPattern::SteadyRed => Some(p.light_red),
            LightGunPattern::AlternatingRedGreen => Some(if phase.is_multiple_of(2) {
                p.light_red
            } else {
                p.light_green
            }),
            LightGunPattern::FlashingGreen
            | LightGunPattern::FlashingRed
            | LightGunPattern::FlashingWhite => {
                if phase.is_multiple_of(2) {
                    Some(match self.pattern {
                        LightGunPattern::FlashingGreen => p.light_green,
                        LightGunPattern::FlashingRed => p.light_red,
                        _ => p.marking,
                    })
                } else {
                    None
                }
            }
        }
    }
}
impl Scene {
    pub fn set_light_gun(&mut self, light: Option<LightGun>) {
        self.light_gun = light;
    }
    pub(crate) fn draw_light_gun(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        eye: Vec3,
        matrix: Mat4,
        palette: Palette,
    ) {
        if self.hood
            || self
                .conditions
                .in_cloud([-f64::from(eye.z), f64::from(eye.x), -f64::from(eye.y)])
        {
            return;
        }
        let Some(light) = self.light_gun else {
            return;
        };
        let Some([r, g, b, a]) = light.color(palette) else {
            return;
        };
        let [n, e, d] = light.position_m.map(|v| v as f32);
        let position = drawing(Vec3::new(n, e, d));
        let distance = eye.distance(position);
        if distance > 14816. || distance > self.conditions.effective_visibility_m() as f32 {
            return;
        }
        for i in 1..64 {
            let v = eye.lerp(position, i as f32 / 64.);
            if self.world.ground(-f64::from(v.z), f64::from(v.x)).height_msl_m > f64::from(v.y) {
                return;
            }
        }
        let clip = matrix * position.extend(1.);
        if clip.w <= 0. {
            return;
        }
        let ndc = clip.truncate() / clip.w;
        if ndc.x.abs() > 1. || ndc.y.abs() > 1. || !(0. ..=1.).contains(&ndc.z) {
            return;
        }
        let center = egui::pos2(
            rect.center().x + ndc.x * rect.width() / 2.,
            rect.center().y - ndc.y * rect.height() / 2.,
        );
        let color = egui::Rgba::from_rgba_unmultiplied(r, g, b, a);
        ui.painter().circle_filled(center, 5., egui::Color32::from(color));
    }
}
