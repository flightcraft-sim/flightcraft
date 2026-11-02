//! Original compact attitude and air-data unit; 45 s alignment and 60 min backup endurance.
use super::sensor::{Ahrs, Power};
use crate::{DT_S, Readout, State, systems::SystemsState};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ElectronicAttitude {
    pub power: Power,
    pub aligning: bool,
    pub alignment_remaining_s: f64,
    pub attitude_valid: bool,
    pub air_data_valid: bool,
    pub roll_deg: f64,
    pub pitch_deg: f64,
    pub heading_deg: f64,
    pub airspeed_kt: f64,
    /// Six-second prediction, in knots relative to the current tape value.
    pub airspeed_trend_kt: f64,
    pub altitude_ft: f64,
    pub altitude_trend_ft: f64,
    pub vertical_speed_fpm: f64,
    pub selected_altitude_ft: f64,
    pub baro_inhg: f64,
    pub slip_g: f64,
    pub turn_rate_deg_s: f64,
    #[serde(skip)]
    pub(crate) sensor: Ahrs,
    #[serde(skip)]
    pub(crate) air_data_failed: bool,
    #[serde(skip)]
    previous_airspeed: Option<f64>,
}
impl Default for ElectronicAttitude {
    fn default() -> Self {
        Self::with_seed(0x71b5_8a31, 3600.)
    }
}
impl ElectronicAttitude {
    pub(crate) fn with_seed(seed: u64, endurance: f64) -> Self {
        Self {
            power: Power::new(endurance),
            aligning: true,
            alignment_remaining_s: 45.,
            attitude_valid: false,
            air_data_valid: false,
            roll_deg: 0.,
            pitch_deg: 0.,
            heading_deg: 0.,
            airspeed_kt: 0.,
            airspeed_trend_kt: 0.,
            altitude_ft: 0.,
            altitude_trend_ft: 0.,
            vertical_speed_fpm: 0.,
            selected_altitude_ft: 3000.,
            baro_inhg: 101325. / 3386.389,
            slip_g: 0.,
            turn_rate_deg_s: 0.,
            sensor: Ahrs::new(seed),
            air_data_failed: false,
            previous_airspeed: None,
        }
    }
    pub(crate) fn hash_words(&self) -> Vec<u64> {
        let mut words = self.sensor.hash_words();
        words.extend(
            [
                self.power.switch_on,
                self.power.powered,
                self.power.on_battery,
                self.aligning,
                self.attitude_valid,
                self.air_data_valid,
            ]
            .map(u64::from),
        );
        words.extend(
            [
                self.power.battery_remaining_s,
                self.power.battery_endurance_s,
                self.alignment_remaining_s,
                self.roll_deg,
                self.pitch_deg,
                self.heading_deg,
                self.airspeed_kt,
                self.airspeed_trend_kt,
                self.altitude_ft,
                self.altitude_trend_ft,
                self.vertical_speed_fpm,
                self.selected_altitude_ft,
                self.baro_inhg,
                self.slip_g,
                self.turn_rate_deg_s,
            ]
            .map(f64::to_bits),
        );
        words.push(u64::from(self.air_data_failed));
        words.push(self.previous_airspeed.map_or(u64::MAX, f64::to_bits));
        words
    }
    #[cfg(test)]
    pub(crate) fn step(&mut self, state: State, truth: Readout, systems: &SystemsState) {
        self.update("electronic_attitude", state, truth, systems, true);
    }
    pub(crate) fn update(
        &mut self,
        load: &str,
        state: State,
        truth: Readout,
        systems: &SystemsState,
        advance: bool,
    ) {
        let bus = systems.electrical.loads.iter().any(|l| l.id == load && l.powered);
        self.power.update(bus, advance);
        let angles = self.sensor.update(state, self.power.powered, advance);
        self.attitude_valid = angles.is_some();
        self.alignment_remaining_s = (45. - self.sensor.alignment_s).max(0.);
        self.aligning = self.power.powered && !self.sensor.failed && self.alignment_remaining_s > 1e-8;
        if let Some([roll, pitch, heading]) = angles {
            self.roll_deg = roll.to_degrees();
            self.pitch_deg = pitch.to_degrees();
            self.heading_deg = heading.to_degrees();
            self.slip_g = truth.lateral_acceleration_g;
            self.turn_rate_deg_s = truth.turn_rate_rps.to_degrees();
        }
        self.air_data_valid = self.power.powered && !self.air_data_failed;
        if self.air_data_valid {
            // This unit's blocked-pitot detector holds its last airspeed sample. Round backups
            // retain the pressure-trapping behaviour of the physical pitot-static system.
            if !systems.failures.pitot_blocked || self.previous_airspeed.is_none() {
                self.airspeed_kt = systems.sensed.indicated_airspeed_mps * 1.94384449;
            }
            if advance {
                let raw = self
                    .previous_airspeed
                    .map_or(0., |old| (self.airspeed_kt - old) / DT_S * 6.);
                self.airspeed_trend_kt +=
                    (raw.clamp(-100., 100.) - self.airspeed_trend_kt) * (1. - libm::exp(-DT_S));
                if systems.failures.pitot_blocked {
                    self.airspeed_trend_kt = 0.;
                }
                self.previous_airspeed = Some(self.airspeed_kt);
            }
            let pressure = systems.static_pressure_pa;
            let setting = self.baro_inhg * 3386.389;
            self.altitude_ft = crate::systems::altitude(pressure, setting) * 3.280839895;
            self.vertical_speed_fpm = systems.sensed.vertical_speed_mps * 196.8503937;
            self.altitude_trend_ft = self.vertical_speed_fpm / 10.;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alignment_noise_failure_and_battery_endurance() {
        let mut systems = SystemsState::default();
        let mut a = ElectronicAttitude::default();
        let mut b = a.clone();
        for _ in 0..5399 {
            a.step(State::default(), Readout::default(), &systems);
        }
        assert!(!a.attitude_valid);
        a.step(State::default(), Readout::default(), &systems);
        assert!(a.attitude_valid);
        for _ in 0..5400 {
            b.step(State::default(), Readout::default(), &systems);
        }
        assert_eq!(a, b);
        a.step(State::default(), Readout::default(), &systems);
        assert_ne!(a.roll_deg, 0.);
        a.sensor.failed = true;
        a.step(State::default(), Readout::default(), &systems);
        assert!(!a.attitude_valid && !a.aligning);
        a.sensor.failed = false;
        for _ in 5401..432000 {
            a.step(State::default(), Readout::default(), &systems);
        }
        assert!(!a.power.powered);
        systems.static_pressure_pa = 101325.;
        a.power.switch_on = false;
        let remaining = a.power.battery_remaining_s;
        a.step(State::default(), Readout::default(), &systems);
        assert_eq!(a.power.battery_remaining_s, remaining);
    }
    #[test]
    fn blocked_pitot_holds_only_the_speed_tape() {
        let mut a = ElectronicAttitude::default();
        let mut s = SystemsState::default();
        s.sensed.indicated_airspeed_mps = 40.;
        s.static_pressure_pa = 90000.;
        a.step(State::default(), Readout::default(), &s);
        let speed = a.airspeed_kt;
        let altitude = a.altitude_ft;
        s.failures.pitot_blocked = true;
        s.sensed.indicated_airspeed_mps = 70.;
        s.static_pressure_pa = 80000.;
        a.step(State::default(), Readout::default(), &s);
        assert_eq!(a.airspeed_kt, speed);
        assert_ne!(a.altitude_ft, altitude);
        assert_eq!(a.airspeed_trend_kt, 0.);
    }
}
