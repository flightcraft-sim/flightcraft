//! Pre-flight walkaround lessons: a parked aircraft, a briefing, and optional seeded findings.
use super::SeedSpec;
use crate::{Command, CommandError, Session, invalid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightLesson {
    pub version: u8,
    pub id: String,
    pub title: String,
    pub briefing: String,
    /// Installed aircraft id; its pack must carry a `preflight` section.
    pub aircraft: String,
    pub region: String,
    pub airport: String,
    /// Runway end the aircraft is parked at.
    pub runway_end: String,
    /// Where the aircraft stands. Default: the ramp beside the runway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot: Option<crate::ParkingSpot>,
    /// Covers, ropes, and chocks are fitted when the lesson starts.
    pub covers_on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_fuel_l: Option<f64>,
    /// Seed for random findings. The same seed always seeds the same findings.
    pub seed: u64,
    /// Findings to seed at random from the pack's seedable defects.
    #[serde(default)]
    #[schemars(range(min = 0, max = 8))]
    pub random_findings: u8,
    /// Findings seeded explicitly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<SeedSpec>,
}

/// Original lesson documents, available identically in native and browser builds.
pub fn preflight_lesson_library() -> Result<Vec<PreflightLesson>, CommandError> {
    [
        include_str!("../../../../lessons/sling-preflight.json"),
        include_str!("../../../../lessons/sling-preflight-findings.json"),
    ]
    .iter()
    .map(|text| serde_json::from_str(text).map_err(|e| invalid("preflight.lessons", e.to_string())))
    .collect()
}

impl PreflightLesson {
    pub fn validate(&self) -> Result<(), CommandError> {
        let bad = |m: &str| invalid("preflight.begin", m.to_string());
        if self.version != 1
            || self.id.is_empty()
            || self.id.len() > 96
            || self.title.is_empty()
            || self.briefing.len() > 2048
            || self.findings.len() > 8
            || self.random_findings > 8
        {
            return Err(bad("invalid pre-flight lesson bounds"));
        }
        if let Some(l) = self.planned_fuel_l {
            crate::range("preflight.begin", "planned_fuel_l", l, 0., 500.)?;
        }
        Ok(())
    }
}

impl Session {
    /// Replace the flight with the lesson's: its aircraft, parked at its airport.
    pub(crate) fn start_preflight_lesson(&mut self, lesson: &PreflightLesson) -> Result<(), CommandError> {
        lesson.validate()?;
        let bad = |m: String| invalid("preflight.begin", m);
        if !self.regions.contains_key(&lesson.region) {
            return Err(bad(format!("lesson region is not installed: {}", lesson.region)));
        }
        if self
            .aircraft_catalog
            .source(flightcraft_aero::FileKind::Aircraft, &lesson.aircraft)
            .is_none()
        {
            return Err(bad(format!(
                "lesson aircraft is not installed: {}",
                lesson.aircraft
            )));
        }
        self.execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":lesson.aircraft},"reset":true}),
        )?;
        self.reset_at_place(&crate::LessonPlace {
            region: lesson.region.clone(),
            airport: lesson.airport.clone(),
            runway_end: lesson.runway_end.clone(),
            spot: lesson.spot.unwrap_or(crate::ParkingSpot::Ramp),
        })?;
        Ok(())
    }
}

/// List the pre-flight lessons.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightLessons {}

impl Command for PreflightLessons {
    const NAME: &'static str = "preflight.lessons";
    const DESCRIPTION: &'static str =
        "List the pre-flight walkaround lessons; start one with preflight.begin.";
    type Output = Vec<PreflightLesson>;
    fn run(self, _: &mut Session) -> Result<Self::Output, CommandError> {
        preflight_lesson_library()
    }
}
