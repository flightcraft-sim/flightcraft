//! A flight script: timed commands the app runs through the same command path as MCP.
//!
//! The format is JSON lines, one command per line, in simulated seconds:
//!
//! ```text
//! {"at_s": 0.0, "command": "sim.set_controls", "input": {"throttle": 1.0}}
//! {"at_s": 9.0, "command": "sim.set_controls", "input": {"elevator": 0.35}}
//! ```
//!
//! Blank lines and lines starting with `#` are skipped. While a script has commands left, the
//! keyboard and gamepad do not move the flight controls.

use flightcraft_control::Session;
use serde::Deserialize;

/// The most commands one script may hold.
pub const MAX_LINES: usize = 10_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    at_s: f64,
    command: String,
    #[serde(default)]
    input: serde_json::Value,
}

#[derive(Debug, Default)]
pub struct Script {
    lines: Vec<Line>,
    next: usize,
}

impl Script {
    /// Parse a script; an error names the line.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            if lines.len() >= MAX_LINES {
                return Err(format!("a script holds at most {MAX_LINES} commands"));
            }
            let line: Line = serde_json::from_str(raw).map_err(|e| format!("line {}: {e}", n + 1))?;
            if !line.at_s.is_finite() || line.at_s < 0.0 {
                return Err(format!("line {}: at_s must be a finite time of 0 or more", n + 1));
            }
            if line.input.is_null() {
                lines.push(Line {
                    input: serde_json::json!({}),
                    ..line
                });
            } else {
                lines.push(line);
            }
        }
        // Stable: commands at the same time keep their order in the file.
        lines.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
        Ok(Self { lines, next: 0 })
    }

    /// True while commands remain.
    #[must_use]
    pub fn active(&self) -> bool {
        self.next < self.lines.len()
    }

    /// Run every command due at or before `time_s`. Stops at the first error and returns it.
    pub fn run_due(&mut self, session: &mut Session, time_s: f64) -> Result<(), String> {
        while let Some(line) = self.lines.get(self.next) {
            if line.at_s > time_s {
                break;
            }
            self.next += 1;
            session
                .execute(&line.command, line.input.clone())
                .map_err(|e| format!("script at {:.2} s: {e}", line.at_s))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_commands_in_time_order() {
        let mut script = Script::parse(
            "# takeoff\n{\"at_s\": 2.0, \"command\": \"sim.set_controls\", \"input\": {\"elevator\": 0.3}}\n\n{\"at_s\": 0.0, \"command\": \"sim.set_controls\", \"input\": {\"throttle\": 1.0}}\n",
        )
        .unwrap();
        let mut session = Session::new().unwrap();
        script.run_due(&mut session, 0.0).unwrap();
        assert_eq!(session.sim().controls().throttle, 1.0);
        assert_eq!(session.sim().controls().elevator, 0.0);
        assert!(script.active());
        script.run_due(&mut session, 2.0).unwrap();
        assert_eq!(session.sim().controls().elevator, 0.3);
        assert!(!script.active());
    }

    #[test]
    fn hostile_scripts_are_errors() {
        for text in [
            "not json",
            "{\"at_s\": -1, \"command\": \"sim.step\", \"input\": {\"steps\": 1}}",
            "{\"at_s\": 0, \"command\": \"sim.step\", \"extra\": 1}",
            "{\"at_s\": 1e999, \"command\": \"sim.step\"}",
        ] {
            assert!(Script::parse(text).is_err(), "{text}");
        }
        let mut script = Script::parse("{\"at_s\": 0, \"command\": \"no.such\"}").unwrap();
        let mut session = Session::new().unwrap();
        assert!(script.run_due(&mut session, 1.0).is_err());
    }
}
