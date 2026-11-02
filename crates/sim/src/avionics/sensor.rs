use crate::{DT_S, State, math};
use schemars::JsonSchema;
use serde::Serialize;

/// Each unit has its own rechargeable battery. Endurance is at rated continuous load.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
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
