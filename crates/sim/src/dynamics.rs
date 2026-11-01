use crate::{Controls, DT_S, State, math::*};
use flightcraft_aero as aero;
use flightcraft_world::{GRAVITY_MPS2 as G, Surface, Wind, atmosphere, ground};

/// Main pair and nose leg, body-frame contact points (m), stiffness (N/m), damping
/// (N s/m), and static weight fraction. All legs have 0.30 m usable travel.
pub const GEAR: [([f64; 3], f64, f64, f64); 3] = [
    ([-0.45, -1.1, 1.15], 80_000.0, 5000.0, 0.4),
    ([-0.45, 1.1, 1.15], 80_000.0, 5000.0, 0.4),
    ([1.8, 0.0, 1.15], 40_000.0, 3000.0, 0.2),
];
// Belly, lowest propeller tip (shaft at CG height), tail, and both wingtips.
const HARD_POINTS: [[f64; 3]; 5] = [
    [0.0, 0.0, 0.5],
    [2.0, 0.0, 0.95],
    [-4.0, 0.0, 0.3],
    [0.0, -5.0, -0.3],
    [0.0, 5.0, -0.3],
];

pub fn aero_controls(c: Controls) -> aero::Controls {
    aero::Controls {
        elevator: c.elevator,
        aileron: c.aileron,
        rudder: c.rudder,
        throttle: c.throttle,
        flaps: c.flaps,
    }
}
pub fn relative_velocity(s: &State, wind: &Wind) -> [f64; 3] {
    let moving_air = wind.velocity_mps(s.position_m);
    // Keep every operation of the original calm-air path, including signed zeros.
    if moving_air == [0.0; 3] {
        s.velocity_mps
    } else {
        let [n, e, d] = s.velocity_mps;
        let [wn, we, wd] = moving_air;
        [n - wn, e - we, d - wd]
    }
}

pub fn airflow(s: &State, wind: &Wind) -> aero::Airflow {
    let relative = relative_velocity(s, wind);
    aero::Airflow {
        velocity_body_mps: rotate(conjugate(s.attitude), relative),
        angular_rate_rps: s.angular_rate_rps,
    }
}
pub fn loads(s: &State, c: Controls, wind: &Wind, failures: aero::Failures) -> aero::Loads {
    let [_, _, z] = s.position_m;
    aero::forces_with_failures(
        airflow(s, wind),
        aero_controls(c),
        atmosphere(-z).density_kg_m3,
        failures,
    )
}
pub fn wheel_height(s: &State, point: [f64; 3]) -> f64 {
    let [x, y, z] = add(s.position_m, rotate(s.attitude, point));
    -z - ground(x, y).height_msl_m
}
pub fn height(s: &State) -> f64 {
    GEAR.iter()
        .fold(f64::INFINITY, |h, (p, _, _, _)| h.min(wheel_height(s, *p)))
}
// Test arrival rate before the spring/damper impulse can mask a hard touchdown.
pub fn hard_touchdown(before: &State, after: &State) -> bool {
    GEAR.iter().any(|(point, _, _, _)| {
        let [_, _, down] = add(
            before.velocity_mps,
            rotate(before.attitude, cross(before.angular_rate_rps, *point)),
        );
        wheel_height(before, *point) > 0.0 && wheel_height(after, *point) <= 0.0 && down > 4.5
    })
}

pub fn crash(s: &State) -> bool {
    if HARD_POINTS.iter().any(|p| wheel_height(s, *p) < 0.0) {
        return true;
    }
    GEAR.iter().any(|(p, _, _, _)| {
        let h = wheel_height(s, *p);
        let [_, _, vz] = add(s.velocity_mps, rotate(s.attitude, cross(s.angular_rate_rps, *p)));
        h < -0.30 || (h < 0.0 && vz > 4.5)
    })
}

pub fn derivative(s: &State, c: Controls, wind: &Wind, failures: aero::Failures) -> State {
    let a = loads(s, c, wind, failures);
    let c = Controls {
        rudder: failures.rudder.unwrap_or(c.rudder),
        ..c
    };
    let gravity = [0.0, 0.0, aero::MASS_KG * G];
    let mut force = add(rotate(s.attitude, a.force_n), gravity);
    let free_force = force;
    let mut moment = a.moment_nm;
    for (point, k, damper, share) in GEAR {
        let h = wheel_height(s, point);
        if h > 0.0 {
            continue;
        }
        let arm = rotate(s.attitude, point);
        let velocity = add(
            s.velocity_mps,
            rotate(s.attitude, cross(s.angular_rate_rps, point)),
        );
        let [vx, vy, vz] = velocity;
        let normal = (-k * h + damper * vz).clamp(0.0, 80_000.0);
        let [x, y, _] = add(s.position_m, arm);
        let rolling = if ground(x, y).surface == Surface::Pavement {
            0.018
        } else {
            0.055
        };
        let [_, _, yaw] = euler(s.attitude);
        let steer = if point == [1.8, 0.0, 1.15] {
            0.45 * c.rudder
        } else {
            0.0
        };
        let angle = yaw + steer;
        let forward = [libm::cos(angle), libm::sin(angle), 0.0];
        let lateral = [-libm::sin(angle), libm::cos(angle), 0.0];
        let planar = [vx, vy, 0.0];
        // A wheel can cancel its share of free force at rest (static friction).
        // The velocity term also prevents brake reversal in a single fixed step.
        let longitudinal =
            (-dot(planar, forward) * aero::MASS_KG * share / DT_S - dot(free_force, forward) * share).clamp(
                -(rolling + 0.65 * c.brake) * normal,
                (rolling + 0.65 * c.brake) * normal,
            );
        let side = (-dot(planar, lateral) * aero::MASS_KG * share / DT_S - dot(free_force, lateral) * share)
            .clamp(-0.75 * normal, 0.75 * normal);
        let contact = add(
            add(scale(forward, longitudinal), scale(lateral, side)),
            [0.0, 0.0, -normal],
        );
        force = add(force, contact);
        moment = add(moment, cross(point, rotate(conjugate(s.attitude), contact)));
    }
    let [ix, iy, iz] = aero::INERTIA_KG_M2;
    let [p, q, r] = s.angular_rate_rps;
    let [mx, my, mz] = moment;
    let angular = [
        (mx + (iy - iz) * q * r) / ix,
        (my + (iz - ix) * r * p) / iy,
        (mz + (ix - iy) * p * q) / iz,
    ];
    State {
        time_s: 1.0,
        position_m: s.velocity_mps,
        velocity_mps: scale(force, 1.0 / aero::MASS_KG),
        attitude: mul(s.attitude, [0.0, p * 0.5, q * 0.5, r * 0.5]),
        angular_rate_rps: angular,
    }
}

fn advance(s: State, d: State, dt: f64) -> State {
    let [w, x, y, z] = s.attitude;
    let [a, b, c, e] = d.attitude;
    State {
        time_s: s.time_s + dt,
        position_m: add(s.position_m, scale(d.position_m, dt)),
        velocity_mps: add(s.velocity_mps, scale(d.velocity_mps, dt)),
        attitude: [w + a * dt, x + b * dt, y + c * dt, z + e * dt],
        angular_rate_rps: add(s.angular_rate_rps, scale(d.angular_rate_rps, dt)),
    }
}
pub fn rk4(s: State, c: Controls, wind: &Wind, failures: aero::Failures) -> State {
    let a = derivative(&s, c, wind, failures);
    let b = derivative(&advance(s, a, DT_S * 0.5), c, wind, failures);
    let d = derivative(&advance(s, b, DT_S * 0.5), c, wind, failures);
    let e = derivative(&advance(s, d, DT_S), c, wind, failures);
    let mut result = advance(
        advance(advance(advance(s, a, DT_S / 6.0), b, DT_S / 3.0), d, DT_S / 3.0),
        e,
        DT_S / 6.0,
    );
    result.attitude = normalise(result.attitude);
    result
}
pub fn valid(s: &State) -> bool {
    s.position_m.iter().all(|v| v.is_finite() && v.abs() <= 1e8)
        && s.velocity_mps.iter().all(|v| v.is_finite() && v.abs() <= 500.0)
        && s.angular_rate_rps
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 50.0)
        && s.attitude.iter().all(|v| v.is_finite() && v.abs() <= 2.0)
        && s.time_s.is_finite()
        && s.time_s >= 0.0
}

/// Nested scalar force balances: level world velocity, pitch=alpha, zero pitch moment.
pub fn trim(height: f64, speed: f64) -> (f64, Controls) {
    let rho = atmosphere(height).density_kg_m3;
    let mut lo = -0.15;
    let mut hi = 0.26;
    let mut controls = Controls::default();
    for _ in 0..48 {
        let alpha = (lo + hi) * 0.5;
        let coeff = aero::coefficients(alpha, 0.0);
        controls.elevator = -coeff.pitch / aero::DERIVATIVES.cm_elevator;
        let drag = 0.5 * rho * speed * speed * aero::WING_AREA_M2 * coeff.drag;
        let mut tl = 0.0;
        let mut th = 1.0;
        for _ in 0..40 {
            let t = (tl + th) * 0.5;
            if aero::propulsion(speed * libm::cos(alpha), t, rho).0 * libm::cos(alpha) > drag {
                th = t;
            } else {
                tl = t;
            }
        }
        controls.throttle = (tl + th) * 0.5;
        let lift = 0.5 * rho * speed * speed * aero::WING_AREA_M2 * coeff.lift;
        let thrust = aero::propulsion(speed * libm::cos(alpha), controls.throttle, rho).0;
        if lift + thrust * libm::sin(alpha) > aero::MASS_KG * G {
            hi = alpha;
        } else {
            lo = alpha;
        }
    }
    ((lo + hi) * 0.5, controls.clamped())
}
