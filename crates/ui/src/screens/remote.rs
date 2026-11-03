//! Opt-in, same-origin LAN panel hosting. Pairing never grants command access by itself.
use crate::runtime::Source;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tungstenite::{
    Message,
    handshake::server::{ErrorResponse, Request, Response},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ReadOnly,
    Control,
}
#[derive(Clone, Debug, Serialize)]
pub struct Client {
    pub id: u64,
    pub name: String,
    pub permission: Permission,
}
struct Peer {
    client: Client,
    stop: Arc<AtomicBool>,
}
struct Shared {
    peers: Mutex<Vec<Peer>>,
    stop: AtomicBool,
    next: AtomicU64,
    active: AtomicU64,
}
pub struct Server {
    address: SocketAddr,
    token: String,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}
fn lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback(),
        IpAddr::V6(v) => v.is_loopback() || v.is_unique_local(),
    }
}
impl Server {
    pub fn start(address: SocketAddr, root: PathBuf, source: Source) -> Result<Self, String> {
        if !lan(address.ip()) {
            return Err("panel host requires an explicit private LAN or loopback address".into());
        }
        let root = root.canonicalize().map_err(|e| format!("web directory: {e}"))?;
        if !root.join("flightcraft_web.js").is_file() || !root.join("index.html").is_file() {
            return Err("web directory needs the packaged browser build (cargo xtask web)".into());
        }
        let listener = TcpListener::bind(address).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let shared = Arc::new(Shared {
            peers: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
            next: AtomicU64::new(1),
            active: AtomicU64::new(0),
        });
        let state = shared.clone();
        let secret = token.clone();
        let origin = format!("http://{address}");
        let thread = std::thread::Builder::new()
            .name("panel-host".into())
            .spawn(move || {
                while !state.stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            if !lan(peer.ip()) || state.active.load(Ordering::Relaxed) >= 16 {
                                continue;
                            }
                            state.active.fetch_add(1, Ordering::Relaxed);
                            let state = state.clone();
                            let root = root.clone();
                            let source = source.clone();
                            let origin = origin.clone();
                            let secret = secret.clone();
                            let worker = state.clone();
                            if std::thread::Builder::new()
                                .name("panel-screen".into())
                                .spawn(move || {
                                    let _ = serve(stream, &root, &source, &origin, &secret, &worker);
                                    worker.active.fetch_sub(1, Ordering::Relaxed);
                                })
                                .is_err()
                            {
                                state.active.fetch_sub(1, Ordering::Relaxed);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            address,
            token,
            shared,
            thread: Some(thread),
        })
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn token(&self) -> &str {
        &self.token
    }
    pub fn clients(&self) -> Vec<Client> {
        self.shared
            .peers
            .lock()
            .map(|p| p.iter().map(|p| p.client.clone()).collect())
            .unwrap_or_default()
    }
    pub fn disconnect(&self, id: u64) {
        if let Ok(peers) = self.shared.peers.lock() {
            for p in peers.iter().filter(|p| p.client.id == id) {
                p.stop.store(true, Ordering::Relaxed);
            }
        }
    }
    pub fn grant(&self, id: u64, permission: Permission) {
        if let Ok(mut peers) = self.shared.peers.lock()
            && let Some(p) = peers.iter_mut().find(|p| p.client.id == id)
        {
            p.client.permission = permission;
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Ok(peers) = self.shared.peers.lock() {
            for p in peers.iter() {
                p.stop.store(true, Ordering::Relaxed);
            }
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    token: String,
    name: String,
}
fn pair(bytes: &str, expected: &str) -> Result<String, String> {
    if bytes.len() > 4096 {
        return Err("pairing message too large".into());
    }
    let hello: Hello = serde_json::from_str(bytes).map_err(|e| e.to_string())?;
    let mismatch = hello.token.len() != expected.len()
        || hello
            .token
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |d, (a, b)| d | (a ^ b))
            != 0;
    if mismatch {
        return Err("invalid pairing token".into());
    }
    if hello.name.is_empty() || hello.name.len() > 80 || hello.name.chars().any(char::is_control) {
        return Err("invalid screen name".into());
    }
    Ok(hello.name)
}
fn origin_ok(origin: Option<&str>, expected: &str) -> bool {
    origin == Some(expected)
}
pub fn command_allowed(permission: Permission, name: &str) -> bool {
    if Permission::ReadOnly == permission {
        return false;
    }
    matches!(
        name,
        "atc.configure"
            | "atc.light_gun"
            | "sim.pause"
            | "sim.resume"
            | "sim.reset"
            | "sim.set_controls"
            | "sim.set_rate"
            | "systems.set"
            | "equipment.set"
            | "avionics.set"
            | "weather.set"
            | "failure.set"
            | "failure.clear"
            | "scenario.start"
            | "scenario.amend"
            | "scenario.signal"
            | "scenario.instructor_failure"
            | "comms.atc"
            | "comms.tune"
            | "comms.audio_panel"
            | "comms.transmit"
            | "emergency.record"
            | "map.set"
            | "assist.set"
            | "assist.off"
            | "autopilot.set"
            | "autopilot.off"
            | "layout.update"
            | "layout.move"
            | "layout.add"
            | "layout.remove"
            | "layout.undo"
            | "layout.redo"
            | "layout.set"
            | "screen.set_page"
            | "view.hood"
    )
}
fn serve(
    mut stream: TcpStream,
    root: &Path,
    source: &Source,
    origin: &str,
    token: &str,
    shared: &Shared,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    // peek keeps the complete upgrade request available to tungstenite's bounded parser.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut header = [0u8; 8192];
    let header = loop {
        let n = stream.peek(&mut header).map_err(|e| e.to_string())?;
        let bytes = header.get(..n).ok_or("invalid HTTP header")?;
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())?;
        }
        if n == header.len() || Instant::now() > deadline {
            return Err("HTTP header exceeds limits".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let request_path = header
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .ok_or("invalid HTTP request")?;
    if request_path != "/screens" {
        // Consume the peeked request before closing the HTTP response socket.
        // Unread request bytes can otherwise turn a successful asset transfer into a TCP reset.
        let mut consumed = vec![0; header.len()];
        stream.read_exact(&mut consumed).map_err(|e| e.to_string())?;
        return static_file(&mut stream, root, request_path);
    }
    let expected = origin.to_owned();
    let mut ws = tungstenite::accept_hdr_with_config(
        stream,
        move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            if request.uri().path() != "/screens"
                || !origin_ok(
                    request.headers().get("Origin").and_then(|v| v.to_str().ok()),
                    &expected,
                )
            {
                let mut e = ErrorResponse::new(Some("Origin rejected".into()));
                *e.status_mut() = tungstenite::http::StatusCode::FORBIDDEN;
                Err(e)
            } else {
                Ok(response)
            }
        },
        Some(
            tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(64 * 1024))
                .max_frame_size(Some(64 * 1024))
                .max_write_buffer_size(256 * 1024 * 1024),
        ),
    )
    .map_err(|e| e.to_string())?;
    let hello = ws.read().map_err(|e| e.to_string())?;
    let name = pair(hello.to_text().map_err(|e| e.to_string())?, token)?;
    let id = shared.next.fetch_add(1, Ordering::Relaxed);
    let stop = Arc::new(AtomicBool::new(false));
    shared
        .peers
        .lock()
        .map_err(|_| "panel list unavailable")?
        .push(Peer {
            client: Client {
                id,
                name,
                permission: Permission::ReadOnly,
            },
            stop: stop.clone(),
        });
    let result = (|| {
        ws.send(Message::text(json!({"type":"paired","id":id,"permission":"read_only","regions":source.snapshot()?.screen_regions()}).to_string())).map_err(|e|e.to_string())?;
        ws.get_mut()
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| e.to_string())?;
        let mut known_world = source.snapshot()?.sim().world().choice();
        let mut period = Instant::now();
        let mut messages = 0u32;
        while !stop.load(Ordering::Relaxed) && !shared.stop.load(Ordering::Relaxed) {
            let message = match ws.read() {
                Ok(v) => v,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(e.to_string()),
            };
            if period.elapsed() >= Duration::from_secs(1) {
                period = Instant::now();
                messages = 0;
            }
            messages += 1;
            if messages > 120 {
                return Err("panel message rate exceeded".into());
            }
            if message.is_close() {
                break;
            }
            if !message.is_text() {
                continue;
            }
            let value: Value = serde_json::from_str(message.to_text().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let permission = shared
                .peers
                .lock()
                .ok()
                .and_then(|p| p.iter().find(|p| p.client.id == id).map(|p| p.client.permission))
                .unwrap_or(Permission::ReadOnly);
            let output = if value.get("type").and_then(Value::as_str) == Some("snapshot") {
                let session = source.snapshot()?;
                let world = session.sim().world().choice();
                let mut output =
                    json!({"type":"snapshot","permission":permission,"snapshot":session.screen_snapshot()});
                if world != known_world {
                    known_world = world;
                    if let Some(object) = output.as_object_mut() {
                        object.insert(
                            "regions".into(),
                            serde_json::to_value(session.screen_regions()).map_err(|e| e.to_string())?,
                        );
                    }
                }
                output
            } else {
                let request = flightcraft_control::bridge::parse(
                    message.to_text().map_err(|e| e.to_string())?.as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                if !command_allowed(permission, &request.command) {
                    flightcraft_control::bridge::failure(
                        request.id,
                        "screen has no permission for this command",
                    )
                } else {
                    let mut session = source.snapshot()?;
                    match session.execute(&request.command, request.input) {
                        Ok(v) => flightcraft_control::bridge::success(request.id, v),
                        Err(e) => flightcraft_control::bridge::failure(request.id, e.to_string()),
                    }
                }
            };
            ws.send(Message::text(output.to_string()))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Ok(mut peers) = shared.peers.lock() {
        peers.retain(|p| p.client.id != id);
    }
    let _ = ws.close(None);
    result
}
fn static_file(stream: &mut TcpStream, root: &Path, path: &str) -> Result<(), String> {
    let path = path.split('?').next().unwrap_or("");
    let relative = if path == "/" {
        "index.html"
    } else {
        path.strip_prefix('/').ok_or("invalid URL")?
    };
    if Path::new(relative)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("invalid path".into());
    }
    let file = root.join(relative).canonicalize().map_err(|e| e.to_string())?;
    if !file.starts_with(root) || !file.is_file() {
        return Err("path outside web build".into());
    }
    let size = std::fs::metadata(&file).map_err(|e| e.to_string())?.len();
    if size > 128 * 1024 * 1024 {
        return Err("static asset too large".into());
    }
    let mime = match file.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    };
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {size}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\n\r\n").map_err(|e|e.to_string())?;
    let mut file = std::fs::File::open(file).map_err(|e| e.to_string())?;
    std::io::copy(&mut file, stream).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn pairing_and_origin_are_exact() {
        assert!(pair(r#"{"token":"bad","name":"tablet"}"#, "good").is_err());
        assert!(pair(r#"{"token":"good","name":"tablet","control":true}"#, "good").is_err());
        assert_eq!(
            pair(r#"{"token":"good","name":"tablet"}"#, "good").unwrap(),
            "tablet"
        );
        assert!(!origin_ok(None, "http://192.168.1.2:7891"));
        assert!(!origin_ok(
            Some("https://evil.example"),
            "http://192.168.1.2:7891"
        ));
        assert!(origin_ok(
            Some("http://192.168.1.2:7891"),
            "http://192.168.1.2:7891"
        ));
    }
    #[test]
    fn permission_is_host_granted_and_commands_are_bounded() {
        for c in [
            "sim.set_controls",
            "systems.set",
            "scenario.instructor_failure",
            "layout.move",
            "aircraft.load",
            "layout.save",
        ] {
            assert!(!command_allowed(Permission::ReadOnly, c));
        }
        assert!(command_allowed(Permission::Control, "systems.set"));
        assert!(!command_allowed(Permission::Control, "layout.save"));
        assert!(!lan("0.0.0.0".parse().unwrap()));
        assert!(!lan("8.8.8.8".parse().unwrap()));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod transport_tests {
    use super::*;
    use tungstenite::client::IntoClientRequest;
    fn connect(server: &Server, origin: &str) -> Result<tungstenite::WebSocket<TcpStream>, String> {
        let mut request = format!("ws://{}/screens", server.address())
            .into_client_request()
            .unwrap();
        request.headers_mut().insert("Origin", origin.parse().unwrap());
        tungstenite::client(request, TcpStream::connect(server.address()).unwrap())
            .map(|(socket, _)| socket)
            .map_err(|e| e.to_string())
    }
    #[test]
    fn websocket_enforces_pairing_origin_and_host_grants() {
        let root = std::env::temp_dir().join(format!("flightcraft-panel-transport-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), "test").unwrap();
        std::fs::write(root.join("flightcraft_web.js"), "test").unwrap();
        let runtime = crate::runtime::Runtime::start(
            flightcraft_control::Session::new().unwrap(),
            crate::script::Script::default(),
            crate::bridge::Bridge::default(),
            eframe::egui::Context::default(),
        )
        .unwrap();
        let server = Server::start("127.0.0.1:0".parse().unwrap(), root.clone(), runtime.source()).unwrap();
        let origin = format!("http://{}", server.address());
        assert!(connect(&server, "https://wrong.example").is_err());
        let mut bad = connect(&server, &origin).unwrap();
        bad.send(Message::text(json!({"token":"bad","name":"bad"}).to_string()))
            .unwrap();
        assert!(bad.read().is_err());
        let mut ws = connect(&server, &origin).unwrap();
        ws.send(Message::text(
            json!({"token":server.token(),"name":"instructor tablet"}).to_string(),
        ))
        .unwrap();
        let paired: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        let id = paired.get("id").and_then(Value::as_u64).unwrap();
        let command =
            json!({"id":1,"command":"systems.set","input":{"setting":{"kind":"mixture","value":0.65}}});
        ws.send(Message::text(command.to_string())).unwrap();
        let response: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert!(response.get("error").is_some());
        server.grant(id, Permission::Control);
        ws.send(Message::text(command.to_string())).unwrap();
        let response: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert!(response.get("ok").is_some(), "{response}");
        assert_eq!(
            runtime
                .source()
                .snapshot()
                .unwrap()
                .execute("sim.state", json!({}))
                .unwrap()
                .get("systems")
                .and_then(|v| v.get("controls"))
                .and_then(|v| v.get("mixture"))
                .and_then(Value::as_f64),
            Some(0.65)
        );
        server.grant(id, Permission::ReadOnly);
        ws.send(Message::text(command.to_string())).unwrap();
        assert!(ws.read().unwrap().to_text().unwrap().contains("no permission"));
        server.disconnect(id);
        drop(ws);
        drop(server);
        std::fs::remove_dir_all(root).unwrap();
    }
}
