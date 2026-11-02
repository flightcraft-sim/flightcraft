#![allow(clippy::unwrap_used)]
use flightcraft_aero::{AircraftCatalog, autopilot::*};
#[test]
fn installed_panel_options_and_hostile_limits_are_checked() {
    let catalog = AircraftCatalog::builtins().unwrap();
    let load = catalog.resolve_aircraft("corvid").unwrap().load;
    assert!(
        catalog
            .resolve_aircraft("corvid")
            .unwrap()
            .panel
            .autopilot
            .is_none()
    );
    for panel in ["corvid-leveler", "corvid-legacy", "corvid-digital"] {
        let resolved = catalog.resolve("corvid-standard", panel, &load).unwrap();
        resolved.panel.autopilot.unwrap().validate().unwrap();
    }
    for bad in [f64::NAN, f64::INFINITY, -1.0, 0.0, 5.0] {
        let mut installation = AutopilotInstallation::new(AutopilotFamily::TwoAxisLegacy);
        installation.servo_torque = bad;
        assert!(installation.validate().is_err());
    }
    let mut leveler = AutopilotInstallation::new(AutopilotFamily::WingLeveler);
    leveler.lateral_modes.push(LateralMode::Hdg);
    assert!(leveler.validate().is_err());
    let mut legacy = AutopilotInstallation::new(AutopilotFamily::TwoAxisLegacy);
    legacy.vertical_modes.push(VerticalMode::Ias);
    assert!(legacy.validate().is_err());
}
