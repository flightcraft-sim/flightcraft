//! Reading simulator state for checklist verification.
use super::runup::RunupObserver;
use crate::Session;
use flightcraft_aero::checklists::{LaneId, PumpId, Verify};
use flightcraft_aero::panel_fidelity::BackupCircuit;
use flightcraft_sim::systems::{FuelSelector, Magnetos};

/// A snapshot of the state a checklist can ask about.
#[derive(Clone, Debug, Default)]
pub(crate) struct Probe {
    pub time_s: f64,
    pub on_ground: bool,
    pub master: bool,
    pub avionics: bool,
    pub electronic: bool,
    pub key_on: bool,
    pub magnetos_both: bool,
    pub lane_a_on: bool,
    pub lane_b_on: bool,
    pub pump1_on: bool,
    pub pump2_on: bool,
    pub backup: Vec<(BackupCircuit, bool, f64)>,
    pub fuel_selector: Option<FuelSelector>,
    pub panel: std::collections::BTreeMap<String, f64>,
    pub rpm: f64,
    pub oil_temp_c: f64,
    pub oil_pressure_bar: f64,
    pub fuel_pressure_kpa: f64,
    pub bus_voltage: f64,
    pub engine_running: bool,
    pub starter: bool,
    pub brake: f64,
    pub flaps: f64,
    pub trim: f64,
    pub throttle: f64,
    pub canopy_latched: bool,
    pub elevator: f64,
    pub aileron: f64,
    pub rudder: f64,
}

impl Probe {
    pub fn read(s: &Session) -> Self {
        let sys = s.sim.systems();
        let c = s.sim.controls();
        let modern = sys.modern.as_ref();
        let electronic = modern.is_some_and(|m| m.electronic);
        Self {
            time_s: s.sim.state().time_s,
            on_ground: s.sim.readout().on_ground,
            master: sys.controls.master,
            avionics: sys.controls.avionics,
            electronic,
            key_on: modern.is_some_and(|m| m.key != flightcraft_sim::modern::KeyPosition::Off),
            magnetos_both: sys.controls.magnetos == Magnetos::Both,
            lane_a_on: modern.is_some_and(|m| m.lane_a.on),
            lane_b_on: modern.is_some_and(|m| m.lane_b.on),
            pump1_on: modern.is_some_and(|m| m.pump_1.on),
            pump2_on: modern.is_some_and(|m| m.pump_2.on),
            backup: modern
                .map(|m| {
                    m.backup_buses
                        .iter()
                        .map(|b| (b.circuit, b.on, b.voltage))
                        .collect()
                })
                .unwrap_or_default(),
            fuel_selector: Some(sys.controls.fuel_selector),
            panel: modern.map(|m| m.panel_controls.clone()).unwrap_or_default(),
            rpm: sys.engine_rpm,
            oil_temp_c: sys.oil_temperature_c,
            oil_pressure_bar: sys.oil_pressure_pa / 100_000.,
            fuel_pressure_kpa: modern.map_or(0., |m| m.fuel_pressure_pa / 1000.),
            bus_voltage: sys.electrical.main_bus_voltage,
            engine_running: sys.engine_running,
            starter: sys.controls.starter,
            brake: c.brake,
            flaps: c.flaps,
            trim: sys.controls.trim,
            throttle: c.throttle,
            canopy_latched: s.sim.canopy().secure(),
            elevator: c.elevator,
            aileron: c.aileron,
            rudder: c.rudder,
        }
    }
}

fn within(v: f64, min: Option<f64>, max: Option<f64>) -> bool {
    min.is_none_or(|m| v >= m) && max.is_none_or(|m| v <= m)
}

/// Whether `v` holds for the probe. Run-up checks consult the observer.
pub(crate) fn eval(v: &Verify, p: &Probe, obs: &RunupObserver) -> bool {
    match v {
        Verify::Master { on } => p.master == *on,
        Verify::Avionics { on } => p.avionics == *on,
        Verify::Ignition { on } => {
            let live = if p.electronic {
                p.key_on && p.lane_a_on && p.lane_b_on
            } else {
                p.magnetos_both
            };
            let dead = if p.electronic {
                !p.key_on || (!p.lane_a_on && !p.lane_b_on)
            } else {
                !p.magnetos_both
            };
            if *on { live } else { dead }
        }
        Verify::Lane { lane, on } => {
            (if *lane == LaneId::A {
                p.lane_a_on
            } else {
                p.lane_b_on
            }) == *on
        }
        Verify::Pump { pump, on } => {
            (if *pump == PumpId::One {
                p.pump1_on
            } else {
                p.pump2_on
            }) == *on
        }
        Verify::BackupBattery {
            circuit,
            on,
            min_voltage,
        } => p
            .backup
            .iter()
            .find(|(c, _, _)| c == circuit)
            .is_some_and(|(_, bus_on, volts)| {
                bus_on == on && (!*on || min_voltage.is_none_or(|m| *volts >= m))
            }),
        Verify::FuelSelector { positions } => p.fuel_selector.is_some_and(|f| {
            let name = match f {
                FuelSelector::Off => "off",
                FuelSelector::Left => "left",
                FuelSelector::Right => "right",
                FuelSelector::Both => "both",
            };
            positions.iter().any(|x| x == name)
        }),
        Verify::Panel { control, min, max } => p.panel.get(control).is_some_and(|v| within(*v, *min, *max)),
        Verify::Rpm { min, max } => within(p.rpm, *min, *max),
        Verify::OilTemp { min_c, max_c } => within(p.oil_temp_c, *min_c, *max_c),
        Verify::OilPressure { min_bar, max_bar } => within(p.oil_pressure_bar, *min_bar, *max_bar),
        Verify::FuelPressure { min_kpa } => p.fuel_pressure_kpa >= *min_kpa,
        Verify::BusVoltage { min_v, max_v } => within(p.bus_voltage, *min_v, *max_v),
        Verify::EngineRunning { running } => p.engine_running == *running,
        Verify::Starter { engaged } => p.starter == *engaged,
        Verify::Brake { set } => (p.brake >= 0.5) == *set,
        Verify::Flaps { min, max } => within(p.flaps, *min, *max),
        Verify::TrimNeutral { tolerance } => p.trim.abs() <= *tolerance,
        Verify::Throttle { min, max } => within(p.throttle, *min, *max),
        Verify::CanopyLatched { latched } => p.canopy_latched == *latched,
        Verify::Check(check) => obs.judge(check, p.time_s) == Some(true),
        Verify::All { of } => of.iter().all(|v| eval(v, p, obs)),
    }
}

/// A recorded run-up measurement that fails its limit (as opposed to one not yet taken).
pub(crate) fn failed_check(v: &Verify, p: &Probe, obs: &RunupObserver) -> Option<String> {
    match v {
        Verify::Check(check) if obs.judge(check, p.time_s) == Some(false) => obs.describe(check),
        Verify::All { of } => of.iter().find_map(|v| failed_check(v, p, obs)),
        _ => None,
    }
}
