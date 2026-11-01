//! Versioned, bounded aircraft definitions. Geometry is measured in body axes from its datum.
use crate::Controls;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_AIRCRAFT_BYTES: usize = 1024 * 1024;
pub const CORVID_JSON: &str = include_str!("../../../aircraft/corvid.aircraft.json");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Aircraft {
    #[schemars(range(min = 1, max = 1))]
    pub format_version: u32,
    pub identity: Identity,
    pub geometry: Geometry,
    pub mass: Mass,
    pub aerodynamics: Aerodynamics,
    pub propulsion: Propulsion,
    pub systems: Systems,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub author: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub wing_area_m2: f64,
    pub span_m: f64,
    pub chord_m: f64,
    pub reference_point_m: [f64; 3],
    #[schemars(length(min = 1, max = 16))]
    pub gear: Vec<Gear>,
    #[schemars(length(min = 1, max = 64))]
    pub hard_points_m: Vec<[f64; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Gear {
    pub point_m: [f64; 3],
    pub stiffness_n_m: f64,
    pub damping_ns_m: f64,
    pub weight_fraction: f64,
    pub steering_rad: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mass {
    /// Total fixed flight mass, including occupants and fuel. No fuel burn in version 1.
    pub mass_kg: f64,
    pub cg_m: [f64; 3],
    /// Full symmetric positive-definite tensor about the CG, in body axes.
    pub inertia_kg_m2: [[f64; 3]; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// [breakpoint, ordinate], linear interpolation, clamped at the edges.
    #[schemars(length(min = 2, max = 256))]
    pub points: Vec<(f64, f64)>,
    pub source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Aerodynamics {
    pub cl_clean: Table,
    pub cl_flaps: Table,
    pub cd_alpha: Table,
    pub cm_alpha: Table,
    pub cy_beta: Table,
    pub cl_beta: Table,
    pub cn_beta: Table,
    pub derivatives: Derivatives,
    pub derivatives_source: String,
    pub cl_max: f64,
    pub induced_drag: f64,
    pub flap_drag: f64,
    pub flap_pitch: f64,
    pub blend_start_rad: f64,
    pub blend_width_rad: f64,
    pub plate_drag_zero: f64,
    pub plate_drag: f64,
    pub plate_pitch: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Propulsion {
    pub max_power_w: f64,
    pub idle_power_w: f64,
    pub prop_diameter_m: f64,
    pub min_rps: f64,
    pub max_rpm: f64,
    pub windmill_drag_area_m2: f64,
    pub windmill_advance_ratio: f64,
    pub prop_ct: Table,
    pub prop_cp: Table,
    pub power_density: Table,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Systems {
    /// Normalised control travel limits, [minimum, maximum].
    pub elevator: [f64; 2],
    pub aileron: [f64; 2],
    pub rudder: [f64; 2],
    pub throttle: [f64; 2],
    pub flaps: [f64; 2],
    pub brake: [f64; 2],
    /// Available normalised flap detents; continuous travel between detents is supported.
    #[schemars(length(min = 2, max = 32))]
    pub flap_stages: Vec<f64>,
    pub gear_travel_m: f64,
    pub max_normal_force_n: f64,
    pub hard_touchdown_mps: f64,
    pub rolling_pavement: f64,
    pub rolling_other: f64,
    pub brake_friction: f64,
    pub lateral_friction: f64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{field}: {message}")]
pub struct AircraftError {
    pub field: String,
    pub message: String,
}

fn error(field: impl Into<String>, message: impl Into<String>) -> AircraftError {
    AircraftError {
        field: field.into(),
        message: message.into(),
    }
}

fn require(ok: bool, field: &str, message: &str) -> Result<(), AircraftError> {
    if ok { Ok(()) } else { Err(error(field, message)) }
}

fn positive(value: f64, field: &str) -> Result<(), AircraftError> {
    require(value >= 1e-6, field, "must be positive (at least 0.000001)")
}

// Covers every numeric leaf, including values supplied by Rust callers. The format has no
// optional/null fields; serde_json serialises non-finite floats as null. A magnitude cap keeps
// finite-but-overflowing engineering data out of subsequent products and interpolation.
fn finite_tree(value: &serde_json::Value, path: &str) -> Result<(), AircraftError> {
    match value {
        serde_json::Value::Null => Err(error(path, "must be finite")),
        serde_json::Value::Number(n) => require(
            n.as_f64().is_some_and(|v| v.is_finite() && v.abs() <= 1e12),
            path,
            "must be finite with magnitude at most 1e12",
        ),
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                finite_tree(item, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        serde_json::Value::Object(items) => {
            for (name, item) in items {
                finite_tree(
                    item,
                    &if path.is_empty() {
                        name.clone()
                    } else {
                        format!("{path}.{name}")
                    },
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

impl Table {
    fn validate(&self, field: &str) -> Result<(), AircraftError> {
        require(
            (2..=256).contains(&self.points.len()),
            &format!("{field}.points"),
            "requires 2 to 256 breakpoints",
        )?;
        require(
            !self.source.trim().is_empty(),
            &format!("{field}.source"),
            "source note is required",
        )?;
        for (i, pair) in self.points.windows(2).enumerate() {
            if let [(x0, _), (x1, _)] = pair {
                require(
                    x1 > x0,
                    &format!("{field}.points[{}][0]", i + 1),
                    "breakpoints must be strictly increasing",
                )?;
            }
        }
        Ok(())
    }
}

impl Aircraft {
    /// Parse at most 1 MiB of UTF-8 JSON, rejecting unknown fields at every level.
    pub fn from_json(text: &str) -> Result<Self, AircraftError> {
        require(
            text.len() <= MAX_AIRCRAFT_BYTES,
            "file_size",
            "aircraft exceeds 1 MiB",
        )?;
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let aircraft: Self = serde_path_to_error::deserialize(&mut deserializer)
            .map_err(|e| error(e.path().to_string(), e.inner().to_string()))?;
        deserializer.end().map_err(|e| error("document", e.to_string()))?;
        aircraft.validate()?;
        Ok(aircraft)
    }

    pub fn corvid() -> Result<Self, AircraftError> {
        Self::from_json(CORVID_JSON)
    }

    /// Validate Rust-created definitions as well as decoded files.
    pub fn validate(&self) -> Result<(), AircraftError> {
        require(
            self.format_version == 1,
            "format_version",
            "only version 1 is supported",
        )?;
        finite_tree(
            &serde_json::to_value(self).map_err(|e| error("document", e.to_string()))?,
            "",
        )?;
        for (field, value) in [
            ("id", &self.identity.id),
            ("display_name", &self.identity.display_name),
            ("description", &self.identity.description),
            ("author", &self.identity.author),
        ] {
            require(
                !value.trim().is_empty() && value.len() <= 4096,
                &format!("identity.{field}"),
                "requires 1 to 4096 bytes of text",
            )?;
        }
        require(
            self.identity
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_'),
            "identity.id",
            "use lowercase ASCII letters, digits, hyphens, or underscores",
        )?;
        positive(self.mass.mass_kg, "mass.mass_kg")?;
        for (name, v) in [
            ("wing_area_m2", self.geometry.wing_area_m2),
            ("span_m", self.geometry.span_m),
            ("chord_m", self.geometry.chord_m),
        ] {
            positive(v, &format!("geometry.{name}"))?;
        }
        let [[a, b, c], [d, e, f], [g, h, i]] = self.mass.inertia_kg_m2;
        let scale = a.abs().max(e.abs()).max(i.abs()).max(1.0);
        require(
            (b - d).abs() <= 1e-9 * scale && (c - g).abs() <= 1e-9 * scale && (f - h).abs() <= 1e-9 * scale,
            "mass.inertia_kg_m2",
            "tensor must be symmetric within 1e-9 of its diagonal scale",
        )?;
        let b = (b + d) * 0.5;
        let c = (c + g) * 0.5;
        let f = (f + h) * 0.5;
        require(
            a >= 1e-6
                && a * e - b * b > 1e-12
                && a * (e * i - f * f) - b * (b * i - f * c) + c * (b * f - e * c) > 1e-18,
            "mass.inertia_kg_m2",
            "tensor must be positive definite",
        )?;
        require(
            (1..=16).contains(&self.geometry.gear.len()),
            "geometry.gear",
            "requires 1 to 16 gear points",
        )?;
        require(
            (1..=64).contains(&self.geometry.hard_points_m.len()),
            "geometry.hard_points_m",
            "requires 1 to 64 hard points",
        )?;
        let [_, _, cg_z] = self.mass.cg_m;
        for (n, gear) in self.geometry.gear.iter().enumerate() {
            let path = format!("geometry.gear[{n}]");
            let [_, _, z] = gear.point_m;
            require(
                z > cg_z,
                &format!("{path}.point_m[2]"),
                "gear contact must be below the CG (+z down)",
            )?;
            positive(gear.stiffness_n_m, &format!("{path}.stiffness_n_m"))?;
            require(
                gear.damping_ns_m >= 0.0,
                &format!("{path}.damping_ns_m"),
                "must be nonnegative",
            )?;
            require(
                gear.weight_fraction > 0.0 && gear.weight_fraction <= 1.0,
                &format!("{path}.weight_fraction"),
                "must be in (0, 1]",
            )?;
            require(
                gear.steering_rad.abs() <= core::f64::consts::FRAC_PI_2,
                &format!("{path}.steering_rad"),
                "must be within +/- pi/2",
            )?;
        }
        require(
            (self.geometry.gear.iter().map(|g| g.weight_fraction).sum::<f64>() - 1.0).abs() <= 1e-9,
            "geometry.gear.weight_fraction",
            "fractions must sum to one",
        )?;
        let a = &self.aerodynamics;
        for (name, table) in [
            ("cl_clean", &a.cl_clean),
            ("cl_flaps", &a.cl_flaps),
            ("cd_alpha", &a.cd_alpha),
            ("cm_alpha", &a.cm_alpha),
            ("cy_beta", &a.cy_beta),
            ("cl_beta", &a.cl_beta),
            ("cn_beta", &a.cn_beta),
        ] {
            table.validate(&format!("aerodynamics.{name}"))?;
        }
        require(
            !a.derivatives_source.trim().is_empty(),
            "aerodynamics.derivatives_source",
            "source note is required",
        )?;
        positive(a.derivatives.cm_elevator, "aerodynamics.derivatives.cm_elevator")?;
        positive(a.cl_max, "aerodynamics.cl_max")?;
        positive(a.blend_width_rad, "aerodynamics.blend_width_rad")?;
        require(
            a.blend_start_rad >= 0.0 && a.blend_start_rad + a.blend_width_rad <= core::f64::consts::PI,
            "aerodynamics.blend_start_rad",
            "blend must lie within 0..=pi",
        )?;
        for (name, v) in [
            ("induced_drag", a.induced_drag),
            ("flap_drag", a.flap_drag),
            ("plate_drag_zero", a.plate_drag_zero),
            ("plate_drag", a.plate_drag),
        ] {
            require(v >= 0.0, &format!("aerodynamics.{name}"), "must be nonnegative")?;
        }
        let p = &self.propulsion;
        for (name, table) in [
            ("prop_ct", &p.prop_ct),
            ("prop_cp", &p.prop_cp),
            ("power_density", &p.power_density),
        ] {
            table.validate(&format!("propulsion.{name}"))?;
        }
        for (name, v) in [
            ("max_power_w", p.max_power_w),
            ("prop_diameter_m", p.prop_diameter_m),
            ("min_rps", p.min_rps),
            ("max_rpm", p.max_rpm),
            ("windmill_advance_ratio", p.windmill_advance_ratio),
        ] {
            positive(v, &format!("propulsion.{name}"))?;
        }
        require(
            p.max_rpm / 60.0 > p.min_rps,
            "propulsion.max_rpm",
            "must exceed min_rps * 60",
        )?;
        require(
            p.idle_power_w >= 0.0 && p.idle_power_w <= p.max_power_w,
            "propulsion.idle_power_w",
            "must lie between zero and max_power_w",
        )?;
        require(
            p.windmill_drag_area_m2 >= 0.0,
            "propulsion.windmill_drag_area_m2",
            "must be nonnegative",
        )?;
        for (name, table, strict) in [
            ("prop_cp", &p.prop_cp, true),
            ("power_density", &p.power_density, false),
        ] {
            require(
                table
                    .points
                    .iter()
                    .all(|(_, v)| if strict { *v > 0.0 } else { *v >= 0.0 }),
                &format!("propulsion.{name}.points"),
                "power ordinates must be nonnegative (strictly positive for prop_cp)",
            )?;
        }
        require(
            a.cd_alpha.points.iter().all(|(_, v)| *v >= 0.0),
            "aerodynamics.cd_alpha.points",
            "drag ordinates must be nonnegative",
        )?;
        let s = &self.systems;
        for (name, [lo, hi], min) in [
            ("elevator", s.elevator, -1.0),
            ("aileron", s.aileron, -1.0),
            ("rudder", s.rudder, -1.0),
            ("throttle", s.throttle, 0.0),
            ("flaps", s.flaps, 0.0),
            ("brake", s.brake, 0.0),
        ] {
            require(
                lo >= min && hi <= 1.0 && lo < hi,
                &format!("systems.{name}"),
                "requires ordered limits inside normalised travel",
            )?;
        }
        require(
            (2..=32).contains(&s.flap_stages.len()),
            "systems.flap_stages",
            "requires 2 to 32 stages",
        )?;
        let [lo, hi] = s.flaps;
        require(
            s.flap_stages.first() == Some(&lo)
                && s.flap_stages.last() == Some(&hi)
                && s.flap_stages.windows(2).all(|p| matches!(p,[a,b] if b>a)),
            "systems.flap_stages",
            "stages must increase strictly from flaps minimum to maximum",
        )?;
        for (name, v) in [
            ("gear_travel_m", s.gear_travel_m),
            ("max_normal_force_n", s.max_normal_force_n),
            ("hard_touchdown_mps", s.hard_touchdown_mps),
        ] {
            positive(v, &format!("systems.{name}"))?;
        }
        for (name, v) in [
            ("rolling_pavement", s.rolling_pavement),
            ("rolling_other", s.rolling_other),
            ("brake_friction", s.brake_friction),
            ("lateral_friction", s.lateral_friction),
        ] {
            require(
                (0.0..=2.0).contains(&v),
                &format!("systems.{name}"),
                "must be within 0..=2",
            )?;
        }
        Ok(())
    }

    #[must_use]
    pub fn limit_controls(&self, c: Controls) -> Controls {
        fn limit(v: f64, [lo, hi]: [f64; 2]) -> f64 {
            crate::finite(v, lo, hi)
        }
        Controls {
            elevator: limit(c.elevator, self.systems.elevator),
            aileron: limit(c.aileron, self.systems.aileron),
            rudder: limit(c.rudder, self.systems.rudder),
            throttle: limit(c.throttle, self.systems.throttle),
            flaps: limit(c.flaps, self.systems.flaps),
        }
    }

    #[must_use]
    pub fn relative_point(&self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let [cx, cy, cz] = self.mass.cg_m;
        // Preserve signed zeros for the original datum at the CG.
        if self.mass.cg_m == [0.0; 3] {
            [x, y, z]
        } else {
            [x - cx, y - cy, z - cz]
        }
    }
}
