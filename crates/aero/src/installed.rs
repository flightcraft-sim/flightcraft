//! Original lumped-parameter systems definitions. All quantities use SI, except RPM and Ah.
use crate::aircraft::{positive, require};
use crate::{AircraftError, Table};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Bus {
    Main,
    Essential,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoadSwitch {
    Avionics,
    Lights,
    FuelPump,
    PitotHeat,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Wing {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SystemsDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vacuum: Option<VacuumDefinition>,
    pub electrical: ElectricalDefinition,
    pub pitot_static: PitotStaticDefinition,
    #[schemars(length(min = 2, max = 16))]
    pub tanks: Vec<TankDefinition>,
    pub engine: EngineOperationDefinition,
    /// Electric attitude/heading sensor; its installation must name an electrical load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub electric_gyro_load: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VacuumDefinition {
    /// Pump suction (Pa below ambient) against engine RPM.
    pub suction_pa: Table,
    pub nominal_suction_pa: f64,
    pub spin_down_s: f64,
    pub spin_up_s: f64,
    pub erection_rate_rps: f64,
    pub drift_rps: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ElectricalDefinition {
    pub battery_capacity_ah: f64,
    pub battery_voltage: f64,
    pub alternator_voltage: f64,
    pub alternator_amps: Table,
    #[schemars(length(min = 1, max = 64))]
    pub loads: Vec<ElectricalLoad>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ElectricalLoad {
    pub id: String,
    pub bus: Bus,
    pub switch: LoadSwitch,
    pub amps: f64,
    pub breaker_amps: f64,
    /// Zero for equipment without an internal battery.
    pub backup_capacity_ah: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PitotStaticDefinition {
    /// Static pressure error (Pa) against normalized flap position.
    pub position_error_pa: Table,
    pub alternate_static_error_pa: f64,
    pub vsi_lag_s: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TankDefinition {
    pub id: String,
    pub wing: Wing,
    /// Load station supplying this tank. Shared stations are divided by tank capacity.
    pub station: String,
    pub capacity_kg: f64,
    pub unusable_kg: f64,
    pub gauge_lag_s: f64,
    pub gauge_bias_kg: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineOperationDefinition {
    pub single_mag_power_fraction: f64,
    pub carb_heat_power_fraction: f64,
    /// Icing accumulation per second against ambient temperature (degrees Celsius).
    pub carb_ice_rate: Table,
    /// Multiplier against relative humidity, 0..1.
    pub carb_ice_humidity: Table,
    pub carb_heat_clear_rate: f64,
    pub egt_lag_s: f64,
    pub cht_lag_s: f64,
    pub start_time_s: f64,
    pub starter_amps: f64,
}

impl SystemsDefinition {
    pub fn validate(&self) -> Result<(), AircraftError> {
        // LayerFile also checks finite numbers. This method is safe for direct Rust callers.
        crate::aircraft::finite_tree(
            &serde_json::to_value(self).map_err(|e| crate::aircraft::error("systems", e.to_string()))?,
            "systems",
        )?;
        let e = &self.electrical;
        for (name, value) in [
            ("battery_capacity_ah", e.battery_capacity_ah),
            ("battery_voltage", e.battery_voltage),
            ("alternator_voltage", e.alternator_voltage),
            ("vsi_lag_s", self.pitot_static.vsi_lag_s),
        ] {
            positive(value, name)?;
        }
        require(
            e.alternator_voltage > e.battery_voltage,
            "alternator_voltage",
            "must exceed battery voltage",
        )?;
        e.alternator_amps.validate("alternator_amps")?;
        require(
            e.alternator_amps
                .points
                .iter()
                .all(|(x, y)| *x >= 0.0 && *y >= 0.0 && *y <= 1000.0),
            "alternator_amps",
            "requires nonnegative RPM and 0..1000 A",
        )?;
        self.pitot_static
            .position_error_pa
            .validate("position_error_pa")?;
        require(
            self.pitot_static
                .position_error_pa
                .points
                .iter()
                .all(|(x, y)| (0.0..=1.0).contains(x) && y.abs() <= 5000.0)
                && self.pitot_static.alternate_static_error_pa.abs() <= 5000.0,
            "position_error_pa",
            "flaps 0..1 and pressure errors within 5000 Pa",
        )?;
        let mut ids = BTreeSet::new();
        require((1..=64).contains(&e.loads.len()), "loads", "requires 1..64 loads")?;
        for l in &e.loads {
            super::layers::id(&l.id, "load.id")?;
            require(ids.insert(&l.id), "loads", "duplicate load id")?;
            positive(l.amps, "load.amps")?;
            positive(l.breaker_amps, "load.breaker_amps")?;
            require(
                l.amps <= 1000.0 && l.breaker_amps <= 1000.0 && (0.0..=100.0).contains(&l.backup_capacity_ah),
                "load",
                "current or backup capacity out of range",
            )?;
        }
        if let Some(id) = &self.electric_gyro_load {
            require(ids.contains(id), "electric_gyro_load", "unknown load")?;
        }
        require(
            self.vacuum.is_some() != self.electric_gyro_load.is_some(),
            "gyro",
            "declare either vacuum or electric gyros",
        )?;
        if let Some(v) = &self.vacuum {
            v.suction_pa.validate("suction_pa")?;
            require(
                v.suction_pa
                    .points
                    .iter()
                    .all(|(x, y)| *x >= 0.0 && *y >= 0.0 && *y <= 100_000.0),
                "suction_pa",
                "requires nonnegative RPM and suction within 100 kPa",
            )?;
            for (name, value) in [
                ("nominal_suction_pa", v.nominal_suction_pa),
                ("spin_down_s", v.spin_down_s),
                ("spin_up_s", v.spin_up_s),
                ("erection_rate_rps", v.erection_rate_rps),
                ("drift_rps", v.drift_rps),
            ] {
                positive(value, name)?;
            }
        }
        ids.clear();
        require(
            (2..=16).contains(&self.tanks.len()),
            "tanks",
            "requires 2..16 tanks",
        )?;
        for t in &self.tanks {
            super::layers::id(&t.id, "tank.id")?;
            super::layers::id(&t.station, "tank.station")?;
            require(ids.insert(&t.id), "tanks", "duplicate tank id")?;
            positive(t.capacity_kg, "capacity_kg")?;
            positive(t.gauge_lag_s, "gauge_lag_s")?;
            require(
                (0.0..t.capacity_kg).contains(&t.unusable_kg) && t.gauge_bias_kg.abs() <= t.capacity_kg,
                "tank",
                "unusable fuel or gauge bias out of range",
            )?;
        }
        require(
            self.tanks.iter().any(|t| t.wing == Wing::Left)
                && self.tanks.iter().any(|t| t.wing == Wing::Right),
            "tanks",
            "requires tanks in both wings",
        )?;
        let op = &self.engine;
        for (name, value) in [
            ("single_mag_power_fraction", op.single_mag_power_fraction),
            ("carb_heat_power_fraction", op.carb_heat_power_fraction),
        ] {
            require((0.5..=1.0).contains(&value), name, "requires 0.5..1")?;
        }
        for (name, value) in [
            ("carb_heat_clear_rate", op.carb_heat_clear_rate),
            ("egt_lag_s", op.egt_lag_s),
            ("cht_lag_s", op.cht_lag_s),
            ("start_time_s", op.start_time_s),
            ("starter_amps", op.starter_amps),
        ] {
            positive(value, name)?;
        }
        for table in [&op.carb_ice_rate, &op.carb_ice_humidity] {
            table.validate("carb_ice")?;
            require(
                table.points.iter().all(|(_, y)| (0.0..=1.0).contains(y)),
                "carb_ice",
                "rate and humidity multipliers require 0..1",
            )?;
        }
        Ok(())
    }
}
