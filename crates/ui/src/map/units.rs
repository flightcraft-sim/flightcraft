//! Pilot display conversions match the engine and fuel instruments; commands remain SI.
use crate::instruments::EngineUnits;
pub(super) fn fuel(kg: f64, units: EngineUnits) -> String {
    match units {
        EngineUnits::Us => format!("{:.1} US gal", kg / (0.72 * 3.785411784)),
        EngineUnits::Si => format!("{kg:.1} kg"),
    }
}
pub(super) fn speed(mps: f64) -> String {
    format!("{:.0} kt", mps / 0.5144444444444445)
}
