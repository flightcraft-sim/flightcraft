#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpListener},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[test]
fn connect_forwards_calls_and_returns_screenshot_image_with_path() {
    let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = server.local_addr().unwrap().port();
    let fake = thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        for expected in ["sim.set_controls", "sim.state", "app.screenshot", "sim.step"] {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["command"], expected);
            let response = match expected {
                "sim.set_controls" => {
                    assert_eq!(request["input"], json!({"throttle":0.6}));
                    json!({"id":request["id"],"ok":{"throttle":0.6}})
                }
                "sim.state" => json!({"id":request["id"],"ok":{"state":{"time_s":123.0}}}),
                "app.screenshot" => {
                    assert_eq!(request["input"], json!({"directory":"captures","base64":true}));
                    json!({"id":request["id"],"ok":{"path":"captures/window.png","png_base64":"cG5n"}})
                }
                _ => {
                    json!({"id":request["id"],"error":"invalid input for sim.step: steps must be between 0 and 72000"})
                }
            };
            writeln!(stream, "{response}").unwrap();
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_flightcraft-mcp"))
        .args(["--connect", &port.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let calls = [
        ("tools/list", json!({})),
        (
            "tools/call",
            json!({"name":"sim_set_controls","arguments":{"throttle":0.6}}),
        ),
        ("tools/call", json!({"name":"sim_state"})),
        (
            "tools/call",
            json!({"name":"app_screenshot","arguments":{"directory":"captures","base64":false}}),
        ),
        (
            "tools/call",
            json!({"name":"sim_step","arguments":{"steps":72001}}),
        ),
    ];
    for (id, (method, params)) in calls.into_iter().enumerate() {
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 5);
    assert_eq!(
        responses[0]["result"]["tools"].as_array().unwrap().len(),
        flightcraft_control::Session::commands().len()
    );
    assert_eq!(
        responses[2]["result"]["structuredContent"]["state"]["time_s"],
        123.0
    );
    assert_eq!(
        responses[3]["result"]["structuredContent"]["path"],
        "captures/window.png"
    );
    assert_eq!(
        responses[3]["result"]["content"][1],
        json!({"type":"image","mimeType":"image/png","data":"cG5n"})
    );
    // The PNG travels once, in the image block, not again in the text or structured copy.
    assert_eq!(
        responses[3]["result"]["structuredContent"],
        json!({"path":"captures/window.png"})
    );
    assert!(
        !responses[3]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("cG5n")
    );
    assert_eq!(responses[4]["error"]["code"], -32602);
    fake.join().unwrap();
}
