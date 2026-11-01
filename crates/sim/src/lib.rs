//! Deterministic, headless 6-DOF flight simulation.
//!
//! See the crate README for start envelopes, trim, contact, crash, and replay semantics.
//!
//! Frames: the world frame is local north-east-down (NED) with its origin at the threshold of
//! runway 36 on the default airfield; the body frame is x forward, y right, z down. Units are SI
//! throughout (metres, seconds, radians, kilograms, newtons); conversions to knots and feet live in
//! the presentation layers only.
//!
//! Determinism: a fixed timestep ([`DT_S`]), no wall clock, no threads, no hash maps in the
//! stepping path, and transcendental maths only through `libm`. The same inputs give the same bits
//! on every platform, checked by [`Sim::state_hash`] in the golden tests.

#![deny(clippy::disallowed_methods)]

pub mod autopilot;
mod dynamics;
mod math;

pub use flightcraft_aero::{Aircraft, AircraftError, ControlSurface, Failures};
use flightcraft_world as world;
pub use flightcraft_world::{TurbulenceLevel, Weather, World, WorldChoice};
use math::*;
use serde::{Deserialize, Serialize};

/// The fixed simulation timestep: 1/120 s.
pub const DT_S: f64 = 1.0 / 120.0;

/// Pilot controls. Each value is clamped to its range when set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Controls {
    /// Elevator, -1 (nose down) to 1 (nose up).
    pub elevator: f64,
    /// Aileron, -1 (roll left) to 1 (roll right).
    pub aileron: f64,
    /// Rudder, -1 (yaw left) to 1 (yaw right).
    pub rudder: f64,
    /// Throttle, 0 to 1.
    pub throttle: f64,
    /// Flaps, 0 (up) to 1 (fully down).
    pub flaps: f64,
    /// Wheel brakes, 0 to 1.
    pub brake: f64,
}

impl Controls {
    /// The same controls with every value clamped to its range; a non-finite value becomes 0.
    #[must_use]
    pub fn clamped(self) -> Self {
        fn c(v: f64, lo: f64, hi: f64) -> f64 {
            if v.is_finite() { v.clamp(lo, hi) } else { 0.0 }
        }
        Self {
            elevator: c(self.elevator, -1.0, 1.0),
            aileron: c(self.aileron, -1.0, 1.0),
            rudder: c(self.rudder, -1.0, 1.0),
            throttle: c(self.throttle, 0.0, 1.0),
            flaps: c(self.flaps, 0.0, 1.0),
            brake: c(self.brake, 0.0, 1.0),
        }
    }
}

/// The full rigid-body state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct State {
    /// Simulated time since the start, seconds.
    pub time_s: f64,
    /// Position of the centre of gravity in the world (NED) frame, metres.
    pub position_m: [f64; 3],
    /// Velocity in the world (NED) frame, m/s.
    pub velocity_mps: [f64; 3],
    /// Attitude, body to world, as a unit quaternion `[w, x, y, z]`.
    pub attitude: [f64; 4],
    /// Angular rate in the body frame `[p, q, r]`, rad/s.
    pub angular_rate_rps: [f64; 3],
}

impl Default for State {
    fn default() -> Self {
        Self {
            time_s: 0.0,
            position_m: [0.0; 3],
            velocity_mps: [0.0; 3],
            attitude: [1.0, 0.0, 0.0, 0.0],
            angular_rate_rps: [0.0; 3],
        }
    }
}

/// Derived values for instruments, agents, and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Readout {
    pub time_s: f64,
    /// Indicated airspeed, m/s.
    pub indicated_airspeed_mps: f64,
    /// True airspeed, m/s.
    pub true_airspeed_mps: f64,
    /// Ground speed, m/s.
    pub ground_speed_mps: f64,
    /// Altitude above mean sea level, metres.
    pub altitude_msl_m: f64,
    /// Height of the lowest wheel above the ground, metres.
    pub height_agl_m: f64,
    /// Climb rate, m/s (positive up).
    pub vertical_speed_mps: f64,
    /// Heading, radians, 0 to 2π, clockwise from north.
    pub heading_rad: f64,
    pub pitch_rad: f64,
    pub roll_rad: f64,
    /// Heading rate about the local vertical, rad/s, positive right.
    /// Zero within 1e-6 rad of vertical pitch, where heading is undefined.
    pub turn_rate_rps: f64,
    /// Aerodynamic specific force along body y, g, positive right.
    /// A slip ball moves opposite this force; coordinated flight reads zero.
    pub lateral_acceleration_g: f64,
    /// Angle of attack, radians.
    pub alpha_rad: f64,
    /// Sideslip, radians.
    pub beta_rad: f64,
    /// Load factor along body z, g (1 in level flight).
    pub load_factor_g: f64,
    /// Engine speed, revolutions per minute.
    pub engine_rpm: f64,
    /// True while any wheel touches the ground.
    pub on_ground: bool,
    /// True once the aircraft has hit the ground too hard or off a gear contact.
    pub crashed: bool,
    pub controls: Controls,
    /// Engaged holds and their SI targets.
    pub autopilot: autopilot::Targets,
    pub weather: Weather,
    /// Total moving-air velocity at the CG, NED m/s.
    pub wind_ned_mps: [f64; 3],
    pub gust_ned_mps: [f64; 3],
    pub failures: Failures,
    /// Actual surface positions after applying jams (throttle remains a demand).
    pub effective_controls: Controls,
}

/// How a simulation starts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Start {
    /// Parked on the runway threshold, engine idling, pointing down the runway.
    Runway,
    /// In the air: height above the runway threshold (m), airspeed (m/s), and heading (rad).
    Air {
        height_m: f64,
        airspeed_mps: f64,
        heading_rad: f64,
    },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SimError {
    #[error(transparent)]
    Aircraft(#[from] AircraftError),
    #[error(transparent)]
    Terrain(#[from] world::TerrainError),
    #[error("invalid start: {0}")]
    InvalidStart(String),
    #[error("unknown aircraft: {0}")]
    UnknownAircraft(String),
    #[error("invalid autopilot targets: {0}")]
    InvalidAutopilot(String),
    #[error(transparent)]
    Weather(#[from] world::WeatherError),
    #[error(transparent)]
    Failure(#[from] flightcraft_aero::FailureError),
}

/// One simulation: an aircraft, a world, and its state.
#[derive(Clone, Debug)]
pub struct Sim {
    aircraft: Aircraft,
    world: World,
    state: State,
    controls: Controls,
    steps: u64,
    crashed: bool,
    autopilot: autopilot::Autopilot,
    wind: world::Wind,
    failures: Failures,
}

impl Sim {
    /// A new simulation of the default aircraft at `start`.
    pub fn new(start: &Start) -> Result<Self, SimError> {
        Self::new_with_weather(start, Weather::default())
    }

    /// Start with weather and its seed. An air start has the requested airspeed;
    /// its ground velocity includes the initial mean wind. Gust filters start at zero.
    pub fn new_with_weather(start: &Start, weather: Weather) -> Result<Self, SimError> {
        Self::new_with_aircraft_and_weather(start, Aircraft::corvid()?, weather)
    }

    /// Start with a validated custom aircraft and calm weather.
    pub fn new_with_aircraft(start: &Start, aircraft: Aircraft) -> Result<Self, SimError> {
        Self::new_with_aircraft_and_weather(start, aircraft, Weather::default())
    }

    pub fn new_with_aircraft_and_weather(
        start: &Start,
        aircraft: Aircraft,
        weather: Weather,
    ) -> Result<Self, SimError> {
        Self::new_with_aircraft_weather_world(start, aircraft, weather, World::default())
    }

    pub fn new_with_world(start: &Start, world: World) -> Result<Self, SimError> {
        Self::new_with_aircraft_weather_world(start, Aircraft::corvid()?, Weather::default(), world)
    }

    pub fn new_with_aircraft_weather_world(
        start: &Start,
        aircraft: Aircraft,
        weather: Weather,
        world: World,
    ) -> Result<Self, SimError> {
        aircraft.validate()?;
        let wind = world::Wind::new(weather)?;
        let mut state = State::default();
        let controls = match *start {
            Start::Runway => {
                // Unequal wheel heights encode a tail-down (or nose-down) parked attitude.
                // Equal-height gear retains the original level initialization exactly.
                let pitch = dynamics::runway_pitch(&aircraft);
                if pitch != 0.0 {
                    state.attitude = attitude(pitch, 0.0);
                }
                state.position_m = [
                    0.0,
                    0.0,
                    -(aircraft
                        .geometry
                        .gear
                        .iter()
                        .map(|g| {
                            let point = aircraft.relative_point(g.point_m);
                            let [_, _, z] = if pitch == 0.0 {
                                point
                            } else {
                                rotate(state.attitude, point)
                            };
                            z
                        })
                        .fold(0.0, f64::max)
                        - aircraft.mass.mass_kg * world::GRAVITY_MPS2
                            / aircraft
                                .geometry
                                .gear
                                .iter()
                                .map(|g| g.stiffness_n_m)
                                .sum::<f64>()),
                ];
                Controls::default()
            }
            Start::Air {
                height_m,
                airspeed_mps,
                heading_rad,
            } => {
                if !height_m.is_finite()
                    || !(2.0..=20_000.0).contains(&height_m)
                    || !airspeed_mps.is_finite()
                    || !(1.0..=150.0).contains(&airspeed_mps)
                    || !heading_rad.is_finite()
                {
                    return Err(SimError::InvalidStart(
                        "finite height in 2..=20000 m and speed in 1..=150 m/s required".into(),
                    ));
                }
                let heading = heading_rad.rem_euclid(core::f64::consts::TAU);
                let (alpha, controls) = dynamics::trim(&aircraft, height_m, airspeed_mps);
                state.position_m = [0.0, 0.0, -height_m];
                state.velocity_mps = [
                    airspeed_mps * libm::cos(heading),
                    airspeed_mps * libm::sin(heading),
                    0.0,
                ];
                state.attitude = attitude(alpha, heading);
                controls
            }
        };
        if matches!(start, Start::Air { .. }) && wind.velocity_mps(state.position_m) != [0.0; 3] {
            state.velocity_mps = add(state.velocity_mps, wind.velocity_mps(state.position_m));
        }
        if !dynamics::valid(&state) {
            return Err(SimError::InvalidStart(
                "aircraft initialization outside finite simulation envelope".into(),
            ));
        }
        let mut sim = Self {
            aircraft,
            world,
            wind,
            failures: Failures::default(),
            state,
            controls,
            steps: 0,
            crashed: false,
            autopilot: autopilot::Autopilot::default(),
        };
        sim.controls = sim.limit_controls(sim.controls);
        Ok(sim)
    }

    /// Restore a finite state for experiments and replay. Quaternion is normalised.
    /// Time must be nonnegative, positions within 100,000 km, body rates within 50 rad/s,
    /// and each velocity component within 500 m/s. Out-of-envelope integration freezes
    /// the last valid state and marks the simulation crashed.
    pub fn from_state(state: State, controls: Controls) -> Result<Self, SimError> {
        Self::from_state_in_world(state, controls, World::default())
    }

    pub fn from_state_in_world(mut state: State, controls: Controls, world: World) -> Result<Self, SimError> {
        if !dynamics::valid(&state) || state.time_s > 1e12 {
            return Err(SimError::InvalidStart(
                "state outside finite simulation envelope".into(),
            ));
        }
        let [w, x, y, z] = state.attitude;
        if w * w + x * x + y * y + z * z < 1e-12 {
            return Err(SimError::InvalidStart("zero attitude quaternion".into()));
        }
        state.attitude = normalise(state.attitude);
        let steps = libm::round(state.time_s / DT_S) as u64;
        state.time_s = steps as f64 * DT_S;
        let aircraft = Aircraft::corvid()?;
        let crashed = dynamics::crash(&aircraft, &state, &world);
        Ok(Self {
            aircraft,
            world,
            wind: world::Wind::new(Weather::default())?,
            failures: Failures::default(),
            state,
            controls: controls.clamped(),
            steps,
            crashed,
            autopilot: autopilot::Autopilot::default(),
        })
    }

    #[must_use]
    pub fn aircraft(&self) -> &Aircraft {
        &self.aircraft
    }

    #[must_use]
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Replace weather atomically and restart the gust process at its supplied seed.
    /// In-flight changes preserve ground velocity; only an air start adds mean wind.
    pub fn set_weather(&mut self, weather: Weather) -> Result<(), SimError> {
        self.wind = world::Wind::new(weather)?;
        Ok(())
    }

    #[must_use]
    pub fn weather(&self) -> Weather {
        self.wind.weather()
    }

    pub fn set_failures(&mut self, failures: Failures) -> Result<(), SimError> {
        failures.validate()?;
        self.failures = failures;
        Ok(())
    }

    #[must_use]
    pub fn failures(&self) -> Failures {
        self.failures
    }

    #[must_use]
    pub fn effective_controls(&self) -> Controls {
        let c = self
            .aircraft
            .limit_controls(self.failures.apply(dynamics::aero_controls(self.controls)));
        Controls {
            elevator: c.elevator,
            aileron: c.aileron,
            rudder: c.rudder,
            flaps: c.flaps,
            ..self.controls
        }
    }

    /// Set the controls (clamped). They hold until set again.
    pub fn set_controls(&mut self, controls: Controls) {
        self.controls = self.limit_controls(controls);
    }

    fn limit_controls(&self, controls: Controls) -> Controls {
        let controls = controls.clamped();
        let c = self.aircraft.limit_controls(dynamics::aero_controls(controls));
        let [lo, hi] = self.aircraft.systems.brake;
        Controls {
            elevator: c.elevator,
            aileron: c.aileron,
            rudder: c.rudder,
            throttle: c.throttle,
            flaps: c.flaps,
            brake: controls.brake.clamp(lo, hi),
        }
    }

    #[must_use]
    pub fn controls(&self) -> Controls {
        self.controls
    }

    /// Replace the engaged holds atomically. Incompatible or hostile targets are errors.
    pub fn set_autopilot(&mut self, targets: autopilot::Targets) -> Result<(), SimError> {
        targets.validate()?;
        self.autopilot.set(targets);
        Ok(())
    }

    /// Disengage all holds. The last controls remain available to the pilot.
    pub fn autopilot_off(&mut self) {
        self.autopilot = autopilot::Autopilot::default();
    }

    /// Advance up to `n` fixed steps of [`DT_S`], stopping at a crash or numerical envelope exit.
    /// The last finite state is retained. Crashed simulations remain frozen.
    pub fn step(&mut self, n: u32) {
        for _ in 0..n {
            if self.crashed {
                break;
            }
            if self.autopilot.targets().engaged() {
                let readout = self.readout();
                self.controls = self
                    .autopilot
                    .update(&self.aircraft, readout, self.state.angular_rate_rps);
            }
            self.controls = self.limit_controls(self.controls);
            let relative = dynamics::relative_velocity(&self.state, &self.wind);
            // Advance exactly once; substages and observational reads never consume noise.
            if self.wind.advance(DT_S, relative).is_err() {
                self.crashed = true;
                break;
            }
            let Some(mut next) = dynamics::rk4(
                &self.aircraft,
                self.state,
                self.controls,
                &self.wind,
                self.failures,
                &self.world,
            ) else {
                self.crashed = true;
                break;
            };
            if !dynamics::valid(&next) {
                self.crashed = true;
                break;
            }
            self.steps = self.steps.saturating_add(1);
            next.time_s = self.steps as f64 * DT_S;
            self.crashed = dynamics::crash(&self.aircraft, &next, &self.world)
                || dynamics::hard_touchdown(&self.aircraft, &self.state, &next, &self.world);
            self.state = next;
        }
    }

    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }

    #[must_use]
    pub fn readout(&self) -> Readout {
        let s = &self.state;
        let [_, _, z] = s.position_m;
        let [vn, ve, vd] = s.velocity_mps;
        let [roll, pitch, heading] = euler(s.attitude);
        let [u, v, w] = dynamics::airflow(s, &self.wind).velocity_body_mps;
        let speed = norm(dynamics::relative_velocity(s, &self.wind));
        let loads = dynamics::loads(&self.aircraft, s, self.controls, &self.wind, self.failures);
        let [_, fy, fz] = loads.force_n;
        let [_, q, r] = s.angular_rate_rps;
        let cos_pitch = libm::cos(pitch);
        let h = dynamics::height(&self.aircraft, s, &self.world);
        Readout {
            time_s: s.time_s,
            indicated_airspeed_mps: speed
                * libm::sqrt(world::atmosphere(-z).density_kg_m3 / world::SEA_LEVEL_DENSITY),
            true_airspeed_mps: speed,
            ground_speed_mps: libm::sqrt(vn * vn + ve * ve),
            altitude_msl_m: -z,
            height_agl_m: h.max(0.0),
            vertical_speed_mps: -vd,
            heading_rad: heading,
            pitch_rad: pitch,
            roll_rad: roll,
            turn_rate_rps: if cos_pitch.abs() > 1e-6 {
                (q * libm::sin(roll) + r * libm::cos(roll)) / cos_pitch
            } else {
                0.0
            },
            lateral_acceleration_g: fy / (self.aircraft.mass.mass_kg * world::GRAVITY_MPS2),
            alpha_rad: libm::atan2(w, u),
            beta_rad: if speed > 0.01 {
                libm::asin((v / speed).clamp(-1.0, 1.0))
            } else {
                0.0
            },
            load_factor_g: -fz / (self.aircraft.mass.mass_kg * world::GRAVITY_MPS2),
            engine_rpm: loads.engine_rpm,
            on_ground: h <= 0.0,
            crashed: self.crashed,
            controls: self.controls,
            autopilot: self.autopilot.targets(),
            weather: self.weather(),
            wind_ned_mps: self.wind.velocity_mps(s.position_m),
            gust_ned_mps: self.wind.gust_mps(),
            failures: self.failures,
            effective_controls: self.effective_controls(),
        }
    }

    /// FNV-1a over rigid-body state, crash flag, and step count, plus nondefault
    /// weather/process memory and failures. Equal on every platform for the same inputs.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let s = &self.state;
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u64| {
            for b in v.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        eat(self.steps);
        eat(u64::from(self.crashed));
        for v in [s.time_s]
            .iter()
            .chain(&s.position_m)
            .chain(&s.velocity_mps)
            .chain(&s.attitude)
            .chain(&s.angular_rate_rps)
        {
            eat(v.to_bits());
        }
        // Preserve the original hash stream for the default calm, healthy scenario.
        let weather = self.weather();
        if weather != Weather::default() || self.failures != Failures::default() {
            eat(0x7765_6174_6865_7201);
            eat(weather.wind_from_deg.to_bits());
            eat(weather.wind_speed_mps.to_bits());
            eat(u64::from(weather.shear));
            eat(match weather.turbulence {
                TurbulenceLevel::Off => 0,
                TurbulenceLevel::Light => 1,
                TurbulenceLevel::Moderate => 2,
                TurbulenceLevel::Severe => 3,
            });
            eat(weather.seed);
            for word in self.wind.hash_words() {
                eat(word);
            }
            eat(self.failures.engine_power_fraction.to_bits());
            for jam in [
                self.failures.elevator,
                self.failures.aileron,
                self.failures.rudder,
                self.failures.flaps,
            ] {
                eat(u64::from(jam.is_some()));
                if let Some(position) = jam {
                    eat(position.to_bits());
                }
            }
        }
        if !self.world.tiles().is_empty() {
            eat(0x7465_7272_6169_6e01);
            eat(self.world.fingerprint());
        }
        h
    }
}
