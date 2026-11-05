//! Pilot aids on the glass display that are pure arithmetic on simulation time: the clock, the
//! flight timer, and the choice of what the data bar's fourth field shows. Nothing here reads a
//! wall clock; replays and saved flights give the same readings.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Time of day the flight starts at (UTC), seconds, unless the pilot sets the clock. It matches
/// the renderer's default illumination hour so the sky and the clock agree out of the box.
pub const DEFAULT_START_UTC_S: f64 = 17.0 * 3600.0;
const DAY_S: f64 = 86_400.0;
/// The longest a timer may be set to count down from.
pub const MAX_PRESET_S: f64 = 99.0 * 3600.0 + 59.0 * 60.0 + 59.0;

/// Which time the clock field shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClockFormat {
    #[default]
    Utc,
    /// Local time at the aircraft: UTC plus the longitude's nautical time zone, one hour per 15
    /// degrees (rounded), as a unit without a time-zone database shows it.
    Local,
}

/// What the data bar's user field (the slot of the minimum safe altitude on the guide's data
/// bar) shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataBarField {
    /// The minimum safe altitude; the engine models none and shows dashes.
    #[default]
    SafeAltitude,
    /// The clock, in the chosen format.
    Clock,
    /// The flight timer.
    Timer,
}

impl DataBarField {
    /// The next field in the touch cycle.
    pub fn next(self) -> Self {
        match self {
            Self::SafeAltitude => Self::Clock,
            Self::Clock => Self::Timer,
            Self::Timer => Self::SafeAltitude,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimerDirection {
    #[default]
    Up,
    Down,
}

/// What the pilot did to the flight timer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimerAction {
    Start {},
    Stop {},
    /// Start when stopped, stop when running (the touch field).
    Toggle {},
    /// Stop and return to zero (up) or to the preset (down).
    Reset {},
    /// Count up from zero or down from the preset. Stops and resets the timer.
    Direction {
        direction: TimerDirection,
    },
    /// The count-down start value. Stops and resets the timer.
    Preset {
        #[schemars(range(min = 1, max = 359999))]
        seconds: f64,
    },
}

/// The flight timer: counts up from zero, or down from a preset to zero. It runs on simulation
/// time, so it stops with the simulation when paused.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlightTimer {
    pub direction: TimerDirection,
    /// The count-down start value, seconds.
    pub preset_s: f64,
    pub running: bool,
    /// Seconds counted before the current run began.
    pub accumulated_s: f64,
    /// Simulation time the current run began at.
    pub started_at_s: f64,
}

impl Default for FlightTimer {
    fn default() -> Self {
        Self {
            direction: TimerDirection::Up,
            preset_s: 300.0,
            running: false,
            accumulated_s: 0.0,
            started_at_s: 0.0,
        }
    }
}

impl FlightTimer {
    /// Seconds counted so far at simulation time `now_s`.
    pub fn elapsed_s(&self, now_s: f64) -> f64 {
        let running = if self.running {
            (now_s - self.started_at_s).max(0.0)
        } else {
            0.0
        };
        self.accumulated_s + running
    }

    /// The value shown: the elapsed time counting up, or the time left counting down (never
    /// below zero).
    pub fn display_s(&self, now_s: f64) -> f64 {
        match self.direction {
            TimerDirection::Up => self.elapsed_s(now_s),
            TimerDirection::Down => (self.preset_s - self.elapsed_s(now_s)).max(0.0),
        }
    }

    /// A count-down that has reached zero.
    pub fn expired(&self, now_s: f64) -> bool {
        self.direction == TimerDirection::Down && self.elapsed_s(now_s) >= self.preset_s
    }

    /// Apply an action at simulation time `now_s`. An invalid preset is refused and changes
    /// nothing.
    pub fn apply(&mut self, action: TimerAction, now_s: f64) -> Result<(), &'static str> {
        match action {
            TimerAction::Start {} => {
                if !self.running && !self.expired(now_s) {
                    self.running = true;
                    self.started_at_s = now_s;
                }
            }
            TimerAction::Stop {} => {
                if self.running {
                    self.accumulated_s = self.elapsed_s(now_s);
                    self.running = false;
                }
            }
            TimerAction::Toggle {} => {
                let next = if self.running {
                    TimerAction::Stop {}
                } else {
                    TimerAction::Start {}
                };
                return self.apply(next, now_s);
            }
            TimerAction::Reset {} => {
                self.running = false;
                self.accumulated_s = 0.0;
            }
            TimerAction::Direction { direction } => {
                self.direction = direction;
                return self.apply(TimerAction::Reset {}, now_s);
            }
            TimerAction::Preset { seconds } => {
                if !seconds.is_finite() || !(1.0..=MAX_PRESET_S).contains(&seconds) {
                    return Err("timer preset must be 1 second to 99:59:59");
                }
                self.preset_s = seconds;
                return self.apply(TimerAction::Reset {}, now_s);
            }
        }
        Ok(())
    }
}

/// Time of day, seconds from midnight UTC, `elapsed_s` after a flight that started at
/// `start_utc_s`.
pub fn utc_seconds(start_utc_s: f64, elapsed_s: f64) -> f64 {
    (start_utc_s + elapsed_s).rem_euclid(DAY_S)
}

/// Whole hours of the nautical time zone at a longitude (east positive).
pub fn local_offset_hours(longitude_deg: f64) -> i32 {
    if !longitude_deg.is_finite() {
        return 0;
    }
    (longitude_deg / 15.0).round().clamp(-12.0, 12.0) as i32
}

/// The clock reading in seconds from midnight for the format.
pub fn clock_seconds(format: ClockFormat, utc_s: f64, longitude_deg: f64) -> f64 {
    match format {
        ClockFormat::Utc => utc_s,
        ClockFormat::Local => {
            (utc_s + f64::from(local_offset_hours(longitude_deg)) * 3600.0).rem_euclid(DAY_S)
        }
    }
}

/// `HH:MM:SS`, from seconds from midnight.
pub fn hms(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!(
        "{:02}:{:02}:{:02}",
        (total / 3600) % 100,
        total / 60 % 60,
        total % 60
    )
}

/// The timer's text: `MM:SS` under an hour and `H:MM:SS` from an hour on.
pub fn timer_text(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    if total >= 3600 {
        format!("{}:{:02}:{:02}", total / 3600, total / 60 % 60, total % 60)
    } else {
        format!("{:02}:{:02}", total / 60, total % 60)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn counting_up_runs_stops_resumes_and_resets() {
        let mut t = FlightTimer::default();
        assert_eq!(t.display_s(100.0), 0.0, "stopped at zero until started");
        t.apply(TimerAction::Start {}, 100.0).unwrap();
        assert_eq!(t.display_s(160.0), 60.0);
        t.apply(TimerAction::Stop {}, 160.0).unwrap();
        assert_eq!(t.display_s(500.0), 60.0, "a stopped timer holds");
        t.apply(TimerAction::Start {}, 500.0).unwrap();
        assert_eq!(t.display_s(530.0), 90.0, "it resumes from where it stopped");
        t.apply(TimerAction::Reset {}, 530.0).unwrap();
        assert!(!t.running);
        assert_eq!(t.display_s(900.0), 0.0);
    }

    #[test]
    fn a_second_start_does_not_move_the_start_time() {
        let mut t = FlightTimer::default();
        t.apply(TimerAction::Start {}, 10.0).unwrap();
        t.apply(TimerAction::Start {}, 50.0).unwrap();
        assert_eq!(t.display_s(70.0), 60.0);
        t.apply(TimerAction::Toggle {}, 70.0).unwrap();
        assert!(!t.running);
        t.apply(TimerAction::Toggle {}, 90.0).unwrap();
        assert!(t.running);
        assert_eq!(t.display_s(100.0), 70.0);
    }

    #[test]
    fn counting_down_stops_at_zero_and_reports_expiry() {
        let mut t = FlightTimer::default();
        t.apply(TimerAction::Preset { seconds: 120.0 }, 0.0).unwrap();
        t.apply(
            TimerAction::Direction {
                direction: TimerDirection::Down,
            },
            0.0,
        )
        .unwrap();
        assert_eq!(t.display_s(5.0), 120.0, "shows the preset before it starts");
        t.apply(TimerAction::Start {}, 5.0).unwrap();
        assert_eq!(t.display_s(65.0), 60.0);
        assert!(!t.expired(65.0));
        assert_eq!(t.display_s(125.0), 0.0);
        assert!(t.expired(125.0));
        assert_eq!(t.display_s(900.0), 0.0, "never negative");
        t.apply(TimerAction::Reset {}, 900.0).unwrap();
        assert_eq!(t.display_s(900.0), 120.0, "reset returns to the preset");
        assert!(!t.expired(900.0));
    }

    #[test]
    fn presets_are_validated_and_changing_direction_resets() {
        let mut t = FlightTimer::default();
        for bad in [0.0, -5.0, f64::NAN, f64::INFINITY, 400_000.0] {
            assert!(
                t.apply(TimerAction::Preset { seconds: bad }, 0.0).is_err(),
                "{bad}"
            );
        }
        assert_eq!(t.preset_s, 300.0);
        t.apply(TimerAction::Start {}, 0.0).unwrap();
        t.apply(
            TimerAction::Direction {
                direction: TimerDirection::Down,
            },
            40.0,
        )
        .unwrap();
        assert!(!t.running);
        assert_eq!(t.display_s(40.0), 300.0);
    }

    #[test]
    fn the_clock_wraps_at_midnight_and_local_time_follows_longitude() {
        assert_eq!(utc_seconds(DEFAULT_START_UTC_S, 0.0), 17.0 * 3600.0);
        assert_eq!(utc_seconds(23.0 * 3600.0, 2.0 * 3600.0), 3600.0);
        assert_eq!(local_offset_hours(-73.9), -5);
        assert_eq!(local_offset_hours(0.0), 0);
        assert_eq!(local_offset_hours(139.7), 9);
        assert_eq!(local_offset_hours(f64::NAN), 0);
        // 17:00 UTC over New York is 12:00; 03:00 UTC there wraps to 22:00 the day before.
        assert_eq!(
            clock_seconds(ClockFormat::Local, 17.0 * 3600.0, -73.9),
            12.0 * 3600.0
        );
        assert_eq!(
            clock_seconds(ClockFormat::Local, 3.0 * 3600.0, -73.9),
            22.0 * 3600.0
        );
        assert_eq!(clock_seconds(ClockFormat::Utc, 3.0 * 3600.0, -73.9), 3.0 * 3600.0);
    }

    #[test]
    fn times_are_written_as_the_data_bar_shows_them() {
        assert_eq!(hms(0.0), "00:00:00");
        assert_eq!(hms(17.0 * 3600.0 + 5.0 * 60.0 + 9.0), "17:05:09");
        assert_eq!(timer_text(59.9), "00:59");
        assert_eq!(timer_text(754.0), "12:34");
        assert_eq!(timer_text(3725.0), "1:02:05");
    }
}
