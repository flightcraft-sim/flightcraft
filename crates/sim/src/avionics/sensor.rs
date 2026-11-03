use crate::{DT_S, State, math, systems::SystemsState};
use schemars::JsonSchema;
use serde::Serialize;

/// Each unit has its own rechargeable battery. Endurance is at rated continuous load.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema, serde::Deserialize)]
pub struct Power {
    pub switch_on: bool,
    pub powered: bool,
    pub on_battery: bool,
    pub battery_remaining_s: f64,
    pub battery_endurance_s: f64,
}
impl Power {
    pub(crate) fn new(endurance: f64) -> Self {
        Self {
            switch_on: true,
            powered: false,
            on_battery: false,
            battery_remaining_s: endurance,
            battery_endurance_s: endurance,
        }
    }
    pub(crate) fn update(&mut self, bus: bool, advance: bool) {
        let dt = if advance { DT_S } else { 0. };
        self.on_battery = self.switch_on && !bus && self.battery_remaining_s > 1e-8;
        if self.on_battery {
            self.battery_remaining_s = (self.battery_remaining_s - dt).max(0.);
        } else if bus {
            self.battery_remaining_s = (self.battery_remaining_s + dt * 0.1).min(self.battery_endurance_s);
        }
        // Collapse sub-microsecond subtraction residue at the fixed-step endpoint.
        if self.battery_remaining_s < 1e-6 {
            self.battery_remaining_s = 0.;
        }
        self.powered = self.switch_on && (bus || self.battery_remaining_s > 1e-8);
        self.on_battery &= self.powered;
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Ahrs {
    pub failed: bool,
    pub alignment_s: f64,
    q: [f64; 4],
    seed: u64,
    initial_seed: u64,
}
impl Default for Ahrs {
    fn default() -> Self {
        Self::new(0x71b5_8a31)
    }
}
impl Ahrs {
    pub fn new(seed: u64) -> Self {
        Self {
            failed: false,
            alignment_s: 0.,
            q: [1., 0., 0., 0.],
            seed,
            initial_seed: seed,
        }
    }
    /// Seed a ready-for-departure source without advancing the simulation clock.
    pub(crate) fn align(&mut self, state: State) {
        self.q = state.attitude;
        self.alignment_s = ALIGNMENT_S;
    }
    pub fn hash_words(&self) -> Vec<u64> {
        let mut words = vec![
            u64::from(self.failed),
            self.alignment_s.to_bits(),
            self.seed,
            self.initial_seed,
        ];
        words.extend(self.q.map(f64::to_bits));
        words
    }
    fn noise(&mut self) -> f64 {
        // A seeded integer sensor process, independent of the weather noise stream.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        ((self.seed >> 11) as f64 / 9007199254740992. - 0.5) * 0.0002
    }
    pub fn update(&mut self, state: State, powered: bool, advance: bool) -> Option<[f64; 3]> {
        if !powered {
            self.alignment_s = 0.;
            self.seed = self.initial_seed;
            return None;
        }
        if self.failed {
            self.alignment_s = 0.;
            return None;
        }
        if !advance {
            return (self.alignment_s >= 45. - 1e-8).then(|| math::euler(self.q));
        }
        if self.alignment_s < 45. - 1e-8 {
            self.q = state.attitude;
            self.alignment_s = (self.alignment_s + DT_S).min(45.);
            if self.alignment_s < 45. - 1e-8 {
                return None;
            }
        } else {
            let [p, q, r] = state.angular_rate_rps;
            let delta = math::mul(self.q, [0., p + self.noise(), q + self.noise(), r + self.noise()]);
            for (value, rate) in self.q.iter_mut().zip(delta) {
                *value += rate * DT_S * 0.5;
            }
            self.q = math::normalise(self.q);
        }
        Some(math::euler(self.q))
    }
}

pub const ALIGNMENT_S: f64 = 45.;
#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema, serde::Deserialize)]
pub struct AirDataComputer {
    pub alignment_remaining_s: f64,
    pub powered: bool,
    pub attitude_valid: bool,
    pub ahrs_failed: bool,
    pub adc_failed: bool,
    pub air_data_valid: bool,
    pub roll_rad: f64,
    pub pitch_rad: f64,
    pub heading_rad: f64,
    pub slip_g: f64,
    pub indicated_airspeed_mps: f64,
    pub altitude_msl_m: f64,
    pub vertical_speed_mps: f64,
    pub airspeed_trend_mps: f64,
    pub altitude_trend_m: f64,
    pub true_airspeed_mps: f64,
    pub outside_temperature_c: f64,
    #[serde(skip)]
    pub(crate) sensor: Ahrs,
    previous_ias: Option<f64>,
}
/// Installation policy; pressure processing and AHRS integration share one implementation.
#[derive(Clone, Copy, Default)]
pub(crate) struct ComputerInput {
    pub powered: bool,
    pub ahrs_failed: bool,
    pub adc_failed: bool,
    pub advance: bool,
    pub hold_blocked: bool,
    pub baro_pa: Option<f64>,
}
impl AirDataComputer {
    pub(crate) fn with_seed(seed: u64) -> Self {
        Self {
            sensor: Ahrs::new(seed),
            ..Default::default()
        }
    }
    pub(crate) fn hash_words(&self) -> Vec<u64> {
        let mut words = self.sensor.hash_words();
        words.extend(
            [
                self.indicated_airspeed_mps,
                self.altitude_msl_m,
                self.vertical_speed_mps,
                self.airspeed_trend_mps,
                self.altitude_trend_m,
                self.true_airspeed_mps,
                self.outside_temperature_c,
            ]
            .map(f64::to_bits),
        );
        words.push(self.previous_ias.map_or(u64::MAX, f64::to_bits));
        words
    }

    /// Shared rate-integrating electronic AHRS, independent of vacuum gyros.
    /// ADC reads the existing pressure sensors, preserving trapped-port errors.
    pub(crate) fn step(
        &mut self,
        systems: &SystemsState,
        state: State,
        truth: crate::Readout,
        input: ComputerInput,
    ) {
        let ComputerInput {
            powered,
            ahrs_failed,
            adc_failed,
            advance,
            hold_blocked,
            baro_pa,
        } = input;
        self.powered = powered;
        self.ahrs_failed = ahrs_failed;
        self.adc_failed = adc_failed;
        let dt_s = if advance { DT_S } else { 0. };
        self.sensor.failed = ahrs_failed;
        let angles = self.sensor.update(state, powered, advance);
        self.alignment_remaining_s = (ALIGNMENT_S - self.sensor.alignment_s).max(0.);
        self.attitude_valid = angles.is_some();
        self.air_data_valid = powered && !adc_failed;
        if let Some([roll, pitch, heading]) = angles {
            self.roll_rad = roll;
            self.pitch_rad = pitch;
            self.heading_rad = (heading + truth.grid_north_true_rad - truth.magnetic_declination_rad)
                .rem_euclid(core::f64::consts::TAU);
        }
        self.slip_g = truth.lateral_acceleration_g;
        let s = systems.sensed;
        if !hold_blocked || !systems.failures.pitot_blocked || self.previous_ias.is_none() {
            self.indicated_airspeed_mps = s.indicated_airspeed_mps;
        }
        let ias = self.indicated_airspeed_mps;
        self.altitude_msl_m = baro_pa.map_or(s.altitude_msl_m, |setting| {
            crate::systems::altitude(systems.static_pressure_pa, setting)
        });
        self.vertical_speed_mps = s.vertical_speed_mps;
        if let Some(previous) = self.previous_ias.filter(|_| dt_s > 0.0) {
            let target = ((ias - previous) / dt_s * 6.0).clamp(-50.0, 50.0);
            self.airspeed_trend_mps += (target - self.airspeed_trend_mps) * (dt_s / 2.0).min(1.0);
        }
        if advance {
            self.previous_ias = powered.then_some(ias);
        }
        if hold_blocked && systems.failures.pitot_blocked {
            self.airspeed_trend_mps = 0.;
        }
        self.altitude_trend_m = s.vertical_speed_mps * 6.0;
        self.outside_temperature_c = systems.controls.temperature_c
            + flightcraft_world::atmosphere(systems.pressure_altitude_m).temperature_k
            - 288.15;
        let density = systems.static_pressure_pa
            / (flightcraft_world::GAS_CONSTANT * (self.outside_temperature_c + 273.15).max(100.0));
        self.true_airspeed_mps =
            s.indicated_airspeed_mps * libm::sqrt(flightcraft_world::SEA_LEVEL_DENSITY / density.max(0.01));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rated_endurance_off_state_and_recharge_do_not_share_energy() {
        for seconds in [1800_u32, 3600] {
            let mut power = Power::new(f64::from(seconds));
            power.update(true, true);
            assert!(power.powered && !power.on_battery);
            for _ in 0..seconds * 120 - 1 {
                power.update(false, true);
            }
            assert!(power.powered && power.on_battery);
            power.update(false, true);
            assert!(!power.powered);
            power.switch_on = false;
            power.update(true, true);
            assert!(!power.powered && power.battery_remaining_s > 0.);
            let stored = power.battery_remaining_s;
            for _ in 0..120 {
                power.update(false, true);
            }
            assert_eq!(power.battery_remaining_s, stored);
            power.switch_on = true;
            power.update(true, false);
            assert!(power.powered && !power.on_battery);
            assert_eq!(power.battery_remaining_s, stored);
        }
    }
    #[test]
    fn ahrs_integrates_rates_after_alignment_and_realigns_after_power_loss() {
        let mut a = Ahrs::new(87);
        for _ in 0..5400 {
            a.update(State::default(), true, true);
        }
        let rotating = State {
            angular_rate_rps: [0., 0., 0.1],
            ..State::default()
        };
        let mut angles = None;
        for _ in 0..1200 {
            angles = a.update(rotating, true, true);
        }
        let [roll, pitch, heading] = angles.unwrap_or([f64::NAN; 3]);
        assert!((heading - 1.).abs() < 0.001 && roll.abs() < 0.001 && pitch.abs() < 0.001);
        assert!(a.update(rotating, false, false).is_none());
        assert_eq!(a.alignment_s, 0.);
        assert!(a.update(rotating, true, false).is_none());
        assert_eq!(a.alignment_s, 0.);
    }
}

#[cfg(test)]
mod shared_computer_tests {
    use super::*;
    #[test]
    fn shared_computer_aligns_and_preserves_pressure_errors_and_failure_independence() {
        let sim = crate::Sim::new(&crate::Start::Runway).unwrap();
        let mut systems = sim.systems().clone();
        let truth = sim.readout();
        let mut computer = AirDataComputer::default();
        let mut input = ComputerInput {
            powered: true,
            advance: true,
            ..Default::default()
        };
        for _ in 0..5399 {
            computer.step(&systems, *sim.state(), truth, input);
        }
        assert!(!computer.attitude_valid);
        computer.step(&systems, *sim.state(), truth, input);
        assert!(computer.attitude_valid);
        systems.sensed.indicated_airspeed_mps = 12.;
        systems.sensed.altitude_msl_m = 800.;
        computer.step(&systems, *sim.state(), truth, input);
        assert_eq!(computer.indicated_airspeed_mps, 12.);
        assert_eq!(computer.altitude_msl_m, 800.);
        assert!(computer.air_data_valid);
        input.ahrs_failed = true;
        computer.step(&systems, *sim.state(), truth, input);
        assert!(!computer.attitude_valid && computer.air_data_valid);
        input.ahrs_failed = false;
        input.adc_failed = true;
        computer.step(&systems, *sim.state(), truth, input);
        assert!(!computer.air_data_valid);
        input.powered = false;
        computer.step(&systems, *sim.state(), truth, input);
        assert_eq!(computer.alignment_remaining_s, 45.);
    }
}
