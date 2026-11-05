//! Bezel, soft chrome, and menus for the landscape touch EFIS display model.
//!
//! Everything here draws the model's controls and sends `DisplayAction`s; the behaviour (what a key
//! or knob does) lives in `flightcraft_control::touch_efis`. A skin pack only changes `Style`.
use eframe::egui::{self, Align2, Color32, FontFamily, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use flightcraft_aero::display_model::{
    BezelControl, DATA_BAR_FRACTION, DisplayModel, PfdElement, PfdMode, SPLIT_PANEL_FRACTION,
    STATUS_BAR_FRACTION,
};
use flightcraft_aero::modern::{MfdPage, NearestPage};
use flightcraft_aero::panel_fidelity::SrgbHex;
use flightcraft_control::touch_efis::{EfisKey, Knob, Overlay, PageChoice, REVERSION_PAGE, Ring, ScreenMode};
use flightcraft_control::touch_entry::{self, EntryShortcut, EntryTarget, PadKey};
use flightcraft_control::{DisplayAction, EquipmentSetting, GlassSetting, SkinContents, TouchDisplayState};

/// Fraction of the screen width the PFD (with its engine column) takes in split mode.
pub(crate) const SPLIT_PFD_FRACTION: f32 = SPLIT_PANEL_FRACTION as f32;
/// Height of the nearest page's tab strip as a fraction of the screen height.
pub(crate) const NEAREST_TAB_FRACTION: f32 = 0.07;
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

/// A rectangle given in the model's native 1280 x 768 pixels, on the screen.
pub(crate) fn native_rect(screen: Rect, [x, y, w, h]: [f64; 4]) -> Rect {
    let k = screen.width() / 1280.;
    Rect::from_min_size(
        screen.min + Vec2::new(x as f32 * k, y as f32 * k),
        Vec2::new(w as f32 * k, h as f32 * k),
    )
}

/// The primary flight display's arrangement for a screen mode, if it shows one.
pub(crate) fn pfd_mode(mode: ScreenMode) -> Option<PfdMode> {
    match mode {
        ScreenMode::FullPfd => Some(PfdMode::Full),
        ScreenMode::Split => Some(PfdMode::Split),
        ScreenMode::FullMfd => None,
    }
}

/// The screen between the data bar and the status bar.
pub(crate) fn content_rect(screen: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            screen.left(),
            screen.top() + screen.height() * DATA_BAR_FRACTION as f32,
        ),
        Pos2::new(
            screen.right(),
            screen.bottom() - screen.height() * STATUS_BAR_FRACTION as f32,
        ),
    )
}

/// The multi-function pane: all the content area, or its part right of the PFD when split.
pub(crate) fn mfd_rect(screen: Rect, mode: ScreenMode) -> Option<Rect> {
    let content = content_rect(screen);
    match mode {
        ScreenMode::FullPfd => None,
        ScreenMode::FullMfd => Some(content),
        ScreenMode::Split => Some(Rect::from_min_max(
            Pos2::new(screen.left() + screen.width() * SPLIT_PFD_FRACTION, content.top()),
            content.max,
        )),
    }
}

/// The page bar's rectangle: the bottom strip of the multi-function pane, above the status bar.
pub(crate) fn page_bar(screen: Rect, mode: ScreenMode) -> Option<Rect> {
    let pane = mfd_rect(screen, mode)?;
    let height = screen.height() * PAGE_BAR_FRACTION;
    Some(Rect::from_min_max(
        Pos2::new(pane.left(), pane.bottom() - height),
        pane.max,
    ))
}

/// The nearest page's tab strip: the top of the pane.
pub(crate) fn nearest_tabs(screen: Rect, mode: ScreenMode) -> Option<Rect> {
    let pane = mfd_rect(screen, mode)?;
    Some(Rect::from_min_size(
        pane.min,
        Vec2::new(pane.width(), screen.height() * NEAREST_TAB_FRACTION),
    ))
}

/// The pane's map or list area: the pane less the page bar, and the tab strip on a nearest page.
pub(crate) fn pane_content(screen: Rect, mode: ScreenMode, page: MfdPage) -> Option<Rect> {
    let pane = mfd_rect(screen, mode)?;
    let mut area = Rect::from_min_max(pane.min, Pos2::new(pane.right(), page_bar(screen, mode)?.top()));
    if page == MfdPage::Nearest {
        area.min.y += screen.height() * NEAREST_TAB_FRACTION;
    }
    Some(area)
}

/// What the display's controls and chrome ask for this frame.
#[derive(Default)]
pub(crate) struct Output {
    pub equipment: Vec<EquipmentSetting>,
    pub glass: Vec<GlassSetting>,
    /// Autopilot commands from the on-screen AFCS controls.
    pub autopilot: Vec<flightcraft_sim::equipment::PanelSetting>,
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

/// Shared (not per display) settings the menus show and change.
#[derive(Clone)]
pub(crate) struct Shared {
    pub backup: bool,
    pub traffic_shown: bool,
    pub track_up: bool,
    pub pressure_hpa: bool,
    pub range_nm: f64,
    pub com_active_khz: u32,
    pub com_standby_khz: u32,
    pub nav_active_khz: Option<u32>,
    pub nav_standby_khz: Option<u32>,
    pub xpdr_code: String,
    pub xpdr_mode: flightcraft_sim::avionics::glass::TransponderMode,
    /// A crew alert (CAS message) is active: its window can be touched to acknowledge it.
    pub cas_active: bool,
    /// More CAS messages than the window holds: its triangle opens the message list page.
    pub cas_overflow: bool,
    /// The CAS messages, for the message list page.
    pub cas: flightcraft_control::touch_cas::CasWindow,
    /// Seconds of transponder IDENT left; zero when not identing.
    pub ident_remaining_s: f64,
    /// What the data bar's user field shows.
    pub user_field: flightcraft_control::DataBarField,
    /// The Vspeed bugs' configuration, and the aircraft pack's Vr, Vx, and Vy in knots.
    pub vspeeds: flightcraft_control::VspeedConfig,
    pub pack_vspeeds_kt: [Option<f64>; 5],
    /// The G meter is shown.
    pub g_meter: bool,
    /// The PFD pitch attitude offset, degrees.
    pub pitch_offset_deg: f64,
    /// The installed autopilot's state; touching the AFCS status box opens its controls.
    pub afcs: Option<flightcraft_sim::equipment::EquipmentState>,
    /// The two bearing pointers follow GPS.
    pub bearing: [bool; 2],
    /// The active flight plan, for the FPL page.
    pub fpl: FplView,
}

/// What the FPL page lists: one row per flight-plan waypoint, and which are selected and active.
#[derive(Clone, Default)]
pub(crate) struct FplView {
    pub rows: Vec<FplRow>,
    pub cursor: usize,
    pub active: usize,
}
#[derive(Clone)]
pub(crate) struct FplRow {
    pub ident: String,
    pub bearing_deg: f32,
    pub distance_nm: f32,
    /// The waypoint's altitude constraint, as written on the page.
    pub constraint: Option<String>,
}

fn hit(ui: &mut egui::Ui, rect: Rect, salt: impl std::hash::Hash) -> bool {
    ui.interact(rect, ui.id().with(("efis-hit", salt)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// Touch chrome on the screen: SPLIT/FULL, the touch boxes of the PFD, the MFD page bar, the
/// nearest tabs, the map zoom buttons, and the open menu or keypad.
pub(crate) fn chrome(
    ui: &mut egui::Ui,
    screen: Rect,
    id: &str,
    d: &TouchDisplayState,
    style: &Style,
    shared: &Shared,
    out: &mut Output,
) {
    // A reverted display shows the split screen with the engine page.
    let mode = if d.reversion {
        ScreenMode::Split
    } else {
        ScreenMode::of(d.layout)
    };
    let page = if d.reversion {
        REVERSION_PAGE
    } else {
        flightcraft_control::touch_efis::effective_page(d.layout, d.mfd_page)
    };
    let model = d.model.unwrap_or(DisplayModel::LandscapeTouchEfis);
    // SPLIT or FULL, at the right end of the data bar.
    let full = model.pfd_geometry(PfdMode::Full);
    if !d.reversion {
        let button = native_rect(screen, full.screen_rect(PfdElement::SplitButton));
        let label = if mode == ScreenMode::Split {
            "FULL"
        } else {
            "SPLIT"
        };
        if chip(ui, button, label, false, style, "efis-split") {
            act(out, id, DisplayAction::ToggleSplit {});
        }
    }
    if d.overlay == Overlay::None {
        let boxes = pfd_mode(mode).map(|m| model.pfd_geometry(m));
        // The radio boxes are in the data bar on every screen: touching the active frequency
        // transfers, touching the standby frequency opens the COM window.
        let com = native_rect(screen, full.screen_rect(PfdElement::ComBox));
        let (active, standby) = com.split_left_right_at_fraction(0.5);
        if hit(ui, active, "com-active") {
            act(
                out,
                id,
                DisplayAction::Transfer {
                    band: flightcraft_control::RadioBand::Com,
                },
            );
        }
        if hit(ui, standby, "com-standby") {
            act(
                out,
                id,
                DisplayAction::OpenOverlay {
                    overlay: Overlay::ComWindow,
                },
            );
        }
        // The data bar's user field (where the guide has minimum safe altitude): the left half
        // cycles dashes, clock, and timer; with the timer shown, the right half starts and stops it.
        if let Some(fx) = boxes.as_ref().and_then(|g| {
            g.data_fields
                .iter()
                .find(|(f, _)| *f == flightcraft_aero::display_model::DataField::SafeAltitude)
                .map(|(_, fx)| *fx)
        }) {
            let field = native_rect(screen, [fx - 55., 0., 110., 60.]);
            let timer = shared.user_field == flightcraft_control::DataBarField::Timer;
            let (left, right) = field.split_left_right_at_fraction(if timer { 0.5 } else { 1. });
            if hit(ui, left, "user-field") {
                out.glass.push(flightcraft_control::GlassSetting::DataBarField {
                    field: shared.user_field.next(),
                });
            }
            if timer && hit(ui, right, "user-field-timer") {
                out.glass.push(flightcraft_control::GlassSetting::Timer {
                    operation: flightcraft_control::TimerAction::Toggle {},
                });
            }
        }
        if let Some(g) = &boxes {
            for (element, overlay) in [
                (
                    PfdElement::SelectedAltitudeBox,
                    Overlay::Keypad(EntryTarget::Altitude),
                ),
                (PfdElement::BaroBox, Overlay::Keypad(EntryTarget::Baro)),
                (PfdElement::MinimumsBox, Overlay::Keypad(EntryTarget::Minimums)),
                (
                    PfdElement::SelectedHeadingBox,
                    Overlay::Keypad(EntryTarget::Heading),
                ),
                (
                    PfdElement::SelectedCourseBox,
                    Overlay::Keypad(EntryTarget::Course),
                ),
                (PfdElement::TransponderBox, Overlay::TransponderWindow),
            ] {
                let rect = native_rect(screen, g.screen_rect(element));
                if rect.width() > 0. && hit(ui, rect, element) {
                    act(out, id, DisplayAction::OpenOverlay { overlay });
                }
            }
            // Touching the altitude tape sets the selected altitude to the current one, and touching
            // the compass rose sets the heading bug to the current heading: the keypads' Set To
            // Current, reached from the instruments themselves.
            let tape = native_rect(screen, g.screen_rect(PfdElement::AltitudeTape));
            if tape.width() > 0. && hit(ui, tape, "altitude-tape") {
                act(
                    out,
                    id,
                    DisplayAction::Shortcut {
                        target: EntryTarget::Altitude,
                        shortcut: flightcraft_control::touch_entry::EntryShortcut::SetToCurrent,
                    },
                );
            }
            let [px, py, ..] = g.panel;
            let [cx, cy] = g.hsi_center;
            let r = g.hsi_radius * 0.7;
            let rose = native_rect(screen, [px + cx - r, py + cy - r, r * 2., r * 2.]);
            if hit(ui, rose, "compass-rose") {
                act(
                    out,
                    id,
                    DisplayAction::Shortcut {
                        target: EntryTarget::Heading,
                        shortcut: flightcraft_control::touch_entry::EntryShortcut::SetToCurrent,
                    },
                );
            }
            // Touching the AFCS status box opens the on-screen autopilot controls.
            let afcs = native_rect(screen, g.screen_rect(PfdElement::AfcsBox));
            if shared.afcs.is_some() && afcs.width() > 0. && hit(ui, afcs, "afcs-box") {
                act(
                    out,
                    id,
                    DisplayAction::OpenOverlay {
                        overlay: Overlay::Afcs,
                    },
                );
            }
            // Touching the G meter starts its record over.
            let meter = native_rect(screen, g.screen_rect(PfdElement::GMeter));
            if shared.g_meter && meter.width() > 0. && hit(ui, meter, "g-meter") {
                out.glass.push(GlassSetting::GMeterReset {});
            }
            // Touching the CAS window acknowledges the active alerts (pilot's guide 3.7).
            let cas = native_rect(screen, g.screen_rect(PfdElement::CasWindow));
            if shared.cas_overflow && cas.width() > 0. {
                // The down-pointing triangle at the window's lower right opens the message list.
                let triangle = Rect::from_min_size(
                    Pos2::new(cas.right() - cas.width() * 0.2, cas.bottom() - cas.height() * 0.3),
                    Vec2::new(cas.width() * 0.2, cas.height() * 0.3),
                );
                if hit(ui, triangle, "cas-triangle") {
                    act(out, id, DisplayAction::MfdPage { page: MfdPage::Cas });
                }
            }
            if shared.cas_active && cas.width() > 0. && hit(ui, cas, "cas-window") {
                act(out, id, DisplayAction::AcknowledgeCas {});
            }
        }
    }
    if let Some(bar) = page_bar(screen, mode) {
        ui.painter().rect_filled(bar, 0., crate::theme::PAGE_BAR);
        // Touching the page name opens the Select Page menu.
        let name = Rect::from_min_size(bar.min, Vec2::new(bar.width() * 0.3, bar.height()));
        // A long name ("CAS messages") shrinks to stay inside its third of the bar, clear of the tabs.
        let mut size = bar.height() * 0.5;
        let mut galley = ui
            .painter()
            .layout_no_wrap(page.label().into(), style.font(size), style.key_text);
        let room = (name.width() - 14.).max(1.);
        if galley.size().x > room {
            size *= room / galley.size().x;
            galley = ui
                .painter()
                .layout_no_wrap(page.label().into(), style.font(size), style.key_text);
        }
        ui.painter().galley(
            name.left_center() + Vec2::new(4., -galley.size().y * 0.5),
            galley,
            style.key_text,
        );
        if d.overlay == Overlay::None && hit(ui, name, "page-name") {
            act(
                out,
                id,
                DisplayAction::OpenOverlay {
                    overlay: Overlay::SelectPage,
                },
            );
        }
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
    if page == MfdPage::Nearest
        && let Some(tabs) = nearest_tabs(screen, mode)
    {
        ui.painter().rect_filled(tabs, 0., crate::theme::PAGE_BAR);
        let width = tabs.width() / NearestPage::ALL.len() as f32;
        for (i, p) in NearestPage::ALL.iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(tabs.left() + width * i as f32, tabs.top()),
                Vec2::new(width, tabs.height()),
            )
            .shrink2(Vec2::new(2., 3.));
            if chip(
                ui,
                rect,
                p.abbreviation(),
                *p == d.nearest_page,
                style,
                ("efis-nearest", i),
            ) {
                act(out, id, DisplayAction::NearestPage { page: *p });
            }
        }
    }
    if page == MfdPage::Cas
        && d.overlay == Overlay::None
        && let Some(area) = pane_content(screen, mode, page)
    {
        cas_page(ui, area, style, shared, out);
    }
    if page == MfdPage::FlightPlan
        && d.overlay == Overlay::None
        && let Some(area) = pane_content(screen, mode, page)
    {
        flight_plan_page(ui, area, id, style, shared, out);
    }
    // The map's zoom buttons at the right edge, and the range at its lower right.
    if matches!(page, MfdPage::Map | MfdPage::Terrain)
        && let Some(area) = pane_content(screen, mode, page)
    {
        let side = screen.height() * 0.075;
        // On the full-screen map page the data column takes the right of the area.
        let map_right = if mode == ScreenMode::FullMfd && page == MfdPage::Map {
            area.left() + area.width() * 0.64
        } else {
            area.right()
        };
        let x = map_right - side - 6.;
        for (i, (label, steps)) in [("+", 1), ("-", -1)].into_iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(x, area.top() + 8. + i as f32 * (side + 4.)),
                Vec2::splat(side),
            );
            if chip(ui, rect, label, false, style, ("efis-zoom", i)) {
                act(out, id, DisplayAction::MapZoom { steps });
            }
        }
        ui.painter().text(
            Pos2::new(map_right - 8., area.bottom() - 6.),
            Align2::RIGHT_BOTTOM,
            flightcraft_control::touch_efis::range_label(shared.range_nm),
            style.font(screen.height() * 0.04),
            Color32::WHITE,
        );
    }
    if d.overlay != Overlay::None {
        overlay(ui, screen, id, d, page, style, shared, out);
    }
}

#[allow(clippy::too_many_arguments)]
fn overlay(
    ui: &mut egui::Ui,
    screen: Rect,
    id: &str,
    d: &TouchDisplayState,
    page: MfdPage,
    style: &Style,
    shared: &Shared,
    out: &mut Output,
) {
    let tall = d.overlay.entry_target().is_some();
    let afcs = d.overlay == Overlay::Afcs;
    let panel = Rect::from_center_size(
        screen.center(),
        if afcs {
            Vec2::new(screen.width() * 0.6, screen.height() * 0.8)
        } else {
            Vec2::new(
                screen.width() * if tall { 0.5 } else { 0.46 },
                screen.height() * if tall { 0.9 } else { 0.72 },
            )
        },
    );
    ui.painter().rect(
        panel,
        4.,
        crate::theme::MENU_PANEL,
        Stroke::new(1.5_f32, style.key_text),
        egui::StrokeKind::Inside,
    );
    if let Some(target) = d.overlay.entry_target() {
        entry_panel(ui, panel, id, d, style, shared, out, target);
        return;
    }
    if afcs {
        afcs_panel(ui, panel, id, style, shared, out);
        return;
    }
    let title = match d.overlay {
        Overlay::PfdOptions => "PFD OPTIONS".to_owned(),
        Overlay::PageMenu => format!("{} MENU", page.label().to_uppercase()),
        Overlay::MainMenu => "MAIN MENU".to_owned(),
        Overlay::SelectPage => "SELECT PAGE".to_owned(),
        Overlay::Vspeeds => "VSPEED BUGS".to_owned(),
        _ => return,
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
    let row_h = panel.height() * 0.085;
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
            for (n, on) in shared.bearing.into_iter().enumerate() {
                use flightcraft_sim::avionics::glass::BearingPointer as B;
                let source = if on { B::Off } else { B::Gps };
                if entry(
                    ui,
                    &format!("Bearing {}: {}", n + 1, if on { "GPS" } else { "Off" }),
                    on,
                ) {
                    out.glass.push(if n == 0 {
                        GlassSetting::BearingPointer { source }
                    } else {
                        GlassSetting::BearingPointer2 { source }
                    });
                }
            }
            if entry(ui, "G meter", shared.g_meter) {
                out.glass.push(GlassSetting::GMeter {
                    shown: !shared.g_meter,
                });
            }
            if shared.g_meter && entry(ui, "Reset G meter", false) {
                out.glass.push(GlassSetting::GMeterReset {});
            }
            // Pitch attitude offset: moves the pitch scale against the aircraft symbol, 2.5 degrees
            // either way. Touching the label zeroes it.
            {
                let row_rect = Rect::from_min_size(
                    Pos2::new(panel.left() + 6., rows_top + row.get() as f32 * (row_h + 2.)),
                    Vec2::new(panel.width() - 12., row_h),
                );
                row.set(row.get() + 1);
                let offset = shared.pitch_offset_deg;
                let cell_w = row_rect.width() / 6.;
                let cell = |i: f32, w: f32| {
                    Rect::from_min_size(
                        row_rect.min + Vec2::new(i * cell_w, 0.),
                        Vec2::new(cell_w * w, row_rect.height()),
                    )
                    .shrink2(Vec2::new(2., 0.))
                };
                let label_rect = cell(0., 2.);
                ui.painter().text(
                    label_rect.left_center() + Vec2::new(4., 0.),
                    Align2::LEFT_CENTER,
                    format!("Pitch offset {offset:+.1}"),
                    style.font(row_h * 0.4),
                    style.key_text,
                );
                for (i, (text, delta)) in [("-0.5", -0.5), ("-0.1", -0.1), ("+0.1", 0.1), ("+0.5", 0.5)]
                    .into_iter()
                    .enumerate()
                {
                    if chip(
                        ui,
                        cell(2. + i as f32, 1.),
                        text,
                        false,
                        style,
                        ("pitch-offset", i),
                    ) {
                        out.glass.push(GlassSetting::PitchOffset {
                            degrees: (offset + delta).clamp(
                                -flightcraft_control::PITCH_OFFSET_LIMIT_DEG,
                                flightcraft_control::PITCH_OFFSET_LIMIT_DEG,
                            ),
                        });
                    }
                }
                if offset != 0.
                    && ui
                        .interact(label_rect, ui.id().with("pitch-offset-zero"), Sense::click())
                        .clicked()
                {
                    out.glass.push(GlassSetting::PitchOffset { degrees: 0. });
                }
            }
            if entry(ui, "Vspeed bugs", false) {
                act(
                    out,
                    id,
                    DisplayAction::OpenOverlay {
                        overlay: Overlay::Vspeeds,
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
        Overlay::Vspeeds => {
            use flightcraft_control::VspeedBug;
            for (i, bug) in VspeedBug::ALL.into_iter().enumerate() {
                let top = rows_top + i as f32 * (row_h + 2.);
                let full = Rect::from_min_size(
                    Pos2::new(panel.left() + 6., top),
                    Vec2::new(panel.width() - 12., row_h),
                );
                let (toggle, set) = full.split_left_right_at_fraction(0.7);
                let pack = shared.pack_vspeeds_kt.get(i).copied().flatten();
                let entry = shared.vspeeds.entry(bug);
                let speed = entry
                    .speed_kt
                    .or(pack)
                    .map_or("---".to_owned(), |kt| format!("{kt:.0} kt"));
                let text = format!(
                    "{:<5}{:>8}   {}",
                    bug.name(),
                    speed,
                    if entry.enabled { "ON" } else { "OFF" }
                );
                if chip(
                    ui,
                    toggle.shrink2(Vec2::new(2., 0.)),
                    &text,
                    entry.enabled,
                    style,
                    ("vspeed", i),
                ) {
                    out.glass.push(GlassSetting::Vspeed {
                        bug,
                        enabled: !entry.enabled,
                    });
                }
                if chip(
                    ui,
                    set.shrink2(Vec2::new(2., 0.)),
                    "Set",
                    false,
                    style,
                    ("vspeed-set", i),
                ) {
                    act(
                        out,
                        id,
                        DisplayAction::OpenOverlay {
                            overlay: Overlay::Keypad(match bug {
                                VspeedBug::R => EntryTarget::VspeedR,
                                VspeedBug::X => EntryTarget::VspeedX,
                                VspeedBug::Y => EntryTarget::VspeedY,
                                VspeedBug::G => EntryTarget::VspeedG,
                                VspeedBug::App => EntryTarget::VspeedApp,
                            }),
                        },
                    );
                }
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
        Overlay::SelectPage => {
            for choice in flightcraft_control::touch_efis::select_page_entries(page) {
                match choice {
                    PageChoice::Main(p) => {
                        if entry(ui, p.label(), p == page) {
                            act(out, id, DisplayAction::MfdPage { page: p });
                        }
                    }
                    PageChoice::Nearest(p) => {
                        if entry(ui, p.label(), p == d.nearest_page) {
                            act(out, id, DisplayAction::NearestPage { page: p });
                            act(out, id, DisplayAction::CloseOverlay {});
                        }
                    }
                }
            }
        }
        Overlay::MainMenu => {
            for (label, overlay) in [
                ("COM1 radio", Overlay::ComWindow),
                ("NAV1 radio", Overlay::NavWindow),
                ("XPDR", Overlay::TransponderWindow),
            ] {
                if entry(ui, label, false) {
                    act(out, id, DisplayAction::OpenOverlay { overlay });
                }
            }
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
        _ => {}
    }
}

/// Where the on-screen AFCS controls' keys sit inside their window.
pub(crate) struct AfcsLayout {
    pub keys: Vec<(flightcraft_control::touch_pfd::AfcsKey, Rect)>,
    /// The nose pair's reference readout, between the two keys.
    pub reference: Rect,
    pub close: Rect,
}

/// The window the AFCS status box opens: three rows of mode keys (AP FD YD, HDG TRK NAV APR, ALT VS
/// IAS LVL), then NOSE UP, the reference, and NOSE DN, then Close.
pub(crate) fn afcs_layout(panel: Rect) -> AfcsLayout {
    use flightcraft_control::touch_pfd::AfcsKey;
    let rows: [&[AfcsKey]; 3] = [
        &[AfcsKey::Ap, AfcsKey::Fd, AfcsKey::Yd],
        &[AfcsKey::Hdg, AfcsKey::Trk, AfcsKey::Nav, AfcsKey::Apr],
        &[AfcsKey::Alt, AfcsKey::Vs, AfcsKey::Ias, AfcsKey::Lvl],
    ];
    let top = panel.top() + panel.height() * 0.13;
    let row_h = panel.height() * 0.135;
    let pitch = row_h + panel.height() * 0.02;
    let inner = panel.shrink2(Vec2::new(10., 0.));
    let mut keys = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let w = inner.width() / row.len() as f32;
        for (c, key) in row.iter().enumerate() {
            keys.push((
                *key,
                Rect::from_min_size(
                    Pos2::new(inner.left() + c as f32 * w, top + r as f32 * pitch),
                    Vec2::new(w, row_h),
                )
                .shrink(3.),
            ));
        }
    }
    let nose_top = top + 3. * pitch;
    let w = inner.width() / 3.;
    for (c, key) in [AfcsKey::NoseUp, AfcsKey::NoseDown].into_iter().enumerate() {
        keys.push((
            key,
            Rect::from_min_size(
                Pos2::new(inner.left() + c as f32 * 2. * w, nose_top),
                Vec2::new(w, row_h),
            )
            .shrink(3.),
        ));
    }
    AfcsLayout {
        keys,
        reference: Rect::from_min_size(Pos2::new(inner.left() + w, nose_top), Vec2::new(w, row_h)).shrink(3.),
        close: Rect::from_min_size(
            Pos2::new(inner.left() + w, top + 4. * pitch),
            Vec2::new(w, row_h * 0.8),
        )
        .shrink(3.),
    }
}

/// The on-screen autopilot controls: mode keys lit green when active and white when armed, the
/// nose up and nose down pair around the active mode's reference, and a close key.
fn afcs_panel(ui: &mut egui::Ui, panel: Rect, id: &str, style: &Style, shared: &Shared, out: &mut Output) {
    use flightcraft_control::touch_pfd::AfcsTone;
    ui.painter().text(
        Pos2::new(panel.center().x, panel.top() + panel.height() * 0.06),
        Align2::CENTER_CENTER,
        "AUTOMATIC FLIGHT CONTROL",
        style.font(panel.height() * 0.055),
        style.key_text,
    );
    let Some(ap) = &shared.afcs else {
        return;
    };
    let window = flightcraft_control::touch_pfd::afcs_window(ap);
    let layout = afcs_layout(panel);
    let mut pressed = None;
    for (key, rect) in &layout.keys {
        let Some(b) = window.buttons.iter().find(|b| b.key == *key) else {
            continue;
        };
        let (fill, edge, ink, width) = match (b.enabled, b.tone) {
            (false, _) => (
                Color32::from_black_alpha(120),
                style.trim.gamma_multiply(0.4),
                style.key_text.gamma_multiply(0.35),
                1.0_f32,
            ),
            (true, Some(AfcsTone::Active)) => (
                Color32::from_rgb(10, 70, 30),
                Color32::from_rgb(40, 220, 90),
                Color32::from_rgb(70, 255, 120),
                3.0,
            ),
            (true, Some(AfcsTone::Armed)) => (
                Color32::from_black_alpha(190),
                Color32::WHITE,
                Color32::WHITE,
                3.0,
            ),
            _ => (Color32::from_black_alpha(190), style.trim, style.key_text, 1.0),
        };
        ui.painter().rect(
            *rect,
            3.,
            fill,
            Stroke::new(width, edge),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            b.label,
            style.font((rect.height() * 0.42).min(rect.width() / (b.label.len().max(1) as f32 * 0.7))),
            ink,
        );
        if ui
            .interact(*rect, ui.id().with(("afcs-key", b.label)), Sense::click())
            .clicked()
            && b.enabled
        {
            pressed = Some(*key);
        }
    }
    ui.painter().text(
        layout.reference.center(),
        Align2::CENTER_CENTER,
        &window.reference,
        style.font(layout.reference.height() * 0.4),
        Color32::from_rgb(70, 255, 120),
    );
    if chip(ui, layout.close, "Close", false, style, "afcs-close") {
        act(out, id, DisplayAction::CloseOverlay {});
    }
    if let Some(key) = pressed {
        out.autopilot
            .extend(flightcraft_control::touch_pfd::afcs_press(ap, key));
    }
}

fn khz_text(khz: u32) -> String {
    format!("{:.3}", f64::from(khz) / 1000.)
}

/// A keypad window: the text typed so far, the keys, the target's shortcut buttons, and ENTER and
/// CANCEL. The radio and transponder windows add their own header and keys above it.
#[allow(clippy::too_many_arguments)]
fn entry_panel(
    ui: &mut egui::Ui,
    panel: Rect,
    id: &str,
    d: &TouchDisplayState,
    style: &Style,
    shared: &Shared,
    out: &mut Output,
    target: EntryTarget,
) {
    use flightcraft_sim::avionics::glass::TransponderMode as Mode;
    let hpa = shared.pressure_hpa;
    // Typed text is the pad's own state, cleared when a different pad opens.
    let key = ui.id().with(("efis-pad", id));
    let (opened, mut text): (Option<Overlay>, String) =
        ui.data(|x| x.get_temp(key)).unwrap_or((None, String::new()));
    if opened != Some(d.overlay) {
        text.clear();
    }
    let mut header: Vec<(String, bool)> = Vec::new();
    match d.overlay {
        Overlay::ComWindow => header.push((
            format!(
                "ACTIVE {}    STBY {}",
                khz_text(shared.com_active_khz),
                khz_text(shared.com_standby_khz)
            ),
            false,
        )),
        Overlay::NavWindow => header.push((
            format!(
                "ACTIVE {}    STBY {}",
                shared
                    .nav_active_khz
                    .map_or("---.--".into(), |k| format!("{:.2}", f64::from(k) / 1000.)),
                shared
                    .nav_standby_khz
                    .map_or("---.--".into(), |k| format!("{:.2}", f64::from(k) / 1000.)),
            ),
            false,
        )),
        Overlay::TransponderWindow => header.push((
            format!(
                "CODE {}    MODE {}{}",
                shared.xpdr_code,
                format!("{:?}", shared.xpdr_mode).to_uppercase(),
                if shared.ident_remaining_s > 0. {
                    "    IDENT"
                } else {
                    ""
                }
            ),
            false,
        )),
        _ => {}
    }
    let rows = target.rows(hpa);
    let shortcuts = EntryShortcut::offered(target);
    let extra_row = matches!(
        d.overlay,
        Overlay::ComWindow | Overlay::NavWindow | Overlay::TransponderWindow
    );
    let units = 1.0
        + 0.8 * header.len() as f32
        + 1.4
        + 0.5 * f32::from(u8::from(true))
        + rows.len() as f32
        + 1.0
        + f32::from(u8::from(!shortcuts.is_empty()))
        + f32::from(u8::from(extra_row))
        + 1.0;
    let unit = (panel.height() - 16.) / units;
    let mut y = panel.top() + 6.;
    ui.painter().text(
        Pos2::new(panel.center().x, y + unit * 0.5),
        Align2::CENTER_CENTER,
        match d.overlay {
            Overlay::ComWindow => "COM1".to_owned(),
            Overlay::NavWindow => "NAV1".to_owned(),
            Overlay::TransponderWindow => "TRANSPONDER".to_owned(),
            _ => target.title().to_owned(),
        },
        style.font(unit * 0.6),
        style.key_text,
    );
    y += unit;
    for (line, _) in &header {
        ui.painter().text(
            Pos2::new(panel.center().x, y + unit * 0.4),
            Align2::CENTER_CENTER,
            line,
            style.font(unit * 0.5),
            style.key_text,
        );
        y += unit * 0.8;
    }
    // The text field, with the reason an entry cannot be accepted yet.
    let field = Rect::from_min_size(
        Pos2::new(panel.left() + 10., y),
        Vec2::new(panel.width() - 20., unit * 1.3),
    );
    ui.painter().rect(
        field,
        2.,
        Color32::BLACK,
        Stroke::new(1.0_f32, style.trim),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        field.left_center() + Vec2::new(8., 0.),
        Align2::LEFT_CENTER,
        format!("{text}_"),
        style.font(unit * 0.9),
        crate::theme::FACE_CYAN,
    );
    y += unit * 1.4;
    let parsed = touch_entry::parse(target, hpa, &text);
    ui.painter().text(
        Pos2::new(panel.center().x, y + unit * 0.25),
        Align2::CENTER_CENTER,
        match (&parsed, text.is_empty()) {
            (Err(e), false) => e.to_string(),
            _ => String::new(),
        },
        style.font(unit * 0.38),
        crate::theme::AMBER,
    );
    y += unit * 0.5;
    // Keys.
    for (r, chars) in rows.iter().enumerate() {
        let n = chars.len() as f32;
        let w = (unit * 1.25).min((panel.width() - 24.) / n.max(7.));
        let x0 = panel.center().x - w * n * 0.5;
        for (c, ch) in chars.iter().enumerate() {
            let rect = Rect::from_min_size(Pos2::new(x0 + c as f32 * w, y), Vec2::new(w, unit)).shrink(1.5);
            if chip(ui, rect, &ch.to_string(), false, style, ("efis-pad-key", r, c)) {
                text = touch_entry::press(target, hpa, &text, PadKey::Char(*ch));
            }
        }
        y += unit;
    }
    // Editing keys, and the window's own controls.
    let strip = |ui: &mut egui::Ui, y: f32, buttons: &[(String, bool)], salt: &str| -> Option<usize> {
        let n = buttons.len() as f32;
        let w = (panel.width() - 24.) / n.max(1.);
        let mut hit = None;
        for (i, (label, lit)) in buttons.iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(panel.left() + 12. + i as f32 * w, y),
                Vec2::new(w, unit),
            )
            .shrink(2.);
            if chip(ui, rect, label, *lit, style, (salt, i)) {
                hit = Some(i);
            }
        }
        hit
    };
    match strip(
        ui,
        y,
        &[("BKSP".into(), false), ("CLR".into(), false)],
        "efis-pad-edit",
    ) {
        Some(0) => text = touch_entry::press(target, hpa, &text, PadKey::Backspace),
        Some(_) => text = touch_entry::press(target, hpa, &text, PadKey::Clear),
        None => {}
    }
    y += unit;
    if extra_row {
        let buttons: Vec<(String, bool)> = match d.overlay {
            Overlay::TransponderWindow => [("STBY", Mode::Standby), ("ALT", Mode::Alt), ("ON", Mode::On)]
                .iter()
                .map(|(l, m)| ((*l).to_owned(), shared.xpdr_mode == *m))
                .chain([("IDENT".to_owned(), shared.ident_remaining_s > 0.)])
                .collect(),
            _ => vec![("XFER".to_owned(), false)],
        };
        if let Some(i) = strip(ui, y, &buttons, "efis-pad-extra") {
            match d.overlay {
                Overlay::TransponderWindow if i == 3 => out.glass.push(GlassSetting::Ident {}),
                Overlay::TransponderWindow => {
                    let mode = [Mode::Standby, Mode::Alt, Mode::On]
                        .get(i)
                        .copied()
                        .unwrap_or(Mode::On);
                    act(out, id, DisplayAction::TransponderMode { mode });
                }
                Overlay::NavWindow => act(
                    out,
                    id,
                    DisplayAction::Transfer {
                        band: flightcraft_control::RadioBand::Nav,
                    },
                ),
                _ => act(
                    out,
                    id,
                    DisplayAction::Transfer {
                        band: flightcraft_control::RadioBand::Com,
                    },
                ),
            }
        }
        y += unit;
    }
    if !shortcuts.is_empty() {
        let buttons: Vec<(String, bool)> = shortcuts.iter().map(|s| (s.label().to_owned(), false)).collect();
        if let Some(i) = strip(ui, y, &buttons, "efis-pad-shortcut")
            && let Some(shortcut) = shortcuts.get(i)
        {
            act(
                out,
                id,
                DisplayAction::Shortcut {
                    target,
                    shortcut: *shortcut,
                },
            );
        }
        y += unit;
    }
    let ok = parsed.is_ok();
    // A hardware keyboard types into the pad as well; Enter accepts and Escape cancels.
    let mut accept = false;
    let mut cancel = false;
    for event in ui.input(|i| i.events.clone()) {
        match event {
            egui::Event::Text(typed) => {
                for c in typed.chars() {
                    text = touch_entry::press(target, hpa, &text, PadKey::Char(c));
                }
            }
            egui::Event::Key {
                key: egui::Key::Backspace,
                pressed: true,
                ..
            } => text = touch_entry::press(target, hpa, &text, PadKey::Backspace),
            egui::Event::Key {
                key: egui::Key::Enter,
                pressed: true,
                ..
            } => accept = true,
            egui::Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                ..
            } => cancel = true,
            _ => {}
        }
    }
    match strip(
        ui,
        y,
        &[("ENTER".into(), ok), ("CANCEL".into(), false)],
        "efis-pad-final",
    ) {
        Some(0) => accept = true,
        Some(_) => cancel = true,
        None => {}
    }
    if accept && touch_entry::parse(target, hpa, &text).is_ok() {
        act(
            out,
            id,
            DisplayAction::Enter {
                target,
                text: text.clone(),
            },
        );
        text.clear();
    } else if cancel {
        act(out, id, DisplayAction::CloseOverlay {});
        text.clear();
    }
    ui.data_mut(|x| x.insert_temp(key, (Some(d.overlay), text)));
}

/// The active flight plan page with touch editing (pilot's guide 6.5 and 6.6, pages 191 to 193):
/// touch a waypoint to select it, then Activate Leg, Insert Before, Insert After, or Remove.
/// The CAS message list page: every active message, warnings first and newest first within a
/// severity, in the guide's colours. An unacknowledged message is highlighted and flashes; touching
/// its row acknowledges it, and an acknowledged one stays listed until its condition clears.
pub(crate) fn cas_page(ui: &mut egui::Ui, area: Rect, style: &Style, shared: &Shared, out: &mut Output) {
    use flightcraft_control::touch_cas::CasSeverity;
    ui.painter().rect_filled(area, 0., crate::theme::BACKGROUND);
    let title_h = area.height() * 0.085;
    ui.painter().text(
        area.left_top() + Vec2::new(8., title_h * 0.5),
        Align2::LEFT_CENTER,
        "CAS MESSAGES",
        style.font(title_h * 0.6),
        style.key_text,
    );
    let list = Rect::from_min_max(Pos2::new(area.left(), area.top() + title_h), area.max);
    let row_h = (area.height() * 0.085).max(8.);
    let visible = ((list.height() / row_h).floor() as usize).max(1);
    let messages = shared.cas.list();
    if messages.is_empty() {
        ui.painter().text(
            list.center_top() + Vec2::new(0., row_h),
            Align2::CENTER_CENTER,
            "No active messages",
            style.font(row_h * 0.6),
            crate::theme::MUTED,
        );
    }
    let flash_on = (ui.input(|i| i.time) * 2.).floor() as i64 % 2 == 0;
    for (i, m) in messages.iter().enumerate().take(visible) {
        let rect = Rect::from_min_size(
            list.left_top() + Vec2::new(4., i as f32 * row_h),
            Vec2::new(list.width() - 8., row_h - 2.),
        );
        let ink = match m.severity {
            CasSeverity::Warning => crate::theme::FACE_RED,
            CasSeverity::Caution => crate::theme::FACE_YELLOW,
            CasSeverity::Advisory => crate::theme::FACE_WHITE,
        };
        let unacknowledged = !m.acknowledged;
        let painter = ui.painter();
        painter.rect(
            rect,
            2.,
            if unacknowledged {
                Color32::from_black_alpha(160)
            } else {
                Color32::from_black_alpha(60)
            },
            Stroke::new(
                if unacknowledged { 2.0_f32 } else { 1.0_f32 },
                if unacknowledged { ink } else { style.trim },
            ),
            egui::StrokeKind::Inside,
        );
        if !unacknowledged || flash_on {
            painter.text(
                rect.left_center() + Vec2::new(8., 0.),
                Align2::LEFT_CENTER,
                &m.text,
                style.font(row_h * 0.55),
                ink,
            );
        }
        if unacknowledged {
            painter.text(
                rect.right_center() - Vec2::new(8., 0.),
                Align2::RIGHT_CENTER,
                "TOUCH TO ACKNOWLEDGE",
                style.font(row_h * 0.35),
                crate::theme::MUTED,
            );
            if hit(ui, rect, ("cas-row", i)) {
                out.glass.push(GlassSetting::CasAcknowledge { index: i });
            }
        }
    }
}

pub(crate) fn flight_plan_page(
    ui: &mut egui::Ui,
    area: Rect,
    id: &str,
    style: &Style,
    shared: &Shared,
    out: &mut Output,
) {
    use flightcraft_control::touch_fpl::FplEdit;
    ui.painter().rect_filled(area, 0., crate::theme::BACKGROUND);
    let title_h = area.height() * 0.085;
    ui.painter().text(
        area.left_top() + Vec2::new(8., title_h * 0.5),
        Align2::LEFT_CENTER,
        "ACTIVE FLIGHT PLAN",
        style.font(title_h * 0.6),
        style.key_text,
    );
    let buttons_h = area.height() * 0.11 * 2.;
    let list = Rect::from_min_max(
        Pos2::new(area.left(), area.top() + title_h),
        Pos2::new(area.right(), area.bottom() - buttons_h - 4.),
    );
    let row_h = (area.height() * 0.085).max(8.);
    let visible = ((list.height() / row_h).floor() as usize).max(1);
    let rows = &shared.fpl.rows;
    // Keep the selected waypoint on screen.
    let first = shared
        .fpl
        .cursor
        .saturating_sub(visible - 1)
        .min(rows.len().saturating_sub(visible));
    if rows.is_empty() {
        ui.painter().text(
            list.center_top() + Vec2::new(0., row_h),
            Align2::CENTER_CENTER,
            "No flight plan: Insert After adds a waypoint",
            style.font(row_h * 0.6),
            crate::theme::MUTED,
        );
    }
    for (i, row) in rows.iter().enumerate().skip(first).take(visible) {
        let rect = Rect::from_min_size(
            list.left_top() + Vec2::new(4., (i - first) as f32 * row_h),
            Vec2::new(list.width() - 8., row_h - 2.),
        );
        let selected = i == shared.fpl.cursor;
        let active = i == shared.fpl.active;
        let painter = ui.painter();
        painter.rect(
            rect,
            2.,
            if selected {
                style.trim
            } else {
                Color32::from_black_alpha(120)
            },
            Stroke::new(1.0_f32, if selected { style.key_text } else { style.trim }),
            egui::StrokeKind::Inside,
        );
        let ink = if active {
            Color32::from_rgb(224, 64, 224)
        } else {
            Color32::WHITE
        };
        let font = style.font(row_h * 0.55);
        painter.text(
            rect.left_center() + Vec2::new(6., 0.),
            Align2::LEFT_CENTER,
            format!("{:>2}{}", i + 1, if active { ">" } else { " " }),
            font.clone(),
            ink,
        );
        painter.text(
            rect.left_center() + Vec2::new(rect.width() * 0.12, 0.),
            Align2::LEFT_CENTER,
            &row.ident,
            font.clone(),
            ink,
        );
        painter.text(
            rect.left_center() + Vec2::new(rect.width() * 0.45, 0.),
            Align2::LEFT_CENTER,
            format!("{:03.0} deg", row.bearing_deg.rem_euclid(360.)),
            font.clone(),
            ink,
        );
        painter.text(
            rect.left_center() + Vec2::new(rect.width() * 0.62, 0.),
            Align2::LEFT_CENTER,
            row.constraint.as_deref().unwrap_or("-----"),
            font.clone(),
            if row.constraint.is_some() {
                ink
            } else {
                crate::theme::MUTED
            },
        );
        painter.text(
            rect.right_center() - Vec2::new(8., 0.),
            Align2::RIGHT_CENTER,
            format!("{:.1} NM", row.distance_nm),
            font,
            ink,
        );
        if hit(ui, rect, ("fpl-row", i)) {
            act(
                out,
                id,
                DisplayAction::FlightPlan {
                    edit: FplEdit::Select { index: i },
                },
            );
        }
    }
    let bar = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - buttons_h), area.max);
    // Two rows: the list edits, then the selected waypoint's altitude constraint (VNAV).
    let labels = [
        "Activate Leg",
        "Insert Before",
        "Insert After",
        "Remove",
        "Alt At",
        "Alt At/Above",
        "Alt At/Below",
        "Alt Clear",
    ];
    let columns = labels.len() / 2;
    let width = bar.width() / columns as f32;
    let height = bar.height() / 2.;
    for (i, label) in labels.iter().enumerate() {
        let rect = Rect::from_min_size(
            Pos2::new(
                bar.left() + width * (i % columns) as f32,
                bar.top() + height * (i / columns) as f32,
            ),
            Vec2::new(width, height),
        )
        .shrink2(Vec2::new(3., 2.));
        // Only Insert After works on an empty plan: the other three act on a selected waypoint.
        if shared.fpl.rows.is_empty() && i != 2 {
            ui.painter().rect_filled(rect, 2., Color32::from_black_alpha(80));
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                *label,
                style.font(rect.height() * 0.4),
                crate::theme::MUTED,
            );
            continue;
        }
        if chip(ui, rect, label, false, style, ("fpl-button", i)) {
            match i {
                0 => act(
                    out,
                    id,
                    DisplayAction::FlightPlan {
                        edit: FplEdit::Activate {},
                    },
                ),
                1 => act(
                    out,
                    id,
                    DisplayAction::OpenOverlay {
                        overlay: Overlay::Keypad(EntryTarget::FplBefore),
                    },
                ),
                2 => act(
                    out,
                    id,
                    DisplayAction::OpenOverlay {
                        overlay: Overlay::Keypad(EntryTarget::FplAfter),
                    },
                ),
                4..=6 => act(
                    out,
                    id,
                    DisplayAction::OpenOverlay {
                        overlay: Overlay::Keypad(match i {
                            4 => EntryTarget::FplAltitudeAt,
                            5 => EntryTarget::FplAltitudeAbove,
                            _ => EntryTarget::FplAltitudeBelow,
                        }),
                    },
                ),
                7 => act(
                    out,
                    id,
                    DisplayAction::FlightPlan {
                        edit: FplEdit::Altitude { constraint: None },
                    },
                ),
                _ => act(
                    out,
                    id,
                    DisplayAction::FlightPlan {
                        edit: FplEdit::Remove {},
                    },
                ),
            }
        }
    }
}
