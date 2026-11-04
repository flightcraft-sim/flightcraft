//! Simulated aircraft: types, fictional callsigns, and a bounded kinematic model.
//! Everything here is pure and deterministic: no clock, no hash iteration, no I/O.
use super::*;

pub(super) const KT: f64 = 0.514_444;
pub(super) const FT: f64 = 0.3048;
pub(super) const NM: f64 = 1852.0;

/// SplitMix64: a tiny, well-mixed, platform-independent generator.
#[derive(Clone, Debug)]
pub(crate) struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut r = Self(seed ^ 0x9e37_79b9_7f4a_7c15);
        r.next();
        r
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            (self.next() % u64::from(n)) as u32
        }
    }
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    Light,
    Twin,
    Turboprop,
    Jet,
}

/// Performance limits for the kinematic model. Speeds are terminal-area values in knots.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TypeSpec {
    pub spoken: &'static str,
    pub designator: &'static str,
    #[allow(dead_code)]
    pub class: Class,
    pub cruise_kt: f64,
    pub approach_kt: f64,
    pub min_kt: f64,
    pub rotate_kt: f64,
    pub climb_fpm: f64,
    pub descent_fpm: f64,
    pub turn_dps: f64,
    pub accel_kt_s: f64,
    pub roll_mps2: f64,
}
const fn spec(
    spoken: &'static str,
    designator: &'static str,
    class: Class,
    [cruise_kt, approach_kt, min_kt, rotate_kt]: [f64; 4],
    [climb_fpm, descent_fpm]: [f64; 2],
    [turn_dps, accel_kt_s, roll_mps2]: [f64; 3],
) -> TypeSpec {
    TypeSpec {
        spoken,
        designator,
        class,
        cruise_kt,
        approach_kt,
        min_kt,
        rotate_kt,
        climb_fpm,
        descent_fpm,
        turn_dps,
        accel_kt_s,
        roll_mps2,
    }
}
pub(super) const GA_COUNT: usize = 9;
pub(super) const TYPES: [TypeSpec; 12] = [
    spec(
        "Cessna",
        "C172",
        Class::Light,
        [110., 75., 70., 55.],
        [700., 800.],
        [3., 1., 1.4],
    ),
    spec(
        "Skyhawk",
        "C172",
        Class::Light,
        [105., 75., 70., 55.],
        [700., 800.],
        [3., 1., 1.4],
    ),
    spec(
        "Cherokee",
        "PA28",
        Class::Light,
        [115., 76., 70., 55.],
        [650., 800.],
        [3., 1., 1.4],
    ),
    spec(
        "Cirrus",
        "SR22",
        Class::Light,
        [155., 82., 78., 70.],
        [1000., 1000.],
        [3., 1.5, 1.8],
    ),
    spec(
        "Bonanza",
        "BE36",
        Class::Light,
        [160., 82., 78., 65.],
        [1000., 1200.],
        [3., 1.5, 1.8],
    ),
    spec(
        "Mooney",
        "M20P",
        Class::Light,
        [150., 80., 75., 65.],
        [900., 1000.],
        [3., 1.5, 1.7],
    ),
    spec(
        "Diamond",
        "DA40",
        Class::Light,
        [125., 78., 72., 58.],
        [800., 900.],
        [3., 1.2, 1.5],
    ),
    spec(
        "Skylane",
        "C182",
        Class::Light,
        [135., 78., 72., 60.],
        [900., 900.],
        [3., 1.2, 1.6],
    ),
    spec(
        "Baron",
        "BE58",
        Class::Twin,
        [180., 90., 85., 72.],
        [1600., 1500.],
        [3., 2., 2.2],
    ),
    spec(
        "King Air",
        "BE20",
        Class::Turboprop,
        [200., 100., 95., 90.],
        [2000., 1800.],
        [3., 2., 2.6],
    ),
    spec(
        "Citation",
        "C525",
        Class::Jet,
        [210., 105., 100., 95.],
        [2500., 2000.],
        [3., 2.5, 3.0],
    ),
    spec(
        "Pilatus",
        "PC12",
        Class::Turboprop,
        [190., 98., 92., 80.],
        [1800., 1700.],
        [3., 2., 2.5],
    ),
];

/// Clearly fictional operator names. None is, or imitates, a real airline.
const OPERATORS: [&str; 5] = ["Tamarack", "Brindle", "Quillon", "Harrow", "Fenwick"];
const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";

#[derive(Clone, Debug)]
pub(crate) struct Identity {
    pub registration: String,
    /// Spoken in full, such as `Cessna Four Two Seven Kilo Papa`.
    pub full: String,
    /// The shortened form a pilot may use after the controller does.
    pub short: String,
    pub spec: usize,
    pub company: bool,
    pub voice: u8,
}
impl Identity {
    pub fn spec(&self) -> &'static TypeSpec {
        TYPES.get(self.spec).unwrap_or(&TYPES[0])
    }
    pub fn spoken_type(&self) -> &'static str {
        if self.company { "" } else { self.spec().spoken }
    }
    pub fn pilot_callsign(&self, abbreviated: bool) -> PilotCallsign {
        let current = if abbreviated { &self.short } else { &self.full };
        PilotCallsign {
            overridden: true,
            registration: self.registration.clone(),
            spoken_type: self.spoken_type().into(),
            not_us: self.company,
            full: current.clone(),
            abbreviated: self.short.clone(),
            current: current.clone(),
        }
    }
}

/// An FAA-format registration: `N`, then one to five characters, no leading zero, no `I` or `O`,
/// letters only at the end and at most two. Checked against the shared validator.
pub(super) fn n_number(rng: &mut Rng) -> String {
    for _ in 0..8 {
        let forms = [(3, 2), (4, 0), (3, 1), (2, 2), (4, 1), (1, 2)];
        let (digits, letters) = forms.get(rng.below(6) as usize).copied().unwrap_or((3, 2));
        let mut s = String::from("N");
        for i in 0..digits {
            let d = if i == 0 { 1 + rng.below(9) } else { rng.below(10) };
            s.push(char::from_digit(d, 10).unwrap_or('1'));
        }
        for _ in 0..letters {
            s.push(char::from(
                LETTERS
                    .get(rng.below(LETTERS.len() as u32) as usize)
                    .copied()
                    .unwrap_or(b'A'),
            ));
        }
        if validate_registration(&s, false).as_deref() == Ok(&s) {
            return s;
        }
    }
    "N1".into()
}

/// A new identity whose spoken forms collide with none in `taken`.
pub(super) fn identity(rng: &mut Rng, taken: &[String], company_bias: f64) -> Identity {
    for _ in 0..32 {
        let company = rng.chance(company_bias);
        let (registration, full, short, index) = if company {
            let name = OPERATORS
                .get(rng.below(OPERATORS.len() as u32) as usize)
                .copied()
                .unwrap_or("Harrow");
            let number = 10 + rng.below(890);
            let spoken = format!("{name} {}", spoken_registration(&number.to_string()));
            (
                format!(
                    "{}{number}",
                    name.to_uppercase().chars().take(3).collect::<String>()
                ),
                spoken.clone(),
                spoken,
                GA_COUNT + rng.below((TYPES.len() - GA_COUNT) as u32) as usize,
            )
        } else {
            let index = rng.below(GA_COUNT as u32) as usize;
            let registration = n_number(rng);
            let kind = TYPES.get(index).map_or("Cessna", |t| t.spoken);
            let (full, short) = phraseology::callsign_forms(&registration, kind);
            let index = if registration.len() == 7 && rng.chance(0.15) {
                8
            } else {
                index
            };
            (registration, full, short, index)
        };
        let clash = taken
            .iter()
            .any(|t| *t == full || *t == short || t.ends_with(&short) || short.ends_with(t.as_str()));
        if !clash {
            return Identity {
                registration,
                full,
                short,
                spec: index,
                company,
                voice: rng.below(8) as u8,
            };
        }
    }
    Identity {
        registration: "N1".into(),
        full: "Cessna One".into(),
        short: "Cessna One".into(),
        spec: 0,
        company: false,
        voice: 0,
    }
}

pub(super) fn wrap_pi(a: f64) -> f64 {
    let mut a = a % core::f64::consts::TAU;
    if a > core::f64::consts::PI {
        a -= core::f64::consts::TAU;
    } else if a < -core::f64::consts::PI {
        a += core::f64::consts::TAU;
    }
    a
}
pub(super) fn bearing(from: [f64; 2], to: [f64; 2]) -> f64 {
    libm::atan2(to[1] - from[1], to[0] - from[0])
}
pub(super) fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    libm::hypot(a[0] - b[0], a[1] - b[1])
}
pub(super) fn offset(p: [f64; 2], heading: f64, meters: f64) -> [f64; 2] {
    [
        p[0] + meters * libm::cos(heading),
        p[1] + meters * libm::sin(heading),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Nav {
    /// Fly a magnetic-converted grid heading.
    Heading(f64),
    /// Follow the waypoint path; with none left, hold the present heading.
    Path,
    /// Straight along the runway during the takeoff or landing roll.
    Roll(f64),
}

/// Kinematic state: position (grid north, east), altitude, heading, speed, vertical speed.
#[derive(Clone, Debug)]
pub(crate) struct Body {
    pub pos: [f64; 2],
    pub alt_m: f64,
    pub hdg: f64,
    pub spd: f64,
    pub vs: f64,
    pub on_ground: bool,
    pub nav: Nav,
    pub path: Vec<[f64; 2]>,
    pub tgt_alt_m: f64,
    pub tgt_spd: f64,
    /// Brake to a stop on the ground (landing rollout).
    pub braking: bool,
}
impl Body {
    pub fn new(pos: [f64; 2], alt_m: f64, hdg: f64, spd_kt: f64) -> Self {
        Self {
            pos,
            alt_m,
            hdg,
            spd: spd_kt * KT,
            vs: 0.0,
            on_ground: false,
            nav: Nav::Path,
            path: Vec::new(),
            tgt_alt_m: alt_m,
            tgt_spd: spd_kt * KT,
            braking: false,
        }
    }
    /// Remaining path length from the present position through every waypoint.
    pub fn path_remaining(&self) -> f64 {
        let mut at = self.pos;
        let mut total = 0.0;
        for wp in &self.path {
            total += dist(at, *wp);
            at = *wp;
        }
        total
    }
    fn desired_heading(&mut self, spec: &TypeSpec) -> f64 {
        match self.nav {
            Nav::Heading(h) | Nav::Roll(h) => h,
            Nav::Path => {
                // Fly-by turns: start the turn a turn radius early.
                let omega = spec.turn_dps.to_radians();
                while let Some(&next) = self.path.first() {
                    let to = bearing(self.pos, next);
                    let d = dist(self.pos, next);
                    let lead = match self.path.get(1) {
                        Some(&after) => {
                            let turn = wrap_pi(bearing(next, after) - to).abs();
                            let radius = self.spd.max(10.0) / omega;
                            (radius * libm::tan((turn / 2.0).min(1.3))).min(0.45 * dist(next, after))
                        }
                        None => 40.0,
                    };
                    if d <= lead.max(40.0) {
                        self.path.remove(0);
                        if self.path.is_empty() {
                            return self.hdg;
                        }
                        continue;
                    }
                    return to;
                }
                self.hdg
            }
        }
    }
    pub fn step(&mut self, spec: &TypeSpec, dt: f64) {
        let max_turn = spec.turn_dps.to_radians() * dt * if self.on_ground { 3.0 } else { 1.0 };
        let want = self.desired_heading(spec);
        self.hdg = (self.hdg + wrap_pi(want - self.hdg).clamp(-max_turn, max_turn))
            .rem_euclid(core::f64::consts::TAU);
        let accel = if self.on_ground {
            if self.braking { 2.2 } else { spec.roll_mps2 }
        } else {
            spec.accel_kt_s * KT
        };
        let dv = accel * dt;
        self.spd = (self.spd + (self.tgt_spd - self.spd).clamp(-dv * 1.5, dv)).max(0.0);
        self.pos = offset(self.pos, self.hdg, self.spd * dt);
        if self.on_ground {
            self.vs = 0.0;
            return;
        }
        let up = spec.climb_fpm * FT / 60.0;
        let down = spec.descent_fpm * FT / 60.0;
        let dz = self.tgt_alt_m - self.alt_m;
        let want_vs = (dz / dt.max(0.01)).clamp(-down, up);
        self.vs += (want_vs - self.vs).clamp(-2.0 * dt, 2.0 * dt);
        let step = self.vs * dt;
        if (dz > 0.0 && step >= dz) || (dz < 0.0 && step <= dz) {
            self.alt_m = self.tgt_alt_m;
            self.vs = 0.0;
        } else {
            self.alt_m += step;
        }
    }
}
