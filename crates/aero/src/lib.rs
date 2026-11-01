//! Original Corvid trainer: SI units, body axes forward/right/down, positive moments
//! right-wing-down/nose-up/nose-right. Controls are normalised; positive elevator pitches up.
//! All tables are original engineering estimates, not measurements of an existing aircraft.
#![deny(clippy::disallowed_methods)]

mod failures;
pub use failures::*;

use serde::{Deserialize, Serialize};
const DEG: f64 = core::f64::consts::PI / 180.0;
pub const MASS_KG: f64 = 1000.0;
pub const INERTIA_KG_M2: [f64; 3] = [1200.0, 1700.0, 2400.0];
pub const WING_AREA_M2: f64 = 15.0;
pub const SPAN_M: f64 = 10.0;
pub const CHORD_M: f64 = 1.5;
pub const MAX_POWER_W: f64 = 120_000.0;
pub const CL_MAX: f64 = 1.50;
pub const PROP_DIAMETER_M: f64 = 1.9;

/// Piecewise linear, edge-clamped table. Invalid queries return the first ordinate;
/// an empty table returns zero. Built-in tables have finite, strictly increasing abscissae.
#[must_use]
pub fn interpolate(table: &[(f64, f64)], x: f64) -> f64 {
    let Some(&(first_x, first_y)) = table.first() else {
        return 0.0;
    };
    if !x.is_finite() || x <= first_x {
        return first_y;
    }
    for pair in table.windows(2) {
        if let [(x0, y0), (x1, y1)] = pair
            && x <= *x1
        {
            return y0 + (y1 - y0) * ((x - x0) / (x1 - x0));
        }
    }
    table.last().map_or(0.0, |&(_, y)| y)
}

/// Original finite-wing estimate: cambered four-digit-section shape informed by
/// NACA Report 824 (1945), with slope reduced by lifting-line theory for AR=6.67.
/// Stall and post-stall ordinates are conservative design assumptions, not digitised data.
pub const CL_CLEAN: &[(f64, f64)] = &[
    (-25.0 * DEG, -0.65),
    (-18.0 * DEG, -0.95),
    (-14.0 * DEG, -1.0),
    (-8.0 * DEG, -0.448),
    (0.0, 0.25),
    (8.0 * DEG, 0.948),
    (14.0 * DEG, 1.47),
    (16.0 * DEG, 1.50),
    (20.0 * DEG, 1.15),
    (25.0 * DEG, 0.85),
];
/// Original plain-flap estimate: +0.5 CL in attached flow, earlier separation.
pub const CL_FLAPS: &[(f64, f64)] = &[
    (-25.0 * DEG, -0.45),
    (-18.0 * DEG, -0.7),
    (-14.0 * DEG, -0.5),
    (-8.0 * DEG, 0.052),
    (0.0, 0.75),
    (8.0 * DEG, 1.448),
    (12.0 * DEG, 1.85),
    (16.0 * DEG, 1.65),
    (20.0 * DEG, 1.3),
    (25.0 * DEG, 1.0),
];
/// Original parasitic drag rise with incidence, plus CDi=0.055 CL² below.
/// k=1/(pi AR e), e≈0.87 for the complete wing.
pub const CD_ALPHA: &[(f64, f64)] = &[
    (-25.0 * DEG, 0.18),
    (-16.0 * DEG, 0.035),
    (0.0, 0.028),
    (16.0 * DEG, 0.035),
    (25.0 * DEG, 0.18),
];
/// Original static margin: Cm_alpha=-1.2/rad, Cm0=0.04, from a tail-volume build-up.
pub const CM_ALPHA: &[(f64, f64)] = &[
    (-25.0 * DEG, 0.563598776),
    (0.0, 0.04),
    (25.0 * DEG, -0.483598776),
];
/// Original vertical-tail / fuselage sideforce estimate, CY_beta=-0.65/rad.
pub const CY_BETA: &[(f64, f64)] = &[(-30.0 * DEG, 0.340339204), (0.0, 0.0), (30.0 * DEG, -0.340339204)];
/// Original high-wing dihedral estimate, Cl_beta=-0.10/rad.
pub const CL_BETA: &[(f64, f64)] = &[(-30.0 * DEG, 0.052359878), (0.0, 0.0), (30.0 * DEG, -0.052359878)];
/// Original tail-volume directional stability estimate, Cn_beta=0.15/rad.
pub const CN_BETA: &[(f64, f64)] = &[(-30.0 * DEG, -0.078539816), (0.0, 0.0), (30.0 * DEG, 0.078539816)];

#[derive(Clone, Copy, Debug)]
pub struct Derivatives {
    pub cl_p: f64,
    pub cl_r: f64,
    pub cm_q: f64,
    pub cn_p: f64,
    pub cn_r: f64,
    pub cy_r: f64,
    pub cl_aileron: f64,
    pub cm_elevator: f64,
    pub cn_rudder: f64,
    pub cy_rudder: f64,
    pub cn_aileron: f64,
}
/// Original strip-theory roll damping, tail-volume pitch/yaw damping, and control
/// effectiveness estimates in the conventional nondimensional rate system (pb/2V, qc/2V, rb/2V).
/// Control derivatives are per full normalised travel, not per radian.
pub const DERIVATIVES: Derivatives = Derivatives {
    cl_p: -0.5,
    cl_r: 0.12,
    cm_q: -8.0,
    cn_p: -0.03,
    cn_r: -0.25,
    cy_r: 0.2,
    cl_aileron: 0.16,
    cm_elevator: 1.0,
    cn_rudder: 0.10,
    cy_rudder: 0.18,
    cn_aileron: -0.012,
};

/// Original fixed-pitch prop estimates, J=V/(nD), CT=T/(rho n² D⁴).
/// Smooth unloading to zero thrust at J=1.1, then windmilling drag.
pub const PROP_CT: &[(f64, f64)] = &[
    (0.0, 0.105),
    (0.3, 0.093),
    (0.6, 0.066),
    (0.9, 0.025),
    (1.1, 0.0),
    (1.4, -0.025),
    (3.0, -0.04),
];
/// Original CP=P/(rho n³ D⁵); paired with CT for peak efficiency below 0.85.
pub const PROP_CP: &[(f64, f64)] = &[
    (0.0, 0.055),
    (0.3, 0.052),
    (0.6, 0.047),
    (0.9, 0.038),
    (1.1, 0.030),
    (1.4, 0.022),
    (3.0, 0.022),
];
/// Original normally aspirated engine: available shaft power proportional to density ratio.
pub const POWER_DENSITY: &[(f64, f64)] = &[
    (0.0, 0.0),
    (0.25, 0.25),
    (0.5, 0.5),
    (0.75, 0.75),
    (1.0, 1.0),
    (1.5, 1.5),
];

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Controls {
    pub elevator: f64,
    pub aileron: f64,
    pub rudder: f64,
    pub throttle: f64,
    pub flaps: f64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Airflow {
    pub velocity_body_mps: [f64; 3],
    pub angular_rate_rps: [f64; 3],
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Loads {
    pub force_n: [f64; 3],
    pub moment_nm: [f64; 3],
    pub engine_rpm: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Coefficients {
    pub lift: f64,
    pub drag: f64,
    pub pitch: f64,
}

fn finite(v: f64, lo: f64, hi: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { 0.0 }
}

/// Bounded attached/stalled coefficients, blended to a flat plate between 25° and 60°.
/// Original full-flap increments: CD0 +0.055 and Cm -0.10, representing plain-flap
/// pressure drag and the aft shift of wing loading; linearly scheduled with flap travel.
#[must_use]
pub fn coefficients(alpha: f64, flaps: f64) -> Coefficients {
    let a = finite(alpha, -core::f64::consts::PI, core::f64::consts::PI);
    let f = finite(flaps, 0.0, 1.0);
    let blend = ((a.abs() - 25.0 * DEG) / (35.0 * DEG)).clamp(0.0, 1.0);
    let blend = blend * blend * (3.0 - 2.0 * blend);
    let cl = interpolate(CL_CLEAN, a) * (1.0 - f) + interpolate(CL_FLAPS, a) * f;
    let lift = cl * (1.0 - blend) + libm::sin(2.0 * a) * blend;
    let drag = (interpolate(CD_ALPHA, a) + 0.055 * cl * cl + 0.055 * f) * (1.0 - blend)
        + (0.04 + 1.8 * libm::sin(a) * libm::sin(a)) * blend;
    let pitch = (interpolate(CM_ALPHA, a) - 0.10 * f) * (1.0 - blend) - 0.15 * libm::sin(a) * blend;
    Coefficients { lift, drag, pitch }
}

/// Solve the fixed-pitch propeller power balance with a fixed 40-iteration bisection.
/// Idle is 1.8 kW at sea level; RPM is capped at 2,800 by the engine power limit.
#[must_use]
pub fn propulsion(speed_mps: f64, throttle: f64, density: f64) -> (f64, f64) {
    propulsion_with_power(speed_mps, throttle, density, 1.0)
}

/// Shaft power is scaled, including idle. At zero power an original windmilling
/// estimate uses an equivalent drag area of 0.10 m² and advance ratio 1.5.
#[must_use]
pub fn propulsion_with_power(speed_mps: f64, throttle: f64, density: f64, power_fraction: f64) -> (f64, f64) {
    let rho = finite(density, 0.01, 2.0).max(0.01);
    let v = finite(speed_mps, 0.0, 500.0);
    let fraction = finite(power_fraction, 0.0, 1.0);
    if fraction == 0.0 {
        return (-0.5 * rho * v * v * 0.10, v / (1.5 * PROP_DIAMETER_M) * 60.0);
    }
    let power = (1800.0 + (MAX_POWER_W - 1800.0) * finite(throttle, 0.0, 1.0))
        * interpolate(POWER_DENSITY, rho / 1.225)
        * fraction;
    let mut lo = 5.0;
    let mut hi = 2800.0 / 60.0;
    for _ in 0..40 {
        let n = (lo + hi) * 0.5;
        let j = v / (n * PROP_DIAMETER_M);
        let absorbed = interpolate(PROP_CP, j) * rho * n * n * n * libm::pow(PROP_DIAMETER_M, 5.0);
        if absorbed > power {
            hi = n;
        } else {
            lo = n;
        }
    }
    let n = (lo + hi) * 0.5;
    let j = v / (n * PROP_DIAMETER_M);
    (
        interpolate(PROP_CT, j) * rho * n * n * libm::pow(PROP_DIAMETER_M, 4.0),
        n * 60.0,
    )
}

/// Forces and moments at the CG, excluding gravity and gear. No hidden integrator state.
/// Hostile inputs are sanitised and bounded before arithmetic.
#[must_use]
pub fn forces(airflow: Airflow, controls: Controls, density: f64) -> Loads {
    forces_with_failures(airflow, controls, density, Failures::default())
}

/// Apply failures before calculating forces; ordinary forces keep the original model.
#[must_use]
pub fn forces_with_failures(airflow: Airflow, controls: Controls, density: f64, failures: Failures) -> Loads {
    let controls = failures.apply(controls);
    let [u, v, w] = airflow.velocity_body_mps.map(|x| finite(x, -500.0, 500.0));
    let [p, q, r] = airflow.angular_rate_rps.map(|x| finite(x, -50.0, 50.0));
    let speed = libm::sqrt(u * u + v * v + w * w);
    let alpha = libm::atan2(w, u);
    let beta = if speed > 0.01 {
        libm::asin((v / speed).clamp(-1.0, 1.0))
    } else {
        0.0
    };
    let c = coefficients(alpha, controls.flaps);
    let dynamic = 0.5 * finite(density, 0.0, 2.0) * speed * speed * WING_AREA_M2;
    let rate = 0.5 / speed.max(10.0);
    let d = DERIVATIVES;
    let elevator = finite(controls.elevator, -1.0, 1.0);
    let aileron = finite(controls.aileron, -1.0, 1.0);
    let rudder = finite(controls.rudder, -1.0, 1.0);
    let cy = interpolate(CY_BETA, beta) + d.cy_r * r * SPAN_M * rate + d.cy_rudder * rudder;
    let cl = interpolate(CL_BETA, beta) + (d.cl_p * p + d.cl_r * r) * SPAN_M * rate + d.cl_aileron * aileron;
    let cm = c.pitch + d.cm_q * q * CHORD_M * rate + d.cm_elevator * elevator;
    let cn = interpolate(CN_BETA, beta)
        + (d.cn_p * p + d.cn_r * r) * SPAN_M * rate
        + d.cn_rudder * rudder
        + d.cn_aileron * aileron;
    let (thrust, rpm) = propulsion_with_power(u, controls.throttle, density, failures.engine_power_fraction);
    // Wind axes to body axes, including beta: drag always opposes the airflow.
    let (sa, ca) = (libm::sin(alpha), libm::cos(alpha));
    let (sb, cb) = (libm::sin(beta), libm::cos(beta));
    Loads {
        force_n: [
            dynamic * (-c.drag * ca * cb - cy * ca * sb + c.lift * sa) + thrust,
            dynamic * (-c.drag * sb + cy * cb),
            dynamic * (-c.drag * sa * cb - cy * sa * sb - c.lift * ca),
        ],
        moment_nm: [
            dynamic * SPAN_M * cl,
            dynamic * CHORD_M * cm,
            dynamic * SPAN_M * cn,
        ],
        engine_rpm: rpm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interpolation_and_stall() {
        assert_eq!(interpolate(&[], 0.0), 0.0);
        assert_eq!(interpolate(&[(0.0, 2.0), (1.0, 4.0)], 0.5), 3.0);
        assert_eq!(interpolate(CL_CLEAN, -100.0), -0.65);
        assert!(coefficients(20.0 * DEG, 0.0).lift < CL_MAX);
        for i in -180..=180 {
            let c = coefficients(f64::from(i) * DEG, 1.0);
            assert!(c.lift.abs() <= 1.9 && c.drag >= 0.0 && c.drag <= 1.9 && c.pitch.abs() < 0.6);
        }
    }
    #[test]
    fn thrust_and_power() {
        let (static_t, rpm) = propulsion(0.0, 1.0, 1.225);
        assert!((2000.0..3500.0).contains(&static_t));
        assert!((2000.0..2800.0).contains(&rpm));
        assert!(propulsion(40.0, 1.0, 0.8).0 < propulsion(40.0, 1.0, 1.225).0);
        for v in [0.0, 20.0, 40.0, 60.0] {
            let (t, n) = propulsion(v, 1.0, 1.225);
            assert!(t * v / MAX_POWER_W < 0.9);
            assert!(n.is_finite());
        }
    }
}
