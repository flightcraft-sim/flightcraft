//! Slow preview flight on fetched geographic data. No state replacements after the initial start.
use super::*;
use flightcraft_world::{HeightTile, Surface, World};
use std::path::Path;

fn route_world() -> World {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain");
    let ny = crate::read_region(&root.join("new-york")).unwrap();
    let pa = crate::read_region(&root.join("eastern-pa")).unwrap();
    let mut region = pa.region().unwrap().clone();
    region.id = "atc-route".into();
    region.name = "ATC route: fetched New York and eastern Pennsylvania terrain".into();
    region.southwest_m = [-60000., -65000.];
    region.tiles_north = 4;
    region.tiles_east = 11;
    region.tiles = (0..44).map(|i| format!("{i:03}.fcht")).collect();
    let mut tiles = Vec::new();
    for e in 0..11 {
        for n in 0..4 {
            let origin = [
                region.southwest_m[0] + f64::from(n) * 15360.,
                region.southwest_m[1] + f64::from(e) * 15360.,
            ];
            let mut heights = Vec::new();
            for x in 0..257 {
                for y in 0..257 {
                    let north = origin[0] + f64::from(y) * 60.;
                    let east = origin[1] + f64::from(x) * 60.;
                    let source = if east > 0. { &ny } else { &pa };
                    heights.push(source.ground(north, east).height_msl_m as f32);
                }
            }
            tiles.push(HeightTile::new(origin, 60., 257, heights, vec![Surface::Grass; 256 * 256]).unwrap());
        }
    }
    let mut airports = pa.airports().unwrap().clone();
    for a in &ny.airports().unwrap().airports {
        if !airports.airports.iter().any(|v| v.ident == a.ident) {
            airports.airports.push(a.clone());
        }
    }
    airports.airports.retain(|a| {
        a.icao
            .as_deref()
            .is_some_and(|id| ["KMMU", "KABE", "KEWR"].contains(&id))
    });
    let mut nav = pa.navdata().unwrap().clone();
    for st in &ny.navdata().unwrap().comms {
        if !nav.comms.iter().any(|v| v.id == st.id) {
            nav.comms.push(st.clone());
        }
    }
    World::from_region(region, tiles)
        .unwrap()
        .with_airports(airports)
        .unwrap()
        .with_navdata(nav)
        .unwrap()
        .with_procedures(pa.procedures().unwrap().clone())
        .unwrap()
        .with_airspace(
            serde_json::from_str(&std::fs::read_to_string(root.join("eastern-pa/airspace.json")).unwrap())
                .unwrap(),
        )
        .unwrap()
}
struct Flight {
    s: Session,
    track: Vec<serde_json::Value>,
}
impl Flight {
    fn new(world: World, airport: &str, runway: &str) -> Self {
        let mut s = Session::new().unwrap();
        s.install_region(world).unwrap();
        s.execute(
            "sim.reset",
            json!({"world":"atc-route","start":{"kind":"airport_runway","airport":airport,"end":runway}}),
        )
        .unwrap();
        let mut point = s.sim.state().position_m;
        let heading = s.sim.readout().heading_grid_rad;
        // Initial runway-aligned hold-short start; never moved again by the generator.
        point[0] -= 120. * heading.cos();
        point[1] -= 120. * heading.sin();
        point[2] = -s.sim.world().ground(point[0], point[1]).height_msl_m;
        s.sim = flightcraft_sim::Sim::new_with_world(
            &flightcraft_sim::Start::RunwayAt {
                threshold_ned_m: point,
                heading_rad: heading,
            },
            s.sim.world().clone(),
        )
        .unwrap();
        s.execute(
            "comms.callsign",
            json!({"registration":"N371FC","spoken_type":"Sling"}),
        )
        .unwrap();
        s.execute("sim.set_controls", json!({"brake":1.,"throttle":0.}))
            .unwrap();
        s.execute("atc.configure", json!({"airport":airport,"destination":"KABE"}))
            .unwrap();
        Self { s, track: Vec::new() }
    }
    fn step(&mut self, seconds: f64) {
        advance(&mut self.s, seconds);
        let r = self.s.sim.readout();
        assert!(
            !r.crashed,
            "flight crashed at {:?}; readout {r:?}",
            self.s.sim.state()
        );
        self.track.push(json!({"time_s":self.s.sim.state().time_s,"position_m":self.s.sim.state().position_m,"velocity_mps":self.s.sim.state().velocity_mps,"altitude_ft":r.altitude_msl_m/0.3048,"heading_deg":r.heading_rad.to_degrees(),"on_ground":r.on_ground}));
    }
    fn hold(&mut self, targets: serde_json::Value) {
        self.s.execute("assist.set", targets).unwrap();
    }
    fn settle(&mut self) {
        for _ in 0..240 {
            if self.s.comms.pending.iter().all(|p| p.channel != Channel::Com)
                && self.s.comms.atc.replies.is_empty()
            {
                return;
            }
            self.step(0.25);
        }
        panic!("radio did not settle");
    }
    fn call(&mut self, text: &str) {
        self.settle();
        self.s.execute("comms.transmit", json!({"text":text})).unwrap();
        self.settle();
        eprintln!(
            "Pilot: {text}; recent events: {}",
            self.s
                .flight_record()
                .events
                .iter()
                .rev()
                .take(5)
                .map(|e| format!(
                    "{}: {} ({})",
                    e.speaker_name,
                    e.text,
                    phraseology::reception(e.reception)
                ))
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }
    fn readback(&mut self) {
        self.settle();
        let p = instruction(&self.s);
        let text = self.s.atc_readback(&p.instruction);
        self.call(&text);
        assert!(
            !self.s.comms.atc.pending.iter().any(|v| v.id == p.id),
            "readback rejected: {}",
            p.phrase
        );
    }
    fn handoff(&mut self) -> ComStation {
        self.settle();
        for _ in 0..120 {
            if !self.s.comms.atc.pending.is_empty() {
                break;
            }
            self.step(1.);
            self.settle();
        }
        let AtcInstruction::Handoff {
            station_id,
            channel_khz,
        } = instruction(&self.s).instruction
        else {
            panic!("expected automatic handoff: {:?}", self.s.atc_state());
        };
        let station = self
            .s
            .stations()
            .into_iter()
            .find(|s| s.id == station_id)
            .unwrap();
        self.readback();
        self.s
            .execute("comms.tune", json!({"radio":1,"channel_khz":channel_khz}))
            .unwrap();
        station
    }
    fn until(&mut self, limit: f64, mut predicate: impl FnMut(&Session) -> bool) {
        let start = self.s.sim.state().time_s;
        while !predicate(&self.s) {
            assert!(
                self.s.sim.state().time_s - start < limit,
                "flight condition timeout: {:?}",
                self.s.sim.readout()
            );
            self.step(1.);
        }
    }
    fn steer(&mut self, n: f64, e: f64) {
        let [pn, pe, _] = self.s.sim.state().position_m;
        let grid = (e - pe).atan2(n - pn);
        let magnetic = self
            .s
            .sim
            .heading_reference()
            .grid_to_magnetic(grid)
            .rem_euclid(std::f64::consts::TAU);
        self.hold(json!({"heading_rad":magnetic}));
    }
    fn waypoint(&mut self, point: [f64; 2], radius: f64) {
        let start = self.s.sim.state().time_s;
        loop {
            let [n, e, _] = self.s.sim.state().position_m;
            if (n - point[0]).hypot(e - point[1]) < radius {
                break;
            }
            assert!(
                self.s.sim.state().time_s - start < 4200.,
                "missed waypoint: target {point:?}; state {:?}; readout {:?}",
                self.s.sim.state(),
                self.s.sim.readout()
            );
            if (self.s.sim.state().time_s - start) % 60. < 2. {
                eprintln!(
                    "Leg: {:.0} seconds, {:.1} NM remaining, {:.0} ft, heading {:.0}",
                    self.s.sim.state().time_s - start,
                    (n - point[0]).hypot(e - point[1]) / 1852.,
                    self.s.sim.readout().altitude_msl_m / 0.3048,
                    self.s.sim.readout().heading_rad.to_degrees()
                );
            }
            self.steer(point[0], point[1]);
            self.step(2.);
            if self
                .s
                .comms
                .atc
                .pending
                .first()
                .is_some_and(|p| matches!(p.instruction, AtcInstruction::Handoff { .. }))
            {
                self.settle();
                let next = self.handoff();
                if next.airport == "KABE" {
                    self.s
                        .execute("atc.configure", json!({"airport":"KABE"}))
                        .unwrap();
                    let r = self.s.sim.readout();
                    let (status, feet) = if r.vertical_speed_mps > 0.3 {
                        (
                            "climbing",
                            r.autopilot.altitude_m.unwrap_or(r.altitude_msl_m) / 0.3048,
                        )
                    } else if r.vertical_speed_mps < -0.3 {
                        (
                            "descending",
                            r.autopilot.altitude_m.unwrap_or(r.altitude_msl_m) / 0.3048,
                        )
                    } else {
                        ("level", r.altitude_msl_m / 0.3048)
                    };
                    let ft = (feet / 100.).round().max(0.) as u32 * 100;
                    self.call(&format!(
                        "{}, {}, {status} {}",
                        phraseology::facility(&next),
                        self.s.pilot_callsign().full,
                        phraseology::altitude(ft)
                    ));
                }
            } else if self.s.comms.atc.pending.first().is_some_and(|p| p.heard) {
                let altitude = match instruction(&self.s).instruction {
                    AtcInstruction::Altitude { altitude_ft } => Some(altitude_ft),
                    _ => None,
                };
                self.readback();
                if let Some(ft) = altitude {
                    self.hold(json!({"altitude_m":f64::from(ft)*0.3048}));
                }
            }
        }
    }
    fn air(world: World, airport: &str, lat: f64, lon: f64, altitude_m: f64, heading_rad: f64) -> Self {
        let origin = world.region().unwrap().origin.clone();
        let projection =
            flightcraft_world::geo::LocalProjection::new(origin.latitude_deg, origin.longitude_deg).unwrap();
        let [n, e] = projection.project(lat, lon).unwrap();
        let agl = altitude_m - world.ground(n, e).height_msl_m;
        let mut s = Session::new().unwrap();
        s.install_region(world).unwrap();
        s.execute("sim.reset",json!({"world":"atc-route","start":{"kind":"geographic_air","position":{"latitude_deg":lat,"longitude_deg":lon},"height_agl_m":agl,"airspeed_mps":36.,"heading_rad":heading_rad}})).unwrap();
        s.execute(
            "comms.callsign",
            json!({"registration":"N371FC","spoken_type":"Sling"}),
        )
        .unwrap();
        s.execute("atc.configure", json!({"airport":airport})).unwrap();
        s.execute(
            "assist.set",
            json!({"altitude_m":altitude_m,"airspeed_mps":36.,"heading_rad":heading_rad}),
        )
        .unwrap();
        Self { s, track: Vec::new() }
    }
    fn land(&mut self, threshold: [f64; 2], forward: [f64; 2], elevation_m: f64) {
        self.hold(json!({"airspeed_mps":32.,"altitude_m":null,"vertical_speed_mps":-3.}));
        loop {
            let [n, e, d] = self.s.sim.state().position_m;
            let along = (threshold[0] - n) * forward[0] + (threshold[1] - e) * forward[1];
            let target = [threshold[0] + 300. * forward[0], threshold[1] + 300. * forward[1]];
            self.steer(target[0], target[1]);
            let height = -d - elevation_m;
            if height < 8. {
                break;
            }
            let distance = along.max(100.);
            let vs = (-(height - 4.) / (distance / 32.)).clamp(-4., -0.5);
            self.hold(json!({"vertical_speed_mps":vs}));
            self.step(1.);
        }
        self.hold(json!({"vertical_speed_mps":-0.6}));
        self.until(60., |s| s.sim.readout().height_agl_m < 3.);
        self.hold(json!({"airspeed_mps":null}));
        self.s
            .execute("sim.set_controls", json!({"throttle":0.}))
            .unwrap();
        self.until(30., |s| s.sim.readout().on_ground);
        self.hold(json!({"vertical_speed_mps":null,"pitch_rad":0.}));
        self.s
            .execute("sim.set_controls", json!({"throttle":0.,"brake":1.}))
            .unwrap();
        self.until(60., |s| s.sim.readout().true_airspeed_mps < 0.2);
        assert!(
            self.s.on_atc_runway("KABE", "06"),
            "touchdown must be on runway six"
        );
    }
    fn save(&mut self, name: &str, title: &str, note: &str) {
        self.settle();
        save_preview(&mut self.s, name, title, note);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/atc");
        std::fs::write(
            root.join(format!("{name}-track.json")),
            serde_json::to_string_pretty(&self.track).unwrap(),
        )
        .unwrap();
    }
}
pub(super) fn route_preview() {
    let mut f = Flight::new(route_world(), "KMMU", "05");
    tune(&mut f.s, "KMMU", FacilityKind::Atis);
    f.step(1.);
    f.settle();
    let st = tune(&mut f.s, "KMMU", FacilityKind::Clearance);
    let info = f.s.atis_report("KMMU").unwrap().information;
    f.call(&format!(
        "{}, {}, with information {info}, request IFR clearance to Allentown",
        phraseology::facility(&st),
        f.s.pilot_callsign().full
    ));
    let AtcInstruction::IfrClearance {
        code, altitude_ft, ..
    } = instruction(&f.s).instruction
    else {
        panic!("CRAFT");
    };
    f.readback();
    f.s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"transponder","code":code,"mode":"alt"}}}),
    )
    .unwrap();
    let st = f.handoff();
    f.call(&format!(
        "{}, {}, with information {info}, request taxi",
        phraseology::facility(&st),
        f.s.pilot_callsign().full
    ));
    f.readback();
    let st = f.handoff();
    f.call(&format!(
        "{}, {}, ready for departure runway zero five",
        phraseology::facility(&st),
        f.s.pilot_callsign().full
    ));
    f.readback();
    let runway_heading = f.s.sim.readout().heading_rad;
    f.hold(json!({"pitch_rad":0.,"heading_rad":runway_heading}));
    f.s.execute("sim.set_controls", json!({"brake":0.,"throttle":1.}))
        .unwrap();
    f.until(90., |s| s.sim.readout().true_airspeed_mps > 28.);
    f.hold(json!({"pitch_rad":0.20}));
    f.until(60., |s| s.sim.readout().height_agl_m > 25.);
    f.hold(json!({"pitch_rad":null,"airspeed_mps":36.,"airspeed_mode":"pitch"}));
    f.until(60., |s| !s.comms.atc.pending.is_empty());
    let st = f.handoff();
    f.call(&format!(
        "{}, {}, climbing {}",
        phraseology::facility(&st),
        f.s.pilot_callsign().full,
        phraseology::altitude(altitude_ft)
    ));
    f.readback();
    // Fly the filed westbound route at the newly assigned altitude, rather than teleporting.
    f.until(480., |s| s.sim.readout().altitude_msl_m >= 5000. * 0.3048 - 5.);
    f.hold(json!({"altitude_m":5000.*0.3048,"airspeed_mode":"throttle","airspeed_mps":45.}));
    let a = crate::regions::airport(f.s.sim.world(), "KABE").unwrap().clone();
    let end = a
        .runways
        .iter()
        .flat_map(|r| &r.ends)
        .find(|e| e.ident == "06")
        .unwrap();
    let threshold = f.s.radio_project(end.latitude_deg, end.longitude_deg).unwrap();
    let grid = end.heading_deg.to_radians() - f.s.sim.heading_reference().grid_north_true_rad;
    let forward = [grid.cos(), grid.sin()];
    let intercept = [
        threshold[0] - 12000. * forward[0],
        threshold[1] - 12000. * forward[1],
    ];
    f.waypoint(intercept, 600.);
    f.steer(threshold[0], threshold[1]);
    f.hold(json!({"airspeed_mps":32.}));
    if f.s.atc_state().airport.as_deref() != Some("KABE") {
        panic!("arrival handoff missing");
    }
    tune(&mut f.s, "KABE", FacilityKind::Atis);
    f.step(1.);
    f.settle();
    let st = tune(&mut f.s, "KABE", FacilityKind::Approach);
    let info = f.s.atis_report("KABE").unwrap().information;
    f.call(&format!(
        "{}, {}, with information {info}, airport in sight, request visual approach runway zero six",
        phraseology::facility(&st),
        f.s.pilot_callsign().full
    ));
    f.readback();
    let st = f.handoff();
    f.call(&format!(
        "{}, {}, with information {info}, inbound straight in runway zero six",
        phraseology::facility(&st),
        f.s.pilot_callsign().full
    ));
    f.readback();
    if !f.s.comms.atc.landing {
        f.call(&format!(
            "Final runway zero six, {}",
            f.s.pilot_callsign().current
        ));
        f.readback();
    }
    f.land(threshold, forward, end.elevation_m);
    f.save("kmmu-kabe","Morristown to Allentown: continuous IFR flight","Continuously flown with hold commands on fetched New York/eastern Pennsylvania elevations and FAA airport/NASR records. Starts aligned before runway 05; ends stopped on runway 06. No taxiway graph or invented runway exit. The accompanying track records every flight segment; no positions are replaced after the initial start.");
}

#[test]
#[ignore = "slow continuously flown route preview on fetched region data"]
fn write_flown_route_preview() {
    route_preview();
}

pub(super) fn other_previews() {
    let world = route_world();
    let a = crate::regions::airport(&world, "KABE").unwrap();
    let end = a
        .runways
        .iter()
        .flat_map(|r| &r.ends)
        .find(|e| e.ident == "06")
        .unwrap()
        .clone();
    let origin = world.region().unwrap().origin.clone();
    let projection =
        flightcraft_world::geo::LocalProjection::new(origin.latitude_deg, origin.longitude_deg).unwrap();
    let threshold = projection.project(end.latitude_deg, end.longitude_deg).unwrap();
    let heading = end.heading_deg.to_radians();
    let forward = [heading.cos(), heading.sin()];
    let left = [-forward[1], forward[0]];
    let start = [
        threshold[0] - 22000. * forward[0],
        threshold[1] - 22000. * forward[1],
    ];
    let geo = projection.unproject(start[0], start[1]).unwrap();
    let mut f = Flight::air(
        world.clone(),
        "KABE",
        geo[0],
        geo[1],
        2500. * 0.3048,
        heading + 12_f64.to_radians(),
    );
    tune(&mut f.s, "KABE", FacilityKind::Atis);
    f.step(1.);
    f.settle();
    let info = f.s.atis_report("KABE").unwrap().information;
    let st = tune(&mut f.s, "KABE", FacilityKind::Approach);
    f.call(&format!(
        "{}, {}, with information {info}, inbound for landing",
        phraseology::facility(&st),
        f.s.pilot_callsign().full
    ));
    let _ = f.handoff();
    f.s.execute("traffic.set", json!({"airport":"KABE","runway":"06","count":3}))
        .unwrap();
    f.call(&format!(
        "Allentown Tower, {}, with information {info}, inbound left downwind runway zero six",
        f.s.pilot_callsign().full
    ));
    f.readback();
    let entry = [
        threshold[0] + 3200. * forward[0] + 1800. * left[0],
        threshold[1] + 3200. * forward[1] + 1800. * left[1],
    ];
    f.waypoint(entry, 600.);
    f.call(&format!(
        "Left downwind runway zero six, {}",
        f.s.pilot_callsign().current
    ));
    f.readback();
    let downwind = [
        threshold[0] - 3500. * forward[0] + 1800. * left[0],
        threshold[1] - 3500. * forward[1] + 1800. * left[1],
    ];
    f.waypoint(downwind, 600.);
    if !f.s.comms.atc.pending.is_empty() {
        f.readback();
    }
    let base = [
        threshold[0] - 3500. * forward[0],
        threshold[1] - 3500. * forward[1],
    ];
    f.waypoint(base, 500.);
    f.call(&format!(
        "Left base runway zero six, {}",
        f.s.pilot_callsign().current
    ));
    let mut go_around = matches!(instruction(&f.s).instruction, AtcInstruction::GoAround { .. });
    f.readback();
    if !go_around {
        let final_point = [
            threshold[0] - 2000. * forward[0],
            threshold[1] - 2000. * forward[1],
        ];
        f.waypoint(final_point, 500.);
        f.call(&format!(
            "Final runway zero six, {}",
            f.s.pilot_callsign().current
        ));
        go_around = matches!(instruction(&f.s).instruction, AtcInstruction::GoAround { .. });
        f.readback();
    }
    if !go_around {
        f.call(&format!(
            "Going around runway zero six, {}",
            f.s.pilot_callsign().current
        ));
        f.readback();
    }
    f.hold(json!({"altitude_m":null,"vertical_speed_mps":2.,"airspeed_mps":36.}));
    f.step(20.);
    f.save("kabe-pattern","Allentown circuit with traffic","Continuous flight from outside the Class C through approach/tower contact, pattern entry, downwind, base, final when cleared, and a go-around. Three deterministic circuit aircraft use their actual current positions. Real fetched terrain; no position replacements.");

    let mut f = Flight::air(world.clone(), "KEWR", 40.75, -74.025, 300., 0.);
    let st = tune(&mut f.s, "KEWR", FacilityKind::Approach);
    f.call(&format!(
        "{}, {}, request Class Bravo transition",
        phraseology::facility(&st),
        f.s.pilot_callsign().full
    ));
    f.readback();
    f.call(&format!(
        "Request flight following, {}",
        f.s.pilot_callsign().current
    ));
    let AtcInstruction::Squawk { code, .. } = instruction(&f.s).instruction else {
        panic!("squawk");
    };
    f.readback();
    f.s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"transponder","code":code,"mode":"alt"}}}),
    )
    .unwrap();
    f.call(&format!(
        "Request Class Bravo transition, altitude two thousand five hundred, {}",
        f.s.pilot_callsign().current
    ));
    f.readback();
    f.hold(json!({"altitude_m":2500.*0.3048}));
    f.until(300., |s| s.sim.readout().altitude_msl_m >= 2500. * 0.3048 - 5.);
    f.step(10.);
    f.save("hudson-bravo","Hudson Class Bravo transition","Continuous VFR flight near the Hudson, starting below the Bravo floor. Remain outside, select the assigned code, then receive the explicit Bravo clearance before entry. The held heading and altitude match the instructions.");

    let mut f = Flight::air(
        world,
        "KABE",
        geo[0],
        geo[1],
        2500. * 0.3048,
        heading + 12_f64.to_radians(),
    );
    tune(&mut f.s, "KABE", FacilityKind::Atis);
    f.step(1.);
    f.settle();
    let info = f.s.atis_report("KABE").unwrap().information;
    let st = tune(&mut f.s, "KABE", FacilityKind::Approach);
    f.call(&format!(
        "{}, {}, with information {info}, inbound for landing",
        phraseology::facility(&st),
        f.s.pilot_callsign().full
    ));
    let _ = f.handoff();
    f.call(&format!(
        "Allentown Tower, {}, with information {info}, inbound straight in runway zero six",
        f.s.pilot_callsign().full
    ));
    f.readback();
    f.s.execute("failure.set", json!({"failure":{"kind":"radio_failure"}}))
        .unwrap();
    for signal in [
        LightGunSignal::FlashingGreen,
        LightGunSignal::SteadyRed,
        LightGunSignal::SteadyGreen,
    ] {
        f.s.run(SetLightGun {
            airport: "KABE".into(),
            signal,
        })
        .unwrap();
        f.step(5.);
        if signal == LightGunSignal::SteadyRed {
            let original = f.s.sim.readout().heading_rad;
            for target in [
                (original + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU),
                original,
            ] {
                f.hold(json!({"heading_rad":target}));
                f.until(120., |s| {
                    flightcraft_sim::autopilot::wrap(s.sim.readout().heading_rad - target).abs() < 0.08
                });
            }
        }
    }
    f.land(threshold, forward, end.elevation_m);
    for signal in [
        LightGunSignal::FlashingRed,
        LightGunSignal::FlashingWhite,
        LightGunSignal::AlternatingRedGreen,
    ] {
        f.s.run(SetLightGun {
            airport: "KABE".into(),
            signal,
        })
        .unwrap();
        f.step(5.);
    }
    f.save("lost-comms","Lost communications: continuous arrival","Continuous Allentown arrival after established approach/tower contact. COM fails; tower signals return, give way, and cleared to land. The aircraft lands under steady green, then the remaining ground signals are demonstrated while stopped. Signal positions remain estimates.");
}

#[test]
#[ignore = "audits generated continuous tracks, runway stops, radio wording, and grades"]
fn validate_generated_atc_previews() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let region: flightcraft_world::Region = serde_json::from_str(
        &std::fs::read_to_string(root.join("assets/terrain/eastern-pa/region.json")).unwrap(),
    )
    .unwrap();
    let projection =
        flightcraft_world::geo::LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
            .unwrap();
    let airports: flightcraft_world::Airports = serde_json::from_str(
        &std::fs::read_to_string(root.join("assets/terrain/eastern-pa/airports.json")).unwrap(),
    )
    .unwrap();
    let airport = airports
        .airports
        .iter()
        .find(|a| a.icao.as_deref() == Some("KABE"))
        .unwrap();
    let runway = airport
        .runways
        .iter()
        .find(|r| r.ends.iter().any(|e| e.ident == "06"))
        .unwrap();
    let [near, far] = &runway.ends;
    let a = projection.project(near.latitude_deg, near.longitude_deg).unwrap();
    let b = projection.project(far.latitude_deg, far.longitude_deg).unwrap();
    let length = (b[0] - a[0]).hypot(b[1] - a[1]);
    for name in ["kmmu-kabe", "kabe-pattern", "hudson-bravo", "lost-comms"] {
        let path = root.join("out/atc");
        let track: Vec<serde_json::Value> =
            serde_json::from_str(&std::fs::read_to_string(path.join(format!("{name}-track.json"))).unwrap())
                .unwrap();
        assert!(track.len() > 10);
        let point =
            |v: &serde_json::Value| -> [f64; 3] { serde_json::from_value(v["position_m"].clone()).unwrap() };
        let mut max_speed = 0_f64;
        for pair in track.windows(2) {
            let dt = pair[1]["time_s"].as_f64().unwrap() - pair[0]["time_s"].as_f64().unwrap();
            assert!(dt > 0., "{name}: time must advance");
            let x = point(&pair[0]);
            let y = point(&pair[1]);
            let speed = ((y[0] - x[0]).powi(2) + (y[1] - x[1]).powi(2) + (y[2] - x[2]).powi(2)).sqrt() / dt;
            max_speed = max_speed.max(speed);
            assert!(
                speed < 70.,
                "{name}: discontinuous track at {} seconds",
                pair[0]["time_s"]
            );
        }
        if name == "kmmu-kabe" || name == "lost-comms" {
            let last = track.last().unwrap();
            assert_eq!(last["on_ground"], true);
            let [n, e, _] = point(last);
            let along = ((n - a[0]) * (b[0] - a[0]) + (e - a[1]) * (b[1] - a[1])) / length;
            let side = (-(n - a[0]) * (b[1] - a[1]) + (e - a[1]) * (b[0] - a[0])) / length;
            assert!(
                (0. ..=length).contains(&along),
                "{name}: stopped outside the runway length: {along}"
            );
            assert!(
                side.abs() <= runway.width_m / 2.,
                "{name}: stopped outside pavement: {side}"
            );
            eprintln!("{name}: stopped {along:.1} m along runway, {side:.1} m from centerline");
        }
        let record: FlightRecord =
            serde_json::from_str(&std::fs::read_to_string(path.join(format!("{name}.json"))).unwrap())
                .unwrap();
        assert!(
            record
                .events
                .iter()
                .all(|e| e.reception != Reception::Pending && !e.speaker_name.is_empty())
        );
        for event in &record.events {
            if let Meaning::Compliance { grade } = &event.meaning {
                assert!(
                    grade.passed || (name == "lost-comms" && grade.item == "radio_failure"),
                    "{name}: unintended grade: {}",
                    grade.observed
                );
            }
        }
        for extension in ["md", "html"] {
            let text = std::fs::read_to_string(path.join(format!("{name}.{extension}"))).unwrap();
            for dump in [
                "Some(",
                "Facility {",
                "role: Tower",
                "request handoff",
                "KABE airport",
                "KMMU airport",
            ] {
                assert!(
                    !text.contains(dump),
                    "{name}: invalid student-facing wording: {dump}"
                );
            }
        }
        eprintln!(
            "{name}: {} continuous samples; maximum interval speed {max_speed:.1} m/s",
            track.len()
        );
    }
}
