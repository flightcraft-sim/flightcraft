//! Spoken automated observations. Cloud bases convert from weather MSL to station AGL.
use super::*;
use flightcraft_world::{Conditions, Coverage, Obscuration, Precipitation};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportedLayer {
    pub coverage: Coverage,
    pub base_agl_m: f64,
}
use super::phraseology::digits;
fn temperature(value: f64) -> String {
    format!(
        "{}{}",
        if value.round() < 0. { "minus " } else { "" },
        digits(value.abs().round() as u32, 1)
    )
}
pub(crate) fn height(value: f64) -> String {
    let hundreds = (value * 3.280839895 / 100.).round().max(0.) as u32;
    if hundreds == 0 {
        "less than one hundred".into()
    } else {
        phraseology::altitude(hundreds * 100)
    }
}
pub(super) fn weather_report(
    station: &ComStation,
    c: &Conditions,
    wind: flightcraft_sim::Weather,
    _time_s: f64,
) -> (String, Meaning) {
    let mut layers = c
        .layers
        .iter()
        .map(|l| ReportedLayer {
            coverage: l.coverage,
            base_agl_m: (l.base_ft_msl * 0.3048 - station.elevation_m).max(0.),
        })
        .collect::<Vec<_>>();
    layers.sort_by(|a, b| a.base_agl_m.total_cmp(&b.base_agl_m));
    let ceiling = layers
        .iter()
        .filter(|l| matches!(l.coverage, Coverage::Bkn | Coverage::Ovc))
        .map(|l| l.base_agl_m)
        .min_by(f64::total_cmp);
    let wind_words = phraseology::wind(
        wind.wind_from_deg.round() as u16,
        (wind.wind_speed_mps / 0.514444).round() as u16,
    );
    let wind_text = format!(
        "Wind{}{}.",
        wind_words.strip_prefix("wind").unwrap_or(&wind_words),
        if wind_words == "wind calm" { "" } else { " knots" }
    );
    let sm = c.effective_visibility_m() / 1609.344;
    let whole = sm.floor() as u32;
    let fraction = (((sm - f64::from(whole)) * 8.).round() as u32).min(7);
    let fraction = match fraction {
        1 => "one eighth",
        2 => "one quarter",
        3 => "three eighths",
        4 => "one half",
        5 => "five eighths",
        6 => "three quarters",
        7 => "seven eighths",
        _ => "",
    };
    let visibility = if sm >= 10. {
        "one zero or more".into()
    } else if fraction.is_empty() {
        digits(whole, 1)
    } else if whole == 0 {
        fraction.into()
    } else {
        format!("{} and {fraction}", digits(whole, 1))
    };
    let sky = if layers.is_empty() {
        "Sky clear.".into()
    } else {
        layers
            .iter()
            .map(|l| {
                format!(
                    "{}{} {}.",
                    if Some(l.base_agl_m) == ceiling {
                        "Ceiling "
                    } else {
                        ""
                    },
                    match l.coverage {
                        Coverage::Few => "few clouds at",
                        Coverage::Sct => "scattered clouds at",
                        Coverage::Bkn => "broken",
                        Coverage::Ovc => "overcast",
                    },
                    height(l.base_agl_m)
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let phenomena = [
        match c.obscuration {
            Obscuration::None => "",
            Obscuration::Mist => "Mist.",
            Obscuration::Fog => "Fog.",
            Obscuration::Haze => "Haze.",
        },
        match c.precipitation {
            Precipitation::None => "",
            Precipitation::Rain => "Rain.",
            Precipitation::Snow => "Snow.",
        },
    ]
    .into_iter()
    .filter(|v| !v.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    let temp = c.temperature_c - 0.0065 * station.elevation_m;
    let dew = (c.dewpoint_c - 0.002 * station.elevation_m).min(temp);
    let alt = (c.sea_level_pressure_pa / 3386.389 * 100.).round() as u32;
    // Do not announce an invented wall-clock/Zulu observation timestamp.
    let text = format!(
        "{} automated weather observation. {wind_text} Visibility {visibility} statute miles. {phenomena} {sky} Temperature {} Celsius. Dew point {} Celsius. Altimeter {}.",
        phraseology::facility(station),
        temperature(temp),
        temperature(dew),
        phraseology::altimeter(alt)
    );
    (
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        Meaning::Weather {
            airport: station.airport.clone(),
            wind_from_deg: wind.wind_from_deg,
            wind_speed_mps: wind.wind_speed_mps,
            visibility_m: Some(c.effective_visibility_m()),
            ceiling_agl_m: ceiling,
            altimeter_pa: c.sea_level_pressure_pa,
            temperature_c: temp,
            dewpoint_c: dew,
            layers,
            obscuration: c.obscuration,
            precipitation: c.precipitation,
        },
    )
}
