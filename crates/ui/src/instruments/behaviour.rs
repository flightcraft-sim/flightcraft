//! Deterministic presentation models. Inputs contain sensor outputs, never flight truth.
use super::{DrawItem, DrawList, Ink, Instrument, InstrumentInput};
use flightcraft_sim::systems::SystemsState;
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy)]
struct Specification {
    id: &'static str,
    title: &'static str,
    unit: &'static str,
    min: f64,
    max: f64,
    lag_s: f64,
}

const SPECS: &[Specification] = &[
    Specification {
        id: "airspeed",
        title: "AIRSPEED",
        unit: "kt IAS",
        min: 0.,
        max: 200.,
        lag_s: 0.25,
    },
    Specification {
        id: "attitude",
        title: "ATTITUDE",
        unit: "deg",
        min: -PI,
        max: PI,
        lag_s: 0.15,
    },
    Specification {
        id: "altimeter",
        title: "ALTIMETER",
        unit: "ft MSL",
        min: -1000.,
        max: 30000.,
        lag_s: 0.3,
    },
    Specification {
        id: "turn_coordinator",
        title: "TURN COORDINATOR",
        unit: "deg/s",
        min: -6.,
        max: 6.,
        lag_s: 0.5,
    },
    Specification {
        id: "heading",
        title: "HEADING",
        unit: "deg",
        min: 0.,
        max: 360.,
        lag_s: 0.3,
    },
    Specification {
        id: "vertical_speed",
        title: "VERTICAL SPEED",
        unit: "ft/min",
        min: -2000.,
        max: 2000.,
        lag_s: 0.7,
    },
    Specification {
        id: "tachometer",
        title: "TACHOMETER",
        unit: "RPM",
        min: 0.,
        max: 3500.,
        lag_s: 0.3,
    },
    Specification {
        id: "oil_temperature",
        title: "OIL TEMP",
        unit: "C",
        min: -40.,
        max: 160.,
        lag_s: 1.,
    },
    Specification {
        id: "oil_pressure",
        title: "OIL PRESS",
        unit: "kPa",
        min: 0.,
        max: 700.,
        lag_s: 0.3,
    },
    Specification {
        id: "egt",
        title: "EGT",
        unit: "C",
        min: 0.,
        max: 1200.,
        lag_s: 0.5,
    },
    Specification {
        id: "cht",
        title: "CHT",
        unit: "C",
        min: 0.,
        max: 350.,
        lag_s: 0.5,
    },
    Specification {
        id: "fuel_left",
        title: "LEFT FUEL",
        unit: "kg",
        min: 0.,
        max: 100.,
        lag_s: 1.,
    },
    Specification {
        id: "fuel_right",
        title: "RIGHT FUEL",
        unit: "kg",
        min: 0.,
        max: 100.,
        lag_s: 1.,
    },
    Specification {
        id: "fuel_flow",
        title: "FUEL FLOW",
        unit: "kg/h",
        min: 0.,
        max: 80.,
        lag_s: 0.5,
    },
    Specification {
        id: "ammeter",
        title: "AMMETER",
        unit: "A +charge",
        min: -80.,
        max: 20.,
        lag_s: 0.4,
    },
    Specification {
        id: "volts",
        title: "MAIN BUS",
        unit: "V",
        min: 0.,
        max: 32.,
        lag_s: 0.2,
    },
    Specification {
        id: "suction",
        title: "SUCTION",
        unit: "kPa",
        min: 0.,
        max: 25.,
        lag_s: 0.5,
    },
    Specification {
        id: "annunciators",
        title: "WARNINGS",
        unit: "",
        min: 0.,
        max: 1.,
        lag_s: 0.,
    },
];

/// Exact held-input first-order response, with bounded travel and shortest-path heading motion.
#[derive(Default)]
struct Needle {
    value: f64,
    time_s: Option<f64>,
}
impl Needle {
    fn step(&mut self, target: f64, time_s: f64, spec: Specification, circular: bool) -> f64 {
        let target = if target.is_finite() {
            target.clamp(spec.min, spec.max)
        } else {
            spec.min
        };
        if !time_s.is_finite() || time_s < 0. {
            return self.value;
        }
        if let Some(previous) = self.time_s.filter(|previous| *previous <= time_s) {
            let delta = if circular {
                (target - self.value + 180.).rem_euclid(360.) - 180.
            } else {
                target - self.value
            };
            let gain = if spec.lag_s > 0. {
                1. - libm::exp(-(time_s - previous) / spec.lag_s)
            } else {
                1.
            };
            self.value += delta * gain;
            if circular {
                self.value = self.value.rem_euclid(360.);
            }
        } else {
            self.value = target;
        }
        self.time_s = Some(time_s);
        self.value = self.value.clamp(spec.min, spec.max);
        self.value
    }
}

pub(super) fn create(id: &str) -> Result<Box<dyn Instrument>, String> {
    let spec = SPECS
        .iter()
        .find(|s| s.id == id)
        .copied()
        .ok_or_else(|| format!("Unknown instrument module: {id}"))?;
    Ok(Box::new(RoundGauge {
        spec,
        needle: Needle::default(),
        pitch: Needle::default(),
        ball: Needle::default(),
    }))
}

struct RoundGauge {
    spec: Specification,
    needle: Needle,
    pitch: Needle,
    ball: Needle,
}

fn value(id: &str, s: &SystemsState) -> f64 {
    match id {
        "airspeed" => s.sensed.indicated_airspeed_mps * 1.943844,
        "attitude" => s.sensed.roll_rad,
        "altimeter" => s.sensed.altitude_msl_m * 3.28084,
        "turn_coordinator" => s.sensed.turn_rate_rps.to_degrees(),
        "heading" => s.sensed.heading_rad.to_degrees().rem_euclid(360.),
        "vertical_speed" => s.sensed.vertical_speed_mps * 196.8504,
        "tachometer" => s.engine_rpm,
        "oil_temperature" => s.oil_temperature_c,
        "oil_pressure" => s.oil_pressure_pa / 1000.,
        "egt" => s.egt_c,
        "cht" => s.cht_c,
        "fuel_left" | "fuel_right" => s
            .tanks
            .iter()
            .filter(|t| (t.wing == flightcraft_aero::Wing::Left) == (id == "fuel_left"))
            .map(|t| t.indicated_kg)
            .sum(),
        "fuel_flow" => s.fuel_flow_kg_s * 3600.,
        "ammeter" => -s.electrical.battery_current_a,
        "volts" => s.electrical.main_bus_voltage,
        "suction" => s.suction_pa / 1000.,
        _ => 0.,
    }
}
impl Instrument for RoundGauge {
    fn update(&mut self, input: InstrumentInput<'_>) -> DrawList {
        let s = input.systems;
        let mut spec = self.spec;
        if matches!(spec.id, "fuel_left" | "fuel_right") {
            spec.max = s
                .tanks
                .iter()
                .filter(|t| (t.wing == flightcraft_aero::Wing::Left) == (spec.id == "fuel_left"))
                .map(|t| t.capacity_kg)
                .sum::<f64>()
                .max(1.);
        }
        let v = self
            .needle
            .step(value(spec.id, s), input.time_s, spec, spec.id == "heading");
        let valid = match spec.id {
            "attitude" | "heading" => s.sensed.attitude_valid,
            "turn_coordinator" => s
                .electrical
                .loads
                .iter()
                .any(|l| l.id == "turn_coordinator" && l.powered),
            _ => true,
        };
        let mut d = DrawList {
            size: [200., 180.],
            items: Vec::new(),
            indicated: v,
            valid,
        };
        let small = !matches!(
            spec.id,
            "airspeed"
                | "attitude"
                | "altimeter"
                | "turn_coordinator"
                | "heading"
                | "vertical_speed"
                | "tachometer"
        );
        if small && spec.id != "annunciators" {
            cluster(&mut d, spec, v, input);
            return d;
        }
        if spec.id != "annunciators" {
            d.circle([100., 88.], 66., Ink::Face);
        }
        d.label([100., 12.], spec.title, if small { 18. } else { 11. }, Ink::Muted);
        match spec.id {
            "vertical_speed" => vsi(&mut d, v),
            "tachometer" => tachometer(
                &mut d,
                v,
                input
                    .aircraft
                    .map_or(2800., |a| a.engine.propulsion.max_rpm * a.engine.gearbox_ratio),
            ),
            "airspeed" => airspeed(&mut d, v, input.aircraft.map(|a| &a.variant.v_speeds)),
            "attitude" => {
                let pitch = self.pitch.step(
                    s.sensed.pitch_rad,
                    input.time_s,
                    Specification {
                        min: -PI / 2.,
                        max: PI / 2.,
                        ..spec
                    },
                    false,
                );
                attitude(&mut d, v, pitch);
                if valid && let Some(ap) = input.autopilot.filter(|ap| ap.flight_director) {
                    let x = 100. + ((ap.command_bank_rad - s.sensed.roll_rad) * 70.).clamp(-30., 30.) as f32;
                    let y =
                        88. - ((ap.command_pitch_rad - s.sensed.pitch_rad) * 140.).clamp(-30., 30.) as f32;
                    d.line([x, 64.], [x, 112.], Ink::Needle, 3.);
                    d.line([76., y], [124., y], Ink::Needle, 3.);
                }
            }
            "altimeter" => altimeter(&mut d, v, s.controls.altimeter_setting_pa, input.options),
            "heading" => {
                for tick in 0..72 {
                    let angle = f64::from(tick) * TAU / 72. - PI / 2. - v.to_radians();
                    d.line(
                        polar(63., angle),
                        polar(if tick % 6 == 0 { 54. } else { 59. }, angle),
                        Ink::Text,
                        1.,
                    );
                    if tick % 6 == 0 {
                        let label = match tick {
                            0 => "N".into(),
                            18 => "E".into(),
                            36 => "S".into(),
                            54 => "W".into(),
                            _ => (tick / 2).to_string(),
                        };
                        d.label(polar(44., angle), label, 12., Ink::Text);
                    }
                }
                super::heading::marker(
                    &mut d,
                    [100., 88.],
                    62.,
                    s.controls.heading_bug_rad,
                    v.to_radians(),
                    PI,
                );
                d.line([100., 24.], [100., 32.], Ink::Needle, 3.);
                super::heading::knob_art(&mut d, [155., 143.], 9.);
                d.label([181., 143.], "HDG", 7., Ink::Needle);
            }
            "turn_coordinator" => {
                let ball = self.ball.step(
                    -s.sensed.lateral_acceleration_g / 0.3,
                    input.time_s,
                    Specification {
                        min: -1.,
                        max: 1.,
                        lag_s: 0.25,
                        ..spec
                    },
                    false,
                );
                turn(&mut d, v, ball);
            }
            "annunciators" => {
                let low_fuel = s.tanks.iter().any(|t| t.indicated_kg < t.capacity_kg * 0.15);
                let lamps = [
                    (
                        "LOW VOLT",
                        s.electrical.battery_capacity_ah > 0.,
                        s.electrical.main_bus_voltage < 13.,
                    ),
                    ("LOW VAC", s.vacuum_installed, s.suction_pa < 10000.),
                    ("LOW FUEL", !s.tanks.is_empty(), low_fuel),
                    // 20 psi: a normal idle (about 21 psi) does not light it.
                    ("OIL PRESS", s.enabled, s.oil_pressure_pa < 137_900.),
                ];
                for (row, (label, _, active)) in lamps
                    .into_iter()
                    .filter(|(_, installed, _)| *installed)
                    .enumerate()
                {
                    let y = 47. + row as f32 * 24.;
                    d.items.push(DrawItem::Polygon {
                        points: vec![[20., y - 10.], [180., y - 10.], [180., y + 10.], [20., y + 10.]],
                        ink: Ink::Face,
                    });
                    d.label(
                        [100., y],
                        label,
                        14.,
                        if active { Ink::Warning } else { Ink::Inactive },
                    );
                }
            }
            _ => {
                for i in 0..=20 {
                    let f = f64::from(i) / 20.;
                    let angle = (-240. + f * 300.).to_radians();
                    d.line(
                        polar(62., angle),
                        polar(if i % 4 == 0 { 53. } else { 57. }, angle),
                        Ink::Muted,
                        1.,
                    );
                    if i % 4 == 0 && !small {
                        d.label(
                            polar(42., angle),
                            format!("{:.0}", spec.min + f * (spec.max - spec.min)),
                            8.,
                            Ink::Muted,
                        );
                    }
                }
                let angle = (-240. + (v - spec.min) / (spec.max - spec.min) * 300.).to_radians();
                d.line([100., 88.], polar(50., angle), Ink::Needle, 3.);
                d.circle([100., 88.], 3., Ink::Text);
                d.label([100., 116.], spec.unit, if small { 14. } else { 9. }, Ink::Muted);
            }
        }
        if spec.id != "annunciators" {
            if valid {
                d.label(
                    [100., 168.],
                    if spec.id == "heading" {
                        format!("HDG {}", super::heading::degrees_text(s.controls.heading_bug_rad))
                    } else {
                        format!("{v:.0} {}", spec.unit)
                    },
                    13.,
                    Ink::Text,
                );
            } else {
                d.label([100., 168.], "GYRO INVALID", 13., Ink::Warning);
            }
        }
        d
    }
}

fn polar(radius: f64, angle: f64) -> [f32; 2] {
    [
        100. + (radius * libm::cos(angle)) as f32,
        88. + (radius * libm::sin(angle)) as f32,
    ]
}
fn attitude(d: &mut DrawList, roll: f64, pitch: f64) {
    let radius = 63.;
    d.circle([100., 88.], radius, Ink::Sky);
    let normal = [libm::sin(roll), libm::cos(roll)];
    let offset = pitch.clamp(-PI / 2., PI / 2.) * radius as f64 / 0.65;
    let ring: Vec<_> = (0..=96)
        .map(|i| polar(radius as f64, f64::from(i) * TAU / 96.))
        .collect();
    let distance =
        |p: [f32; 2]| f64::from(p[0] - 100.) * normal[0] + f64::from(p[1] - 88.) * normal[1] - offset;
    let mut polygon = Vec::new();
    for pair in ring.windows(2) {
        if let [a, b] = pair {
            let da = distance(*a);
            let db = distance(*b);
            if da >= 0. {
                polygon.push(*a);
            }
            if (da >= 0.) != (db >= 0.) {
                let f = (da / (da - db)) as f32;
                polygon.push([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]);
            }
        }
    }
    if polygon.len() >= 3 {
        d.items.push(DrawItem::Polygon {
            points: polygon,
            ink: Ink::Earth,
        });
    }
    for degrees in [-20_f64, -10., 0., 10., 20.] {
        let y = offset - degrees.to_radians() * radius as f64 / 0.65;
        if y.abs() < 60. {
            let half = if degrees == 0. {
                libm::sqrt(60. * 60. - y * y)
            } else {
                16.
            };
            let at = [100. + normal[0] * y, 88. + normal[1] * y];
            let right = [normal[1] * half, -normal[0] * half];
            d.line(
                [(at[0] - right[0]) as f32, (at[1] - right[1]) as f32],
                [(at[0] + right[0]) as f32, (at[1] + right[1]) as f32],
                Ink::Text,
                1.,
            );
        }
    }
    d.line([65., 88.], [90., 88.], Ink::Warning, 3.);
    d.line([110., 88.], [135., 88.], Ink::Warning, 3.);
    d.circle([100., 88.], 3., Ink::Warning);
}
fn turn(d: &mut DrawList, rate: f64, ball: f64) {
    let angle = (rate / 3.).clamp(-2., 2.) * PI / 6.;
    // The lower left and right marks identify the standard-rate bank in each direction.
    for (side, label) in [(-1., "L"), (1., "R")] {
        let a = if side < 0. { 5. * PI / 6. } else { PI / 6. };
        let point = |r| [100. + (r * libm::cos(a)) as f32, 76. + (r * libm::sin(a)) as f32];
        d.line(point(46.), point(55.), Ink::Text, 2.);
        let [x, y] = point(58.);
        d.label([x, y + 6.], label, 10., Ink::Muted);
    }
    d.label([100., 45.], "2 MIN", 10., Ink::Muted);
    let rotate = |x: f64, y: f64| {
        [
            100. + (x * libm::cos(angle) - y * libm::sin(angle)) as f32,
            76. + (x * libm::sin(angle) + y * libm::cos(angle)) as f32,
        ]
    };
    for (a, b) in [
        ((-41., 0.), (-8., 0.)),
        ((-8., 0.), (0., 5.)),
        ((0., 5.), (8., 0.)),
        ((8., 0.), (41., 0.)),
        ((0., -13.), (0., 12.)),
        ((-11., 10.), (11., 10.)),
    ] {
        d.line(rotate(a.0, a.1), rotate(b.0, b.1), Ink::Needle, 2.5);
    }
    d.line([62., 125.], [138., 125.], Ink::Muted, 10.);
    for x in [91., 109.] {
        d.line([x, 116.], [x, 134.], Ink::Text, 1.);
    }
    d.circle([100. + ball as f32 * 36., 125.], 5., Ink::Text);
}

/// Clockwise dial angles: hundreds, thousands, and ten-thousands, with zero at twelve.
pub(super) fn altimeter_angles(feet: f64) -> [f64; 3] {
    [1000., 10000., 100000.].map(|period| (feet / period).rem_euclid(1.) * TAU - PI / 2.)
}
fn altimeter(d: &mut DrawList, feet: f64, pressure_pa: f64, options: super::DisplayOptions) {
    for tick in 0..50 {
        let angle = f64::from(tick) * TAU / 50. - PI / 2.;
        d.line(
            polar(63., angle),
            polar(if tick % 5 == 0 { 55. } else { 59. }, angle),
            Ink::Text,
            1.,
        );
        if tick % 5 == 0 {
            d.label(polar(46., angle), (tick / 5).to_string(), 13., Ink::Text);
        }
    }
    d.label([100., 65.], "ALT", 8., Ink::Muted);
    let [hundreds, thousands, ten_thousands] = altimeter_angles(feet);
    d.line([100., 88.], polar(33., thousands), Ink::Text, 5.);
    d.line([100., 88.], polar(57., hundreds), Ink::Needle, 2.);
    let tip = polar(51., ten_thousands);
    d.items.push(DrawItem::Polygon {
        points: vec![
            tip,
            polar(42., ten_thousands - 0.10),
            polar(42., ten_thousands + 0.10),
        ],
        ink: Ink::Warning,
    });
    d.circle([100., 88.], 3., Ink::Text);
    d.items.push(DrawItem::Polygon {
        points: vec![[108., 90.], [139., 90.], [139., 107.], [108., 107.]],
        ink: Ink::Face,
    });
    for (a, b) in [
        ([108., 90.], [139., 90.]),
        ([139., 90.], [139., 107.]),
        ([139., 107.], [108., 107.]),
        ([108., 107.], [108., 90.]),
    ] {
        d.line(a, b, Ink::Muted, 0.7);
    }
    d.label(
        [124., 99.],
        options.pressure_value(pressure_pa),
        11.,
        Ink::Warning,
    );
    d.label([124., 112.], options.pressure_label(), 8., Ink::Muted);
    d.circle([155., 143.], 9., Ink::Needle);
}

/// Airspeed's mechanical zero is the bottom stop; the usable sweep is clockwise.
pub(super) fn airspeed_angle(knots: f64) -> f64 {
    PI / 2. + knots.clamp(0., 200.) / 200. * 5. * PI / 3.
}
fn arc(d: &mut DrawList, radius: f64, start: f64, end: f64, ink: Ink, width: f32) {
    for step in 0..80 {
        let a = start + (end - start) * f64::from(step) / 80.;
        let b = start + (end - start) * f64::from(step + 1) / 80.;
        d.line(polar(radius, a), polar(radius, b), ink, width);
    }
}
fn airspeed(d: &mut DrawList, knots: f64, speeds: Option<&flightcraft_aero::VSpeeds>) {
    if let Some(speeds) = speeds {
        for (radius, lo, hi, ink) in [
            (56., speeds.vso_mps, speeds.vfe_mps, Ink::Text),
            (62., speeds.vs_mps, speeds.vno_mps, Ink::Normal),
            (62., speeds.vno_mps, speeds.vne_mps, Ink::Warning),
        ] {
            arc(
                d,
                radius,
                airspeed_angle(lo * 1.943844),
                airspeed_angle(hi * 1.943844),
                ink,
                3.,
            );
        }
        let red = airspeed_angle(speeds.vne_mps * 1.943844);
        d.line(polar(55., red), polar(65., red), Ink::Alert, 3.);
    }
    for tick in 0..=40 {
        let knots = f64::from(tick) * 5.;
        let angle = airspeed_angle(knots);
        d.line(
            polar(52., angle),
            polar(if tick % 4 == 0 { 47. } else { 50. }, angle),
            Ink::Text,
            1.,
        );
        if tick % 4 == 0 {
            d.label(polar(40., angle), format!("{knots:.0}"), 10., Ink::Text);
        }
    }
    d.label([100., 108.], "KNOTS", 9., Ink::Muted);
    d.line([100., 88.], polar(53., airspeed_angle(knots)), Ink::Needle, 2.);
    d.circle([100., 88.], 3., Ink::Text);
}

// Original estimate for the bundled aviation gasoline; volume is presentation only.
pub(super) const KG_PER_US_GALLON: f64 = 0.72 * 3.785411784;
pub(super) fn engine_reading(id: &str, value: f64, units: super::EngineUnits) -> String {
    use super::EngineUnits;
    match (id, units) {
        ("oil_temperature" | "egt" | "cht", EngineUnits::Us) => format!("{:.0} °F", value * 1.8 + 32.),
        ("oil_temperature" | "egt" | "cht", EngineUnits::Si) => format!("{value:.0} °C"),
        ("oil_pressure", EngineUnits::Us) => format!("{:.0} psi", value * 1000. / 6894.757293),
        ("oil_pressure", EngineUnits::Si) => format!("{value:.0} kPa"),
        ("fuel_left" | "fuel_right", EngineUnits::Us) => format!("{:.1} US gal", value / KG_PER_US_GALLON),
        ("fuel_left" | "fuel_right", EngineUnits::Si) => format!("{value:.1} kg"),
        ("fuel_flow", EngineUnits::Us) => format!("{:.1} gal/h", value / KG_PER_US_GALLON),
        ("fuel_flow", EngineUnits::Si) => format!("{value:.1} kg/h"),
        ("ammeter", _) => format!("{value:+.1} amps"),
        ("volts", _) => format!("{value:.1} volts"),
        ("suction", EngineUnits::Us) => format!("{:.1} inHg", value * 1000. / super::options::PA_PER_INHG),
        ("suction", EngineUnits::Si) => format!("{value:.1} kPa"),
        _ => format!("{value:.0}"),
    }
}
fn cluster(d: &mut DrawList, spec: Specification, v: f64, input: InstrumentInput<'_>) {
    d.size = [320., 22.];
    d.label([50., 11.], spec.title, 12., Ink::Text);
    d.label(
        [267., 11.],
        engine_reading(spec.id, v, input.options.engine),
        13.,
        Ink::Text,
    );
    let rated_flow = input
        .aircraft
        .and_then(|a| a.engine.fuel_flow_kg_s.points.last())
        .map_or(45., |(_, flow)| flow * 3600.);
    // Original fictional operating bands, in each module's internal units.
    let [red_low, normal_low, normal_high, red_high] = match spec.id {
        "oil_temperature" => [40., 50., 115., 130.],
        "oil_pressure" => [150., 200., 500., 600.],
        "egt" => [200., 400., 900., 1000.],
        "cht" => [40., 80., 220., 250.],
        "fuel_left" | "fuel_right" => [spec.max * 0.15, spec.max * 0.25, spec.max, spec.max],
        "fuel_flow" => [0., 0., rated_flow, rated_flow * 1.15],
        "ammeter" => [-20., -5., 5., 10.],
        "volts" => [12., 13., 15., 16.],
        "suction" => [8., 12., 20., 23.],
        _ => [spec.min, spec.min, spec.max, spec.max],
    };
    let x = |value: f64| 116. + ((value - spec.min) / (spec.max - spec.min)).clamp(0., 1.) as f32 * 92.;
    for (a, b, ink) in [
        (spec.min, red_low, Ink::Alert),
        (red_low, normal_low, Ink::Warning),
        (normal_low, normal_high, Ink::Normal),
        (normal_high, red_high, Ink::Warning),
        (red_high, spec.max, Ink::Alert),
    ] {
        if b > a {
            d.line([x(a), 11.], [x(b), 11.], ink, 6.);
        }
    }
    d.line([x(v), 3.], [x(v), 19.], Ink::Text, 2.);
}
fn tachometer(d: &mut DrawList, rpm: f64, rated_rpm: f64) {
    let angle = |rpm: f64| 3. * PI / 4. + rpm.clamp(0., 3500.) / 3500. * 3. * PI / 2.;
    arc(
        d,
        62.,
        angle(rated_rpm * 0.6),
        angle(rated_rpm * 0.96),
        Ink::Normal,
        4.,
    );
    let red = angle(rated_rpm);
    d.line(polar(55., red), polar(65., red), Ink::Alert, 3.);
    for tick in 0..=35 {
        let a = angle(f64::from(tick) * 100.);
        d.line(
            polar(54., a),
            polar(if tick % 5 == 0 { 49. } else { 52. }, a),
            Ink::Text,
            1.,
        );
        if tick % 5 == 0 {
            d.label(polar(40., a), tick.to_string(), 11., Ink::Text);
        }
    }
    d.label([100., 102.], "RPM ×100", 9., Ink::Muted);
    d.line([100., 88.], polar(53., angle(rpm)), Ink::Needle, 2.);
    d.circle([100., 88.], 3., Ink::Text);
}

fn vsi(d: &mut DrawList, fpm: f64) {
    let angle = |fpm: f64| PI + fpm.clamp(-2000., 2000.) / 2000. * 5. * PI / 6.;
    for tick in -20_i32..=20 {
        let a = angle(f64::from(tick) * 100.);
        d.line(
            polar(63., a),
            polar(if tick % 5 == 0 { 54. } else { 59. }, a),
            Ink::Text,
            1.,
        );
        if tick % 5 == 0 {
            d.label(
                polar(42., a),
                format!("{:.1}", f64::from(tick.abs()) / 10.),
                11.,
                Ink::Text,
            );
        }
    }
    d.label([118., 76.], "UP", 8., Ink::Muted);
    d.label([118., 108.], "DN", 8., Ink::Muted);
    d.label([118., 92.], "×1000 ft/min", 7., Ink::Muted);
    d.line([100., 88.], polar(51., angle(fpm)), Ink::Needle, 2.);
    d.circle([100., 88.], 3., Ink::Text);
}
