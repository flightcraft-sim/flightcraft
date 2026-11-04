use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AtcInstruction {
    Hold {
        fix: String,
        inbound_deg: u16,
        right_turns: bool,
        leg_time_s: Option<u16>,
        leg_distance_nm: Option<f64>,
        altitude_ft: u32,
        efc_utc: u16,
    },
    MissedApproach {
        airport: String,
        procedure: String,
    },
    Parking,
    Taxi {
        runway: String,
        route: Vec<String>,
        hold_short: bool,
    },
    Takeoff {
        runway: String,
        closed_traffic: bool,
    },
    Land {
        runway: String,
    },
    Pattern {
        runway: String,
        entry: PatternEntry,
        number: u8,
        follow: Option<String>,
    },
    /// Touch-and-go landing clearance (JO 7110.65 3-9-4); used for circuit traffic.
    TouchAndGo {
        runway: String,
    },
    /// Speed adjustment (JO 7110.65 5-7-1); `None` cancels the restriction.
    Speed {
        knots: Option<u16>,
    },
    ExtendDownwind,
    GoAround {
        runway: String,
    },
    Vector {
        heading_deg: u16,
        altitude_ft: u32,
    },
    Altitude {
        altitude_ft: u32,
    },
    Squawk {
        code: String,
    },
    Handoff {
        station_id: String,
        channel_khz: u32,
    },
    Bravo {
        cleared: bool,
        heading_deg: u16,
        altitude_ft: u32,
    },
    Approach {
        airport: String,
        procedure: String,
        name: String,
        runway: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intercept: Option<ApproachIntercept>,
    },
    IfrClearance {
        destination: String,
        destination_name: String,
        expected_altitude_ft: u32,
        expect_minutes: u8,
        route: String,
        altitude_ft: u32,
        departure_khz: u32,
        code: String,
    },
    ExitRunway {
        runway: String,
        ground_khz: u32,
    },
}
/// A heading and altitude restriction remain in force until the final course is established.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproachIntercept {
    pub heading_deg: u16,
    pub altitude_ft: u32,
    pub turn_left: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatternEntry {
    StraightIn,
    LeftDownwind,
    RightDownwind,
    LeftBase,
    RightBase,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PendingInstruction {
    pub id: u64,
    pub station_id: String,
    pub airport: String,
    pub instruction: AtcInstruction,
    pub phrase: String,
    pub heard: bool,
    pub readback: bool,
    pub issued_s: f64,
    pub reminder_s: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ComplianceGrade {
    pub time_s: f64,
    pub instruction_id: Option<u64>,
    pub item: String,
    pub passed: bool,
    pub observed: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AtisReport {
    pub airport: String,
    pub station_id: String,
    pub channel_khz: u32,
    pub information: String,
    pub runway: String,
    pub remarks: Vec<String>,
    pub text: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LightGunSignal {
    SteadyGreen,
    FlashingGreen,
    SteadyRed,
    FlashingRed,
    FlashingWhite,
    AlternatingRedGreen,
}
impl LightGunSignal {
    pub fn label(self) -> &'static str {
        match self {
            Self::SteadyGreen => "Steady green",
            Self::FlashingGreen => "Flashing green",
            Self::SteadyRed => "Steady red",
            Self::FlashingRed => "Flashing red",
            Self::FlashingWhite => "Flashing white",
            Self::AlternatingRedGreen => "Alternating red and green",
        }
    }
    pub fn indication(self, on_ground: bool) -> &'static str {
        match (self, on_ground) {
            (Self::SteadyGreen, true) => "Cleared for takeoff",
            (Self::SteadyGreen, false) => "Cleared to land",
            (Self::FlashingGreen, true) => "Cleared to taxi",
            (Self::FlashingGreen, false) => "Return for landing; await steady green",
            (Self::SteadyRed, true) => "Stop",
            (Self::SteadyRed, false) => "Give way to other aircraft and continue circling",
            (Self::FlashingRed, true) => "Taxi clear of runway in use",
            (Self::FlashingRed, false) => "Airport unsafe; do not land",
            (Self::FlashingWhite, true) => "Return to starting point on airport",
            (Self::FlashingWhite, false) => "No airborne meaning",
            (Self::AlternatingRedGreen, _) => "Exercise extreme caution",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LightGunState {
    pub airport: String,
    pub signal: LightGunSignal,
    pub position_m: [f64; 3],
    pub position_estimated: bool,
    pub since_s: f64,
    pub indication: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AtcState {
    #[serde(default)]
    pub external_controller: bool,
    /// Closed traffic has been accepted by a completed, heard pilot readback.
    #[serde(default)]
    pub closed_traffic: bool,
    pub airport: Option<String>,
    pub current_facility: Option<ComStation>,
    pub expected_next_call: String,
    pub atis: Option<AtisReport>,
    pub reported_information: Option<String>,
    pub pending_instructions: Vec<PendingInstruction>,
    pub compliance: Vec<ComplianceGrade>,
    pub bravo_clearance: bool,
    pub ifr_clearance: bool,
    pub two_way_providers: Vec<String>,
    pub radio_failed: bool,
    pub light_gun: Option<LightGunState>,
    pub squawk: String,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Atc {
    pub vector_approach: Option<AtcInstruction>,
    pub external_controller: bool,
    pub airport: Option<String>,
    pub destination: Option<String>,
    pub reported: Option<(String, String)>,
    pub pending: Vec<PendingInstruction>,
    pub compliance: Vec<ComplianceGrade>,
    pub radio_failure_observed: bool,
    pub light_gun: Option<LightGunState>,
    pub light_override: bool,
    pub replies: VecDeque<Reply>,
    pub current: Option<ComStation>,
    pub two_way: std::collections::BTreeSet<String>,
    /// Radar facility that has actually identified this aircraft over a heard transmission.
    pub radar_facility: Option<String>,
    pub bravo: Option<String>,
    pub ifr_clearance: bool,
    pub next_feedback_s: f64,
    pub cue_queue: Vec<ComplianceGrade>,
    pub last_observed: std::collections::BTreeMap<String, bool>,
    pub active: Vec<PendingInstruction>,
    pub takeoff: bool,
    pub landing: bool,
    pub inbound: bool,
    pub closed_traffic: bool,
    pub was_airborne: bool,
    pub last_ground: Option<bool>,
    pub last_sector: Option<String>,
    pub next_check: f64,
    pub info_epochs: std::collections::BTreeMap<String, (String, u64)>,
}
#[derive(Clone, Debug)]
pub(crate) struct Reply {
    pub station: ComStation,
    pub text: String,
    pub meaning: Meaning,
}
