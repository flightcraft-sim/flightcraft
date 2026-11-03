//! Deterministic passenger pilot. Guidance emits ordinary control commands at fixed intervals.
mod instrument;
mod narration;
mod pilot;
mod planning;
mod radio;
use crate::*;
use flightcraft_world::GeoPoint;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CrewMode {
    #[default]
    Solo,
    Watching,
    Demonstrating,
    Instructing,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlOwner {
    #[default]
    Pilot,
    Crew,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PilotIntent {
    Approach {
        airport: String,
        procedure: String,
        mode: InstrumentApproach,
        minimum_ft: f64,
        #[serde(default)]
        coupled: bool,
    },
    HoldAtFix {
        airport: String,
        procedure: String,
        fix: String,
    },
    TakeOff {
        airport: String,
        runway: String,
    },
    FlyToAirport {
        airport: String,
        runway: String,
        altitude_m: f64,
    },
    FlyToCoordinate {
        position: GeoPoint,
        altitude_m: f64,
    },
    FlightPlan {},
    JoinPattern {
        airport: String,
        runway: String,
    },
    LandRunway {
        airport: String,
        runway: String,
    },
    Scenic {
        preset: ScenicPreset,
        airport: String,
        runway: String,
    },
    Hold {
        altitude_m: f64,
        heading_rad: f64,
    },
    Orbit {
        position: GeoPoint,
        altitude_m: f64,
        radius_m: f64,
    },
    GoAround {},
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScenicPreset {
    DelawareWaterGap,
    HudsonCorridor,
    PoconoPlateau,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PilotPhase {
    #[default]
    Idle,
    BeforeTakeoff,
    Takeoff,
    Climb,
    Cruise,
    Descent,
    Entry,
    Crosswind,
    Downwind,
    Base,
    Final,
    Flare,
    Rollout,
    TaxiClear,
    Parking,
    Complete,
    Hold,
    Orbit,
    GoAround,
    Refused,
}
impl PilotPhase {
    /// Passenger-facing phase label, shared by the state explanation and UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::BeforeTakeoff => "Before takeoff",
            Self::Takeoff => "Taking off",
            Self::Climb => "Climbing",
            Self::Cruise => "Cruising",
            Self::Descent => "Descending",
            Self::Entry => "Pattern entry",
            Self::Crosswind => "Crosswind",
            Self::Downwind => "Downwind",
            Self::Base => "Base",
            Self::Final => "Final approach",
            Self::Flare => "Flare",
            Self::Rollout => "Rollout",
            Self::TaxiClear => "Taxiing clear",
            Self::Parking => "Taxiing to parking",
            Self::Complete => "Stopped",
            Self::Hold => "Holding",
            Self::Orbit => "Scenic orbit",
            Self::GoAround => "Going around",
            Self::Refused => "Flight stopped",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CrewLeg {
    pub name: String,
    pub position_m: [f64; 3],
    pub position: Option<GeoPoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CrewRunway {
    pub airport: String,
    pub runway: String,
    pub threshold_m: [f64; 3],
    pub heading_grid_rad: f64,
    pub length_m: f64,
    pub width_m: f64,
    pub pattern_altitude_m: f64,
}
impl CrewRunway {
    pub(super) fn point(&self, along: f64, right: f64, height: f64) -> [f64; 3] {
        let (s, c) = libm::sincos(self.heading_grid_rad);
        let [n, e, d] = self.threshold_m;
        [n + c * along - s * right, e + s * along + c * right, d - height]
    }
    pub(super) fn relative(&self, [n, e, _]: [f64; 3]) -> (f64, f64) {
        let [rn, re, _] = self.threshold_m;
        let (s, c) = libm::sincos(self.heading_grid_rad);
        ((n - rn) * c + (e - re) * s, -(n - rn) * s + (e - re) * c)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CrewPlan {
    pub briefing: String,
    pub legs: Vec<CrewLeg>,
    pub arrival: Option<CrewRunway>,
    pub departure: Option<CrewRunway>,
    pub altitude_m: f64,
    pub checks: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Touchdown {
    pub time_s: f64,
    pub distance_m: f64,
    pub centreline_m: f64,
    pub sink_mps: f64,
    pub within_limits: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CrewState {
    pub mode: CrewMode,
    pub controls: ControlOwner,
    pub pending_handover: Option<ControlOwner>,
    pub intent: Option<PilotIntent>,
    pub plan: Option<CrewPlan>,
    pub phase: PilotPhase,
    pub leg: usize,
    pub next_step: String,
    pub reason: String,
    pub touchdown: Option<Touchdown>,
    pub taxi_clear: bool,
    pub parked: bool,
    pub director: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Crew {
    pub(crate) state: CrewState,
    proposer: Option<ControlOwner>,
    last_tick: Option<f64>,
    last_step_time: Option<f64>,
    phase_since: f64,
    pitch_trim: f64,
    throttle_trim: f64,
    last_speech: f64,
    spoken_landmarks: std::collections::BTreeSet<u32>,
    last_landmark_check: f64,
    last_landmark_s: Option<f64>,
    cruise_level_m: Option<f64>,
    radio: radio::CrewRadio,
    pattern_side: f64,
    callouts: u8,
    previous_agl: Option<f64>,
    airborne_sink_mps: f64,
    hold_heading_grid_rad: Option<f64>,
    taxi_exit: Option<(f64, [f64; 3])>,
    radio_queue: std::collections::VecDeque<String>,
    last_radio_error: Option<String>,
    instrument_calls: u8,
    hold_path: Vec<[f64; 3]>,
    hold_loop_start: usize,
}
impl Session {
    pub fn crew_state(&self) -> CrewState {
        self.crew.state.clone()
    }
    pub fn crew_owns_controls(&self) -> bool {
        self.crew.state.controls == ControlOwner::Crew
    }
    pub(crate) fn crew_say(&mut self, text: String) {
        self.crew_voice(text, false);
    }
    pub(crate) fn crew_callout(&mut self, text: String) {
        self.crew_voice(text, true);
    }
    fn crew_voice(&mut self, text: String, critical: bool) {
        self.start_local(
            Channel::Intercom,
            Speaker::Crew,
            text,
            Meaning::Crew { critical },
            EventSource::Rules,
        );
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCrewMode {
    pub mode: CrewMode,
    #[serde(default)]
    pub director: bool,
}
impl Command for SetCrewMode {
    const NAME: &'static str = "crew.mode";
    const DESCRIPTION: &'static str = "Select crew presentation; solo requires the pilot to own controls. Ownership changes through crew.handover.";
    type Output = CrewState;
    fn run(self, s: &mut Session) -> Result<CrewState, CommandError> {
        if self.mode == CrewMode::Solo && s.crew_owns_controls() {
            return Err(invalid(Self::NAME, "confirm Your controls before selecting solo"));
        }
        if self.mode != CrewMode::Solo {
            CommsCallsign {
                abbreviate_after_contact: Some(false),
                ..Default::default()
            }
            .run(s)?;
        }
        s.crew.state.mode = self.mode;
        s.crew.state.director = self.director;
        if self.mode == CrewMode::Solo {
            s.crew = Crew::default();
        }
        Ok(s.crew_state())
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrewHandover {
    pub to: ControlOwner,
    pub actor: ControlOwner,
    #[serde(default)]
    pub confirm: bool,
}
impl Command for CrewHandover {
    const NAME: &'static str = "crew.handover";
    const DESCRIPTION: &'static str =
        "Propose an exchange, then confirm as the other participant; both acknowledgements are required.";
    type Output = CrewState;
    fn run(self, s: &mut Session) -> Result<CrewState, CommandError> {
        if s.crew.state.mode == CrewMode::Solo {
            return Err(invalid(Self::NAME, "select a crew mode first"));
        }
        if self.to == s.crew.state.controls {
            return Err(invalid(Self::NAME, "that participant already has controls"));
        }
        if self.confirm {
            if s.crew.state.pending_handover != Some(self.to)
                || s.crew.proposer == Some(self.actor)
                || s.crew.proposer.is_none()
            {
                return Err(invalid(
                    Self::NAME,
                    "confirmation must match a proposal from the other participant",
                ));
            }
            if self.to == ControlOwner::Crew && s.crew.state.plan.is_none() {
                return Err(invalid(Self::NAME, "give the crew an intent first"));
            }
            if self.to == ControlOwner::Crew {
                AssistOff {}.run(s)?;
                if s.sim.installed_autopilot().is_some() {
                    SetAvionics {
                        setting: AvionicsSetting::Autopilot {
                            setting: flightcraft_sim::equipment::PanelSetting::Disconnect,
                        },
                    }
                    .run(s)?;
                }
            }
            s.crew.state.controls = self.to;
            s.crew.state.pending_handover = None;
            s.crew.proposer = None;
            s.crew.last_tick = None;
            s.crew_say(
                if self.to == ControlOwner::Crew {
                    "My controls. I'll fly the plan."
                } else {
                    "Your controls. You have the aircraft."
                }
                .into(),
            );
        } else {
            s.crew.state.pending_handover = Some(self.to);
            s.crew.proposer = Some(self.actor);
            s.crew_say(
                if self.to == ControlOwner::Crew {
                    "Your controls offered to me. Confirm when you're ready."
                } else {
                    "Your controls. Confirm when you're ready."
                }
                .into(),
            );
        }
        Ok(s.crew_state())
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CrewIntent {
    pub intent: PilotIntent,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CrewIntentOutput {
    pub accepted: bool,
    pub refusal: Option<String>,
    pub state: CrewState,
}
impl Command for CrewIntent {
    const NAME: &'static str = "crew.intent";
    const DESCRIPTION: &'static str = "Validate and brief a deterministic flight intent; returns legs or a reasoned refusal without changing the current plan.";
    type Output = CrewIntentOutput;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let planned = planning::plan(s, &self.intent);
        let (plan, phase) = match planned {
            Ok(p) => p,
            Err(e) => {
                return Ok(CrewIntentOutput {
                    accepted: false,
                    refusal: Some(e.to_string()),
                    state: s.crew_state(),
                });
            }
        };
        if let PilotIntent::Approach {
            airport,
            procedure,
            mode,
            ..
        } = &self.intent
        {
            s.prepare_instrument_receiver(airport, procedure, *mode)?;
        }
        let text = plan.briefing.clone();
        let mut runtime = Crew {
            state: CrewState {
                mode: s.crew.state.mode,
                controls: s.crew.state.controls,
                pending_handover: s.crew.state.pending_handover,
                director: s.crew.state.director,
                intent: Some(self.intent),
                plan: Some(plan),
                phase,
                next_step: phase.label().into(),
                reason: text.clone(),
                ..Default::default()
            },
            proposer: s.crew.proposer,
            phase_since: s.sim.state().time_s,
            last_speech: s.sim.state().time_s,
            ..Default::default()
        };
        runtime.pitch_trim = 0.0;
        s.crew = runtime;
        s.rate = 1.0;
        s.crew_say(text);
        Ok(CrewIntentOutput {
            accepted: true,
            refusal: None,
            state: s.crew_state(),
        })
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCrewState {}
impl Command for GetCrewState {
    const NAME: &'static str = "crew.state";
    const DESCRIPTION: &'static str = "Read crew intent, checked plan, phase, control ownership, next step, reason, and measured touchdown.";
    type Output = CrewState;
    fn run(self, s: &mut Session) -> Result<CrewState, CommandError> {
        Ok(s.crew_state())
    }
}
