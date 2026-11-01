//! Flight acceptance test driven exclusively through JSON commands.
#![allow(clippy::unwrap_used)]

mod support;

use flightcraft_control::Session;

#[test]
fn agent_flies_a_traffic_pattern_and_lands_on_the_runway() {
    let mut session = Session::new().unwrap();
    let report = support::fly(|name, input| session.execute(name, input).unwrap(), true);
    assert!(report.touchdown_m.is_some());
    assert!(report.tool_calls > 0);
}
