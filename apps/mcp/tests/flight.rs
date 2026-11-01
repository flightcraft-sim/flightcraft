//! Fly the takeoff/climb prefix through the same boundary used by an MCP client.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

#[path = "../../../crates/control/tests/support/mod.rs"]
mod support;

use flightcraft_control::{Session, mcp};
use serde_json::{Value, json};

#[test]
fn tools_call_takes_off_and_climbs_to_150_m() {
    let mut session = Session::new().unwrap();
    let initialize = json!({
        "jsonrpc": "2.0", "id": "initialize", "method": "initialize",
        "params": {
            "protocolVersion": mcp::PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "flight-test", "version": "1"},
        },
    });
    let initialized: Value =
        serde_json::from_str(&mcp::handle_line(&mut session, &initialize.to_string()).unwrap()).unwrap();
    assert!(initialized.get("error").is_none(), "{initialized}");
    assert_eq!(initialized["result"]["protocolVersion"], mcp::PROTOCOL_VERSION);
    assert!(
        mcp::handle_line(
            &mut session,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .is_none()
    );
    let mut request_id = 0;
    let report = support::fly(
        |name, arguments| {
            request_id += 1;
            let request = json!({
                "jsonrpc": "2.0", "id": request_id, "method": "tools/call",
                "params": {"name": name.replace('.', "_"), "arguments": arguments},
            });
            let response_line = mcp::handle_line(&mut session, &format!("{request}\n")).unwrap();
            assert_eq!(response_line.lines().count(), 1);
            let response: Value = serde_json::from_str(&response_line).unwrap();
            assert_eq!(response["jsonrpc"], "2.0");
            assert_eq!(response["id"], request_id);
            assert!(response.get("error").is_none(), "{response}");
            let result = &response["result"];
            assert_eq!(result["isError"], false, "{response}");
            assert_eq!(result["content"][0]["type"], "text");
            let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(text, result["structuredContent"]);
            result["structuredContent"].clone()
        },
        false,
    );
    assert_eq!(report.tool_calls, request_id);
    assert!(report.touchdown_m.is_none());
}
