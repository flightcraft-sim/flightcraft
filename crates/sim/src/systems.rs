//! Deterministic sensors and lumped aircraft systems. Stepped once, outside RK4 substages.
use crate::{DT_S, Failures, Readout, SimError};
use flightcraft_aero::{
    Bus, Induction, LoadSwitch, ResolvedAircraft, SystemsDefinition, Table, Wing, interpolate,
};
use flightcraft_world::{SEA_LEVEL_DENSITY, atmosphere};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Magnetos {
    Off,
    Left,
    Right,
    Both,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FuelSelector {
    Off,
    Left,
    Right,
    Both,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemSwitch {
    Master,
    Avionics,
    Alternator,
    Lights,
    FuelPump,
    PitotHeat,
    AlternateStatic,
    CarbHeat,
    Starter,
}

/// One atomic panel action. Numeric values are validated before any state changes.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SystemSetting {
    Equipment {
        setting: crate::modern::ModernSetting,
    },
    Switch {
        switch: SystemSwitch,
        on: bool,
    },
    Magnetos {
        position: Magnetos,
    },
    FuelSelector {
        position: FuelSelector,
    },
    Mixture {
        #[schemars(range(min = 0, max = 1))]
        value: f64,
    },
    HeadingBug {
        /// Magnetic heading, radians clockwise from magnetic north.
        #[schemars(range(min = 0, max = core::f64::consts::TAU))]
        heading_rad: f64,
    },
    Altimeter {
        #[schemars(range(min = 80000, max = 110000))]
        pressure_pa: f64,
    },
    /// Primer charge, normalized; consumed while cranking.
    Primer {
        #[schemars(range(min = 0, max = 1))]
        charge: f64,
    },
    Breaker {
        load: String,
        closed: bool,
    },
    /// A deterministic load fault, for overload practice. 1 restores rated current.
    LoadMultiplier {
        load: String,
        #[schemars(range(min = 0, max = 10))]
        value: f64,
    },
    /// Ground servicing / scenario setup; mass updates on the next fixed step.
    TankQuantity {
        tank: String,
        mass_kg: f64,
    },
    Environment {
        #[schemars(range(min = -60, max = 60))]
        temperature_c: f64,
        #[schemars(range(min = 0, max = 1))]
        relative_humidity: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SystemControls {
    pub master: bool,
    pub avionics: bool,
    pub alternator: bool,
    pub heading_bug_rad: f64,
    pub lights: bool,
    pub fuel_pump: bool,
    pub pitot_heat: bool,
    pub alternate_static: bool,
    pub carb_heat: bool,
    pub starter: bool,
    pub magnetos: Magnetos,
    pub mixture: f64,
    pub fuel_selector: FuelSelector,
    pub altimeter_setting_pa: f64,
    pub temperature_c: f64,
    pub relative_humidity: f64,
}
impl Default for SystemControls {
    fn default() -> Self {
        Self {
            master: true,
            avionics: true,
            alternator: true,
            heading_bug_rad: 0.0,
            lights: false,
            fuel_pump: false,
            pitot_heat: false,
            alternate_static: false,
            carb_heat: false,
            starter: false,
            magnetos: Magnetos::Both,
            mixture: 1.0,
            fuel_selector: FuelSelector::Both,
            altimeter_setting_pa: 101325.0,
            temperature_c: 15.0,
            relative_humidity: 0.4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct LoadState {
    pub id: String,
    pub breaker_closed: bool,
    pub tripped: bool,
    pub overload_s: f64,
    pub multiplier: f64,
    pub current_a: f64,
    pub powered: bool,
    pub on_backup: bool,
    pub backup_remaining_ah: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ElectricalState {
    pub battery_capacity_ah: f64,
    pub battery_soc: f64,
    pub battery_voltage: f64,
    /// Positive discharges the battery; negative charges it.
    pub battery_current_a: f64,
    pub alternator_current_a: f64,
    pub main_bus_voltage: f64,
    pub essential_bus_voltage: f64,
    pub loads: Vec<LoadState>,
}
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TankState {
    pub id: String,
    pub wing: Wing,
    pub capacity_kg: f64,
    pub unusable_kg: f64,
    pub quantity_kg: f64,
    pub indicated_kg: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct SensedInstruments {
    pub indicated_airspeed_mps: f64,
    pub altitude_msl_m: f64,
    pub vertical_speed_mps: f64,
    pub roll_rad: f64,
    pub pitch_rad: f64,
    pub heading_rad: f64,
    pub attitude_valid: bool,
    pub turn_rate_rps: f64,
    /// Canted electric gyro: yaw rate plus roll-rate sensitivity for a rate autopilot.
    pub turn_coordinator_rate_rps: f64,
    pub lateral_acceleration_g: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SystemsState {
    /// Normalized cockpit smoke obscuration; accumulates under a smoke source, clears after isolation.
    pub smoke_density: f64,
    pub modern: Option<crate::modern::ModernState>,
    pub avionics: crate::avionics::AvionicsState,
    pub enabled: bool,
    pub vacuum_installed: bool,
    pub carb_heat_installed: bool,
    pub controls: SystemControls,
    pub failures: Failures,
    pub electrical: ElectricalState,
    pub tanks: Vec<TankState>,
    pub suction_pa: f64,
    pub gyro_spin: f64,
    pub gyro_error_rad: [f64; 3],
    pub gyro_truth_rad: [f64; 3],
    /// Free heading gyro is aligned to the compass at reset, then retains this reference.
    pub gyro_heading_reference_rad: f64,
    pub pitot_total_pressure_pa: f64,
    pub static_pressure_pa: f64,
    pub pressure_altitude_m: f64,
    pub pitot_heat_powered: bool,
    pub engine_running: bool,
    pub engine_rpm: f64,
    pub engine_power_fraction: f64,
    pub fuel_flow_kg_s: f64,
    pub fuel_starved: bool,
    pub carb_ice: f64,
    pub egt_c: f64,
    pub cht_c: f64,
    pub oil_temperature_c: f64,
    pub oil_pressure_pa: f64,
    pub primer_charge: f64,
    pub cranking_s: f64,
    pub sensed: SensedInstruments,
}

#[derive(Clone, Debug)]
pub(crate) struct SystemsModel {
    pub definition: SystemsDefinition,
    pub engine_control: flightcraft_aero::modern::EngineControl,
    pub gearbox_ratio: f64,
    pub navigator_load: Option<String>,
    pub induction: Induction,
    pub fuel_flow: Table,
    pub idle_fraction: f64,
    pub power_density: Table,
    pub variant: flightcraft_aero::Variant,
    pub load: flightcraft_aero::Load,
}

impl Default for SystemsState {
    fn default() -> Self {
        Self {
            modern: None,
            avionics: crate::avionics::AvionicsState::default(),
            smoke_density: 0.0,
            enabled: false,
            vacuum_installed: false,
            carb_heat_installed: false,
            controls: SystemControls::default(),
            failures: Failures::default(),
            electrical: ElectricalState {
                battery_capacity_ah: 0.0,
                battery_soc: 1.0,
                battery_voltage: 0.0,
                battery_current_a: 0.0,
                alternator_current_a: 0.0,
                main_bus_voltage: 0.0,
                essential_bus_voltage: 0.0,
                loads: Vec::new(),
            },
            tanks: Vec::new(),
            suction_pa: 0.0,
            gyro_spin: 1.0,
            gyro_error_rad: [0.0; 3],
            gyro_truth_rad: [0.0; 3],
            gyro_heading_reference_rad: 0.,
            pitot_total_pressure_pa: 101325.0,
            static_pressure_pa: 101325.0,
            pressure_altitude_m: 0.0,
            pitot_heat_powered: false,
            engine_running: true,
            engine_rpm: 0.0,
            engine_power_fraction: 1.0,
            fuel_flow_kg_s: 0.0,
            fuel_starved: false,
            carb_ice: 0.0,
            egt_c: 700.0,
            cht_c: 150.0,
            oil_temperature_c: 70.0,
            oil_pressure_pa: 350000.0,
            primer_charge: 0.0,
            cranking_s: 0.0,
            sensed: SensedInstruments::default(),
        }
    }
}

fn invalid(message: &str) -> SimError {
    SimError::InvalidSystems(message.into())
}
fn bounded(value: f64, lo: f64, hi: f64) -> Result<(), SimError> {
    if value.is_finite() && (lo..=hi).contains(&value) {
        Ok(())
    } else {
        Err(invalid("systems value outside finite permitted range"))
    }
}
fn lag(value: &mut f64, target: f64, seconds: f64) {
    *value += (target - *value) * (DT_S / seconds).min(1.0);
}
pub(crate) fn altitude(pressure: f64, setting: f64) -> f64 {
    // Inverse of the same ISA layers as world::atmosphere; extended below sea level.
    let p11 = atmosphere(11000.0).pressure_pa * setting / 101325.0;
    if pressure >= p11 {
        288.15 / 0.0065
            * (1.0
                - libm::pow(
                    pressure.max(1.0) / setting,
                    0.0065 * flightcraft_world::GAS_CONSTANT / flightcraft_world::GRAVITY_MPS2,
                ))
    } else {
        11000.0
            - flightcraft_world::GAS_CONSTANT * 216.65 / flightcraft_world::GRAVITY_MPS2
                * libm::log(pressure.max(1.0) / p11)
    }
}

impl SystemsState {
    /// Legacy installations expose ideal sensors through the same presentation interface.
    pub(crate) fn ideal_sensors(&mut self, r: Readout) {
        self.sensed = SensedInstruments {
            indicated_airspeed_mps: r.indicated_airspeed_mps,
            altitude_msl_m: r.altitude_msl_m,
            vertical_speed_mps: r.vertical_speed_mps,
            roll_rad: r.roll_rad,
            pitch_rad: r.pitch_rad,
            heading_rad: r.heading_rad,
            turn_rate_rps: r.turn_rate_rps,
            turn_coordinator_rate_rps: r.turn_rate_rps,
            lateral_acceleration_g: r.lateral_acceleration_g,
            attitude_valid: true,
        };
        self.engine_rpm = r.engine_rpm;
    }

    pub(crate) fn hash_words(&self) -> Vec<u64> {
        let c = &self.controls;
        let e = &self.electrical;
        let sensed = self.sensed;
        let mut words = vec![c.magnetos as u64, c.fuel_selector as u64];
        words.extend(self.failures.emergency_hash_words());
        if self.smoke_density > 0.0 {
            words.extend([0x736d6f6b65, self.smoke_density.to_bits()]);
        }
        words.extend(
            [
                self.enabled,
                self.vacuum_installed,
                self.carb_heat_installed,
                c.master,
                c.avionics,
                c.alternator,
                c.lights,
                c.fuel_pump,
                c.pitot_heat,
                c.alternate_static,
                c.carb_heat,
                c.starter,
                self.engine_running,
                self.fuel_starved,
                self.pitot_heat_powered,
                sensed.attitude_valid,
                self.failures.vacuum_pump,
                self.failures.alternator,
                self.failures.pitot_blocked,
                self.failures.static_blocked,
            ]
            .map(u64::from),
        );
        words.extend(
            [
                c.mixture,
                c.heading_bug_rad,
                self.oil_temperature_c,
                self.oil_pressure_pa,
                sensed.turn_rate_rps,
                sensed.lateral_acceleration_g,
                c.altimeter_setting_pa,
                c.temperature_c,
                c.relative_humidity,
                e.battery_capacity_ah,
                e.battery_soc,
                e.battery_voltage,
                e.battery_current_a,
                e.alternator_current_a,
                e.main_bus_voltage,
                e.essential_bus_voltage,
                self.suction_pa,
                self.gyro_spin,
                self.pitot_total_pressure_pa,
                self.static_pressure_pa,
                self.pressure_altitude_m,
                self.engine_rpm,
                self.engine_power_fraction,
                self.fuel_flow_kg_s,
                self.carb_ice,
                self.egt_c,
                self.cht_c,
                self.primer_charge,
                self.cranking_s,
                sensed.indicated_airspeed_mps,
                sensed.altitude_msl_m,
                sensed.vertical_speed_mps,
                sensed.roll_rad,
                sensed.pitch_rad,
                sensed.heading_rad,
                sensed.turn_coordinator_rate_rps,
            ]
            .into_iter()
            .chain(self.gyro_error_rad)
            .chain(self.gyro_truth_rad)
            .map(f64::to_bits),
        );
        if self.gyro_heading_reference_rad != 0. {
            words.push(0x6d61676e65746963);
            words.push(self.gyro_heading_reference_rad.to_bits());
        }
        for l in &e.loads {
            words.push(l.id.len() as u64);
            words.extend(l.id.bytes().map(u64::from));
            words.extend([l.breaker_closed, l.tripped, l.powered, l.on_backup].map(u64::from));
            words.extend([l.overload_s, l.multiplier, l.current_a, l.backup_remaining_ah].map(f64::to_bits));
        }
        for t in &self.tanks {
            words.push(t.id.len() as u64);
            words.extend(t.id.bytes().map(u64::from));
            words.extend([t.capacity_kg, t.unusable_kg, t.quantity_kg, t.indicated_kg].map(f64::to_bits));
        }
        if let Some(s) = &self.modern
            && let Ok(bytes) = serde_json::to_vec(s)
        {
            words.extend(bytes.into_iter().map(u64::from));
        }
        words
    }

    pub(crate) fn configure(
        r: &ResolvedAircraft,
        readout: Readout,
    ) -> Result<(Self, Option<SystemsModel>), SimError> {
        let Some(d) = &r.installed_systems.definition else {
            let mut state = Self::default();
            state.ideal_sensors(readout);
            return Ok((state, None));
        };
        d.validate()?;
        // ResolvedAircraft is public: native callers can mutate metadata after resolution.
        // Validate every input consumed here before installing any runtime state.
        flightcraft_aero::LayerFile::Engine(r.engine.clone()).validate()?;
        flightcraft_aero::modern::validate_installation(&r.engine, Some(d), &r.panel)?;
        r.variant.weight_balance(&r.load)?;
        let mut engine_model = r.aircraft.clone();
        engine_model.propulsion = r.engine.propulsion.clone();
        engine_model.validate()?;
        let mut s = Self {
            modern: (d.modern.is_some()
                || r.panel.equipment.is_some()
                || !matches!(
                    r.engine.control,
                    flightcraft_aero::modern::EngineControl::Magnetos
                ))
            .then(|| crate::modern::ModernState::new(&r.engine.control, d.modern.as_ref())),
            enabled: true,
            vacuum_installed: d.vacuum.is_some(),
            carb_heat_installed: r.engine.induction == Induction::Carburettor,
            ..Self::default()
        };
        if d.modern.as_ref().is_some_and(|m| !m.selector_both) {
            s.controls.fuel_selector = FuelSelector::Left;
        }
        if let Some(modern) = &mut s.modern
            && let Some(panel) = &r.panel.equipment
        {
            for e in &panel.elements {
                let initial = if e.appearance.binding.is_some() { 1. } else { 0. };
                modern.panel_controls.insert(e.id.clone(), initial);
                if let Some(binding) = e.appearance.binding {
                    modern.panel_bindings.insert(e.id.clone(), binding);
                }
                if matches!(
                    e.element,
                    flightcraft_aero::modern::PanelElement::Toggle { guarded: true, .. }
                        | flightcraft_aero::modern::PanelElement::FuelSelector
                ) {
                    modern.panel_guards.insert(e.id.clone(), false);
                }
            }
        }
        s.electrical.battery_capacity_ah = d.electrical.battery_capacity_ah;
        s.electrical.battery_voltage = d.electrical.battery_voltage;
        s.electrical.main_bus_voltage = d.electrical.alternator_voltage;
        s.electrical.essential_bus_voltage = d.electrical.alternator_voltage;
        s.electrical.loads = d
            .electrical
            .loads
            .iter()
            .map(|l| LoadState {
                id: l.id.clone(),
                breaker_closed: true,
                tripped: false,
                overload_s: 0.0,
                multiplier: 1.0,
                current_a: 0.0,
                powered: false,
                on_backup: false,
                backup_remaining_ah: l.backup_capacity_ah,
            })
            .collect();
        for tank in &d.tanks {
            let mass = r
                .load
                .stations
                .iter()
                .find(|l| l.station == tank.station)
                .map_or(0.0, |l| l.mass_kg);
            let capacity: f64 = d
                .tanks
                .iter()
                .filter(|t| t.station == tank.station)
                .map(|t| t.capacity_kg)
                .sum();
            let quantity = (mass * tank.capacity_kg / capacity).clamp(0.0, tank.capacity_kg);
            s.tanks.push(TankState {
                id: tank.id.clone(),
                wing: tank.wing,
                capacity_kg: tank.capacity_kg,
                unusable_kg: tank.unusable_kg,
                quantity_kg: quantity,
                indicated_kg: (quantity + tank.gauge_bias_kg).clamp(0.0, tank.capacity_kg),
            });
        }
        s.ports(d, readout, true);
        s.gyro_truth_rad = [readout.roll_rad, readout.pitch_rad, readout.heading_grid_rad];
        s.gyro_heading_reference_rad = readout.grid_north_true_rad - readout.magnetic_declination_rad;
        s.engine_rpm = readout.engine_rpm;
        let model = SystemsModel {
            definition: d.clone(),
            engine_control: r.engine.control.clone(),
            gearbox_ratio: r.engine.gearbox_ratio,
            navigator_load: r.panel.equipment.as_ref().and_then(|p| {
                p.elements.iter().find_map(|e| {
                    if let flightcraft_aero::modern::PanelElement::Navigator { load, .. } = &e.element {
                        Some(load.clone())
                    } else {
                        None
                    }
                })
            }),
            induction: r.engine.induction,
            fuel_flow: r.engine.fuel_flow_kg_s.clone(),
            idle_fraction: r.engine.propulsion.idle_power_w / r.engine.propulsion.max_power_w,
            power_density: r.engine.propulsion.power_density.clone(),
            variant: r.variant.clone(),
            load: r.load.clone(),
        };
        // Reset establishes electrical indications without consuming a simulation step.
        for (load, definition) in s.electrical.loads.iter_mut().zip(&d.electrical.loads) {
            let on = matches!(definition.switch, LoadSwitch::Always | LoadSwitch::Avionics);
            load.powered = on;
            load.current_a = if on { definition.amps } else { 0.0 };
        }
        s.refresh_electronic(&model, readout);
        s.engine_power_fraction = s.power(&model);
        Ok((s, Some(model)))
    }

    pub(crate) fn set(
        &mut self,
        setting: SystemSetting,
        model: Option<&SystemsModel>,
    ) -> Result<(), SimError> {
        let m = model.ok_or_else(|| invalid("aircraft has no operational systems definition"))?;
        match setting {
            SystemSetting::Equipment { setting } => {
                self.modern
                    .as_mut()
                    .ok_or_else(|| invalid("modern equipment not installed"))?
                    .set(setting)?;
                if let Some(s) = &self.modern
                    && s.electronic
                {
                    self.controls.starter = s.key == crate::modern::KeyPosition::Start;
                }
            }
            SystemSetting::Switch { switch, on } => {
                if self.modern.as_ref().is_some_and(|s| s.electronic)
                    && matches!(switch, SystemSwitch::Starter | SystemSwitch::FuelPump)
                {
                    return Err(invalid("use electronic key or pump controls"));
                }
                if switch == SystemSwitch::CarbHeat && m.induction != Induction::Carburettor {
                    return Err(invalid("carb heat is not installed"));
                }
                *match switch {
                    SystemSwitch::Master => &mut self.controls.master,
                    SystemSwitch::Avionics => &mut self.controls.avionics,
                    SystemSwitch::Alternator => &mut self.controls.alternator,
                    SystemSwitch::Lights => &mut self.controls.lights,
                    SystemSwitch::FuelPump => &mut self.controls.fuel_pump,
                    SystemSwitch::PitotHeat => &mut self.controls.pitot_heat,
                    SystemSwitch::AlternateStatic => &mut self.controls.alternate_static,
                    SystemSwitch::CarbHeat => &mut self.controls.carb_heat,
                    SystemSwitch::Starter => &mut self.controls.starter,
                } = on;
            }
            SystemSetting::Magnetos { position } => {
                if self.modern.as_ref().is_some_and(|s| s.electronic) {
                    return Err(invalid("magnetos not installed"));
                }
                self.controls.magnetos = position;
            }
            SystemSetting::FuelSelector { position } => {
                if position == FuelSelector::Both
                    && m.definition.modern.as_ref().is_some_and(|s| !s.selector_both)
                {
                    return Err(invalid("BOTH not installed"));
                }
                self.controls.fuel_selector = position;
            }
            SystemSetting::Mixture { value } => {
                if self.modern.as_ref().is_some_and(|s| s.electronic) {
                    return Err(invalid("mixture is electronically controlled"));
                }
                bounded(value, 0.0, 1.0)?;
                self.controls.mixture = value;
            }
            SystemSetting::HeadingBug { heading_rad } => {
                bounded(heading_rad, 0.0, core::f64::consts::TAU)?;
                self.controls.heading_bug_rad = heading_rad.rem_euclid(core::f64::consts::TAU);
            }
            SystemSetting::Altimeter { pressure_pa } => {
                bounded(pressure_pa, 80000.0, 110000.0)?;
                self.controls.altimeter_setting_pa = pressure_pa;
                self.sensed.altitude_msl_m = altitude(self.static_pressure_pa, pressure_pa);
            }
            SystemSetting::Primer { charge } => {
                if m.induction == Induction::Injection {
                    return Err(invalid("primer not installed"));
                }
                bounded(charge, 0.0, 1.0)?;
                self.primer_charge = charge;
            }
            SystemSetting::Environment {
                temperature_c,
                relative_humidity,
            } => {
                bounded(temperature_c, -60.0, 60.0)?;
                bounded(relative_humidity, 0.0, 1.0)?;
                self.controls.temperature_c = temperature_c;
                self.controls.relative_humidity = relative_humidity;
            }
            SystemSetting::Breaker { load, closed } => {
                let l = self
                    .electrical
                    .loads
                    .iter_mut()
                    .find(|l| l.id == load)
                    .ok_or_else(|| invalid("unknown breaker"))?;
                l.breaker_closed = closed;
                l.tripped = false;
                l.overload_s = 0.0;
            }
            SystemSetting::LoadMultiplier { load, value } => {
                bounded(value, 0.0, 10.0)?;
                let l = self
                    .electrical
                    .loads
                    .iter_mut()
                    .find(|l| l.id == load)
                    .ok_or_else(|| invalid("unknown load"))?;
                l.multiplier = value;
            }
            SystemSetting::TankQuantity { tank, mass_kg } => {
                let t = self
                    .tanks
                    .iter_mut()
                    .find(|t| t.id == tank)
                    .ok_or_else(|| invalid("unknown tank"))?;
                bounded(mass_kg, 0.0, t.capacity_kg)?;
                t.quantity_kg = mass_kg;
            }
        }
        self.engine_power_fraction = self.power(m);
        Ok(())
    }

    fn ignition_available(&self) -> bool {
        self.modern
            .as_ref()
            .filter(|s| s.electronic)
            .map_or(self.controls.magnetos != Magnetos::Off, |s| {
                s.key != crate::modern::KeyPosition::Off && (s.lane_a.running || s.lane_b.running)
            })
    }
    fn electronic_load_on(modern: Option<&crate::modern::ModernState>, m: &SystemsModel, id: &str) -> bool {
        if let Some(state) = modern
            && state.wigwag
            && let Some(w) = m.definition.modern.as_ref().and_then(|d| d.wigwag.as_ref())
            && (id == w.left_load || id == w.right_load)
        {
            let left = state.wigwag_elapsed_s < w.half_period_s;
            return (id == w.left_load) == left
                && state
                    .light_switches
                    .iter()
                    .any(|(s, on)| *s == LoadSwitch::Landing && *on);
        }
        if let Some(state) = modern
            && let Some(load) = m.definition.electrical.loads.iter().find(|l| l.id == id)
            && let Some((_, on)) = state
                .light_switches
                .iter()
                .find(|(light, _)| *light == load.switch)
        {
            return *on;
        }
        if let Some(s) = modern
            && let flightcraft_aero::modern::EngineControl::ElectronicDualLane {
                lane_a_load,
                lane_b_load,
                pump_1_load,
                pump_2_load,
                ..
            } = &m.engine_control
        {
            if id == lane_a_load {
                return s.lane_a.on;
            }
            if id == lane_b_load {
                return s.lane_b.on;
            }
            if id == pump_1_load {
                return s.pump_1.on;
            }
            if id == pump_2_load {
                return s.pump_2.on;
            }
        }
        true
    }
    fn refresh_electronic(&mut self, m: &SystemsModel, r: Readout) {
        let fuel = self.fuel_available(m) > 0.;
        let Some(s) = &mut self.modern else {
            return;
        };
        s.propeller_rpm = r.engine_rpm / m.gearbox_ratio;
        if let flightcraft_aero::modern::EngineControl::ElectronicDualLane {
            lane_a_load,
            lane_b_load,
            pump_1_load,
            pump_2_load,
            fuel_pressure_pa,
            manifold_pressure_fraction,
            ..
        } = &m.engine_control
        {
            let powered = |id: &str| self.electrical.loads.iter().any(|l| l.id == id && l.powered);
            s.lane_a.running = s.lane_a.on && !s.lane_a.failed && powered(lane_a_load);
            s.lane_b.running = s.lane_b.on && !s.lane_b.failed && powered(lane_b_load);
            s.lane_a.light = !s.lane_a.running;
            s.lane_b.light = !s.lane_b.running;
            s.engine_control_warning = s.lane_a.light || s.lane_b.light;
            s.pump_1.running = s.pump_1.on && !s.pump_1.failed && powered(pump_1_load);
            s.pump_2.running = s.pump_2.on && !s.pump_2.failed && powered(pump_2_load);
            s.fuel_pressure_pa = if fuel && (s.pump_1.running || s.pump_2.running) {
                *fuel_pressure_pa
            } else {
                0.
            };
            s.manifold_pressure_pa = atmosphere(r.altitude_msl_m).pressure_pa
                * if self.engine_running {
                    interpolate(&manifold_pressure_fraction.points, r.controls.throttle)
                } else {
                    1.
                };
        }
    }
    fn selected(&self, wing: Wing) -> bool {
        match self.controls.fuel_selector {
            FuelSelector::Off => false,
            FuelSelector::Both => true,
            FuelSelector::Left => wing == Wing::Left,
            FuelSelector::Right => wing == Wing::Right,
        }
    }
    fn fuel_available(&self, m: &SystemsModel) -> f64 {
        self.tanks
            .iter()
            .zip(&m.definition.tanks)
            .filter(|(_, d)| self.selected(d.wing))
            .map(|(t, _)| (t.quantity_kg - t.unusable_kg).max(0.0))
            .sum()
    }
    fn power(&self, m: &SystemsModel) -> f64 {
        if !self.engine_running
            || !self.ignition_available()
            || self.controls.mixture < 0.1
            || self.fuel_available(m) <= 0.0
            || self
                .modern
                .as_ref()
                .is_some_and(|s| s.electronic && s.fuel_pressure_pa <= 0.)
        {
            return 0.0;
        }
        let mags = if let Some(s) = &self.modern
            && s.electronic
        {
            if s.lane_a.running && s.lane_b.running {
                1.
            } else if let flightcraft_aero::modern::EngineControl::ElectronicDualLane {
                single_lane_power_fraction,
                ..
            } = &m.engine_control
            {
                *single_lane_power_fraction
            } else {
                0.
            }
        } else if self.controls.magnetos == Magnetos::Both {
            1.0
        } else {
            m.definition.engine.single_mag_power_fraction
        };
        // Full-rich is exactly one. Moderate leaning costs little power, then lean misfire.
        let mixture = if self.controls.mixture >= 0.7 {
            1.0 - (1.0 - self.controls.mixture) * 0.08
        } else {
            ((self.controls.mixture - 0.1) / 0.6).clamp(0.0, 1.0) * 0.976
        };
        let heat = if self.controls.carb_heat && m.induction == Induction::Carburettor {
            m.definition.engine.carb_heat_power_fraction
        } else {
            1.0
        };
        (mags * mixture * heat * (1.0 - self.carb_ice * 0.9)).clamp(0.0, 1.0)
    }

    pub(crate) fn step(&mut self, m: &SystemsModel, r: Readout, failures: Failures) {
        self.failures = failures;
        let smoking = failures.cabin_smoke
            || (failures.electrical_fire && self.controls.master)
            || (failures.engine_fire && self.engine_running);
        if smoking {
            self.smoke_density = (self.smoke_density + DT_S / 15.0).min(1.0);
        } else if self.smoke_density > 0.0 {
            self.smoke_density = (self.smoke_density - DT_S / 30.0).max(0.0);
        }
        self.engine_rpm = r.engine_rpm;
        self.electrical_step(m, r.engine_rpm, failures);
        self.refresh_electronic(m, r);
        let op = &m.definition.engine;
        let ambient = self.controls.temperature_c + atmosphere(r.altitude_msl_m).temperature_k - 288.15;
        self.fuel_starved = self.fuel_available(m) <= 0.0
            || self
                .modern
                .as_ref()
                .is_some_and(|s| s.electronic && s.fuel_pressure_pa <= 0.);
        if self.fuel_starved
            || !self.ignition_available()
            || self.controls.mixture < 0.1
            || failures.engine_power_fraction == 0.0
        {
            self.engine_running = false;
        }
        let cranking = self.controls.starter && self.electrical.main_bus_voltage > 9.0;
        if cranking && !self.engine_running {
            self.engine_rpm = self.engine_rpm.max(200.0);
        }
        if !self.engine_running
            && cranking
            && !self.fuel_starved
            && self.ignition_available()
            && self.controls.mixture >= 0.2
            && (self.primer_charge > 0.05 || self.cht_c > 60.0 || m.induction == Induction::Injection)
            && failures.engine_power_fraction > 0.0
        {
            self.cranking_s += DT_S;
            if self.cranking_s >= op.start_time_s {
                self.engine_running = true;
                self.cranking_s = 0.0;
            }
        } else {
            self.cranking_s = 0.0;
        }
        if cranking {
            self.primer_charge = (self.primer_charge - DT_S * 0.02).max(0.0);
        }
        if m.induction == Induction::Carburettor {
            if self.controls.carb_heat && self.engine_running {
                self.carb_ice = (self.carb_ice - op.carb_heat_clear_rate * DT_S).max(0.0);
            } else if self.engine_running {
                self.carb_ice = (self.carb_ice
                    + interpolate(&op.carb_ice_rate.points, ambient)
                        * interpolate(&op.carb_ice_humidity.points, self.controls.relative_humidity)
                        * (1.0 - 0.5 * r.controls.throttle)
                        * DT_S)
                    .min(1.0);
            }
        }
        if failures.carb_icing
            && m.induction == Induction::Carburettor
            && !self.controls.carb_heat
            && ambient > -15.0
            && ambient < 30.0
            && self.controls.relative_humidity > 0.5
        {
            self.carb_ice = (self.carb_ice + 0.025 * DT_S).min(1.0);
        }
        if failures.engine_fire {
            self.cht_c += 20.0 * DT_S;
            self.oil_temperature_c += 8.0 * DT_S;
        }
        self.engine_power_fraction = self.power(m);
        let shaft = (m.idle_fraction + (1.0 - m.idle_fraction) * r.controls.throttle)
            * self.engine_power_fraction
            * failures.engine_power_fraction
            * interpolate(
                &m.power_density.points,
                atmosphere(r.altitude_msl_m).density_kg_m3 / SEA_LEVEL_DENSITY,
            );
        if let Some(s) = &mut self.modern
            && let flightcraft_aero::modern::EngineControl::ElectronicDualLane {
                coolant_temperature_c,
                coolant_lag_s,
                ..
            } = &m.engine_control
        {
            lag(
                &mut s.coolant_temperature_c,
                if self.engine_running {
                    interpolate(&coolant_temperature_c.points, shaft.clamp(0., 1.))
                } else {
                    ambient
                },
                *coolant_lag_s,
            );
        }
        self.fuel_flow_kg_s = if self.engine_running {
            interpolate(&m.fuel_flow.points, shaft.clamp(0.0, 1.0)) * self.controls.mixture
        } else {
            0.0
        };
        let available = self.fuel_available(m);
        let draw = (self.fuel_flow_kg_s * DT_S).min(available);
        let selector = self.controls.fuel_selector;
        for (t, d) in self.tanks.iter_mut().zip(&m.definition.tanks) {
            let selected = match selector {
                FuelSelector::Off => false,
                FuelSelector::Both => true,
                FuelSelector::Left => d.wing == Wing::Left,
                FuelSelector::Right => d.wing == Wing::Right,
            };
            if selected && available > 0.0 {
                t.quantity_kg = (t.quantity_kg - draw * (t.quantity_kg - t.unusable_kg).max(0.0) / available)
                    .max(t.unusable_kg.min(t.quantity_kg));
            }
            lag(
                &mut t.indicated_kg,
                (t.quantity_kg + d.gauge_bias_kg).clamp(0.0, d.capacity_kg),
                d.gauge_lag_s,
            );
        }
        if self.fuel_available(m) <= 1e-12 {
            self.fuel_starved = true;
            self.engine_running = false;
            self.engine_power_fraction = 0.0;
            self.fuel_flow_kg_s = 0.0;
        }
        let peak = 1.0 - ((self.controls.mixture - 0.72) / 0.72).abs().min(1.0);
        lag(
            &mut self.egt_c,
            if self.engine_running {
                ambient + 450.0 + 350.0 * shaft + 120.0 * peak
            } else {
                ambient
            },
            op.egt_lag_s,
        );
        lag(
            &mut self.cht_c,
            if self.engine_running {
                ambient + 90.0 + 110.0 * shaft + 25.0 * peak
            } else {
                ambient
            },
            op.cht_lag_s,
        );
        // Original estimates: warm oil approaches 65–110 C; pressure follows the pump,
        // with a modest loss as viscosity falls. No type-specific measurements.
        lag(
            &mut self.oil_temperature_c,
            if self.engine_running {
                ambient + 50.0 + 45.0 * shaft
            } else {
                ambient
            },
            180.0,
        );
        let viscosity = (1.15 - (self.oil_temperature_c - 40.0) * 0.003).clamp(0.5, 1.2);
        let oil_target = if self.engine_running {
            (self.engine_rpm / 1800.0).clamp(0.0, 1.0) * 400000.0 * viscosity
        } else {
            0.0
        };
        lag(&mut self.oil_pressure_pa, oil_target, 2.0);
        self.sensed.turn_rate_rps = if self
            .electrical
            .loads
            .iter()
            .any(|l| l.id == "turn_coordinator" && l.powered)
        {
            r.turn_rate_rps
        } else {
            0.0
        };
        let [previous_roll, _, _] = self.gyro_truth_rad;
        self.sensed.turn_coordinator_rate_rps = if self
            .electrical
            .loads
            .iter()
            .any(|l| l.id == "turn_coordinator" && l.powered)
        {
            r.turn_rate_rps + 0.5773502691896257 * crate::autopilot::wrap(r.roll_rad - previous_roll) / DT_S
        } else {
            0.0
        };
        self.sensed.lateral_acceleration_g = r.lateral_acceleration_g;
        self.ports(&m.definition, r, false);
        self.gyro(&m.definition, r);
        if let Some(s) = &mut self.modern {
            s.recovery_step(m.definition.modern.as_ref());
            s.annunciations_step(
                m.definition.modern.as_ref(),
                self.electrical.main_bus_voltage,
                self.engine_running,
            );
        }
    }

    pub(crate) fn electrical_step(&mut self, m: &SystemsModel, rpm: f64, failures: Failures) {
        if let Some(state) = &mut self.modern
            && state.wigwag
            && let Some(w) = m.definition.modern.as_ref().and_then(|d| d.wigwag.as_ref())
        {
            state.wigwag_elapsed_s = libm::fmod(state.wigwag_elapsed_s + DT_S, w.half_period_s * 2.);
        }
        let feeds: Vec<_> = m
            .definition
            .modern
            .as_ref()
            .into_iter()
            .flat_map(|d| &d.backup_buses)
            .filter_map(|d| {
                self.modern
                    .as_ref()?
                    .backup_buses
                    .iter()
                    .find(|s| s.circuit == d.circuit && s.on && s.remaining_ah > 0.)
                    .map(|s| (d, s.remaining_ah))
            })
            .collect();
        let mut backup_draw = Vec::new();
        let unit_on = |id: &str| {
            Self::electronic_load_on(self.modern.as_ref(), m, id)
                && match id {
                    "electronic_attitude" => self.avionics.electronic_attitude.power.switch_on,
                    "electronic_hsi" => self.avionics.electronic_hsi.reversion_attitude.power.switch_on,
                    _ => true,
                }
        };
        let d = &m.definition.electrical;
        let c = &self.controls;
        let e = &mut self.electrical;
        let battery_available = c.master && e.battery_soc > 0.001;
        let alternator = if c.master && c.alternator && !failures.alternator {
            interpolate(&d.alternator_amps.points, rpm)
        } else {
            0.0
        };
        let supply = (battery_available || alternator > 0.0) && !failures.electrical_fire;
        let mut main_demand = if c.starter && supply {
            m.definition.engine.starter_amps
        } else {
            0.0
        };
        let mut essential_demand = 0.0;
        for (l, def) in e.loads.iter_mut().zip(&d.loads) {
            let switched = unit_on(&def.id)
                && match def.switch {
                    LoadSwitch::Avionics => c.avionics,
                    LoadSwitch::Lights => c.lights,
                    LoadSwitch::FuelPump => c.fuel_pump,
                    LoadSwitch::PitotHeat => c.pitot_heat,
                    LoadSwitch::Always
                    | LoadSwitch::Beacon
                    | LoadSwitch::Nav
                    | LoadSwitch::Strobe
                    | LoadSwitch::Taxi
                    | LoadSwitch::Landing => true,
                };
            let amps = def.amps * l.multiplier;
            let backup_supply = feeds.iter().any(|(b, _)| b.loads.contains(&def.id));
            if l.breaker_closed && (supply || backup_supply) && switched && amps > def.breaker_amps {
                l.overload_s += DT_S * ((amps / def.breaker_amps) * (amps / def.breaker_amps) - 1.0);
                if l.overload_s >= 1.0 {
                    l.breaker_closed = false;
                    l.tripped = true;
                }
            } else {
                l.overload_s = (l.overload_s - DT_S).max(0.0);
            }
            l.current_a = if supply && switched && l.breaker_closed {
                amps
            } else {
                0.0
            };
            match def.bus {
                Bus::Main => main_demand += l.current_a,
                Bus::Essential => essential_demand += l.current_a,
            }
        }
        // With no usable battery, shed the main bus before losing essential equipment.
        let main_on = supply && (battery_available || alternator >= main_demand + essential_demand);
        let essential_on = supply && (battery_available || alternator >= essential_demand);
        let demand =
            if main_on { main_demand } else { 0.0 } + if essential_on { essential_demand } else { 0.0 };
        let charge = if c.master {
            (alternator - demand)
                .clamp(0.0, 5.0)
                .min((1.0 - e.battery_soc) * d.battery_capacity_ah * 3600.0 / DT_S)
        } else {
            0.0
        };
        e.alternator_current_a = alternator.min(demand + charge);
        e.battery_current_a = if battery_available {
            demand - e.alternator_current_a
        } else {
            -charge
        };
        e.battery_soc =
            (e.battery_soc - e.battery_current_a * DT_S / (3600.0 * d.battery_capacity_ah)).clamp(0.0, 1.0);
        e.battery_voltage = if e.battery_soc > 0.001 {
            d.battery_voltage * (0.85 + 0.15 * e.battery_soc)
        } else {
            0.0
        };
        let voltage = if alternator >= demand && alternator > 0.0 {
            d.alternator_voltage
        } else if battery_available {
            (e.battery_voltage - 0.015 * e.battery_current_a.max(0.0)).max(0.0)
        } else {
            0.0
        };
        e.main_bus_voltage = if main_on { voltage } else { 0.0 };
        e.essential_bus_voltage = if essential_on { voltage } else { 0.0 };
        for (l, def) in e.loads.iter_mut().zip(&d.loads) {
            let bus = match def.bus {
                Bus::Main => e.main_bus_voltage,
                Bus::Essential => e.essential_bus_voltage,
            };
            let switched = unit_on(&def.id)
                && match def.switch {
                    LoadSwitch::Avionics => c.avionics,
                    LoadSwitch::Lights => c.lights,
                    LoadSwitch::FuelPump => c.fuel_pump,
                    LoadSwitch::PitotHeat => c.pitot_heat,
                    LoadSwitch::Always
                    | LoadSwitch::Beacon
                    | LoadSwitch::Nav
                    | LoadSwitch::Strobe
                    | LoadSwitch::Taxi
                    | LoadSwitch::Landing => true,
                };
            if bus < 9.0 {
                l.current_a = 0.0;
            }
            let feed = feeds.iter().find(|(b, _)| b.loads.contains(&def.id));
            let shared_backup = switched && l.breaker_closed && l.current_a == 0. && feed.is_some();
            l.on_backup = shared_backup || (switched && l.current_a == 0.0 && l.backup_remaining_ah > 0.0);
            l.powered = l.current_a > 0.0 || l.on_backup;
            if shared_backup {
                if let Some((bus, _)) = feed {
                    backup_draw.push((bus.circuit, def.amps * l.multiplier * DT_S / 3600.));
                }
            } else if l.on_backup {
                l.backup_remaining_ah = (l.backup_remaining_ah - def.amps * DT_S / 3600.0).max(0.0);
                l.powered = l.backup_remaining_ah > 0.0;
            } else if l.current_a > 0.0 {
                l.backup_remaining_ah =
                    (l.backup_remaining_ah + def.amps * 0.1 * DT_S / 3600.0).min(def.backup_capacity_ah);
            }
        }
        if let Some(state) = &mut self.modern {
            for bus in &mut state.backup_buses {
                let draw: f64 = backup_draw
                    .iter()
                    .filter(|(c, _)| *c == bus.circuit)
                    .map(|(_, ah)| ah)
                    .sum();
                bus.remaining_ah = (bus.remaining_ah - draw).max(0.);
                bus.voltage = if bus.on && bus.remaining_ah > 0. {
                    feeds
                        .iter()
                        .find(|(d, _)| d.circuit == bus.circuit)
                        .map_or(0., |(d, _)| d.voltage)
                } else {
                    0.
                };
            }
        }
        self.pitot_heat_powered = e
            .loads
            .iter()
            .zip(&d.loads)
            .any(|(l, def)| def.switch == LoadSwitch::PitotHeat && l.powered);
    }

    fn ports(&mut self, d: &SystemsDefinition, r: Readout, initialize: bool) {
        let mut a = atmosphere(r.altitude_msl_m);
        if (80000.0..=110000.0).contains(&r.sea_level_pressure_pa) {
            a.pressure_pa *= r.sea_level_pressure_pa / 101325.0;
        }
        let error = interpolate(
            &d.pitot_static.position_error_pa.points,
            r.effective_controls.flaps,
        );
        if initialize || !self.failures.pitot_blocked {
            self.pitot_total_pressure_pa =
                a.pressure_pa + 0.5 * SEA_LEVEL_DENSITY * r.indicated_airspeed_mps * r.indicated_airspeed_mps;
        }
        if initialize || self.controls.alternate_static || !self.failures.static_blocked {
            self.static_pressure_pa = (a.pressure_pa
                + if self.controls.alternate_static {
                    d.pitot_static.alternate_static_error_pa
                } else {
                    error
                })
            .max(1.0);
        }
        let pressure_alt = altitude(self.static_pressure_pa, 101325.0);
        let vertical = if initialize {
            r.vertical_speed_mps
        } else {
            (pressure_alt - self.pressure_altitude_m) / DT_S
        };
        if self.failures.static_blocked && !self.controls.alternate_static {
            self.sensed.vertical_speed_mps = 0.0;
        } else {
            lag(
                &mut self.sensed.vertical_speed_mps,
                vertical,
                d.pitot_static.vsi_lag_s,
            );
        }
        self.pressure_altitude_m = pressure_alt;
        self.sensed.indicated_airspeed_mps = libm::sqrt(
            (2.0 * (self.pitot_total_pressure_pa - self.static_pressure_pa) / SEA_LEVEL_DENSITY).max(0.0),
        );
        self.sensed.altitude_msl_m = altitude(self.static_pressure_pa, self.controls.altimeter_setting_pa);
        if initialize {
            self.sensed.turn_rate_rps = r.turn_rate_rps;
            self.sensed.turn_coordinator_rate_rps = r.turn_rate_rps;
            self.sensed.lateral_acceleration_g = r.lateral_acceleration_g;
            self.sensed.roll_rad = r.roll_rad;
            self.sensed.pitch_rad = r.pitch_rad;
            self.sensed.heading_rad = r.heading_rad;
            self.sensed.attitude_valid = true;
        }
    }

    fn gyro(&mut self, d: &SystemsDefinition, r: Readout) {
        let (drive, up, down, erection, drift) = if let Some(v) = &d.vacuum {
            self.suction_pa = if self.failures.vacuum_pump {
                0.0
            } else {
                interpolate(&v.suction_pa.points, r.engine_rpm)
            };
            (
                (self.suction_pa / v.nominal_suction_pa).clamp(0.0, 1.0),
                v.spin_up_s,
                v.spin_down_s,
                v.erection_rate_rps,
                v.drift_rps,
            )
        } else {
            let powered = d
                .electric_gyro_load
                .as_ref()
                .is_some_and(|id| self.electrical.loads.iter().any(|l| &l.id == id && l.powered));
            (if powered { 1.0 } else { 0.0 }, 10.0, 30.0, 0.01, 0.008)
        };
        let spin_time = if drive > self.gyro_spin { up } else { down };
        lag(&mut self.gyro_spin, drive, spin_time);
        let loss = 1.0 - self.gyro_spin;
        let truth = [r.roll_rad, r.pitch_rad, r.heading_grid_rad];
        for ((error, previous), current) in self
            .gyro_error_rad
            .iter_mut()
            .zip(&mut self.gyro_truth_rad)
            .zip(truth)
        {
            let delta = (current - *previous + core::f64::consts::PI).rem_euclid(core::f64::consts::TAU)
                - core::f64::consts::PI;
            *error -= delta * (1.0 - self.gyro_spin * self.gyro_spin);
            *previous = current;
        }
        let [roll, pitch, heading] = &mut self.gyro_error_rad;
        for (axis, (error, rate)) in [
            (roll, drift * loss * loss),
            (pitch, drift * 0.7 * loss * loss),
            (heading, drift * (0.005 + 1.5 * loss)),
        ]
        .into_iter()
        .enumerate()
        {
            // Erection levels roll and pitch; the free heading gyro retains slow drift.
            let correction = if axis == 2 {
                0.0
            } else {
                (*error).clamp(
                    -erection * self.gyro_spin * DT_S,
                    erection * self.gyro_spin * DT_S,
                )
            };
            *error = (*error + rate * DT_S - correction).clamp(-core::f64::consts::PI, core::f64::consts::PI);
        }
        let [roll, pitch, heading] = self.gyro_error_rad;
        self.sensed.roll_rad = (r.roll_rad + roll + core::f64::consts::PI).rem_euclid(core::f64::consts::TAU)
            - core::f64::consts::PI;
        self.sensed.pitch_rad =
            (r.pitch_rad + pitch).clamp(-core::f64::consts::FRAC_PI_2, core::f64::consts::FRAC_PI_2);
        self.sensed.heading_rad = (r.heading_grid_rad + heading + self.gyro_heading_reference_rad)
            .rem_euclid(core::f64::consts::TAU);
        self.sensed.attitude_valid = self.gyro_spin > 0.7;
    }
}
