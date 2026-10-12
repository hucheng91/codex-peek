//! Just enough of MCP (JSON-RPC 2.0 over newline-delimited stdio) to be a
//! well-behaved, tool-less server. Its only purpose is lifecycle.

use serde_json::{json, Value};

/// Returns the response line for one incoming message, or None for notifications.
pub fn respond(line: &str) -> Option<String> {
    let msg: Value = serde_json::from_str(line.trim()).ok()?;
    let id = msg.get("id")?.clone();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => {
            let version = msg
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-06-18");
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "codex-peek", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "codex-peek shows pasted images in a floating window. It provides no tools."
            })
        }
        "tools/list" => json!({ "tools": [] }),
        "ping" => json!({}),
        _ => return Some(
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}})
                .to_string(),
        ),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: Option<String>) -> Value {
        serde_json::from_str(&s.unwrap()).unwrap()
    }

    #[test]
    fn initialize_echoes_version() {
        let r = parse(respond(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        ));
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "codex-peek");
    }

    #[test]
    fn tools_and_errors() {
        let r = parse(respond(
            r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#,
        ));
        assert_eq!(r["result"]["tools"], json!([]));
        let r = parse(respond(
            r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#,
        ));
        assert_eq!(r["error"]["code"], -32601);
        assert!(respond(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert!(respond("not json").is_none());
    }
}
