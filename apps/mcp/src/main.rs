//! Newline-delimited MCP transport. Standard output contains protocol responses only.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use flightcraft_control::{Session, mcp};

fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let port = match args.next().as_deref() {
        None => None,
        Some("--connect") => {
            let port: u16 = args
                .next()
                .ok_or("--connect needs a port from 1 to 65535")?
                .parse()?;
            if port == 0 || args.next().is_some() {
                return Err("usage: flightcraft-mcp [--connect <port>]".into());
            }
            Some(port)
        }
        Some("--help" | "-h") => {
            eprintln!(
                "usage: flightcraft-mcp [--connect <port>]\nConnect to the desktop's 127.0.0.1 control channel, or run headless."
            );
            return Ok(());
        }
        Some(_) => return Err("usage: flightcraft-mcp [--connect <port>]".into()),
    };
    #[cfg(not(target_arch = "wasm32"))]
    let mut client = port
        .map(flightcraft_control::bridge::Client::connect)
        .transpose()?;
    #[cfg(target_arch = "wasm32")]
    if port.is_some() {
        return Err("--connect is unavailable on wasm".into());
    }
    let mut session = Session::new()?;
    // Regions from the working directory, else those shipped beside the executable.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let local = std::path::PathBuf::from("assets/terrain");
        let directory = if local.is_dir() {
            local
        } else {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("assets/terrain")))
                .filter(|dir| dir.is_dir())
                .unwrap_or(local)
        };
        for error in session.load_regions_directory(&directory) {
            eprintln!("{error}");
        }
    }
    let mut execute = |name: &str, input| {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(client) = &mut client {
            return client.execute(name, input);
        }
        session.execute(name, input)
    };
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        // Consume in chunks to bound memory even for a hostile line with no newline.
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            if !line.is_empty() || oversized {
                respond(&mut execute, &mut output, &line, oversized)?;
            }
            return Ok(());
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(buffer.len(), |index| index + 1);
        if !oversized {
            if line.len().saturating_add(count) > mcp::MAX_LINE_BYTES {
                oversized = true;
                line.clear();
            } else if let Some(chunk) = buffer.get(..count) {
                line.extend_from_slice(chunk);
            }
        }
        input.consume(count);
        if newline.is_some() {
            respond(&mut execute, &mut output, &line, oversized)?;
            line.clear();
            oversized = false;
        }
    }
}

fn respond(
    execute: &mut impl FnMut(
        &str,
        serde_json::Value,
    ) -> Result<serde_json::Value, flightcraft_control::CommandError>,
    output: &mut impl Write,
    bytes: &[u8],
    oversized: bool,
) -> io::Result<()> {
    let response = if oversized {
        Some("{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32700,\"message\":\"request line exceeds 1048576 bytes\"}}\n".to_string())
    } else {
        // Invalid UTF-8 cannot be JSON; the handler will produce a normal parse error.
        let line = std::str::from_utf8(bytes).unwrap_or("invalid UTF-8");
        mcp::handle_line_with(execute, line)
    };
    if let Some(response) = response {
        output.write_all(response.as_bytes())?;
        output.flush()?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flightcraft-mcp: {error}");
            ExitCode::FAILURE
        }
    }
}
