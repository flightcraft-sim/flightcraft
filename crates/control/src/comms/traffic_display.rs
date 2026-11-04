//! What the glass avionics, the moving map, the traffic page, and tablet EFB apps show about other
//! aircraft: one merged target list with ADS-B style classification, the display filter, and the
//! highlight that ties a controller's call to the aircraft it names. See `docs/traffic.md`.
use super::*;
use serde_json::Value;

const NM_M: f64 = 1852.0;
const FT_M: f64 = 0.3048;
/// Fleet addresses are `FLEET_ADDRESS_BASE + aircraft id`; lower addresses are circuit (CTAF) traffic.
pub const FLEET_ADDRESS_BASE: u32 = 0xf2_0000;
/// How long a named aircraft stays highlighted after the call that mentioned it ends.
pub const HIGHLIGHT_S: f64 = 20.0;

/// Proximate traffic: within 6 NM and 1,200 ft of the aircraft (Garmin and Dynon convention).
pub const PROXIMATE_RANGE_NM: f64 = 6.0;
pub const PROXIMATE_ALTITUDE_FT: f64 = 1200.0;
/// A traffic alert: within 2 NM and 500 ft now, or on course to pass inside 1 NM and 500 ft within 45 s.
pub const ALERT_RANGE_NM: f64 = 2.0;
pub const ALERT_ALTITUDE_FT: f64 = 500.0;
pub const ALERT_CPA_NM: f64 = 1.0;
pub const ALERT_TAU_S: f64 = 45.0;
/// A climb or descent arrow is drawn when the vertical rate exceeds this.
pub const TREND_ARROW_FPM: f64 = 500.0;

/// ADS-B traffic symbol class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrafficClass {
    /// Hollow arrow.
    #[default]
    Other,
    /// Filled arrow.
    Proximate,
    /// Amber filled symbol and the TRAFFIC annunciation.
    Alert,
}
pub use flightcraft_sim::avionics::glass::TrafficFilter;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrafficOrigin {
    /// Busy-airspace traffic controlled by the tower and approach rules.
    Fleet,
    /// A circuit aircraft started by `traffic.set`.
    Circuit,
}

/// Classify one target from geometry alone. `closing` is the projected pass: time to the closest
/// point of approach (seconds), the horizontal distance there (NM), and the altitude difference then (ft).
pub fn classify(
    range_nm: f64,
    relative_altitude_ft: f64,
    on_ground: bool,
    closest_approach: Option<(f64, f64, f64)>,
) -> TrafficClass {
    if on_ground {
        return TrafficClass::Other;
    }
    let vertical = relative_altitude_ft.abs();
    if range_nm <= ALERT_RANGE_NM && vertical <= ALERT_ALTITUDE_FT {
        return TrafficClass::Alert;
    }
    if let Some((tau_s, cpa_nm, cpa_altitude_ft)) = closest_approach
        && range_nm <= PROXIMATE_RANGE_NM
        && (0.0..=ALERT_TAU_S).contains(&tau_s)
        && cpa_nm <= ALERT_CPA_NM
        && cpa_altitude_ft.abs() <= ALERT_ALTITUDE_FT
    {
        return TrafficClass::Alert;
    }
    if range_nm <= PROXIMATE_RANGE_NM && vertical <= PROXIMATE_ALTITUDE_FT {
        TrafficClass::Proximate
    } else {
        TrafficClass::Other
    }
}

/// Straight-line projection of the pass. Positions are in NM east/north of the aircraft, velocities
/// in knots, altitudes in feet and feet per minute. `None` when the two are not closing.
pub fn closest_approach(
    relative_nm: [f64; 2],
    relative_kt: [f64; 2],
    relative_altitude_ft: f64,
    relative_fpm: f64,
) -> Option<(f64, f64, f64)> {
    let speed_sq = relative_kt[0] * relative_kt[0] + relative_kt[1] * relative_kt[1];
    if speed_sq < 1e-6 {
        return None;
    }
    let along = relative_nm[0] * relative_kt[0] + relative_nm[1] * relative_kt[1];
    if along >= 0. {
        return None;
    }
    let hours = -along / speed_sq;
    let at = [
        relative_nm[0] + relative_kt[0] * hours,
        relative_nm[1] + relative_kt[1] * hours,
    ];
    Some((
        hours * 3600.,
        libm::hypot(at[0], at[1]),
        relative_altitude_ft + relative_fpm * hours * 60.,
    ))
}

/// One other aircraft as the displays see it, from either traffic source.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TrafficTarget {
    /// Stable 24-bit address: `FLEET_ADDRESS_BASE + id` for the fleet. Also the GDL 90 address.
    pub address: u32,
    /// The fleet aircraft id that comms events carry (`target_aircraft`); none for circuit traffic.
    pub fleet_id: Option<u32>,
    pub origin: TrafficOrigin,
    /// Spoken, such as `Cessna Four Two Seven Kilo Papa`.
    pub callsign: String,
    /// Written, such as `N427KP`.
    pub registration: String,
    pub aircraft_type: String,
    pub type_designator: String,
    pub latitude_deg: Option<f64>,
    pub longitude_deg: Option<f64>,
    pub position_m: [f64; 3],
    pub altitude_msl_ft: f64,
    pub track_true_deg: f64,
    /// Heading in the region's grid frame, for placing the symbol on a projected chart.
    pub track_grid_rad: f64,
    pub ground_speed_kt: f64,
    pub vertical_rate_fpm: f64,
    pub on_ground: bool,
    pub range_nm: f64,
    pub bearing_true_deg: f64,
    /// Traffic minus the aircraft, feet.
    pub relative_altitude_ft: f64,
    pub class: TrafficClass,
    /// The aircraft is on course to pass the user inside the alert volume soon.
    pub closing: bool,
    pub squawk: Option<String>,
    pub frequency_khz: Option<u32>,
    pub facility: Option<String>,
    /// Flow and phase, such as `arrival: final approach`.
    pub intent: Option<String>,
    pub assigned_altitude_ft: Option<u32>,
    pub assigned_heading_deg: Option<u16>,
    pub assigned_speed_kt: Option<u16>,
    /// A recent controller call named this aircraft.
    pub highlighted: bool,
    pub highlight_remaining_s: f64,
}
impl TrafficTarget {
    /// Relative altitude in hundreds of feet with a sign, as drawn on glass: `+05`, `-12`, `00`.
    pub fn altitude_tag(&self) -> String {
        altitude_tag(self.relative_altitude_ft)
    }
    /// `1` climbing, `-1` descending, `0` level, by the 500 fpm threshold.
    pub fn trend(&self) -> i8 {
        trend(self.vertical_rate_fpm)
    }
    /// Short map label: written registration and type, such as `N427KP C172`.
    pub fn short_label(&self) -> String {
        if self.type_designator.is_empty() {
            self.registration.clone()
        } else {
            format!("{} {}", self.registration, self.type_designator)
        }
    }
    /// The details a hover or click on the map shows: identity, altitude, speed, assigned values,
    /// frequency, and intent.
    pub fn details(&self) -> String {
        let mut text = format!(
            "{} ({})\n{}\nALT {:.0} ft ({} rel)  GS {:.0} kt  TRK {:03.0}  V/S {:+.0} fpm\nRANGE {:.1} NM  BRG {:03.0}",
            self.callsign,
            self.registration,
            if self.aircraft_type.is_empty() {
                self.type_designator.clone()
            } else {
                format!("{} {}", self.aircraft_type, self.type_designator)
            },
            self.altitude_msl_ft,
            self.altitude_tag(),
            self.ground_speed_kt,
            self.track_true_deg,
            self.vertical_rate_fpm,
            self.range_nm,
            self.bearing_true_deg,
        );
        let mut assigned = Vec::new();
        if let Some(ft) = self.assigned_altitude_ft {
            assigned.push(format!("alt {ft} ft"));
        }
        if let Some(deg) = self.assigned_heading_deg {
            assigned.push(format!("hdg {deg:03}"));
        }
        if let Some(kt) = self.assigned_speed_kt {
            assigned.push(format!("spd {kt} kt"));
        }
        text.push_str(&format!(
            "\nASSIGNED {}",
            if assigned.is_empty() {
                "none".into()
            } else {
                assigned.join(", ")
            }
        ));
        if let Some(khz) = self.frequency_khz {
            text.push_str(&format!(
                "\nFREQ {:.3}{}",
                f64::from(khz) / 1000.,
                self.facility.as_ref().map_or(String::new(), |f| format!(" {f}"))
            ));
        }
        if let Some(squawk) = &self.squawk {
            text.push_str(&format!("  SQUAWK {squawk}"));
        }
        if let Some(intent) = &self.intent {
            text.push_str(&format!("\nINTENT {intent}"));
        }
        match self.class {
            TrafficClass::Alert => text.push_str("\nTRAFFIC ALERT"),
            TrafficClass::Proximate => text.push_str("\nPROXIMATE"),
            TrafficClass::Other => {}
        }
        if self.highlighted {
            text.push_str("\nNAMED BY CONTROLLER");
        }
        text
    }
}
pub fn altitude_tag(relative_altitude_ft: f64) -> String {
    let hundreds = (relative_altitude_ft / 100.).round() as i64;
    match hundreds {
        0 => "00".into(),
        n if n > 0 => format!("+{n:02}"),
        n => format!("-{:02}", -n),
    }
}
pub fn trend(vertical_rate_fpm: f64) -> i8 {
    if vertical_rate_fpm > TREND_ARROW_FPM {
        1
    } else if vertical_rate_fpm < -TREND_ARROW_FPM {
        -1
    } else {
        0
    }
}

/// Aircraft a transmission names: fleet aircraft ids and callsigns or registrations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrafficReferences {
    pub aircraft_ids: Vec<u32>,
    pub callsigns: Vec<String>,
}
const ID_KEYS: &[&str] = &[
    "target_aircraft",
    "target_aircraft_id",
    "target_aircraft_ids",
    "traffic_aircraft",
    "aircraft_ids",
];
const CALLSIGN_KEYS: &[&str] = &["follow", "follows", "target_callsign", "traffic_callsign"];

/// Aircraft the transmission names, searched through the whole typed meaning: any
/// `target_aircraft` style id (a number or a list), a sequencing `follow`/`follows` callsign, and the
/// `callsign` of a traffic advisory. The speaker's own identity (`call.aircraft`, `call.callsign`)
/// is never a reference, and nothing an AI pilot says names another aircraft.
pub fn traffic_references(event: &Transmission) -> TrafficReferences {
    let mut found = TrafficReferences::default();
    let Ok(meaning) = serde_json::to_value(&event.meaning) else {
        return found;
    };
    let advisory = meaning.get("type").and_then(Value::as_str) == Some("traffic_advisory");
    let pilot = meaning
        .get("call")
        .and_then(|c| c.get("role"))
        .and_then(Value::as_str)
        == Some("pilot");
    if !pilot {
        collect_references(&meaning, advisory, &mut found);
    }
    found.aircraft_ids.sort_unstable();
    found.aircraft_ids.dedup();
    found.callsigns.sort();
    found.callsigns.dedup();
    found
}
fn collect_references(value: &Value, advisory: bool, found: &mut TrafficReferences) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                let key = key.as_str();
                if ID_KEYS.contains(&key) {
                    match v {
                        Value::Number(n) => found
                            .aircraft_ids
                            .extend(n.as_u64().and_then(|n| u32::try_from(n).ok())),
                        Value::Array(items) => found.aircraft_ids.extend(
                            items
                                .iter()
                                .filter_map(Value::as_u64)
                                .filter_map(|n| u32::try_from(n).ok()),
                        ),
                        _ => {}
                    }
                } else if CALLSIGN_KEYS.contains(&key) || (advisory && key == "callsign") {
                    if let Some(text) = v.as_str().filter(|t| !t.is_empty()) {
                        found.callsigns.push(text.to_owned());
                    }
                } else {
                    collect_references(v, advisory, found);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_references(v, advisory, found)),
        _ => {}
    }
}
fn same_callsign(a: &str, b: &str) -> bool {
    !a.is_empty() && a.eq_ignore_ascii_case(b)
}
impl TrafficReferences {
    pub fn names(&self, target: &TrafficTarget) -> bool {
        target.fleet_id.is_some_and(|id| self.aircraft_ids.contains(&id))
            || self
                .callsigns
                .iter()
                .any(|c| same_callsign(c, &target.callsign) || same_callsign(c, &target.registration))
    }
}

/// The aircraft addresses highlighted at `now_s` and the seconds each has left, from the events
/// still inside the highlight window. Calls that were not heard name nothing.
pub fn highlights(events: &[Transmission], targets: &[TrafficTarget], now_s: f64) -> Vec<(u32, f64)> {
    let mut result: Vec<(u32, f64)> = Vec::new();
    for event in events.iter().rev() {
        let end = event.time_s + event.duration_s.max(0.);
        if now_s - end > HIGHLIGHT_S + 60. {
            break;
        }
        if event.time_s > now_s || !matches!(event.reception, Reception::Heard | Reception::Pending) {
            continue;
        }
        let remaining = if now_s <= end {
            HIGHLIGHT_S
        } else {
            HIGHLIGHT_S - (now_s - end)
        };
        if remaining <= 0. {
            continue;
        }
        let refs = traffic_references(event);
        for target in targets.iter().filter(|t| refs.names(t)) {
            match result.iter_mut().find(|(a, _)| *a == target.address) {
                Some(entry) => entry.1 = entry.1.max(remaining),
                None => result.push((target.address, remaining)),
            }
        }
    }
    result
}

impl Session {
    /// Whether the aircraft has a traffic receiver feeding its glass displays: an ADS-B In
    /// (or TIS-B) installation, declared by `systems.definition.adsb_in` and defaulting to installed
    /// wherever there is a glass display.
    pub fn adsb_in_installed(&self) -> bool {
        let Some(c) = self.aircraft_configuration() else {
            return false;
        };
        let glass_load = c.installed_systems.definition.as_ref().is_some_and(|d| {
            d.electrical
                .loads
                .iter()
                .any(|l| l.id == "glass_pfd" || l.id == "glass_mfd")
        });
        let touch = c.panel.equipment.as_ref().is_some_and(|p| {
            p.elements.iter().any(|e| {
                matches!(
                    e.element,
                    flightcraft_aero::modern::PanelElement::TouchDisplay { .. }
                )
            })
        });
        c.installed_systems
            .definition
            .as_ref()
            .map_or(glass_load || touch, |d| d.adsb_in_installed(glass_load || touch))
    }

    /// Every other aircraft, nearest first, with its symbol class and any controller highlight.
    pub fn traffic_targets(&self) -> Vec<TrafficTarget> {
        let [pn, pe, pd] = self.sim.state().position_m;
        let [vn, ve, vd] = self.sim.state().velocity_mps;
        let grid_north = self.sim.heading_reference().grid_north_true_rad;
        let user_track = libm::atan2(ve, vn) + grid_north;
        let user_kt = libm::hypot(vn, ve) / 0.514_444;
        let user_velocity = [user_kt * libm::sin(user_track), user_kt * libm::cos(user_track)];
        let user_fpm = -vd / FT_M * 60.;
        let mut targets = Vec::new();
        // Circuit traffic first; the fleet's circuit and final aircraft appear in `traffic_state` too
        // but are listed here from the richer fleet state.
        for a in self
            .traffic_state()
            .aircraft
            .into_iter()
            .filter(|a| a.address < FLEET_ADDRESS_BASE)
        {
            let dn = a.position_m[0] - pn;
            let de = a.position_m[1] - pe;
            let bearing = libm::atan2(de, dn) + grid_north;
            let track = a.heading_grid_rad + grid_north;
            let speed_kt = a.speed_mps / 0.514_444;
            targets.push(self.build_target(
                BuildInput {
                    address: a.address,
                    fleet_id: None,
                    origin: TrafficOrigin::Circuit,
                    callsign: a.callsign,
                    registration: a.registration,
                    aircraft_type: a.aircraft_type,
                    type_designator: String::new(),
                    latitude_deg: a.latitude_deg,
                    longitude_deg: a.longitude_deg,
                    position_m: a.position_m,
                    altitude_msl_ft: a.altitude_msl_m / FT_M,
                    track_true_deg: track.to_degrees().rem_euclid(360.),
                    track_grid_rad: a.heading_grid_rad,
                    ground_speed_kt: speed_kt,
                    vertical_rate_fpm: a.vertical_speed_mps / FT_M * 60.,
                    on_ground: false,
                    range_nm: libm::hypot(dn, de) / NM_M,
                    bearing_true_deg: bearing.to_degrees().rem_euclid(360.),
                    relative_altitude_ft: (a.altitude_msl_m + pd) / FT_M,
                    squawk: None,
                    frequency_khz: self.comms.pattern.as_ref().map(|p| p.frequency_khz),
                    facility: None,
                    intent: Some(format!("circuit: {:?}", a.leg).to_lowercase()),
                    assigned: (None, None, None),
                },
                user_velocity,
                user_fpm,
            ));
        }
        for a in self.fleet_state().aircraft {
            let grid_track = (a.track_true_deg.to_radians() - grid_north).rem_euclid(core::f64::consts::TAU);
            targets.push(self.build_target(
                BuildInput {
                    address: FLEET_ADDRESS_BASE + a.id,
                    fleet_id: Some(a.id),
                    origin: TrafficOrigin::Fleet,
                    callsign: a.callsign,
                    registration: a.registration,
                    aircraft_type: a.aircraft_type,
                    type_designator: a.type_designator,
                    latitude_deg: a.latitude_deg,
                    longitude_deg: a.longitude_deg,
                    position_m: a.position_m,
                    altitude_msl_ft: a.altitude_msl_ft,
                    track_true_deg: a.track_true_deg,
                    track_grid_rad: grid_track,
                    ground_speed_kt: a.ground_speed_kt,
                    vertical_rate_fpm: a.vertical_rate_fpm,
                    on_ground: a.on_ground,
                    range_nm: a.range_nm,
                    bearing_true_deg: a.bearing_true_deg,
                    relative_altitude_ft: a.relative_altitude_ft,
                    squawk: Some(a.squawk),
                    frequency_khz: Some(a.frequency_khz),
                    facility: Some(a.facility),
                    intent: Some(format!(
                        "{}: {}",
                        match a.intent.kind {
                            FleetFlight::Arrival => "arrival",
                            FleetFlight::Departure => "departure",
                            FleetFlight::FlightFollowing => "flight following",
                            FleetFlight::Pattern => "circuit",
                        },
                        a.intent.phase
                    )),
                    assigned: (
                        a.assigned_altitude_ft,
                        a.assigned_heading_deg,
                        a.assigned_speed_kt,
                    ),
                },
                user_velocity,
                user_fpm,
            ));
        }
        targets.sort_by(|a, b| a.range_nm.total_cmp(&b.range_nm).then(a.address.cmp(&b.address)));
        let lit = highlights(&self.comms.record.events, &targets, self.sim.state().time_s);
        for t in &mut targets {
            if let Some((_, left)) = lit.iter().find(|(a, _)| *a == t.address) {
                t.highlighted = true;
                t.highlight_remaining_s = *left;
            }
        }
        targets
    }

    fn build_target(&self, i: BuildInput, user_velocity: [f64; 2], user_fpm: f64) -> TrafficTarget {
        let bearing = i.bearing_true_deg.to_radians();
        let relative_nm = [i.range_nm * libm::sin(bearing), i.range_nm * libm::cos(bearing)];
        let track = i.track_true_deg.to_radians();
        let relative_kt = [
            i.ground_speed_kt * libm::sin(track) - user_velocity[0],
            i.ground_speed_kt * libm::cos(track) - user_velocity[1],
        ];
        let pass = closest_approach(
            relative_nm,
            relative_kt,
            i.relative_altitude_ft,
            i.vertical_rate_fpm - user_fpm,
        );
        let class = classify(i.range_nm, i.relative_altitude_ft, i.on_ground, pass);
        let closing = pass.is_some_and(|(tau, cpa, _)| tau <= ALERT_TAU_S && cpa <= ALERT_CPA_NM);
        TrafficTarget {
            address: i.address,
            fleet_id: i.fleet_id,
            origin: i.origin,
            callsign: i.callsign,
            registration: i.registration,
            aircraft_type: i.aircraft_type,
            type_designator: i.type_designator,
            latitude_deg: i.latitude_deg,
            longitude_deg: i.longitude_deg,
            position_m: i.position_m,
            altitude_msl_ft: i.altitude_msl_ft,
            track_true_deg: i.track_true_deg,
            track_grid_rad: i.track_grid_rad,
            ground_speed_kt: i.ground_speed_kt,
            vertical_rate_fpm: i.vertical_rate_fpm,
            on_ground: i.on_ground,
            range_nm: i.range_nm,
            bearing_true_deg: i.bearing_true_deg,
            relative_altitude_ft: i.relative_altitude_ft,
            class,
            closing,
            squawk: i.squawk,
            frequency_khz: i.frequency_khz,
            facility: i.facility,
            intent: i.intent,
            assigned_altitude_ft: i.assigned.0,
            assigned_heading_deg: i.assigned.1,
            assigned_speed_kt: i.assigned.2,
            highlighted: false,
            highlight_remaining_s: 0.,
        }
    }
}
struct BuildInput {
    address: u32,
    fleet_id: Option<u32>,
    origin: TrafficOrigin,
    callsign: String,
    registration: String,
    aircraft_type: String,
    type_designator: String,
    latitude_deg: Option<f64>,
    longitude_deg: Option<f64>,
    position_m: [f64; 3],
    altitude_msl_ft: f64,
    track_true_deg: f64,
    track_grid_rad: f64,
    ground_speed_kt: f64,
    vertical_rate_fpm: f64,
    on_ground: bool,
    range_nm: f64,
    bearing_true_deg: f64,
    relative_altitude_ft: f64,
    squawk: Option<String>,
    frequency_khz: Option<u32>,
    facility: Option<String>,
    intent: Option<String>,
    assigned: (Option<u32>, Option<u16>, Option<u16>),
}

#[cfg(test)]
mod tests;
