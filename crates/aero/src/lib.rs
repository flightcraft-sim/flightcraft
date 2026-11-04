//! Original fictional aircraft: SI units, body axes forward/right/down, positive moments
//! right-wing-down/nose-up/nose-right. Controls are normalised; positive elevator pitches up.
//! All tables are original engineering estimates, not measurements of an existing aircraft.
#![deny(clippy::disallowed_methods)]

mod aircraft;
pub mod autopilot;
mod layers;
pub use layers::*;
pub mod exterior;
mod packs;
pub use aircraft::*;
pub use packs::*;
mod installed;
pub mod modern;
pub mod panel_fidelity;
pub use installed::*;
pub mod checklists;
pub mod emergencies;
pub mod exterior_framing;
mod failures;
pub mod preflight;
pub mod walkaround;
pub use failures::*;

use serde::{Deserialize, Serialize};
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
    if v.is_finite() && lo.is_finite() && hi.is_finite() && lo <= hi {
        v.clamp(lo, hi)
    } else {
        0.0
    }
}

/// Bounded attached/stalled coefficients, blended to a flat plate between 25° and 60°.
/// Original full-flap increments: CD0 +0.055 and Cm -0.10, representing plain-flap
/// pressure drag and the aft shift of wing loading; linearly scheduled with flap travel.
#[must_use]
pub fn coefficients(aircraft: &Aircraft, alpha: f64, flaps: f64) -> Coefficients {
    let a = finite(alpha, -core::f64::consts::PI, core::f64::consts::PI);
    let f = finite(flaps, 0.0, 1.0);
    let blend = ((a.abs() - aircraft.aerodynamics.blend_start_rad) / (aircraft.aerodynamics.blend_width_rad))
        .clamp(0.0, 1.0);
    let blend = blend * blend * (3.0 - 2.0 * blend);
    let cl = interpolate(&aircraft.aerodynamics.cl_clean.points, a) * (1.0 - f)
        + interpolate(&aircraft.aerodynamics.cl_flaps.points, a) * f;
    let lift = cl * (1.0 - blend) + libm::sin(2.0 * a) * blend;
    let drag = (interpolate(&aircraft.aerodynamics.cd_alpha.points, a)
        + aircraft.aerodynamics.induced_drag * cl * cl
        + aircraft.aerodynamics.flap_drag * f)
        * (1.0 - blend)
        + (aircraft.aerodynamics.plate_drag_zero
            + aircraft.aerodynamics.plate_drag * libm::sin(a) * libm::sin(a))
            * blend;
    let pitch = (interpolate(&aircraft.aerodynamics.cm_alpha.points, a)
        + aircraft.aerodynamics.flap_pitch * f)
        * (1.0 - blend)
        + aircraft.aerodynamics.plate_pitch * libm::sin(a) * blend;
    Coefficients { lift, drag, pitch }
}

/// Solve the fixed-pitch propeller power balance with a fixed 40-iteration bisection.
/// Uses the definition's density-scaled shaft power, propeller tables, and RPM limits.
#[must_use]
pub fn propulsion(aircraft: &Aircraft, speed_mps: f64, throttle: f64, density: f64) -> (f64, f64) {
    propulsion_with_power(aircraft, speed_mps, throttle, density, 1.0)
}

/// Shaft power is scaled, including idle. At zero power the definition supplies the
/// equivalent windmilling drag area and advance ratio.
#[must_use]
pub fn propulsion_with_power(
    aircraft: &Aircraft,
    speed_mps: f64,
    throttle: f64,
    density: f64,
    power_fraction: f64,
) -> (f64, f64) {
    let rho = finite(density, 0.01, 2.0).max(0.01);
    let v = finite(speed_mps, 0.0, 500.0);
    let fraction = finite(power_fraction, 0.0, 1.0);
    if fraction == 0.0 {
        return (
            -0.5 * rho * v * v * aircraft.propulsion.windmill_drag_area_m2,
            v / (aircraft.propulsion.windmill_advance_ratio * aircraft.propulsion.prop_diameter_m) * 60.0,
        );
    }
    let power = (aircraft.propulsion.idle_power_w
        + (aircraft.propulsion.max_power_w - aircraft.propulsion.idle_power_w) * finite(throttle, 0.0, 1.0))
        * interpolate(&aircraft.propulsion.power_density.points, rho / 1.225)
        * fraction;
    let mut lo = aircraft.propulsion.min_rps;
    let mut hi = aircraft.propulsion.max_rpm / 60.0;
    for _ in 0..40 {
        let n = (lo + hi) * 0.5;
        let j = v / (n * aircraft.propulsion.prop_diameter_m);
        let absorbed = interpolate(&aircraft.propulsion.prop_cp.points, j)
            * rho
            * n
            * n
            * n
            * libm::pow(aircraft.propulsion.prop_diameter_m, 5.0);
        if absorbed > power {
            hi = n;
        } else {
            lo = n;
        }
    }
    let n = (lo + hi) * 0.5;
    let j = v / (n * aircraft.propulsion.prop_diameter_m);
    (
        interpolate(&aircraft.propulsion.prop_ct.points, j)
            * rho
            * n
            * n
            * libm::pow(aircraft.propulsion.prop_diameter_m, 4.0),
        n * 60.0,
    )
}

/// Forces and moments at the CG, excluding gravity and gear. No hidden integrator state.
/// Hostile inputs are sanitised and bounded before arithmetic.
#[must_use]
pub fn forces(aircraft: &Aircraft, airflow: Airflow, controls: Controls, density: f64) -> Loads {
    forces_with_failures(aircraft, airflow, controls, density, Failures::default())
}

/// Apply failures before calculating forces; ordinary forces keep the original model.
#[must_use]
pub fn forces_with_failures(
    aircraft: &Aircraft,
    airflow: Airflow,
    controls: Controls,
    density: f64,
    failures: Failures,
) -> Loads {
    let controls = aircraft.limit_controls(failures.apply(controls));
    let [u, v, w] = airflow.velocity_body_mps.map(|x| finite(x, -500.0, 500.0));
    let [p, q, r] = airflow.angular_rate_rps.map(|x| finite(x, -50.0, 50.0));
    let speed = libm::sqrt(u * u + v * v + w * w);
    let alpha = libm::atan2(w, u);
    let beta = if speed > 0.01 {
        libm::asin((v / speed).clamp(-1.0, 1.0))
    } else {
        0.0
    };
    let mut c = coefficients(aircraft, alpha, controls.flaps);
    if failures.airframe_icing {
        c.lift *= 0.7;
        c.drag += 0.065;
    }
    if failures.door_open {
        c.drag += 0.025;
    }
    let dynamic = 0.5 * finite(density, 0.0, 2.0) * speed * speed * aircraft.geometry.wing_area_m2;
    let rate = 0.5 / speed.max(10.0);
    let d = aircraft.aerodynamics.derivatives;
    let elevator = finite(controls.elevator, -1.0, 1.0);
    let aileron = finite(controls.aileron, -1.0, 1.0);
    let rudder = finite(controls.rudder, -1.0, 1.0);
    let cy = interpolate(&aircraft.aerodynamics.cy_beta.points, beta)
        + d.cy_r * r * aircraft.geometry.span_m * rate
        + d.cy_rudder * rudder;
    let cl = interpolate(&aircraft.aerodynamics.cl_beta.points, beta)
        + (d.cl_p * p + d.cl_r * r) * aircraft.geometry.span_m * rate
        + d.cl_aileron * aileron;
    let cm = c.pitch + d.cm_q * q * aircraft.geometry.chord_m * rate + d.cm_elevator * elevator;
    let cn = interpolate(&aircraft.aerodynamics.cn_beta.points, beta)
        + (d.cn_p * p + d.cn_r * r) * aircraft.geometry.span_m * rate
        + d.cn_rudder * rudder
        + d.cn_aileron * aileron;
    let (thrust, rpm) = propulsion_with_power(
        aircraft,
        u,
        controls.throttle,
        density,
        if failures.engine_fire {
            failures.engine_power_fraction * 0.15
        } else {
            failures.engine_power_fraction
        },
    );
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
            dynamic
                * aircraft.geometry.span_m
                * (cl
                    + if failures.flap_asymmetry {
                        0.08 * controls.flaps.max(0.2)
                    } else {
                        0.0
                    }),
            dynamic * aircraft.geometry.chord_m * cm,
            dynamic * aircraft.geometry.span_m * cn,
        ],
        engine_rpm: rpm,
    }
}

#[cfg(test)]
mod legacy_corvid;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    const DEG: f64 = core::f64::consts::PI / 180.0;
    #[test]
    fn interpolation_and_stall() {
        let aircraft = &Aircraft::corvid().unwrap();
        assert_eq!(interpolate(&[], 0.0), 0.0);
        assert_eq!(interpolate(&[(0.0, 2.0), (1.0, 4.0)], 0.5), 3.0);
        assert_eq!(interpolate(&aircraft.aerodynamics.cl_clean.points, -100.0), -0.65);
        assert!(coefficients(aircraft, 20.0 * DEG, 0.0).lift < aircraft.aerodynamics.cl_max);
        for i in -180..=180 {
            let c = coefficients(aircraft, f64::from(i) * DEG, 1.0);
            assert!(c.lift.abs() <= 1.9 && c.drag >= 0.0 && c.drag <= 1.9 && c.pitch.abs() < 0.6);
        }
    }
    #[test]
    fn thrust_and_power() {
        let aircraft = &Aircraft::corvid().unwrap();
        let (static_t, rpm) = propulsion(aircraft, 0.0, 1.0, 1.225);
        assert!((2000.0..3500.0).contains(&static_t));
        assert!((2000.0..aircraft.propulsion.max_rpm).contains(&rpm));
        assert!(propulsion(aircraft, 40.0, 1.0, 0.8).0 < propulsion(aircraft, 40.0, 1.0, 1.225).0);
        for v in [0.0, 20.0, 40.0, 60.0] {
            let (t, n) = propulsion(aircraft, v, 1.0, 1.225);
            assert!(t * v / aircraft.propulsion.max_power_w < 0.9);
            assert!(n.is_finite());
        }
    }
}

#[cfg(test)]
mod migration {
    #![allow(clippy::unwrap_used)]
    use super::{Aircraft, legacy_corvid as old};
    #[test]
    fn loaded_corvid_matches_every_original_table_bit_for_bit() {
        let a = Aircraft::corvid().unwrap();
        for (loaded, original) in [
            (&a.aerodynamics.cl_clean, old::CL_CLEAN),
            (&a.aerodynamics.cl_flaps, old::CL_FLAPS),
            (&a.aerodynamics.cd_alpha, old::CD_ALPHA),
            (&a.aerodynamics.cm_alpha, old::CM_ALPHA),
            (&a.aerodynamics.cy_beta, old::CY_BETA),
            (&a.aerodynamics.cl_beta, old::CL_BETA),
            (&a.aerodynamics.cn_beta, old::CN_BETA),
            (&a.propulsion.prop_ct, old::PROP_CT),
            (&a.propulsion.prop_cp, old::PROP_CP),
            (&a.propulsion.power_density, old::POWER_DENSITY),
        ] {
            assert_eq!(loaded.points.len(), original.len());
            for ((x, y), (ox, oy)) in loaded.points.iter().zip(original) {
                assert_eq!((x.to_bits(), y.to_bits()), (ox.to_bits(), oy.to_bits()));
            }
        }
        assert_eq!(a.mass.mass_kg, old::MASS_KG);
        let [[ix, _, _], [_, iy, _], [_, _, iz]] = a.mass.inertia_kg_m2;
        assert_eq!([ix, iy, iz], old::INERTIA_KG_M2);
        assert_eq!(a.geometry.wing_area_m2, old::WING_AREA_M2);
        assert_eq!(a.geometry.span_m, old::SPAN_M);
        assert_eq!(a.geometry.chord_m, old::CHORD_M);
        assert_eq!(a.propulsion.max_power_w, old::MAX_POWER_W);
        assert_eq!(a.propulsion.prop_diameter_m, old::PROP_DIAMETER_M);
        assert_eq!(a.aerodynamics.cl_max, old::CL_MAX);
        assert_eq!(a.aerodynamics.derivatives.cl_p, old::DERIVATIVES.cl_p);
        assert_eq!(a.aerodynamics.derivatives.cl_r, old::DERIVATIVES.cl_r);
        assert_eq!(a.aerodynamics.derivatives.cm_q, old::DERIVATIVES.cm_q);
        assert_eq!(a.aerodynamics.derivatives.cn_p, old::DERIVATIVES.cn_p);
        assert_eq!(a.aerodynamics.derivatives.cn_r, old::DERIVATIVES.cn_r);
        assert_eq!(a.aerodynamics.derivatives.cy_r, old::DERIVATIVES.cy_r);
        assert_eq!(a.aerodynamics.derivatives.cl_aileron, old::DERIVATIVES.cl_aileron);
        assert_eq!(
            a.aerodynamics.derivatives.cm_elevator,
            old::DERIVATIVES.cm_elevator
        );
        assert_eq!(a.aerodynamics.derivatives.cn_rudder, old::DERIVATIVES.cn_rudder);
        assert_eq!(a.aerodynamics.derivatives.cy_rudder, old::DERIVATIVES.cy_rudder);
        assert_eq!(a.aerodynamics.derivatives.cn_aileron, old::DERIVATIVES.cn_aileron);
    }
}
