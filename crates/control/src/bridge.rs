//! Native loopback transport. Threads move bounded lines; the flight owner executes at a step boundary.
use crate::CommandError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub const MAX_LINE_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(50);

/// Preserves partial lines across socket timeouts and drains oversized lines without allocating.
#[derive(Default)]
pub struct Lines {
    bytes: Vec<u8>,
    oversized: bool,
}
impl Lines {
    pub fn read(
        &mut self,
        input: &mut impl BufRead,
        limit: usize,
    ) -> io::Result<Option<Result<Vec<u8>, String>>> {
        loop {
            let buf = input.fill_buf()?;
            if buf.is_empty() {
                return if self.bytes.is_empty() && !self.oversized {
                    Ok(None)
                } else {
                    Ok(Some(self.finish()))
                };
            }
            let newline = buf.iter().position(|b| *b == b'\n');
            let count = newline.map_or(buf.len(), |i| i + 1);
            if !self.oversized {
                if self.bytes.len().saturating_add(count) > limit {
                    self.bytes.clear();
                    self.oversized = true;
                } else if let Some(chunk) = buf.get(..count) {
                    self.bytes.extend_from_slice(chunk);
                }
            }
            input.consume(count);
            if newline.is_some() {
                return Ok(Some(self.finish()));
            }
        }
    }
    fn finish(&mut self) -> Result<Vec<u8>, String> {
        if std::mem::take(&mut self.oversized) {
            Err("request line exceeds size limit".into())
        } else {
            Ok(std::mem::take(&mut self.bytes))
        }
    }
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: Value,
    pub command: String,
    pub input: Value,
}

pub fn parse(bytes: &[u8]) -> Result<Request, Value> {
    let result = (|| {
        if bytes.len() > MAX_LINE_BYTES {
            return Err("request line exceeds size limit".to_string());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let valid_id = id.is_null() || id.is_number() || id.is_string();
        let request: Request = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if !valid_id
            || !request.input.is_object()
            || request.command.is_empty()
            || request.command.len() > 128
        {
            return Err("id must be a scalar, command a name, and input an object".into());
        }
        Ok(request)
    })();
    result.map_err(|message| {
        // Preserve a usable correlation id even when another field is invalid.
        let id = (bytes.len() <= MAX_LINE_BYTES)
            .then(|| serde_json::from_slice::<Value>(bytes).ok())
            .flatten()
            .and_then(|v| v.get("id").cloned())
            .filter(|v| v.is_null() || v.is_number() || v.is_string())
            .unwrap_or(Value::Null);
        failure(id, message)
    })
}

pub fn success(id: Value, output: Value) -> Value {
    json!({"id": id, "ok": output})
}
pub fn failure(id: Value, message: impl Into<String>) -> Value {
    json!({"id": id, "error": message.into()})
}

pub struct Incoming {
    pub line: Result<Vec<u8>, String>,
    pub reply: mpsc::Sender<Value>,
}

pub struct Listener {
    requests: mpsc::Receiver<Incoming>,
    stop: Arc<AtomicBool>,
    address: SocketAddr,
}
impl Listener {
    /// Refuses all addresses except IPv4 127.0.0.1. Port zero is useful for tests.
    pub fn bind(address: SocketAddr) -> io::Result<Self> {
        if address.ip() != Ipv4Addr::LOCALHOST {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "control channel must bind to 127.0.0.1",
            ));
        }
        let socket = TcpListener::bind(address)?;
        let address = socket.local_addr()?;
        socket.set_nonblocking(true)?;
        let (send, requests) = mpsc::sync_channel(16);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let active = Arc::new(AtomicUsize::new(0));
        thread::Builder::new()
            .name("control-listener".into())
            .spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    match socket.accept() {
                        Ok((stream, _)) => {
                            if active.load(Ordering::Relaxed) >= 8 {
                                continue;
                            }
                            active.fetch_add(1, Ordering::Relaxed);
                            let active_worker = active.clone();
                            let send = send.clone();
                            let stop = stopping.clone();
                            if thread::Builder::new()
                                .name("control-client".into())
                                .spawn(move || {
                                    let _ = peer(stream, send, &stop);
                                    active_worker.fetch_sub(1, Ordering::Relaxed);
                                })
                                .is_err()
                            {
                                active.fetch_sub(1, Ordering::Relaxed);
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            requests,
            stop,
            address,
        })
    }
    pub fn loopback(port: u16) -> io::Result<Self> {
        Self::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into())
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn try_recv(&self) -> Result<Incoming, mpsc::TryRecvError> {
        self.requests.try_recv()
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn peer(stream: TcpStream, send: mpsc::SyncSender<Incoming>, stop: &AtomicBool) -> io::Result<()> {
    stream.set_read_timeout(Some(POLL))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut input = BufReader::new(stream.try_clone()?);
    let mut output = stream;
    let mut lines = Lines::default();
    while !stop.load(Ordering::Relaxed) {
        let line = match lines.read(&mut input, MAX_LINE_BYTES) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e),
        };
        let (reply, receive) = mpsc::channel();
        if send.try_send(Incoming { line, reply }).is_err() {
            break;
        }
        let response = loop {
            match receive.recv_timeout(POLL) {
                Ok(response) => break response,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    continue;
                }
                Err(_) => return Ok(()),
            }
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

/// Persistent synchronous forwarding connection, with bounded replies and correlation checks.
pub struct Client {
    input: BufReader<TcpStream>,
    next_id: u64,
}
impl Client {
    pub fn connect(port: u16) -> io::Result<Self> {
        let socket = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))?;
        socket.set_read_timeout(Some(Duration::from_secs(130)))?;
        socket.set_write_timeout(Some(Duration::from_secs(5)))?;
        Ok(Self {
            input: BufReader::new(socket),
            next_id: 0,
        })
    }
    pub fn execute(&mut self, command: &str, input: Value) -> Result<Value, CommandError> {
        let output = self
            .forward(command, input)
            .map_err(|e| CommandError::Failed(format!("app connection: {e}")))?;
        output.map_err(|message| {
            if let Some(detail) = message.strip_prefix("invalid input for ")
                && let Some((command, message)) = detail.split_once(": ")
            {
                CommandError::InvalidInput {
                    command: command.into(),
                    message: message.into(),
                }
            } else {
                CommandError::Failed(message)
            }
        })
    }
    fn forward(&mut self, command: &str, input: Value) -> Result<Result<Value, String>, String> {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        let request = json!({"id": id, "command": command, "input": input}).to_string();
        if request.len() + 1 > MAX_LINE_BYTES {
            return Err("request line exceeds size limit".into());
        }
        writeln!(self.input.get_mut(), "{request}").map_err(|e| e.to_string())?;
        self.input.get_mut().flush().map_err(|e| e.to_string())?;
        let bytes = Lines::default()
            .read(&mut self.input, MAX_RESPONSE_BYTES)
            .map_err(|e| e.to_string())?
            .ok_or("app disconnected")??;
        let response: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if response.get("id") != Some(&json!(id)) {
            return Err("response id mismatch".into());
        }
        match (response.get("ok"), response.get("error")) {
            (Some(output), None) => Ok(Ok(output.clone())),
            (None, Some(Value::String(error))) => Ok(Err(error.clone())),
            _ => Err("invalid app response".into()),
        }
    }
}
