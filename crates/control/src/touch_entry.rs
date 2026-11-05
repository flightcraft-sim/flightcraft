//! Touch keypad entry for the landscape touch EFIS: what each box accepts, how typing edits the
//! field, and how an entered value becomes a setting.
//!
//! The pilot's guide describes one keypad for every entry: touch a box on the PFD (selected
//! altitude, selected heading, course, baro, a COM or NAV standby frequency, the transponder
//! code) or the DIRECT key's waypoint field, type, and touch Enter. Some boxes also offer a
//! shortcut button: Set To Current (heading, altitude, course), Set To Standard and Set For Field
//! (baro). This module is pure; nothing reads a session or a clock.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a keypad is entering a value for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntryTarget {
    /// Altimeter barometric setting, inHg or hPa as the display's units.
    Baro,
    /// Selected heading, degrees magnetic.
    Heading,
    /// Selected altitude, feet.
    Altitude,
    /// Selected course (OBS), degrees magnetic.
    Course,
    /// Barometric minimums altitude, feet.
    Minimums,
    /// Direct-to waypoint identifier.
    DirectTo,
    /// Standby frequency of the first communication radio.
    ComStandby,
    /// Standby frequency of the first navigation radio.
    NavStandby,
    /// Transponder code.
    Transponder,
    /// Identifier of a waypoint to insert before the selected flight-plan waypoint.
    FplBefore,
    /// Identifier of a waypoint to insert after the selected flight-plan waypoint.
    FplAfter,
    /// Altitude the selected flight-plan waypoint is crossed at, feet.
    FplAltitudeAt,
    /// Altitude the selected flight-plan waypoint is crossed at or above, feet.
    FplAltitudeAbove,
    /// Altitude the selected flight-plan waypoint is crossed at or below, feet.
    FplAltitudeBelow,
}

impl EntryTarget {
    pub const ALL: [Self; 14] = [
        Self::Baro,
        Self::Heading,
        Self::Altitude,
        Self::Course,
        Self::Minimums,
        Self::DirectTo,
        Self::ComStandby,
        Self::NavStandby,
        Self::Transponder,
        Self::FplBefore,
        Self::FplAfter,
        Self::FplAltitudeAt,
        Self::FplAltitudeAbove,
        Self::FplAltitudeBelow,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Baro => "BARO",
            Self::Heading => "HEADING",
            Self::Altitude => "ALTITUDE",
            Self::Course => "COURSE",
            Self::Minimums => "MINIMUMS",
            Self::DirectTo => "DIRECT-TO",
            Self::ComStandby => "COM1 STANDBY",
            Self::NavStandby => "NAV1 STANDBY",
            Self::Transponder => "XPDR CODE",
            Self::FplBefore => "INSERT BEFORE",
            Self::FplAfter => "INSERT AFTER",
            Self::FplAltitudeAt => "WPT ALTITUDE",
            Self::FplAltitudeAbove => "WPT AT OR ABOVE",
            Self::FplAltitudeBelow => "WPT AT OR BELOW",
        }
    }

    /// Longest entry, in characters, including a decimal point.
    pub fn max_len(self) -> usize {
        match self {
            Self::Baro => 5,
            Self::Heading | Self::Course => 3,
            Self::Altitude | Self::Minimums => 5,
            Self::FplAltitudeAt | Self::FplAltitudeAbove | Self::FplAltitudeBelow => 5,
            Self::DirectTo | Self::FplBefore | Self::FplAfter => 6,
            Self::ComStandby | Self::NavStandby => 7,
            Self::Transponder => 4,
        }
    }

    /// Whether letters are accepted (waypoint identifiers) rather than digits only.
    pub fn alphanumeric(self) -> bool {
        matches!(self, Self::DirectTo | Self::FplBefore | Self::FplAfter)
    }

    /// Whether a decimal point is accepted. A baro in hPa is a whole number.
    pub fn decimal(self, hpa: bool) -> bool {
        match self {
            Self::Baro => !hpa,
            Self::ComStandby | Self::NavStandby => true,
            _ => false,
        }
    }

    /// Whether the box offers Set To Current.
    pub fn has_set_to_current(self) -> bool {
        matches!(self, Self::Heading | Self::Altitude | Self::Course)
    }

    /// The characters on the keypad, in rows. Numbers are a 3-column pad; a waypoint keypad
    /// adds the alphabet.
    pub fn rows(self, hpa: bool) -> Vec<Vec<char>> {
        if self.alphanumeric() {
            return ["ABCDEFG", "HIJKLMN", "OPQRSTU", "VWXYZ", "0123456789"]
                .iter()
                .map(|r| r.chars().collect())
                .collect();
        }
        let mut rows: Vec<Vec<char>> = ["123", "456", "789"]
            .iter()
            .map(|r| r.chars().collect::<Vec<char>>())
            .collect();
        rows.push(if self.decimal(hpa) {
            vec!['.', '0']
        } else {
            vec!['0']
        });
        rows
    }
}

/// A key on the keypad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadKey {
    Char(char),
    Backspace,
    Clear,
}

/// Apply one key to the text so far. Keys the target does not accept, and keys past the length
/// limit, change nothing. Letters are stored upper case.
pub fn press(target: EntryTarget, hpa: bool, text: &str, key: PadKey) -> String {
    let mut out = text.to_owned();
    match key {
        PadKey::Backspace => {
            out.pop();
        }
        PadKey::Clear => out.clear(),
        PadKey::Char(c) => {
            let c = c.to_ascii_uppercase();
            let ok = if c.is_ascii_digit() {
                true
            } else if c == '.' {
                target.decimal(hpa) && !out.contains('.')
            } else {
                target.alphanumeric() && c.is_ascii_uppercase()
            };
            if ok && out.chars().count() < target.max_len() {
                out.push(c);
            }
        }
    }
    out
}

/// A parsed, range-checked entry, in the engine's units.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Baro {
        pressure_pa: f64,
    },
    Heading {
        rad: f64,
    },
    Altitude {
        m: f64,
    },
    /// Barometric minimums altitude.
    Minimums {
        m: f64,
    },
    Course {
        rad: f64,
    },
    DirectTo {
        ident: String,
    },
    /// An altitude constraint for the selected flight-plan waypoint.
    FplAltitude {
        kind: flightcraft_sim::avionics::vnav::ConstraintKind,
        m: f64,
    },
    /// Insert a waypoint into the flight plan next to the selected one.
    FplInsert {
        ident: String,
        after: bool,
    },
    ComStandby {
        khz: u32,
    },
    NavStandby {
        khz: u32,
    },
    Transponder {
        code: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryError {
    Empty,
    Invalid(&'static str),
}

impl std::fmt::Display for EntryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("nothing entered"),
            Self::Invalid(why) => f.write_str(why),
        }
    }
}

const PA_PER_INHG: f64 = 3386.389;
const M_PER_FT: f64 = 0.3048;

fn whole(text: &str, max: u32, what: &'static str) -> Result<u32, EntryError> {
    if text.is_empty() {
        return Err(EntryError::Empty);
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EntryError::Invalid(what));
    }
    text.parse::<u32>()
        .ok()
        .filter(|v| *v <= max)
        .ok_or(EntryError::Invalid(what))
}

/// A frequency typed as `118.275`, `118275`, or `118.3` in kHz.
fn khz(text: &str) -> Result<u32, EntryError> {
    if text.is_empty() {
        return Err(EntryError::Empty);
    }
    let (whole_part, fraction) = match text.split_once('.') {
        Some((w, f)) => (w, f),
        // Six digits are MHz and kHz run together.
        None if text.len() == 6 => text.split_at(3),
        None => (text, ""),
    };
    if whole_part.is_empty()
        || fraction.len() > 3
        || !whole_part
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(EntryError::Invalid("not a frequency"));
    }
    let mhz: u32 = whole_part
        .parse()
        .map_err(|_| EntryError::Invalid("not a frequency"))?;
    let frac: u32 = format!("{fraction:0<3}")
        .parse()
        .map_err(|_| EntryError::Invalid("not a frequency"))?;
    Ok(mhz * 1000 + frac)
}

/// Parse and range-check an entry. `hpa` is the display's pressure units.
pub fn parse(target: EntryTarget, hpa: bool, text: &str) -> Result<Entry, EntryError> {
    let text = text.trim();
    match target {
        EntryTarget::Baro => {
            if text.is_empty() {
                return Err(EntryError::Empty);
            }
            if hpa {
                let v = whole(text, 2000, "baro must be a whole number of hPa")?;
                if !(900..=1100).contains(&v) {
                    return Err(EntryError::Invalid("baro must be 900 to 1100 hPa"));
                }
                Ok(Entry::Baro {
                    pressure_pa: f64::from(v) * 100.,
                })
            } else {
                // 2992 is read as 29.92.
                let text = if !text.contains('.') && text.len() == 4 {
                    format!("{}.{}", &text[..2], &text[2..])
                } else {
                    text.to_owned()
                };
                let v: f64 = text
                    .parse()
                    .map_err(|_| EntryError::Invalid("baro must be a number of inHg"))?;
                if !(27.5..=31.5).contains(&v) {
                    return Err(EntryError::Invalid("baro must be 27.50 to 31.50 inHg"));
                }
                Ok(Entry::Baro {
                    pressure_pa: v * PA_PER_INHG,
                })
            }
        }
        EntryTarget::Heading | EntryTarget::Course => {
            let v = whole(text, 360, "enter a bearing 0 to 360")?;
            let rad = f64::from(v % 360).to_radians();
            Ok(if target == EntryTarget::Heading {
                Entry::Heading { rad }
            } else {
                Entry::Course { rad }
            })
        }
        EntryTarget::Altitude => {
            let v = whole(text, 99_999, "enter an altitude in feet")?;
            if !(100..=30_000).contains(&v) {
                return Err(EntryError::Invalid("altitude must be 100 to 30,000 ft"));
            }
            Ok(Entry::Altitude {
                m: f64::from(v) * M_PER_FT,
            })
        }
        EntryTarget::FplAltitudeAt | EntryTarget::FplAltitudeAbove | EntryTarget::FplAltitudeBelow => {
            use flightcraft_sim::avionics::vnav::ConstraintKind as K;
            let v = whole(text, 99_999, "enter an altitude in feet")?;
            if v > 45_000 {
                return Err(EntryError::Invalid("altitude must be 0 to 45,000 ft"));
            }
            let kind = match target {
                EntryTarget::FplAltitudeAt => K::At,
                EntryTarget::FplAltitudeAbove => K::AtOrAbove,
                _ => K::AtOrBelow,
            };
            Ok(Entry::FplAltitude {
                kind,
                m: f64::from(v) * M_PER_FT,
            })
        }
        EntryTarget::Minimums => {
            let v = whole(text, 99_999, "enter a minimums altitude in feet")?;
            if v > 15_000 {
                return Err(EntryError::Invalid("minimums must be 0 to 15,000 ft"));
            }
            Ok(Entry::Minimums {
                m: f64::from(v) * M_PER_FT,
            })
        }
        EntryTarget::DirectTo | EntryTarget::FplBefore | EntryTarget::FplAfter => {
            if text.is_empty() {
                return Err(EntryError::Empty);
            }
            if text.len() > target.max_len() || !text.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(EntryError::Invalid("enter a waypoint identifier"));
            }
            let ident = text.to_ascii_uppercase();
            Ok(match target {
                EntryTarget::DirectTo => Entry::DirectTo { ident },
                _ => Entry::FplInsert {
                    ident,
                    after: target == EntryTarget::FplAfter,
                },
            })
        }
        EntryTarget::ComStandby => {
            let v = khz(text)?;
            if flightcraft_world::comms::valid_channel(v, false) {
                Ok(Entry::ComStandby { khz: v })
            } else {
                Err(EntryError::Invalid(
                    "COM frequency must be 118.000 to 136.975 on a 25 kHz channel",
                ))
            }
        }
        EntryTarget::NavStandby => {
            let v = khz(text)?;
            if (108_000..=117_950).contains(&v) && v % 50 == 0 {
                Ok(Entry::NavStandby { khz: v })
            } else {
                Err(EntryError::Invalid(
                    "NAV frequency must be 108.00 to 117.95 on a 50 kHz channel",
                ))
            }
        }
        EntryTarget::Transponder => {
            if text.is_empty() {
                return Err(EntryError::Empty);
            }
            if text.len() == 4 && text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
                Ok(Entry::Transponder {
                    code: text.to_owned(),
                })
            } else {
                Err(EntryError::Invalid("code must be four digits, 0 to 7"))
            }
        }
    }
}

/// The shortcut buttons beside a keypad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntryShortcut {
    /// Heading, altitude, or course: take the current value.
    SetToCurrent,
    /// Baro: standard pressure (29.92 inHg, 1013 hPa).
    SetToStandard,
    /// Baro, on the ground: the pressure that makes the altimeter read the field elevation.
    SetForField,
    /// Minimums: remove the minimums bug.
    TurnOff,
}

impl EntryShortcut {
    pub fn label(self) -> &'static str {
        match self {
            Self::SetToCurrent => "Set To Current",
            Self::SetToStandard => "Set To Standard",
            Self::SetForField => "Set For Field",
            Self::TurnOff => "Minimums Off",
        }
    }

    /// Shortcuts the target's keypad offers.
    pub fn offered(target: EntryTarget) -> &'static [Self] {
        match target {
            EntryTarget::Heading | EntryTarget::Altitude | EntryTarget::Course => &[Self::SetToCurrent],
            EntryTarget::Baro => &[Self::SetToStandard, Self::SetForField],
            EntryTarget::Minimums => &[Self::TurnOff],
            _ => &[],
        }
    }
}

pub const STANDARD_PRESSURE_PA: f64 = 101_325.;

/// The altimeter setting that makes an altimeter at the given static pressure read the given
/// elevation, by the standard atmosphere.
pub fn pressure_for_field(static_pressure_pa: f64, elevation_m: f64) -> f64 {
    // ISA pressure altitude formula, inverted for sea-level pressure.
    let ratio = 1. - 2.255_77e-5 * elevation_m;
    static_pressure_pa / libm::pow(ratio.max(0.01), 5.255_88)
}

/// The value Set To Current takes for an altitude: the nearest 100 ft, as the altitude knob steps.
pub fn current_altitude_m(altitude_m: f64) -> f64 {
    (libm::round(altitude_m / M_PER_FT / 100.) * 100.).max(100.) * M_PER_FT
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn type_in(target: EntryTarget, hpa: bool, keys: &str) -> String {
        keys.chars().fold(String::new(), |t, c| match c {
            '<' => press(target, hpa, &t, PadKey::Backspace),
            '!' => press(target, hpa, &t, PadKey::Clear),
            c => press(target, hpa, &t, PadKey::Char(c)),
        })
    }

    #[test]
    fn typing_follows_each_targets_rules() {
        assert_eq!(type_in(EntryTarget::Heading, false, "09x0"), "090");
        // A fourth digit is ignored.
        assert_eq!(type_in(EntryTarget::Heading, false, "2705"), "270");
        assert_eq!(type_in(EntryTarget::Heading, false, "1.2"), "12");
        assert_eq!(type_in(EntryTarget::Baro, false, "29.9.2"), "29.92");
        // hPa has no decimal point.
        assert_eq!(type_in(EntryTarget::Baro, true, "1013."), "1013");
        assert_eq!(type_in(EntryTarget::DirectTo, false, "kmmu"), "KMMU");
        assert_eq!(type_in(EntryTarget::DirectTo, false, "1n7"), "1N7");
        assert_eq!(type_in(EntryTarget::Transponder, false, "1200"), "1200");
        assert_eq!(type_in(EntryTarget::Transponder, false, "12a00"), "1200");
        assert_eq!(type_in(EntryTarget::Course, false, "123<<4"), "14");
        assert_eq!(type_in(EntryTarget::Course, false, "123!9"), "9");
    }

    #[test]
    fn keypads_show_letters_only_for_waypoints() {
        let digits = EntryTarget::Heading.rows(false);
        assert_eq!(digits.concat(), "1234567890".chars().collect::<Vec<_>>());
        let waypoint = EntryTarget::DirectTo.rows(false);
        assert!(waypoint.iter().flatten().any(|c| *c == 'Z'));
        assert!(waypoint.iter().flatten().any(|c| *c == '7'));
        assert!(EntryTarget::Baro.rows(false).iter().flatten().any(|c| *c == '.'));
        assert!(!EntryTarget::Baro.rows(true).iter().flatten().any(|c| *c == '.'));
    }

    #[test]
    fn baro_entry_accepts_both_units_and_a_run_together_inhg() {
        let inhg = |t| parse(EntryTarget::Baro, false, t);
        let Ok(Entry::Baro { pressure_pa }) = inhg("29.92") else {
            panic!("29.92");
        };
        assert!((pressure_pa - 101_321.).abs() < 5.);
        assert_eq!(inhg("2992"), inhg("29.92"));
        assert!(inhg("30.5").is_ok());
        assert!(inhg("27.0").is_err());
        assert!(inhg("32").is_err());
        assert_eq!(inhg(""), Err(EntryError::Empty));
        let hpa = parse(EntryTarget::Baro, true, "1013").unwrap();
        assert_eq!(
            hpa,
            Entry::Baro {
                pressure_pa: 101_300.
            }
        );
        assert!(parse(EntryTarget::Baro, true, "700").is_err());
        assert!(parse(EntryTarget::Baro, true, "10.5").is_err());
    }

    #[test]
    fn heading_and_course_wrap_360_to_north_and_reject_larger_values() {
        for target in [EntryTarget::Heading, EntryTarget::Course] {
            let north = parse(target, false, "360").unwrap();
            let zero = parse(target, false, "0").unwrap();
            assert_eq!(north, zero);
            assert!(parse(target, false, "361").is_err());
            assert!(parse(target, false, "-5").is_err());
            assert_eq!(parse(target, false, ""), Err(EntryError::Empty));
        }
        let Entry::Heading { rad } = parse(EntryTarget::Heading, false, "90").unwrap() else {
            panic!()
        };
        assert!((rad - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }

    #[test]
    fn altitude_entry_is_feet_and_bounded() {
        let Entry::Altitude { m } = parse(EntryTarget::Altitude, false, "3500").unwrap() else {
            panic!()
        };
        assert!((m - 1066.8).abs() < 1e-9);
        assert!(parse(EntryTarget::Altitude, false, "30001").is_err());
        assert!(parse(EntryTarget::Altitude, false, "99").is_err());
        assert!(parse(EntryTarget::Altitude, false, "100").is_ok());
        assert!(parse(EntryTarget::Altitude, false, "12.5").is_err());
    }

    #[test]
    fn minimums_entry_is_feet_zero_to_fifteen_thousand_with_an_off_shortcut() {
        let Entry::Minimums { m } = parse(EntryTarget::Minimums, false, "250").unwrap() else {
            panic!()
        };
        assert!((m - 76.2).abs() < 1e-9);
        assert!(parse(EntryTarget::Minimums, false, "0").is_ok());
        assert!(parse(EntryTarget::Minimums, false, "15001").is_err());
        assert!(parse(EntryTarget::Minimums, false, "2.5").is_err());
        assert_eq!(parse(EntryTarget::Minimums, false, ""), Err(EntryError::Empty));
        assert_eq!(type_in(EntryTarget::Minimums, false, "12345678"), "12345");
        assert_eq!(
            EntryShortcut::offered(EntryTarget::Minimums),
            [EntryShortcut::TurnOff]
        );
    }

    #[test]
    fn direct_to_identifier_is_normalised_and_checked() {
        assert_eq!(
            parse(EntryTarget::DirectTo, false, "kmmu").unwrap(),
            Entry::DirectTo { ident: "KMMU".into() }
        );
        assert!(parse(EntryTarget::DirectTo, false, "K M").is_err());
        assert!(parse(EntryTarget::DirectTo, false, "TOOLONG7").is_err());
        assert_eq!(parse(EntryTarget::DirectTo, false, " "), Err(EntryError::Empty));
    }

    #[test]
    fn radio_entry_checks_the_band_and_channel_grid() {
        let com = |t| parse(EntryTarget::ComStandby, false, t);
        assert_eq!(com("121.5").unwrap(), Entry::ComStandby { khz: 121_500 });
        assert_eq!(com("118.275").unwrap(), Entry::ComStandby { khz: 118_275 });
        assert_eq!(com("122750").unwrap(), Entry::ComStandby { khz: 122_750 });
        assert!(com("117.9").is_err());
        assert!(com("137.0").is_err());
        assert!(com("121.512").is_err());
        let nav = |t| parse(EntryTarget::NavStandby, false, t);
        assert_eq!(nav("113.8").unwrap(), Entry::NavStandby { khz: 113_800 });
        assert!(nav("113.85").is_ok());
        assert!(nav("113.83").is_err());
        assert!(nav("118.0").is_err());
    }

    #[test]
    fn transponder_code_is_four_octal_digits() {
        assert_eq!(
            parse(EntryTarget::Transponder, false, "1200").unwrap(),
            Entry::Transponder { code: "1200".into() }
        );
        for bad in ["120", "12000", "1208", "abcd"] {
            assert!(parse(EntryTarget::Transponder, false, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn shortcuts_are_offered_where_the_guide_has_them() {
        assert_eq!(
            EntryShortcut::offered(EntryTarget::Heading),
            [EntryShortcut::SetToCurrent]
        );
        assert_eq!(
            EntryShortcut::offered(EntryTarget::Altitude),
            [EntryShortcut::SetToCurrent]
        );
        assert_eq!(
            EntryShortcut::offered(EntryTarget::Course),
            [EntryShortcut::SetToCurrent]
        );
        assert_eq!(
            EntryShortcut::offered(EntryTarget::Baro),
            [EntryShortcut::SetToStandard, EntryShortcut::SetForField]
        );
        assert!(EntryShortcut::offered(EntryTarget::DirectTo).is_empty());
        assert!(EntryTarget::Heading.has_set_to_current());
        assert!(!EntryTarget::Baro.has_set_to_current());
    }

    #[test]
    fn set_for_field_makes_the_altimeter_read_the_field_elevation() {
        // At sea level the field pressure is the setting itself.
        let qnh = pressure_for_field(101_325., 0.);
        assert!((qnh - 101_325.).abs() < 1e-6);
        // 1,000 m up the standard atmosphere has about 89,875 Pa; the setting recovers 1013.25 hPa.
        let at_altitude = 89_874.6;
        assert!((pressure_for_field(at_altitude, 1000.) - 101_325.).abs() < 30.);
        // A low-pressure day gives a lower setting for the same field.
        assert!(pressure_for_field(88_000., 1000.) < 101_325.);
    }

    #[test]
    fn set_to_current_altitude_rounds_to_the_nearest_hundred_feet() {
        let ft = |f: f64| f * M_PER_FT;
        assert!((current_altitude_m(ft(3_449.)) - ft(3_400.)).abs() < 1e-9);
        assert!((current_altitude_m(ft(3_451.)) - ft(3_500.)).abs() < 1e-9);
    }
}
