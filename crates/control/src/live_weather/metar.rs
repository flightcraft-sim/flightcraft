//! Tolerant, bounded METAR decoding. Missing or unsupported groups stay unknown.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub station: Option<String>,
    /// DDHHMMZ, resolved against retrieval time by the source runtime.
    pub time: Option<String>,
    pub automatic: Option<bool>,
    pub corrected: Option<bool>,
    pub wind: Option<Wind>,
    pub visibility_sm: Option<f64>,
    /// Some(empty) means explicitly clear; None means unreported.
    pub layers: Option<Vec<Layer>>,
    pub temperature_c: Option<f64>,
    pub dewpoint_c: Option<f64>,
    pub altimeter_inhg: Option<f64>,
    pub present_weather: Option<Vec<String>>,
    pub runway_in_use: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wind {
    /// METAR direction is true, including the DigiWx preset's wind fields.
    pub from_true_deg: Option<f64>,
    pub variable: bool,
    pub speed_kt: f64,
    pub gust_kt: Option<f64>,
    pub variable_sector_deg: Option<[f64; 2]>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// FEW, SCT, BKN, OVC, or VV. Unknown heights remain None.
    pub coverage: String,
    pub base_agl_ft: Option<f64>,
}
impl Observation {
    pub fn validate(&self) -> Result<(), String> {
        let bounded =
            |v: Option<f64>, min: f64, max: f64| v.is_none_or(|n| n.is_finite() && (min..=max).contains(&n));
        if self
            .station
            .as_ref()
            .is_some_and(|s| s.len() > 8 || !s.bytes().all(|b| b.is_ascii_alphanumeric()))
            || self.time.as_ref().is_some_and(|s| {
                s.len() != 7
                    || !s.ends_with('Z')
                    || !s.get(..6).is_some_and(|v| v.bytes().all(|b| b.is_ascii_digit()))
            })
            || !bounded(self.temperature_c, -100., 100.)
            || !bounded(self.dewpoint_c, -100., 100.)
            || !bounded(self.visibility_sm, 0., 100.)
            || !bounded(self.altimeter_inhg, 23.63, 32.49)
            || self.layers.as_ref().is_some_and(|layers| {
                layers.len() > 32
                    || layers.iter().any(|l| {
                        !matches!(l.coverage.as_str(), "FEW" | "SCT" | "BKN" | "OVC" | "VV")
                            || !bounded(l.base_agl_ft, 0., 60000.)
                    })
            })
            || self.wind.as_ref().is_some_and(|w| {
                !bounded(w.from_true_deg, 0., 360.)
                    || !bounded(Some(w.speed_kt), 0., 300.)
                    || !bounded(w.gust_kt, 0., 400.)
                    || w.variable_sector_deg
                        .is_some_and(|a| a.into_iter().any(|v| !bounded(Some(v), 0., 360.)))
            })
            || self.present_weather.as_ref().is_some_and(|w| {
                w.len() > 32
                    || w.iter().any(|v| {
                        v.len() > 8
                            || !v
                                .bytes()
                                .all(|b| b.is_ascii_uppercase() || matches!(b, b'-' | b'+'))
                    })
            })
            || self.runway_in_use.as_ref().is_some_and(|s| {
                s.len() > 3
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_digit() || matches!(b, b'L' | b'R' | b'C'))
            })
        {
            return Err("Observation fields exceed supported bounds".into());
        }
        Ok(())
    }
    pub fn usable(&self) -> bool {
        self.wind.is_some()
            || self.visibility_sm.is_some()
            || self.layers.is_some()
            || self.temperature_c.is_some()
            || self.altimeter_inhg.is_some()
    }
}
fn number(text: &str, min: f64, max: f64) -> Option<f64> {
    text.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && (min..=max).contains(v))
}
pub fn temperature(text: &str) -> Option<f64> {
    let text = text.trim();
    let value = if let Some(v) = text.strip_prefix('M') {
        -number(v, 0., 100.)?
    } else {
        number(text, -100., 100.)?
    };
    Some(value)
}
pub fn visibility(text: &str) -> Option<f64> {
    let clean = text
        .trim()
        .trim_end_matches("SM")
        .trim()
        .trim_start_matches(['P', 'M']);
    let mut parts = clean.split_whitespace();
    let first = parts.next()?;
    let fraction = |s: &str| -> Option<f64> {
        if let Some((a, b)) = s.split_once('/') {
            Some(number(a, 0., 100.)? / number(b, 1., 100.)?)
        } else {
            number(s, 0., 100.)
        }
    };
    let value = fraction(first)? + parts.next().and_then(fraction).unwrap_or(0.);
    (value <= 100.).then_some(value)
}
pub fn layer(text: &str) -> Option<Layer> {
    let coverage = ["FEW", "SCT", "BKN", "OVC", "VV"]
        .into_iter()
        .find(|p| text.starts_with(p))?;
    let height = text.get(coverage.len()..coverage.len() + 3)?;
    Some(Layer {
        coverage: coverage.into(),
        base_agl_ft: number(height, 0., 600.).map(|v| v * 100.),
    })
}
pub fn wind(text: &str) -> Option<Wind> {
    let (value, factor) = if let Some(v) = text.strip_suffix("MPS") {
        (v, 1.94384449)
    } else {
        (text.strip_suffix("KT").unwrap_or(text), 1.)
    };
    let direction = value.get(..3)?;
    let rest = value.get(3..)?;
    let (speed, gust) = rest.split_once('G').map_or((rest, None), |(s, g)| (s, Some(g)));
    if !(2..=3).contains(&speed.len()) {
        return None;
    }
    Some(Wind {
        from_true_deg: if direction == "VRB" {
            None
        } else {
            Some(number(direction, 0., 360.)?)
        },
        variable: direction == "VRB",
        speed_kt: number(speed, 0., 145.)? * factor,
        gust_kt: gust.and_then(|v| number(v, 0., 200.)).map(|v| v * factor),
        variable_sector_deg: None,
    })
}
/// At most 16 KiB / 512 groups. Garbage is ignored, never indexed without bounds checks.
pub fn parse(text: &str) -> Observation {
    if text.len() > 16 * 1024 {
        return Observation::default();
    }
    let upper = text.to_ascii_uppercase();
    let tokens = upper.split_whitespace().take(512).collect::<Vec<_>>();
    let mut o = Observation::default();
    let mut after_wind = false;
    for (i, token) in tokens.iter().enumerate() {
        let t = token.trim_end_matches('=');
        if t == "RMK" {
            break;
        }
        if matches!(t, "METAR" | "SPECI") {
            if o.usable() {
                break;
            }
            continue;
        }
        if t == "AUTO" {
            o.automatic = Some(true);
            continue;
        }
        if t == "COR" {
            o.corrected = Some(true);
            continue;
        }
        if t.len() == 7
            && t.ends_with('Z')
            && t.get(..6).is_some_and(|v| v.bytes().all(|c| c.is_ascii_digit()))
        {
            o.time = Some(t.into());
            continue;
        }
        if o.station.is_none()
            && o.time.is_none()
            && (3..=4).contains(&t.len())
            && t.bytes().all(|c| c.is_ascii_alphanumeric())
            && t.bytes().any(|c| c.is_ascii_alphabetic())
        {
            o.station = Some(t.into());
            continue;
        }
        if t.ends_with("KT")
            || t.ends_with("MPS")
            || (o.wind.is_none() && t.len() == 5 && t.bytes().all(|b| b.is_ascii_digit()))
        {
            if let Some(w) = wind(t) {
                o.wind = Some(w);
                after_wind = true;
            }
            continue;
        }
        if t.len() == 7
            && let Some((a, b)) = t.split_once('V')
            && let Some(w) = &mut o.wind
        {
            w.variable_sector_deg = number(a, 0., 360.).zip(number(b, 0., 360.)).map(|(a, b)| [a, b]);
            continue;
        }
        if t.ends_with("SM") {
            let previous = i
                .checked_sub(1)
                .and_then(|j| tokens.get(j))
                .copied()
                .unwrap_or("");
            let joined = if t == "SM" || t.contains('/') && number(previous, 0., 99.).is_some() {
                if t == "SM" && previous.contains('/') {
                    let whole = i
                        .checked_sub(2)
                        .and_then(|j| tokens.get(j))
                        .copied()
                        .filter(|s| number(s, 0., 99.).is_some())
                        .unwrap_or("");
                    format!("{whole} {previous} {t}")
                } else {
                    format!("{previous} {t}")
                }
            } else {
                t.into()
            };
            o.visibility_sm = visibility(&joined);
            continue;
        }
        if t == "CAVOK" {
            o.visibility_sm = Some(6.21371);
            o.layers = Some(Vec::new());
            continue;
        }
        if after_wind && t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit()) {
            o.visibility_sm = number(t, 0., 9999.).map(|v| v / 1609.344);
            after_wind = false;
            continue;
        }
        if matches!(t, "CLR" | "SKC" | "NSC" | "NCD") {
            o.layers = Some(Vec::new());
            continue;
        }
        if let Some(l) = layer(t) {
            o.layers.get_or_insert_with(Vec::new).push(l);
            continue;
        }
        if let Some(value) = t.strip_prefix('A').filter(|v| v.len() == 4) {
            o.altimeter_inhg = number(value, 2363., 3248.).map(|v| v / 100.);
            continue;
        }
        if let Some(value) = t.strip_prefix('Q').filter(|v| v.len() == 4) {
            o.altimeter_inhg = number(value, 800., 1100.).map(|v| v / 33.86389);
            continue;
        }
        if let Some((a, b)) = t.split_once('/')
            && a.len() <= 3
            && b.len() <= 3
        {
            if let Some(v) = temperature(a) {
                o.temperature_c = Some(v);
            }
            if let Some(v) = temperature(b) {
                o.dewpoint_c = Some(v);
            }
            continue;
        }
        let phenomena = t.trim_start_matches(['-', '+']);
        if !phenomena.is_empty()
            && phenomena.len() <= 8
            && phenomena.len().is_multiple_of(2)
            && phenomena.as_bytes().as_chunks::<2>().0.iter().all(|c| {
                matches!(
                    c,
                    b"RA" | b"SN" | b"BR" | b"FG" | b"HZ" | b"TS" | b"SH" | b"DZ" | b"FZ" | b"VC" | b"UP"
                )
            })
        {
            o.present_weather.get_or_insert_with(Vec::new).push(t.into());
        }
    }
    if o.usable() && o.time.is_some() && o.present_weather.is_none() {
        o.present_weather = Some(Vec::new());
    }
    o
}
