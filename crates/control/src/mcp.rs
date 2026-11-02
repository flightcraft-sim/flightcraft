//! Synchronous, transport-independent MCP over newline-delimited JSON-RPC 2.0.
//! Pass each line to [`handle_line`]; notifications return no response.

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{CommandError, Session};

/// Bounds both the stdio transport and direct handler calls.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Handle a request without IO, clocks, or an async runtime. Responses include a trailing newline.
#[must_use]
pub fn handle_line(session: &mut Session, line: &str) -> Option<String> {
    handle_line_with(&mut |name, input| session.execute(name, input), line)
}

/// Use the same MCP protocol and registry with a different command transport.
pub fn handle_line_with(
    execute: &mut impl FnMut(&str, Value) -> Result<Value, CommandError>,
    line: &str,
) -> Option<String> {
    let response = if line.len() > MAX_LINE_BYTES {
        error(Value::Null, -32700, "request line exceeds 1048576 bytes")
    } else {
        match serde_json::from_str::<Value>(line) {
            Ok(request) => handle_request(execute, request)?,
            Err(parse_error) => error(Value::Null, -32700, &parse_error.to_string()),
        }
    };
    // Value contains only JSON-compatible values; Display serializes without a fallible writer.
    Some(format!("{response}\n"))
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn handle_request(
    execute: &mut impl FnMut(&str, Value) -> Result<Value, CommandError>,
    request: Value,
) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(error(Value::Null, -32600, "request must be an object"));
    };
    let id = object.get("id").cloned();
    let valid_id = id
        .as_ref()
        .is_none_or(|id| id.is_string() || id.is_number() || id.is_null());
    let method = object.get("method").and_then(Value::as_str);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || method.is_none() || !valid_id {
        let id = if valid_id {
            id.unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        return Some(error(id, -32600, "invalid JSON-RPC 2.0 request"));
    }
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = dispatch(execute, method.unwrap_or_default(), params);
    // JSON-RPC notifications never receive responses, including errors.
    let id = id?;
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => error(id, code, &message),
    })
}

type RpcResult = Result<Value, (i32, String)>;

fn params<T: DeserializeOwned>(value: Value) -> Result<T, (i32, String)> {
    if !value.is_object() {
        return Err((-32602, "params must be an object".into()));
    }
    serde_path_to_error::deserialize(value).map_err(|error| (-32602, error.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {
    #[serde(rename = "_meta")]
    _meta: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitializeParams {
    protocol_version: String,
    capabilities: serde_json::Map<String, Value>,
    client_info: ClientInfo,
    #[serde(rename = "_meta")]
    _meta: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientInfo {
    name: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListParams {
    cursor: Option<String>,
    #[serde(rename = "_meta")]
    _meta: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallParams {
    name: String,
    #[serde(default = "empty_object")]
    arguments: Value,
    #[serde(rename = "_meta")]
    _meta: Option<serde_json::Map<String, Value>>,
}

fn empty_object() -> Value {
    json!({})
}

fn dispatch(
    execute: &mut impl FnMut(&str, Value) -> Result<Value, CommandError>,
    method: &str,
    input: Value,
) -> RpcResult {
    match method {
        "initialize" => {
            if !input.get("clientInfo").is_some_and(Value::is_object) {
                return Err((-32602, "clientInfo must be an object".into()));
            }
            let input: InitializeParams = params(input)?;
            // Accept older supported versions, otherwise negotiate the server's version.
            let version = match input.protocol_version.as_str() {
                "2024-11-05" => "2024-11-05",
                "2025-03-26" => "2025-03-26",
                _ => PROTOCOL_VERSION,
            };
            // Deserialization validates the shape of client metadata; none is retained.
            let _ = (
                input.capabilities,
                input.client_info.name,
                input.client_info.version,
            );
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "flightcraft-mcp", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Use sim_reset, autopilot_set, sim_run_until, autopilot_off, sim_set_controls, sim_step, and sim_state. Units are SI except weather cloud heights (MSL ft) and visibility (SM). In bridged mode pause before sim_step or sim_run_until; sim_wait waits for real-time flight while running. App commands need a desktop connection."
            }))
        }
        "ping" | "notifications/initialized" => {
            let _: EmptyParams = params(input)?;
            Ok(json!({}))
        }
        "tools/list" => {
            let input: ListParams = params(input)?;
            if input.cursor.is_some() {
                return Err((-32602, "unknown cursor; all tools fit in one page".into()));
            }
            let tools: Vec<Value> = Session::commands()
                .into_iter()
                .map(|command| {
                    json!({
                        "name": command.name.replace('.', "_"),
                        "description": command.description,
                        "inputSchema": command.input_schema,
                        "outputSchema": command.output_schema,
                    })
                })
                .collect();
            Ok(json!({"tools": tools}))
        }
        "tools/call" => {
            let mut input: CallParams = params(input)?;
            let command = Session::commands()
                .into_iter()
                .find(|command| command.name.replace('.', "_") == input.name)
                .ok_or_else(|| (-32602, format!("unknown tool: {}", input.name)))?;
            if command.name == "app.screenshot"
                && let Some(args) = input.arguments.as_object_mut()
                && args.get("base64").is_none_or(Value::is_boolean)
            {
                args.insert("base64".into(), json!(true));
            }
            match execute(command.name, input.arguments) {
                Ok(mut output) => {
                    // A screenshot's PNG goes in the image block only: repeating it in the text and
                    // the structured copy would triple what a client has to read.
                    let image = if command.name == "app.screenshot" {
                        output
                            .as_object_mut()
                            .and_then(|fields| fields.remove("png_base64"))
                            .filter(Value::is_string)
                    } else {
                        None
                    };
                    let mut content = vec![json!({"type": "text", "text": output.to_string()})];
                    if let Some(data) = image {
                        content.push(json!({"type": "image", "mimeType": "image/png", "data": data}));
                    }
                    Ok(json!({"content": content, "structuredContent": output, "isError": false}))
                }
                Err(error @ (CommandError::InvalidInput { .. } | CommandError::Unknown(_))) => {
                    Err((-32602, error.to_string()))
                }
                Err(error) => Ok(json!({
                    "content": [{"type": "text", "text": error.to_string()}],
                    "isError": true,
                })),
            }
        }
        _ => Err((-32601, format!("unknown method: {method}"))),
    }
}
