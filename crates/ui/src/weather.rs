//! Weather controls use the same validated commands as MCP and timed scenario scripts.
use crate::keymap::{Action, Keymap};
use eframe::egui;
use flightcraft_control::{CommandError, Session, SetHood, SetWeather};
use flightcraft_world::{CloudLayer, Coverage, Obscuration, Precipitation, TimeOfDay, WeatherPreset};

pub fn note(session: &Session) -> String {
    let c = session.sim().conditions();
    let layers = c
        .layers
        .iter()
        .map(|l| format!("{:?} {:.0}–{:.0} MSL", l.coverage, l.base_ft_msl, l.top_ft_msl))
        .collect::<Vec<_>>()
        .join(" / ");
    format!(
        "{} · {:.1} SM{}{} · {:?}{}{}",
        if layers.is_empty() { "CLR" } else { &layers },
        c.effective_visibility_m() / 1609.344,
        if c.obscuration == Obscuration::None {
            String::new()
        } else {
            format!(" · {:?}", c.obscuration)
        },
        if c.precipitation == Precipitation::None {
            String::new()
        } else {
            format!(" · {:?}", c.precipitation)
        },
        c.time_of_day,
        if c.in_cloud(session.sim().state().position_m) {
            " · IN CLOUD"
        } else {
            ""
        },
        if session.sim().hood() { " · HOOD" } else { "" }
    )
}
pub fn panel(ui: &mut egui::Ui, session: &mut Session, keymap: &Keymap) -> Result<(), CommandError> {
    ui.label("Weather · cloud heights ft MSL");
    for (label, preset) in [
        ("VFR", WeatherPreset::Vfr),
        ("MVFR", WeatherPreset::Mvfr),
        ("IFR · 600 OVC 2SM BR", WeatherPreset::Ifr),
        ("LIFR · 200 OVC ½SM FG", WeatherPreset::Lifr),
        ("Night", WeatherPreset::Night),
    ] {
        if ui.button(label).clicked() {
            session.run(SetWeather {
                preset: Some(preset),
                ..Default::default()
            })?;
        }
    }
    ui.separator();
    let mut c = session.sim().conditions().clone();
    let mut changed = ui
        .add(
            egui::Slider::new(&mut c.visibility_sm, 0.125..=100.)
                .logarithmic(true)
                .text("Visibility SM"),
        )
        .changed();
    ui.horizontal(|ui| {
        changed |= ui
            .add(
                egui::DragValue::new(&mut c.temperature_c)
                    .range(-60.0..=60.0)
                    .prefix("Sea-level temp C "),
            )
            .changed();
        changed |= ui
            .add(
                egui::DragValue::new(&mut c.dewpoint_c)
                    .range(-100.0..=c.temperature_c)
                    .prefix("Dew point C "),
            )
            .changed();
    });
    let mut altimeter = c.sea_level_pressure_pa / 3386.389;
    if ui
        .add(
            egui::DragValue::new(&mut altimeter)
                .range(23.63..=32.48)
                .speed(0.01)
                .prefix("Altimeter inHg "),
        )
        .changed()
    {
        c.sea_level_pressure_pa = altimeter * 3386.389;
        changed = true;
    }
    for (index, layer) in c.layers.iter_mut().enumerate() {
        ui.push_id(index, |ui| {
            ui.horizontal(|ui| {
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut layer.base_ft_msl)
                            .speed(50.)
                            .prefix("Base "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut layer.top_ft_msl)
                            .speed(50.)
                            .prefix("Top "),
                    )
                    .changed();
                egui::ComboBox::from_id_salt("coverage")
                    .selected_text(format!("{:?}", layer.coverage))
                    .show_ui(ui, |ui| {
                        for coverage in [Coverage::Few, Coverage::Sct, Coverage::Bkn, Coverage::Ovc] {
                            changed |= ui
                                .selectable_value(&mut layer.coverage, coverage, format!("{coverage:?}"))
                                .changed();
                        }
                    });
            });
        });
    }
    ui.horizontal(|ui| {
        if c.layers.len() < 3 && ui.button("Add layer").clicked() {
            c.layers.push(CloudLayer {
                base_ft_msl: 2000.,
                top_ft_msl: 4000.,
                coverage: Coverage::Ovc,
            });
            changed = true;
        }
        if !c.layers.is_empty() && ui.button("Remove last layer").clicked() {
            c.layers.pop();
            changed = true;
        }
    });
    egui::ComboBox::from_id_salt("obscuration")
        .selected_text(format!("{:?}", c.obscuration))
        .show_ui(ui, |ui| {
            for v in [
                Obscuration::None,
                Obscuration::Mist,
                Obscuration::Fog,
                Obscuration::Haze,
            ] {
                changed |= ui
                    .selectable_value(&mut c.obscuration, v, format!("{v:?}"))
                    .changed();
            }
        });
    egui::ComboBox::from_id_salt("precipitation")
        .selected_text(format!("{:?}", c.precipitation))
        .show_ui(ui, |ui| {
            for v in [Precipitation::None, Precipitation::Rain, Precipitation::Snow] {
                changed |= ui
                    .selectable_value(&mut c.precipitation, v, format!("{v:?}"))
                    .changed();
            }
        });
    egui::ComboBox::from_id_salt("time-of-day")
        .selected_text(format!("{:?}", c.time_of_day))
        .show_ui(ui, |ui| {
            for v in [TimeOfDay::Day, TimeOfDay::Dusk, TimeOfDay::Night, TimeOfDay::Dawn] {
                changed |= ui
                    .selectable_value(&mut c.time_of_day, v, format!("{v:?}"))
                    .changed();
            }
        });
    if changed {
        session.run(SetWeather {
            temperature_c: Some(c.temperature_c),
            dewpoint_c: Some(c.dewpoint_c),
            sea_level_pressure_pa: Some(c.sea_level_pressure_pa),
            layers: Some(c.layers),
            visibility_sm: Some(c.visibility_sm),
            precipitation: Some(c.precipitation),
            obscuration: Some(c.obscuration),
            time_of_day: Some(c.time_of_day),
            ..Default::default()
        })?;
    }
    ui.label("HUD visibility includes rain or snow reduction.");
    ui.separator();
    let mut wind = session.pilot_weather();
    let mut wind_changed = false;
    ui.horizontal(|ui| {
        wind_changed |= ui
            .add(
                egui::DragValue::new(&mut wind.wind_from_deg)
                    .range(0. ..=360.)
                    .prefix("Wind from °M "),
            )
            .changed();
        wind_changed |= ui
            .add(
                egui::DragValue::new(&mut wind.wind_speed_mps)
                    .range(0. ..=75.)
                    .prefix("m/s "),
            )
            .changed();
    });
    wind_changed |= ui.checkbox(&mut wind.shear, "Surface wind shear").changed();
    egui::ComboBox::from_id_salt("turbulence")
        .selected_text(format!("{:?}", wind.turbulence))
        .show_ui(ui, |ui| {
            for v in [
                flightcraft_world::TurbulenceLevel::Off,
                flightcraft_world::TurbulenceLevel::Light,
                flightcraft_world::TurbulenceLevel::Moderate,
                flightcraft_world::TurbulenceLevel::Severe,
            ] {
                wind_changed |= ui
                    .selectable_value(&mut wind.turbulence, v, format!("{v:?}"))
                    .changed();
            }
        });
    if wind_changed {
        session.run(SetWeather {
            wind_from_deg: Some(wind.wind_from_deg),
            wind_speed_mps: Some(wind.wind_speed_mps),
            shear: Some(wind.shear),
            turbulence: Some(wind.turbulence),
            seed: Some(wind.seed),
            ..Default::default()
        })?;
    }
    ui.separator();
    let mut hood = session.sim().hood();
    if ui
        .checkbox(&mut hood, format!("Hood [{}]", keymap.label(Action::Hood)))
        .changed()
    {
        session.run(SetHood { enabled: hood })?;
    }
    let time = session.sim().instrument_time();
    ui.label(format!(
        "Simulated instrument practice · hood {:.1} min · IMC {:.1} min",
        time.hood_s / 60.,
        time.imc_s / 60.
    ));
    ui.label("Simulation only; no FAA logbook credit.");
    Ok(())
}
