//! Exterior pre-flight walkaround: a figure walks the aircraft's own inspection list, the student
//! sees each check, instructors seed findings, and unfound findings have real consequences.
//!
//! The pack's `preflight` section supplies stations, items, handbook references, positions, and
//! props. This module adds no inspection steps. See `docs/preflight.md`.
pub(crate) mod choreo;
mod lesson;
pub use lesson::*;

use crate::{Command, CommandError, Session, invalid};
use choreo::{Context, Effect, Frame, Plan, Stance0, V3, evaluate, plan_item, v3};
use flightcraft_aero::Wheel;
use flightcraft_aero::preflight::{Action, DefectKind, Preflight, PropKind, Severity};
use flightcraft_aero::walkaround::{
    CameraFrame, CapState, Cue, DipstickView, FigureAction, FigureState, FuelView, PropState,
    SurfaceOverride, WalkaroundScene,
};
use flightcraft_sim::canopy::CanopyAction;
use flightcraft_sim::hazards::GroundHazards;
use flightcraft_sim::systems::{Magnetos, SystemSetting, SystemSwitch};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// Real seconds per internal animation substep.
const SUBSTEP_S: f64 = 1. / 30.;
/// Fuel left in a tank seeded as low, as a fraction of the level that just covers its bottom.
const LOW_FUEL_FRACTION: f64 = 0.4;
/// Dipstick level, 0 at the MIN mark and 1 at the MAX mark.
const OIL_NORMAL: f32 = 0.65;
const OIL_LOW: f32 = -0.35;
const CAMERA_FOV_DEG: f64 = 42.;
const CAMERA_TAU_S: f64 = 0.55;

#[derive(Clone, Debug)]
struct ItemRt {
    done: bool,
    performed: u32,
    seeded: Option<DefectKind>,
    finding: Option<Finding>,
    resolved: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Finding {
    pub kind: DefectKind,
    pub description: String,
    pub severity: Severity,
}

#[derive(Clone, Debug)]
struct Running {
    item: String,
    plan: Plan,
    from: Stance0,
    t: f32,
    applied: usize,
}

#[derive(Clone, Debug)]
struct CameraRt {
    target: V3,
    eye: V3,
    station: String,
}

#[derive(Clone, Debug)]
struct Walk {
    def: Preflight,
    generation: u64,
    lesson: Option<String>,
    seed: u64,
    speed: f64,
    finished: bool,
    planned_fuel_l: Option<f64>,
    items: BTreeMap<String, ItemRt>,
    props: BTreeMap<String, bool>,
    stance: Stance0,
    queue: VecDeque<String>,
    running: Option<Running>,
    camera: CameraRt,
    oil_level: f32,
    saved_tank_kg: BTreeMap<String, f64>,
    frame: Frame,
    last_item: Option<String>,
    departed: bool,
    departure_no_go: Vec<String>,
    messages: Vec<String>,
    accumulator: f64,
    ground_z: f32,
    open_cap: Option<String>,
    surfaces: Vec<(String, f32)>,
    previous_camera: Option<crate::CameraMode>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Runtime {
    walk: Option<Walk>,
}

/// Deterministic mixer, so a seed always picks the same findings.
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Pick `count` findings on distinct items, in a fixed order for a given seed.
#[must_use]
pub fn select_findings(def: &Preflight, seed: u64, count: usize) -> Vec<SeedSpec> {
    let mut pool: Vec<(String, DefectKind)> = def
        .items
        .iter()
        .flat_map(|i| i.defects.iter().map(|d| (i.id.clone(), d.kind)))
        .collect();
    let mut state = seed ^ 0x5052_4546_4c49_4748;
    let mut out: Vec<SeedSpec> = Vec::new();
    while out.len() < count && !pool.is_empty() {
        let index = (splitmix(&mut state) % pool.len() as u64) as usize;
        let (item, kind) = pool.swap_remove(index);
        pool.retain(|(i, _)| *i != item);
        out.push(SeedSpec { item, kind });
    }
    out
}

fn prop_description(kind: PropKind) -> &'static str {
    match kind {
        PropKind::PitotCover => "The pitot cover was still fitted over the pitot tube.",
        PropKind::Chock => "Wheel chocks were still in place.",
        PropKind::TieDownRope => "A tie-down rope was still attached.",
    }
}

impl Walk {
    fn defect_option(
        &self,
        item: &str,
        kind: DefectKind,
    ) -> Option<&flightcraft_aero::preflight::DefectOption> {
        self.def.item(item)?.defects.iter().find(|d| d.kind == kind)
    }

    fn station_of(&self, item: &str) -> String {
        self.def
            .item(item)
            .map(|i| i.station.clone())
            .or_else(|| self.def.stations.first().map(|s| s.id.clone()))
            .unwrap_or_default()
    }

    fn current_station(&self) -> String {
        match (&self.running, &self.last_item) {
            (Some(r), _) => self.station_of(&r.item),
            (None, Some(i)) => self.station_of(i),
            _ => self
                .def
                .stations
                .first()
                .map(|s| s.id.clone())
                .unwrap_or_default(),
        }
    }

    /// Hazards the walkaround currently leaves on the aircraft.
    fn hazards(&self) -> GroundHazards {
        let present = |kind: PropKind| {
            self.def
                .props
                .iter()
                .any(|p| p.kind == kind && self.props.get(&p.id).copied().unwrap_or(false))
        };
        let defect = |kind: DefectKind| self.items.values().any(|i| i.seeded == Some(kind) && !i.resolved);
        GroundHazards {
            pitot_cover: present(PropKind::PitotCover),
            chocks: present(PropKind::Chock),
            tie_downs: present(PropKind::TieDownRope),
            flap_hardware: defect(DefectKind::FlapHardwareMissing),
            aileron_hardware: defect(DefectKind::AileronHardwareMissing),
            low_oil: defect(DefectKind::LowOil),
            loose_cowl: defect(DefectKind::LooseCowlFastener),
            damaged_tyre: self
                .open_defect_cue(DefectKind::DamagedTyre)
                .map(|cue| Wheel::from_lateral_m(cue[1])),
            fairing_crack: defect(DefectKind::WheelFairingCrack),
        }
    }

    /// Body-frame cue position of the first unresolved seeded finding of this kind.
    fn open_defect_cue(&self, kind: DefectKind) -> Option<[f64; 3]> {
        self.def.items.iter().find_map(|item| {
            let rt = self.items.get(&item.id)?;
            (rt.seeded == Some(kind) && !rt.resolved)
                .then(|| self.defect_option(&item.id, kind).map(|d| d.cue_m))
                .flatten()
        })
    }

    /// Findings that make the aircraft unfit to fly right now, found or not.
    fn open_no_go(&self) -> Vec<(String, DefectKind)> {
        let mut out = Vec::new();
        for item in &self.def.items {
            let rt = self.items.get(&item.id);
            if let Some(rt) = rt
                && let Some(kind) = rt.seeded
                && !kind.is_prop()
                && !rt.resolved
                && self
                    .defect_option(&item.id, kind)
                    .is_some_and(|d| d.severity == Severity::NoGo)
            {
                out.push((item.id.clone(), kind));
            }
        }
        for prop in &self.def.props {
            if self.props.get(&prop.id).copied().unwrap_or(false) {
                out.push((prop.id.clone(), prop.kind.defect()));
            }
        }
        out
    }
}

fn tank_litres(session: &Session, tank: &str, density: f64) -> Option<(f64, f64)> {
    let t = session.sim.systems().tanks.iter().find(|t| t.id == tank)?;
    Some((t.quantity_kg / density, t.capacity_kg / density))
}

impl Session {
    fn walk_ref(&self, command: &str) -> Result<&Walk, CommandError> {
        self.preflight
            .walk
            .as_ref()
            .ok_or_else(|| invalid(command, "no pre-flight walkaround is active; run preflight.begin"))
    }

    /// Return the main view to the mode it had before the walkaround took it.
    fn leave_walkaround_camera(&mut self, previous: Option<crate::CameraMode>) {
        if let (Some(c), Some(mode)) = (self.camera.as_mut(), previous)
            && c.mode == crate::CameraMode::Walkaround
        {
            c.mode = mode;
        }
    }

    fn sync_preflight_hazards(&mut self) {
        if let Some(walk) = &self.preflight.walk {
            let hazards = walk.hazards();
            self.sim.set_hazards(hazards);
        }
    }

    /// Drawing data for the exterior: figure, props, hand-moved surfaces, close-ups, and cues.
    #[must_use]
    pub fn preflight_scene(&self) -> Option<WalkaroundScene> {
        let walk = self.preflight.walk.as_ref().filter(|w| !w.finished)?;
        Some(self.build_scene(walk))
    }

    fn build_scene(&self, walk: &Walk) -> WalkaroundScene {
        let density = walk.def.fuel.density_kg_per_l;
        let props = walk
            .def
            .props
            .iter()
            .filter(|p| walk.props.get(&p.id).copied().unwrap_or(false))
            .map(|p| {
                let carried = walk.frame.taking.as_ref().filter(|(id, _)| *id == p.id);
                PropState {
                    id: p.id.clone(),
                    kind: p.kind,
                    position_m: carried.map_or(v3(p.position_m), |(_, at)| *at),
                    yaw_rad: p.yaw_rad as f32,
                }
            })
            .collect();
        let fuel_view = walk.frame.fuel_view.as_ref().and_then(|tank| {
            let cap = walk.def.fuel.caps.iter().find(|c| &c.tank == tank)?;
            let (litres, capacity) = tank_litres(self, tank, density)?;
            Some(FuelView {
                tank: tank.clone(),
                cap_m: v3(cap.cap_m),
                litres: litres as f32,
                capacity_l: capacity as f32,
                covers_bottom_l: walk.def.fuel.covers_bottom_l as f32,
                covers_bottom: litres >= walk.def.fuel.covers_bottom_l,
            })
        });
        let dipstick = walk.frame.dipstick.then(|| DipstickView {
            position_m: walk
                .frame
                .figure
                .right_hand_m
                .or(walk.frame.item_target)
                .unwrap_or_else(|| v3([0.0, 0.2, -0.2])),
            level: walk.oil_level,
        });
        let cues = walk
            .items
            .iter()
            .filter_map(|(id, rt)| {
                let finding = rt.finding.as_ref().filter(|_| !rt.resolved)?;
                if finding.kind.is_prop() {
                    return None;
                }
                let option = walk.defect_option(id, finding.kind)?;
                Some(Cue {
                    kind: finding.kind,
                    position_m: v3(option.cue_m),
                })
            })
            .collect();
        let mut surfaces: Vec<SurfaceOverride> = Vec::new();
        for (node, value) in &walk.surfaces {
            surfaces.push(SurfaceOverride {
                node: node.clone(),
                value: *value,
            });
        }
        WalkaroundScene {
            figure: walk.frame.figure.clone(),
            props,
            surfaces,
            propeller_phase: walk.frame.propeller_phase,
            caps: walk
                .def
                .fuel
                .caps
                .iter()
                .map(|c| CapState {
                    tank: c.tank.clone(),
                    position_m: v3(c.cap_m),
                    open: walk.open_cap.as_deref() == Some(c.tank.as_str()),
                })
                .collect(),
            fuel_view,
            dipstick,
            cues,
            ground_z_m: walk.ground_z,
            camera: Some(CameraFrame {
                target_m: walk.camera.target,
                eye_m: walk.camera.eye,
                fov_deg: CAMERA_FOV_DEG as f32,
            }),
        }
    }

    /// Advance the walkaround by `dt` real seconds (scaled by its speed setting).
    pub(crate) fn advance_preflight(&mut self, dt: f64) {
        let Some(mut walk) = self.preflight.walk.take() else {
            return;
        };
        if walk.generation != self.flight_generation {
            self.leave_walkaround_camera(walk.previous_camera);
            return;
        }
        // Departure tracking continues after the walkaround is finished.
        let r = self.sim.readout();
        if !walk.departed && (r.height_agl_m > 1. || r.ground_speed_mps > 3.) {
            walk.departed = true;
            walk.departure_no_go = walk
                .open_no_go()
                .into_iter()
                .map(|(id, kind)| format!("{id}: {kind:?}"))
                .collect();
        }
        if !walk.finished {
            walk.accumulator += dt;
            while walk.accumulator >= SUBSTEP_S {
                walk.accumulator -= SUBSTEP_S;
                let scaled = SUBSTEP_S * walk.speed;
                self.substep_preflight(&mut walk, scaled, SUBSTEP_S);
            }
        }
        self.preflight.walk = Some(walk);
        self.sync_preflight_hazards();
    }

    fn substep_preflight(&mut self, walk: &mut Walk, dt: f64, real_dt: f64) {
        while walk.running.is_none()
            && let Some(next) = walk.queue.pop_front()
        {
            if matches!(
                walk.def.item(&next).map(|i| &i.action),
                Some(Action::PalmProp { .. })
            ) && self.propeller_unsafe()
            {
                walk.messages.push(
                    "The propeller was not touched: ignition must be off and the engine stopped first."
                        .into(),
                );
                continue;
            }
            let from = walk.stance.clone();
            let plan = plan_item(&walk.def, &next, &from, walk.def.pace_mps as f32);
            walk.running = Some(Running {
                item: next,
                plan,
                from,
                t: 0.,
                applied: 0,
            });
        }
        let mut effects: Vec<(String, Effect)> = Vec::new();
        let mut finished_item = None;
        let frame = if let Some(run) = walk.running.as_mut() {
            let before = run.t;
            run.t += dt as f32;
            for (time, effect) in run.plan.effects.iter().skip(run.applied) {
                if *time <= run.t {
                    effects.push((run.item.clone(), effect.clone()));
                    run.applied += 1;
                } else {
                    break;
                }
            }
            let _ = before;
            let ctx = Context {
                def: &walk.def,
                ground_z: walk.ground_z,
            };
            let frame = evaluate(&ctx, &run.plan, &run.from, run.t.min(run.plan.total));
            if run.t >= run.plan.total {
                finished_item = Some(run.item.clone());
            }
            frame
        } else {
            let ctx = Context {
                def: &walk.def,
                ground_z: walk.ground_z,
            };
            evaluate(&ctx, &Plan::default(), &walk.stance, 0.)
        };
        for (item, effect) in effects {
            self.apply_effect(walk, &item, &effect);
        }
        walk.surfaces = frame.surfaces.clone();
        walk.open_cap = frame.open_cap.clone();
        if let Some(end) = frame.end.clone().filter(|_| finished_item.is_some()) {
            walk.stance = end;
        }
        if let Some(item) = finished_item {
            if let Some(rt) = walk.items.get_mut(&item) {
                rt.done = true;
                rt.performed += 1;
            }
            walk.last_item = Some(item);
            walk.running = None;
            walk.surfaces.clear();
            walk.open_cap = None;
        }
        walk.frame = frame;
        self.follow_camera(walk, real_dt);
    }

    fn follow_camera(&self, walk: &mut Walk, dt: f64) {
        let station = walk.current_station();
        let Some(st) = walk.def.station(&station) else {
            return;
        };
        let figure = &walk.frame.figure;
        let chest = [
            figure.position_m[0],
            figure.position_m[1],
            figure.position_m[2] - if figure.seated { 0.6 } else { 1.0 },
        ];
        let focus = walk
            .frame
            .item_target
            .map_or(chest, |t| choreo::lerp(chest, t, 0.5));
        // Wheels, brakes and the wing underside are best seen low, from behind the wing.
        let low = walk.running.as_ref().is_some_and(|r| {
            matches!(
                walk.def.item(&r.item).map(|i| &i.action),
                Some(Action::Inspect {
                    stance: flightcraft_aero::preflight::Stance::Crouch,
                    ..
                })
            )
        });
        let o = st.camera.eye_offset_m.map(|v| v as f32);
        let offset = if low {
            [-o[0].abs() * 0.8, o[1] * 0.9, o[2] * 0.5]
        } else {
            o
        };
        let eye = [focus[0] + offset[0], focus[1] + offset[1], focus[2] + offset[2]];
        let k = (1. - (-dt / CAMERA_TAU_S).exp()) as f32;
        if walk.camera.station.is_empty() {
            walk.camera.target = focus;
            walk.camera.eye = eye;
        } else {
            walk.camera.target = choreo::lerp(walk.camera.target, focus, k);
            walk.camera.eye = choreo::lerp(walk.camera.eye, eye, k);
        }
        walk.camera.station = station;
    }

    fn apply_effect(&mut self, walk: &mut Walk, item: &str, effect: &Effect) {
        match effect {
            Effect::Reveal => self.reveal(walk, item),
            Effect::RemoveProp(id) => {
                walk.props.insert(id.clone(), false);
            }
            Effect::OpenCanopy => {
                let canopy = self.sim.canopy();
                if canopy.sliding && canopy.open_fraction < 1. {
                    let result = self
                        .sim
                        .set_canopy(CanopyAction::Unlatch, None)
                        .and_then(|_| self.sim.set_canopy(CanopyAction::Open, None));
                    if let Err(e) = result {
                        walk.messages.push(format!("Canopy: {e}"));
                    }
                }
            }
            Effect::Cabin(effect) => self.cabin_effect(walk, *effect),
            Effect::Inside(_) => {}
        }
        walk.messages.truncate(16);
    }

    fn cabin_effect(&mut self, walk: &mut Walk, effect: flightcraft_aero::preflight::CabinEffect) {
        use flightcraft_aero::preflight::CabinEffect as C;
        let result = match effect {
            C::None | C::FuelIndicator => Ok(()),
            C::MasterOn => self.sim.set_systems(SystemSetting::Switch {
                switch: SystemSwitch::Master,
                on: true,
            }),
            C::MasterOff => self.sim.set_systems(SystemSetting::Switch {
                switch: SystemSwitch::Master,
                on: false,
            }),
            C::IgnitionOff => self.sim.set_systems(SystemSetting::Magnetos {
                position: Magnetos::Off,
            }),
            C::FlapsFull => {
                let mut controls = self.sim.controls();
                controls.flaps = 1.;
                self.sim.set_controls(controls);
                Ok(())
            }
        };
        if let Err(e) = result {
            walk.messages.push(format!("Cabin: {e}"));
        }
    }

    fn reveal(&mut self, walk: &mut Walk, item_id: &str) {
        let Some(item) = walk.def.item(item_id).cloned() else {
            return;
        };
        let Some(seeded) = walk.items.get(item_id).and_then(|rt| rt.seeded) else {
            return;
        };
        let finding = if seeded.is_prop() {
            walk.def
                .props_of(&item)
                .into_iter()
                .find(|p| p.kind.defect() == seeded)
                .map(|p| Finding {
                    kind: seeded,
                    description: prop_description(p.kind).into(),
                    severity: Severity::NoGo,
                })
        } else {
            walk.defect_option(item_id, seeded).map(|d| Finding {
                kind: seeded,
                description: d.description.clone(),
                severity: d.severity,
            })
        };
        if let Some(rt) = walk.items.get_mut(item_id) {
            rt.finding = finding;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Output types

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StationView {
    pub id: String,
    pub name: String,
    pub poh_ref: String,
    /// Position in the walk order, from 1.
    pub order: u32,
    pub items_total: u32,
    pub items_done: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ItemView {
    pub id: String,
    pub station: String,
    pub text: String,
    pub poh_ref: String,
    pub done: bool,
    pub optional: bool,
    /// A handbook warning or caution that applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// None until the item reveals something.
    pub finding: Option<Finding>,
    /// Instructor view only: the kind seeded on this item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault_seeded: Option<DefectKind>,
}

/// Framing for the exterior inset.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CameraView {
    pub station: String,
    /// Body-frame point to look at (x forward, y right, z down), metres.
    pub target_m: [f64; 3],
    /// Suggested eye offset from the target: a close, slightly elevated view.
    pub eye_offset_m: [f64; 3],
    /// Smoothed eye position, `target_m + eye_offset_m` once settled.
    pub eye_m: [f64; 3],
    pub up: [f64; 3],
    pub fov_deg: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FuelStatus {
    pub tank: String,
    pub litres: f64,
    pub capacity_l: f64,
    /// Fuel visibly covers the whole bottom of the tank (30 litres or more in the Sling).
    pub covers_bottom: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SeededView {
    pub item: String,
    pub kind: DefectKind,
    pub found: bool,
    pub severity: Severity,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PreflightGrade {
    pub required_total: u32,
    pub required_done: u32,
    pub items_missed: Vec<String>,
    pub seeded_total: u32,
    pub seeded_found: u32,
    /// Instructor view only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeded: Option<Vec<SeededView>>,
    /// Covers, ropes, and chocks still fitted.
    pub props_left: Vec<String>,
    /// No-go findings still open (found or not) and covers/ropes/chocks still fitted.
    pub no_go_open: u32,
    pub departed: bool,
    /// The aircraft left the ground or moved under power while a no-go was open.
    pub flew_with_no_go: bool,
    pub passed: bool,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PreflightState {
    pub active: bool,
    pub finished: bool,
    pub lesson: Option<String>,
    /// Seed for random findings.
    pub seed: u64,
    pub speed: f64,
    pub stations: Vec<StationView>,
    pub items: Vec<ItemView>,
    pub current_station: Option<String>,
    pub current_item: Option<String>,
    pub queue: Vec<String>,
    pub camera: Option<CameraView>,
    pub figure: FigureState,
    pub action: FigureAction,
    pub hazards: GroundHazards,
    /// Set while a restraint stops the aircraft: why it will not move.
    pub blocked: Option<String>,
    pub fuel: Vec<FuelStatus>,
    pub planned_fuel_l: Option<f64>,
    /// 0 at the dipstick MIN mark and 1 at MAX, once an oil check has been performed.
    pub oil_level: Option<f32>,
    pub messages: Vec<String>,
    pub grade: PreflightGrade,
    pub scene: WalkaroundScene,
}

fn empty_state() -> PreflightState {
    PreflightState {
        active: false,
        finished: false,
        lesson: None,
        seed: 0,
        speed: 1.,
        stations: Vec::new(),
        items: Vec::new(),
        current_station: None,
        current_item: None,
        queue: Vec::new(),
        camera: None,
        figure: FigureState::default(),
        action: FigureAction::Idle,
        hazards: GroundHazards::default(),
        blocked: None,
        fuel: Vec::new(),
        planned_fuel_l: None,
        oil_level: None,
        messages: Vec::new(),
        grade: PreflightGrade {
            required_total: 0,
            required_done: 0,
            items_missed: Vec::new(),
            seeded_total: 0,
            seeded_found: 0,
            seeded: None,
            props_left: Vec::new(),
            no_go_open: 0,
            departed: false,
            flew_with_no_go: false,
            passed: false,
            notes: Vec::new(),
        },
        scene: WalkaroundScene::default(),
    }
}

impl Session {
    fn grade_preflight(&self, walk: &Walk, instructor: bool) -> PreflightGrade {
        let mut required_total = 0;
        let mut required_done = 0;
        let mut missed = Vec::new();
        for item in walk.def.items.iter().filter(|i| !i.optional) {
            required_total += 1;
            if walk.items.get(&item.id).is_some_and(|rt| rt.done) {
                required_done += 1;
            } else {
                missed.push(item.id.clone());
            }
        }
        let mut seeded = Vec::new();
        for item in &walk.def.items {
            let Some(rt) = walk.items.get(&item.id) else {
                continue;
            };
            let Some(kind) = rt.seeded else { continue };
            let severity = if kind.is_prop() {
                Severity::NoGo
            } else {
                walk.defect_option(&item.id, kind)
                    .map_or(Severity::Caution, |d| d.severity)
            };
            seeded.push(SeededView {
                item: item.id.clone(),
                kind,
                found: rt.finding.is_some(),
                severity,
            });
        }
        let props_left: Vec<String> = walk
            .def
            .props
            .iter()
            .filter(|p| walk.props.get(&p.id).copied().unwrap_or(false))
            .map(|p| p.id.clone())
            .collect();
        let no_go_open = walk.open_no_go().len() as u32;
        let flew = walk.departed && !walk.departure_no_go.is_empty();
        let seeded_found = seeded.iter().filter(|s| s.found).count() as u32;
        let mut notes = Vec::new();
        if !missed.is_empty() {
            notes.push(format!("{} required item(s) not performed.", missed.len()));
        }
        if seeded_found < seeded.len() as u32 {
            notes.push("A seeded finding was not found.".into());
        }
        if !props_left.is_empty() {
            notes.push("A cover, rope, or chock was left on the aircraft.".into());
        }
        if flew {
            notes.push("Flew with a no-go finding open.".into());
        }
        PreflightGrade {
            required_total,
            required_done,
            passed: missed.is_empty()
                && seeded_found == seeded.len() as u32
                && props_left.is_empty()
                && !flew,
            items_missed: missed,
            seeded_total: seeded.len() as u32,
            seeded_found,
            seeded: instructor.then_some(seeded),
            props_left,
            no_go_open,
            departed: walk.departed,
            flew_with_no_go: flew,
            notes,
        }
    }

    /// Everything about the walkaround for display and scripts.
    #[must_use]
    pub fn preflight_state(&self, instructor: bool) -> PreflightState {
        let Some(walk) = self.preflight.walk.as_ref() else {
            return empty_state();
        };
        let density = walk.def.fuel.density_kg_per_l;
        let stations = walk
            .def
            .stations
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let items: Vec<_> = walk.def.items.iter().filter(|it| it.station == s.id).collect();
                StationView {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    poh_ref: s.poh_ref.clone(),
                    order: i as u32 + 1,
                    items_total: items.len() as u32,
                    items_done: items
                        .iter()
                        .filter(|it| walk.items.get(&it.id).is_some_and(|rt| rt.done))
                        .count() as u32,
                }
            })
            .collect();
        let items = walk
            .def
            .items
            .iter()
            .map(|i| {
                let rt = walk.items.get(&i.id);
                ItemView {
                    id: i.id.clone(),
                    station: i.station.clone(),
                    text: i.text.clone(),
                    poh_ref: i.poh_ref.clone(),
                    done: rt.is_some_and(|r| r.done),
                    optional: i.optional,
                    warning: i.warning.clone(),
                    finding: rt
                        .and_then(|r| r.finding.clone())
                        .filter(|_| rt.is_some_and(|r| !r.resolved)),
                    fault_seeded: instructor.then(|| rt.and_then(|r| r.seeded)).flatten(),
                }
            })
            .collect();
        let station = walk.current_station();
        let camera = walk.def.station(&station).map(|st| {
            let t = walk.camera.target;
            let e = walk.camera.eye;
            CameraView {
                station: station.clone(),
                target_m: t.map(f64::from),
                eye_offset_m: st.camera.eye_offset_m,
                eye_m: e.map(f64::from),
                up: [0., 0., -1.],
                fov_deg: CAMERA_FOV_DEG,
            }
        });
        let fuel = walk
            .def
            .fuel
            .caps
            .iter()
            .filter_map(|c| {
                let (litres, capacity_l) = tank_litres(self, &c.tank, density)?;
                Some(FuelStatus {
                    tank: c.tank.clone(),
                    litres,
                    capacity_l,
                    covers_bottom: litres >= walk.def.fuel.covers_bottom_l,
                })
            })
            .collect();
        let hazards = self.sim.hazards();
        let r = self.sim.readout();
        let blocked = (hazards.restrained() && r.controls.throttle > 0.05).then(|| {
            let what = match (hazards.chocks, hazards.tie_downs) {
                (true, true) => "Chocks and tie-down ropes are",
                (true, false) => "Chocks are",
                _ => "A tie-down rope is",
            };
            format!("{what} still in place: the aircraft will not move. Remove them first (pre-flight, nose section).")
        });
        let performed_oil = walk.items.iter().any(|(id, rt)| {
            rt.done
                && matches!(
                    walk.def.item(id).map(|i| &i.action),
                    Some(Action::OilCheck { .. })
                )
        });
        PreflightState {
            active: !walk.finished,
            finished: walk.finished,
            lesson: walk.lesson.clone(),
            seed: walk.seed,
            speed: walk.speed,
            stations,
            items,
            current_station: Some(station),
            current_item: walk.running.as_ref().map(|r| r.item.clone()),
            queue: walk.queue.iter().cloned().collect(),
            camera,
            figure: walk.frame.figure.clone(),
            action: walk.frame.figure.action,
            hazards,
            blocked,
            fuel,
            planned_fuel_l: walk.planned_fuel_l,
            oil_level: performed_oil.then_some(walk.oil_level),
            messages: walk.messages.clone(),
            grade: self.grade_preflight(walk, instructor),
            scene: self.build_scene(walk),
        }
    }

    /// Advance the walkaround on wall-clock frame time while the flight is paused.
    ///
    /// A preflight is a ground activity, so the figure keeps walking when the simulation is
    /// stopped. While the flight runs, the fixed steps feed the animation instead, so this does
    /// nothing then. The flight state is untouched; only the walkaround's own effects apply.
    /// `elapsed_s` is clamped so a stalled frame does not skip the animation.
    pub fn advance_walkaround_paused(&mut self, elapsed_s: f64) {
        if self.paused() && self.preflight.walk.is_some() && elapsed_s.is_finite() && elapsed_s > 0. {
            self.advance_preflight(elapsed_s.min(0.25));
        }
    }

    pub(crate) fn sequence_preflight(&mut self) {
        if self.preflight.walk.is_some() {
            self.advance_preflight(1. / 120.);
        }
    }

    fn preflight_definition(&self, command: &str) -> Result<(Preflight, f32, String), CommandError> {
        let configuration = self
            .active_configuration
            .as_ref()
            .ok_or_else(|| invalid(command, "this aircraft has no resolved configuration"))?;
        let def = configuration
            .installed_systems
            .preflight
            .clone()
            .map(|p| *p)
            .ok_or_else(|| invalid(command, "the active aircraft pack has no preflight section"))?;
        let ground_z = self
            .sim
            .aircraft()
            .geometry
            .gear
            .iter()
            .map(|g| g.point_m[2])
            .fold(f64::MIN, f64::max) as f32;
        Ok((def, ground_z, configuration.aircraft.identity.id.clone()))
    }

    /// Begin a walkaround on the active aircraft with explicit data. Tests and tools use this
    /// directly; `preflight.begin` takes the data from the active aircraft pack.
    pub fn begin_preflight_with(
        &mut self,
        def: Preflight,
        ground_z: f32,
        options: BeginOptions,
    ) -> Result<(), CommandError> {
        def.validate()
            .map_err(|e| invalid("preflight.begin", e.to_string()))?;
        let Some(first) = def.stations.first() else {
            return Err(invalid("preflight.begin", "the preflight has no stations"));
        };
        let node = def
            .node(&first.node)
            .ok_or_else(|| invalid("preflight.begin", "unknown start node"))?;
        let pos = [node.position_m[0] as f32, node.position_m[1] as f32];
        let heading = (-(pos[1])).atan2(-(pos[0]) + 1.5);
        let speed = options.speed.unwrap_or(1.);
        crate::range("preflight.begin", "speed", speed, 0.25, 16.)?;
        let items = def
            .items
            .iter()
            .map(|i| {
                (
                    i.id.clone(),
                    ItemRt {
                        done: false,
                        performed: 0,
                        seeded: None,
                        finding: None,
                        resolved: false,
                    },
                )
            })
            .collect();
        let covers_on = options.covers_on.unwrap_or(true);
        let props = def
            .props
            .iter()
            .map(|p| (p.id.clone(), p.present_at_start && covers_on))
            .collect();
        let station = first.id.clone();
        let stance = Stance0 {
            node: first.node.clone(),
            pos,
            heading,
            inside: false,
            phase: 0.,
        };
        let walk = Walk {
            def,
            generation: self.flight_generation,
            lesson: options.lesson,
            seed: options.seed,
            speed,
            finished: false,
            planned_fuel_l: options.planned_fuel_l,
            items,
            props,
            stance,
            queue: VecDeque::new(),
            running: None,
            camera: CameraRt {
                target: [0.; 3],
                eye: [0.; 3],
                station: String::new(),
            },
            oil_level: OIL_NORMAL,
            saved_tank_kg: BTreeMap::new(),
            frame: Frame::default(),
            last_item: None,
            departed: false,
            departure_no_go: Vec::new(),
            messages: Vec::new(),
            accumulator: 0.,
            ground_z,
            open_cap: None,
            surfaces: Vec::new(),
            previous_camera: None,
        };
        let _ = station;
        let mut walk = walk;
        if let Some(c) = self.camera.as_mut() {
            walk.previous_camera = Some(c.mode);
            c.mode = crate::CameraMode::Walkaround;
        }
        self.preflight.walk = Some(walk);
        // Show the figure standing at its first station before anything is performed.
        if let Some(mut walk) = self.preflight.walk.take() {
            self.substep_preflight(&mut walk, 0., 0.);
            self.preflight.walk = Some(walk);
        }
        self.sync_preflight_hazards();
        for spec in options.findings {
            self.seed_preflight(&spec.item, spec.kind)?;
        }
        Ok(())
    }

    fn seed_preflight(&mut self, item_id: &str, kind: DefectKind) -> Result<(), CommandError> {
        let command = "preflight.fault";
        let (density, covers) = {
            let walk = self.walk_ref(command)?;
            (walk.def.fuel.density_kg_per_l, walk.def.fuel.covers_bottom_l)
        };
        let walk = self.walk_ref(command)?;
        let item = walk
            .def
            .item(item_id)
            .ok_or_else(|| invalid(command, format!("unknown item: {item_id}")))?
            .clone();
        if kind.is_prop() {
            let matching: Vec<_> = walk
                .def
                .props_of(&item)
                .into_iter()
                .filter(|p| p.kind.defect() == kind)
                .map(|p| p.id.clone())
                .collect();
            if matching.is_empty() {
                return Err(invalid(command, format!("{item_id} does not remove a {kind:?}")));
            }
            if let Some(walk) = self.preflight.walk.as_mut() {
                for id in matching {
                    walk.props.insert(id, true);
                }
            }
        } else if walk.defect_option(item_id, kind).is_none() {
            return Err(invalid(
                command,
                format!("{kind:?} cannot be seeded on {item_id}"),
            ));
        }
        self.clear_seed(item_id);
        let mut tank_set = None;
        if let Some(walk) = self.preflight.walk.as_mut() {
            if let Some(rt) = walk.items.get_mut(item_id) {
                rt.seeded = Some(kind);
                rt.finding = None;
                rt.resolved = false;
            }
            match kind {
                DefectKind::LowOil => walk.oil_level = OIL_LOW,
                DefectKind::LowFuel => {
                    if let Action::FuelCap { tank } = &item.action {
                        tank_set = Some(tank.clone());
                    }
                }
                _ => {}
            }
        }
        if let Some(tank) = tank_set {
            let current = self
                .sim
                .systems()
                .tanks
                .iter()
                .find(|t| t.id == tank)
                .map(|t| t.quantity_kg);
            if let Some(kg) = current {
                if let Some(walk) = self.preflight.walk.as_mut() {
                    walk.saved_tank_kg.entry(tank.clone()).or_insert(kg);
                }
                let low = covers * LOW_FUEL_FRACTION * density;
                self.sim
                    .set_systems(SystemSetting::TankQuantity {
                        tank,
                        mass_kg: low.min(kg),
                    })
                    .map_err(CommandError::Sim)?;
            }
        }
        self.sync_preflight_hazards();
        Ok(())
    }

    /// Undo the effect of whatever is seeded on an item, before reseeding or resolving.
    fn clear_seed(&mut self, item_id: &str) {
        let Some(walk) = self.preflight.walk.as_mut() else {
            return;
        };
        let Some(kind) = walk.items.get(item_id).and_then(|rt| rt.seeded) else {
            return;
        };
        let mut restore = None;
        match kind {
            DefectKind::LowOil => walk.oil_level = OIL_NORMAL,
            DefectKind::LowFuel => {
                if let Some(Action::FuelCap { tank }) = walk.def.item(item_id).map(|i| &i.action)
                    && let Some(kg) = walk.saved_tank_kg.remove(tank)
                {
                    restore = Some((tank.clone(), kg));
                }
            }
            _ => {}
        }
        if let Some(rt) = walk.items.get_mut(item_id) {
            rt.seeded = None;
        }
        if let Some((tank, mass_kg)) = restore {
            let _ = self
                .sim
                .set_systems(SystemSetting::TankQuantity { tank, mass_kg });
        }
    }

    fn propeller_unsafe(&self) -> bool {
        let s = self.sim.systems();
        s.engine_running || s.controls.magnetos != Magnetos::Off
    }

    /// Handling the propeller needs ignition off and the engine stopped. An earlier queued
    /// ignition-off item counts, and is checked again when the item actually starts.
    fn check_propeller_safe(&self, item: &flightcraft_aero::preflight::Item) -> Result<(), CommandError> {
        if !matches!(item.action, Action::PalmProp { .. }) || !self.propeller_unsafe() {
            return Ok(());
        }
        let queued_off = self.preflight.walk.as_ref().is_some_and(|w| {
            w.queue.iter().any(|q| {
                matches!(
                    w.def.item(q).map(|i| &i.action),
                    Some(Action::Cabin {
                        effect: flightcraft_aero::preflight::CabinEffect::IgnitionOff,
                        ..
                    })
                )
            })
        });
        if queued_off {
            return Ok(());
        }
        Err(invalid(
            "preflight.perform",
            "ignition must be off and the engine stopped before handling the propeller: do the cabin ignition item first",
        ))
    }
}

/// Options for starting a walkaround.
#[derive(Clone, Debug, Default)]
pub struct BeginOptions {
    pub lesson: Option<String>,
    pub seed: u64,
    pub findings: Vec<SeedSpec>,
    pub covers_on: Option<bool>,
    pub planned_fuel_l: Option<f64>,
    pub speed: Option<f64>,
}

// ---------------------------------------------------------------------------------------------
// Commands

/// A finding to seed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeedSpec {
    pub item: String,
    pub kind: DefectKind,
}

/// Start a walkaround of the active aircraft, or of a lesson's aircraft parked at its airport.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightBegin {
    /// A pre-flight lesson id from `preflight.lessons`. Replaces the flight with the lesson's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lesson: Option<String>,
    /// Seed for random findings; the same seed always picks the same findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Number of findings to seed at random from the pack's seedable defects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 8))]
    pub random_findings: Option<u8>,
    /// Findings to seed explicitly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<SeedSpec>,
    /// Covers, ropes, and chocks fitted at the start. Default true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covers_on: Option<bool>,
    /// Fuel the planned flight needs, litres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 500))]
    pub planned_fuel_l: Option<f64>,
    /// Animation speed-up, 0.25 to 16. Default 1 (real pace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.25, max = 16))]
    pub speed: Option<f64>,
}

impl Command for PreflightBegin {
    const NAME: &'static str = "preflight.begin";
    const DESCRIPTION: &'static str =
        "Start the exterior pre-flight walkaround, optionally from a lesson and with seeded findings.";
    type Output = PreflightState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let mut findings = self.findings.clone();
        let mut options = BeginOptions {
            lesson: None,
            seed: self.seed.unwrap_or(0),
            findings: Vec::new(),
            covers_on: self.covers_on,
            planned_fuel_l: self.planned_fuel_l,
            speed: self.speed,
        };
        let mut random = self.random_findings.map(usize::from);
        if let Some(id) = &self.lesson {
            let lesson = preflight_lesson_library()?
                .into_iter()
                .find(|l| &l.id == id)
                .ok_or_else(|| invalid(Self::NAME, format!("unknown pre-flight lesson: {id}")))?;
            s.start_preflight_lesson(&lesson)?;
            options.lesson = Some(lesson.id.clone());
            options.seed = self.seed.unwrap_or(lesson.seed);
            options.covers_on = self.covers_on.or(Some(lesson.covers_on));
            options.planned_fuel_l = self.planned_fuel_l.or(lesson.planned_fuel_l);
            if findings.is_empty() {
                findings = lesson.findings.clone();
            }
            random = random.or(Some(usize::from(lesson.random_findings)));
        }
        let (def, ground_z, _) = s.preflight_definition(Self::NAME)?;
        if let Some(n) = random.filter(|n| *n > 0) {
            let taken: std::collections::BTreeSet<_> = findings.iter().map(|f| f.item.clone()).collect();
            findings.extend(
                select_findings(&def, options.seed, n + taken.len())
                    .into_iter()
                    .filter(|f| !taken.contains(&f.item))
                    .take(n),
            );
        }
        options.findings = findings;
        s.begin_preflight_with(def, ground_z, options)?;
        Ok(s.preflight_state(false))
    }
}

/// Read the walkaround. Instructors can see seeded faults and the full grade.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightStateCommand {
    /// Include seeded faults and the per-finding grade.
    #[serde(default)]
    pub instructor: bool,
}

impl Command for PreflightStateCommand {
    const NAME: &'static str = "preflight.state";
    const DESCRIPTION: &'static str = "Read the walkaround: stations, items, findings, the exterior camera framing, and the figure's action.";
    type Output = PreflightState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        Ok(s.preflight_state(self.instructor))
    }
}

/// Send the figure to an item's station to do its check. The item is done when the animation ends.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightPerform {
    pub item: String,
}

impl Command for PreflightPerform {
    const NAME: &'static str = "preflight.perform";
    const DESCRIPTION: &'static str = "Walk the figure to an item's station and perform its check; the item completes when the animation finishes.";
    type Output = PreflightState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let item = {
            let walk = s.walk_ref(Self::NAME)?;
            if walk.finished {
                return Err(invalid(Self::NAME, "the walkaround is finished"));
            }
            if walk.queue.len() >= flightcraft_aero::preflight::MAX_ITEMS {
                return Err(invalid(Self::NAME, "too many queued items"));
            }
            walk.def
                .item(&self.item)
                .ok_or_else(|| invalid(Self::NAME, format!("unknown item: {}", self.item)))?
                .clone()
        };
        s.check_propeller_safe(&item)?;
        if let Some(walk) = s.preflight.walk.as_mut() {
            walk.queue.push_back(self.item);
        }
        Ok(s.preflight_state(false))
    }
}

/// Seed a finding on an item. It is revealed only when that item is performed.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightFault {
    pub item: String,
    pub kind: DefectKind,
}

impl Command for PreflightFault {
    const NAME: &'static str = "preflight.fault";
    const DESCRIPTION: &'static str = "Seed a finding (a left-on cover, rope or chock, low oil or fuel, a loose cowl fastener, missing hinge hardware, a damaged tyre or fairing) on an item.";
    type Output = PreflightState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        s.walk_ref(Self::NAME)?;
        s.seed_preflight(&self.item, self.kind)?;
        Ok(s.preflight_state(true))
    }
}

/// Rectify a finding the student has found: refuel, top up, tighten, or replace.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightResolve {
    pub item: String,
}

impl Command for PreflightResolve {
    const NAME: &'static str = "preflight.resolve";
    const DESCRIPTION: &'static str = "Rectify a finding that has been found (maintenance, refuel, or top-up), so the aircraft is fit to fly.";
    type Output = PreflightState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let walk = s.walk_ref(Self::NAME)?;
        let rt = walk
            .items
            .get(&self.item)
            .ok_or_else(|| invalid(Self::NAME, format!("unknown item: {}", self.item)))?;
        if rt.finding.is_none() || rt.resolved {
            return Err(invalid(
                Self::NAME,
                "there is no found, unresolved finding on this item",
            ));
        }
        s.clear_seed(&self.item);
        if let Some(rt) = s
            .preflight
            .walk
            .as_mut()
            .and_then(|w| w.items.get_mut(&self.item))
        {
            rt.resolved = true;
        }
        s.sync_preflight_hazards();
        Ok(s.preflight_state(false))
    }
}

/// Run the walkaround clock without stepping the flight, for scripts and tests.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightAdvance {
    #[schemars(range(min = 0, max = 600))]
    pub seconds: f64,
}

impl Command for PreflightAdvance {
    const NAME: &'static str = "preflight.advance";
    const DESCRIPTION: &'static str =
        "Advance the walkaround animation by real seconds without stepping the flight clock.";
    type Output = PreflightState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        s.walk_ref(Self::NAME)?;
        crate::range(Self::NAME, "seconds", self.seconds, 0., 600.)?;
        s.advance_preflight(self.seconds);
        Ok(s.preflight_state(false))
    }
}

/// Set the animation speed-up.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightSpeed {
    #[schemars(range(min = 0.25, max = 16))]
    pub speed: f64,
}

impl Command for PreflightSpeed {
    const NAME: &'static str = "preflight.speed";
    const DESCRIPTION: &'static str =
        "Set the walkaround animation speed-up (1 is a realistic walking pace).";
    type Output = PreflightState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        s.walk_ref(Self::NAME)?;
        crate::range(Self::NAME, "speed", self.speed, 0.25, 16.)?;
        if let Some(walk) = s.preflight.walk.as_mut() {
            walk.speed = self.speed;
        }
        Ok(s.preflight_state(false))
    }
}

/// Finish the walkaround and read the grade. Seeded hazards stay on the aircraft.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightEnd {
    /// Include the per-finding grade.
    #[serde(default)]
    pub instructor: bool,
}

impl Command for PreflightEnd {
    const NAME: &'static str = "preflight.end";
    const DESCRIPTION: &'static str = "Finish the walkaround: the figure leaves the scene and the grade is returned. Unremoved covers and unrectified findings stay on the aircraft.";
    type Output = PreflightState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        s.walk_ref(Self::NAME)?;
        let previous = s.preflight.walk.as_mut().and_then(|walk| {
            walk.finished = true;
            walk.queue.clear();
            walk.running = None;
            walk.previous_camera.take()
        });
        s.leave_walkaround_camera(previous);
        Ok(s.preflight_state(self.instructor))
    }
}

#[cfg(test)]
mod tests;
