#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_aero::{Aircraft, CORVID_JSON, MAX_AIRCRAFT_BYTES};
use serde_json::{Value, json};

#[test]
fn hostile_files_name_the_field() {
    let original: Value = serde_json::from_str(CORVID_JSON).unwrap();
    let cases = [
        ("/format_version", json!(2), "format_version"),
        ("/mass/mass_kg", json!(-1), "mass.mass_kg"),
        ("/mass/mass_kg", json!("NaN"), "mass.mass_kg"),
        ("/geometry/wing_area_m2", json!(0), "geometry.wing_area_m2"),
        ("/geometry/span_m", json!(-1), "geometry.span_m"),
        ("/geometry/chord_m", json!(1e300), "geometry.chord_m"),
        (
            "/mass/inertia_kg_m2",
            json!([[1, 2, 0], [0, 1, 0], [0, 0, 1]]),
            "mass.inertia_kg_m2",
        ),
        (
            "/mass/inertia_kg_m2",
            json!([[1, 2, 0], [2, 1, 0], [0, 0, 1]]),
            "mass.inertia_kg_m2",
        ),
        (
            "/geometry/gear/0/point_m/2",
            json!(-1),
            "geometry.gear[0].point_m[2]",
        ),
        ("/geometry/gear", json!([]), "geometry.gear"),
        (
            "/aerodynamics/cl_clean/points",
            json!([[0, 1]]),
            "aerodynamics.cl_clean.points",
        ),
        (
            "/aerodynamics/cl_clean/points",
            json!([[1, 1], [0, 2]]),
            "aerodynamics.cl_clean.points[1][0]",
        ),
        (
            "/aerodynamics/cl_clean/points",
            json!([[0, 1], [0, 2]]),
            "aerodynamics.cl_clean.points[1][0]",
        ),
        (
            "/aerodynamics/cl_clean/points",
            json!(vec![(0.0, 1.0); 100_000]),
            "aerodynamics.cl_clean.points",
        ),
        (
            "/aerodynamics/cl_clean/source",
            json!(""),
            "aerodynamics.cl_clean.source",
        ),
        (
            "/propulsion/prop_cp/points",
            json!([[0, 0], [1, 0]]),
            "propulsion.prop_cp.points",
        ),
        ("/propulsion/max_rpm", json!(1), "propulsion.max_rpm"),
        ("/systems/elevator", json!([1, -1]), "systems.elevator"),
        ("/systems/flap_stages", json!([0, 1, 0.5]), "systems.flap_stages"),
    ];
    for (pointer, replacement, field) in cases {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let error = Aircraft::from_json(&value.to_string()).unwrap_err();
        assert!(error.to_string().contains(field), "expected {field}, got {error}");
    }
    let unknown = CORVID_JSON.replacen("\"mass_kg\":", "\"surprise\": 0, \"mass_kg\":", 1);
    assert!(
        Aircraft::from_json(&unknown)
            .unwrap_err()
            .to_string()
            .contains("surprise")
    );
    let overflow = CORVID_JSON.replacen("1000.0", "1e999", 1);
    assert!(
        Aircraft::from_json(&overflow)
            .unwrap_err()
            .to_string()
            .contains("mass.mass_kg")
    );
    assert_eq!(
        Aircraft::from_json(&" ".repeat(MAX_AIRCRAFT_BYTES + 1))
            .unwrap_err()
            .field,
        "file_size"
    );
    assert!(
        Aircraft::from_json(&format!("{CORVID_JSON} {{}}"))
            .unwrap_err()
            .to_string()
            .contains("document")
    );
}

#[test]
fn rust_definitions_also_validate_nonfinite_leaves() {
    for nonfinite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut a = Aircraft::corvid().unwrap();
        a.mass.cg_m[1] = nonfinite;
        assert_eq!(a.validate().unwrap_err().field, "mass.cg_m[1]");
        let mut a = Aircraft::corvid().unwrap();
        a.aerodynamics.cl_clean.points[0].1 = nonfinite;
        assert_eq!(
            a.validate().unwrap_err().field,
            "aerodynamics.cl_clean.points[0][1]"
        );
    }
}

// Original deterministic xorshift generator; no runtime randomness or external fuzzing crate.
fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

#[test]
fn deterministic_byte_mutations_never_panic() {
    let mut seed = 0x3824_d3f0_9b1e_5a71;
    for iteration in 0..4000 {
        let mut bytes = CORVID_JSON.as_bytes().to_vec();
        for _ in 0..(next(&mut seed) % 16 + 1) {
            if bytes.is_empty() {
                break;
            }
            let position = next(&mut seed) as usize % bytes.len();
            match next(&mut seed) % 4 {
                0 => bytes[position] = next(&mut seed) as u8,
                1 => {
                    bytes.remove(position);
                }
                2 => bytes.insert(position, next(&mut seed) as u8),
                _ => bytes.truncate(position + 1),
            }
        }
        let text = String::from_utf8_lossy(&bytes);
        let outcome = std::panic::catch_unwind(|| Aircraft::from_json(&text));
        assert!(outcome.is_ok(), "mutation {iteration} panicked");
        if let Ok(Ok(aircraft)) = outcome {
            aircraft.validate().unwrap();
        }
    }
}

#[test]
fn deterministic_structural_mutations_never_panic() {
    let original: Value = serde_json::from_str(CORVID_JSON).unwrap();
    let paths = [
        "/mass/mass_kg",
        "/mass/inertia_kg_m2",
        "/geometry/gear",
        "/aerodynamics/cl_clean/points",
        "/propulsion/prop_cp/points",
        "/systems/flap_stages",
    ];
    let replacements = [
        Value::Null,
        json!("NaN"),
        json!(-1e300),
        json!(0),
        json!([]),
        json!({"nested":[1,2,3]}),
        json!([[0, 1], [1, 2]]),
    ];
    let mut seed = 0xb318_2045_661a_9991;
    for _ in 0..1000 {
        let mut value = original.clone();
        let path = paths[next(&mut seed) as usize % paths.len()];
        *value.pointer_mut(path).unwrap() =
            replacements[next(&mut seed) as usize % replacements.len()].clone();
        let text = value.to_string();
        assert!(std::panic::catch_unwind(|| Aircraft::from_json(&text)).is_ok());
    }
}

#[test]
fn symmetric_tensor_tolerance_and_round_trip() {
    let mut a = Aircraft::corvid().unwrap();
    a.mass.inertia_kg_m2[0][1] = 10.0;
    a.mass.inertia_kg_m2[1][0] = 10.0 + 1e-8;
    a.validate().unwrap();
    let encoded = serde_json::to_string(&a).unwrap();
    assert_eq!(Aircraft::from_json(&encoded).unwrap(), a);
}
