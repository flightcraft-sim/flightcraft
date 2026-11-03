#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::source::{Extracted, MAX_BODY};
use super::*;
use std::collections::BTreeMap;
const HTML: &[u8] = include_bytes!("../../tests/fixtures/local-weather.html");
fn local() -> WeatherSource {
    WeatherSource::preset("1N7", "http://127.0.0.1/weather", Extraction::Digiwx)
}
fn at() -> i64 {
    time::Date::from_calendar_date(2026, time::Month::November, 3)
        .unwrap()
        .with_hms(14, 15, 0)
        .unwrap()
        .assume_utc()
        .unix_timestamp()
}
#[test]
fn forty_eight_original_real_format_metars() {
    // Hand-authored format examples, not downloaded weather records. First is the requested 1N7 report.
    let cases = [
        (
            "METAR 1N7 031348Z AUTO 27000 10 SM OVC100 08/08 A3031",
            0.,
            10.,
            8.,
        ),
        ("METAR KZZZ 031400Z 27010KT 10SM CLR 15/05 A3000", 10., 10., 15.),
        (
            "METAR KZZZ 031401Z AUTO 00000KT 10SM SKC 14/06 A3001",
            0.,
            10.,
            14.,
        ),
        (
            "SPECI KZZZ 031402Z COR 18012G21KT 5SM -RA BKN020 12/10 A2998",
            12.,
            5.,
            12.,
        ),
        (
            "METAR KZZZ 031403Z VRB03KT 6SM HZ SCT040 18/09 A2995",
            3.,
            6.,
            18.,
        ),
        (
            "METAR KZZZ 031404Z 23014KT 190V260 8SM BKN035 17/12 A2999",
            14.,
            8.,
            17.,
        ),
        (
            "METAR KZZZ 031405Z 01008KT 1/2SM FG VV002 06/06 A3010",
            8.,
            0.5,
            6.,
        ),
        (
            "METAR KZZZ 031406Z 34009KT 1 1/2SM -SN OVC008 M02/M04 A3020",
            9.,
            1.5,
            -2.,
        ),
        (
            "METAR KZZZ 031407Z 36011KT 3/4SM BR BKN004 03/02 A2980",
            11.,
            0.75,
            3.,
        ),
        (
            "METAR KZZZ 031408Z 09006KT 2 1/4SM RA OVC010 11/10 A2975",
            6.,
            2.25,
            11.,
        ),
        ("METAR KZZZ 031409Z 12005KT P6SM SCT030 20/11 A3005", 5., 6., 20.),
        (
            "METAR KZZZ 031410Z 21004KT M1/4SM FG VV001 02/02 A3011",
            4.,
            0.25,
            2.,
        ),
        (
            "METAR KZZZ 031411Z 30015G26KT 4SM -SN BKN015 M08/M12 A3025",
            15.,
            4.,
            -8.,
        ),
        (
            "METAR KZZZ 031412Z 05007KT 10 SM FEW050 21/13 A3009",
            7.,
            10.,
            21.,
        ),
        (
            "METAR KZZZ 031413Z 17009KT 2SM BR OVC006 09/08 A2990 RMK AO2",
            9.,
            2.,
            9.,
        ),
        (
            "METAR KZZZ 031414Z 25012KT 7SM SCT022 BKN060 OVC100 19/14 A2989",
            12.,
            7.,
            19.,
        ),
        ("METAR KZZZ 031415Z 08004KT 9SM SKC 10/M01 A3030", 4., 9., 10.),
        ("METAR KZZZ 031416Z 02003KT 6SM HZ NSC 23/12 Q1015", 3., 6., 23.),
        (
            "METAR KZZZ 031417Z 29010KT 10SM CLR M10/M15 Q1023",
            10.,
            10.,
            -10.,
        ),
        (
            "METAR KZZZ 031418Z 14008KT 5SM RA BKN025CB 16/15 Q1002",
            8.,
            5.,
            16.,
        ),
        (
            "METAR KZZZ 031419Z 16011KT 3SM +RA OVC012 18/17 A2965",
            11.,
            3.,
            18.,
        ),
        ("METAR KZZZ 031420Z VRB02KT 1SM BR OVC003 04/04 A3018", 2., 1., 4.),
        (
            "METAR KZZZ 031421Z 24018G32KT 8SM SCT030TCU 25/13 A2978",
            18.,
            8.,
            25.,
        ),
        (
            "METAR KZZZ 031422Z 33016KT 2 3/4SM SN OVC018 M04/M06 A3008",
            16.,
            2.75,
            -4.,
        ),
        ("METAR KZZZ 031423Z 28006KT 10SM NCD 07/00 Q1030", 6., 10., 7.),
        (
            "METAR KZZZ 031424Z 11005KT 4SM BR BKN005 OVC020 13/12 A2991",
            5.,
            4.,
            13.,
        ),
        (
            "METAR KZZZ 031425Z 20013KT 6SM -RA SCT012 OVC045 22/20 A2983",
            13.,
            6.,
            22.,
        ),
        (
            "METAR KZZZ 031426Z 31017G28KT 10SM FEW070 M01/M09 A3035",
            17.,
            10.,
            -1.,
        ),
        (
            "METAR KZZZ 031427Z 06003KT 1/4SM FG VV/// 01/01 A3022",
            3.,
            0.25,
            1.,
        ),
        (
            "METAR KZZZ 031428Z 22008KT 2 1/2SM -SN BKN009 M03/M04 A2997",
            8.,
            2.5,
            -3.,
        ),
        (
            "METAR KZZZ 031429Z AUTO 19007KT 10SM CLR 26/16 A2996 RMK T02610156",
            7.,
            10.,
            26.,
        ),
        (
            "METAR KZZZ 031430Z COR 35010KT 5SM HZ SCT090 24/11 Q1019",
            10.,
            5.,
            24.,
        ),
        (
            "METAR KZZZ 031431Z 13009KT 3 1/2SM BR OVC007 08/07 A3014",
            9.,
            3.5,
            8.,
        ),
        (
            "METAR KZZZ 031432Z 26012KT 4 3/4SM RA BKN019 15/14 A2993",
            12.,
            4.75,
            15.,
        ),
        ("METAR KZZZ 031433Z 03004KT 10SM SKC M00/M02 A3040", 4., 10., 0.),
        (
            "METAR KZZZ 031434Z 10006KT 7SM FEW015 SCT080 27/19 A2985",
            6.,
            7.,
            27.,
        ),
        (
            "METAR KZZZ 031435Z 15008KT 6SM -DZ OVC011 14/13 A2992",
            8.,
            6.,
            14.,
        ),
        ("METAR KZZZ 031436Z 27011KT 9SM BKN200 16/02 A3006", 11., 9., 16.),
        (
            "METAR KZZZ 031437Z 32014G22KT 1 3/4SM +SN VV006 M06/M07 A3016",
            14.,
            1.75,
            -6.,
        ),
        ("METAR KZZZ 031438Z 04002KT 8SM SCT100 05/M03 Q1021", 2., 8., 5.),
        (
            "METAR KZZZ 031439Z 18010KT 3SM TSRA BKN025CB 21/19 A2981",
            10.,
            3.,
            21.,
        ),
        (
            "METAR KZZZ 031440Z VRB01KT 5/8SM FG OVC002 00/M01 A3029",
            1.,
            0.625,
            0.,
        ),
        ("METAR KZZZ 031441Z 20005KT 10SM CLR 29/20 A2979", 5., 10., 29.),
        (
            "METAR KZZZ 031442Z 28009KT 3/8SM FZFG VV001 M05/M05 Q1025",
            9.,
            0.375,
            -5.,
        ),
        (
            "METAR KZZZ 031443Z 09003KT 7/8SM BR BKN003 07/07 A3019",
            3.,
            0.875,
            7.,
        ),
        (
            "METAR KZZZ 031444Z 25006KT 6 1/2SM HZ SCT050 30/18 A2994",
            6.,
            6.5,
            30.,
        ),
        (
            "METAR KZZZ 031445Z 34012KT 10SM CLR M12/M19 Q1033",
            12.,
            10.,
            -12.,
        ),
        (
            "METAR KZZZ 031446Z 16007KT 5SM -SHRA SCT018 BKN040 23/21 A2988=",
            7.,
            5.,
            23.,
        ),
    ];
    assert!(cases.len() >= 40);
    for (text, wind, vis, temp) in cases {
        let o = metar::parse(text);
        assert_eq!(o.wind.as_ref().map(|w| w.speed_kt), Some(wind), "{text}");
        assert_eq!(o.visibility_sm, Some(vis), "{text}");
        assert_eq!(o.temperature_c, Some(temp), "{text}");
        assert!(o.altimeter_inhg.is_some(), "{text}");
    }
}
#[test]
fn optional_groups_and_metric_reports() {
    let o = metar::parse(
        "METAR EZZZ 312350Z COR AUTO 24008G15MPS 210V280 4500 -SN BR BKN012 M03/M05 Q1008 RMK 18099KT A9999",
    );
    assert!(o.corrected.unwrap());
    assert!(o.automatic.unwrap());
    let w = o.wind.unwrap();
    assert!((w.speed_kt - 15.55075592).abs() < 1e-6);
    assert_eq!(w.variable_sector_deg, Some([210., 280.]));
    assert!((o.visibility_sm.unwrap() - 4500. / 1609.344).abs() < 1e-10);
    assert!((o.altimeter_inhg.unwrap() - 1008. / 33.86389).abs() < 1e-10);
    assert_eq!(o.present_weather.unwrap(), vec!["-SN", "BR"]);
    assert_eq!(
        metar::parse("METAR EZZZ 010010Z VRB02KT CAVOK 12/04 Q1020").layers,
        Some(Vec::new())
    );
    let partial = metar::parse("METAR KZZZ 031400Z /////KT //// // /// ////");
    assert!(partial.wind.is_none());
    assert!(partial.temperature_c.is_none());
    assert!(partial.visibility_sm.is_none());
}
#[test]
fn digiwx_and_all_selectors() {
    let got = local().extract(HTML).unwrap();
    assert_eq!(got.observation.temperature_c, Some(12.));
    assert_eq!(got.observation.runway_in_use.as_deref(), Some("07"));
    assert_eq!(got.matches["wind"], "180° @ 7 KT");
    assert_eq!(got.observation.layers.as_ref().unwrap().len(), 2);
    let body = std::str::from_utf8(HTML).unwrap();
    for selector in [
        Selector::ElementId {
            element_id: "MetarLine".into(),
        },
        Selector::CssLike {
            css_like: "#MetarLine.current".into(),
        },
        Selector::CssLike {
            css_like: ".metar.current".into(),
        },
        Selector::Regex {
            regex: r#"id="MetarLine">([^<]+)"#.into(),
        },
    ] {
        assert_eq!(selector.extract(body).unwrap(), got.matches.get("metar").cloned());
    }
    let mut source = local();
    source.extraction = Extraction::Metar {
        selector: Selector::JsonPointer {
            json_pointer: "/station/report".into(),
        },
        fields: Fields::default(),
    };
    assert_eq!(
        source
            .extract(include_bytes!("../../tests/fixtures/local-weather.json"))
            .unwrap()
            .observation
            .temperature_c,
        Some(12.)
    );
    source.extraction = Extraction::MetarText;
    assert!(
        source
            .extract(b"METAR 1N7 031415Z 18007KT 7SM CLR 12/06 A3007")
            .unwrap()
            .observation
            .layers
            .unwrap()
            .is_empty()
    );
    source.extraction = Extraction::Aviationweather;
    assert!(
        source
            .extract(br#"[{"rawOb":"METAR 1N7 031415Z 18007KT 7SM CLR 12/06 A3007"}]"#)
            .is_ok()
    );
}
#[test]
fn per_field_units_and_metar_supplement() {
    let mut source = local();
    source.extraction = Extraction::Fields {
        fields: Fields::digiwx(),
    };
    let o = source.extract(HTML).unwrap().observation;
    assert_eq!(o.wind.unwrap().gust_kt, Some(14.));
    assert_eq!(o.altimeter_inhg, Some(30.07));
    assert_eq!(o.layers.unwrap().len(), 2);
    assert_eq!(o.dewpoint_c, Some(6.));
    let id = |s: &str| Some(Selector::ElementId { element_id: s.into() });
    source.extraction = Extraction::Fields {
        fields: Fields {
            wind: id("w"),
            temperature: id("t"),
            visibility: id("v"),
            altimeter: id("a"),
            layers: id("c"),
            ..Default::default()
        },
    };
    let o = source.extract(b"<span id='w'>VRB @ 3 M/S</span><span id='t'>50 F</span><span id='v'>8000 M</span><span id='a'>1013.25 hPa</span><span id='c'>CLR</span>").unwrap().observation;
    assert_eq!(o.temperature_c, Some(10.));
    assert!((o.visibility_sm.unwrap() - 8000. / 1609.344).abs() < 1e-9);
    assert!((o.altimeter_inhg.unwrap() - 29.9212524).abs() < 1e-6);
    assert!(o.layers.unwrap().is_empty());
}
#[test]
fn hostile_inputs_are_bounded_and_missing_is_unknown() {
    let source = local();
    assert!(source.extract(&vec![b'x'; MAX_BODY + 1]).is_err());
    assert!(source.extract(&[0, 255, 0]).is_err());
    assert!(
        source
            .extract(b"<span id='MetarLine'>nothing here</span>")
            .is_err()
    );
    assert!(source.extract(b"<html>no fields</html>").is_err());
    for pattern in [
        "(a)(b)",
        "no capture",
        "([)",
        "(a{1000000000})",
        &format!("({})", "x".repeat(2048)),
        r"((?=a)b)",
    ] {
        assert!(
            Selector::Regex {
                regex: pattern.into()
            }
            .validate()
            .is_err(),
            "{pattern}"
        );
    }
    for css in ["span", "#x > .y", ".a[attr]", "script()"] {
        assert!(Selector::CssLike { css_like: css.into() }.validate().is_err());
    }
    let mut bad = source;
    bad.allow_http = false;
    assert!(bad.validate().is_err());
    bad = local();
    bad.refresh_seconds = 59;
    assert!(bad.validate().is_err());
    bad = local();
    bad.disclaimer.clear();
    assert!(bad.validate().is_err());
    bad = local();
    bad.url = "file:///tmp/weather".into();
    assert!(bad.validate().is_err());
    bad = local();
    bad.url = "https://name:password@example.invalid".into();
    assert!(bad.validate().is_err());
    bad = local();
    bad.airport = "KZZZ".into();
    assert!(bad.extract(HTML).is_err());
}
#[test]
fn metar_garbage_fuzz_smoke_never_panics() {
    let mut state = 981725u64;
    for size in (0..512).chain([16384, 16385, 1024 * 1024]) {
        let bytes = (0..size)
            .map(|_| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                (state >> 32) as u8
            })
            .collect::<Vec<_>>();
        let text = String::from_utf8_lossy(&bytes);
        let _ = metar::parse(&text);
    }
    for text in [
        "💨27012KT",
        "METAR 💨 031400Z 🌤🌤🌤",
        "\0\0\0",
        "RMK A3000",
        "",
        "METAR KZZZ 031400Z 270//KT BKN💨 M💨/💨",
    ] {
        let _ = metar::parse(text);
    }
}
#[test]
fn observation_age_uses_station_time_and_month_rollover() {
    let o = LiveObservation::from_body(local(), HTML, at()).unwrap();
    assert_eq!(o.observed_at, Some(at()));
    assert!(!o.stale(at() + 5400));
    assert!(o.stale(at() + 5401));
    let mut source = local();
    source.extraction = Extraction::MetarText;
    let jan = time::Date::from_calendar_date(2027, time::Month::January, 1)
        .unwrap()
        .with_hms(0, 10, 0)
        .unwrap()
        .assume_utc()
        .unix_timestamp();
    let o = LiveObservation::from_body(
        source.clone(),
        b"METAR 1N7 312350Z 18007KT 7SM CLR 12/06 A3007",
        jan,
    )
    .unwrap();
    assert_eq!(o.age_seconds(jan), 20 * 60);
    let o = LiveObservation::from_body(
        source.clone(),
        b"METAR 1N7 301200Z 18007KT 7SM CLR 12/06 A3007",
        jan,
    )
    .unwrap();
    assert!(o.stale(jan));
    assert!(
        LiveObservation::from_body(source, b"METAR 1N7 019999Z 18007KT 7SM CLR 12/06 A3007", jan).is_err()
    );
}
#[test]
fn source_schema_and_example_match() {
    let schema = schemars::schema_for!(WeatherSource);
    let schema = serde_json::to_value(schema).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let example = include_str!("../../../../examples/weather-sources/1n7-digiwx.json");
    let value = serde_json::from_str(example).unwrap();
    assert!(validator.is_valid(&value));
    let source = WeatherSource::from_json(example).unwrap();
    assert_eq!(source, local_example());
    let mut value = value;
    value["refresh_seconds"] = 59.into();
    assert!(!validator.is_valid(&value));
}
fn local_example() -> WeatherSource {
    serde_json::from_str(include_str!(
        "../../../../examples/weather-sources/1n7-digiwx.json"
    ))
    .unwrap()
}
#[test]
fn live_phraseology_keeps_unknowns_and_converts_true_wind() {
    let o = LiveObservation::from_body(local(), HTML, at()).unwrap();
    let line = crate::comms::phraseology::live_weather(&o, "Fixture", Some(-12.), at());
    assert!(
        line.contains("Wind one niner zero at seven, gusts one four knots"),
        "{line}"
    );
    assert!(line.contains("Altimeter three zero zero seven"));
    assert!(line.contains(&o.source.disclaimer));
    assert!(line.contains("Indicated runway zero seven"));
    let mut partial = o;
    partial.extracted.observation.present_weather = Some(vec!["-SN".into()]);
    assert!(
        crate::comms::phraseology::live_weather(&partial, "Fixture", Some(-12.), at())
            .contains("Light snow.")
    );
    partial.extracted = Extracted {
        observation: metar::parse("METAR 1N7 031415Z A3007"),
        matches: BTreeMap::new(),
    };
    let line = crate::comms::phraseology::live_weather(&partial, "Fixture", None, at() + 6000);
    assert!(line.contains("Wind unavailable"));
    assert!(!line.contains("Temperature"));
    assert!(line.contains("stale"));
}
#[test]
fn command_schemas_are_registered_and_programmatic_calls_do_not_open_dialogs() {
    let mut session = Session::new().unwrap();
    for name in [
        "weather.sources",
        "weather.source_test",
        "weather.live",
        "weather.source_save",
        "weather.phone",
    ] {
        assert!(Session::commands().iter().any(|c| c.name == name));
    }
    let sources = session
        .run(WeatherSources {
            airport: Some("1N7".into()),
        })
        .unwrap();
    assert!(sources.sources.iter().all(|s| !s.fetching));
    assert!(
        session
            .run(WeatherLive::UseNow {
                source_id: "1n7-digiwx".into()
            })
            .is_err()
    );
    assert!(session.run(WeatherSourceSave { source: local() }).is_err());
    assert!(session.run(WeatherLive::Stop).is_ok());
}
#[test]
fn nearby_reports_keep_identity_and_cross_check_thresholds() {
    let at = at();
    let stations = nearby::parse_reports(
        include_bytes!("../../tests/fixtures/nearby-metar.json"),
        [40.97, -75.0],
        "1N7",
        at,
    )
    .unwrap();
    assert_eq!(stations.len(), 4);
    for ident in ["KFWN", "K12N", "KMPO", "KSMQ"] {
        let s = stations.iter().find(|s| s.station == ident).unwrap();
        assert_eq!(s.observation.source.airport, ident);
        assert!(s.distance_nm > 0.);
        assert!(s.label().contains("nearest reporting station"));
    }
    let mut source = local();
    source.extraction = Extraction::MetarText;
    let mut local = LiveObservation::from_body(
        source,
        b"METAR 1N7 031348Z AUTO 27000 10 SM OVC100 08/08 A3031",
        at,
    )
    .unwrap();
    let check = nearby::compare(&local, &stations, at);
    assert!((check.official_altimeter_mean_inhg.unwrap() - 30.305).abs() < 1e-9);
    assert!(!check.altimeter_disagreement);
    assert!(!check.time_disagreement);
    local.extracted.observation.altimeter_inhg = Some(30.36);
    assert!(nearby::compare(&local, &stations, at).altimeter_disagreement);
    local.extracted.observation.altimeter_inhg = Some(30.355);
    assert!(!nearby::compare(&local, &stations, at).altimeter_disagreement);
    local.observed_at = Some(
        stations
            .iter()
            .filter_map(|s| s.observation.observed_at)
            .max()
            .unwrap()
            - 3600,
    );
    assert!(!nearby::compare(&local, &stations, at).time_disagreement);
    local.observed_at = local.observed_at.map(|t| t - 1);
    assert!(nearby::compare(&local, &stations, at).time_disagreement);
    assert!(nearby::parse_reports(b"[]", [40.97, -75.0], "1N7", at).is_err());
    assert!(
        nearby::discovery_source("1N7", [40.97, -75.0])
            .unwrap()
            .url
            .contains("/metar?bbox=")
    );
}

use crate as flightcraft_control;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/radio_fixture.rs"
));
#[test]
fn follow_blends_at_fixed_ticks_and_replay_matches_without_network() {
    let mut live = radio_session();
    let mut replay = radio_session();
    let observation = LiveObservation::from_body(local(), HTML, at()).unwrap();
    let initial = live.sim().conditions().sea_level_pressure_pa;
    live.accept_live(observation, true).unwrap();
    let record = live.flight_record().live_weather[0].clone();
    assert_eq!(live.sim().conditions().sea_level_pressure_pa, initial);
    replay
        .run(WeatherLive::Replay {
            record: Box::new(record.clone()),
        })
        .unwrap();
    live.run(crate::Step { steps: 30 * 120 }).unwrap();
    replay.run(crate::Step { steps: 30 * 120 }).unwrap();
    let middle = live.sim().conditions().sea_level_pressure_pa;
    assert!((middle - (initial + record.target_conditions.sea_level_pressure_pa) / 2.).abs() < 1e-5);
    assert_eq!(live.sim().state_hash(), replay.sim().state_hash());
    live.run(crate::Step { steps: 30 * 120 + 1 }).unwrap();
    replay.run(crate::Step { steps: 30 * 120 + 1 }).unwrap();
    assert!(
        (live.sim().conditions().sea_level_pressure_pa - record.target_conditions.sea_level_pressure_pa)
            .abs()
            < 1e-8
    );
    assert_eq!(live.sim().conditions(), &record.target_conditions);
    assert_eq!(live.sim().state_hash(), replay.sim().state_hash());
    let json = serde_json::to_string(live.flight_record()).unwrap();
    let imported = crate::FlightRecord::from_json(&json).unwrap();
    assert_eq!(imported.live_weather.len(), 1);
    assert_eq!(imported.live_weather[0], record);
}
#[test]
fn stale_is_never_applied_and_awos_uses_live_station_values() {
    let mut s = radio_session();
    let mut observation = LiveObservation::from_body(local(), HTML, at()).unwrap();
    observation.observed_at = Some(runtime::now() - 5401);
    let source = observation.source.clone();
    let mut status = SourceStatus::new(source.clone());
    status.finish(Ok(observation.clone()), runtime::now());
    s.live_weather.entries.insert(source.id.clone(), status);
    let hash = s.sim().state_hash();
    assert!(
        s.run(WeatherLive::UseNow {
            source_id: source.id.clone()
        })
        .is_err()
    );
    assert_eq!(hash, s.sim().state_hash());
    let station = radio_station(flightcraft_world::FacilityKind::Awos, 123000);
    let (line, meaning) = s.station_weather_report(&station);
    assert!(
        line.starts_with("Blairstown automated weather observation."),
        "{line}"
    );
    assert!(line.contains("Altimeter three zero zero seven"));
    assert!(line.contains("stale"));
    assert!(matches!(meaning, crate::Meaning::LiveWeather { .. }));
    s.execute("comms.tune", serde_json::json!({"radio":1,"channel_khz":123000}))
        .unwrap();
    s.run(crate::Step { steps: 120 }).unwrap();
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| matches!(e.meaning, crate::Meaning::LiveWeather { .. })
                && e.text.contains(&source.disclaimer))
    );
    let phone = s
        .run(WeatherPhone {
            airport: "1N7".into(),
        })
        .unwrap();
    assert_eq!(phone.text, line);
}

#[test]
fn phone_without_awos_uses_faa_name_and_rounds_after_magnetic_conversion() {
    let mut s = radio_session();
    // 1N7 has no published AWOS/ASOS; exercise the real phone-only path.
    s.run(crate::CommsFacilities {
        stations: Some(vec![]),
    })
    .unwrap();
    let mut source = local();
    source.extraction = Extraction::MetarText;
    let observation = LiveObservation::from_body(
        source,
        b"METAR 1N7 031636Z AUTO 21005 10 SM SCT110 BKN210 18/09 A3026",
        at() + 2 * 3600 + 21 * 60,
    )
    .unwrap();
    let mut status = SourceStatus::new(observation.source.clone());
    status.finish(Ok(observation), runtime::now());
    s.live_weather.entries.insert(status.source.id.clone(), status);
    let phone = s
        .run(WeatherPhone {
            airport: "1N7".into(),
        })
        .unwrap();
    assert!(
        phone
            .text
            .starts_with("Blairstown automated weather observation."),
        "{}",
        phone.text
    );
    assert!(
        phone.text.contains("Wind two two zero at five knots."),
        "{}",
        phone.text
    );
    assert_eq!(
        phone
            .observation
            .unwrap()
            .extracted
            .observation
            .wind
            .unwrap()
            .from_true_deg,
        Some(210.)
    );
    let mut station = radio_station(flightcraft_world::FacilityKind::Awos, 123000);
    station.name = "1N7 AWOS".into();
    assert_eq!(s.station_weather_report(&station).0, phone.text);
    station.name = "BLAIRSTOWN AWOS-3".into();
    assert_eq!(
        crate::comms::phraseology::weather_station_name(&station),
        "Blairstown"
    );
}

#[test]
fn applied_station_units_and_reported_visibility_survive_precipitation() {
    let mut s = radio_session();
    let mut source = local();
    source.extraction = Extraction::MetarText;
    let observation = LiveObservation::from_body(
        source,
        b"METAR 1N7 031415Z 27010KT 5SM -RA OVC020 12/06 A3031",
        at(),
    )
    .unwrap();
    s.accept_live(observation, false).unwrap();
    let elevation = s.sim().world().airports().unwrap().airports[0].elevation_m;
    let c = s.sim().conditions();
    assert!((c.effective_visibility_m() / 1609.344 - 5.).abs() < 1e-9);
    assert!((c.layers[0].base_ft_msl - (2000. + elevation / 0.3048)).abs() < 1e-9);
    assert!((c.temperature_c - 0.0065 * elevation - 12.).abs() < 1e-9);
    assert!((c.sea_level_pressure_pa / 3386.389 - 30.31).abs() < 1e-9);
    let reference = s.sim().heading_reference();
    assert!(
        (s.pilot_weather().wind_from_deg - reference.true_to_magnetic(270f64.to_radians()).to_degrees())
            .abs()
            < 1e-9
    );
    assert!((s.pilot_weather().wind_speed_mps / 0.514444 - 10.).abs() < 1e-9);
}

#[test]
fn spaced_fraction_and_multiple_reports_do_not_mix_stations() {
    assert_eq!(
        metar::parse("METAR 1N7 031400Z 27010KT 1 1/2 SM OVC010 12/10 A3000").visibility_sm,
        Some(1.5)
    );
    let o = metar::parse(
        "METAR 1N7 031400Z 27010KT 5SM OVC010 12/10 A3000\nMETAR KZZZ 031400Z 09020KT 1SM OVC002 M01/M02 A2990",
    );
    assert_eq!(o.station.as_deref(), Some("1N7"));
    assert_eq!(o.altimeter_inhg, Some(30.));
    assert_eq!(o.wind.unwrap().speed_kt, 10.);
}

#[test]
fn html_quoted_angle_brackets_and_numeric_ceiling_units() {
    let selector = Selector::ElementId {
        element_id: "report".into(),
    };
    assert_eq!(
        selector
            .extract("<span title='value > threshold' id='report'>30.31</span>")
            .unwrap()
            .as_deref(),
        Some("30.31")
    );
    let mut source = local();
    source.extraction = Extraction::Fields {
        fields: Fields {
            ceiling: Some(Selector::Body),
            ..Default::default()
        },
    };
    let layer = source
        .extract(b"Broken 300 meters")
        .unwrap()
        .observation
        .layers
        .unwrap()
        .remove(0);
    assert_eq!(layer.coverage, "BKN");
    assert!((layer.base_agl_ft.unwrap() - 300. / 0.3048).abs() < 1e-9);
}

#[test]
fn field_units_ignore_case_and_multiline_disclaimer_records_round_trip() {
    let mut source = local();
    let selector = |id: &str| {
        Some(Selector::ElementId {
            element_id: id.into(),
        })
    };
    source.extraction = Extraction::Fields {
        fields: Fields {
            wind: selector("w"),
            gust: selector("g"),
            temperature: selector("t"),
            ceiling: selector("c"),
            ..Default::default()
        },
    };
    let o = source
        .extract(
            b"<b id='w'>VRB @ 3 m/s</b><b id='g'>7 m/s</b><b id='t'>M02 C</b><b id='c'>Overcast 300m</b>",
        )
        .unwrap()
        .observation;
    assert!((o.wind.unwrap().gust_kt.unwrap() - 7. * 1.94384449).abs() < 1e-9);
    assert_eq!(o.temperature_c, Some(-2.));
    assert!((o.layers.unwrap()[0].base_agl_ft.unwrap() - 300. / 0.3048).abs() < 1e-9);
    let mut s = radio_session();
    let mut live = LiveObservation::from_body(local(), HTML, at()).unwrap();
    live.source.disclaimer = format!(
        "Advisory only.\n{} Not for flight planning.",
        "Long advisory text. ".repeat(45)
    );
    let mut status = SourceStatus::new(live.source.clone());
    status.finish(Ok(live), runtime::now());
    s.live_weather.entries.insert(status.source.id.clone(), status);
    s.execute("comms.tune", serde_json::json!({"radio":1,"channel_khz":123000}))
        .unwrap();
    s.run(crate::Step { steps: 120 }).unwrap();
    let event = s
        .flight_record()
        .events
        .iter()
        .find(|e| matches!(e.meaning, crate::Meaning::LiveWeather { .. }))
        .unwrap();
    assert!(event.text.len() > 1024);
    assert!(!event.text.contains('\n'));
    assert!(crate::FlightRecord::from_json(&serde_json::to_string(s.flight_record()).unwrap()).is_ok());
}

#[test]
fn stopping_mid_blend_records_an_exact_freeze_for_replay() {
    let mut live = radio_session();
    let mut replay = radio_session();
    let observation = LiveObservation::from_body(local(), HTML, at()).unwrap();
    live.accept_live(observation, true).unwrap();
    replay
        .run(WeatherLive::Replay {
            record: Box::new(live.flight_record().live_weather[0].clone()),
        })
        .unwrap();
    live.run(crate::Step { steps: 17 * 120 }).unwrap();
    replay.run(crate::Step { steps: 17 * 120 }).unwrap();
    assert_eq!(live.sim().state_hash(), replay.sim().state_hash());
    let stopped_pressure = live.sim().conditions().sea_level_pressure_pa;
    live.run(WeatherLive::Stop).unwrap();
    assert_eq!(live.flight_record().live_weather.len(), 2);
    let freeze = live.flight_record().live_weather[1].clone();
    assert_eq!(freeze.duration_s, 0.);
    replay
        .run(WeatherLive::Replay {
            record: Box::new(freeze),
        })
        .unwrap();
    live.run(crate::Step { steps: 60 * 120 }).unwrap();
    replay.run(crate::Step { steps: 60 * 120 }).unwrap();
    assert_eq!(live.sim().conditions().sea_level_pressure_pa, stopped_pressure);
    assert_eq!(live.sim().state_hash(), replay.sim().state_hash());
}

#[test]
fn entity_floods_and_large_selected_elements_are_bounded() {
    let selector = Selector::ElementId {
        element_id: "report".into(),
    };
    let body = format!("<span id='report'>{}</span>", "&".repeat(16384));
    assert_eq!(selector.extract(&body).unwrap().unwrap().len(), 16384);
    let body = format!("<span id='report'>{}</span>", "&".repeat(16385));
    assert!(selector.extract(&body).is_err());
}

#[test]
fn worker_seed_restores_saved_sources_without_promoting_untested_drafts() {
    let mut s = radio_session();
    s.run(WeatherSources::default()).unwrap();
    let source = local();
    s.live_weather.insert(source.clone()).unwrap();
    assert!(!s.worker_seed().weather_sources.iter().any(|v| v.id == source.id));
    s.live_weather
        .saved_sources
        .insert(source.id.clone(), source.clone());
    let seed = s.worker_seed();
    assert!(seed.weather_sources.contains(&source));
    let mut restored = Session::from_worker_seed(seed).unwrap();
    assert_eq!(restored.live_weather.saved_sources.get(&source.id), Some(&source));
    let sources = restored
        .run(WeatherSources {
            airport: Some("1N7".into()),
        })
        .unwrap();
    let status = sources.sources.iter().find(|v| v.source.id == source.id).unwrap();
    assert!(status.last_good.is_none());
    assert!(!status.fetching);
    let snapshot = restored.screen_snapshot();
    snapshot.live_weather.validate().unwrap();
    assert!(snapshot.live_weather.saved_sources.contains(&source));
}

#[test]
fn nearby_reports_choose_newest_station_report_before_comparing() {
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_slice(include_bytes!("../../tests/fixtures/nearby-metar.json")).unwrap();
    let mut old = rows.iter().find(|r| r["icaoId"] == "KFWN").unwrap().clone();
    old["rawOb"] = "METAR KFWN 031253Z AUTO 26004KT 10SM OVC095 09/07 A2990".into();
    rows.insert(0, old);
    let stations =
        nearby::parse_reports(&serde_json::to_vec(&rows).unwrap(), [40.97, -75.0], "1N7", at()).unwrap();
    assert_eq!(stations.len(), 4);
    assert_eq!(
        stations
            .iter()
            .find(|s| s.station == "KFWN")
            .unwrap()
            .observation
            .extracted
            .observation
            .altimeter_inhg,
        Some(30.29)
    );
}

#[test]
fn a_page_clock_cannot_refresh_an_old_metar_timestamp() {
    let html = std::str::from_utf8(HTML).unwrap().replace("031415Z", "031215Z");
    let live = LiveObservation::from_body(local(), html.as_bytes(), at()).unwrap();
    assert_eq!(live.observed_at, Some(at() - 2 * 3600));
    assert!(live.stale(at()));
    let december = time::Date::from_calendar_date(2026, time::Month::December, 3)
        .unwrap()
        .with_hms(14, 15, 0)
        .unwrap()
        .assume_utc()
        .unix_timestamp();
    let live = LiveObservation::from_body(local(), HTML, december).unwrap();
    assert_eq!(live.observed_at, Some(at()));
    assert!(live.stale(december));
}
