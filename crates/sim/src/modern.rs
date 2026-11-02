//! Fixed-step electronic engine controls, annunciation latches, and pilot recovery controls.
use crate::{DT_S, SimError};
use flightcraft_aero::modern::{AnnunciationSource, EngineControl, ModernSystems, Severity};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    A,
    B,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Pump {
    One,
    Two,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeyPosition {
    Off,
    On,
    Start,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    LaneA,
    LaneB,
    Pump1,
    Pump2,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModernSetting {
    PanelControl {
        id: String,
        value: f64,
    },
    PanelGuard {
        id: String,
        open: bool,
    },
    Light {
        light: flightcraft_aero::LoadSwitch,
        on: bool,
    },
    Lane {
        lane: Lane,
        on: bool,
    },
    Pump {
        pump: Pump,
        on: bool,
    },
    Failure {
        component: Component,
        failed: bool,
    },
    Key {
        position: KeyPosition,
    },
    Acknowledge {
        severity: Severity,
    },
    Door {
        open: bool,
    },
    ParachutePin {
        inserted: bool,
    },
    ParachuteHandle {
        held: bool,
    },
    /// Explicit pilot command; bypasses the UI hold, but never the safety pin.
    ParachuteDeploy,
}
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct LaneState {
    pub on: bool,
    pub failed: bool,
    pub running: bool,
    pub light: bool,
}
impl Default for LaneState {
    fn default() -> Self {
        Self {
            on: true,
            failed: false,
            running: true,
            light: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PumpState {
    pub on: bool,
    pub failed: bool,
    pub running: bool,
}
impl Default for PumpState {
    fn default() -> Self {
        Self {
            on: true,
            failed: false,
            running: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPhase {
    Stowed,
    Deploying,
    Inflating,
    Descending,
    Impact,
}
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct RecoveryState {
    pub pin_inserted: bool,
    pub handle_held: bool,
    pub hold_progress: f64,
    pub phase: RecoveryPhase,
    pub elapsed_s: f64,
    pub inflation: f64,
    pub impact_vertical_speed_mps: Option<f64>,
}
impl Default for RecoveryState {
    fn default() -> Self {
        Self {
            pin_inserted: true,
            handle_held: false,
            hold_progress: 0.,
            phase: RecoveryPhase::Stowed,
            elapsed_s: 0.,
            inflation: 0.,
            impact_vertical_speed_mps: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Annunciation {
    pub source: AnnunciationSource,
    pub severity: Severity,
    pub label: String,
    pub active: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ModernState {
    pub panel_controls: std::collections::BTreeMap<String, f64>,
    pub panel_guards: std::collections::BTreeMap<String, bool>,
    pub light_switches: Vec<(flightcraft_aero::LoadSwitch, bool)>,
    pub electronic: bool,
    pub lane_a: LaneState,
    pub lane_b: LaneState,
    pub pump_1: PumpState,
    pub pump_2: PumpState,
    pub key: KeyPosition,
    pub propeller_rpm: f64,
    pub fuel_pressure_pa: f64,
    pub coolant_temperature_c: f64,
    pub manifold_pressure_pa: f64,
    pub engine_control_warning: bool,
    pub door_open: bool,
    pub master_caution: bool,
    pub master_warning: bool,
    pub annunciations: Vec<Annunciation>,
    pub recovery: Option<RecoveryState>,
}
impl ModernState {
    pub(crate) fn new(control: &EngineControl, d: Option<&ModernSystems>) -> Self {
        Self {
            panel_controls: Default::default(),
            panel_guards: Default::default(),
            light_switches: [
                flightcraft_aero::LoadSwitch::Beacon,
                flightcraft_aero::LoadSwitch::Nav,
                flightcraft_aero::LoadSwitch::Strobe,
                flightcraft_aero::LoadSwitch::Taxi,
                flightcraft_aero::LoadSwitch::Landing,
            ]
            .into_iter()
            .map(|light| (light, false))
            .collect(),
            electronic: matches!(control, EngineControl::ElectronicDualLane { .. }),
            lane_a: LaneState::default(),
            lane_b: LaneState::default(),
            pump_1: PumpState::default(),
            pump_2: PumpState::default(),
            key: KeyPosition::On,
            propeller_rpm: 0.,
            fuel_pressure_pa: 0.,
            coolant_temperature_c: 85.,
            manifold_pressure_pa: 101325.,
            engine_control_warning: false,
            door_open: false,
            master_caution: false,
            master_warning: false,
            annunciations: d
                .map(|d| {
                    d.annunciations
                        .iter()
                        .map(|a| Annunciation {
                            source: a.source,
                            severity: a.severity,
                            label: a.label.clone(),
                            active: false,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            recovery: d
                .and_then(|d| d.parachute.as_ref())
                .map(|_| RecoveryState::default()),
        }
    }
    pub(crate) fn set(&mut self, setting: ModernSetting) -> Result<(), SimError> {
        let invalid = |message: &str| SimError::InvalidSystems(message.into());
        match setting {
            ModernSetting::PanelControl { id, value } => {
                if !value.is_finite() || !(0. ..=1.).contains(&value) {
                    return Err(invalid("panel control requires 0..1"));
                }
                if self.panel_guards.get(&id) == Some(&false) {
                    return Err(invalid("open the control guard first"));
                }
                *self
                    .panel_controls
                    .get_mut(&id)
                    .ok_or_else(|| invalid("unknown panel control"))? = value;
            }
            ModernSetting::PanelGuard { id, open } => {
                *self
                    .panel_guards
                    .get_mut(&id)
                    .ok_or_else(|| invalid("unknown control guard"))? = open;
            }
            ModernSetting::Light { light, on } => {
                let value = self
                    .light_switches
                    .iter_mut()
                    .find(|(switch, _)| *switch == light)
                    .ok_or_else(|| invalid("not an individual light switch"))?;
                value.1 = on;
            }
            ModernSetting::Lane { lane, on } => {
                if !self.electronic {
                    return Err(invalid("electronic lanes not installed"));
                }
                match lane {
                    Lane::A => self.lane_a.on = on,
                    Lane::B => self.lane_b.on = on,
                }
            }
            ModernSetting::Pump { pump, on } => {
                if !self.electronic {
                    return Err(invalid("dual pumps not installed"));
                }
                match pump {
                    Pump::One => self.pump_1.on = on,
                    Pump::Two => self.pump_2.on = on,
                }
            }
            ModernSetting::Failure { component, failed } => {
                if !self.electronic {
                    return Err(invalid("electronic controls not installed"));
                }
                *match component {
                    Component::LaneA => &mut self.lane_a.failed,
                    Component::LaneB => &mut self.lane_b.failed,
                    Component::Pump1 => &mut self.pump_1.failed,
                    Component::Pump2 => &mut self.pump_2.failed,
                } = failed;
            }
            ModernSetting::Key { position } => {
                if !self.electronic {
                    return Err(invalid("three-position key not installed"));
                }
                self.key = position;
            }
            ModernSetting::Acknowledge { severity } => match severity {
                Severity::Caution => self.master_caution = false,
                Severity::Warning => self.master_warning = false,
            },
            ModernSetting::Door { open } => self.door_open = open,
            setting => {
                let p = self
                    .recovery
                    .as_mut()
                    .ok_or_else(|| invalid("recovery parachute not installed"))?;
                match setting {
                    ModernSetting::ParachutePin { inserted } => {
                        if p.phase != RecoveryPhase::Stowed {
                            return Err(invalid("recovery already deployed"));
                        }
                        p.pin_inserted = inserted;
                        if inserted {
                            p.handle_held = false;
                            p.hold_progress = 0.;
                        }
                    }
                    ModernSetting::ParachuteHandle { held } => {
                        if held && p.pin_inserted {
                            return Err(invalid("remove safety pin before pulling"));
                        }
                        p.handle_held = held;
                        if !held {
                            p.hold_progress = 0.;
                        }
                    }
                    ModernSetting::ParachuteDeploy => {
                        if p.pin_inserted {
                            return Err(invalid("remove safety pin before deployment"));
                        }
                        if p.phase == RecoveryPhase::Stowed {
                            p.phase = RecoveryPhase::Deploying;
                            p.elapsed_s = 0.;
                            p.hold_progress = 1.;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    pub(crate) fn recovery_step(&mut self, d: Option<&ModernSystems>) {
        let Some((p, def)) = self.recovery.as_mut().zip(d.and_then(|d| d.parachute.as_ref())) else {
            return;
        };
        if p.phase == RecoveryPhase::Stowed {
            if p.handle_held && !p.pin_inserted {
                p.hold_progress = (p.hold_progress + DT_S / def.hold_s).min(1.);
                if p.hold_progress >= 1. - 1e-12 {
                    p.phase = RecoveryPhase::Deploying;
                    p.elapsed_s = 0.;
                }
            }
            return;
        }
        if p.phase == RecoveryPhase::Impact {
            return;
        }
        p.elapsed_s += DT_S;
        if p.elapsed_s >= def.deployment_delay_s {
            p.inflation = ((p.elapsed_s - def.deployment_delay_s) / def.inflation_s).clamp(0., 1.);
            p.phase = if p.inflation >= 1. {
                RecoveryPhase::Descending
            } else {
                RecoveryPhase::Inflating
            };
        }
    }
    pub(crate) fn annunciations_step(
        &mut self,
        d: Option<&ModernSystems>,
        voltage: f64,
        engine_running: bool,
    ) {
        let Some(d) = d else {
            return;
        };
        for (a, def) in self.annunciations.iter_mut().zip(&d.annunciations) {
            let active = match def.source {
                AnnunciationSource::LowVoltage => voltage < def.threshold,
                AnnunciationSource::LowFuelPressure => self.fuel_pressure_pa < def.threshold,
                AnnunciationSource::LaneFault => self.engine_control_warning,
                AnnunciationSource::HighCoolantTemperature => self.coolant_temperature_c > def.threshold,
                AnnunciationSource::DoorOpen => self.door_open,
                AnnunciationSource::ParachuteArmed => self.recovery.as_ref().is_some_and(|p| !p.pin_inserted),
                AnnunciationSource::EngineShutdown => {
                    engine_running
                        && self
                            .recovery
                            .as_ref()
                            .is_some_and(|p| p.phase != RecoveryPhase::Stowed)
                }
            };
            if active && !a.active {
                match a.severity {
                    Severity::Caution => self.master_caution = true,
                    Severity::Warning => self.master_warning = true,
                }
            }
            a.active = active;
        }
    }
}
