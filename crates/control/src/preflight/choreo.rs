//! Choreography: turns one pre-flight item into a timed plan of walking and acting, and evaluates
//! the figure and scene at any moment of that plan. Pure functions of the plan and the time, so
//! the same item always plays the same way, at any speed.
use flightcraft_aero::preflight::{Action, CabinEffect, Preflight, PropKind, Stance};
use flightcraft_aero::walkaround::{FigureAction, FigureState};
use std::f32::consts::{PI, TAU};

pub(crate) type V3 = [f32; 3];

pub(crate) fn v3(p: [f64; 3]) -> V3 {
    p.map(|v| v as f32)
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, k: f32) -> V3 {
    a.map(|v| v * k)
}
pub(crate) fn lerp(a: V3, b: V3, t: f32) -> V3 {
    add(scale(a, 1. - t), scale(b, t))
}
fn smooth(x: f32) -> f32 {
    let x = x.clamp(0., 1.);
    x * x * (3. - 2. * x)
}
/// Rise over `[a, b]`.
fn ramp(u: f32, a: f32, b: f32) -> f32 {
    smooth((u - a) / (b - a))
}
fn angle_lerp(a: f32, b: f32, t: f32) -> f32 {
    let mut d = (b - a).rem_euclid(TAU);
    if d > PI {
        d -= TAU;
    }
    a + d * t
}
fn right_of(heading: f32) -> V3 {
    [-heading.sin(), heading.cos(), 0.]
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Effect {
    /// The finding for this item becomes visible.
    Reveal,
    RemoveProp(String),
    OpenCanopy,
    Cabin(CabinEffect),
    Inside(bool),
}

#[derive(Clone, Debug)]
pub(crate) enum ActKind {
    Inspect {
        target: V3,
        crouch: bool,
        touch: bool,
    },
    Control {
        parts: Vec<String>,
        target: V3,
        amplitude: f32,
    },
    Fuel {
        tank: String,
        cap: V3,
    },
    Oil {
        target: V3,
    },
    Propeller {
        target: V3,
    },
    Take {
        prop: String,
        kind: PropKind,
        at: V3,
    },
    Cabin {
        target: V3,
    },
    /// Climb in (true) or out (false) via the step.
    Board {
        into: bool,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum Step {
    Walk { points: Vec<[f32; 2]>, length: f32 },
    Act { kind: ActKind },
}

#[derive(Clone, Debug)]
pub(crate) struct Timed {
    pub step: Step,
    pub start: f32,
    pub duration: f32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Plan {
    pub steps: Vec<Timed>,
    pub effects: Vec<(f32, Effect)>,
    pub total: f32,
}

/// Where the figure starts, between plans.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stance0 {
    pub node: String,
    pub pos: [f32; 2],
    pub heading: f32,
    pub inside: bool,
    pub phase: f32,
}

/// Fixed scene context for evaluation.
pub(crate) struct Context<'a> {
    pub def: &'a Preflight,
    pub ground_z: f32,
}

impl Plan {
    fn push(&mut self, step: Step, duration: f32) -> f32 {
        let start = self.total;
        self.steps.push(Timed {
            step,
            start,
            duration,
        });
        self.total += duration;
        start
    }
}

/// Shortest route along the closed loop from `from` to `to`, node positions inclusive.
pub(crate) fn route_between(def: &Preflight, from: &str, to: &str) -> Vec<[f32; 2]> {
    let n = def.route.len();
    let index = |id: &str| def.route.iter().position(|r| r.id == id);
    let (Some(a), Some(b)) = (index(from), index(to)) else {
        return Vec::new();
    };
    let at = |i: usize| {
        def.route
            .get(i % n)
            .map_or([0., 0.], |r| [r.position_m[0] as f32, r.position_m[1] as f32])
    };
    let length = |points: &[[f32; 2]]| -> f32 {
        points
            .windows(2)
            .map(|w| match w {
                [p, q] => ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt(),
                _ => 0.,
            })
            .sum()
    };
    let forward: Vec<_> = (0..=((b + n - a) % n)).map(|k| at(a + k)).collect();
    let backward: Vec<_> = (0..=((a + n - b) % n)).map(|k| at(a + n - k)).collect();
    if length(&forward) <= length(&backward) {
        forward
    } else {
        backward
    }
}

fn polyline_length(points: &[[f32; 2]]) -> f32 {
    points
        .windows(2)
        .map(|w| match w {
            [p, q] => ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt(),
            _ => 0.,
        })
        .sum()
}

/// Build the plan for one item from where the figure currently stands.
pub(crate) fn plan_item(def: &Preflight, item_id: &str, from: &Stance0, pace_mps: f32) -> Plan {
    let mut plan = Plan::default();
    let Some(item) = def.item(item_id) else {
        return plan;
    };
    let default_node = def
        .station(&item.station)
        .map_or(from.node.as_str(), |s| s.node.as_str())
        .to_string();
    let node = item.node.clone().unwrap_or(default_node);
    let mut at = from.node.clone();
    let mut inside = from.inside;
    let walk = |plan: &mut Plan, at: &mut String, to: &str| {
        if at.as_str() != to {
            let points = route_between(def, at, to);
            let length = polyline_length(&points);
            if length > 0.01 {
                plan.push(Step::Walk { points, length }, length / pace_mps);
            }
            *at = to.to_string();
        }
    };
    let leave = |plan: &mut Plan, inside: &mut bool| {
        if *inside {
            plan.push(
                Step::Act {
                    kind: ActKind::Board { into: false },
                },
                1.4,
            );
            plan.effects.push((plan.total, Effect::Inside(false)));
            *inside = false;
        }
    };
    match &item.action {
        Action::Cabin { effect, target_m } => {
            walk(&mut plan, &mut at, &node);
            if !inside {
                plan.effects.push((plan.total, Effect::OpenCanopy));
                plan.push(
                    Step::Act {
                        kind: ActKind::Board { into: true },
                    },
                    1.4,
                );
                plan.effects.push((plan.total, Effect::Inside(true)));
            }
            let start = plan.push(
                Step::Act {
                    kind: ActKind::Cabin {
                        target: v3(*target_m),
                    },
                },
                3.0,
            );
            plan.effects.push((start + 1.5, Effect::Cabin(*effect)));
            plan.effects.push((start + 1.5, Effect::Reveal));
        }
        Action::Remove { props } => {
            leave(&mut plan, &mut inside);
            for id in props {
                let Some(prop) = def.prop(id) else { continue };
                walk(&mut plan, &mut at, &prop.node);
                let start = plan.push(
                    Step::Act {
                        kind: ActKind::Take {
                            prop: id.clone(),
                            kind: prop.kind,
                            at: v3(prop.position_m),
                        },
                    },
                    2.6,
                );
                plan.effects.push((start + 1.0, Effect::Reveal));
                plan.effects.push((start + 2.6, Effect::RemoveProp(id.clone())));
            }
        }
        action => {
            leave(&mut plan, &mut inside);
            walk(&mut plan, &mut at, &node);
            let (kind, duration) = match action {
                Action::Inspect {
                    target_m,
                    stance,
                    touch,
                } => (
                    ActKind::Inspect {
                        target: v3(*target_m),
                        crouch: *stance == Stance::Crouch,
                        touch: *touch,
                    },
                    3.2,
                ),
                Action::MoveControl {
                    parts,
                    target_m,
                    amplitude,
                } => (
                    ActKind::Control {
                        parts: parts.clone(),
                        target: v3(*target_m),
                        amplitude: *amplitude as f32,
                    },
                    4.4,
                ),
                Action::FuelCap { tank } => {
                    let cap = def
                        .fuel
                        .caps
                        .iter()
                        .find(|c| &c.tank == tank)
                        .map_or([0.; 3], |c| v3(c.cap_m));
                    (
                        ActKind::Fuel {
                            tank: tank.clone(),
                            cap,
                        },
                        6.0,
                    )
                }
                Action::OilCheck { target_m } => (
                    ActKind::Oil {
                        target: v3(*target_m),
                    },
                    5.5,
                ),
                Action::PalmProp { target_m } => (
                    ActKind::Propeller {
                        target: v3(*target_m),
                    },
                    4.5,
                ),
                Action::Cabin { .. } | Action::Remove { .. } => return plan,
            };
            let start = plan.push(Step::Act { kind }, duration);
            plan.effects.push((start + duration * 0.55, Effect::Reveal));
        }
    }
    plan
}

/// Everything the plan contributes at one moment, beyond the figure itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Frame {
    pub figure: FigureState,
    pub surfaces: Vec<(String, f32)>,
    pub propeller_phase: Option<f32>,
    pub open_cap: Option<String>,
    pub fuel_view: Option<String>,
    pub dipstick: bool,
    /// A prop in the hand, with its current position.
    pub taking: Option<(String, V3)>,
    pub item_target: Option<V3>,
    /// Where the figure ends up, for the next plan.
    pub end: Option<Stance0>,
}

fn rest_hand(pos: V3, heading: f32, side: f32, ground_z: f32) -> V3 {
    let r = right_of(heading);
    [
        pos[0] + r[0] * 0.25 * side,
        pos[1] + r[1] * 0.25 * side,
        ground_z - 0.82,
    ]
}

fn face(from: [f32; 2], to: V3) -> f32 {
    (to[1] - from[1]).atan2(to[0] - from[0])
}

/// The figure and scene at time `t` of `plan`, starting from `from`.
pub(crate) fn evaluate(ctx: &Context<'_>, plan: &Plan, from: &Stance0, t: f32) -> Frame {
    let gz = ctx.ground_z;
    let mut pos2 = from.pos;
    let mut heading = from.heading;
    let mut phase = from.phase;
    let mut inside = from.inside;
    let mut node = from.node.clone();
    let mut frame = Frame::default();
    // Replay completed steps to find where the figure stands, then play the current one.
    for timed in &plan.steps {
        let local = t - timed.start;
        let done = local >= timed.duration;
        let u = (local / timed.duration.max(1e-3)).clamp(0., 1.);
        match &timed.step {
            Step::Walk { points, length } => {
                let d = if done { *length } else { u * *length };
                let (p, h) = along(points, d);
                if local >= 0. {
                    pos2 = p;
                    heading = h.unwrap_or(heading);
                    phase = from.phase + d * TAU / 1.5;
                    if let Some(last) = points.last().filter(|_| done) {
                        pos2 = *last;
                    }
                }
                if local >= 0. && !done {
                    frame.figure = FigureState {
                        position_m: [pos2[0], pos2[1], gz],
                        heading_rad: heading,
                        walk_phase: phase,
                        walking: true,
                        action: FigureAction::Walking,
                        ..Default::default()
                    };
                    frame.end = None;
                    return finish(frame, &node, pos2, heading, inside, phase);
                }
                if done && let Some(r) = points.last().and_then(|p| nearest_node(ctx.def, *p)) {
                    node = r;
                }
            }
            Step::Act { kind } => {
                if local < 0. {
                    continue;
                }
                if done {
                    // Completed acts only change where the figure is and whether it is inside.
                    match kind {
                        ActKind::Board { into } => inside = *into,
                        ActKind::Take { at, .. } => {
                            heading = face(pos2, *at);
                        }
                        ActKind::Inspect { target, .. }
                        | ActKind::Control { target, .. }
                        | ActKind::Oil { target }
                        | ActKind::Propeller { target }
                        | ActKind::Cabin { target } => heading = face(pos2, *target),
                        ActKind::Fuel { cap, .. } => heading = face(pos2, *cap),
                    }
                    continue;
                }
                act(ctx, kind, u, local, pos2, heading, inside, phase, &mut frame);
                return finish(frame, &node, pos2, heading, inside, phase);
            }
        }
    }
    // Plan complete or empty: stand at the end.
    frame.figure = if inside {
        seated(ctx, heading, FigureAction::InCockpit)
    } else {
        FigureState {
            position_m: [pos2[0], pos2[1], gz],
            heading_rad: heading,
            walk_phase: phase,
            ..Default::default()
        }
    };
    finish(frame, &node, pos2, heading, inside, phase)
}

fn finish(mut frame: Frame, node: &str, pos: [f32; 2], heading: f32, inside: bool, phase: f32) -> Frame {
    frame.end = Some(Stance0 {
        node: node.to_string(),
        pos,
        heading,
        inside,
        phase,
    });
    frame
}

fn seated(ctx: &Context<'_>, heading: f32, action: FigureAction) -> FigureState {
    let seat = v3(ctx.def.seat_m);
    FigureState {
        position_m: seat,
        heading_rad: heading,
        seated: true,
        action,
        ..Default::default()
    }
}

fn nearest_node(def: &Preflight, p: [f32; 2]) -> Option<String> {
    def.route
        .iter()
        .min_by(|a, b| {
            let da = (a.position_m[0] as f32 - p[0]).powi(2) + (a.position_m[1] as f32 - p[1]).powi(2);
            let db = (b.position_m[0] as f32 - p[0]).powi(2) + (b.position_m[1] as f32 - p[1]).powi(2);
            da.total_cmp(&db)
        })
        .map(|n| n.id.clone())
}

fn along(points: &[[f32; 2]], d: f32) -> ([f32; 2], Option<f32>) {
    let mut left = d;
    for w in points.windows(2) {
        let [p, q] = match w {
            [p, q] => [*p, *q],
            _ => continue,
        };
        let seg = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
        if seg < 1e-6 {
            continue;
        }
        let heading = (q[1] - p[1]).atan2(q[0] - p[0]);
        if left <= seg {
            let k = left / seg;
            return (
                [p[0] + (q[0] - p[0]) * k, p[1] + (q[1] - p[1]) * k],
                Some(heading),
            );
        }
        left -= seg;
    }
    (points.last().copied().unwrap_or([0., 0.]), None)
}

#[allow(clippy::too_many_arguments)]
fn act(
    ctx: &Context<'_>,
    kind: &ActKind,
    u: f32,
    local: f32,
    pos2: [f32; 2],
    start_heading: f32,
    inside: bool,
    phase: f32,
    frame: &mut Frame,
) {
    let gz = ctx.ground_z;
    let pos = [pos2[0], pos2[1], gz];
    let focus = |target: V3| face(pos2, target);
    let turn = |target: V3| angle_lerp(start_heading, focus(target), ramp(u, 0., 0.15));
    let base = |heading: f32, action: FigureAction| FigureState {
        position_m: pos,
        heading_rad: heading,
        walk_phase: phase,
        action,
        ..Default::default()
    };
    let hand_to = |heading: f32, side: f32, target: V3, w: f32| -> Option<V3> {
        (w > 0.001).then(|| lerp(rest_hand(pos, heading, side, gz), target, w))
    };
    match kind {
        ActKind::Inspect {
            target,
            crouch,
            touch,
        } => {
            let h = turn(*target);
            let w = ramp(u, 0.1, 0.3) * (1. - ramp(u, 0.85, 1.));
            let mut f = base(
                h,
                if *crouch {
                    FigureAction::Crouching
                } else {
                    FigureAction::Looking
                },
            );
            f.crouch = if *crouch { 0.85 * w } else { 0. };
            f.lean_rad = 0.3 * w;
            f.gaze_m = Some(*target);
            if *touch {
                let hw = ramp(u, 0.3, 0.5) * (1. - ramp(u, 0.7, 0.85));
                f.right_hand_m = hand_to(h, 1., *target, hw);
                if hw > 0.5 {
                    f.action = FigureAction::Touching;
                }
            }
            frame.item_target = Some(*target);
            frame.figure = f;
        }
        ActKind::Control {
            parts,
            target,
            amplitude,
        } => {
            let h = turn(*target);
            let rocking = ramp(u, 0.12, 0.2) * (1. - ramp(u, 0.88, 0.96));
            let cycles = 3.0;
            let wave = ((u - 0.15) / 0.75 * cycles * TAU).sin();
            let mut f = base(h, FigureAction::MovingControl);
            f.lean_rad = 0.25 * rocking;
            let bob = [0., 0., 0.1 * wave * rocking * amplitude];
            let hand = add(*target, bob);
            f.right_hand_m = hand_to(h, 1., hand, rocking);
            f.gaze_m = Some(*target);
            for p in parts {
                let value = if p.starts_with("flap") {
                    amplitude * (0.5 - 0.5 * ((u - 0.15) / 0.75 * cycles * TAU).cos()) * rocking
                } else {
                    amplitude * wave * rocking
                };
                frame.surfaces.push((p.clone(), value));
            }
            frame.item_target = Some(*target);
            frame.figure = f;
        }
        ActKind::Fuel { tank, cap } => {
            let h = turn(*cap);
            let reach = ramp(u, 0.08, 0.22) * (1. - ramp(u, 0.9, 1.));
            let look = ramp(u, 0.4, 0.55) * (1. - ramp(u, 0.8, 0.88));
            let mut f = base(h, FigureAction::CheckingFuel);
            f.lean_rad = 0.2 * reach + 0.35 * look;
            f.crouch = 0.25 * look;
            f.right_hand_m = hand_to(h, 1., add(*cap, [0., 0., -0.05]), reach);
            f.gaze_m = Some(*cap);
            if (0.3..0.88).contains(&u) {
                frame.open_cap = Some(tank.clone());
            }
            if (0.42..0.84).contains(&u) {
                frame.fuel_view = Some(tank.clone());
            }
            frame.item_target = Some(*cap);
            frame.figure = f;
        }
        ActKind::Oil { target } => {
            let h = turn(*target);
            let reach = ramp(u, 0.08, 0.25) * (1. - ramp(u, 0.9, 1.));
            let mut f = base(h, FigureAction::CheckingOil);
            f.lean_rad = 0.3 * reach;
            let pull = ramp(u, 0.3, 0.45) * (1. - ramp(u, 0.8, 0.9));
            let hand = add(*target, [0., 0., -0.25 * pull]);
            f.right_hand_m = hand_to(h, 1., hand, reach);
            f.gaze_m = Some(hand);
            frame.dipstick = (0.4..0.85).contains(&u);
            frame.item_target = Some(*target);
            frame.figure = f;
        }
        ActKind::Propeller { target } => {
            let h = turn(*target);
            let on = ramp(u, 0.12, 0.25) * (1. - ramp(u, 0.88, 0.96));
            let angle = (TAU / 3.) * ramp(u, 0.25, 0.85);
            let centre = [target[0], 0., target[2]];
            let radius = (target[1].abs()).max(0.3);
            let blade = [
                centre[0],
                centre[1] + radius * angle.cos(),
                centre[2] + radius * angle.sin(),
            ];
            let mut f = base(h, FigureAction::HandlingPropeller);
            f.lean_rad = 0.35 * on;
            f.right_hand_m = hand_to(h, 1., blade, on);
            f.gaze_m = Some(blade);
            frame.propeller_phase = Some(angle);
            frame.item_target = Some(blade);
            frame.figure = f;
        }
        ActKind::Take { prop, kind, at } => {
            let h = turn(*at);
            let reach = ramp(u, 0.08, 0.3);
            let lift = ramp(u, 0.45, 0.8);
            let carried = lift > 0.0;
            let mut f = base(h, FigureAction::Removing);
            f.crouch = 0.7 * reach * (1. - lift);
            f.lean_rad = 0.35 * reach * (1. - lift);
            let rest = rest_hand(pos, h, 1., gz);
            let hand = if carried { lerp(*at, rest, lift) } else { *at };
            f.right_hand_m = hand_to(h, 1., hand, reach);
            f.gaze_m = Some(*at);
            if u >= 0.4 {
                f.carrying = Some(*kind);
                frame.taking = Some((prop.clone(), hand));
            }
            frame.item_target = Some(*at);
            frame.figure = f;
        }
        ActKind::Cabin { target } => {
            let mut f = seated(ctx, start_heading, FigureAction::InCockpit);
            let reach = ramp(u, 0.15, 0.4) * (1. - ramp(u, 0.7, 0.95));
            f.heading_rad = angle_lerp(start_heading, 0., 1.);
            let seat = v3(ctx.def.seat_m);
            f.right_hand_m = (reach > 0.001).then(|| lerp(add(seat, [0.35, 0.25, -0.35]), *target, reach));
            f.gaze_m = Some(*target);
            f.lean_rad = 0.2 * reach;
            frame.item_target = Some(*target);
            frame.figure = f;
        }
        ActKind::Board { into } => {
            let seat = v3(ctx.def.seat_m);
            let step = [-2.85, -0.8, 0.285];
            let u2 = if *into { u } else { 1. - u };
            let p = if u2 < 0.5 {
                lerp(pos, step, smooth(u2 / 0.5))
            } else {
                lerp(step, seat, smooth((u2 - 0.5) / 0.5))
            };
            let mut f = base(
                angle_lerp(start_heading, 0., if *into { ramp(u, 0.3, 0.9) } else { 0. }),
                FigureAction::Boarding,
            );
            f.position_m = p;
            f.seated = u2 > 0.75;
            f.crouch = 0.4 * (1. - (2. * u2 - 1.).abs());
            frame.item_target = None;
            let _ = (inside, local);
            frame.figure = f;
        }
    }
}
