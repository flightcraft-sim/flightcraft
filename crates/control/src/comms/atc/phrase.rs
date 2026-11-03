//! Typed instruction wording and compact model readbacks, using the shared radio primitives.
use super::*;
use crate::comms::phraseology as p;
impl Session {
    pub(super) fn airport_radio_name(&self, airport: &str) -> String {
        self.stations()
            .iter()
            .find(|s| s.airport == airport && s.kind == FacilityKind::Tower)
            .map(|s| {
                p::normal_case(
                    s.metadata
                        .as_ref()
                        .map_or(s.name.as_str(), |m| m.callsign.as_str()),
                )
            })
            .or_else(|| {
                crate::regions::airport(self.sim.world(), airport)
                    .ok()
                    .map(|a| p::normal_case(&a.name))
            })
            .unwrap_or_else(|| "the destination".into())
    }
    pub(super) fn instruction_phrase(&self, i: &AtcInstruction) -> String {
        let altitude = |ft| p::altitude_assignment(ft, -self.sim.state().position_m[2] / 0.3048);
        match i {
            AtcInstruction::Parking => "Taxi to parking. Use pilot discretion on the apron.".into(),
            AtcInstruction::Taxi {
                runway, hold_short, ..
            } => format!(
                "Runway {}, taxi to the runway.{}",
                p::runway(runway),
                if *hold_short {
                    format!(" Hold short of runway {}.", p::runway(runway))
                } else {
                    String::new()
                }
            ),
            AtcInstruction::Takeoff {
                runway,
                closed_traffic,
            } => format!(
                "Runway {}, cleared for takeoff.{}",
                p::runway(runway),
                if *closed_traffic {
                    " Make left closed traffic."
                } else {
                    ""
                }
            ),
            AtcInstruction::Land { runway } => format!("Runway {}, cleared to land.", p::runway(runway)),
            AtcInstruction::Pattern {
                runway,
                entry,
                number,
                follow,
            } => {
                let traffic = follow.as_ref().and_then(|call| {
                    self.traffic_state()
                        .aircraft
                        .into_iter()
                        .find(|a| a.callsign == *call)
                });
                let suffix = traffic.map_or(String::new(), |a| {
                    format!(
                        " Number {}, follow the {} on {}.{}",
                        p::digits(u32::from(*number), 1),
                        a.aircraft_type,
                        a.leg.phrase(),
                        if a.aircraft_type.contains("Heavy") {
                            " Caution wake turbulence."
                        } else {
                            ""
                        }
                    )
                });
                format!(
                    "Enter {} runway {}. Report {}.{suffix}",
                    super::rules::entry_phrase(*entry),
                    p::runway(runway),
                    if *entry == PatternEntry::StraightIn {
                        "final"
                    } else {
                        "downwind"
                    }
                )
            }
            AtcInstruction::ExtendDownwind => "Extend downwind, I will call your base.".into(),
            AtcInstruction::GoAround { runway } => {
                format!("Go around, runway {}, make left traffic.", p::runway(runway))
            }
            AtcInstruction::Vector {
                heading_deg,
                altitude_ft,
            } => format!(
                "Fly heading {}, {}.",
                p::heading(*heading_deg),
                altitude(*altitude_ft)
            ),
            AtcInstruction::Altitude { altitude_ft } => format!("{}.", altitude(*altitude_ft)),
            AtcInstruction::Squawk { code } => format!("Squawk {}.", p::digits(code.parse().unwrap_or(0), 4)),
            AtcInstruction::Handoff {
                station_id,
                channel_khz,
            } => format!(
                "Contact {} {}.",
                self.stations()
                    .iter()
                    .find(|s| s.id == *station_id)
                    .map_or("the next controller".into(), p::facility),
                p::frequency(*channel_khz)
            ),
            AtcInstruction::Bravo {
                cleared,
                heading_deg,
                altitude_ft,
            } => {
                if *cleared {
                    format!(
                        "Cleared into the Class Bravo. Fly heading {}, {}.",
                        p::heading(*heading_deg),
                        altitude(*altitude_ft)
                    )
                } else {
                    "Remain outside the Class Bravo. Request flight following for radar identification."
                        .into()
                }
            }
            AtcInstruction::Approach {
                name,
                runway,
                procedure,
                ..
            } => {
                if procedure == "visual" {
                    format!("Cleared visual approach runway {}.", p::runway(runway))
                } else {
                    format!(
                        "Cleared {} runway {} approach.",
                        name.split_whitespace().next().unwrap_or("published"),
                        p::runway(runway)
                    )
                }
            }
            AtcInstruction::IfrClearance {
                destination_name,
                route,
                altitude_ft,
                expected_altitude_ft,
                expect_minutes,
                departure_khz,
                code,
                ..
            } => format!(
                "Cleared to the {destination_name} airport {route}. Climb and maintain {}. Expect {} {} minutes after departure. Departure frequency {}. Squawk {}.",
                p::altitude(*altitude_ft),
                p::altitude(*expected_altitude_ft),
                p::digits(u32::from(*expect_minutes), 2),
                p::frequency(*departure_khz),
                p::digits(code.parse().unwrap_or(0), 4)
            ),
            AtcInstruction::ExitRunway { ground_khz, .. } => format!(
                "Exit at the next available taxiway, contact ground {}.",
                p::ground_frequency(*ground_khz)
            ),
        }
    }
    /// Short key-item readback for rehearsal clients. Appends the current aircraft identity.
    /// This is an example renderer; the parser also accepts equivalent pilot wording.
    pub fn atc_readback(&self, instruction: &AtcInstruction) -> String {
        let text = match instruction {
            AtcInstruction::Parking => "Taxi to parking".into(),
            AtcInstruction::ExitRunway { ground_khz, .. } => {
                format!("Contact ground {}", p::ground_frequency(*ground_khz))
            }
            AtcInstruction::Taxi {
                runway, hold_short, ..
            } => {
                if *hold_short {
                    format!("Hold short of runway {}", p::runway(runway))
                } else {
                    format!("Taxi to runway {}", p::runway(runway))
                }
            }
            AtcInstruction::Pattern {
                runway,
                entry,
                number,
                ..
            } => format!(
                "{}{} runway {}",
                if *number > 1 {
                    format!("Number {}, ", p::digits(u32::from(*number), 1))
                } else {
                    String::new()
                },
                super::rules::entry_phrase(*entry),
                p::runway(runway)
            ),
            AtcInstruction::Bravo {
                cleared: true,
                heading_deg,
                altitude_ft,
            } => format!(
                "Cleared into the Class Bravo, heading {}, {}",
                p::heading(*heading_deg),
                p::altitude_assignment(*altitude_ft, -self.sim.state().position_m[2] / 0.3048)
            ),
            AtcInstruction::Handoff {
                station_id,
                channel_khz,
            } => format!(
                "{} {}",
                self.stations()
                    .iter()
                    .find(|s| s.id == *station_id)
                    .map_or("Contact".into(), p::facility),
                p::frequency(*channel_khz)
            ),
            AtcInstruction::IfrClearance {
                destination_name,
                route,
                altitude_ft,
                expected_altitude_ft,
                expect_minutes,
                departure_khz,
                code,
                ..
            } => format!(
                "Cleared to {destination_name} {route}, climb and maintain {}, expect {} in {}, departure {}, squawk {}",
                p::altitude(*altitude_ft),
                p::altitude(*expected_altitude_ft),
                if *expect_minutes == 10 {
                    "ten".into()
                } else {
                    p::digits(u32::from(*expect_minutes), 1)
                },
                p::frequency(*departure_khz),
                p::digits(code.parse().unwrap_or(0), 4)
            ),
            AtcInstruction::Takeoff {
                runway,
                closed_traffic,
            } => format!(
                "Runway {}, cleared for takeoff{}",
                p::runway(runway),
                if *closed_traffic {
                    ", left closed traffic"
                } else {
                    ""
                }
            ),
            AtcInstruction::Land { runway } => format!("Runway {}, cleared to land", p::runway(runway)),
            AtcInstruction::GoAround { runway } => format!("Going around, runway {}", p::runway(runway)),
            AtcInstruction::ExtendDownwind => "Extending downwind".into(),
            AtcInstruction::Bravo { cleared: false, .. } => "Remaining outside the Class Bravo".into(),
            _ => self.instruction_phrase(instruction).trim_end_matches('.').into(),
        };
        format!("{text}, {}.", self.pilot_callsign().current)
    }
}
