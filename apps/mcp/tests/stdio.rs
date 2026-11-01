#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn stdio_is_protocol_only_and_recovers_after_bad_lines() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_flightcraft-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Drain stdout concurrently to avoid pipe backpressure while sending the oversized line.
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut stdout = stdout;
        let mut result = String::new();
        stdout.read_to_string(&mut result).unwrap();
        result
    });
    let mut input = child.stdin.take().unwrap();
    input.write_all(b"garbage\n\xff\n").unwrap();
    input.write_all(&vec![b' '; 1024 * 1024 + 1]).unwrap();
    input.write_all(b"\n").unwrap();
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .unwrap();
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
        .unwrap();
    // EOF also terminates a valid final request without a newline.
    input
        .write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"sim_state\"}}",
        )
        .unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = reader.join().unwrap();
    let responses: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 5);
    for response in responses.iter().take(3) {
        assert_eq!(response["error"]["code"], -32700);
    }
    assert_eq!(responses[3]["id"], 1);
    assert_eq!(responses[3]["result"], serde_json::json!({}));
    assert_eq!(responses[4]["id"], 2);
    assert_eq!(
        responses[4]["result"]["structuredContent"]["state"]["time_s"],
        0.0
    );
}
