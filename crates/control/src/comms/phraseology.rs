//! Small, stateless FAA JO 7110.65 / AIM phrase primitives shared by radio, ATC, and scenarios.
//! Inputs retain aviation units: degrees, feet MSL, channel kHz, and hundredths of inHg.
use super::{Reception, Speaker};
use flightcraft_world::{ComStation, FacilityKind};

/// Individual digits, including aviation “niner”, with a minimum field width.
pub fn digits(value: u32, width: usize) -> String {
    let width = width.min(10);
    format!("{value:0width$}")
        .chars()
        .filter_map(|c| match c {
            '0' => Some("zero"),
            '1' => Some("one"),
            '2' => Some("two"),
            '3' => Some("three"),
            '4' => Some("four"),
            '5' => Some("five"),
            '6' => Some("six"),
            '7' => Some("seven"),
            '8' => Some("eight"),
            '9' => Some("niner"),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}
/// Runway designator including L/R/C, preserving the leading zero.
pub fn runway(value: &str) -> String {
    value
        .trim_start_matches("RW")
        .chars()
        .filter_map(|c| match c {
            'L' | 'l' => Some("left".into()),
            'R' | 'r' => Some("right".into()),
            'C' | 'c' => Some("center".into()),
            c if c.is_ascii_digit() => c.to_digit(10).map(|n| digits(n, 1)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn heading(degrees: u16) -> String {
    digits(
        u32::from(if degrees.is_multiple_of(360) {
            360
        } else {
            degrees % 360
        }),
        3,
    )
}
/// Below flight levels: thousands as individual digits, followed by hundreds.
pub fn altitude(feet: u32) -> String {
    let mut result = Vec::new();
    if feet >= 1000 {
        result.push(format!("{} thousand", digits(feet / 1000, 1)));
    }
    if feet % 1000 >= 100 {
        result.push(format!("{} hundred", digits(feet % 1000 / 100, 1)));
    }
    if !feet.is_multiple_of(100) || result.is_empty() {
        result.push(digits(feet % 100, 1));
    }
    result.join(" ")
}
/// COM channel: omit insignificant trailing fractional zeros, retaining at least one digit.
pub fn frequency(khz: u32) -> String {
    let fraction = format!("{:03}", khz % 1000);
    let fraction = fraction.trim_end_matches('0');
    format!(
        "{} point {}",
        digits(khz / 1000, 3),
        if fraction.is_empty() {
            digits(0, 1)
        } else {
            fraction
                .chars()
                .filter_map(|c| c.to_digit(10))
                .map(|n| digits(n, 1))
                .collect::<Vec<_>>()
                .join(" ")
        }
    )
}
/// Standard ground frequencies may omit the common 121 MHz prefix (AIM 4-2-9).
pub fn ground_frequency(khz: u32) -> String {
    let full = frequency(khz);
    if (121000..122000).contains(&khz) {
        full.strip_prefix("one two one ").unwrap_or(&full).into()
    } else {
        full
    }
}
/// Initial radar identification includes the aircraft's position relative to a named fix.
pub fn radar_contact(miles: u32, direction: &str, fix: &str) -> String {
    format!("Radar contact, {} miles {direction} of {fix}.", digits(miles, 1))
}
pub fn altimeter(hundredths_inhg: u32) -> String {
    digits(hundredths_inhg, 4)
}
pub fn wind(from_deg: u16, knots: u16) -> String {
    if knots == 0 {
        "wind calm".into()
    } else {
        format!("wind {} at {}", heading(from_deg), digits(u32::from(knots), 1))
    }
}
/// A clearance to change altitude, or to hold an altitude already reached (100 ft tolerance).
pub fn altitude_assignment(assigned_ft: u32, actual_ft: f64) -> String {
    let verb = if actual_ft + 100. < f64::from(assigned_ft) {
        "climb and maintain"
    } else if actual_ft - 100. > f64::from(assigned_ft) {
        "descend and maintain"
    } else {
        "maintain"
    };
    format!("{verb} {}", altitude(assigned_ft))
}
/// Normal case for imported uppercase NASR names; preserve aviation acronyms.
pub fn normal_case(text: &str) -> String {
    text.split_whitespace()
        .map(|w| match w.to_ascii_uppercase().as_str() {
            "ATIS" => "ATIS".into(),
            "AWOS" | "AWOS-3" => "AWOS".into(),
            "ASOS" => "ASOS".into(),
            "INTL" => "International".into(),
            "MUNI" => "Municipal".into(),
            _ => w
                .split('-')
                .map(|part| {
                    let mut chars = part.chars();
                    chars.next().map_or(String::new(), |c| {
                        format!("{}{}", c.to_uppercase(), chars.as_str().to_lowercase())
                    })
                })
                .collect::<Vec<_>>()
                .join("-"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn facility_kind(kind: FacilityKind) -> &'static str {
    match kind {
        FacilityKind::Tower => "Tower",
        FacilityKind::Ground => "Ground",
        FacilityKind::Clearance => "Clearance Delivery",
        FacilityKind::Departure => "Departure",
        FacilityKind::Approach => "Approach",
        FacilityKind::Atis => "ATIS",
        FacilityKind::Asos => "ASOS",
        FacilityKind::Awos => "AWOS",
        FacilityKind::Ctaf => "Traffic",
        FacilityKind::Unicom => "Unicom",
    }
}
/// Prefer the NASR radio call, not the airport's administrative name or provider identifier.
pub fn facility(station: &ComStation) -> String {
    let call = station
        .metadata
        .as_ref()
        .map(|m| m.callsign.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&station.name);
    let call = normal_case(call);
    let role = facility_kind(station.kind);
    if call.to_lowercase().ends_with(&role.to_lowercase()) {
        call
    } else {
        format!("{call} {role}")
    }
}
pub fn speaker(speaker: &Speaker, stations: &[ComStation]) -> String {
    match speaker {
        Speaker::Pilot => "Pilot".into(),
        Speaker::Instructor => "Instructor".into(),
        Speaker::Crew => "Pilot in command · intercom".into(),
        Speaker::Traffic { callsign } => callsign.clone(),
        Speaker::Nav { ident } => format!("{ident} identification"),
        Speaker::Facility { id, role } => stations
            .iter()
            .find(|s| s.id == *id)
            .map_or_else(|| facility_kind(*role).into(), facility),
    }
}
pub fn reception(value: Reception) -> &'static str {
    match value {
        Reception::Heard => "Heard",
        Reception::Pending => "In progress",
        Reception::Blocked => "Blocked",
        Reception::NotReceived => "Not received",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn faa_number_slots() {
        assert_eq!(runway("05"), "zero five");
        assert_eq!(runway("04L"), "zero four left");
        assert_eq!(heading(48), "zero four eight");
        assert_eq!(altitude(2200), "two thousand two hundred");
        assert_eq!(altitude(10000), "one zero thousand");
        assert_eq!(frequency(119200), "one one niner point two");
        assert_eq!(frequency(126975), "one two six point niner seven five");
        assert_eq!(altimeter(2992), "two niner niner two");
        assert_eq!(wind(60, 8), "wind zero six zero at eight");
        assert_eq!(wind(0, 0), "wind calm");
        assert_eq!(
            altitude_assignment(2200, 400.),
            "climb and maintain two thousand two hundred"
        );
        assert_eq!(
            altitude_assignment(2200, 3000.),
            "descend and maintain two thousand two hundred"
        );
    }
}

/// Render a validated scripted or `comms.atc` instruction, addressed to the pilot.
/// Altitudes are metres MSL; wind is the pilot magnetic weather report. This only formats text.
pub fn scripted_transmission(
    station: &ComStation,
    pilot: &super::PilotCallsign,
    instruction: &crate::scenario::Instruction,
    wind: flightcraft_sim::Weather,
) -> String {
    use crate::scenario::Instruction;
    let body = match instruction {
        Instruction::Land { runway, .. } => format!(
            "runway {}, cleared to land, {}",
            self::runway(runway),
            self::wind(
                wind.wind_from_deg.round() as u16,
                (wind.wind_speed_mps / 0.514444).round() as u16
            )
        ),
        Instruction::GoAround { runway, .. } => format!("go around, runway {}", self::runway(runway)),
        Instruction::Altitude { altitude_msl_m, .. } => {
            format!(
                "maintain {}",
                altitude((*altitude_msl_m / 0.3048 / 100.).round().max(0.) as u32 * 100)
            )
        }
        Instruction::AirspaceClearance { id } => format!("cleared into {id}"),
        Instruction::RequestReadback { text } | Instruction::Advisory { text } => {
            text.trim_end_matches(['.', ' ']).to_owned()
        }
    };
    format!("{}, {}, {}.", pilot.current, facility(station), body)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod scenario_tests {
    use super::*;
    use crate::scenario::Instruction;
    use crate::{CommsCallsign, Session};
    use flightcraft_world::FacilityKind;
    #[test]
    fn nasr_clearance_is_addressed_and_numbers_are_spoken() {
        let nav = flightcraft_world::Navdata::from_json(include_str!(
            "../../../../assets/terrain/eastern-pa/navdata.json"
        ))
        .unwrap();
        let station = nav
            .comms
            .iter()
            .find(|s| s.airport == "KABE" && s.kind == FacilityKind::Tower)
            .unwrap();
        let mut s = Session::new().unwrap();
        let pilot = s
            .run(CommsCallsign {
                registration: Some("N34B".into()),
                spoken_type: Some("Sling".into()),
                ..Default::default()
            })
            .unwrap();
        let instruction = Instruction::Land {
            airport: "KABE".into(),
            runway: "24".into(),
        };
        let mut wind = flightcraft_sim::Weather::default();
        assert_eq!(
            scripted_transmission(station, &pilot, &instruction, wind),
            "Sling Three Four Bravo, Allentown Tower, runway two four, cleared to land, wind calm."
        );
        wind.wind_from_deg = 230.;
        wind.wind_speed_mps = 8. * 0.514444;
        assert!(
            scripted_transmission(station, &pilot, &instruction, wind)
                .ends_with("wind two three zero at eight.")
        );
    }
}

/// The virtual pilot and human helpers share facility names and spoken runway slots.
/// A CTAF report repeats the airport name; a tower call addresses the controller once.
pub fn pilot_position(
    station: &ComStation,
    pilot: &super::PilotCallsign,
    runway_id: &str,
    report: &str,
) -> String {
    let name = facility(station);
    let repeat = if matches!(station.kind, FacilityKind::Ctaf | FacilityKind::Unicom) {
        format!(", {}", name.strip_suffix(" Traffic").unwrap_or(&name))
    } else {
        String::new()
    };
    format!(
        "{name}, {}, {report} runway {}{repeat}.",
        if station.kind == FacilityKind::Ctaf {
            &pilot.full
        } else {
            &pilot.current
        },
        runway(runway_id)
    )
}

/// Initial contact always uses the complete aircraft identity and the received information letter.
pub fn pilot_request(
    station: &ComStation,
    pilot: &super::PilotCallsign,
    request: &str,
    information: Option<&str>,
) -> String {
    format!(
        "{}, {}, {}{}.",
        facility(station),
        pilot.full,
        request.trim_end_matches('.'),
        information.map_or(String::new(), |letter| format!(", with information {letter}"))
    )
}
/// AIM 4-1-9 inbound position, using actual range and cardinal direction from the field.
pub fn pilot_inbound(
    station: &ComStation,
    pilot: &super::PilotCallsign,
    runway_id: &str,
    miles: u32,
    direction: &str,
    information: Option<&str>,
) -> String {
    let body = format!(
        "{} {} {direction}, inbound for landing, runway {}",
        digits(miles, if miles >= 10 { 2 } else { 1 }),
        if miles == 1 { "mile" } else { "miles" },
        runway(runway_id)
    );
    if station.kind == FacilityKind::Ctaf {
        let name = facility(station);
        let name = name.trim_end_matches(" Traffic");
        format!("{name} Traffic, {}, {body}, {name}.", pilot.full)
    } else {
        pilot_request(station, pilot, &body, information)
    }
}

/// Published instrument clearance wording shared by instructor commands and controller rules.
pub fn approach_clearance(family: &str, runway_id: &str) -> String {
    format!("Cleared {family} runway {} approach.", runway(runway_id))
}
pub fn published_missed() -> String {
    "Execute the published missed approach.".into()
}
pub fn hold_clearance(
    fix: &str,
    inbound_deg: u16,
    right_turns: bool,
    leg_time_s: Option<u16>,
    leg_distance_nm: Option<f64>,
    altitude_ft: u32,
    efc_utc: u16,
) -> String {
    let sector = ((f64::from(inbound_deg) + 180.).rem_euclid(360.) / 45. + 0.5).floor() as usize % 8;
    let direction = [
        "north",
        "northeast",
        "east",
        "southeast",
        "south",
        "southwest",
        "west",
        "northwest",
    ]
    .get(sector)
    .copied()
    .unwrap_or("north");
    let legs = hold_legs(leg_time_s, leg_distance_nm);
    format!(
        "Hold {direction} of {fix} on the {} inbound course, {} turns, {legs}. Maintain {}. Expect further clearance at {}.",
        heading(inbound_deg),
        if right_turns { "right" } else { "left" },
        altitude(altitude_ft),
        digits(u32::from(efc_utc), 4)
    )
}

pub fn hold_legs(leg_time_s: Option<u16>, leg_distance_nm: Option<f64>) -> String {
    leg_distance_nm.map_or_else(
        || {
            format!(
                "{} minute legs",
                digits(u32::from(leg_time_s.unwrap_or(60)) / 60, 1)
            )
        },
        |nm| format!("{} mile legs", digits(nm.round() as u32, 1)),
    )
}

/// AIM 5-4-3: aircraft identity, present altitude, and the received ATIS on approach contact.
pub fn pilot_ifr_checkin(
    station: &ComStation,
    pilot: &super::PilotCallsign,
    altitude_ft: u32,
    information: Option<&str>,
) -> String {
    format!(
        "{}, {}, {}{}.",
        facility(station),
        pilot.full,
        altitude(altitude_ft),
        information.map_or(String::new(), |letter| format!(", information {letter}"))
    )
}
/// JO 7110.65 4-8-1: final heading, altitude restriction, then the approach clearance.
pub fn approach_intercept(
    family: &str,
    runway_id: &str,
    heading_deg: u16,
    altitude_ft: u32,
    turn_left: bool,
) -> String {
    let course = if family == "ILS" || family == "LOC" {
        "the localizer"
    } else {
        "the final approach course"
    };
    format!(
        "Turn {} heading {}, maintain {} until established on {course}, {}",
        if turn_left { "left" } else { "right" },
        heading(heading_deg),
        altitude(altitude_ft),
        approach_clearance(family, runway_id).replacen("Cleared", "cleared", 1)
    )
}

pub fn spoken_registration(registration: &str) -> String {
    registration
        .chars()
        .filter_map(|c| {
            Some(match c {
                '0' => "Zero",
                '1' => "One",
                '2' => "Two",
                '3' => "Three",
                '4' => "Four",
                '5' => "Five",
                '6' => "Six",
                '7' => "Seven",
                '8' => "Eight",
                '9' => "Niner",
                'A' => "Alfa",
                'B' => "Bravo",
                'C' => "Charlie",
                'D' => "Delta",
                'E' => "Echo",
                'F' => "Foxtrot",
                'G' => "Golf",
                'H' => "Hotel",
                'I' => "India",
                'J' => "Juliett",
                'K' => "Kilo",
                'L' => "Lima",
                'M' => "Mike",
                'N' => "November",
                'O' => "Oscar",
                'P' => "Papa",
                'Q' => "Quebec",
                'R' => "Romeo",
                'S' => "Sierra",
                'T' => "Tango",
                'U' => "Uniform",
                'V' => "Victor",
                'W' => "Whiskey",
                'X' => "X-ray",
                'Y' => "Yankee",
                'Z' => "Zulu",
                _ => return None,
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn callsign_forms(registration: &str, kind: &str) -> (String, String) {
    // US pilots may replace November with the aircraft type. Other nationality prefixes remain.
    let suffix = registration
        .strip_prefix('N')
        .filter(|r| !r.contains('-') && r.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(registration);
    let short = suffix
        .chars()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    let identity = if kind.is_empty() {
        spoken_registration(registration)
    } else {
        format!("{kind} {}", spoken_registration(suffix))
    };
    let abbreviated = if kind.is_empty() {
        spoken_registration(&short)
    } else {
        format!("{kind} {}", spoken_registration(&short))
    };
    (identity, abbreviated)
}
