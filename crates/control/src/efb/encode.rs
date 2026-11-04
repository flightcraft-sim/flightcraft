//! Original encoders for FAA GDL 90 Rev A and the published EFB extensions.
//! All multi-byte fields are big endian, except the GDL 90 frame CRC.

/// Aircraft truth in geographic coordinates and SI units. No instrument lag or failures.
#[derive(Clone, Copy, Debug)]
pub struct Aircraft {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub altitude_msl_m: f64,
    pub pressure_altitude_m: Option<f64>,
    pub track_deg: f64,
    pub ground_speed_mps: f64,
    pub vertical_speed_mps: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub roll_deg: f64,
    pub airborne: bool,
}

const fn crc_table_entry(mut value: u16) -> u16 {
    let mut bit = 0;
    while bit < 8 {
        value = if value & 0x8000 != 0 {
            (value << 1) ^ 0x1021
        } else {
            value << 1
        };
        bit += 1;
    }
    value
}

/// GDL 90's table recurrence (input enters the low byte), initial remainder zero.
pub fn crc(bytes: &[u8]) -> u16 {
    // Array::from_fn constructs the 256-entry CCITT table without unchecked indexing.
    let table: [u16; 256] = std::array::from_fn(|i| crc_table_entry((i as u16) << 8));
    bytes.iter().fold(0_u16, |value, byte| {
        table.get(usize::from(value >> 8)).copied().unwrap_or(0) ^ (value << 8) ^ u16::from(*byte)
    })
}

pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut result = vec![0x7e];
    for byte in payload.iter().copied().chain(crc(payload).to_le_bytes()) {
        if byte == 0x7e || byte == 0x7d {
            result.extend([0x7d, byte ^ 0x20]);
        } else {
            result.push(byte);
        }
    }
    result.push(0x7e);
    result
}

pub fn heartbeat(utc_seconds: u32) -> Vec<u8> {
    let seconds = utc_seconds % 86400;
    let [low, high, ..] = (seconds as u16).to_le_bytes();
    vec![0, 0x81, 0x01 | (((seconds >> 16) as u8) << 7), low, high, 0, 0]
}

/// Signed 24-bit semicircles, rounded to the nearest representable coordinate.
pub fn semicircle(degrees: f64) -> [u8; 3] {
    let value = (degrees * (8_388_608.0 / 180.0)).round() as i32;
    let [_, a, b, c] = value.to_be_bytes();
    [a, b, c]
}

pub fn pressure_altitude(metres: Option<f64>) -> u16 {
    metres.filter(|m| m.is_finite()).map_or(0xfff, |m| {
        let steps = ((m / 0.3048 + 1000.0) / 25.0).round();
        if (0.0..4095.0).contains(&steps) {
            steps as u16
        } else {
            0xfff
        }
    })
}

/// One other aircraft for a Traffic Report (ID 20, `0x14`).
#[derive(Clone, Debug)]
pub struct Traffic {
    pub aircraft: Aircraft,
    /// 24-bit self-assigned address; the fleet derives it from the aircraft id.
    pub address: u32,
    pub callsign: String,
    /// GDL 90 emitter category (table 11 of the specification), such as 1 for a light aircraft.
    pub emitter: u8,
    /// Traffic alert status bit: the aircraft is a TRAFFIC alert on the display.
    pub alert: bool,
}
/// Emitter category for a maximum takeoff weight in pounds: 1 light (under 15,500 lb), 2 small
/// (15,500 to 75,000 lb), 3 large (75,000 to 300,000 lb), 5 heavy (over 300,000 lb).
pub fn emitter_category(max_takeoff_lb: f64) -> u8 {
    if max_takeoff_lb < 15_500. {
        1
    } else if max_takeoff_lb < 75_000. {
        2
    } else if max_takeoff_lb <= 300_000. {
        3
    } else {
        5
    }
}
/// The Traffic Report for one aircraft.
pub fn traffic_report(t: &Traffic) -> Vec<u8> {
    report_with(&t.aircraft, false, t.address, &t.callsign, t.emitter, t.alert)
}
/// IDs 10 and 20 share the FAA's 28-byte report layout. Address is self-assigned,
/// NIC/NACp are unknown (zero); simulated truth carries no certified integrity claim.
pub fn report(aircraft: &Aircraft, ownship: bool, address: u32, callsign: &str) -> Vec<u8> {
    report_with(aircraft, ownship, address, callsign, 1, false)
}
fn report_with(
    aircraft: &Aircraft,
    ownship: bool,
    address: u32,
    callsign: &str,
    emitter: u8,
    alert: bool,
) -> Vec<u8> {
    // Byte 1: traffic alert status in the high nibble, address type 1 (ADS-B, self-assigned) below.
    let mut result = vec![if ownship { 10 } else { 20 }, 1 | (u8::from(alert) << 4)];
    let [_, a, b, c] = address.to_be_bytes();
    result.extend([a, b, c]);
    result.extend(semicircle(aircraft.latitude_deg));
    result.extend(semicircle(aircraft.longitude_deg));
    let altitude = pressure_altitude(aircraft.pressure_altitude_m);
    result.extend([
        (altitude >> 4) as u8,
        ((altitude & 15) as u8) << 4 | if aircraft.airborne { 9 } else { 1 },
        0,
    ]);
    let horizontal = if aircraft.ground_speed_mps.is_finite() && aircraft.ground_speed_mps >= 0.0 {
        (aircraft.ground_speed_mps / (1852.0 / 3600.0))
            .round()
            .clamp(0.0, 4094.0) as u16
    } else {
        0xfff
    };
    let vertical = if aircraft.vertical_speed_mps.is_finite() {
        let steps = (aircraft.vertical_speed_mps * 60.0 / 0.3048 / 64.0).round();
        steps.clamp(-511.0, 511.0) as i16 as u16 & 0xfff
    } else {
        0x800
    };
    result.extend([
        (horizontal >> 4) as u8,
        ((horizontal & 15) << 4) as u8 | (vertical >> 8) as u8,
        vertical as u8,
    ]);
    result.push((aircraft.track_deg.rem_euclid(360.0) * 256.0 / 360.0).round() as u16 as u8);
    result.push(emitter);
    result.extend(
        callsign
            .bytes()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    b' '
                }
            })
            .chain(std::iter::repeat(b' '))
            .take(8),
    );
    result.push(0); // No emergency.
    result
}

pub fn geometric_altitude(metres: f64) -> Vec<u8> {
    let altitude = if metres.is_finite() {
        (metres / 0.3048 / 5.0).round().clamp(-32767.0, 32767.0) as i16
    } else {
        i16::MIN
    };
    let [a, b] = altitude.to_be_bytes();
    vec![11, a, b, 0x7f, 0xff] // Vertical figure of merit unavailable.
}

/// Extension version 1: serial (8), short name (8), long name (16), capabilities (4).
pub fn identification(serial: u64) -> Vec<u8> {
    let mut result = vec![0x65, 0, 1];
    result.extend(serial.to_be_bytes());
    result.extend(*b"FlightCr");
    result.extend(*b"FlightCraft\0\0\0\0\0");
    result.extend([0, 0, 0, 0]); // Geometric altitude is MSL, not WGS84 ellipsoid height.
    result
}

pub fn ahrs(aircraft: &Aircraft) -> Vec<u8> {
    let mut result = vec![0x65, 1];
    for degrees in [aircraft.roll_deg, aircraft.pitch_deg] {
        let value = if degrees.is_finite() {
            (degrees * 10.0).round().clamp(-1800.0, 1800.0) as i16
        } else {
            i16::MAX
        };
        result.extend(value.to_be_bytes());
    }
    // Bit 15 marks true heading. Airspeeds are unavailable, rather than substituted ground speed.
    let heading = if aircraft.heading_deg.is_finite() {
        0x8000 | ((aircraft.heading_deg.rem_euclid(360.0) * 10.0).round() as u16 % 3600)
    } else {
        0xffff
    };
    result.extend(heading.to_be_bytes());
    result.extend([0xff, 0xff, 0xff, 0xff]);
    result
}

pub fn xgps(a: &Aircraft) -> String {
    format!(
        "XGPSFlightCraft,{:.7},{:.7},{:.2},{:.2},{:.2}\n",
        a.longitude_deg, a.latitude_deg, a.altitude_msl_m, a.track_deg, a.ground_speed_mps
    )
}
pub fn xatt(a: &Aircraft) -> String {
    format!(
        "XATTFlightCraft,{:.2},{:.2},{:.2}\n",
        a.heading_deg, a.pitch_deg, a.roll_deg
    )
}

/// Traffic's text protocol uses feet, knots, and feet/minute, unlike XGPS's SI fields.
pub fn xtraffic(a: &Aircraft, address: u32) -> String {
    format!(
        "XTRAFFICFlightCraft,{},{:.7},{:.7},{:.1},{:.1},{},{:.2},{:.2}\n",
        address & 0xffffff,
        a.latitude_deg,
        a.longitude_deg,
        a.altitude_msl_m / 0.3048,
        a.vertical_speed_mps * 60.0 / 0.3048,
        u8::from(a.airborne),
        a.heading_deg,
        a.ground_speed_mps * 3600.0 / 1852.0
    )
}
