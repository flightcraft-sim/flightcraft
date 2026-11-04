use super::encode::*;

pub(super) fn truth() -> Aircraft {
    Aircraft {
        latitude_deg: 45.0,
        longitude_deg: -90.0,
        altitude_msl_m: 304.8,
        pressure_altitude_m: Some(304.8),
        track_deg: 90.0,
        ground_speed_mps: 51.44444444444444,
        vertical_speed_mps: -3.2512,
        heading_deg: 180.0,
        pitch_deg: 2.5,
        roll_deg: -10.0,
        airborne: true,
    }
}

// Independent polynomial long division: the FAA recurrence appends each input byte
// to the remainder's LOW byte after shifting. No production lookup table is used.
fn reference_crc(bytes: &[u8]) -> u16 {
    let mut remainder = 0_u32;
    for byte in bytes {
        remainder = (remainder << 8) | u32::from(*byte);
        for bit in (16..24).rev() {
            if remainder & (1 << bit) != 0 {
                remainder ^= 0x11021 << (bit - 16);
            }
        }
    }
    remainder as u16
}
#[test]
fn known_heartbeat_crc_and_stuffing() {
    // UTC 0: initialized, position valid, UTC valid; counts zero. Long division
    // gives remainder 0x9cbc, transmitted low byte first.
    let payload = [0, 0x81, 1, 0, 0, 0, 0];
    assert_eq!(heartbeat(0), payload);
    assert_eq!(reference_crc(&payload), 0x9cbc);
    assert_eq!(crc(&payload), 0x9cbc);
    assert_eq!(frame(&payload), [0x7e, 0, 0x81, 1, 0, 0, 0, 0, 0xbc, 0x9c, 0x7e]);
    // A second independently divided heartbeat vector has CRC 0x8bb3.
    let known = [0, 0x81, 0x41, 0xdb, 0xd0, 8, 2];
    assert_eq!(reference_crc(&known), 0x8bb3);
    assert_eq!(crc(&known), 0x8bb3);
    // Two bytes enter an initially empty remainder unchanged: CRC 0x7e7d.
    // Both data AND both CRC octets must be escaped.
    assert_eq!(
        frame(&[0x7e, 0x7d]),
        [0x7e, 0x7d, 0x5e, 0x7d, 0x5d, 0x7d, 0x5d, 0x7d, 0x5e, 0x7e]
    );
    assert_eq!(heartbeat(65536), [0, 0x81, 0x81, 0, 0, 0, 0]);
    assert_eq!(heartbeat(86400), heartbeat(0));
    for value in 0..=255 {
        let payload = [0, value, 0x65, 0x7d];
        assert_eq!(crc(&payload), reference_crc(&payload));
    }
}
#[test]
fn semicircles_and_altitude_offset() {
    // 45/180 * 2^23 = 2^21; -90/180 * 2^23 = -2^22.
    assert_eq!(semicircle(45.0), [0x20, 0, 0]);
    assert_eq!(semicircle(-90.0), [0xc0, 0, 0]);
    // A point at Blairstown (1N7), rounded independently:
    // 40.971145 * 2^23 / 180 = 1909393.748423... -> 0x1d2292;
    // -74.997475 * 2^23 / 180 = -3495135.659804... -> 0xcaab20.
    assert_eq!(semicircle(40.971145), [0x1d, 0x22, 0x92]);
    assert_eq!(semicircle(-74.997475), [0xca, 0xab, 0x20]);
    assert_eq!(pressure_altitude(Some(-304.8)), 0);
    assert_eq!(pressure_altitude(Some(0.0)), 40);
    assert_eq!(pressure_altitude(Some(304.8)), 80);
    assert_eq!(pressure_altitude(Some(3.81)), 41); // Half of a 25 ft step rounds up.
    assert_eq!(pressure_altitude(None), 0xfff);
    assert_eq!(pressure_altitude(Some(f64::NAN)), 0xfff);
    assert_eq!(pressure_altitude(Some(100000.0)), 0xfff);
}
#[test]
fn ownship_and_traffic_exact_layouts() {
    // Address type 1 (self-assigned), altitude (1000 + 1000)/25 = 80,
    // airborne + true track = 9, NIC/NACp 0, speed 100 kt = 0x064,
    // -640 ft/min / 64 = -10 = 0xff6; east track = 64; light emitter = 1.
    let expected = [
        10, 1, 0x12, 0x34, 0x56, 0x20, 0, 0, 0xc0, 0, 0, 5, 9, 0, 6, 0x4f, 0xf6, 64, 1, b'F', b'L', b'I',
        b'G', b'H', b'T', b'C', b'R', 0,
    ];
    assert_eq!(report(&truth(), true, 0x123456, "FLIGHTCR"), expected);
    let mut traffic = expected;
    traffic[0] = 20;
    assert_eq!(report(&truth(), false, 0x123456, "FLIGHTCR"), traffic);
    let mut invalid = truth();
    invalid.pressure_altitude_m = None;
    invalid.ground_speed_mps = f64::NAN;
    invalid.vertical_speed_mps = f64::NAN;
    invalid.airborne = false;
    let packet = report(&invalid, true, 0x123456, "ab");
    assert_eq!(&packet[11..17], &[0xff, 0xf1, 0, 0xff, 0xf8, 0]);
    assert_eq!(&packet[19..27], b"AB      ");
}
#[test]
fn geometric_and_extension_layouts() {
    // 1000 ft / 5 = 200 = 0x00c8; VFOM unknown.
    assert_eq!(geometric_altitude(304.8), [11, 0, 0xc8, 0x7f, 0xff]);
    assert_eq!(geometric_altitude(-304.8), [11, 0xff, 0x38, 0x7f, 0xff]);
    assert_eq!(geometric_altitude(f64::NAN), [11, 0x80, 0, 0x7f, 0xff]);
    let mut identity = vec![0x65, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8];
    identity.extend(b"FlightCrFlightCraft\0\0\0\0\0");
    identity.extend([0, 0, 0, 0]);
    assert_eq!(identification(0x0102030405060708), identity);
    assert_eq!(identity.len(), 39);
    // Roll -100 tenths = ff9c; pitch 25 = 0019; true heading 1800 = 8708.
    assert_eq!(
        ahrs(&truth()),
        [0x65, 1, 0xff, 0x9c, 0, 0x19, 0x87, 8, 0xff, 0xff, 0xff, 0xff]
    );
}
#[test]
fn simulator_text_units_order_and_termination() {
    assert_eq!(
        xgps(&truth()),
        "XGPSFlightCraft,-90.0000000,45.0000000,304.80,90.00,51.44\n"
    );
    assert_eq!(xatt(&truth()), "XATTFlightCraft,180.00,2.50,-10.00\n");
    assert_eq!(
        xtraffic(&truth(), 42),
        "XTRAFFICFlightCraft,42,45.0000000,-90.0000000,1000.0,-640.0,1,180.00,100.00\n"
    );
}
#[test]
fn traffic_report_id_20_exact_layout_with_alert_and_emitter() {
    // FAA 560-1058-00 Rev A, Traffic Report: byte 0 is ID 20 (0x14); byte 1 holds the traffic alert
    // status in its high nibble and the address type (1, ADS-B self-assigned) in its low nibble;
    // bytes 2-4 are the address; 5-7 latitude and 8-10 longitude in 24-bit semicircles; 11-12 and the
    // top nibble of byte 12 hold altitude in 25 ft steps from -1000 ft (2000 ft / 25 = 80 = 0x050);
    // the low nibble of byte 12 is the misc field (airborne, true track) = 9; byte 13 is NIC and
    // NACp; 14-16 pack a 12-bit horizontal velocity (100 kt = 0x064) and a 12-bit vertical
    // velocity in 64 fpm steps (-640 fpm = -10 = 0xff6); byte 17 is the track in 360/256 degree
    // steps (east = 64); byte 18 the emitter category; 19-26 the callsign; byte 27 priority/spare.
    let traffic = Traffic {
        aircraft: truth(),
        address: 0xf2_0005,
        callsign: "N427KP".into(),
        emitter: 1,
        alert: true,
    };
    let expected = [
        20, 0x11, 0xf2, 0, 5, 0x20, 0, 0, 0xc0, 0, 0, 5, 9, 0, 6, 0x4f, 0xf6, 64, 1, b'N', b'4', b'2', b'7',
        b'K', b'P', b' ', b' ', 0,
    ];
    assert_eq!(traffic_report(&traffic), expected);
    assert_eq!(traffic_report(&traffic).len(), 28);
    // No alert, a larger emitter, and on the ground: status nibble clear, misc 1, category 2.
    let mut ground = traffic.clone();
    ground.alert = false;
    ground.emitter = 2;
    ground.aircraft.airborne = false;
    let packet = traffic_report(&ground);
    assert_eq!((packet[1], packet[12], packet[18]), (0x01, 1, 2));
    // The plain `report` keeps the light-aircraft category and no alert.
    let plain = report(&truth(), false, 0xf2_0005, "N427KP");
    assert_eq!((plain[0], plain[1], plain[18]), (20, 1, 1));
}
#[test]
fn traffic_report_addresses_and_framing_are_independent_per_aircraft() {
    let make = |address| Traffic {
        aircraft: truth(),
        address,
        callsign: "TAM123".into(),
        emitter: 1,
        alert: false,
    };
    let a = traffic_report(&make(0xf2_0001));
    let b = traffic_report(&make(0xf2_0002));
    assert_eq!(&a[2..5], &[0xf2, 0, 1]);
    assert_eq!(&b[2..5], &[0xf2, 0, 2]);
    // Everything except the address is identical, and each frames with its own CRC.
    assert_eq!(a[5..], b[5..]);
    assert_ne!(frame(&a), frame(&b));
    // The framed message starts with the flag byte, ID 0x14, and ends with a flag.
    let framed = frame(&a);
    assert_eq!(
        (framed[0], framed[1], *framed.last().unwrap()),
        (0x7e, 0x14, 0x7e)
    );
}
#[test]
fn emitter_categories_follow_the_weight_classes() {
    assert_eq!(emitter_category(2550.), 1);
    assert_eq!(emitter_category(15_499.), 1);
    assert_eq!(emitter_category(15_500.), 2);
    assert_eq!(emitter_category(74_999.), 2);
    assert_eq!(emitter_category(75_000.), 3);
    assert_eq!(emitter_category(300_000.), 3);
    assert_eq!(emitter_category(300_001.), 5);
}
