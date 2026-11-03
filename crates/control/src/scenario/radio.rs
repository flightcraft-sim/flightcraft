//! Temporary adapter for typed scenario calls; switch this entry point to the ATC renderer on integration.
use super::Instruction;
use crate::comms::*;
use flightcraft_world::ComStation;
fn runway_words(runway: &str) -> String {
    runway
        .chars()
        .map(|c| match c {
            'L' => "left".into(),
            'R' => "right".into(),
            'C' => "center".into(),
            _ => c
                .to_digit(10)
                .map_or_else(|| c.to_string(), |n| crate::comms::weather::digits(n, 1)),
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub(super) fn scenario_radio_text(
    station: &ComStation,
    pilot: &PilotCallsign,
    instruction: &Instruction,
    wind: flightcraft_sim::Weather,
) -> String {
    let body = match instruction {
        Instruction::Land { runway, .. } => format!(
            "runway {}, cleared to land, {}",
            runway_words(runway),
            crate::comms::weather::spoken_wind(wind)
        ),
        Instruction::GoAround { runway, .. } => format!("go around, runway {}", runway_words(runway)),
        Instruction::Altitude { altitude_msl_m, .. } => {
            format!("maintain {}", crate::comms::weather::height(*altitude_msl_m))
        }
        Instruction::AirspaceClearance { id } => format!("cleared into {id}"),
        Instruction::RequestReadback { text } | Instruction::Advisory { text } => {
            text.trim_end_matches(['.', ' ']).to_owned()
        }
    };
    format!("{}, {}, {}.", pilot.current, facility_radio_name(station), body)
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::Session;
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
            scenario_radio_text(station, &pilot, &instruction, wind),
            "Sling Three Four Bravo, Allentown Tower, runway two four, cleared to land, wind calm."
        );
        wind.wind_from_deg = 230.;
        wind.wind_speed_mps = 8. * 0.514444;
        assert!(
            scenario_radio_text(station, &pilot, &instruction, wind)
                .ends_with("wind two three zero at eight.")
        );
    }
}
