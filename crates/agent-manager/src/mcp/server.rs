//! A minimal MCP "streamable HTTP" server (JSON-RPC 2.0 over HTTP POST)
//! that hosts a single in-process [`super::McpService`] on `127.0.0.1`.
//!
//! This is intentionally small: just enough of the MCP spec for `am` to
//! inject an in-process tool into a harness that only knows how to talk to
//! remote MCP servers over http. It implements `initialize`,
//! `notifications/initialized`, `tools/list`, and `tools/call`; anything
//! else gets a JSON-RPC "method not found" error. Sessions, SSE streaming,
//! and other transport-level MCP features are out of scope — every request
//! is handled fully before the next is read (one request at a time).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};

use super::McpService;
use crate::Result;

/// How often the serving thread wakes up to check whether it should stop.
/// Bounds shutdown latency on [`Drop`] without needing `tiny_http::unblock`.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// A running loopback HTTP MCP server backed by an [`McpService`].
///
/// Kept alive for the duration of a run (see [`crate::provision::Provisioned`]);
/// dropping it stops the serving thread and joins it.
pub struct InProcessServer {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl InProcessServer {
    /// The `http://127.0.0.1:<port>/mcp` URL harnesses should be pointed at.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }
}

impl std::fmt::Debug for InProcessServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessServer")
            .field("url", &self.url())
            .finish()
    }
}

impl Drop for InProcessServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            // Best-effort: the serving thread polls `stop` at most
            // `POLL_INTERVAL` apart, so this returns promptly. If the
            // thread already panicked, there's nothing more to do.
            let _ = handle.join();
        }
    }
}

/// Start hosting `service` on a fresh loopback port; returns once the port
/// is bound and the serving thread is running.
pub fn start(service: Arc<dyn McpService>) -> Result<InProcessServer> {
    let http_server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|err| anyhow::anyhow!("failed to bind in-process MCP server: {err}"))?;
    let port = http_server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow::anyhow!("in-process MCP server has no IP listen address"))?
        .port();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let handle = std::thread::spawn(move || serve(http_server, service, stop_thread));

    Ok(InProcessServer {
        port,
        stop,
        handle: Some(handle),
    })
}

/// The serving loop: poll for a request with a bounded timeout (so `Drop`
/// can stop us promptly) and handle it fully before looping.
fn serve(http_server: tiny_http::Server, service: Arc<dyn McpService>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        match http_server.recv_timeout(POLL_INTERVAL) {
            Ok(Some(request)) => handle_request(request, &service),
            Ok(None) => continue, // timed out, re-check `stop`
            Err(_) => break,      // listener gone; nothing more we can do
        }
    }
}

/// Handle one HTTP request end to end: read the JSON-RPC body, dispatch, and
/// respond. Never panics on malformed input — worst case is a JSON-RPC
/// parse-error response.
fn handle_request(mut request: tiny_http::Request, service: &Arc<dyn McpService>) {
    let mut body = String::new();
    if let Err(err) = request.as_reader().read_to_string(&mut body) {
        let _ = request.respond(json_response(&parse_error(&err.to_string())));
        return;
    }

    let parsed: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(err) => {
            let _ = request.respond(json_response(&parse_error(&err.to_string())));
            return;
        }
    };

    // JSON-RPC notifications carry no "id" member and get no response body
    // — just a bare 202 Accepted (covers `notifications/initialized`, and
    // any other notification a client might send).
    let Some(id) = parsed.get("id").cloned() else {
        let _ = request.respond(tiny_http::Response::empty(202));
        return;
    };

    let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
    let params = parsed.get("params").cloned().unwrap_or(Value::Null);

    let response = match dispatch(method, params, service) {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    };
    let _ = request.respond(json_response(&response));
}

/// Dispatch one JSON-RPC method call to its MCP handler. `Ok` becomes the
/// `result` field; `Err(code, message)` becomes the `error` field.
fn dispatch(
    method: &str,
    params: Value,
    service: &Arc<dyn McpService>,
) -> std::result::Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "agent-manager-inproc", "version": "0.1.0"},
        })),
        "tools/list" => {
            let tools: Vec<Value> = service
                .tools()
                .into_iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                    })
                })
                .collect();
            Ok(json!({"tools": tools}))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match service.call(name, arguments) {
                Ok(value) => {
                    let text = serde_json::to_string(&value).unwrap_or_default();
                    Ok(json!({
                        "content": [{"type": "text", "text": text}],
                        "isError": false,
                    }))
                }
                Err(err) => Ok(json!({
                    "content": [{"type": "text", "text": err.to_string()}],
                    "isError": true,
                })),
            }
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

/// A JSON-RPC parse-error response (`-32700`). No `id` is recoverable from
/// unparsable input, so `id` is `null` per the JSON-RPC spec.
fn parse_error(message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": {"code": -32700, "message": format!("parse error: {message}")},
    })
}

/// Wrap a JSON-RPC response body as a `200 application/json` HTTP response.
fn json_response(body: &Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    let content_type =
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
            .expect("static header is valid");
    tiny_http::Response::from_data(bytes).with_header(content_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::ToolDef;
    use std::time::Duration;

    struct EchoService;

    impl McpService for EchoService {
        fn tools(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "echo".to_string(),
                description: "Echoes its arguments back.".to_string(),
                input_schema: json!({"type": "object", "properties": {"x": {"type": "number"}}}),
            }]
        }

        fn call(&self, name: &str, arguments: Value) -> crate::Result<Value> {
            if name != "echo" {
                anyhow::bail!("unknown tool: {name}");
            }
            Ok(json!({ "echoed": arguments }))
        }
    }

    fn post(url: &str, body: Value) -> Value {
        ureq::post(url)
            .timeout(Duration::from_secs(5))
            .send_string(&body.to_string())
            .expect("request should succeed")
            .into_json()
            .expect("response should be JSON")
    }

    #[test]
    fn initialize_tools_list_and_tools_call_round_trip() {
        let server = start(Arc::new(EchoService)).unwrap();
        let url = server.url();

        let init = post(&url, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
        assert_eq!(init["result"]["serverInfo"]["name"], "agent-manager-inproc");

        let list = post(&url, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "echo");
        assert_eq!(tools[0]["inputSchema"]["type"], "object");

        let call = post(
            &url,
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "echo", "arguments": {"x": 1}}}),
        );
        assert_eq!(call["result"]["isError"], false);
        let text = call["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"x\":1"), "text was: {text}");
    }

    #[test]
    fn tools_call_on_unknown_tool_is_an_in_band_error_not_a_json_rpc_error() {
        let server = start(Arc::new(EchoService)).unwrap();
        let call = post(
            &server.url(),
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "nope", "arguments": {}}}),
        );
        assert!(call.get("error").is_none());
        assert_eq!(call["result"]["isError"], true);
        assert!(call["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
    }

    #[test]
    fn unknown_method_is_a_json_rpc_error() {
        let server = start(Arc::new(EchoService)).unwrap();
        let resp = post(&server.url(), json!({"jsonrpc": "2.0", "id": 1, "method": "nope/nope"}));
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn notification_gets_a_bare_202_with_no_body() {
        let server = start(Arc::new(EchoService)).unwrap();
        let resp = ureq::post(&server.url())
            .timeout(Duration::from_secs(5))
            .send_string(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string())
            .expect("request should succeed");
        assert_eq!(resp.status(), 202);
        assert_eq!(resp.into_string().unwrap(), "");
    }

    #[test]
    fn malformed_body_gets_a_parse_error_not_a_panic() {
        let server = start(Arc::new(EchoService)).unwrap();
        let resp = ureq::post(&server.url())
            .timeout(Duration::from_secs(5))
            .send_string("not json")
            .expect("request should succeed");
        let value: Value = resp.into_json().unwrap();
        assert_eq!(value["error"]["code"], -32700);
    }

    #[test]
    fn drop_stops_the_serving_thread_without_hanging() {
        let server = start(Arc::new(EchoService)).unwrap();
        let _ = post(&server.url(), json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}));
        // Dropping joins the serving thread; this test's own bounded
        // duration (well under the harness's timeouts) is the hang guard.
        drop(server);
    }
}
