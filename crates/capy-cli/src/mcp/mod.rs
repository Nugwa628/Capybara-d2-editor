//! `capy mcp --profile <id>`: a minimal Model Context Protocol server over
//! stdio (JSON-RPC 2.0, one message per line): `initialize`, `ping`,
//! `tools/list`, `tools/call`. Nothing else is advertised. Map edits go
//! through `capy-edit`; saves go to `capybara\saves\<profile>` only.

mod session;

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{Value, json};
use session::{Content, Session};

const SUPPORTED: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Capybara's D2 Editor edits Diablo II map presets (.ds1). Open a map with open_map (list_maps finds one) or new_map, look with get_cells / render_map, change it with paint, set_cell, copy_area, find_replace, reroll, resize_map, set_cell_flags, add_object, edit_objects, check it with check_map, and write it with save_map (into capybara\\saves\\<profile>; the game folder is never written). Edits can be undone with undo until the map is replaced. Cells are numbered from 0 at the map's top corner; x runs down-right, y down-left. Objects use sub-tiles: 5 per cell.";

/// Handles one line; the reply line, or `None` for notifications.
pub(crate) fn handle_line(
    session: &mut Option<Result<Session, String>>,
    make: &dyn Fn() -> Result<Session, String>,
    line: &str,
) -> Option<String> {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            return Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}})
                    .to_string(),
            );
        }
    };
    let id = msg.get("id").cloned();
    let notification = id.is_none();
    let id = id.unwrap_or(Value::Null);
    let reply = |result: Value| Some(json!({"jsonrpc":"2.0","id":id,"result":result}).to_string());
    let error = |code: i32, message: String| {
        if notification {
            None
        } else {
            Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
                    .to_string(),
            )
        }
    };
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("");
            let version = if SUPPORTED.contains(&asked) {
                asked
            } else {
                SUPPORTED[0]
            };
            reply(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "capybara", "title": "Capybara's D2 Editor", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => reply(json!({})),
        "tools/list" => reply(json!({"tools": session::tools_json()})),
        "tools/call" => {
            // The game data loads on the first call, so the handshake is instant.
            if session.is_none() {
                *session = Some(make());
            }
            let s = match session.as_mut() {
                Some(Ok(s)) => s,
                Some(Err(e)) => return error(-32603, e.clone()),
                None => return error(-32603, "no session".into()),
            };
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = match s.call(name, &args) {
                Ok(parts) => {
                    let content: Vec<Value> = parts
                        .into_iter()
                        .map(|c| match c {
                            Content::Text(t) => json!({"type":"text","text":t}),
                            Content::Image(png) => json!({"type":"image","data":session::base64(&png),"mimeType":"image/png"}),
                        })
                        .collect();
                    json!({"content": content})
                }
                Err(e) => json!({"content":[{"type":"text","text":e}],"isError":true}),
            };
            reply(result)
        }
        _ if notification => None,
        other => error(-32601, format!("Method not found: {other}")),
    }
}

/// Serves MCP on `input`/`output` until `input` ends. `profile` is already
/// loaded by the caller.
pub fn serve(
    root: PathBuf,
    profile: d2_assets::Profile,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> std::io::Result<()> {
    let make = move || Session::new(root.clone(), profile.clone());
    let mut session: Option<Result<Session, String>> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(reply) = handle_line(&mut session, &make, l) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
}
