//! Bezel, soft chrome, and menus for the landscape touch EFIS display model.
//!
//! Everything here draws the model's controls and sends `DisplayAction`s; the behaviour (what a key
//! or knob does) lives in `flightcraft_control::touch_efis`. A skin pack only changes `Style`.
use eframe::egui::{self, Align2, Color32, FontFamily, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use flightcraft_aero::display_model::{BezelControl, DisplayModel};
use flightcraft_aero::modern::MfdPage;
use flightcraft_aero::panel_fidelity::SrgbHex;
use flightcraft_control::touch_efis::{EfisKey, Knob, Overlay, Ring, ScreenMode};
use flightcraft_control::{DisplayAction, EquipmentSetting, GlassSetting, SkinContents, TouchDisplayState};

/// Fraction of the screen width the PFD takes in split mode.
pub(crate) const SPLIT_PFD_FRACTION: f32 = 0.60;
/// Height of the MFD page bar as a fraction of the screen height.
pub(crate) const PAGE_BAR_FRACTION: f32 = 0.07;
/// A press at least this long on BACK is a press-and-hold.
const HOLD_S: f64 = 0.6;
/// Horizontal drag distance, in points, for one knob detent.
const DETENT_PX: f32 = 14.0;

/// Colours, labels, and font family for the bezel. `built_in` is the engine's own public skin.
#[derive(Clone)]
pub(crate) struct Style {
    pub bezel: Color32,
    pub trim: Color32,
    pub key_face: Color32,
    pub key_text: Color32,
    pub knob: Color32,
    pub knob_mark: Color32,
    pub surround: Color32,
    pub family: FontFamily,
    labels: Vec<(BezelControl, String)>,
}

impl Style {
    pub fn built_in() -> Self {
        Self {
            bezel: crate::theme::BEZEL_BODY,
            trim: crate::theme::BEZEL_TRIM,
            key_face: crate::theme::KEY_FACE,
            key_text: crate::theme::TEXT,
            knob: crate::theme::KNOB,
            knob_mark: crate::theme::KNOB_MARK,
            surround: Color32::BLACK,
            family: FontFamily::Proportional,
            labels: Vec::new(),
        }
    }

    /// The skin's colours and labels over the built-in values. The skin font is registered with
    /// [`install_font`] and used only if it loaded.
    pub fn from_skin(ctx: &egui::Context, skin: &SkinContents) -> Self {
        let mut s = Self::built_in();
        let p = &skin.manifest.palette;
        let c = |v: &Option<SrgbHex>, fallback: Color32| {
            v.as_ref()
                .and_then(SrgbHex::rgb)
                .map_or(fallback, |[r, g, b]| Color32::from_rgb(r, g, b))
        };
        s.bezel = c(&p.bezel, s.bezel);
        s.trim = c(&p.bezel_trim, s.trim);
        s.key_face = c(&p.key_face, s.key_face);
        s.key_text = c(&p.key_text, s.key_text);
        s.knob = c(&p.knob, s.knob);
        s.knob_mark = c(&p.knob_mark, s.knob_mark);
        s.surround = c(&p.screen_surround, s.surround);
        s.labels = BezelControl::ALL
            .into_iter()
            .map(|k| (k, skin.manifest.label(k).to_owned()))
            .collect();
        if let Some(family) = install_font(ctx, skin) {
            s.family = family;
        }
        s
    }

    fn label(&self, control: BezelControl) -> &str {
        self.labels
            .iter()
            .find(|(k, _)| *k == control)
            .map_or_else(|| control.label(), |(_, l)| l.as_str())
    }

    fn font(&self, size: f32) -> FontId {
        FontId::new(size.max(5.), self.family.clone())
    }
}

/// Register a skin's chrome font once. A font that does not parse is ignored.
fn install_font(ctx: &egui::Context, skin: &SkinContents) -> Option<FontFamily> {
    let bytes = skin.font()?;
    let name = format!("skin-{}", skin.manifest.id);
    let family = FontFamily::Name(name.as_str().into());
    let key = egui::Id::new(("skin-font", &name));
    let valid = match ctx.data(|d| d.get_temp::<bool>(key)) {
        Some(valid) => valid,
        None => {
            let valid = ab_glyph::FontRef::try_from_slice(bytes).is_ok();
            ctx.data_mut(|d| d.insert_temp(key, valid));
            if valid {
                ctx.add_font(egui::epaint::text::FontInsert::new(
                    &name,
                    egui::FontData::from_owned(bytes.to_vec()),
                    vec![egui::epaint::text::InsertFontFamily {
                        family: family.clone(),
                        priority: egui::epaint::text::FontPriority::Lowest,
                    }],
                ));
                ctx.request_repaint();
            }
            valid
        }
    };
    // A font added this frame is bound from the next one; drawing with an unbound family would panic.
    (valid && ctx.fonts(|f| f.families().contains(&family))).then_some(family)
}

/// The largest rectangle with the model's bezel aspect that fits `cell`, centred.
pub(crate) fn bezel_rect(cell: Rect, model: DisplayModel) -> Rect {
    let aspect = model.bezel_aspect() as f32;
    let (w, h) = if cell.width() / cell.height() > aspect {
        (cell.height() * aspect, cell.height())
    } else {
        (cell.width(), cell.width() / aspect)
    };
    Rect::from_center_size(cell.center(), Vec2::new(w, h))
}

pub(crate) fn screen_rect(bezel: Rect, model: DisplayModel) -> Rect {
    let [l, t, w, h] = model.geometry().screen_rect;
    Rect::from_min_size(
        bezel.min + Vec2::new(l as f32 * bezel.width(), t as f32 * bezel.height()),
        Vec2::new(w as f32 * bezel.width(), h as f32 * bezel.height()),
    )
}

pub(crate) fn control_rect(bezel: Rect, model: DisplayModel, control: BezelControl) -> Option<Rect> {
    let p = model.placement(control)?;
    let center = bezel.min
        + Vec2::new(
            p.center[0] as f32 * bezel.width(),
            p.center[1] as f32 * bezel.height(),
        );
    let size = Vec2::new(
        p.size[0] as f32 * bezel.width(),
        p.size[1] as f32 * bezel.height(),
    );
    Some(if control.is_knob() {
        // Knobs are round: the bezel row's height sets their diameter.
        Rect::from_center_size(center, Vec2::splat(size.x.min(size.y).max(size.x * 0.9)))
    } else {
        Rect::from_center_size(center, size)
    })
}

/// The page bar's rectangle: the bottom strip of the MFD, under the split pane when split.
pub(crate) fn page_bar(screen: Rect, mode: ScreenMode) -> Option<Rect> {
    let height = screen.height() * PAGE_BAR_FRACTION;
    let left = match mode {
        ScreenMode::FullPfd => return None,
        ScreenMode::FullMfd => screen.left(),
        ScreenMode::Split => screen.left() + screen.width() * SPLIT_PFD_FRACTION,
    };
    Some(Rect::from_min_max(
        Pos2::new(left, screen.bottom() - height),
        screen.right_bottom(),
    ))
}

/// What the display's controls and chrome ask for this frame.
#[derive(Default)]
pub(crate) struct Output {
    pub equipment: Vec<EquipmentSetting>,
    pub glass: Vec<GlassSetting>,
}

fn act(out: &mut Output, id: &str, action: DisplayAction) {
    out.equipment.push(EquipmentSetting::Display {
        id: id.to_owned(),
        action,
    });
}

/// Draw the bezel body, keys, knobs, and card slot (no interaction).
pub(crate) fn paint_bezel(ui: &egui::Ui, bezel: Rect, model: DisplayModel, style: &Style) {
    let p = ui.painter();
    let radius = (bezel.width() * 0.012).max(2.);
    p.rect_filled(bezel, radius, style.bezel);
    p.rect_stroke(
        bezel,
        radius,
        Stroke::new((bezel.width() * 0.002).max(1.), style.trim),
        egui::StrokeKind::Inside,
    );
    let screen = screen_rect(bezel, model);
    p.rect_filled(screen.expand(bezel.width() * 0.004), 2., style.surround);
    for control in BezelControl::ALL {
        let Some(rect) = control_rect(bezel, model, control) else {
            continue;
        };
        match control {
            BezelControl::LeftKnob | BezelControl::RightKnob => {
                let radius = rect.width() / 2.;
                p.circle_filled(rect.center(), radius, style.knob);
                p.circle_stroke(rect.center(), radius, Stroke::new(1.2_f32, style.trim));
                p.circle_filled(rect.center(), radius * 0.55, style.key_face);
                p.circle_stroke(
                    rect.center(),
                    radius * 0.55,
                    Stroke::new(1.0_f32, style.knob_mark),
                );
                let tip = rect.center() + Vec2::new(0., -radius * 0.5);
                p.line_segment([rect.center(), tip], Stroke::new(1.5_f32, style.knob_mark));
            }
            BezelControl::CardSlot => {
                p.rect_filled(rect, 1., Color32::BLACK);
                p.rect_stroke(
                    rect,
                    1.,
                    Stroke::new(1.0_f32, style.trim),
                    egui::StrokeKind::Inside,
                );
            }
            _ => {
                p.rect(
                    rect,
                    (rect.height() * 0.2).max(1.),
                    style.key_face,
                    Stroke::new(1.0_f32, style.trim),
                    egui::StrokeKind::Inside,
                );
                p.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    style.label(control),
                    style.font(rect.height() * 0.32),
                    style.key_text,
                );
            }
        }
    }
}

/// Keys and knobs. Press, hold, wheel, drag, and push become display actions.
pub(crate) fn controls(ui: &mut egui::Ui, bezel: Rect, model: DisplayModel, id: &str, out: &mut Output) {
    let now = ui.input(|i| i.time);
    for (control, key) in [
        (BezelControl::NearestKey, EfisKey::Nearest),
        (BezelControl::DirectKey, EfisKey::Direct),
        (BezelControl::MenuKey, EfisKey::Menu),
        (BezelControl::BackKey, EfisKey::Back),
    ] {
        let Some(rect) = control_rect(bezel, model, control) else {
            continue;
        };
        let response = ui.interact(rect, ui.id().with(("efis-key", control)), Sense::click_and_drag());
        let state_id = ui.id().with(("efis-hold", control));
        if key == EfisKey::Back {
            let (start, fired): (Option<f64>, bool) =
                ui.data(|d| d.get_temp(state_id)).unwrap_or((None, false));
            if response.is_pointer_button_down_on() {
                let started = start.unwrap_or(now);
                let hold = !fired && now - started >= HOLD_S;
                if hold {
                    act(out, id, DisplayAction::Key { key, hold: true });
                }
                ui.data_mut(|d| d.insert_temp(state_id, (Some(started), fired || hold)));
                // Keep repainting so the hold fires without further input.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            } else {
                if (response.clicked() || response.drag_stopped()) && !fired {
                    act(out, id, DisplayAction::Key { key, hold: false });
                }
                ui.data_mut(|d| d.remove::<(Option<f64>, bool)>(state_id));
            }
        } else if response.clicked() {
            act(out, id, DisplayAction::Key { key, hold: false });
        }
    }
    for (control, knob) in [
        (BezelControl::LeftKnob, Knob::Left),
        (BezelControl::RightKnob, Knob::Right),
    ] {
        let Some(rect) = control_rect(bezel, model, control) else {
            continue;
        };
        let radius = rect.width() / 2.;
        let response = ui.interact(
            rect,
            ui.id().with(("efis-knob", control)),
            Sense::click_and_drag(),
        );
        let ring_at = |p: Pos2| {
            if p.distance(rect.center()) > radius * 0.55 {
                Ring::Outer
            } else {
                Ring::Inner
            }
        };
        let drag_id = ui.id().with(("efis-knob-drag", control));
        if response.hovered() {
            let wheel = ui.input(|i| i.raw_scroll_delta.y);
            if wheel != 0. {
                let ring = response.hover_pos().map_or(Ring::Outer, ring_at);
                let detents = if wheel > 0. { 1 } else { -1 };
                act(
                    out,
                    id,
                    DisplayAction::Knob {
                        knob,
                        ring,
                        detents,
                        time_s: now,
                    },
                );
            }
        }
        if response.drag_started() {
            let ring = response.interact_pointer_pos().map_or(Ring::Outer, ring_at);
            ui.data_mut(|d| d.insert_temp(drag_id, (ring, 0.0_f32)));
        }
        if response.dragged() {
            let delta = response.drag_delta().x - response.drag_delta().y;
            if let Some((ring, mut carry)) = ui.data(|d| d.get_temp::<(Ring, f32)>(drag_id)) {
                carry += delta;
                let steps = (carry / DETENT_PX).trunc();
                if steps != 0. {
                    carry -= steps * DETENT_PX;
                    act(
                        out,
                        id,
                        DisplayAction::Knob {
                            knob,
                            ring,
                            detents: steps as i32,
                            time_s: now,
                        },
                    );
                }
                ui.data_mut(|d| d.insert_temp(drag_id, (ring, carry)));
            }
        }
        if response.clicked()
            && response
                .interact_pointer_pos()
                .is_some_and(|p| ring_at(p) == Ring::Inner)
        {
            act(out, id, DisplayAction::KnobPush { knob, time_s: now });
        }
    }
}

fn chip(
    ui: &mut egui::Ui,
    rect: Rect,
    label: &str,
    lit: bool,
    style: &Style,
    salt: impl std::hash::Hash,
) -> bool {
    let p = ui.painter();
    p.rect(
        rect,
        2.,
        if lit {
            style.trim
        } else {
            Color32::from_black_alpha(190)
        },
        Stroke::new(1.0_f32, if lit { style.key_text } else { style.trim }),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        style.font((rect.height() * 0.52).min(rect.width() / (label.len().max(1) as f32 * 0.62))),
        style.key_text,
    );
    ui.interact(rect, ui.id().with(salt), Sense::click()).clicked()
}

/// Touch chrome on the screen: SPLIT/FULL, the MFD page bar, and the open menu or keypad.
pub(crate) fn chrome(
    ui: &mut egui::Ui,
    screen: Rect,
    id: &str,
    d: &TouchDisplayState,
    style: &Style,
    shared: Shared,
    out: &mut Output,
) {
    // A reverted display shows the PFD whatever page it has chosen.
    let mode = if d.reversion {
        ScreenMode::FullPfd
    } else {
        ScreenMode::of(d.layout)
    };
    let page = flightcraft_control::touch_efis::effective_page(d.layout, d.mfd_page);
    // SPLIT or FULL, in the upper corner clear of the data bar.
    let button = Rect::from_min_size(
        Pos2::new(
            screen.right() - screen.width() * 0.085,
            screen.top() + screen.height() * 0.09,
        ),
        Vec2::new(screen.width() * 0.075, screen.height() * 0.055),
    );
    let label = if mode == ScreenMode::Split {
        "FULL"
    } else {
        "SPLIT"
    };
    if chip(ui, button, label, false, style, "efis-split") {
        act(out, id, DisplayAction::ToggleSplit {});
    }
    if let Some(bar) = page_bar(screen, mode) {
        ui.painter().rect_filled(bar, 0., crate::theme::PAGE_BAR);
        let name = Rect::from_min_size(bar.min, Vec2::new(bar.width() * 0.3, bar.height()));
        ui.painter().text(
            name.left_center() + Vec2::new(4., 0.),
            Align2::LEFT_CENTER,
            page.label(),
            style.font(bar.height() * 0.5),
            style.key_text,
        );
        let tabs = MfdPage::MAIN;
        let width = bar.width() * 0.7 / tabs.len() as f32;
        for (i, p) in tabs.iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(bar.right() - width * (tabs.len() - i) as f32, bar.top()),
                Vec2::new(width, bar.height()),
            )
            .shrink2(Vec2::new(1., 2.));
            if chip(ui, rect, p.abbreviation(), *p == page, style, ("efis-page", i)) {
                act(out, id, DisplayAction::MfdPage { page: *p });
            }
        }
    }
    if d.overlay != Overlay::None {
        overlay(ui, screen, id, d, page, style, shared, out);
    }
}

/// Shared (not per display) settings the menus show and change.
#[derive(Clone, Copy)]
pub(crate) struct Shared {
    pub backup: bool,
    pub traffic_shown: bool,
    pub track_up: bool,
}

#[allow(clippy::too_many_arguments)]
fn overlay(
    ui: &mut egui::Ui,
    screen: Rect,
    id: &str,
    d: &TouchDisplayState,
    page: MfdPage,
    style: &Style,
    shared: Shared,
    out: &mut Output,
) {
    let panel = Rect::from_center_size(
        screen.center(),
        Vec2::new(screen.width() * 0.46, screen.height() * 0.72),
    );
    ui.painter().rect(
        panel,
        4.,
        crate::theme::MENU_PANEL,
        Stroke::new(1.5_f32, style.key_text),
        egui::StrokeKind::Inside,
    );
    let title = match d.overlay {
        Overlay::PfdOptions => "PFD OPTIONS".to_owned(),
        Overlay::PageMenu => format!("{} MENU", page.label().to_uppercase()),
        Overlay::MainMenu => "MAIN MENU".to_owned(),
        Overlay::DirectTo => "DIRECT-TO".to_owned(),
        Overlay::None => return,
    };
    let title_rect = Rect::from_min_size(panel.min, Vec2::new(panel.width(), panel.height() * 0.11));
    ui.painter().text(
        title_rect.center(),
        Align2::CENTER_CENTER,
        title,
        style.font(title_rect.height() * 0.6),
        style.key_text,
    );
    let row = std::cell::Cell::new(0usize);
    let rows_top = title_rect.bottom() + 2.;
    let row_h = panel.height() * 0.095;
    let entry = |ui: &mut egui::Ui, label: &str, lit: bool| -> bool {
        let rect = Rect::from_min_size(
            Pos2::new(panel.left() + 6., rows_top + row.get() as f32 * (row_h + 2.)),
            Vec2::new(panel.width() - 12., row_h),
        );
        row.set(row.get() + 1);
        chip(ui, rect, label, lit, style, ("efis-entry", row.get()))
    };
    match d.overlay {
        Overlay::PfdOptions => {
            if entry(ui, "Synthetic vision", d.synthetic_vision) {
                act(
                    out,
                    id,
                    DisplayAction::SyntheticVision {
                        enabled: !d.synthetic_vision,
                    },
                );
            }
            if entry(ui, "Engine strip", d.engine_strip) {
                act(
                    out,
                    id,
                    DisplayAction::EngineStrip {
                        visible: !d.engine_strip,
                    },
                );
            }
            if entry(ui, "Select page", false) {
                act(
                    out,
                    id,
                    DisplayAction::Key {
                        key: EfisKey::Menu,
                        hold: false,
                    },
                );
            }
        }
        Overlay::PageMenu => {
            if matches!(page, MfdPage::Map | MfdPage::Terrain | MfdPage::Traffic) {
                if entry(ui, "Traffic", shared.traffic_shown) {
                    out.glass.push(GlassSetting::Traffic {
                        shown: !shared.traffic_shown,
                    });
                }
                if entry(ui, "Track up", shared.track_up) {
                    use flightcraft_sim::avionics::glass::MapOrientation;
                    out.glass.push(GlassSetting::Orientation {
                        orientation: if shared.track_up {
                            MapOrientation::NorthUp
                        } else {
                            MapOrientation::TrackUp
                        },
                    });
                }
            } else {
                entry(ui, "No Options", false);
            }
        }
        Overlay::MainMenu => {
            for p in MfdPage::MAIN {
                if entry(ui, &format!("Page: {}", p.label()), p == page) {
                    act(out, id, DisplayAction::MfdPage { page: p });
                }
            }
            if entry(ui, "Display backup", shared.backup) {
                out.glass.push(GlassSetting::DisplayReversion {
                    enabled: !shared.backup,
                });
            }
        }
        Overlay::DirectTo => {
            let key = ui.id().with(("efis-direct", id));
            let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_default();
            let field = Rect::from_min_size(
                Pos2::new(panel.left() + 8., rows_top),
                Vec2::new(panel.width() - 16., row_h * 1.4),
            );
            let edit = ui.put(
                field,
                egui::TextEdit::singleline(&mut text)
                    .hint_text("Identifier")
                    .font(style.font(field.height() * 0.5))
                    .char_limit(8),
            );
            text = text.to_ascii_uppercase();
            ui.data_mut(|d| d.insert_temp(key, text.clone()));
            row.set(2);
            let enter = entry(ui, "ENTER", !text.is_empty());
            let enter_key = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (enter || enter_key) && !text.is_empty() {
                act(out, id, DisplayAction::DirectTo { query: text.clone() });
            }
            if entry(ui, "CANCEL", false) {
                act(out, id, DisplayAction::CloseOverlay {});
            }
        }
        Overlay::None => {}
    }
}
