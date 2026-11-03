#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_sim::systems::{SystemSetting, SystemSwitch};
use flightcraft_sim::{Failures, Sim, Start};
fn airborne() -> Sim {
    Sim::new(&Start::Air {
        height_m: 1200.,
        airspeed_mps: 35.,
        heading_rad: 0.,
    })
    .unwrap()
}
#[test]
fn door_flaps_trim_fire_and_ice_have_distinct_physical_effects() {
    let healthy = airborne();
    for f in [
        Failures {
            engine_fire: true,
            ..Default::default()
        },
        Failures {
            door_open: true,
            ..Default::default()
        },
        Failures {
            flap_asymmetry: true,
            ..Default::default()
        },
        Failures {
            trim_runaway: true,
            ..Default::default()
        },
        Failures {
            trim_stuck: true,
            ..Default::default()
        },
    ] {
        let mut a = healthy.clone();
        let mut b = healthy.clone();
        a.set_failures(f).unwrap();
        a.step(120);
        b.step(120);
        assert_ne!(a.state(), b.state(), "{f:?}");
        assert!(!a.readout().crashed);
        assert_ne!(a.state_hash(), b.state_hash());
        if f.engine_fire {
            assert!(a.systems().cht_c > b.systems().cht_c);
            assert!(a.systems().oil_temperature_c > b.systems().oil_temperature_c);
        }
    }
    let mut a = healthy.clone();
    let mut b = healthy.clone();
    a.set_failures(Failures {
        airframe_icing: true,
        ..Default::default()
    })
    .unwrap();
    a.step(120);
    b.step(120);
    assert_eq!(
        a.state(),
        b.state(),
        "warm clear air must not accrete airframe ice"
    );
    let mut c = airborne();
    let mut d = airborne();
    let mut conditions = flightcraft_world::Conditions::preset(flightcraft_world::WeatherPreset::Ifr);
    conditions.temperature_c = -5.;
    conditions.dewpoint_c = -6.;
    conditions.precipitation = flightcraft_world::Precipitation::Snow;
    c.set_conditions(conditions.clone()).unwrap();
    d.set_conditions(conditions).unwrap();
    c.set_failures(Failures {
        airframe_icing: true,
        ..Default::default()
    })
    .unwrap();
    c.step(120);
    d.step(120);
    assert_ne!(c.state(), d.state());
}
#[test]
fn smoke_accumulates_and_clears_and_electrical_fire_isolates_buses() {
    let mut a = airborne();
    a.set_failures(Failures {
        cabin_smoke: true,
        ..Default::default()
    })
    .unwrap();
    a.step(1200);
    assert!(a.systems().smoke_density > 0.6);
    a.set_failures(Failures::default()).unwrap();
    a.step(1200);
    assert!(a.systems().smoke_density < 0.4);
    a.set_failures(Failures {
        electrical_fire: true,
        ..Default::default()
    })
    .unwrap();
    a.step(1);
    assert_eq!(a.systems().electrical.main_bus_voltage, 0.);
    assert_eq!(a.systems().electrical.essential_bus_voltage, 0.);
}
#[test]
fn forced_carb_ice_requires_humidity_temperature_and_carburettor_and_heat_clears() {
    let mut a = airborne();
    let mut b = airborne();
    for s in [&mut a, &mut b] {
        s.set_systems(SystemSetting::Environment {
            temperature_c: 15.,
            relative_humidity: 0.95,
        })
        .unwrap();
    }
    a.set_failures(Failures {
        carb_icing: true,
        ..Default::default()
    })
    .unwrap();
    a.step(2400);
    b.step(2400);
    assert!(a.systems().carb_ice > b.systems().carb_ice + 0.1);
    assert!(a.systems().engine_power_fraction < b.systems().engine_power_fraction);
    let before = a.systems().carb_ice;
    a.set_systems(SystemSetting::Switch {
        switch: SystemSwitch::CarbHeat,
        on: true,
    })
    .unwrap();
    a.step(1200);
    assert!(a.systems().carb_ice < before);
    let mut dry = airborne();
    dry.set_systems(SystemSetting::Environment {
        temperature_c: 15.,
        relative_humidity: 0.1,
    })
    .unwrap();
    dry.set_failures(Failures {
        carb_icing: true,
        ..Default::default()
    })
    .unwrap();
    dry.step(1200);
    assert_eq!(dry.systems().carb_ice, 0.);
    let aircraft = flightcraft_aero::Aircraft::larrow().unwrap();
    let mut injected = Sim::new_with_aircraft(
        &Start::Air {
            height_m: 1200.,
            airspeed_mps: 40.,
            heading_rad: 0.,
        },
        aircraft,
    )
    .unwrap();
    injected
        .set_systems(SystemSetting::Environment {
            temperature_c: 15.,
            relative_humidity: 0.95,
        })
        .unwrap();
    injected
        .set_failures(Failures {
            carb_icing: true,
            ..Default::default()
        })
        .unwrap();
    injected.step(1200);
    assert_eq!(injected.systems().carb_ice, 0.);
}
