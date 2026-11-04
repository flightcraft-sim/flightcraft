#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use crate as flightcraft_control;
use serde_json::json;
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));

#[test]
fn symbology_classes_follow_the_adsb_thresholds() {
    // Beyond 6 NM or 1,200 ft: other traffic.
    assert_eq!(classify(12., 0., false, None), TrafficClass::Other);
    assert_eq!(classify(5., 1300., false, None), TrafficClass::Other);
    assert_eq!(classify(6.1, 0., false, None), TrafficClass::Other);
    // Inside 6 NM and 1,200 ft: proximate, with the boundaries inclusive.
    assert_eq!(classify(6., 1200., false, None), TrafficClass::Proximate);
    assert_eq!(classify(3., -1100., false, None), TrafficClass::Proximate);
    // Inside 2 NM and 500 ft: alert. A little more of either is only proximate.
    assert_eq!(classify(2., 500., false, None), TrafficClass::Alert);
    assert_eq!(classify(0.4, -450., false, None), TrafficClass::Alert);
    assert_eq!(classify(2.1, 100., false, None), TrafficClass::Proximate);
    assert_eq!(classify(1., 600., false, None), TrafficClass::Proximate);
    // Closing: a pass inside 1 NM and 500 ft within 45 s alerts from out to 6 NM.
    assert_eq!(
        classify(4., 200., false, Some((40., 0.5, 100.))),
        TrafficClass::Alert
    );
    assert_eq!(
        classify(4., 200., false, Some((50., 0.5, 100.))),
        TrafficClass::Proximate
    );
    assert_eq!(
        classify(4., 200., false, Some((40., 1.5, 100.))),
        TrafficClass::Proximate
    );
    assert_eq!(
        classify(4., 200., false, Some((40., 0.5, 700.))),
        TrafficClass::Proximate
    );
    assert_eq!(
        classify(8., 200., false, Some((40., 0.5, 100.))),
        TrafficClass::Other
    );
    // Aircraft on the ground never alert.
    assert_eq!(classify(0.5, 0., true, None), TrafficClass::Other);
}

#[test]
fn closest_approach_projects_head_on_and_diverging_traffic() {
    // 3 NM ahead, flying straight at us: 120 kt closure puts the pass 90 s out, miss distance zero.
    let (tau, miss, altitude) = closest_approach([0., 3.], [0., -120.], 600., -400.).unwrap();
    assert!((tau - 90.).abs() < 1e-6 && miss < 1e-9);
    assert!((altitude - (600. - 400. * 1.5)).abs() < 1e-6);
    // Offset by a mile and moving the same way: the miss distance is the offset.
    let (_, miss, _) = closest_approach([1., 3.], [0., -120.], 0., 0.).unwrap();
    assert!((miss - 1.).abs() < 1e-9);
    // Moving away, or no relative motion: not closing.
    assert!(closest_approach([0., 3.], [0., 60.], 0., 0.).is_none());
    assert!(closest_approach([0., 3.], [0., 0.], 0., 0.).is_none());
}

#[test]
fn filters_match_glass_altitude_windows() {
    assert!(TrafficFilter::Normal.shows(2700.) && TrafficFilter::Normal.shows(-2700.));
    assert!(!TrafficFilter::Normal.shows(2701.) && !TrafficFilter::Normal.shows(-2701.));
    assert!(TrafficFilter::Above.shows(9900.) && !TrafficFilter::Above.shows(-2800.));
    assert!(TrafficFilter::Below.shows(-9900.) && !TrafficFilter::Below.shows(2800.));
    assert!(TrafficFilter::Unrestricted.shows(-9900.) && TrafficFilter::Unrestricted.shows(9900.));
    assert!(!TrafficFilter::Unrestricted.shows(10_000.));
    let mut f = TrafficFilter::Normal;
    for _ in 0..4 {
        f = f.next();
    }
    assert_eq!(f, TrafficFilter::Normal);
}

#[test]
fn altitude_tags_and_trend_arrows() {
    assert_eq!(altitude_tag(500.), "+05");
    assert_eq!(altitude_tag(-1240.), "-12");
    assert_eq!(altitude_tag(30.), "00");
    assert_eq!(altitude_tag(-30.), "00");
    assert_eq!(altitude_tag(12_000.), "+120");
    assert_eq!((trend(600.), trend(-600.), trend(500.), trend(0.)), (1, -1, 0, 0));
}

#[test]
fn references_come_from_ids_callsigns_and_advisories_but_not_the_speaker() {
    let mut found = TrafficReferences::default();
    collect_references(
        &json!({"type":"x","target_aircraft":7,"nested":{"aircraft_ids":[3,9],"follow":"Cessna One"}}),
        false,
        &mut found,
    );
    found.aircraft_ids.sort_unstable();
    assert_eq!(found.aircraft_ids, vec![3, 7, 9]);
    assert_eq!(found.callsigns, vec!["Cessna One".to_string()]);
    // `callsign` names traffic only in an advisory; the fleet call's own fields never do.
    let mut plain = TrafficReferences::default();
    collect_references(&json!({"callsign":"Speaker","aircraft":4}), false, &mut plain);
    assert_eq!(plain, TrafficReferences::default());
    let mut advisory = TrafficReferences::default();
    collect_references(&json!({"callsign":"Piper Two"}), true, &mut advisory);
    assert_eq!(advisory.callsigns, vec!["Piper Two".to_string()]);
}

fn busy_session() -> Session {
    let mut s = atc_session("KABE");
    s.execute("traffic.density", json!({"density":"busy","airport":"KABE"}))
        .unwrap();
    s
}
fn call(s: &Session, meaning: Meaning, reception: Reception) -> Transmission {
    Transmission {
        id: 10_000,
        time_s: s.sim().state().time_s,
        duration_s: 4.,
        channel: Channel::Com,
        frequency_hz: None,
        speaker: Speaker::Facility {
            id: "KABE-TWR".into(),
            role: FacilityKind::Tower,
        },
        speaker_name: "Allentown Tower".into(),
        text: "Number two, follow the traffic.".into(),
        meaning,
        source: EventSource::Rules,
        reception,
        reason: None,
        grade: None,
    }
}
fn follow(callsign: &str, entry: PatternEntry) -> Meaning {
    Meaning::Atc {
        instruction: AtcInstruction::Pattern {
            runway: "06".into(),
            entry,
            number: 2,
            follow: Some(callsign.into()),
        },
    }
}

#[test]
fn a_sequencing_call_to_the_user_highlights_only_the_aircraft_it_names() {
    let mut s = busy_session();
    let targets = s.traffic_targets();
    assert!(targets.len() >= 8, "{}", targets.len());
    assert!(targets.iter().all(|t| !t.highlighted));
    // Nearest first.
    assert!(targets.windows(2).all(|w| w[0].range_nm <= w[1].range_nm));
    let named = targets
        .iter()
        .find(|t| t.origin == TrafficOrigin::Fleet)
        .unwrap()
        .clone();
    let event = call(
        &s,
        follow(&named.callsign, PatternEntry::LeftBase),
        Reception::Heard,
    );
    s.comms.record.events.push(event);
    let lit: Vec<_> = s
        .traffic_targets()
        .into_iter()
        .filter(|t| t.highlighted)
        .collect();
    assert_eq!(
        lit.len(),
        1,
        "{:?}",
        lit.iter().map(|t| &t.callsign).collect::<Vec<_>>()
    );
    assert_eq!(lit[0].address, named.address);
    assert!(lit[0].highlight_remaining_s > 15. && lit[0].highlight_remaining_s <= HIGHLIGHT_S);
    // It fades about 20 s after the call ends.
    s.run(flightcraft_control::Step { steps: 120 * 30 }).unwrap();
    assert!(s.traffic_targets().iter().all(|t| !t.highlighted));
}

#[test]
fn calls_that_were_not_heard_highlight_nothing_and_registrations_match() {
    let mut s = busy_session();
    let named = s.traffic_targets().into_iter().next().unwrap();
    let blocked = call(
        &s,
        follow(&named.callsign, PatternEntry::StraightIn),
        Reception::Blocked,
    );
    s.comms.record.events.push(blocked);
    assert!(s.traffic_targets().iter().all(|t| !t.highlighted));
    let heard = call(
        &s,
        follow(&named.registration, PatternEntry::StraightIn),
        Reception::Heard,
    );
    s.comms.record.events.push(heard);
    assert!(
        s.traffic_targets()
            .iter()
            .any(|t| t.highlighted && t.address == named.address)
    );
}

#[test]
fn fleet_controller_sequence_calls_name_the_followed_aircraft() {
    let mut s = busy_session();
    let targets = s.traffic_targets();
    let (leader, follower) = (&targets[0], &targets[1]);
    let call_data = FleetCall {
        exchange: 1,
        aircraft: follower.fleet_id.unwrap(),
        callsign: follower.callsign.clone(),
        role: FleetRole::Controller,
        kind: FleetCallKind::Instruction,
        instructions: Vec::new(),
        facility: "Allentown Tower".into(),
        frequency_khz: 120_500,
        voice: 0,
        for_user: false,
        sequence: Some(FleetSequence {
            number: 2,
            follows: leader.callsign.clone(),
        }),
    };
    let event = call(&s, Meaning::Fleet { call: call_data }, Reception::Heard);
    let refs = traffic_references(&event);
    assert_eq!(refs.callsigns, vec![leader.callsign.clone()]);
    s.comms.record.events.push(event);
    let lit: Vec<_> = s
        .traffic_targets()
        .into_iter()
        .filter(|t| t.highlighted)
        .map(|t| t.address)
        .collect();
    assert_eq!(lit, vec![leader.address]);
}

#[test]
fn targets_carry_fleet_identity_motion_and_classification() {
    let s = busy_session();
    for t in s.traffic_targets() {
        let id = t.fleet_id.unwrap();
        assert_eq!(t.address, FLEET_ADDRESS_BASE + id);
        assert!(!t.callsign.is_empty() && !t.registration.is_empty() && !t.type_designator.is_empty());
        assert!((0. ..360.).contains(&t.track_true_deg));
        let plain = classify(t.range_nm, t.relative_altitude_ft, t.on_ground, None);
        // Closing can only raise a class, never lower it.
        assert!(t.class as u8 >= plain as u8);
    }
}

#[test]
fn every_fleet_aircraft_becomes_a_gdl90_traffic_report_with_a_stable_address() {
    let s = busy_session();
    let targets = s.traffic_targets();
    let reports: Vec<_> = targets.iter().map(crate::efb::efb_traffic).collect();
    assert_eq!(reports.len(), targets.len());
    let mut first: Vec<u32> = reports.iter().map(|r| r.address).collect();
    first.sort_unstable();
    first.dedup();
    assert_eq!(first.len(), targets.len(), "addresses must be unique");
    for (t, r) in targets.iter().zip(&reports) {
        assert_eq!(r.address, FLEET_ADDRESS_BASE + t.fleet_id.unwrap());
        let packet = crate::efb::encode::traffic_report(r);
        assert_eq!((packet.len(), packet[0]), (28, 20));
        assert_eq!(&packet[2..5], &r.address.to_be_bytes()[1..]);
        assert_eq!(
            &packet[5..8],
            &crate::efb::encode::semicircle(t.latitude_deg.unwrap())
        );
        assert_eq!(packet[1] >> 4, u8::from(t.class == TrafficClass::Alert));
        assert_eq!(packet[18], 1, "all busy-airspace types weigh under 15,500 lb");
    }
    // The same seed gives the same addresses on a second run.
    let mut second: Vec<u32> = busy_session()
        .traffic_targets()
        .iter()
        .map(|t| crate::efb::efb_traffic(t).address)
        .collect();
    second.sort_unstable();
    assert_eq!(first, second);
}

#[test]
fn equipment_gates_the_traffic_receiver() {
    // The flat-terrain fixture flies the analog Corvid: no glass display, so no traffic receiver.
    let analog = atc_session("KABE");
    assert!(!analog.adsb_in_installed());
    assert!(!analog.glass_state().traffic_installed);
    let mut glass = Session::new().unwrap();
    glass
        .execute(
            "aircraft.load",
            json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
        )
        .unwrap();
    assert!(glass.adsb_in_installed());
    assert!(glass.glass_state().traffic_installed);
}

#[test]
fn a_remote_screen_snapshot_carries_the_busy_fleet_and_its_highlight() {
    let mut owner = busy_session();
    let named = owner
        .traffic_targets()
        .into_iter()
        .find(|t| t.origin == TrafficOrigin::Fleet)
        .unwrap();
    let event = call(
        &owner,
        follow(&named.callsign, PatternEntry::LeftBase),
        Reception::Heard,
    );
    owner.comms.record.events.push(event);
    let snapshot = owner.screen_snapshot();
    assert!(!snapshot.traffic.is_empty());
    assert!(snapshot.traffic.len() <= flightcraft_control::presentation::MAX_SNAPSHOT_TRAFFIC);
    assert!(
        snapshot
            .traffic
            .windows(2)
            .all(|w| w[0].range_nm <= w[1].range_nm)
    );
    let wire = serde_json::to_string(&snapshot).unwrap();
    let mut remote = atc_session("KABE");
    assert!(remote.traffic_targets().is_empty());
    remote
        .apply_screen_snapshot(serde_json::from_str(&wire).unwrap())
        .unwrap();
    let seen = remote.traffic_targets();
    assert_eq!(
        seen.iter().map(|t| t.address).collect::<Vec<_>>(),
        snapshot.traffic.iter().map(|t| t.address).collect::<Vec<_>>()
    );
    let lit: Vec<u32> = seen.iter().filter(|t| t.highlighted).map(|t| t.address).collect();
    assert_eq!(lit, vec![named.address]);
}

#[test]
fn snapshot_traffic_is_capped_to_the_nearest_targets() {
    let s = busy_session();
    let all = s.traffic_targets();
    let snapshot = s.screen_snapshot();
    let cap = flightcraft_control::presentation::MAX_SNAPSHOT_TRAFFIC;
    assert_eq!(snapshot.traffic.len(), all.len().min(cap));
    // A hostile snapshot cannot grow a remote screen's list past the cap.
    let mut big = s.screen_snapshot();
    let template = big.traffic[0].clone();
    big.traffic = (0..100)
        .map(|i| TrafficTarget {
            address: i,
            ..template.clone()
        })
        .collect();
    let mut remote = atc_session("KABE");
    remote.apply_screen_snapshot(big).unwrap();
    assert_eq!(remote.traffic_targets().len(), cap);
}
