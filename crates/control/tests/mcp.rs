#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use flightcraft_control::{Session, mcp};
use serde_json::{Value, json};

fn request(session: &mut Session, id: Value, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let response = mcp::handle_line(session, &line).unwrap();
    assert!(response.ends_with('\n'));
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], id);
    response
}

fn call(session: &mut Session, name: &str, arguments: Value) -> Value {
    let response = request(
        session,
        json!(name),
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    );
    assert!(response.get("error").is_none(), "{response}");
    let result = &response["result"];
    assert_eq!(result["isError"], false);
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"]);
    result["structuredContent"].clone()
}

#[test]
fn end_to_end_initialize_list_reset_controls_step_state() {
    let mut session = Session::new().unwrap();
    let initialized = request(
        &mut session,
        json!(1),
        "initialize",
        json!({"protocolVersion": mcp::PROTOCOL_VERSION, "capabilities": {}, "clientInfo": {"name": "test-client", "version": "1"}}),
    );
    assert_eq!(initialized["result"]["protocolVersion"], mcp::PROTOCOL_VERSION);
    assert_eq!(
        initialized["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert!(
        mcp::handle_line(
            &mut session,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .is_none()
    );
    let listed = request(&mut session, json!(2), "tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    let commands = Session::commands();
    assert_eq!(tools.len(), commands.len());
    for (tool, command) in tools.iter().zip(commands) {
        assert_eq!(tool["name"], command.name.replace('.', "_"));
        assert_eq!(tool["inputSchema"], command.input_schema);
        assert_eq!(tool["outputSchema"], command.output_schema);
    }
    call(&mut session, "sim_reset", json!({"start": {"kind": "runway"}}));
    let controls = call(&mut session, "sim_set_controls", json!({"throttle": 0.8}));
    assert_eq!(controls["throttle"], 0.8);
    let step = call(&mut session, "sim_step", json!({"steps": 240}));
    assert_eq!(step["time_s"], 2.0);
    let state = call(&mut session, "sim_state", json!({}));
    assert_eq!(state["state"]["time_s"], 2.0);
    assert_eq!(state["readout"]["controls"]["throttle"], 0.8);
    assert_eq!(
        request(&mut session, json!(null), "ping", json!({}))["result"],
        json!({})
    );
}

#[test]
fn protocol_errors_and_bad_tool_inputs() {
    let mut session = Session::new().unwrap();
    for (method, input, code) in [
        ("unknown", json!({}), -32601),
        ("ping", json!({"extra": 1}), -32602),
        ("ping", json!([]), -32602),
        ("ping", json!(null), -32602),
        ("initialize", json!({}), -32602),
        ("tools/list", json!({"cursor": "unknown"}), -32602),
        ("tools/call", json!({}), -32602),
        (
            "tools/call",
            json!({"name":"app_screenshot","arguments":{"base64":null}}),
            -32602,
        ),
        ("tools/call", json!({"name": "unknown"}), -32602),
        (
            "tools/call",
            json!({"name": "sim.step", "arguments": {"steps": 1}}),
            -32602,
        ),
        (
            "tools/call",
            json!({"name": "sim_step", "arguments": {"steps": 72001}}),
            -32602,
        ),
        (
            "tools/call",
            json!({"name": "sim_step", "arguments": {"steps": "NaN"}}),
            -32602,
        ),
        (
            "tools/call",
            json!({"name": "sim_step", "arguments": {"steps": 1, "extra": true}}),
            -32602,
        ),
        (
            "tools/call",
            json!({"name": "sim_state", "arguments": false}),
            -32602,
        ),
    ] {
        let response = request(&mut session, json!("id"), method, input);
        assert_eq!(response["error"]["code"], code, "{response}");
        assert!(response.get("result").is_none());
    }
    assert_eq!(session.sim().state().time_s, 0.0);
}

#[test]
fn garbage_lines_never_panic_and_notifications_are_silent() {
    let mut session = Session::new().unwrap();
    for (line, code) in [
        ("", -32700),
        ("garbage", -32700),
        ("{", -32700),
        ("{}{}", -32700),
        ("{\"value\":NaN}", -32700),
        ("{\"value\":1e999}", -32700),
        ("null", -32600),
        ("42", -32600),
        ("[]", -32600),
        ("{}", -32600),
        (r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#, -32600),
        (r#"{"jsonrpc":"2.0","id":{},"method":"ping"}"#, -32600),
        (r#"{"jsonrpc":"2.0","id":1,"method":null}"#, -32600),
    ] {
        let response: Value = serde_json::from_str(&mcp::handle_line(&mut session, line).unwrap()).unwrap();
        assert_eq!(response["error"]["code"], code, "{line}: {response}");
    }
    for method in ["ping", "unknown", "notifications/initialized", "tools/call"] {
        assert!(
            mcp::handle_line(
                &mut session,
                &json!({"jsonrpc": "2.0", "method": method}).to_string()
            )
            .is_none()
        );
    }
    // A deterministic corpus covers delimiters, escapes, Unicode, and nested parser input.
    for length in 1..256 {
        let line: String = (0..length)
            .map(|i| char::from(((i * 31 + length * 17) % 127) as u8))
            .collect();
        if let Some(response) = mcp::handle_line(&mut session, &line) {
            assert!(serde_json::from_str::<Value>(&response).is_ok());
        }
    }
    let nested = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    assert!(
        mcp::handle_line(&mut session, &nested)
            .unwrap()
            .contains("-32700")
    );
    assert!(
        mcp::handle_line(&mut session, &" ".repeat(mcp::MAX_LINE_BYTES + 1))
            .unwrap()
            .contains("-32700")
    );
    assert_eq!(
        request(&mut session, json!(99), "ping", json!({}))["result"],
        json!({})
    );
}
