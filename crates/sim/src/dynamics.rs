use crate::{Controls, DT_S, State, math::*};
use flightcraft_aero as aero;
use flightcraft_world::{GRAVITY_MPS2 as G, Surface, Wind, atmosphere, ground};

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
pub fn loads(
    aircraft: &aero::Aircraft,
    s: &State,
    c: Controls,
    wind: &Wind,
    failures: aero::Failures,
) -> aero::Loads {
    let [_, _, z] = s.position_m;
    aero::forces_with_failures(
        aircraft,
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
pub fn height(aircraft: &aero::Aircraft, s: &State) -> f64 {
    aircraft.geometry.gear.iter().fold(f64::INFINITY, |h, gear| {
        h.min(wheel_height(s, aircraft.relative_point(gear.point_m)))
    })
}
// Test arrival rate before the spring/damper impulse can mask a hard touchdown.
pub fn hard_touchdown(aircraft: &aero::Aircraft, before: &State, after: &State) -> bool {
    aircraft.geometry.gear.iter().any(|gear| {
        let point = aircraft.relative_point(gear.point_m);
        let [_, _, down] = add(
            before.velocity_mps,
            rotate(before.attitude, cross(before.angular_rate_rps, point)),
        );
        wheel_height(before, point) > 0.0
            && wheel_height(after, point) <= 0.0
            && down > aircraft.systems.hard_touchdown_mps
    })
}

pub fn crash(aircraft: &aero::Aircraft, s: &State) -> bool {
    if aircraft
        .geometry
        .hard_points_m
        .iter()
        .any(|p| wheel_height(s, aircraft.relative_point(*p)) < 0.0)
    {
        return true;
    }
    aircraft.geometry.gear.iter().any(|gear| {
        let p = aircraft.relative_point(gear.point_m);
        let h = wheel_height(s, p);
        let [_, _, vz] = add(s.velocity_mps, rotate(s.attitude, cross(s.angular_rate_rps, p)));
        h < -aircraft.systems.gear_travel_m || (h < 0.0 && vz > aircraft.systems.hard_touchdown_mps)
    })
}

pub fn derivative(
    aircraft: &aero::Aircraft,
    s: &State,
    c: Controls,
    wind: &Wind,
    failures: aero::Failures,
) -> State {
    let a = loads(aircraft, s, c, wind, failures);
    let c = Controls {
        rudder: failures.rudder.unwrap_or(c.rudder),
        ..c
    };
    let gravity = [0.0, 0.0, aircraft.mass.mass_kg * G];
    let mut force = add(rotate(s.attitude, a.force_n), gravity);
    let free_force = force;
    let mut moment = a.moment_nm;
    for gear in &aircraft.geometry.gear {
        let point = aircraft.relative_point(gear.point_m);
        let (k, damper, share) = (gear.stiffness_n_m, gear.damping_ns_m, gear.weight_fraction);
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
        let normal = (-k * h + damper * vz).clamp(0.0, aircraft.systems.max_normal_force_n);
        let [x, y, _] = add(s.position_m, arm);
        let rolling = if ground(x, y).surface == Surface::Pavement {
            aircraft.systems.rolling_pavement
        } else {
            aircraft.systems.rolling_other
        };
        let [_, _, yaw] = euler(s.attitude);
        let steer = if gear.steering_rad == 0.0 {
            0.0
        } else {
            gear.steering_rad * c.rudder
        };
        let angle = yaw + steer;
        let forward = [libm::cos(angle), libm::sin(angle), 0.0];
        let lateral = [-libm::sin(angle), libm::cos(angle), 0.0];
        let planar = [vx, vy, 0.0];
        // A wheel can cancel its share of free force at rest (static friction).
        // The velocity term also prevents brake reversal in a single fixed step.
        let longitudinal = (-dot(planar, forward) * aircraft.mass.mass_kg * share / DT_S
            - dot(free_force, forward) * share)
            .clamp(
                -(rolling + aircraft.systems.brake_friction * c.brake) * normal,
                (rolling + aircraft.systems.brake_friction * c.brake) * normal,
            );
        let side = (-dot(planar, lateral) * aircraft.mass.mass_kg * share / DT_S
            - dot(free_force, lateral) * share)
            .clamp(
                -aircraft.systems.lateral_friction * normal,
                aircraft.systems.lateral_friction * normal,
            );
        let contact = add(
            add(scale(forward, longitudinal), scale(lateral, side)),
            [0.0, 0.0, -normal],
        );
        force = add(force, contact);
        moment = add(moment, cross(point, rotate(conjugate(s.attitude), contact)));
    }
    let [[ix, ixy, ixz], [iyx, iy, iyz], [izx, izy, iz]] = aircraft.mass.inertia_kg_m2;
    let [p, q, r] = s.angular_rate_rps;
    let [mx, my, mz] = moment;
    let angular = if [ixy, ixz, iyx, iyz, izx, izy] == [0.0; 6] {
        [
            (mx + (iy - iz) * q * r) / ix,
            (my + (iz - ix) * r * p) / iy,
            (mz + (ix - iy) * p * q) / iz,
        ]
    } else {
        tensor_acceleration(aircraft.mass.inertia_kg_m2, moment, s.angular_rate_rps)
    };
    State {
        time_s: 1.0,
        position_m: s.velocity_mps,
        velocity_mps: scale(force, 1.0 / aircraft.mass.mass_kg),
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
pub fn rk4(
    aircraft: &aero::Aircraft,
    s: State,
    c: Controls,
    wind: &Wind,
    failures: aero::Failures,
) -> Option<State> {
    let a = derivative(aircraft, &s, c, wind, failures);
    let sb = advance(s, a, DT_S * 0.5);
    if !valid(&sb) {
        return None;
    }
    let b = derivative(aircraft, &sb, c, wind, failures);
    let sd = advance(s, b, DT_S * 0.5);
    if !valid(&sd) {
        return None;
    }
    let d = derivative(aircraft, &sd, c, wind, failures);
    let se = advance(s, d, DT_S);
    if !valid(&se) {
        return None;
    }
    let e = derivative(aircraft, &se, c, wind, failures);
    let mut result = advance(
        advance(advance(advance(s, a, DT_S / 6.0), b, DT_S / 3.0), d, DT_S / 3.0),
        e,
        DT_S / 6.0,
    );
    result.attitude = normalise(result.attitude);
    Some(result)
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
pub fn trim(aircraft: &aero::Aircraft, height: f64, speed: f64) -> (f64, Controls) {
    let rho = atmosphere(height).density_kg_m3;
    let mut lo = -0.15;
    let mut hi = 0.26;
    let mut controls = Controls::default();
    for _ in 0..48 {
        let alpha = (lo + hi) * 0.5;
        let coeff = aero::coefficients(aircraft, alpha, 0.0);
        controls.elevator = -coeff.pitch / aircraft.aerodynamics.derivatives.cm_elevator;
        let drag = 0.5 * rho * speed * speed * aircraft.geometry.wing_area_m2 * coeff.drag;
        let mut tl = 0.0;
        let mut th = 1.0;
        for _ in 0..40 {
            let t = (tl + th) * 0.5;
            if aero::propulsion(aircraft, speed * libm::cos(alpha), t, rho).0 * libm::cos(alpha) > drag {
                th = t;
            } else {
                tl = t;
            }
        }
        controls.throttle = (tl + th) * 0.5;
        let lift = 0.5 * rho * speed * speed * aircraft.geometry.wing_area_m2 * coeff.lift;
        let thrust = aero::propulsion(aircraft, speed * libm::cos(alpha), controls.throttle, rho).0;
        if lift + thrust * libm::sin(alpha) > aircraft.mass.mass_kg * G {
            hi = alpha;
        } else {
            lo = alpha;
        }
    }
    ((lo + hi) * 0.5, controls.clamped())
}

// Euler's rigid-body equation for a full tensor. Average the accepted near-symmetric entries.
fn tensor_acceleration(
    [[a, b, c], [d, e, f], [g, h, i]]: [[f64; 3]; 3],
    moment: [f64; 3],
    rate: [f64; 3],
) -> [f64; 3] {
    let b = (b + d) * 0.5;
    let c = (c + g) * 0.5;
    let f = (f + h) * 0.5;
    let [p, q, r] = rate;
    let momentum = [
        a * p + b * q + c * r,
        b * p + e * q + f * r,
        c * p + f * q + i * r,
    ];
    let rhs = add(moment, scale(cross(rate, momentum), -1.0));
    let det = a * (e * i - f * f) - b * (b * i - f * c) + c * (b * f - e * c);
    let [x, y, z] = rhs;
    [
        ((e * i - f * f) * x + (c * f - b * i) * y + (b * f - c * e) * z) / det,
        ((c * f - b * i) * x + (a * i - c * c) * y + (b * c - a * f) * z) / det,
        ((b * f - c * e) * x + (b * c - a * f) * y + (a * e - b * b) * z) / det,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_tensor_satisfies_euler_equation() {
        let tensor = [[4.0, 1.0, 0.5], [1.0, 5.0, 0.25], [0.5, 0.25, 6.0]];
        let rates = [0.2, -0.3, 0.4];
        let moment = [2.0, 3.0, -1.0];
        let acceleration = tensor_acceleration(tensor, moment, rates);
        let multiply = |v| tensor.map(|row| dot(row, v));
        let recovered = add(multiply(acceleration), cross(rates, multiply(rates)));
        assert!(norm(add(recovered, scale(moment, -1.0))) < 1e-12);
    }
}
