//! The listener: one loopback port, every agent and every built-in server behind it.
//!
//! MCP's "streamable HTTP" transport is JSON-RPC 2.0 over `POST`, and the protocol handling here —
//! `initialize`, a bare 202 for a notification, `tools/list`, `tools/call` with in-band `isError`,
//! `-32601` for an unknown method, `-32700` for a body that will not parse — is the same shape
//! `crates/agent-manager/src/mcp/server.rs` proved. The one difference is the reason this exists
//! at all: that one hosts a single service, this one hosts every `(agent, server)` pair Ubiq has,
//! so the identity that would have been the server's own comes off the URL on every request.
//!
//! ```text
//! POST /mcps/<agent-or-pane-id>/<mcp-name>
//! ```
//!
//! **Stateless, deliberately.** No session id, no SSE stream, nothing remembered between two
//! requests, and no authentication beyond the loopback interface and an unguessable ULID in the
//! path. A session layer would be state to lose when the harness reconnects, and the only thing it
//! would buy is a second place for the agent's identity to live — the URL already carries it, on
//! every call, and the harness was handed that URL by the run Ubiq composed.
//!
//! An address that resolves to nobody — an agent that has retired, a server this build does not
//! offer, a path of the wrong shape — is a plain **404**, not a JSON-RPC error. The distinction is
//! the one the transport makes everywhere: a JSON-RPC error means "this server heard you and
//! refused"; there is no server at that address to hear.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};
use ubiq_proto::bus::Voice;

use super::catalogue::{self, ServerSpec};
use super::registry::{AgentFacts, Registry};

/// How often the serving thread wakes to check whether it should stop. Bounds shutdown latency
/// without needing to unblock the listener.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The MCP protocol revision this server answers as. The same one the library's in-process server
/// names, because it implements the same subset.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// The running listener: where it is bound, and the thread serving it.
///
/// Held by the coordinator for the life of the process, the way [`crate::files::Files`] and the
/// other worker handles are. Dropping it stops the thread and joins it, so a host that goes takes
/// its port with it rather than leaving a listener answering for agents that no longer exist.
pub struct Serving {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Serving {
    /// What a run's injected server URL is built on: `http://127.0.0.1:<port>`, with the agent and
    /// the server name appended by whoever composes the run.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl std::fmt::Debug for Serving {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Serving")
            .field("base_url", &self.base_url())
            .finish()
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            // The serving thread polls `stop` at most `POLL_INTERVAL` apart, so this returns
            // promptly. A thread that already panicked has nothing left to join.
            let _ = handle.join();
        }
    }
}

/// Bind a loopback port and start serving, on a thread of its own.
///
/// Returns once the port is bound, so the caller can hand the URL to the first run it composes —
/// which is why this happens before [`crate::agent::Agents`] is built rather than lazily on the
/// first agent that asks for a server.
pub fn start(registry: Registry, voice: Voice) -> anyhow::Result<Serving> {
    let http = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|error| anyhow::anyhow!("binding the MCP listener: {error}"))?;
    let port = http
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow::anyhow!("the MCP listener bound no IP address"))?
        .port();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let handle = std::thread::Builder::new()
        .name("ubiq-mcp".to_string())
        .spawn(move || serve(http, registry, voice, stop_thread))
        .expect("the MCP listener thread");

    Ok(Serving {
        port,
        stop,
        handle: Some(handle),
    })
}

/// The serving loop: poll with a bounded timeout so [`Drop`] can stop it, and handle each request
/// fully before reading the next. One request at a time is enough — a tool call here reads a
/// snapshot under a read lock and returns.
fn serve(http: tiny_http::Server, registry: Registry, voice: Voice, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        match http.recv_timeout(POLL_INTERVAL) {
            Ok(Some(request)) => handle(request, &registry, &voice),
            Ok(None) => continue,
            Err(_) => break,
        }
    }
}

/// Resolve the address, then speak the protocol. Never panics on anything a client sent: the worst
/// a malformed request gets is a parse error, and the worst a wrong address gets is a 404.
fn handle(mut request: tiny_http::Request, registry: &Registry, voice: &Voice) {
    let Some((key, server)) = route(request.url()) else {
        let _ = request.respond(not_found());
        return;
    };
    let Some(spec) = catalogue::server(&server) else {
        let _ = request.respond(not_found());
        return;
    };
    let Some(facts) = registry.facts(&key) else {
        // Ordinary rather than alarming: a harness can outlive the conversation that started it by
        // a moment, and a stale URL is exactly what that looks like from here.
        tracing::debug!(agent = %key, mcp = %server, "an MCP request named an agent nobody knows");
        let _ = request.respond(not_found());
        return;
    };

    let mut body = String::new();
    if let Err(error) = request.as_reader().read_to_string(&mut body) {
        let _ = request.respond(json_response(&parse_error(&error.to_string())));
        return;
    }

    let parsed: Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(error) => {
            let _ = request.respond(json_response(&parse_error(&error.to_string())));
            return;
        }
    };

    // A JSON-RPC notification carries no `id` and gets no body — a bare 202. That covers
    // `notifications/initialized` and anything else a client fires and forgets.
    let Some(id) = parsed.get("id").cloned() else {
        let _ = request.respond(tiny_http::Response::empty(202));
        return;
    };

    let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
    let params = parsed.get("params").cloned().unwrap_or(Value::Null);

    let response = match dispatch(method, params, spec, &facts, voice) {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    };
    let _ = request.respond(json_response(&response));
}

/// Read `/mcps/<agent>/<name>` off a request URL, ignoring any query string. Anything else — a
/// different prefix, a missing segment, an extra one — is not an address this host serves.
fn route(url: &str) -> Option<(String, String)> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    if segments.next()? != "mcps" {
        return None;
    }
    let key = segments.next()?.to_string();
    let server = segments.next()?.to_string();
    if segments.next().is_some() || key.is_empty() || server.is_empty() {
        return None;
    }
    Some((key, server))
}

/// One JSON-RPC method, for one agent's view of one server. `Ok` becomes `result`;
/// `Err((code, message))` becomes `error`.
///
/// A failing *tool* is not an error here: it comes back as `isError` inside a normal result,
/// because that is the failure a model is meant to read and correct. A method this server does not
/// implement is `-32601`, which is a client that is speaking a protocol we do not.
fn dispatch(
    method: &str,
    params: Value,
    spec: &ServerSpec,
    facts: &AgentFacts,
    voice: &Voice,
) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": format!("ubiq-{}", spec.name), "version": env!("CARGO_PKG_VERSION")},
        })),
        "tools/list" => Ok(json!({"tools": spec.tool_list()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match super::tools::call(spec.name, name, &arguments, facts, voice) {
                Ok(value) => {
                    let text = serde_json::to_string(&value).unwrap_or_default();
                    Ok(json!({
                        "content": [{"type": "text", "text": text}],
                        "isError": false,
                    }))
                }
                Err(error) => Ok(json!({
                    "content": [{"type": "text", "text": error}],
                    "isError": true,
                })),
            }
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

/// A JSON-RPC parse error (`-32700`). Nothing recoverable identifies the call, so `id` is null,
/// as the spec says.
fn parse_error(message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": {"code": -32700, "message": format!("parse error: {message}")},
    })
}

fn not_found() -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_data(Vec::new()).with_status_code(404)
}

fn json_response(body: &Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    let content_type =
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
            .expect("a static header is valid");
    tiny_http::Response::from_data(bytes).with_header(content_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::registry::{AgentFacts, ProjectFacts};
    use ubiq_proto::bus;
    use ubiq_proto::messages::Message;

    /// Every test binds a real port and speaks real HTTP to it, because the thing worth proving is
    /// the transport rather than the dispatch. The timeout is what keeps a wrong answer a failure
    /// instead of a hang.
    const TIMEOUT: Duration = Duration::from_secs(5);

    const KEY: &str = "01JBTESTAGENTIDULID000000";

    fn facts() -> AgentFacts {
        AgentFacts {
            key: KEY.to_string(),
            name: "claude 1".to_string(),
            harness: "Claude Code".to_string(),
            account: Some("work".to_string()),
            model: Some("opus".to_string()),
            mode: Some("default".to_string()),
            cwd: "/tmp/project".to_string(),
            session: None,
            project: ProjectFacts {
                id: "01JBTESTPROJECTID00000000".to_string(),
                name: "Ubiq".to_string(),
                path: "/tmp/project".to_string(),
                colour: 3,
            },
        }
    }

    /// A listener with one agent registered, and the host end the notification tool speaks into.
    ///
    /// The `Hub` comes back with it and is held for the test's life on purpose: a [`bus::Voice`]
    /// holds a weak sender, so a dropped hub is a bus that has closed and a notification that goes
    /// nowhere.
    fn running() -> (Serving, bus::Hub, bus::HostEnd) {
        let (hub, host) = bus::hub();
        let registry = Registry::new();
        registry.register(facts());
        let serving = start(registry, host.voice()).expect("the listener binds");
        (serving, hub, host)
    }

    fn url(serving: &Serving, key: &str, server: &str) -> String {
        format!("{}/mcps/{key}/{server}", serving.base_url())
    }

    fn post(url: &str, body: Value) -> Value {
        ureq::post(url)
            .timeout(TIMEOUT)
            .send_string(&body.to_string())
            .expect("the request is answered")
            .into_json()
            .expect("the answer is JSON")
    }

    fn call(url: &str, tool: &str, arguments: Value) -> Value {
        post(
            url,
            json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
                   "params": {"name": tool, "arguments": arguments}}),
        )
    }

    /// The text a tool answered, parsed back out of MCP's content block.
    fn answered(response: &Value) -> Value {
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        serde_json::from_str(text).unwrap_or(Value::Null)
    }

    #[test]
    fn the_test_server_initializes_lists_and_calls() {
        let (serving, _hub, host) = running();
        let url = url(&serving, KEY, "test");

        let init = post(
            &url,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "ubiq-test");
        assert_eq!(init["result"]["protocolVersion"], PROTOCOL_VERSION);

        let list = post(
            &url,
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        );
        let tools = list["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["send_notification", "write_log"]);
        assert_eq!(tools[0]["inputSchema"]["type"], "object");

        let logged = call(
            &url,
            "write_log",
            json!({"message": "hello", "level": "warn"}),
        );
        assert_eq!(logged["result"]["isError"], false);
        assert_eq!(answered(&logged)["logged"], true);

        let raised = call(&url, "send_notification", json!({"text": "wired up"}));
        assert_eq!(raised["result"]["isError"], false);
        assert_eq!(answered(&raised)["level"], "info");

        // The notification is a real one: it reaches the host's inbox as the message a window
        // would have sent, so the same centre answers it.
        match host.recv_timeout(TIMEOUT) {
            Ok(bus::FromClient::Said {
                message: Message::RaiseNotification { request },
                ..
            }) => {
                assert_eq!(request.text, "wired up");
                assert_eq!(request.actor.as_deref(), Some("claude 1"));
                assert_eq!(request.category.as_deref(), Some("Claude Code"));
            }
            other => panic!("expected a raised notification, got {other:?}"),
        }
    }

    #[test]
    fn the_project_info_server_answers_this_agents_own_facts() {
        let (serving, _hub, _host) = running();
        let url = url(&serving, KEY, "project-info");

        let list = post(
            &url,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["project_info", "whoami"]);

        let project = answered(&call(&url, "project_info", json!({})));
        assert_eq!(project["name"], "Ubiq");
        assert_eq!(project["path"], "/tmp/project");
        assert_eq!(project["colour"], 3);

        let me = answered(&call(&url, "whoami", json!({})));
        assert_eq!(me["agent_id"], KEY);
        assert_eq!(me["harness"], "Claude Code");
        assert_eq!(me["account"], "work");
        assert_eq!(me["model"], "opus");
    }

    #[test]
    fn an_unknown_agent_is_a_404() {
        let (serving, _hub, _host) = running();
        let response = ureq::post(&url(&serving, "01JNOBODY0000000000000000", "test"))
            .timeout(TIMEOUT)
            .send_string(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}).to_string());
        assert_eq!(status(response), 404);
    }

    #[test]
    fn an_unknown_server_and_a_bogus_path_are_both_404() {
        let (serving, _hub, _host) = running();
        let unknown = ureq::post(&url(&serving, KEY, "no-such-server"))
            .timeout(TIMEOUT)
            .send_string("{}");
        assert_eq!(status(unknown), 404);

        for path in [
            "/",
            "/mcps",
            "/mcps/only-one",
            "/elsewhere/a/b",
            "/mcps/a/b/c",
        ] {
            let response = ureq::post(&format!("{}{path}", serving.base_url()))
                .timeout(TIMEOUT)
                .send_string("{}");
            assert_eq!(status(response), 404, "path {path}");
        }
    }

    #[test]
    fn an_unknown_tool_is_an_in_band_error_not_a_json_rpc_error() {
        let (serving, _hub, _host) = running();
        let response = call(&url(&serving, KEY, "test"), "nope", json!({}));
        assert!(response.get("error").is_none());
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("unknown tool")
        );
    }

    #[test]
    fn a_tool_called_without_its_argument_fails_in_band() {
        let (serving, _hub, _host) = running();
        let response = call(&url(&serving, KEY, "test"), "write_log", json!({}));
        assert_eq!(response["result"]["isError"], true);
    }

    #[test]
    fn an_unknown_method_is_a_json_rpc_error_and_a_notification_is_a_bare_202() {
        let (serving, _hub, _host) = running();
        let url = url(&serving, KEY, "test");

        let response = post(
            &url,
            json!({"jsonrpc": "2.0", "id": 1, "method": "nope/nope"}),
        );
        assert_eq!(response["error"]["code"], -32601);

        let notified = ureq::post(&url)
            .timeout(TIMEOUT)
            .send_string(
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
            )
            .expect("the request is answered");
        assert_eq!(notified.status(), 202);
        assert_eq!(notified.into_string().unwrap(), "");
    }

    #[test]
    fn a_body_that_will_not_parse_is_a_parse_error() {
        let (serving, _hub, _host) = running();
        let response = ureq::post(&url(&serving, KEY, "test"))
            .timeout(TIMEOUT)
            .send_string("not json")
            .expect("the request is answered");
        let value: Value = response.into_json().unwrap();
        assert_eq!(value["error"]["code"], -32700);
    }

    /// `ureq` reports a 4xx as an `Err`, so both sides of the result have a status.
    fn status(result: Result<ureq::Response, ureq::Error>) -> u16 {
        match result {
            Ok(response) => response.status(),
            Err(ureq::Error::Status(code, _)) => code,
            Err(error) => panic!("the request failed outright: {error}"),
        }
    }
}
