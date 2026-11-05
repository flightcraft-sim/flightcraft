//! The traffic engine: kinematics, the approach/tower/departure controllers, and the shared
//! frequency. Pure and deterministic: the session only supplies time, busy channels, and the ATIS letter.
use super::craft::*;
use super::field::*;
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Stage {
    // Arrival
    Inbound,
    AwaitHi,
    HiDescent,
    LoCall,
    AwaitLo,
    Lo,
    TowerCall,
    AwaitTower,
    Landing,
    Rollout,
    Exiting,
    // Departure
    Hold,
    AwaitTakeoff,
    Rolling,
    DepCall,
    AwaitDep,
    Climb,
    HiCall2,
    AwaitHi2,
    Out,
    // Flight following
    FfCall,
    AwaitFf,
    Enroute,
    // Circuit
    PatCall,
    AwaitPat,
    PatFly,
    PatClimb,
    Done,
}
impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inbound => "inbound, not yet in contact",
            Self::AwaitHi | Self::FfCall | Self::AwaitFf => "establishing radar contact",
            Self::HiDescent => "descending in the approach high sector",
            Self::LoCall | Self::AwaitLo => "changing to the approach low sector",
            Self::Lo => "vectored for sequencing",
            Self::TowerCall | Self::AwaitTower => "cleared for the approach, on tower",
            Self::Landing => "cleared to land",
            Self::Rollout => "landing rollout",
            Self::Exiting => "exiting the runway",
            Self::Hold => "holding short",
            Self::AwaitTakeoff => "ready for departure",
            Self::Rolling => "takeoff roll and initial climb",
            Self::DepCall | Self::AwaitDep => "checking in with departure",
            Self::Climb => "climbing on departure",
            Self::HiCall2 | Self::AwaitHi2 => "changing to approach",
            Self::Out => "departing the area",
            Self::Enroute => "VFR flight following",
            Self::PatCall | Self::AwaitPat => "in the circuit, awaiting clearance",
            Self::PatFly => "cleared in the circuit",
            Self::PatClimb => "circuit climb-out",
            Self::Done => "radar service terminated",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Ac {
    pub id: u32,
    pub ident: Identity,
    pub body: Body,
    pub kind: FleetFlight,
    pub stage: Stage,
    pub busy: bool,
    pub freq: u32,
    pub station: usize,
    pub squawk: String,
    pub alt_ft: Option<u32>,
    pub speed_kt: Option<u16>,
    pub hdg_deg: Option<u16>,
    pub cleared: bool,
    pub touch_go: bool,
    pub call_at: f64,
    pub rng: Rng,
    pub cruise_ft: u32,
    pub laps_left: u8,
    pub exit: [f64; 2],
    pub remove_at: Option<f64>,
    pub gone: bool,
    pub dep_hdg: f64,
    pub speed_cmd_at: f64,
    pub contacted: bool,
    /// A transit flight between two busy airports: the index of the airport it is flying to. The
    /// departure area hands it over at the midpoint.
    pub transit: Option<usize>,
    /// Flies the corridor between two busy airports (a transit, before or after the hand-over). No
    /// other flight-following traffic shares its altitude, since the two tracks would be long
    /// and the faster aircraft would overtake the slower in the same lane.
    pub corridor: bool,
    /// Place in the landing sequence, ascending. Fixed once given: ETAs shift as speeds are assigned, but
    /// nobody overtakes on the common final.
    pub order: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct Msg {
    pub ready: f64,
    pub seq: u64,
    pub ac: u32,
    pub ex: u32,
    pub kind: FleetCallKind,
    pub role: FleetRole,
    pub station: usize,
    pub text: String,
    pub instr: Vec<AtcInstruction>,
    pub next: Stage,
    pub sequence: Option<FleetSequence>,
    pub freq: u32,
    pub corrected: bool,
    /// Sent again because the user stepped on the first attempt.
    pub repeat: bool,
}
impl Msg {
    fn priority(&self) -> u8 {
        match self.kind {
            FleetCallKind::Correction => 0,
            FleetCallKind::Readback => 1,
            FleetCallKind::Instruction => 2,
            FleetCallKind::Information | FleetCallKind::Terminate => 3,
            FleetCallKind::CheckIn => 4,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Chan {
    pub quiet_until: f64,
    /// The recorded event of the transmission in flight, when the user's radios can hear it.
    pub event: Option<u64>,
    pub inflight: Option<(f64, Msg)>,
    pub queue: Vec<Msg>,
}

/// One transmission that starts now. The session decides whether the user can hear it.
#[derive(Clone, Debug)]
pub(crate) struct Tx {
    pub freq: u32,
    pub station: usize,
    pub role: FleetRole,
    pub text: String,
    pub call: FleetCall,
    pub origin: Option<[f64; 3]>,
    pub callsign: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Fleet {
    pub density: TrafficDensity,
    pub field: Field,
    pub craft: Vec<Ac>,
    pub chans: BTreeMap<u32, Chan>,
    pub seed: u64,
    pub clock: f64,
    pub stats: FleetStats,
    pub info: String,
    pub info_at: f64,
    reserved: Vec<String>,
    rng: Rng,
    next_id: u32,
    next_ex: u32,
    seq: u64,
    next_spawn: f64,
    last_release: f64,
    pub guard_note: String,
    /// Every kind of pair the physics guard had to act on: label -> (steps, first time).
    pub guard_pairs: BTreeMap<String, (u64, f64)>,
    /// Other busy airports in the region: their index in the session and field centre.
    pub neighbours: Vec<(usize, [f64; 2])>,
    /// Callsigns in use at the other airports, so no two aircraft in the region share one.
    pub foreign: Vec<String>,
    /// Transit flights that crossed the midpoint to the airport at the index, for the session to hand over.
    pub handoffs: Vec<(usize, Ac)>,
    scratch: Ac,
}

fn capital(text: &str) -> String {
    let mut c = text.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
}
/// Landing gaps in seconds: a spawn needs `SPAWN_GAP_S`; a follower closer than `SPEED_TRIGGER_S` is
/// slowed so the gap opens to `SPEED_TARGET_S`; the restriction ends once the unrestricted gap passes
/// `SPEED_CANCEL_S`.
/// Gap in seconds a circuit clearance leaves behind the landing aircraft ahead of it, and ahead of the
/// one behind it (which the speed control then opens further).
const CIRCUIT_AHEAD_S: f64 = 150.0;
const CIRCUIT_BEHIND_S: f64 = 150.0;
/// The least gap left before the landing aircraft behind, whatever slowing it has in hand.
const CIRCUIT_BEHIND_MIN_S: f64 = 60.0;
/// Below this gap to the aircraft ahead, with no speed left to give, the follower goes around.
const GO_AROUND_S: f64 = 70.0;
const SPAWN_GAP_S: f64 = 130.0;
const SPEED_TRIGGER_S: f64 = 115.0;
const SPEED_TARGET_S: f64 = 135.0;
const SPEED_CANCEL_S: f64 = 150.0;
/// Distance of the downwind leg from the runway, metres: wide enough that opposite sides of the
/// circuit stay well over a mile and a half apart even where the turns cut the corners.
const PATTERN_Y: f64 = 3400.0;
/// Every circuit aircraft flies the same circuit speed, so a faster type never closes on a slower one
/// in the pattern; a cleared aircraft slows to its own approach speed if that is lower.
const CIRCUIT_KT: f64 = 90.0;
fn circuit_kt(spec: &TypeSpec, cleared: bool) -> f64 {
    if cleared {
        (spec.approach_kt + 8.0).min(CIRCUIT_KT)
    } else {
        CIRCUIT_KT
    }
}
fn profile_kt(spec: &TypeSpec, rem: f64) -> f64 {
    (spec.approach_kt + (rem - 4.0 * NM) / (14.0 * NM) * (spec.cruise_kt - spec.approach_kt))
        .clamp(spec.approach_kt, spec.cruise_kt)
}
/// Time to fly `rem` metres of the approach profile, with an optional assigned speed cap in knots.
fn eta_profile(spec: &TypeSpec, rem: f64, cap_kt: Option<f64>) -> f64 {
    let mut t = 0.0;
    let mut r = rem.max(0.0);
    while r > 1.0 {
        let step = r.min(NM);
        let mut kt = profile_kt(spec, r - step / 2.0);
        if let Some(k) = cap_kt {
            kt = kt.min(k).max(spec.min_kt);
        }
        t += step / (kt * KT);
        r -= step;
    }
    t
}

impl Fleet {
    pub fn new(
        field: Field,
        density: TrafficDensity,
        seed: u64,
        now: f64,
        reserved: Vec<String>,
        id_base: u32,
        neighbours: Vec<(usize, [f64; 2])>,
    ) -> Self {
        let mut placeholder = Rng::new(0);
        let mut scratch = Ac {
            id: 0,
            ident: identity(&mut placeholder, &[], 0.0),
            body: Body::new([0.0, 0.0], 0.0, 0.0, 0.0),
            kind: FleetFlight::Arrival,
            stage: Stage::Done,
            busy: true,
            freq: 0,
            station: 0,
            squawk: String::new(),
            alt_ft: None,
            speed_kt: None,
            hdg_deg: None,
            cleared: false,
            touch_go: false,
            call_at: 0.0,
            rng: placeholder,
            cruise_ft: 0,
            laps_left: 0,
            exit: [0.0, 0.0],
            remove_at: None,
            gone: true,
            dep_hdg: 0.0,
            speed_cmd_at: 0.0,
            contacted: false,
            transit: None,
            corridor: false,
            order: 0.0,
        };
        scratch.gone = true;
        let mut fleet = Self {
            density,
            field,
            craft: Vec::new(),
            chans: BTreeMap::new(),
            seed,
            clock: now - WARMUP_S,
            stats: FleetStats::default(),
            info: "alpha".into(),
            info_at: f64::MAX,
            reserved,
            rng: Rng::new(seed ^ u64::from(density as u8).wrapping_mul(0x1234_5677)),
            next_id: id_base + 1,
            next_ex: 1,
            seq: 0,
            next_spawn: now - WARMUP_S,
            last_release: f64::MIN,
            guard_note: String::new(),
            guard_pairs: BTreeMap::new(),
            neighbours,
            foreign: Vec::new(),
            handoffs: Vec::new(),
            scratch,
        };
        // Run the traffic silently for ten minutes so the frequency opens already busy.
        let mut t = now - WARMUP_S;
        while t < now - 1e-9 {
            t += 0.25;
            let _ = fleet.tick(t, 0.25, &[]);
        }
        fleet.stats = FleetStats::default();
        fleet.guard_note.clear();
        fleet.guard_pairs.clear();
        fleet.info_at = now;
        fleet
    }

    pub fn idx(&self, id: u32) -> Option<usize> {
        self.craft.iter().position(|a| a.id == id)
    }

    fn channel_key(&self, freq: u32) -> Option<u32> {
        let carrier = carrier_thirds_hz(freq)?;
        // The traffic channels are keyed by their published kHz; match by carrier.
        Some(
            self.chans
                .keys()
                .copied()
                .find(|k| carrier_thirds_hz(*k) == Some(carrier))
                .unwrap_or(freq),
        )
    }
    /// Could a controller call to the user start now? It may only start when the carrier is idle,
    /// past the inter-call gap, and no correction or readback is waiting.
    pub fn channel_open(&self, freq: u32, now: f64, in_use: bool) -> bool {
        let Some(key) = self.channel_key(freq) else {
            return false;
        };
        self.chans.get(&key).is_none_or(|c| {
            c.inflight.is_none()
                && now >= c.quiet_until
                && !c.queue.iter().any(|m| m.ready <= now && m.priority() <= 1)
        }) && !in_use
    }
    /// A controller call to the user goes through the same frequency queue as the traffic: it holds
    /// the carrier for its spoken duration and the usual 1.5-3.5 s gap follows it.
    pub fn claim_channel(&mut self, freq: u32, now: f64, in_use: bool, text: &str, voice: u8) -> bool {
        if !self.channel_open(freq, now, in_use) {
            return false;
        }
        let Some(key) = self.channel_key(freq) else {
            return false;
        };
        let hold = Msg {
            ready: now,
            seq: 0,
            ac: 0,
            ex: 0,
            kind: FleetCallKind::Information,
            role: FleetRole::Controller,
            station: 0,
            text: text.into(),
            instr: Vec::new(),
            next: Stage::Done,
            sequence: None,
            freq: key,
            corrected: false,
            repeat: false,
        };
        // Held for the voice's spoken duration; its audio plays while it is on the air.
        let duration = controller_voice(voice).duration_s(text);
        let c = self.chans.entry(key).or_default();
        c.inflight = Some((now + duration, hold));
        true
    }

    pub fn tick(&mut self, now: f64, dt: f64, busy: &[u32]) -> Vec<Tx> {
        self.advance(now, dt);
        self.start_tx(now, busy)
    }

    /// Move the traffic and let the controllers decide, without starting any transmission yet. The
    /// session may take the carrier for a call to the user before `start_tx` hands it to a queued call.
    pub fn advance(&mut self, now: f64, dt: f64) {
        self.clock = now;
        self.finish(now);
        for i in 0..self.craft.len() {
            self.fly(i, now, dt);
        }
        self.safety_net(dt);
        self.spawn_step(now);
        for i in 0..self.craft.len() {
            self.decide(i, now);
        }
        self.craft.retain(|a| !a.gone);
    }

    // ---------------------------------------------------------------- landing order

    fn is_lander(a: &Ac) -> bool {
        match a.kind {
            FleetFlight::Arrival => a.stage < Stage::Rollout,
            FleetFlight::Pattern => a.cleared && a.stage == Stage::PatFly,
            _ => false,
        }
    }
    fn eta(&self, a: &Ac) -> f64 {
        self.eta_with(a, a.speed_kt.map(f64::from))
    }
    /// Time to the threshold if the aircraft flew with `cap_kt` assigned (`None`: its normal profile).
    fn eta_with(&self, a: &Ac, cap_kt: Option<f64>) -> f64 {
        let rem = a.body.path_remaining();
        let spec = a.ident.spec();
        match a.kind {
            FleetFlight::Arrival => eta_profile(spec, rem, cap_kt),
            _ => {
                // Before clearance, estimate with the speed it will fly once cleared, and a little extra
                // distance for the turns.
                let waiting = matches!(a.stage, Stage::PatCall | Stage::AwaitPat);
                let kt = circuit_kt(spec, a.cleared || waiting);
                let kt = cap_kt.map_or(kt, |k| kt.min(k).max(spec.min_kt));
                self.circuit_distance(a) * if waiting { 1.08 } else { 1.0 } / (kt * KT)
            }
        }
    }
    /// Metres a circuit aircraft still has to fly to touch down: along its cleared path, or, before it
    /// is cleared, around the rest of the circuit with the base turn made as soon as it can be.
    fn circuit_distance(&self, a: &Ac) -> f64 {
        let f = &self.field;
        match a.stage {
            Stage::PatCall | Stage::AwaitPat => {
                let x_f = f.along(a.body.pos).min(-2200.0) - 300.0;
                dist(a.body.pos, f.rw(x_f, 0.0)) + x_f.abs()
            }
            Stage::PatClimb => a.body.path_remaining() + PATTERN_Y + 2200.0,
            _ => a.body.path_remaining(),
        }
    }
    /// Whether the lander `id` can still be slowed enough to leave a circuit aircraft that lands
    /// `gap` seconds ahead of it the full spacing: the speed control opens the gap further out, where
    /// slowing to the lowest speed buys minutes, not seconds.
    fn can_open_behind(&self, id: u32, gap: f64) -> bool {
        let Some(l) = self.idx(id).and_then(|k| self.craft.get(k)) else {
            return false;
        };
        let slowest = (l.ident.spec().min_kt / 5.0).ceil() * 5.0;
        let capacity = (self.eta_with(l, Some(slowest)) - self.eta(l)).max(0.0);
        gap >= CIRCUIT_BEHIND_MIN_S && gap + 0.5 * capacity >= CIRCUIT_BEHIND_S
    }
    /// The landing sequence: (id, ETA to the threshold), in landing order.
    fn landers(&self) -> Vec<(u32, f64)> {
        let mut v: Vec<(f64, u32, f64)> = self
            .craft
            .iter()
            .filter(|a| !a.gone && Self::is_lander(a))
            .map(|a| (a.order, a.id, self.eta(a)))
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        v.into_iter().map(|(_, id, eta)| (id, eta)).collect()
    }
    /// The latest predicted landing time of anyone in the sequence.
    fn last_eta(&self) -> Option<f64> {
        self.landers()
            .iter()
            .map(|l| l.1)
            .fold(None, |m, e| Some(m.map_or(e, |m: f64| m.max(e))))
    }
    /// An order value that puts an aircraft landing at `eta` into the sequence between its neighbours.
    fn order_for(&self, id: u32, eta: f64) -> f64 {
        let others: Vec<(f64, f64)> = self
            .craft
            .iter()
            .filter(|a| !a.gone && a.id != id && Self::is_lander(a))
            .map(|a| (a.order, self.eta(a)))
            .collect();
        let before = others
            .iter()
            .filter(|o| o.1 < eta)
            .map(|o| o.0)
            .fold(f64::MIN, f64::max);
        let after = others
            .iter()
            .map(|o| o.0)
            .filter(|k| *k > before)
            .fold(f64::MAX, f64::min);
        match (before > f64::MIN, after < f64::MAX) {
            (true, true) => (before + after) / 2.0,
            (true, false) => before + 1.0,
            (false, true) => after - 1.0,
            (false, false) => 0.0,
        }
    }
    /// The lander sequenced immediately ahead, and this aircraft's landing number.
    fn leader_of(&self, id: u32) -> (Option<u32>, u8) {
        let landers = self.landers();
        let Some(pos) = landers.iter().position(|l| l.0 == id) else {
            return (landers.last().map(|l| l.0), landers.len() as u8 + 1);
        };
        (
            pos.checked_sub(1).and_then(|p| landers.get(p)).map(|l| l.0),
            pos as u8 + 1,
        )
    }
    fn runway_occupied(&self) -> bool {
        self.craft.iter().any(|a| {
            !a.gone && {
                let x = self.field.along(a.body.pos);
                let y = self.field.cross(a.body.pos);
                let low = a.body.on_ground || a.body.alt_m - self.field.elev_m < 60.0;
                low && a.stage != Stage::Hold
                    && a.stage != Stage::AwaitTakeoff
                    && x > -30.0
                    && x < self.field.length_m + 30.0
                    && y.abs() < 45.0
                    && !(a.kind == FleetFlight::Arrival && a.stage < Stage::Landing)
            }
        })
    }
    fn can_release(&self, now: f64) -> bool {
        !self.runway_occupied()
            && now - self.last_release >= 75.0
            // The departure ahead is airborne, turned, and well clear of the field (radar separation).
            && !self.craft.iter().any(|a| {
                !a.gone
                    && a.kind == FleetFlight::Departure
                    && a.stage >= Stage::Rolling
                    && a.stage <= Stage::Out
                    && (a.body.on_ground || dist(a.body.pos, self.field.thr) < 3.5 * NM)
            })
            && self.landers().iter().all(|l| l.1 > 110.0)
            && !self.craft.iter().any(|a| {
                a.stage == Stage::PatClimb && self.field.along(a.body.pos) < self.field.length_m + 2000.0
            })
    }

    // ---------------------------------------------------------------- spawning

    fn count(&self, kind: FleetFlight) -> usize {
        self.craft.iter().filter(|a| !a.gone && a.kind == kind).count()
    }
    fn spawn_step(&mut self, now: f64) {
        let want = self.density.target();
        if want == 0 || self.craft.len() >= want || now < self.next_spawn {
            return;
        }
        let pattern = if want >= 12 {
            2
        } else if want >= 6 {
            1
        } else {
            0
        };
        let departures = (want / 6).max(1);
        let overflights = want / 4;
        let arrivals = want.saturating_sub(pattern + departures + overflights);
        let wanted = [
            (FleetFlight::Arrival, arrivals),
            (FleetFlight::FlightFollowing, overflights),
            (FleetFlight::Departure, departures),
            (FleetFlight::Pattern, pattern),
        ];
        // The most under-represented kind first; when its slot is not clear, the next kind.
        let mut order: Vec<(i64, usize, FleetFlight)> = wanted
            .iter()
            .enumerate()
            .map(|(rank, (k, n))| (*n as i64 - self.count(*k) as i64, rank, *k))
            .filter(|(deficit, ..)| *deficit > 0)
            .collect();
        order.sort_by_key(|(deficit, rank, _)| (-*deficit, *rank));
        let mut ok = false;
        for (.., kind) in order {
            if self.spawn(kind, now) {
                ok = true;
                break;
            }
        }
        self.next_spawn = now + if ok { self.rng.range(8.0, 18.0) } else { 12.0 };
    }

    /// Every spoken and short callsign of this airport's aircraft.
    pub fn callsigns(&self) -> Vec<String> {
        self.craft
            .iter()
            .flat_map(|a| [a.ident.full.clone(), a.ident.short.clone()])
            .collect()
    }

    /// Take over a transit flight that the neighbouring airport handed over: it checks in with this
    /// airport's approach and carries on to its exit point. Dropped if it would share an altitude with
    /// flight-following traffic already close by.
    pub fn adopt(&mut self, mut ac: Ac, now: f64) {
        let clash = self
            .craft
            .iter()
            .any(|o| !o.gone && o.kind == FleetFlight::FlightFollowing && o.cruise_ft == ac.cruise_ft);
        if clash {
            return;
        }
        ac.id = self.next_id;
        self.next_id += 1;
        self.stats.spawned += 1;
        ac.station = self.field.hi;
        ac.freq = self.field.st(ac.station).channel_khz;
        ac.stage = Stage::FfCall;
        ac.busy = false;
        ac.gone = false;
        ac.transit = None;
        ac.corridor = true;
        ac.contacted = false;
        ac.cleared = false;
        ac.remove_at = None;
        ac.alt_ft = None;
        ac.speed_kt = None;
        ac.hdg_deg = None;
        ac.call_at = now + ac.rng.range(3.0, 15.0);
        self.craft.push(ac);
    }

    fn taken(&self) -> Vec<String> {
        let mut v = self.reserved.clone();
        v.extend(self.foreign.iter().cloned());
        for a in &self.craft {
            v.push(a.ident.full.clone());
            v.push(a.ident.short.clone());
        }
        v
    }
    fn new_ac(
        &mut self,
        kind: FleetFlight,
        now: f64,
        station: usize,
        stage: Stage,
        body: Body,
        ident: Identity,
    ) -> Ac {
        let id = self.next_id;
        self.next_id += 1;
        self.stats.spawned += 1;
        let mut rng = Rng::new(self.seed ^ u64::from(id).wrapping_mul(0x9e37_79b9));
        let squawk = format!(
            "{}{}{}{}",
            1 + rng.below(6),
            rng.below(8),
            rng.below(8),
            rng.below(8)
        );
        let cruise_ft = 4000 + 1000 * rng.below(2);
        Ac {
            id,
            ident,
            body,
            kind,
            stage,
            busy: false,
            freq: self.field.st(station).channel_khz,
            station,
            squawk,
            alt_ft: None,
            speed_kt: None,
            hdg_deg: None,
            cleared: false,
            touch_go: false,
            call_at: now,
            rng,
            cruise_ft,
            laps_left: 0,
            exit: [0.0, 0.0],
            remove_at: None,
            gone: false,
            dep_hdg: 0.0,
            speed_cmd_at: f64::MIN,
            contacted: false,
            transit: None,
            corridor: false,
            order: 0.0,
        }
    }

    fn spawn(&mut self, kind: FleetFlight, now: f64) -> bool {
        let f = self.field.clone();
        let taken = self.taken();
        let bias = match kind {
            FleetFlight::Pattern => 0.0,
            _ => 0.15,
        };
        let ident = identity(&mut self.rng, &taken, bias);
        let spec = *ident.spec();
        match kind {
            FleetFlight::Arrival => {
                for _ in 0..5 {
                    let Some(entry) = f
                        .entries
                        .get(self.rng.below(f.entries.len().max(1) as u32) as usize)
                    else {
                        return false;
                    };
                    let out = bearing(f.center, entry.pos);
                    let reach = self.rng.range(26.0, 32.0) * NM;
                    let start = if dist(f.center, entry.pos) < reach {
                        offset(f.center, out, reach)
                    } else {
                        entry.pos
                    };
                    let mut path = vec![entry.pos, f.iaf, f.faf, f.thr];
                    if dist(start, entry.pos) < 500.0 {
                        path.remove(0);
                    }
                    let first = path.first().copied().unwrap_or(f.thr);
                    let mut body = Body::new(start, 0.0, bearing(start, first), spec.cruise_kt);
                    body.path = path;
                    let eta = eta_profile(&spec, body.path_remaining(), None);
                    // A new arrival joins the end of the landing sequence. A faster aircraft that
                    // would land ahead of a slower one already on the route would have to overtake
                    // it, which controllers never allow on a common path.
                    let gap = SPAWN_GAP_S + self.rng.range(0.0, 60.0);
                    if self.last_eta().is_some_and(|last| eta < last + gap) {
                        continue;
                    }
                    // Nobody else flies within 8 NM of the entry point at the arrival altitudes.
                    if self.craft.iter().any(|o| {
                        !o.gone
                            && !o.body.on_ground
                            && dist(o.body.pos, start) < 8.0 * NM
                            && (2800.0 * FT..=6200.0 * FT).contains(&o.body.alt_m)
                    }) {
                        continue;
                    }
                    let order = self.craft.iter().map(|o| o.order).fold(0.0, f64::max) + 1.0;
                    let mut ac = self.new_ac(kind, now, f.hi, Stage::Inbound, body, ident.clone());
                    ac.order = order;
                    ac.body.alt_m = f64::from(ac.cruise_ft) * FT;
                    ac.body.tgt_alt_m = ac.body.alt_m;
                    ac.call_at = now + ac.rng.range(2.0, 25.0);
                    self.craft.push(ac);
                    return true;
                }
                false
            }
            FleetFlight::Departure => {
                if self
                    .craft
                    .iter()
                    .any(|a| a.kind == kind && a.stage <= Stage::AwaitTakeoff && a.stage >= Stage::Hold)
                {
                    return false;
                }
                let pos = offset(f.thr, f.rh, -45.0);
                let mut body = Body::new(pos, f.elev_m, f.rh, 0.0);
                body.on_ground = true;
                body.nav = Nav::Roll(f.rh);
                body.tgt_spd = 0.0;
                let mut ac = self.new_ac(kind, now, f.tower, Stage::Hold, body, ident);
                ac.call_at = now + ac.rng.range(20.0, 80.0);
                let away = if f.left_pattern { 1.0 } else { -1.0 };
                ac.dep_hdg = f.rh + away * ac.rng.range(0.1, 0.5);
                self.craft.push(ac);
                true
            }
            FleetFlight::FlightFollowing => {
                for _ in 0..6 {
                    // With another busy airport in the region, about half the flights are transits
                    // between the two: a track parallel to the line joining the fields and 9 NM
                    // off it, so it clears both circuits.
                    let mut transit = None;
                    let (a, z) = if !self.neighbours.is_empty() && self.rng.chance(0.55) {
                        let pick = self.rng.below(self.neighbours.len() as u32) as usize;
                        let (index, centre) = self.neighbours.get(pick).copied().unwrap_or((0, f.center));
                        let axis = bearing(f.center, centre);
                        let side = if self.rng.chance(0.5) { 1.0 } else { -1.0 };
                        let shift =
                            |p: [f64; 2]| offset(p, axis + side * core::f64::consts::FRAC_PI_2, 9.0 * NM);
                        transit = Some(index);
                        (
                            shift(offset(f.center, axis + core::f64::consts::PI, 30.0 * NM)),
                            shift(offset(centre, axis, 30.0 * NM)),
                        )
                    } else {
                        let b = self.rng.range(0.0, core::f64::consts::TAU);
                        let turn = self.rng.range(2.2, 4.0);
                        let radius = 34.0 * NM;
                        (offset(f.center, b, radius), offset(f.center, b + turn, radius))
                    };
                    // Closest approach of the track to the field must clear the circuit by 6 NM.
                    let (dx, dy) = (z[0] - a[0], z[1] - a[1]);
                    let t = (((f.center[0] - a[0]) * dx + (f.center[1] - a[1]) * dy) / (dx * dx + dy * dy))
                        .clamp(0.0, 1.0);
                    let close = dist(f.center, [a[0] + t * dx, a[1] + t * dy]);
                    if close < 6.0 * NM {
                        continue;
                    }
                    // VFR cruising altitudes (14 CFR 91.159): odd thousands plus 500 eastbound.
                    let course = f.mag_deg(bearing(a, z)) % 360;
                    let eastbound = course < 180;
                    let level = if transit.is_some() {
                        5 + self.rng.below(8)
                    } else {
                        6 + self.rng.below(4)
                    };
                    let level = if (level % 2 == 1) == eastbound {
                        level
                    } else {
                        level + 1
                    };
                    let alt = level * 1000 + 500;
                    // One flight following per altitude: tracks across the area are random and cross.
                    if self
                        .craft
                        .iter()
                        .any(|o| !o.gone && o.kind == kind && o.cruise_ft == alt)
                    {
                        continue;
                    }
                    let mut body = Body::new(a, f64::from(alt) * FT, bearing(a, z), spec.cruise_kt);
                    body.path = vec![z];
                    let mut ac = self.new_ac(kind, now, f.hi, Stage::FfCall, body, ident.clone());
                    ac.cruise_ft = alt;
                    ac.exit = z;
                    ac.transit = transit;
                    ac.corridor = transit.is_some();
                    ac.call_at = now + ac.rng.range(3.0, 20.0);
                    self.craft.push(ac);
                    return true;
                }
                false
            }
            FleetFlight::Pattern => {
                let d1 = f.rw(f.length_m + 900.0, PATTERN_Y);
                let d2 = f.rw(-2200.0, PATTERN_Y);
                // Joins the downwind only with the circuit clear around the entry point.
                let entry = offset(d1, bearing(d1, d2), 600.0);
                if self.craft.iter().any(|o| {
                    !o.gone
                        && !o.body.on_ground
                        && dist(o.body.pos, entry) < 3.0 * NM
                        && (o.body.alt_m - (f.elev_m + 305.0)).abs() < 400.0
                }) {
                    return false;
                }
                let mut body = Body::new(
                    offset(d1, bearing(d1, d2), 600.0),
                    f.elev_m + 305.0,
                    bearing(d1, d2),
                    circuit_kt(&spec, false),
                );
                body.path = vec![d2];
                let mut ac = self.new_ac(kind, now, f.tower, Stage::PatCall, body, ident);
                ac.laps_left = 3;
                ac.call_at = now + ac.rng.range(2.0, 10.0);
                ac.contacted = true;
                self.craft.push(ac);
                true
            }
        }
    }

    // ---------------------------------------------------------------- flight

    fn fly(&mut self, i: usize, now: f64, dt: f64) {
        let f = self.field.clone();
        let Some(a) = self.craft.get_mut(i) else { return };
        if a.gone {
            return;
        }
        let spec = *a.ident.spec();
        let rem = a.body.path_remaining();
        let alt_cmd = a.alt_ft.map(|ft| f64::from(ft) * FT);
        match a.kind {
            FleetFlight::Arrival => {
                if a.stage < Stage::Rollout {
                    let mut kt = profile_kt(&spec, rem);
                    if let Some(k) = a.speed_kt {
                        kt = kt.min(f64::from(k)).max(spec.min_kt);
                    }
                    a.body.tgt_spd = kt * KT;
                    let mut alt = alt_cmd.unwrap_or(f64::from(a.cruise_ft) * FT);
                    if a.cleared {
                        alt = alt.min(f.elev_m + 15.0 + rem * 0.0524);
                    }
                    a.body.tgt_alt_m = alt;
                }
            }
            FleetFlight::FlightFollowing => {
                a.body.tgt_spd = spec.cruise_kt * KT;
                a.body.tgt_alt_m = f64::from(a.cruise_ft) * FT;
            }
            FleetFlight::Departure => {
                if a.body.on_ground {
                    if a.stage == Stage::Rolling {
                        a.body.tgt_spd = (spec.rotate_kt + 12.0) * KT;
                    }
                } else {
                    a.body.tgt_spd = (spec.rotate_kt + 15.0).max(0.78 * spec.cruise_kt) * KT;
                    a.body.tgt_alt_m = alt_cmd.unwrap_or(f.elev_m + 450.0);
                }
            }
            FleetFlight::Pattern => {
                if a.stage < Stage::Rollout || a.stage >= Stage::PatCall {
                    a.body.tgt_spd = circuit_kt(&spec, a.cleared) * KT;
                    a.body.tgt_alt_m = if a.cleared {
                        (f.elev_m + 8.0 + rem * 0.07).min(f.elev_m + 305.0)
                    } else {
                        f.elev_m + 305.0
                    };
                    if let Some(k) = a.speed_kt {
                        a.body.tgt_spd = a.body.tgt_spd.min(f64::from(k) * KT);
                    }
                }
            }
        }
        a.body.step(&spec, dt);
        // Liftoff.
        if a.kind == FleetFlight::Departure && a.body.on_ground && a.body.spd >= spec.rotate_kt * KT {
            a.body.on_ground = false;
            a.body.nav = Nav::Heading(f.rh);
            a.body.tgt_alt_m = f.elev_m + 450.0;
        }
        // Touchdown or touch-and-go.
        let landing = matches!(a.kind, FleetFlight::Arrival | FleetFlight::Pattern)
            && a.cleared
            && !a.body.on_ground
            && a.body.path.is_empty()
            && f.along(a.body.pos) > -200.0;
        if landing {
            a.cleared = false;
            if a.touch_go {
                a.touch_go = false;
                a.body.alt_m = f.elev_m + 1.0;
                a.body.tgt_alt_m = f.elev_m + 305.0;
                a.body.nav = Nav::Path;
                a.body.path = vec![
                    f.rw(f.length_m + 900.0, 0.0),
                    f.rw(f.length_m + 900.0, PATTERN_Y),
                    f.rw(-2200.0, PATTERN_Y),
                ];
                a.laps_left = a.laps_left.saturating_sub(1);
                a.stage = Stage::PatClimb;
            } else {
                a.body.on_ground = true;
                a.body.alt_m = f.elev_m;
                a.body.vs = 0.0;
                a.body.nav = Nav::Roll(f.rh);
                a.body.tgt_spd = 0.0;
                a.body.braking = true;
                a.speed_kt = None;
                a.stage = Stage::Rollout;
            }
        }
        if a.stage == Stage::PatClimb && a.body.path.len() <= 1 {
            a.stage = Stage::PatCall;
            a.call_at = now + 1.0;
        }
        if a.stage == Stage::Exiting {
            a.body.braking = false;
            if a.remove_at.is_some_and(|t| now >= t) {
                a.gone = true;
                self.stats.completed += 1;
            }
        }
    }

    /// Last-resort physics guard; counts every use so tests can prove the controllers keep it at zero.
    fn safety_net(&mut self, dt: f64) {
        let n = self.craft.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let (Some(a), Some(b)) = (self.craft.get(i), self.craft.get(j)) else {
                    continue;
                };
                if a.body.on_ground || b.body.on_ground || a.gone || b.gone {
                    continue;
                }
                if dist(a.body.pos, b.body.pos) < 1.3 * NM && (a.body.alt_m - b.body.alt_m).abs() < 140.0 {
                    let a_behind = {
                        let to = bearing(b.body.pos, a.body.pos);
                        wrap_pi(to - b.body.hdg).abs() > core::f64::consts::FRAC_PI_2
                    };
                    let slow = if a_behind { i } else { j };
                    let label = format!(
                        "{:?}/{} x {:?}/{}",
                        a.kind,
                        a.stage.label(),
                        b.kind,
                        b.stage.label()
                    );
                    let note = format!(
                        "t={:.0} {:?}/{} and {:?}/{} at {:.2} NM, {:.0} m apart vertically",
                        self.clock,
                        a.kind,
                        a.stage.label(),
                        b.kind,
                        b.stage.label(),
                        dist(a.body.pos, b.body.pos) / NM,
                        (a.body.alt_m - b.body.alt_m).abs()
                    ) + &format!(
                        " | A id{} {} {:.0}kt rem {:.1}NM eta {:.0} lim {:?} alt {:.0}ft | B id{} {} {:.0}kt rem {:.1}NM eta {:.0} lim {:?} alt {:.0}ft",
                        a.id,
                        a.ident.spec().spoken,
                        a.body.spd / KT,
                        a.body.path_remaining() / NM,
                        self.eta(a),
                        a.speed_kt,
                        a.body.alt_m / FT,
                        b.id,
                        b.ident.spec().spoken,
                        b.body.spd / KT,
                        b.body.path_remaining() / NM,
                        self.eta(b),
                        b.speed_kt,
                        b.body.alt_m / FT
                    );
                    if let Some(s) = self.craft.get_mut(slow) {
                        s.body.spd = (s.body.spd - 2.0 * dt).max(20.0);
                    }
                    self.stats.safety_slowdowns += 1;
                    let clock = self.clock;
                    self.guard_pairs.entry(label).or_insert((0, clock)).0 += 1;
                    if self.guard_note.is_empty() {
                        self.guard_note = note;
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- controllers

    /// Bounds-safe access by position; an unknown position reads a retired placeholder.
    fn ar(&self, i: usize) -> &Ac {
        self.craft.get(i).unwrap_or(&self.scratch)
    }
    fn a(&mut self, i: usize) -> &mut Ac {
        self.craft.get_mut(i).unwrap_or(&mut self.scratch)
    }
    fn pc(&self, i: usize, full: bool) -> PilotCallsign {
        self.ar(i).ident.pilot_callsign(!full)
    }
    fn cs(&self, i: usize) -> String {
        self.craft.get(i).map_or(String::new(), |a| a.ident.short.clone())
    }
    fn enqueue(&mut self, mut m: Msg) {
        self.seq += 1;
        m.seq = self.seq;
        self.stats.transmissions += 1;
        self.chans.entry(m.freq).or_default().queue.push(m);
    }
    fn workload(&self, freq: u32) -> usize {
        self.chans
            .get(&freq)
            .map_or(0, |c| c.queue.len() + usize::from(c.inflight.is_some()))
    }
    fn alt_ft(a: &Ac) -> f64 {
        a.body.alt_m / FT
    }

    fn say_pilot(&mut self, i: usize, st: usize, text: String, next: Stage, now: f64) {
        let Some(a) = self.craft.get_mut(i) else { return };
        a.busy = true;
        a.contacted = true;
        let (id, freq) = (a.id, a.freq);
        let ex = self.next_ex;
        self.next_ex += 1;
        self.enqueue(Msg {
            ready: now + 0.5,
            seq: 0,
            ac: id,
            ex,
            kind: FleetCallKind::CheckIn,
            role: FleetRole::Pilot,
            station: st,
            text,
            instr: Vec::new(),
            next,
            sequence: None,
            freq,
            corrected: false,
            repeat: false,
        });
    }
    fn say_ctrl(
        &mut self,
        i: usize,
        prefix: &str,
        instr: Vec<AtcInstruction>,
        next: Stage,
        sequence: Option<FleetSequence>,
        now: f64,
    ) {
        let Some(a) = self.craft.get_mut(i) else { return };
        a.busy = true;
        let (id, freq, st, alt) = (a.id, a.freq, a.station, a.body.alt_m / FT);
        let body = instr
            .iter()
            .map(|x| capital(&render_instruction(x, alt, &self.field.stations, &[])))
            .collect::<Vec<_>>()
            .join(" ");
        let text = format!("{}, {prefix}{body}", self.cs(i));
        let ex = self.next_ex;
        self.next_ex += 1;
        let ready = now + self.rng.range(0.8, 2.2);
        self.enqueue(Msg {
            ready,
            seq: 0,
            ac: id,
            ex,
            kind: FleetCallKind::Instruction,
            role: FleetRole::Controller,
            station: st,
            text,
            instr,
            next,
            sequence,
            freq,
            corrected: false,
            repeat: false,
        });
    }
    fn say_info(&mut self, i: usize, text: &str, kind: FleetCallKind, next: Stage, now: f64) {
        let Some(a) = self.craft.get_mut(i) else { return };
        a.busy = true;
        let (id, freq, st) = (a.id, a.freq, a.station);
        let text = format!("{}, {text}", self.cs(i));
        let ex = self.next_ex;
        self.next_ex += 1;
        let ready = now + self.rng.range(0.8, 2.0);
        self.enqueue(Msg {
            ready,
            seq: 0,
            ac: id,
            ex,
            kind,
            role: FleetRole::Controller,
            station: st,
            text,
            instr: Vec::new(),
            next,
            sequence: None,
            freq,
            corrected: false,
            repeat: false,
        });
    }
    fn handoff(&self, st: usize) -> AtcInstruction {
        let s = self.field.st(st);
        AtcInstruction::Handoff {
            station_id: s.id.clone(),
            channel_khz: s.channel_khz,
        }
    }

    fn sequence_remark(&self, id: u32) -> (String, Option<FleetSequence>) {
        let (leader, number) = self.leader_of(id);
        let Some(l) = leader.and_then(|l| self.idx(l)).and_then(|l| self.craft.get(l)) else {
            return (String::new(), None);
        };
        if number < 2 {
            return (String::new(), None);
        }
        let rem = l.body.path_remaining();
        let position = if l.kind == FleetFlight::Pattern {
            let leg = if l.body.path.len() <= 1 { "final" } else { "base" };
            let side = if self.field.left_pattern { "left" } else { "right" };
            if leg == "base" {
                format!("on {side} base")
            } else {
                phraseology::sequence_position("final", Some(((rem / NM).round() as u32).max(1)))
            }
        } else if rem < 14.0 * NM {
            phraseology::sequence_position("final", Some(((rem / NM).round() as u32).max(1)))
        } else {
            return (String::new(), None);
        };
        (
            format!(
                "{} ",
                phraseology::sequence(number, l.ident.spec().spoken, &position)
            ),
            Some(FleetSequence {
                number,
                follows: l.ident.full.clone(),
            }),
        )
    }

    fn decide(&mut self, i: usize, now: f64) {
        let f = self.field.clone();
        let Some(a) = self.craft.get(i) else { return };
        if a.gone || a.busy || self.workload(a.freq) >= 3 {
            return;
        }
        let (stage, kind, id) = (a.stage, a.kind, a.id);
        let rem = a.body.path_remaining();
        let pos = a.body.pos;
        let alt = Self::alt_ft(a);
        let info = self.info.clone();
        let spec = *a.ident.spec();
        let rwy = f.runway.clone();
        // Spacing: slow down behind the sequenced leader; resume once the gap opens.
        if (kind == FleetFlight::Arrival && (Stage::HiDescent..=Stage::Landing).contains(&stage))
            || (kind == FleetFlight::Pattern && stage == Stage::PatFly)
        {
            let (leader, _) = self.leader_of(id);
            if let Some(l) = leader.and_then(|l| self.idx(l)).and_then(|l| self.craft.get(l)) {
                let me = self.ar(i);
                let leader_eta = self.eta(l);
                // The time gap by predicted arrival, and by distance along the common path at the
                // leader's present speed: the closer of the two governs, since a slower aircraft's
                // predicted arrival hides how near it is now.
                let mut gap = self.eta(me) - leader_eta;
                if l.body.path_remaining() > 0.0 && rem > 0.0 {
                    gap = gap.min((rem - l.body.path_remaining()) / l.body.spd.max(20.0));
                }
                let free_gap = self.eta_with(me, None) - leader_eta;
                let restricted = me.speed_kt;
                let since = now - me.speed_cmd_at;
                if gap < SPEED_TRIGGER_S && since > 30.0 {
                    // The fastest assigned speed (steps of five knots) that opens the gap to the target.
                    let top = (profile_kt(&spec, rem) / 5.0).floor() * 5.0;
                    let bottom = (spec.min_kt / 5.0).ceil() * 5.0;
                    let mut knots = top;
                    while knots > bottom && self.eta_with(me, Some(knots)) - leader_eta < SPEED_TARGET_S {
                        knots -= 5.0;
                    }
                    let knots = knots.max(bottom) as u16;
                    if restricted.is_none_or(|r| knots + 5 <= r) {
                        self.a(i).speed_cmd_at = now;
                        self.say_ctrl(
                            i,
                            "",
                            vec![AtcInstruction::Speed { knots: Some(knots) }],
                            stage,
                            None,
                            now,
                        );
                        return;
                    }
                }
                // Already as slow as the aircraft can fly and still too close, on the tower frequency:
                // send the follower around rather than let the runway conflict develop.
                let bottom = (spec.min_kt / 5.0).ceil() * 5.0;
                if gap < GO_AROUND_S
                    && stage >= Stage::AwaitTower
                    && restricted.is_some_and(|r| f64::from(r) <= bottom)
                    && rem > 1.5 * NM
                    && rem < 3.5 * NM
                {
                    self.say_ctrl(
                        i,
                        "",
                        vec![AtcInstruction::GoAround { runway: rwy }],
                        Stage::PatClimb,
                        None,
                        now,
                    );
                    return;
                }
                if free_gap > SPEED_CANCEL_S && since > 45.0 && restricted.is_some() {
                    self.a(i).speed_cmd_at = now;
                    self.say_ctrl(
                        i,
                        "",
                        vec![AtcInstruction::Speed { knots: None }],
                        stage,
                        None,
                        now,
                    );
                    return;
                }
            }
        }

        match stage {
            Stage::Inbound if now >= self.ar(i).call_at => {
                let pc = self.pc(i, true);
                let text = phraseology::pilot_request(
                    f.st(f.hi),
                    &pc,
                    &format!(
                        "{}, inbound for landing",
                        phraseology::altitude((alt / 100.0).round() as u32 * 100)
                    ),
                    Some(&info),
                );
                self.say_pilot(i, f.hi, text, Stage::AwaitHi, now);
            }
            Stage::AwaitHi => {
                let squawk = self.ar(i).squawk.clone();
                let target = self.ar(i).cruise_ft.min(4000);
                self.say_ctrl(
                    i,
                    &format!("{} ", phraseology::radar_identified()),
                    vec![
                        AtcInstruction::Squawk { code: squawk },
                        AtcInstruction::Altitude { altitude_ft: target },
                    ],
                    Stage::HiDescent,
                    None,
                    now,
                );
            }
            Stage::HiDescent if dist(pos, f.center) <= 22.0 * NM => {
                let h = self.handoff(f.lo);
                self.say_ctrl(i, "", vec![h], Stage::LoCall, None, now);
            }
            Stage::LoCall if now >= self.ar(i).call_at => {
                let pc = self.pc(i, false);
                let text = format!(
                    "{}, {}, {}.",
                    phraseology::facility(f.st(f.lo)),
                    pc.current,
                    phraseology::altitude((alt / 100.0).round() as u32 * 100)
                );
                self.say_pilot(i, f.lo, text, Stage::AwaitLo, now);
            }
            Stage::AwaitLo => {
                self.say_ctrl(
                    i,
                    &format!("{} ", phraseology::radar_identified()),
                    vec![AtcInstruction::Altitude { altitude_ft: 3000 }],
                    Stage::Lo,
                    None,
                    now,
                );
            }
            Stage::Lo if rem <= 11.5 * NM => {
                let (remark, seq) = self.sequence_remark(id);
                let approach = AtcInstruction::Approach {
                    airport: f.airport.clone(),
                    procedure: f.procedure.clone(),
                    name: f.family.clone(),
                    runway: rwy,
                    intercept: None,
                };
                let h = self.handoff(f.tower);
                self.say_ctrl(i, &remark, vec![approach, h], Stage::TowerCall, seq, now);
            }
            Stage::TowerCall if now >= self.ar(i).call_at => {
                let pc = self.pc(i, false);
                let miles = ((rem / NM).round() as u32).clamp(1, 12);
                let text =
                    phraseology::pilot_position(f.st(f.tower), &pc, &rwy, &phraseology::miles_final(miles));
                self.say_pilot(i, f.tower, text, Stage::AwaitTower, now);
            }
            Stage::AwaitTower => {
                let clear_rw = !self.craft.iter().any(|o| {
                    o.id != id
                        && matches!(o.stage, Stage::Rolling)
                        && f.along(o.body.pos) < f.length_m + 300.0
                }) && !self
                    .craft
                    .iter()
                    .any(|o| o.id != id && o.stage == Stage::PatClimb && f.along(o.body.pos) < f.length_m);
                if rem <= 5.0 * NM && clear_rw {
                    self.say_ctrl(
                        i,
                        "",
                        vec![AtcInstruction::Land { runway: rwy }],
                        Stage::Landing,
                        None,
                        now,
                    );
                } else if rem <= 1.3 * NM {
                    self.say_ctrl(
                        i,
                        "",
                        vec![AtcInstruction::GoAround { runway: rwy }],
                        Stage::PatClimb,
                        None,
                        now,
                    );
                }
            }
            Stage::Rollout if self.ar(i).body.spd <= 12.0 => {
                if let Some(g) = f.ground {
                    let khz = f.st(g).channel_khz;
                    let station = f.st(g).id.clone();
                    let _ = station;
                    self.say_ctrl(
                        i,
                        "",
                        vec![AtcInstruction::ExitRunway {
                            runway: rwy,
                            ground_khz: khz,
                        }],
                        Stage::Exiting,
                        None,
                        now,
                    );
                } else {
                    self.a(i).stage = Stage::Exiting;
                    self.a(i).remove_at = Some(now + 30.0);
                }
            }
            // Departures.
            Stage::Hold if now >= self.ar(i).call_at => {
                let pc = self.pc(i, true);
                let text = phraseology::pilot_position(f.st(f.tower), &pc, &rwy, "ready for departure");
                self.say_pilot(i, f.tower, text, Stage::AwaitTakeoff, now);
            }
            Stage::AwaitTakeoff if self.can_release(now) => {
                self.last_release = now;
                // Departures that are still close behind each other fly diverging headings (at least
                // fifteen degrees apart), so a faster aircraft cannot run up the track of a slower one.
                let away = if f.left_pattern { 1.0 } else { -1.0 };
                let others: Vec<f64> = self
                    .craft
                    .iter()
                    .filter(|o| {
                        !o.gone && o.id != id && o.kind == FleetFlight::Departure && o.stage >= Stage::Rolling
                    })
                    .map(|o| o.dep_hdg)
                    .collect();
                let mut best = (f64::MIN, 0.12);
                for offset in [0.12, 0.30, 0.48, 0.66, 0.84] {
                    let hdg = f.rh + away * offset;
                    let spread = others
                        .iter()
                        .map(|h| wrap_pi(hdg - h).abs())
                        .fold(f64::MAX, f64::min)
                        .min(0.5);
                    if spread > best.0 + 1e-9 {
                        best = (spread, offset);
                    }
                }
                self.a(i).dep_hdg = f.rh + away * best.1;
                self.say_ctrl(
                    i,
                    "",
                    vec![AtcInstruction::Takeoff {
                        runway: rwy,
                        closed_traffic: false,
                    }],
                    Stage::Rolling,
                    None,
                    now,
                );
            }
            Stage::Rolling if !self.ar(i).body.on_ground && alt - f.elev_m / FT > 350.0 => {
                let h = self.handoff(f.dep);
                self.say_ctrl(i, "", vec![h], Stage::DepCall, None, now);
            }
            Stage::DepCall if now >= self.ar(i).call_at => {
                let pc = self.pc(i, true);
                let text = phraseology::pilot_request(
                    f.st(f.dep),
                    &pc,
                    &format!(
                        "passing {}",
                        phraseology::altitude((alt / 100.0).round() as u32 * 100)
                    ),
                    None,
                );
                self.say_pilot(i, f.dep, text, Stage::AwaitDep, now);
            }
            Stage::AwaitDep => {
                let squawk = self.ar(i).squawk.clone();
                let heading = f.mag_deg(self.ar(i).dep_hdg);
                self.say_ctrl(
                    i,
                    &format!("{} ", phraseology::radar_identified()),
                    vec![
                        AtcInstruction::Squawk { code: squawk },
                        AtcInstruction::Vector {
                            heading_deg: heading,
                            altitude_ft: 4000,
                        },
                    ],
                    Stage::Climb,
                    None,
                    now,
                );
            }
            Stage::Climb if alt >= 2700.0 => {
                let h = self.handoff(f.hi);
                self.say_ctrl(i, "", vec![h], Stage::HiCall2, None, now);
            }
            Stage::HiCall2 if now >= self.ar(i).call_at => {
                let pc = self.pc(i, false);
                let text = format!(
                    "{}, {}, passing {} for four thousand.",
                    phraseology::facility(f.st(f.hi)),
                    pc.current,
                    phraseology::altitude((alt / 100.0).round() as u32 * 100)
                );
                self.say_pilot(i, f.hi, text, Stage::AwaitHi2, now);
            }
            Stage::AwaitHi2 => {
                self.say_ctrl(
                    i,
                    &format!("{} ", phraseology::radar_identified()),
                    vec![AtcInstruction::Altitude { altitude_ft: 5000 }],
                    Stage::Out,
                    None,
                    now,
                );
            }
            Stage::Out if dist(pos, f.center) >= 24.0 * NM => {
                self.say_info(
                    i,
                    phraseology::radar_service_terminated(),
                    FleetCallKind::Terminate,
                    Stage::Done,
                    now,
                );
            }
            // Flight following.
            Stage::FfCall if now >= self.ar(i).call_at => {
                let pc = self.pc(i, true);
                let text = phraseology::pilot_request(
                    f.st(f.hi),
                    &pc,
                    &format!(
                        "{}, VFR, request flight following",
                        phraseology::altitude(self.ar(i).cruise_ft)
                    ),
                    None,
                );
                self.say_pilot(i, f.hi, text, Stage::AwaitFf, now);
            }
            Stage::AwaitFf => {
                let squawk = self.ar(i).squawk.clone();
                self.say_ctrl(
                    i,
                    &format!("{} ", phraseology::radar_identified()),
                    vec![AtcInstruction::Squawk { code: squawk }],
                    Stage::Enroute,
                    None,
                    now,
                );
            }
            // A transit flight past the midpoint to the next busy airport is handed over to it.
            Stage::Enroute
                if self
                    .ar(i)
                    .transit
                    .and_then(|k| self.neighbours.iter().find(|n| n.0 == k))
                    .is_some_and(|n| dist(pos, n.1) < dist(pos, f.center)) =>
            {
                self.say_info(
                    i,
                    phraseology::radar_service_terminated(),
                    FleetCallKind::Terminate,
                    Stage::Done,
                    now,
                );
            }
            Stage::Enroute if dist(pos, self.ar(i).exit) <= 6.0 * NM => {
                self.say_info(
                    i,
                    phraseology::radar_service_terminated(),
                    FleetCallKind::Terminate,
                    Stage::Done,
                    now,
                );
            }
            // Circuit.
            Stage::PatCall if now >= self.ar(i).call_at => {
                let pc = self.pc(i, !self.ar(i).contacted);
                let side = if f.left_pattern { "left" } else { "right" };
                let text = phraseology::pilot_position(f.st(f.tower), &pc, &rwy, &format!("{side} downwind"));
                self.say_pilot(i, f.tower, text, Stage::AwaitPat, now);
            }
            Stage::AwaitPat if now >= self.ar(i).call_at => {
                let (x_now, y_now) = (f.along(pos), f.cross(pos));
                let _ = y_now;
                let hyp = self.eta(self.ar(i));
                let conflict = self
                    .landers()
                    .iter()
                    .any(|l| {
                        l.0 != id
                            && if l.1 < hyp {
                                hyp - l.1 < CIRCUIT_AHEAD_S
                            } else {
                                !self.can_open_behind(l.0, l.1 - hyp)
                            }
                    })
                    // Circuit aircraft are cleared in the order they fly: never ahead of one still waiting.
                    || self.craft.iter().any(|o| {
                        o.id != id
                            && !o.gone
                            && o.kind == FleetFlight::Pattern
                            && matches!(o.stage, Stage::PatCall | Stage::AwaitPat)
                            && self.eta(o) < hyp
                    })
                    || self.runway_occupied()
                    || self.craft.iter().any(|o| o.id != id && o.stage == Stage::Rolling);
                if conflict {
                    if x_now > -1500.0 || self.ar(i).speed_cmd_at == f64::MIN {
                        self.a(i).speed_cmd_at = now;
                        self.a(i).call_at = now + 15.0;
                        self.say_ctrl(
                            i,
                            "",
                            vec![AtcInstruction::ExtendDownwind],
                            Stage::AwaitPat,
                            None,
                            now,
                        );
                    } else {
                        self.a(i).call_at = now + 10.0;
                    }
                } else {
                    let (remark, seq) = {
                        // The circuit aircraft is not yet in the landing order; describe its leader by eta.
                        let landers = self.landers();
                        let ahead = landers.iter().rfind(|l| l.1 < hyp).copied();
                        match ahead.and_then(|l| self.idx(l.0)).and_then(|l| self.craft.get(l)) {
                            Some(l) => {
                                let position = phraseology::sequence_position(
                                    "final",
                                    Some(((l.body.path_remaining() / NM).round() as u32).max(1)),
                                );
                                let number = landers.iter().filter(|x| x.1 < hyp).count() as u8 + 1;
                                (
                                    format!(
                                        "{} ",
                                        phraseology::sequence(number, l.ident.spec().spoken, &position)
                                    ),
                                    Some(FleetSequence {
                                        number,
                                        follows: l.ident.full.clone(),
                                    }),
                                )
                            }
                            None => (String::new(), None),
                        }
                    };
                    let last = self.ar(i).laps_left <= 1;
                    let ins = if last {
                        AtcInstruction::Land { runway: rwy }
                    } else {
                        AtcInstruction::TouchAndGo { runway: rwy }
                    };
                    self.say_ctrl(i, &remark, vec![ins], Stage::PatFly, seq, now);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- effects

    fn apply(&mut self, i: usize, ins: &AtcInstruction, now: f64) {
        let f = self.field.clone();
        let station_index = |id: &str| f.stations.iter().position(|s| s.id == id);
        // A circuit aircraft cleared to land takes its place in the landing sequence, and keeps it.
        let sequenced = (matches!(
            ins,
            AtcInstruction::Land { .. } | AtcInstruction::TouchAndGo { .. }
        ) && self.ar(i).kind == FleetFlight::Pattern)
            .then(|| self.order_for(self.ar(i).id, self.eta(self.ar(i))));
        let Some(a) = self.craft.get_mut(i) else { return };
        match ins {
            AtcInstruction::Altitude { altitude_ft } => a.alt_ft = Some(*altitude_ft),
            AtcInstruction::Vector {
                heading_deg,
                altitude_ft,
            } => {
                a.hdg_deg = Some(*heading_deg);
                a.alt_ft = Some(*altitude_ft);
                a.body.nav = Nav::Heading(f.grid_from_mag(*heading_deg));
            }
            AtcInstruction::Squawk { code } => a.squawk = code.clone(),
            AtcInstruction::Handoff {
                station_id,
                channel_khz,
            } => {
                if let Some(index) = station_index(station_id) {
                    a.station = index;
                }
                a.freq = *channel_khz;
                a.call_at = now + a.rng.range(2.0, 5.0);
            }
            AtcInstruction::Approach { .. } => a.cleared = true,
            AtcInstruction::Takeoff { .. } => {
                a.body.nav = Nav::Roll(f.rh);
                a.body.braking = false;
            }
            AtcInstruction::Land { .. } | AtcInstruction::TouchAndGo { .. } => {
                a.cleared = true;
                a.touch_go = matches!(ins, AtcInstruction::TouchAndGo { .. });
                if let Some(order) = sequenced {
                    a.order = order;
                }
                if a.kind == FleetFlight::Pattern {
                    let x = f.along(a.body.pos);
                    a.body.nav = Nav::Path;
                    a.body.path = if x > -2000.0 {
                        vec![f.rw(-2200.0, PATTERN_Y), f.rw(-2200.0, 0.0), f.thr]
                    } else {
                        vec![f.rw(x - 300.0, 0.0), f.thr]
                    };
                }
            }
            AtcInstruction::GoAround { .. } => {
                a.kind = FleetFlight::Pattern;
                a.cleared = false;
                a.touch_go = false;
                a.laps_left = 1;
                a.speed_kt = None;
                a.alt_ft = None;
                a.body.nav = Nav::Path;
                a.body.path = vec![
                    f.rw(f.length_m + 900.0, 0.0),
                    f.rw(f.length_m + 900.0, PATTERN_Y),
                    f.rw(-2200.0, PATTERN_Y),
                ];
            }
            AtcInstruction::Speed { knots } => a.speed_kt = *knots,
            AtcInstruction::ExitRunway { .. } => {
                let away = if f.left_pattern { -1.0 } else { 1.0 };
                a.body.nav = Nav::Heading(f.rh + away * 1.0);
                a.body.tgt_spd = 5.0;
                a.body.braking = false;
                a.remove_at = Some(now + 35.0);
            }
            _ => {}
        }
    }

    fn corrupt(ins: &AtcInstruction, rng: &mut Rng) -> Option<AtcInstruction> {
        Some(match ins {
            AtcInstruction::Altitude { altitude_ft } => AtcInstruction::Altitude {
                altitude_ft: if *altitude_ft > 2000 {
                    altitude_ft - 1000
                } else {
                    altitude_ft + 1000
                },
            },
            AtcInstruction::Vector {
                heading_deg,
                altitude_ft,
            } => AtcInstruction::Vector {
                heading_deg: (heading_deg % 360) + 10,
                altitude_ft: *altitude_ft,
            },
            AtcInstruction::Squawk { code } => {
                let mut c: Vec<char> = code.chars().collect();
                let last = c.pop()?;
                let d = last.to_digit(8)?;
                c.push(char::from_digit((d + 1 + rng.below(6)) % 8, 8)?);
                AtcInstruction::Squawk {
                    code: c.into_iter().collect(),
                }
            }
            AtcInstruction::Handoff {
                station_id,
                channel_khz,
            } => AtcInstruction::Handoff {
                station_id: station_id.clone(),
                channel_khz: channel_khz + 100,
            },
            AtcInstruction::Speed { knots: Some(k) } => AtcInstruction::Speed { knots: Some(k + 10) },
            _ => return None,
        })
    }

    // ---------------------------------------------------------------- the frequency

    fn finish(&mut self, now: f64) {
        let mut done = Vec::new();
        let keys: Vec<u32> = self.chans.keys().copied().collect();
        for k in keys {
            let gap = self.rng.range(1.5, 3.5);
            if let Some(c) = self.chans.get_mut(&k)
                && c.inflight.as_ref().is_some_and(|(end, _)| *end <= now + 1e-9)
                && let Some((end, m)) = c.inflight.take()
            {
                c.quiet_until = end + gap;
                done.push(m);
            }
        }
        for m in done {
            self.complete(m, now);
        }
    }

    fn complete(&mut self, m: Msg, now: f64) {
        let Some(i) = self.idx(m.ac) else { return };
        match (m.role, m.kind) {
            (FleetRole::Pilot, FleetCallKind::Readback) => {
                if self.readback_ok(i, &m) {
                    for ins in &m.instr {
                        self.apply(i, ins, now);
                    }
                    if let Some(a) = self.craft.get_mut(i) {
                        a.busy = false;
                        a.stage = m.next;
                    }
                    self.stats.readbacks_accepted += 1;
                    if m.corrected {
                        self.stats.readbacks_corrected += 1;
                    }
                } else {
                    let phrases = self.render_all(i, &m.instr);
                    let text = format!("{}, negative, {phrases}", self.cs(i));
                    let ready = now + self.rng.range(0.6, 1.4);
                    self.enqueue(Msg {
                        ready,
                        kind: FleetCallKind::Correction,
                        role: FleetRole::Controller,
                        text,
                        corrected: true,
                        seq: 0,
                        ..m
                    });
                }
            }
            (FleetRole::Pilot, _) => {
                if let Some(a) = self.craft.get_mut(i) {
                    a.busy = false;
                    a.stage = m.next;
                }
            }
            (FleetRole::Controller, FleetCallKind::Instruction | FleetCallKind::Correction) => {
                let mistaken = !m.corrected && self.craft.get_mut(i).is_some_and(|a| a.rng.chance(0.07));
                let mut spoken = m.instr.clone();
                if mistaken {
                    let mut rng = self.rng.clone();
                    for slot in spoken.iter_mut() {
                        if let Some(bad) = Self::corrupt(slot, &mut rng) {
                            *slot = bad;
                            break;
                        }
                    }
                    self.rng = rng;
                }
                let alt = self.craft.get(i).map_or(0.0, Self::alt_ft);
                let bodies = spoken
                    .iter()
                    .map(|x| render_readback(x, alt, &self.field.stations))
                    .collect::<Vec<_>>()
                    .join(", ");
                let text = format!("{bodies}, {}.", self.cs(i));
                let ready = now + self.rng.range(1.0, 2.6);
                self.enqueue(Msg {
                    ready,
                    kind: FleetCallKind::Readback,
                    role: FleetRole::Pilot,
                    text,
                    seq: 0,
                    ..m
                });
            }
            (FleetRole::Controller, FleetCallKind::Terminate) => {
                if let Some(a) = self.craft.get_mut(i) {
                    a.gone = true;
                    a.stage = Stage::Done;
                    if let Some(next) = a.transit {
                        self.handoffs.push((next, a.clone()));
                    }
                }
                self.stats.completed += 1;
            }
            (FleetRole::Controller, _) => {
                if let Some(a) = self.craft.get_mut(i) {
                    a.busy = false;
                    a.stage = m.next;
                }
            }
        }
    }

    fn render_all(&self, i: usize, instr: &[AtcInstruction]) -> String {
        let alt = self.craft.get(i).map_or(0.0, Self::alt_ft);
        instr
            .iter()
            .map(|x| capital(&render_instruction(x, alt, &self.field.stations, &[])))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The same checker the user's readbacks pass through.
    fn readback_ok(&self, i: usize, m: &Msg) -> bool {
        let message = parse_pilot(&m.text, &self.pc(i, false));
        m.instr.iter().all(|x| readback_matches(x, &message))
    }

    /// What the listener hears: a repeat after the user stepped on the call says so first.
    fn spoken_text(m: &Msg) -> String {
        match (m.repeat, m.role) {
            (false, _) => m.text.clone(),
            (true, FleetRole::Controller) => format!("Blocked, I say again. {}", m.text),
            (true, FleetRole::Pilot) => format!("Blocked, say again. {}", m.text),
        }
    }
    fn call_for(&self, m: &Msg, freq: u32) -> Option<FleetCall> {
        let i = self.idx(m.ac)?;
        let a = self.ar(i);
        let st = self.field.st(m.station);
        Some(FleetCall {
            exchange: m.ex,
            aircraft: a.id,
            callsign: a.ident.full.clone(),
            role: m.role,
            kind: m.kind,
            instructions: m.instr.clone(),
            facility: st.id.clone(),
            frequency_khz: freq,
            voice: if m.role == FleetRole::Pilot {
                a.ident.voice
            } else {
                facility_voice(&self.field.stations, m.station)
            },
            for_user: false,
            sequence: m.sequence.clone(),
        })
    }

    /// The user keyed over the transmission in flight on each of `blocked`: the call did not get
    /// through, so it is sent again after the usual gap with a short "blocked, say again". Its
    /// aircraft stays waiting, so nothing the call carried takes effect until it is heard.
    pub fn repeat_blocked(&mut self, blocked: &[u32]) {
        for k in blocked {
            let gap = self.rng.range(1.5, 3.5);
            let Some(c) = self.chans.get_mut(k) else { continue };
            let Some((end, mut m)) = c.inflight.take() else {
                continue;
            };
            c.quiet_until = end + gap;
            c.event = None;
            m.repeat = true;
            m.ready = end + gap;
            self.stats.stepped_on += 1;
            self.seq += 1;
            m.seq = self.seq;
            self.stats.transmissions += 1;
            if let Some(c) = self.chans.get_mut(k) {
                c.queue.push(m);
            }
        }
    }

    pub fn start_tx(&mut self, now: f64, busy: &[u32]) -> Vec<Tx> {
        let mut out = Vec::new();
        let keys: Vec<u32> = self.chans.keys().copied().collect();
        for k in keys {
            let alive: Vec<u32> = self.craft.iter().map(|a| a.id).collect();
            if let Some(c) = self.chans.get_mut(&k) {
                c.queue.retain(|m| alive.contains(&m.ac));
            }
            let Some(c) = self.chans.get(&k) else { continue };
            if c.inflight.is_some() || now < c.quiet_until || busy.contains(&k) {
                continue;
            }
            // Audio plays while the call is on the air, so the carrier alone paces the frequency.
            let Some(pick) = c
                .queue
                .iter()
                .enumerate()
                .filter(|(_, m)| m.ready <= now)
                .min_by_key(|(_, m)| (m.priority(), m.seq))
                .map(|(n, _)| n)
            else {
                continue;
            };
            let Some(mut m) = self.chans.get_mut(&k).map(|c| c.queue.remove(pick)) else {
                continue;
            };
            let text = Self::spoken_text(&m);
            m.repeat = false;
            let Some(call) = self.call_for(&m, k) else {
                continue;
            };
            let duration = fleet_call_duration(&text, &call);
            if let Some(c) = self.chans.get_mut(&k) {
                c.inflight = Some((now + duration, m.clone()));
                c.event = None;
            }
            let Some(i) = self.idx(m.ac) else { continue };
            let a = self.ar(i);
            out.push(Tx {
                freq: k,
                station: m.station,
                role: m.role,
                text,
                call,
                origin: (m.role == FleetRole::Pilot).then(|| [a.body.pos[0], a.body.pos[1], -a.body.alt_m]),
                callsign: a.ident.full.clone(),
            });
        }
        out
    }
}
