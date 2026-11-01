#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::{
    Session,
    bridge::{self, Lines, Listener},
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Cursor, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    thread,
    time::{Duration, Instant},
};

#[test]
fn parser_rejects_hostile_lines_and_preserves_valid_ids() {
    for line in [
        "",
        "garbage",
        "{}{}",
        "[]",
        "null",
        "{}",
        "{\"id\":1,\"command\":\"sim.state\"}",
        "{\"id\":{},\"command\":\"sim.state\",\"input\":{}}",
        "{\"id\":1,\"command\":\"sim.state\",\"input\":null}",
        "{\"id\":1,\"command\":\"sim.state\",\"input\":{},\"extra\":0}",
    ] {
        assert!(
            bridge::parse(line.as_bytes()).unwrap_err().get("error").is_some(),
            "{line}"
        );
    }
    assert!(bridge::parse(&[0xff]).is_err());
    assert!(bridge::parse(&vec![b' '; bridge::MAX_LINE_BYTES + 1]).is_err());
    let invalid = bridge::parse(br#"{"id":"keep","command":"sim.state","input":[]}"#).unwrap_err();
    assert_eq!(invalid["id"], "keep");
    for id in [json!(1), json!("client"), Value::Null] {
        let request = bridge::parse(
            json!({"id":id,"command":"sim.state","input":{}})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(request.id, id);
    }
    let nested = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    assert!(bridge::parse(nested.as_bytes()).is_err());
}

#[test]
fn bounded_lines_drain_garbage_and_accept_eof_and_crlf() {
    let mut input = Cursor::new(b"1234567890\nok\r\nlast");
    let mut lines = Lines::default();
    assert!(lines.read(&mut input, 8).unwrap().unwrap().is_err());
    assert_eq!(lines.read(&mut input, 8).unwrap().unwrap().unwrap(), b"ok\r\n");
    assert_eq!(lines.read(&mut input, 8).unwrap().unwrap().unwrap(), b"last");
    assert!(lines.read(&mut input, 8).unwrap().is_none());
}

#[test]
fn loopback_listener_moves_lines_to_executor_and_recovers_after_garbage() {
    assert!(Listener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into()).is_err());
    assert!(Listener::bind(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), 0).into()).is_err());
    let listener = Listener::loopback(0).unwrap();
    let port = listener.address().port();
    let executor = thread::spawn(move || {
        let mut session = Session::new().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut count = 0;
        while count < 5 && Instant::now() < deadline {
            if let Ok(incoming) = listener.try_recv() {
                count += 1;
                let request = incoming
                    .line
                    .map_err(|e| bridge::failure(Value::Null, e))
                    .and_then(|line| bridge::parse(&line));
                let response = match request {
                    Ok(r) => match session.execute(&r.command, r.input) {
                        Ok(output) => bridge::success(r.id, output),
                        Err(error) => bridge::failure(r.id, error.to_string()),
                    },
                    Err(error) => error,
                };
                incoming.reply.send(response).unwrap();
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(count, 5);
        assert_eq!(session.sim().controls().throttle, 0.75);
    });
    let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut reader = BufReader::new(socket.try_clone().unwrap());
    for data in [
        b"garbage\n".to_vec(),
        vec![b' '; bridge::MAX_LINE_BYTES + 1]
            .into_iter()
            .chain([b'\n'])
            .collect(),
        b"\xff\n".to_vec(),
        b"{\"id\":4,\"command\":\"sim.set_controls\",\"input\":{\"throttle\":0.75}}\n".to_vec(),
        b"{\"id\":5,\"command\":\"sim.state\",\"input\":{}}\n".to_vec(),
    ] {
        // Concurrent writing is not needed: the listener drains each entire line before replying.
        socket.write_all(&data).unwrap();
        let mut response = String::new();
        reader.read_line(&mut response).unwrap();
        let response: Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("error").is_some() || response.get("ok").is_some());
        if response["id"] == 5 {
            assert_eq!(response["ok"]["readout"]["controls"]["throttle"], 0.75);
        }
    }
    executor.join().unwrap();
}
