//! Bounded IFR clearances using imported procedures and the existing readback queue.
use super::*;
use crate::lesson::{PublishedHold, engine::ApproachPath, wrap_deg};
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IfrInstruction {
    VectorToFinal {
        procedure: String,
        heading_deg: u16,
        altitude_ft: u32,
    },
    Hold {
        procedure: String,
        fix: String,
        efc_utc: u16,
    },
    Missed {
        procedure: String,
    },
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueIfr {
    pub airport: String,
    pub instruction: IfrInstruction,
}
impl Command for IssueIfr {
    const NAME: &'static str = "atc.ifr";
    const DESCRIPTION: &'static str = "Issue a validated final intercept, published missed approach, or published hold with EFC through the normal radio and readback queue.";
    type Output = AtcState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let st = s
            .next_station(&self.airport, FacilityKind::Approach)
            .ok_or_else(|| invalid(Self::NAME, "imported approach facility unavailable"))?;
        if !s.comms.atc.pending.is_empty() {
            return Err(invalid(Self::NAME, "read back the outstanding instruction first"));
        }
        let instruction = match self.instruction {
            IfrInstruction::VectorToFinal {
                procedure,
                heading_deg,
                altitude_ft,
            } => {
                if heading_deg > 360 {
                    return Err(invalid(
                        Self::NAME,
                        "heading must be between zero and 360 degrees",
                    ));
                }
                s.validate_final_intercept(&self.airport, &procedure, heading_deg, altitude_ft)?;
                let p = s
                    .sim
                    .world()
                    .procedures()
                    .and_then(|d| {
                        d.procedures
                            .iter()
                            .find(|p| p.airport == self.airport && p.ident == procedure)
                    })
                    .ok_or_else(|| invalid(Self::NAME, "procedure unavailable"))?;
                let runway = p
                    .runway
                    .as_deref()
                    .unwrap_or("")
                    .trim_start_matches("RW")
                    .to_owned();
                s.comms.atc.vector_approach = Some(AtcInstruction::Approach {
                    airport: self.airport.clone(),
                    intercept: None,
                    procedure: procedure.clone(),
                    name: format!(
                        "{} runway {}",
                        if procedure.starts_with('I') {
                            "ILS"
                        } else if procedure.starts_with('L') {
                            "LOC"
                        } else {
                            "RNAV"
                        },
                        runway
                    ),
                    runway,
                });
                AtcInstruction::Vector {
                    heading_deg,
                    altitude_ft,
                }
            }
            IfrInstruction::Hold {
                procedure,
                fix,
                efc_utc,
            } => {
                if efc_utc / 100 > 23 || efc_utc % 100 > 59 {
                    return Err(invalid(Self::NAME, "EFC must be a UTC HHMM time"));
                }
                let hold = PublishedHold::find(s, &self.airport, &procedure, &fix)?;
                AtcInstruction::Hold {
                    fix: hold.fix,
                    inbound_deg: hold.inbound_deg.round() as u16,
                    right_turns: hold.turn == flightcraft_world::procedures::Turn::R,
                    leg_time_s: hold.time_s.map(|v| v.round() as u16),
                    leg_distance_nm: hold.distance_nm,
                    altitude_ft: (hold.altitude_m / 0.3048).round() as u32,
                    efc_utc,
                }
            }
            IfrInstruction::Missed { procedure } => {
                ApproachPath::find(s, &self.airport, &procedure)?;
                AtcInstruction::MissedApproach {
                    airport: self.airport.clone(),
                    procedure,
                }
            }
        };
        s.comms.atc.airport = Some(self.airport);
        s.issue(&st, instruction);
        Ok(s.atc_state())
    }
}
impl Session {
    pub(crate) fn validate_final_intercept(
        &self,
        airport: &str,
        procedure: &str,
        heading_deg: u16,
        altitude_ft: u32,
    ) -> Result<(), CommandError> {
        let path = ApproachPath::find(self, airport, procedure)?;
        let r = self.sim.readout();
        let position = flightcraft_world::GeoPoint {
            latitude_deg: r
                .latitude_deg
                .ok_or_else(|| invalid("atc.ifr", "geographic position required"))?,
            longitude_deg: r
                .longitude_deg
                .ok_or_else(|| invalid("atc.ifr", "geographic position required"))?,
        };
        let remaining = path.remaining(position);
        let true_heading = self
            .sim
            .heading_reference()
            .magnetic_to_grid(f64::from(heading_deg).to_radians())
            + r.grid_north_true_rad;
        let difference_signed = wrap_deg(true_heading.to_degrees() - path.course_deg);
        let difference = difference_signed.abs();
        let (distance, bearing) = path.endpoint.inverse(position);
        let cross = distance * libm::sin((bearing - path.course_deg).to_radians());
        let sine = libm::sin(difference_signed.to_radians());
        let intercept_distance = if cross.abs() < 30. {
            0.
        } else if sine.abs() > 0.001 {
            -cross / sine
        } else {
            return Err(invalid("atc.ifr", "parallel vector does not intercept final"));
        };
        if !(0. ..=30. * 1852.).contains(&intercept_distance) {
            return Err(invalid("atc.ifr", "vector points away from the final course"));
        }
        let intercept_remaining = remaining - intercept_distance * libm::cos(difference_signed.to_radians());
        let limit = if intercept_remaining / 1852. - path.faf_distance_nm < 2. {
            20.
        } else {
            30.
        };
        if difference > limit + 0.1 {
            return Err(invalid(
                "atc.ifr",
                "final intercept exceeds the JO 7110.65 angle limit",
            ));
        }
        // JO 7110.65 5-9-1: absent the weather/pilot-request exceptions, vector at least
        // two miles outside the approach gate (one mile outside the FAF, at least five from runway).
        let gate_nm = (path.faf_distance_nm + 1.).max(5.);
        if difference > 1. && intercept_remaining / 1852. < gate_nm + 2. {
            return Err(invalid(
                "atc.ifr",
                "vector must intercept at least two miles outside the approach gate",
            ));
        }
        if intercept_remaining / 1852. < path.faf_distance_nm {
            return Err(invalid(
                "atc.ifr",
                "final intercept must be outside the final approach fix",
            ));
        }
        if f64::from(altitude_ft) * 0.3048 > path.altitude(intercept_remaining) + 1. {
            return Err(invalid(
                "atc.ifr",
                "final intercept must be at or below the glidepath",
            ));
        }
        let [n, e, _] = self.sim.state().position_m;
        let grid_heading = true_heading - r.grid_north_true_rad;
        let samples = (intercept_distance / 250.).ceil() as u32;
        for step in 0..=samples {
            let distance = intercept_distance * f64::from(step) / f64::from(samples.max(1));
            let sn = n + distance * libm::cos(grid_heading);
            let se = e + distance * libm::sin(grid_heading);
            if self.sim.world().terrain_ground(sn, se).is_none()
                || f64::from(altitude_ft) * 0.3048 < self.sim.world().ground(sn, se).height_msl_m + 304.8
            {
                return Err(invalid(
                    "atc.ifr",
                    "vector altitude lacks terrain coverage or 1,000 feet of terrain clearance",
                ));
            }
        }
        Ok(())
    }
}

impl Session {
    pub(super) fn complete_final_intercept(&mut self) {
        let Some(AtcInstruction::Approach {
            airport, procedure, ..
        }) = self.comms.atc.vector_approach.clone()
        else {
            return;
        };
        if !self.comms.atc.pending.is_empty()
            || !self.comms.atc.replies.is_empty()
            || !self
                .comms
                .atc
                .active
                .iter()
                .any(|p| p.readback && matches!(p.instruction, AtcInstruction::Vector { .. }))
        {
            return;
        }
        let Some(st) = self
            .comms
            .atc
            .current
            .clone()
            .filter(|s| s.kind == FacilityKind::Approach)
        else {
            return;
        };
        let Ok(path) = ApproachPath::find(self, &airport, &procedure) else {
            return;
        };
        let r = self.sim.readout();
        let (Some(latitude_deg), Some(longitude_deg)) = (r.latitude_deg, r.longitude_deg) else {
            return;
        };
        let position = flightcraft_world::GeoPoint {
            latitude_deg,
            longitude_deg,
        };
        let remaining = path.remaining(position);
        let (distance, bearing) = path.endpoint.inverse(position);
        let cross = distance * libm::sin((bearing - path.course_deg).to_radians());
        let [vn, ve, _] = self.sim.state().velocity_mps;
        let track = libm::atan2(ve, vn) + r.grid_north_true_rad;
        if cross.abs() < 100.
            && wrap_deg(track.to_degrees() - path.course_deg).abs() < 10.
            && r.altitude_msl_m <= path.altitude(remaining) + 9.144
            && let Some(instruction) = self.comms.atc.vector_approach.take()
        {
            self.issue(&st, instruction);
        }
    }
}

impl Session {
    /// The airborne lesson is a continuation of a filed IFR clearance, not a VFR radar request.
    pub(super) fn issue_lesson_intercept(&mut self, st: &ComStation) -> bool {
        let Some(setup) = self.lesson.document.as_ref().map(|d| d.setup.clone()) else {
            return false;
        };
        if self
            .comms
            .atc
            .active
            .iter()
            .any(|p| matches!(p.instruction, AtcInstruction::Approach { .. }))
        {
            return true;
        }
        let Ok(path) = ApproachPath::find(self, &setup.airport, &setup.procedure) else {
            return false;
        };
        let r = self.sim.readout();
        let (Some(latitude_deg), Some(longitude_deg)) = (r.latitude_deg, r.longitude_deg) else {
            return false;
        };
        let position = flightcraft_world::GeoPoint {
            latitude_deg,
            longitude_deg,
        };
        let (distance, bearing) = path.endpoint.inverse(position);
        let cross = distance * libm::sin((bearing - path.course_deg).to_radians());
        let course = self
            .sim
            .heading_reference()
            .grid_to_magnetic(path.course_deg.to_radians() - r.grid_north_true_rad)
            .to_degrees();
        let heading_deg = (course
            + if cross > 100. {
                -20.
            } else if cross < -100. {
                20.
            } else {
                0.
            })
        .round()
        .rem_euclid(360.) as u16;
        // Use the published intermediate altitude, not an altitude computed below the FAF.
        let Some(altitude_ft) = self
            .sim
            .world()
            .procedures()
            .and_then(|d| {
                d.procedures
                    .iter()
                    .find(|p| p.airport == setup.airport && p.ident == setup.procedure)
            })
            .and_then(|p| p.routes.iter().find(|r| r.transition.is_empty()))
            .and_then(|r| r.legs.first())
            .and_then(|l| l.altitude1_m)
            .map(|m| (m / 0.3048).round() as u32)
        else {
            return false;
        };
        if self
            .validate_final_intercept(&setup.airport, &setup.procedure, heading_deg, altitude_ft)
            .is_err()
        {
            self.atc_reply(
                st,
                "Unable final vector from this position and altitude. Request a new intercept.".into(),
                Meaning::Text,
            );
            return true;
        }
        let Some(runway) = self
            .sim
            .world()
            .procedures()
            .and_then(|d| {
                d.procedures
                    .iter()
                    .find(|p| p.airport == setup.airport && p.ident == setup.procedure)
            })
            .and_then(|p| p.runway.as_deref())
            .map(|r| r.trim_start_matches("RW").to_owned())
        else {
            return false;
        };
        let family = match setup.mode {
            crate::InstrumentApproach::Ils => "ILS",
            crate::InstrumentApproach::Localizer => "LOC",
            _ => "RNAV",
        };
        self.issue(
            st,
            AtcInstruction::Approach {
                airport: setup.airport,
                procedure: setup.procedure,
                name: format!("{family} runway {runway}"),
                runway,
                intercept: Some(ApproachIntercept {
                    heading_deg,
                    altitude_ft,
                    turn_left: wrap_deg(f64::from(heading_deg) - r.heading_rad.to_degrees()) <= 0.,
                }),
            },
        );
        true
    }
    /// Instrument handoff is a spatial event at the FAF; VFR arrivals retain their range trigger.
    pub(super) fn at_instrument_faf(&self) -> Option<bool> {
        let (airport, procedure) = self
            .comms
            .atc
            .active
            .iter()
            .rev()
            .find_map(|p| match &p.instruction {
                AtcInstruction::Approach {
                    airport, procedure, ..
                } if p.readback && procedure != "visual" => Some((airport, procedure)),
                _ => None,
            })?;
        let path = ApproachPath::find(self, airport, procedure).ok()?;
        let r = self.sim.readout();
        let position = flightcraft_world::GeoPoint {
            latitude_deg: r.latitude_deg?,
            longitude_deg: r.longitude_deg?,
        };
        Some(path.remaining(position) / 1852. <= path.faf_distance_nm + 0.25)
    }
}
