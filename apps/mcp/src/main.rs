//! Newline-delimited MCP transport. Standard output contains protocol responses only.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use flightcraft_control::{Session, mcp};

fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::new()?;
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
                respond(&mut session, &mut output, &line, oversized)?;
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
            respond(&mut session, &mut output, &line, oversized)?;
            line.clear();
            oversized = false;
        }
    }
}

fn respond(session: &mut Session, output: &mut impl Write, bytes: &[u8], oversized: bool) -> io::Result<()> {
    let response = if oversized {
        Some("{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32700,\"message\":\"request line exceeds 1048576 bytes\"}}\n".to_string())
    } else {
        // Invalid UTF-8 cannot be JSON; the handler will produce a normal parse error.
        let line = std::str::from_utf8(bytes).unwrap_or("invalid UTF-8");
        mcp::handle_line(session, line)
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
