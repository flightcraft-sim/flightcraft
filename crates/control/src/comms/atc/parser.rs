//! Bounded phrase grammar: normalize punctuation, then consume named numeric slots.
use super::*;
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PilotMessage {
    pub normalized: String,
    pub callsign_present: bool,
    pub facility: Option<FacilityKind>,
    pub addressed_facility: Option<String>,
    pub information: Option<String>,
    pub runway: Option<String>,
    pub heading_deg: Option<u16>,
    pub altitude_ft: Option<u32>,
    pub expected_altitude_ft: Option<u32>,
    pub expect_minutes: Option<u8>,
    pub channel_khz: Option<u32>,
    pub squawk: Option<String>,
    pub request: Option<PilotRequest>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PilotRequest {
    Taxi,
    VfrDeparture,
    IfrClearance,
    Ready,
    Inbound,
    FlightFollowing,
    Bravo,
    Approach,
    GoAround,
    ClosedTraffic,
    ClearRunway,
    Handoff,
    Position,
}
const ALPHABET: [&str; 26] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet", "kilo",
    "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra", "tango", "uniform", "victor",
    "whiskey", "xray", "yankee", "zulu",
];
pub(super) fn letter(n: u64) -> String {
    ALPHABET.get((n % 26) as usize).copied().unwrap_or("alpha").into()
}
fn digit(w: &str) -> Option<u32> {
    match w {
        "zero" | "oh" => Some(0),
        "one" => Some(1),
        "two" => Some(2),
        "three" | "tree" => Some(3),
        "four" | "fower" => Some(4),
        "five" | "fife" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" | "niner" => Some(9),
        _ => None,
    }
}
/// Parse digit groups and spoken hundreds/thousands, retaining leading zeros for radio codes.
fn number(tokens: &[&str]) -> Option<(String, usize)> {
    let mut value = 0u32;
    let mut group = 0u32;
    let mut digits = String::new();
    let mut scaled = false;
    let mut consumed = 0;
    for t in tokens.iter().take(12) {
        if let Some(d) = digit(t) {
            group = group.checked_mul(10)?.checked_add(d)?;
            digits.push(char::from_digit(d, 10)?);
        } else if t.bytes().all(|b| b.is_ascii_digit()) && !t.is_empty() {
            group = group
                .checked_mul(10u32.checked_pow(t.len() as u32)?)?
                .checked_add(t.parse::<u32>().ok()?)?;
            digits.push_str(t);
        } else if matches!(
            *t,
            "ten"
                | "eleven"
                | "twelve"
                | "thirteen"
                | "fourteen"
                | "fifteen"
                | "sixteen"
                | "seventeen"
                | "eighteen"
                | "nineteen"
                | "twenty"
                | "thirty"
                | "forty"
                | "fifty"
        ) {
            let n = match *t {
                "ten" => 10,
                "eleven" => 11,
                "twelve" => 12,
                "thirteen" => 13,
                "fourteen" => 14,
                "fifteen" => 15,
                "sixteen" => 16,
                "seventeen" => 17,
                "eighteen" => 18,
                "nineteen" => 19,
                "twenty" => 20,
                "thirty" => 30,
                "forty" => 40,
                _ => 50,
            };
            group = group.checked_add(n)?;
            digits.push_str(&n.to_string());
        } else if *t == "hundred" {
            group = group.checked_mul(100)?;
            scaled = true;
        } else if *t == "thousand" {
            value = value.checked_add(group.checked_mul(1000)?)?;
            group = 0;
            scaled = true;
        } else {
            break;
        }
        consumed += 1;
    }
    if consumed == 0 {
        None
    } else {
        Some((
            if scaled {
                value.checked_add(group)?.to_string()
            } else {
                digits
            },
            consumed,
        ))
    }
}
fn after(tokens: &[&str], key: &str) -> Option<(String, usize, usize)> {
    let i = tokens.iter().position(|t| *t == key)? + 1;
    let (v, n) = number(tokens.get(i..)?)?;
    Some((v, n, i))
}
pub fn parse_pilot(text: &str, callsign: &PilotCallsign) -> PilotMessage {
    let mut radio_text = String::new();
    let mut chars = text.chars().peekable();
    let mut previous_digit = false;
    while let Some(c) = chars.next() {
        if c == '.' && previous_digit && chars.peek().is_some_and(char::is_ascii_digit) {
            radio_text.push_str(" point ");
        } else {
            radio_text.push(c);
        }
        previous_digit = c.is_ascii_digit();
    }
    let normalized = words(&radio_text)
        .split_whitespace()
        .map(|w| match w {
            "requesting" => "request",
            "squawking" => "squawk",
            _ => w,
        })
        .collect::<Vec<_>>()
        .join(" ");
    let t: Vec<_> = normalized.split_whitespace().collect();
    let contains = |phrase: &str| format!(" {normalized} ").contains(&format!(" {phrase} "));
    let callsign_present = [
        words(&callsign.full),
        words(&callsign.current),
        words(&callsign.abbreviated),
        words(&callsign.registration),
        words(&format!(
            "{} {}",
            callsign.spoken_type,
            callsign.registration.trim_start_matches('N')
        )),
        words(&format!(
            "November {}",
            spoken_registration(callsign.registration.trim_start_matches('N'))
        )),
    ]
    .iter()
    .any(|c| !c.is_empty() && format!(" {normalized} ").contains(&format!(" {c} ")));
    let runway = after(&t, "runway")
        .and_then(|(v, n, i)| {
            let num = v.parse::<u8>().ok()?;
            if !(1..=36).contains(&num) {
                return None;
            }
            let suffix = match t.get(i + n).copied() {
                Some("left") => "L",
                Some("right") => "R",
                Some("center") => "C",
                _ => "",
            };
            Some(format!("{num:02}{suffix}"))
        })
        .or_else(|| {
            t.windows(2).find_map(|p| {
                let ["runway", v] = p else {
                    return None;
                };
                let v = v.to_uppercase();
                let n = v.trim_end_matches(['L', 'R', 'C']).parse::<u8>().ok()?;
                (1..=36).contains(&n).then(|| {
                    format!(
                        "{n:02}{}",
                        v.chars()
                            .last()
                            .filter(char::is_ascii_alphabetic)
                            .map_or(String::new(), |c| c.to_string())
                    )
                })
            })
        });
    let channel_khz = t
        .iter()
        .position(|v| *v == "point" || *v == "decimal")
        .and_then(|i| {
            let mut whole = None;
            for start in i.saturating_sub(3)..i {
                if let Some((v, n)) = number(t.get(start..i)?)
                    && start + n == i
                    && v.len() == 3
                {
                    whole = v.parse::<u32>().ok();
                    break;
                }
            }
            let (v, _) = number(t.get(i + 1..)?)?;
            if v.len() > 3 {
                return None;
            }
            let fraction = v.parse::<u32>().ok()? * 10u32.pow(3 - v.len() as u32);
            let khz = whole?.checked_mul(1000)?.checked_add(fraction)?;
            flightcraft_world::comms::valid_channel(khz, true).then_some(khz)
        })
        .or_else(|| {
            t.iter().find_map(|v| {
                v.parse::<u32>()
                    .ok()
                    .filter(|f| flightcraft_world::comms::valid_channel(*f, true))
            })
        });
    let information = t.windows(2).find_map(|pair| {
        let ["information", l] = pair else {
            return None;
        };
        ALPHABET
            .iter()
            .position(|a| a == l)
            .or_else(|| {
                (l.len() == 1)
                    .then(|| l.bytes().next().map(|b| b.saturating_sub(b'a') as usize))
                    .flatten()
                    .filter(|n| *n < 26)
            })
            .map(|n| letter(n as u64))
    });
    let heading_deg = after(&t, "heading")
        .and_then(|(v, _, _)| v.parse::<u16>().ok())
        .filter(|v| *v <= 360);
    let altitude_ft = ["maintain", "altitude", "leaving", "climbing", "descending"]
        .iter()
        .find_map(|key| after(&t, key).and_then(|(v, _, _)| v.parse::<u32>().ok()))
        .filter(|v| *v <= 60000);
    let expectation = t.iter().position(|v| *v == "expect").and_then(|i| {
        let rest = t.get(i + 1..)?;
        if let Some(in_pos) = rest.iter().position(|v| *v == "in") {
            return Some((
                number(rest.get(..in_pos)?)?.0.parse::<u32>().ok()?,
                number(rest.get(in_pos + 1..)?)?.0.parse::<u8>().ok()?,
            ));
        }
        let scale = rest.iter().position(|v| *v == "thousand")?;
        let end = rest
            .iter()
            .take(scale + 4)
            .position(|v| *v == "hundred")
            .map_or(scale + 1, |v| v + 1);
        Some((
            number(rest.get(..end)?)?.0.parse::<u32>().ok()?,
            number(rest.get(end..)?)?.0.parse::<u8>().ok()?,
        ))
    });
    let expected_altitude_ft = expectation.map(|v| v.0);
    let expect_minutes = expectation.map(|v| v.1);
    let squawk = after(&t, "squawk")
        .map(|(v, _, _)| v)
        .filter(|v| v.len() == 4 && v.bytes().all(|b| (b'0'..=b'7').contains(&b)));
    let facility = [
        ("clearance", FacilityKind::Clearance),
        ("ground", FacilityKind::Ground),
        ("tower", FacilityKind::Tower),
        ("approach", FacilityKind::Approach),
        ("departure", FacilityKind::Departure),
    ]
    .into_iter()
    .find_map(|(w, f)| contains(w).then_some(f));
    let addressed_facility = t
        .iter()
        .position(|w| matches!(*w, "clearance" | "ground" | "tower" | "approach" | "departure"))
        .filter(|i| *i > 0 && *i <= 3)
        .and_then(|i| {
            let prefix = t.get(..i)?.join(" ");
            let pilot_type = words(&callsign.spoken_type);
            (!prefix.contains(&pilot_type)
                && !prefix.contains("november")
                && !prefix.contains(&callsign.registration.to_lowercase()))
            .then_some(prefix)
        });
    let request = if contains("go around") || contains("going around") {
        Some(PilotRequest::GoAround)
    } else if contains("clear of runway") || contains("clear of the runway") {
        Some(PilotRequest::ClearRunway)
    } else if contains("request closed traffic") {
        Some(PilotRequest::ClosedTraffic)
    } else if contains("ready") {
        Some(PilotRequest::Ready)
    } else if contains("request ifr") || contains("ifr clearance") {
        Some(PilotRequest::IfrClearance)
    } else if contains("request taxi") || contains("taxi request") {
        Some(PilotRequest::Taxi)
    } else if contains("request vfr") {
        Some(PilotRequest::VfrDeparture)
    } else if contains("request bravo") || contains("request class bravo") || contains("bravo transition") {
        Some(PilotRequest::Bravo)
    } else if contains("request flight following") {
        Some(PilotRequest::FlightFollowing)
    } else if contains("request ils")
        || contains("request rnav")
        || contains("request vor")
        || contains("request loc")
        || contains("request localizer")
        || contains("request ndb")
        || contains("request approach")
        || contains("request visual")
    {
        Some(PilotRequest::Approach)
    } else if contains("inbound") || contains("request landing") {
        Some(PilotRequest::Inbound)
    } else if contains("request handoff") || contains("request frequency change") {
        Some(PilotRequest::Handoff)
    } else if contains("downwind")
        || contains("base")
        || contains("final")
        || contains("departure")
        || contains("level")
        || contains("climbing")
        || contains("descending")
    {
        Some(PilotRequest::Position)
    } else {
        None
    };
    PilotMessage {
        normalized,
        callsign_present,
        facility,
        addressed_facility,
        information,
        runway,
        heading_deg,
        altitude_ft,
        expected_altitude_ft,
        expect_minutes,
        channel_khz,
        squawk,
        request,
    }
}
