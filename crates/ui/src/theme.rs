//! All interface and scene colour tokens live here.
use eframe::egui::{self, Color32};
use flightcraft_render::Palette;

pub const BACKGROUND: Color32 = Color32::from_rgb(12, 19, 28);
pub const PANEL: Color32 = Color32::from_rgb(19, 29, 40);
pub const RAISED: Color32 = Color32::from_rgb(29, 43, 57);
pub const BORDER: Color32 = Color32::from_rgb(49, 66, 81);
pub const TEXT: Color32 = Color32::from_rgb(228, 237, 242);
pub const MUTED: Color32 = Color32::from_rgb(143, 163, 179);
pub const ACCENT: Color32 = Color32::from_rgb(91, 212, 195);
pub const GREEN: Color32 = Color32::from_rgb(110, 211, 132);
pub const AMBER: Color32 = Color32::from_rgb(247, 194, 97);
pub const DANGER: Color32 = Color32::from_rgb(242, 103, 103);
pub const DIAL: Color32 = Color32::from_rgb(10, 18, 26);
pub const SKY: Color32 = Color32::from_rgb(64, 126, 167);
pub const EARTH: Color32 = Color32::from_rgb(126, 94, 64);
pub const GLASS_GPS: Color32 = Color32::from_rgb(224, 116, 218);
pub const AIRSPACE_BLUE: Color32 = Color32::from_rgb(75, 143, 255);
pub const TERRAIN_HIGH: Color32 = Color32::from_rgb(71, 91, 67);
pub const TERRAIN_LOW: Color32 = Color32::from_rgb(32, 64, 56);
pub const CLEAR: Color32 = Color32::TRANSPARENT;

pub fn install(ctx: &egui::Context) {
    // Bundle only an Apache-2.0 font, without egui's differently licensed default font set.
    let mut fonts = egui::FontDefinitions::empty();
    fonts.font_data.insert(
        "Droid Sans".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/DroidSans.ttf")).into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.insert(family, vec!["Droid Sans".to_owned()]);
    }
    ctx.set_fonts(fonts);
    // The palette is dark only: never follow the system or browser light theme, or egui falls back
    // to its default light widgets beside our panels.
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BACKGROUND;
    v.faint_bg_color = RAISED;
    v.selection.bg_fill = RAISED;
    v.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, MUTED);
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.bg_fill = BORDER;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.bg_fill = BORDER;
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 7.0);
    });
}

pub fn scene() -> Palette {
    Palette {
        airspace_b: [0.12, 0.40, 1.0, 0.82],
        airspace_c: [0.85, 0.12, 0.68, 0.82],
        airspace_d: [0.12, 0.40, 1.0, 0.70],
        airspace_e: [0.85, 0.12, 0.68, 0.70],
        airspace_mode_c: [0.65, 0.68, 0.71, 0.22],
        sky_top: [0.24, 0.48, 0.68, 1.0],
        sky_horizon: [0.76, 0.84, 0.85, 1.0],
        ground: [0.43, 0.53, 0.34, 1.0],
        earth: [0.49, 0.37, 0.25, 1.0],
        dry_grass: [0.66, 0.60, 0.39, 1.0],
        rock: [0.53, 0.55, 0.51, 1.0],
        forest: [0.23, 0.37, 0.29, 1.0],
        shoreline: [0.72, 0.70, 0.51, 1.0],
        water: [0.12, 0.38, 0.47, 1.0],
        grid: [0.66, 0.66, 0.46, 1.0],
        runway: [0.23, 0.27, 0.29, 1.0],
        marking: [0.91, 0.91, 0.82, 1.0],
        aircraft: [0.87, 0.91, 0.90, 1.0],
        accent: [0.23, 0.72, 0.66, 1.0],
        glass: [0.12, 0.24, 0.32, 1.0],
        rubber: [0.08, 0.11, 0.13, 1.0],
        shadow: [0.19, 0.25, 0.22, 1.0],
        cockpit_frame: [0.20, 0.27, 0.30, 1.0],
        glare_shield: [0.035, 0.055, 0.07, 1.0],
        propeller: [0.68, 0.75, 0.76, 0.16],
    }
}
