//! "Show me": the virtual pilot performs a checklist item with an intercom explanation.
//!
//! The demonstration is a short script of the same control actions a student would make (switch,
//! rpm hold, lane off for three seconds), so the run-up observer records it exactly as it would a
//! student's, and the item ticks by its own `verify` binding.
use super::eval::{Probe, eval};
use super::runup::{LANE_SETTLE_S, PUMP_SETTLE_S, RunupObserver};
use crate::Session;
use flightcraft_aero::checklists::{ChecklistItem, LaneId, PumpId, RunupCheck, Verify};
use flightcraft_aero::panel_fidelity::BackupCircuit;
use flightcraft_sim::modern::{KeyPosition, Lane, ModernSetting, Pump};
use flightcraft_sim::systems::{FuelSelector, Magnetos, SystemSetting, SystemSwitch};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
enum Action {
    Master(bool),
    Avionics(bool),
    Ignition(bool),
    Lane(LaneId, bool),
    Pump(PumpId, bool),
    Backup(BackupCircuit, bool),
    Fuel(FuelSelector),
    Panel(String, f64),
    Key(KeyPosition),
    Starter(bool),
    Brake(f64),
    ParkBrake(bool),
    Flaps(f64),
    Trim,
    Throttle(f64),
    Elevator(f64),
    Aileron(f64),
    Rudder(f64),
    Canopy,
}
#[derive(Clone, Debug)]
enum Step {
    Do(Action),
    Wait(f64),
    Until(Verify, f64),
    Hold(Option<f64>),
}
#[derive(Clone, Debug)]
pub(crate) struct ShowMe {
    pub item: String,
    steps: VecDeque<Step>,
    hold_rpm: Option<f64>,
    wait_until: Option<f64>,
    deadline: Option<f64>,
}

fn simple(v: &Verify, out: &mut VecDeque<Step>) {
    use Step::{Do, Hold, Until, Wait};
    match v {
        Verify::Master { on } => out.push_back(Do(Action::Master(*on))),
        Verify::Avionics { on } => out.push_back(Do(Action::Avionics(*on))),
        Verify::Ignition { on } => out.push_back(Do(Action::Ignition(*on))),
        Verify::Lane { lane, on } => out.push_back(Do(Action::Lane(*lane, *on))),
        Verify::Pump { pump, on } => out.push_back(Do(Action::Pump(*pump, *on))),
        Verify::BackupBattery { circuit, on, .. } => out.push_back(Do(Action::Backup(*circuit, *on))),
        Verify::FuelSelector { positions } => {
            let f = match positions.first().map(String::as_str) {
                Some("right") => FuelSelector::Right,
                Some("both") => FuelSelector::Both,
                Some("off") => FuelSelector::Off,
                _ => FuelSelector::Left,
            };
            out.push_back(Do(Action::Fuel(f)));
        }
        Verify::Panel { control, min, max } => {
            let value = match (min, max) {
                (Some(m), _) if *m > 0. => 1.,
                (_, Some(m)) if *m < 1. => 0.,
                _ => 1.,
            };
            out.push_back(Do(Action::Panel(control.clone(), value)));
        }
        Verify::Rpm { min, max } => {
            let target = match (min, max) {
                (Some(a), Some(b)) => (a + b) / 2.,
                (Some(a), None) => a + 50.,
                (None, Some(b)) => (b - 50.).max(900.),
                (None, None) => 2000.,
            };
            out.push_back(Hold(Some(target)));
            out.push_back(Until(v.clone(), 25.));
        }
        Verify::EngineRunning { running: true } => {
            out.push_back(Do(Action::Throttle(0.1)));
            out.push_back(Do(Action::Key(KeyPosition::Start)));
            out.push_back(Do(Action::Starter(true)));
            out.push_back(Until(v.clone(), 9.));
            out.push_back(Do(Action::Key(KeyPosition::On)));
            out.push_back(Do(Action::Starter(false)));
            out.push_back(Wait(2.));
        }
        Verify::EngineRunning { running: false } => {
            out.push_back(Hold(None));
            out.push_back(Do(Action::Throttle(0.)));
            out.push_back(Do(Action::Ignition(false)));
        }
        Verify::Starter { engaged } => {
            out.push_back(Do(Action::Key(if *engaged {
                KeyPosition::Start
            } else {
                KeyPosition::On
            })));
            out.push_back(Do(Action::Starter(*engaged)));
        }
        Verify::Brake { set } => {
            out.push_back(Do(Action::Brake(if *set { 1. } else { 0. })));
            if !*set {
                out.push_back(Do(Action::ParkBrake(false)));
            }
        }
        Verify::ParkBrake { set } => out.push_back(Do(Action::ParkBrake(*set))),
        Verify::Flaps { min, max } => {
            let down = min.is_some_and(|m| m > 0.1) && max.is_none();
            out.push_back(Do(Action::Flaps(if down { 0.5 } else { 0. })));
        }
        Verify::TrimNeutral { .. } => out.push_back(Do(Action::Trim)),
        Verify::Throttle { min, max } => {
            let t = match (min, max) {
                (Some(a), Some(b)) => (a + b) / 2.,
                (Some(a), None) => (*a + 1.) / 2.,
                (None, Some(b)) => *b / 2.,
                (None, None) => 0.,
            };
            out.push_back(Do(Action::Throttle(t)));
        }
        Verify::CanopyLatched { latched: true } => out.push_back(Do(Action::Canopy)),
        Verify::Check(check) => match *check {
            RunupCheck::Lane { lane, at_rpm, .. } => {
                out.push_back(Hold(Some(at_rpm)));
                out.push_back(Until(
                    Verify::Rpm {
                        min: Some(at_rpm - 100.),
                        max: Some(at_rpm + 100.),
                    },
                    25.,
                ));
                out.push_back(Wait(3.));
                // Freeze the throttle: the check reads the drop at a fixed power setting.
                out.push_back(Hold(None));
                out.push_back(Do(Action::Lane(lane, false)));
                out.push_back(Wait(LANE_SETTLE_S + 0.8));
                out.push_back(Do(Action::Lane(lane, true)));
                out.push_back(Wait(2.));
            }
            RunupCheck::Pump { pump, .. } => {
                let other = if pump == PumpId::One {
                    PumpId::Two
                } else {
                    PumpId::One
                };
                out.push_back(Do(Action::Pump(pump, true)));
                out.push_back(Do(Action::Pump(other, false)));
                out.push_back(Wait(PUMP_SETTLE_S + 0.8));
                out.push_back(Do(Action::Pump(other, true)));
                out.push_back(Wait(1.));
            }
            RunupCheck::Controls { .. } => {
                for a in [
                    Action::Elevator(1.),
                    Action::Elevator(-1.),
                    Action::Elevator(0.),
                    Action::Aileron(1.),
                    Action::Aileron(-1.),
                    Action::Aileron(0.),
                    Action::Rudder(1.),
                    Action::Rudder(-1.),
                    Action::Rudder(0.),
                ] {
                    out.push_back(Do(a));
                    out.push_back(Wait(0.8));
                }
            }
            RunupCheck::MaxRpm { .. } => {
                out.push_back(Hold(None));
                out.push_back(Do(Action::Throttle(0.3)));
                out.push_back(Wait(2.));
                out.push_back(Do(Action::Throttle(1.)));
                out.push_back(Wait(4.));
                out.push_back(Do(Action::Throttle(0.3)));
            }
        },
        Verify::All { of } => of.iter().for_each(|v| simple(v, out)),
        Verify::BusVoltage { min_v: Some(_), .. } => {
            // The alternator carries the load only at higher rpm (POH 9.4.12 item 15: about 3,000).
            out.push_back(Hold(Some(3000.)));
            out.push_back(Until(v.clone(), 25.));
        }
        Verify::OilTemp { .. }
        | Verify::OilPressure { .. }
        | Verify::FuelPressure { .. }
        | Verify::BusVoltage { .. }
        | Verify::CanopyLatched { latched: false } => {}
    }
}

impl ShowMe {
    pub fn plan(_s: &Session, item: &ChecklistItem) -> Option<Self> {
        let mut steps = VecDeque::new();
        if let Some(v) = &item.verify {
            simple(v, &mut steps);
        }
        Some(Self {
            item: item.id.clone(),
            steps,
            hold_rpm: None,
            wait_until: None,
            deadline: None,
        })
    }

    pub fn explanation(&self, item: &ChecklistItem) -> String {
        let how = match &item.verify {
            Some(Verify::Check(RunupCheck::Lane {
                lane,
                at_rpm,
                max_drop_rpm,
                ..
            })) => format!(
                " Hold {at_rpm:.0} rpm, switch lane {} off for about three seconds, note the drop (limit {max_drop_rpm:.0} rpm), then back on. One lane at a time.",
                if *lane == LaneId::A { "A" } else { "B" }
            ),
            Some(Verify::Check(RunupCheck::Pump { pump, .. })) => format!(
                " Switch the other pump off and watch fuel pressure hold on pump {} alone, then restore it.",
                if *pump == PumpId::One { "1" } else { "2" }
            ),
            Some(Verify::Check(RunupCheck::Controls { .. })) => {
                " Move stick fully forward and back, fully left and right, and both rudder pedals, watching the surfaces move freely and the right way.".into()
            }
            Some(Verify::Check(RunupCheck::MaxRpm { .. })) => {
                " Advance smoothly to full power, check the rpm and that acceleration is smooth, then bring it back.".into()
            }
            Some(Verify::OilTemp { .. } | Verify::OilPressure { .. } | Verify::BusVoltage { .. } | Verify::FuelPressure { .. }) => {
                " That is a reading to check on the engine page, not something to set.".into()
            }
            None => " Nothing in the simulator to set; confirm it by eye or with your passenger.".into(),
            _ => String::new(),
        };
        format!("{}: {}. ({}).{}", item.text, item.response, item.poh_ref, how)
    }

    /// Runs the script; `None` when finished.
    pub fn tick(mut self, s: &mut Session, p: &Probe, obs: &RunupObserver) -> Option<Self> {
        let now = p.time_s;
        loop {
            if self.wait_until.is_some_and(|t| now < t) {
                break;
            }
            self.wait_until = None;
            let Some(step) = self.steps.front().cloned() else {
                break;
            };
            match step {
                Step::Do(a) => {
                    apply(s, &a);
                    self.steps.pop_front();
                }
                Step::Hold(r) => {
                    self.hold_rpm = r;
                    self.steps.pop_front();
                }
                Step::Wait(d) => {
                    self.wait_until = Some(now + d);
                    self.steps.pop_front();
                    break;
                }
                Step::Until(v, timeout) => {
                    let deadline = *self.deadline.get_or_insert(now + timeout);
                    if eval(&v, p, obs) || now > deadline {
                        self.deadline = None;
                        self.steps.pop_front();
                    } else {
                        break;
                    }
                }
            }
        }
        if let Some(target) = self.hold_rpm {
            let mut c = s.sim.controls();
            c.throttle = (c.throttle + ((target - p.rpm) * 0.00008).clamp(-0.03, 0.03)).clamp(0., 1.);
            s.sim.set_controls(c);
        }
        if self.steps.is_empty() && self.wait_until.is_none_or(|t| now >= t) {
            return None;
        }
        Some(self)
    }
}

fn apply(s: &mut Session, a: &Action) {
    let electronic = s.sim.systems().modern.as_ref().is_some_and(|m| m.electronic);
    let mut controls = s.sim.controls();
    let equip = |s: &mut Session, setting| {
        let _ = s.sim.set_systems(SystemSetting::Equipment { setting });
    };
    match a {
        Action::Master(on) => {
            let _ = s.sim.set_systems(SystemSetting::Switch {
                switch: SystemSwitch::Master,
                on: *on,
            });
        }
        Action::Avionics(on) => {
            let _ = s.sim.set_systems(SystemSetting::Switch {
                switch: SystemSwitch::Avionics,
                on: *on,
            });
        }
        Action::Ignition(on) => {
            if electronic {
                equip(
                    s,
                    ModernSetting::Lane {
                        lane: Lane::A,
                        on: *on,
                    },
                );
                equip(
                    s,
                    ModernSetting::Lane {
                        lane: Lane::B,
                        on: *on,
                    },
                );
                equip(
                    s,
                    ModernSetting::Key {
                        position: if *on { KeyPosition::On } else { KeyPosition::Off },
                    },
                );
            } else {
                let _ = s.sim.set_systems(SystemSetting::Magnetos {
                    position: if *on { Magnetos::Both } else { Magnetos::Off },
                });
            }
        }
        Action::Lane(lane, on) => equip(
            s,
            ModernSetting::Lane {
                lane: if *lane == LaneId::A { Lane::A } else { Lane::B },
                on: *on,
            },
        ),
        Action::Pump(pump, on) => equip(
            s,
            ModernSetting::Pump {
                pump: if *pump == PumpId::One {
                    Pump::One
                } else {
                    Pump::Two
                },
                on: *on,
            },
        ),
        Action::Backup(circuit, on) => equip(
            s,
            ModernSetting::BackupPower {
                circuit: *circuit,
                on: *on,
            },
        ),
        Action::Fuel(position) => {
            let _ = s
                .sim
                .set_systems(SystemSetting::FuelSelector { position: *position });
        }
        Action::Panel(id, value) => equip(
            s,
            ModernSetting::PanelControl {
                id: id.clone(),
                value: *value,
            },
        ),
        Action::Key(position) => {
            if electronic {
                equip(s, ModernSetting::Key { position: *position });
            }
        }
        Action::Starter(on) => {
            if !electronic {
                let _ = s.sim.set_systems(SystemSetting::Switch {
                    switch: SystemSwitch::Starter,
                    on: *on,
                });
            }
        }
        Action::Brake(v) => {
            controls.brake = *v;
            s.sim.set_controls(controls);
        }
        Action::ParkBrake(on) => {
            let _ = s.sim.set_systems(SystemSetting::ParkBrake { on: *on });
        }
        Action::Flaps(v) => {
            controls.flaps = *v;
            s.sim.set_controls(controls);
        }
        Action::Trim => {
            let _ = s.sim.set_systems(SystemSetting::Trim { value: 0. });
        }
        Action::Throttle(v) => {
            controls.throttle = *v;
            s.sim.set_controls(controls);
        }
        Action::Elevator(v) => {
            controls.elevator = *v;
            s.sim.set_controls(controls);
        }
        Action::Aileron(v) => {
            controls.aileron = *v;
            s.sim.set_controls(controls);
        }
        Action::Rudder(v) => {
            controls.rudder = *v;
            s.sim.set_controls(controls);
        }
        Action::Canopy => {
            use flightcraft_sim::canopy::CanopyAction;
            let _ = s
                .sim
                .set_canopy(CanopyAction::Close, None)
                .and_then(|_| s.sim.set_canopy(CanopyAction::Latch, None));
        }
    }
}
