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
use super::{AskReach, HelpReach, KbReach, MissionReach, PlanReach, WorkAccess};

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
#[allow(clippy::too_many_arguments)]
pub fn start(
    registry: Registry,
    voice: Voice,
    work: Option<WorkAccess>,
    plan: Option<PlanReach>,
    mission: Option<MissionReach>,
    kb: Option<KbReach>,
    help: Option<HelpReach>,
    ask: Option<AskReach>,
) -> anyhow::Result<Serving> {
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
        .spawn(move || {
            serve(
                http,
                registry,
                voice,
                work,
                plan,
                mission,
                kb,
                help,
                ask,
                stop_thread,
            )
        })
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
///
/// **Except one.** `ubiq-ask`'s tool waits for a person, and a request that waits here is every
/// other agent's tool calls waiting behind it. [`handle`] moves that one request onto a thread of
/// its own and comes straight back, which is what keeps this loop the cheap thing it is (`D138`).
#[allow(clippy::too_many_arguments)]
fn serve(
    http: tiny_http::Server,
    registry: Registry,
    voice: Voice,
    work: Option<WorkAccess>,
    plan: Option<PlanReach>,
    mission: Option<MissionReach>,
    kb: Option<KbReach>,
    help: Option<HelpReach>,
    ask: Option<AskReach>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::SeqCst) {
        match http.recv_timeout(POLL_INTERVAL) {
            Ok(Some(request)) => handle(
                request,
                &registry,
                &voice,
                work.as_ref(),
                plan.as_ref(),
                mission.as_ref(),
                kb.as_ref(),
                help.as_ref(),
                ask.as_ref(),
            ),
            Ok(None) => continue,
            Err(_) => break,
        }
    }
}

/// Resolve the address, then speak the protocol. Never panics on anything a client sent: the worst
/// a malformed request gets is a parse error, and the worst a wrong address gets is a 404.
#[allow(clippy::too_many_arguments)]
fn handle(
    mut request: tiny_http::Request,
    registry: &Registry,
    voice: &Voice,
    work: Option<&WorkAccess>,
    plan: Option<&PlanReach>,
    mission: Option<&MissionReach>,
    kb: Option<&KbReach>,
    help: Option<&HelpReach>,
    ask: Option<&AskReach>,
) {
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

    // The parking tool is the one request this thread does not serve itself. `tiny_http`'s
    // `Request` is `Send`, so the whole of it — the call and the response nobody has written yet —
    // moves onto a thread that can afford to wait for a human, and the listener goes straight back
    // to reading (`D138`). Only `tools/call` is moved: `initialize` and `tools/list` on the same
    // server answer instantly and belong here.
    //
    // **And only the parking tool of the two.** `register_question` files a row and returns
    // (`D175`), so it is an ordinary request and a thread for it would be a thread to do nothing
    // on.
    if spec.name == catalogue::UBIQ_ASK
        && method == "tools/call"
        && params.get("name").and_then(Value::as_str) == Some("ask_user_question")
        && let Some(reach) = ask
    {
        let reach = AskReach {
            asks: Arc::clone(&reach.asks),
            armed: Arc::clone(&reach.armed),
        };
        let voice = voice.clone();
        let spawned = std::thread::Builder::new()
            .name("ubiq-ask-call".to_string())
            .spawn(move || {
                let result = dispatch(
                    "tools/call", params, spec, &facts, &voice, None, None, None, None, None,
                    Some(&reach),
                );
                let response = match result {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err((code, message)) => {
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
                    }
                };
                let _ = request.respond(json_response(&response));
            });
        if let Err(error) = spawned {
            // A thread that will not start is not a call that may hang here instead.
            tracing::error!("an ask could not be served on a thread of its own: {error}");
        }
        return;
    }

    let response = match dispatch(
        method, params, spec, &facts, voice, work, plan, mission, kb, help, ask,
    ) {
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
#[allow(clippy::too_many_arguments)]
fn dispatch(
    method: &str,
    params: Value,
    spec: &ServerSpec,
    facts: &AgentFacts,
    voice: &Voice,
    work: Option<&WorkAccess>,
    plan: Option<&PlanReach>,
    mission: Option<&MissionReach>,
    kb: Option<&KbReach>,
    help: Option<&HelpReach>,
    ask: Option<&AskReach>,
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
            match super::tools::call(
                spec.name, name, &arguments, facts, voice, work, plan, mission, kb, help, ask,
            ) {
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
            mission: None,
            key: KEY.to_string(),
            name: "claude 1".to_string(),
            harness: "Claude Code".to_string(),
            account: Some("work".to_string()),
            model: Some("opus".to_string()),
            mode: Some("default".to_string()),
            cwd: "/tmp/project".to_string(),
            session: None,
            project: ProjectFacts {
                id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
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
        let serving = start(registry, host.voice(), None, None, None, None, None, None)
            .expect("the listener binds");
        (serving, hub, host)
    }

    fn running_with_work() -> (Serving, bus::Hub, bus::HostEnd) {
        let (hub, host) = bus::hub();
        let registry = Registry::new();
        registry.register(facts());
        let work = crate::work::Handle::new(crate::work::Work::open(Box::new(
            crate::store::memory::MemoryTaskStore::new(),
        )));
        let access = crate::mcp::WorkAccess {
            work,
            everyone: host.mailbox(ubiq_proto::bus::To::Everyone),
            wake: host.voice(),
        };
        let serving = start(
            registry,
            host.voice(),
            Some(access),
            None,
            None,
            None,
            None,
            None,
        )
        .expect("the listener binds");
        (serving, hub, host)
    }

    /// A listener wired to a real `Plans`, with one mission already on the board — a level is
    /// what lets it carry a plan at all. The `TempDir` rides along on [`crate::plan::Plans`]'s own
    /// tests' footing: dropped, its directory goes with it, and the store must outlive the test.
    #[allow(clippy::type_complexity)]
    fn running_with_plan() -> (
        Serving,
        bus::Hub,
        bus::HostEnd,
        ubiq_proto::ids::TaskId,
        crate::plan::Handle,
        tempfile::TempDir,
    ) {
        let (hub, host) = bus::hub();
        let registry = Registry::new();
        registry.register(facts());
        let project: ubiq_proto::ids::ProjectId = facts().project.id.parse().unwrap();
        let work = crate::work::Handle::new(crate::work::Work::open(Box::new(
            crate::store::memory::MemoryTaskStore::new(),
        )));
        let task = {
            let mut work = work.lock();
            let replies = work.create(project, "a mission".to_string(), None);
            let task = replies
                .iter()
                .find_map(|reply| match reply.message() {
                    Message::TaskCreated { task, .. } => Some(task.id),
                    _ => None,
                })
                .expect("the task was created");
            work.set_field(
                project,
                task,
                ubiq_proto::messages::TaskField::Level(Some(ubiq_proto::work::Level::Mission)),
            );
            task
        };
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::plan::FilePlanStore::new(dir.path().to_path_buf());
        let plans = crate::plan::Handle::new(crate::plan::Plans::open(store, work));
        let reach = PlanReach {
            plans: plans.clone(),
            everyone: host.mailbox(ubiq_proto::bus::To::Everyone),
        };
        let serving = start(
            registry,
            host.voice(),
            None,
            Some(reach),
            None,
            None,
            None,
            None,
        )
        .expect("the listener binds");
        (serving, hub, host, task, plans, dir)
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
    fn the_task_server_lists_creates_searches_and_deletes() {
        let (serving, _hub, _host) = running_with_work();
        let url = url(&serving, KEY, "manage-ubiq-tasks");

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
        assert_eq!(
            names,
            [
                "overview",
                "search_tasks",
                "list_tags",
                "create_tag",
                "create_task",
                "update_task",
                "delete_task",
                "add_todo",
                "update_todo",
                "delete_todo",
                "get_task",
                "add_comment",
            ]
        );

        let created = answered(&call(
            &url,
            "create_task",
            json!({
                "title": "Name the events",
                "status": "ready",
                "priority": "high",
                "complexity": "medium",
                "assigned_to": "ada",
                "labels": ["urgent"],
            }),
        ));
        assert_eq!(created["task"]["title"], "Name the events");
        assert_eq!(created["task"]["status"], "ready");
        assert_eq!(created["task"]["priority"], "high");
        assert_eq!(created["task"]["complexity"], "medium");
        assert_eq!(created["task"]["assigned_to"], "ada");
        assert_eq!(created["task"]["labels"][0]["name"], "urgent");
        let task_id = created["task"]["id"].as_str().unwrap().to_string();

        let overview = answered(&call(&url, "overview", json!({})));
        // One column per status, counted off `Status::all()` rather than written out here, so a
        // status added to the board does not fail a test about the overview.
        assert_eq!(
            overview["columns"].as_array().unwrap().len(),
            ubiq_proto::work::Status::all().len()
        );
        let ready = overview["columns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|column| column["status"] == "ready")
            .unwrap();
        assert_eq!(ready["total"], 1);
        assert_eq!(ready["tasks"][0]["title"], "Name the events");

        let filtered = answered(&call(
            &url,
            "overview",
            json!({"categories": ["backlog", "done"]}),
        ));
        assert_eq!(filtered["columns"].as_array().unwrap().len(), 2);
        assert_eq!(filtered["columns"][0]["total"], 0);

        let found = answered(&call(
            &url,
            "search_tasks",
            json!({"text": "events", "statuses": ["ready"], "labels": ["urgent"]}),
        ));
        assert_eq!(found["total"], 1);
        assert_eq!(found["tasks"][0]["id"], task_id);

        let tagged = answered(&call(&url, "create_tag", json!({"name": "later"})));
        assert_eq!(tagged["tag"]["name"], "later");
        let tags = answered(&call(&url, "list_tags", json!({})));
        let tag_names: Vec<&str> = tags["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tag| tag["name"].as_str().unwrap())
            .collect();
        assert!(tag_names.contains(&"urgent"));
        assert!(tag_names.contains(&"later"));

        let patched = answered(&call(
            &url,
            "update_task",
            json!({
                "task_id": task_id,
                "title": "Name them",
                "complexity": "high",
                "assigned_to": "  grace  ",
                "labels": ["urgent", "later"],
            }),
        ));
        assert_eq!(patched["task"]["title"], "Name them");
        assert_eq!(patched["task"]["complexity"], "high");
        // Trimmed on the way in, the way a key is.
        assert_eq!(patched["task"]["assigned_to"], "grace");
        assert_eq!(patched["task"]["labels"].as_array().unwrap().len(), 2);
        assert_eq!(patched["changed"], true);

        let ignored = answered(&call(
            &url,
            "update_task",
            json!({"task_id": task_id, "title": null, "priority": null}),
        ));
        assert_eq!(ignored["task"]["title"], "Name them");
        assert_eq!(ignored["changed"], false);

        let todo = answered(&call(
            &url,
            "add_todo",
            json!({"task_id": task_id, "title": "write the list"}),
        ));
        assert_eq!(todo["todo"]["title"], "write the list");
        assert_eq!(todo["todo"]["done"], false);
        let todo_id = todo["todo"]["id"].as_str().unwrap().to_string();

        let ticked = answered(&call(
            &url,
            "update_todo",
            json!({"task_id": task_id, "todo_id": todo_id, "done": true}),
        ));
        assert_eq!(ticked["todo"]["done"], true);
        assert_eq!(ticked["changed"], true);

        let deleted_todo = answered(&call(
            &url,
            "delete_todo",
            json!({"task_id": task_id, "todo_id": todo_id}),
        ));
        assert_eq!(deleted_todo["deleted"], true);

        let noted = answered(&call(
            &url,
            "add_comment",
            json!({"task_id": task_id, "text": "ship it"}),
        ));
        assert_eq!(noted["comment"]["text"], "ship it");
        assert_eq!(noted["comment"]["author"], "agent");

        let got = answered(&call(&url, "get_task", json!({"task_id": task_id})));
        assert_eq!(got["task"]["comments"].as_array().unwrap().len(), 1);
        assert_eq!(got["task"]["comments"][0]["author"], "agent");

        let deleted = answered(&call(&url, "delete_task", json!({"task_id": task_id})));
        assert_eq!(deleted["deleted"], true);

        let empty = answered(&call(&url, "search_tasks", json!({"text": "Name"})));
        assert_eq!(empty["total"], 0);
    }

    #[test]
    fn the_use_task_server_searches_gets_moves_and_comments() {
        let (serving, _hub, _host) = running_with_work();
        let manage = url(&serving, KEY, "manage-ubiq-tasks");
        let url = url(&serving, KEY, "use-task");

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
        assert_eq!(
            names,
            [
                "search_tasks",
                "get_task",
                "change_state",
                "add_comment",
                "add_todo",
                "update_todo",
                "delete_todo",
            ]
        );

        let created = answered(&call(
            &manage,
            "create_task",
            json!({"title": "Ship the board", "status": "backlog"}),
        ));
        let task_id = created["task"]["id"].as_str().unwrap().to_string();

        let found = answered(&call(&url, "search_tasks", json!({"text": "board"})));
        assert_eq!(found["total"], 1);

        let moved = answered(&call(
            &url,
            "change_state",
            json!({"task_id": task_id, "status": "in progress"}),
        ));
        assert_eq!(moved["task"]["status"], "in progress");
        assert_eq!(moved["changed"], true);

        let noted = answered(&call(
            &url,
            "add_comment",
            json!({"task_id": task_id, "text": "picked up"}),
        ));
        assert_eq!(noted["comment"]["author"], "agent");

        let got = answered(&call(&url, "get_task", json!({"task_id": task_id})));
        assert_eq!(got["task"]["status"], "in progress");
        assert_eq!(got["task"]["comments"][0]["text"], "picked up");

        let todo = answered(&call(
            &url,
            "add_todo",
            json!({"task_id": task_id, "title": "write the list"}),
        ));
        assert_eq!(todo["todo"]["title"], "write the list");
        let todo_id = todo["todo"]["id"].as_str().unwrap().to_string();

        let ticked = answered(&call(
            &url,
            "update_todo",
            json!({"task_id": task_id, "todo_id": todo_id, "done": true}),
        ));
        assert_eq!(ticked["todo"]["done"], true);

        let deleted_todo = answered(&call(
            &url,
            "delete_todo",
            json!({"task_id": task_id, "todo_id": todo_id}),
        ));
        assert_eq!(deleted_todo["deleted"], true);
    }

    #[test]
    fn a_task_tool_without_a_board_fails_in_band() {
        let (serving, _hub, _host) = running();
        let response = call(
            &url(&serving, KEY, "manage-ubiq-tasks"),
            "overview",
            json!({}),
        );
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no task board")
        );
    }

    #[test]
    fn the_plan_server_reads_writes_lists_replies_and_resolves_annotations() {
        let (serving, _hub, _host, task_id, plans, _dir) = running_with_plan();
        let url = url(&serving, KEY, "ubiq-plan");

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
        assert_eq!(
            names,
            [
                "read_plan",
                "write_plan",
                "plan_changes",
                "list_annotations",
                "reply_annotation",
                "resolve_annotation",
            ]
        );

        let task_id = task_id.to_string();

        let empty = answered(&call(&url, "read_plan", json!({"task_id": task_id})));
        assert_eq!(empty["body"], "");
        assert_eq!(
            empty["revision"], 0,
            "a plan nobody has written stands at revision 0",
        );

        let written = answered(&call(
            &url,
            "write_plan",
            json!({"task_id": task_id, "body": "# Plan\n\nStep one.\n\nStep two."}),
        ));
        assert_eq!(written["body"], "# Plan\n\nStep one.\n\nStep two.");
        assert_eq!(written["revision"], 1);

        // No annotation yet: the list comes back empty rather than erroring.
        let none_yet = answered(&call(&url, "list_annotations", json!({"task_id": task_id})));
        assert_eq!(none_yet["annotations"].as_array().unwrap().len(), 0);

        // The block ids are the host's, assigned on save — get them through the plan store the
        // way an interface would only ever see through the (not yet built) UI panel. The test
        // reaches in through the same `Plans` handle the tool used, on `plan/mod.rs`'s own
        // footing, since annotating one is not itself a slice-4 tool.
        let block = {
            let (blocks, _) = plans
                .lock()
                .annotation_list(&crate::plan::Target::plan(
                    facts().project.id.parse().unwrap(),
                    task_id.parse().unwrap(),
                ))
                .expect("the plan reads");
            blocks[1].id
        };
        let annotation_id = {
            let mut plan_lock = plans.lock();
            let replies = plan_lock.annotate(
                &crate::plan::Target::plan(
                    facts().project.id.parse().unwrap(),
                    task_id.parse().unwrap(),
                ),
                block,
                Some("Step one".to_string()),
                ubiq_proto::work::CommentAuthor::User,
                "which one?".to_string(),
            );
            replies
                .iter()
                .find_map(|reply| match reply.message() {
                    Message::PlanAnnotations { annotations, .. } => {
                        Some(annotations[0].id.to_string())
                    }
                    _ => None,
                })
                .expect("the annotation was created")
        };

        let listed = answered(&call(&url, "list_annotations", json!({"task_id": task_id})));
        let annotations = listed["annotations"].as_array().unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0]["state"], "open");
        assert_eq!(annotations[0]["orphaned"], false);
        assert_eq!(annotations[0]["quote"], "Step one");
        assert_eq!(annotations[0]["block_text"], "Step one.");
        assert_eq!(annotations[0]["thread"][0]["text"], "which one?");
        assert_eq!(annotations[0]["thread"][0]["author"], "user");

        let replied = answered(&call(
            &url,
            "reply_annotation",
            json!({"task_id": task_id, "annotation_id": annotation_id, "text": "the first one"}),
        ));
        assert_eq!(replied["thread"].as_array().unwrap().len(), 2);
        assert_eq!(replied["thread"][1]["author"], "agent");
        assert_eq!(replied["thread"][1]["text"], "the first one");

        let resolved = answered(&call(
            &url,
            "resolve_annotation",
            json!({"task_id": task_id, "annotation_id": annotation_id}),
        ));
        assert_eq!(resolved["state"], "resolved");

        // Resolved is excluded by default, and comes back with `include_resolved`.
        let open_only = answered(&call(&url, "list_annotations", json!({"task_id": task_id})));
        assert_eq!(open_only["annotations"].as_array().unwrap().len(), 0);
        let with_resolved = answered(&call(
            &url,
            "list_annotations",
            json!({"task_id": task_id, "include_resolved": true}),
        ));
        assert_eq!(with_resolved["annotations"].as_array().unwrap().len(), 1);

        let reopened = answered(&call(
            &url,
            "resolve_annotation",
            json!({"task_id": task_id, "annotation_id": annotation_id, "resolved": false}),
        ));
        assert_eq!(reopened["state"], "open");

        // Orphaning is one-way and visible: dropping step two's own block from the plan flags the
        // step-two comment, not the one just re-tested above, which stays anchored.
        let orphan_id = {
            let mut plan_lock = plans.lock();
            let block = {
                let (blocks, _) = plan_lock
                    .annotation_list(&crate::plan::Target::plan(
                        facts().project.id.parse().unwrap(),
                        task_id.parse().unwrap(),
                    ))
                    .unwrap();
                blocks[2].id
            };
            let replies = plan_lock.annotate(
                &crate::plan::Target::plan(
                    facts().project.id.parse().unwrap(),
                    task_id.parse().unwrap(),
                ),
                block,
                None,
                ubiq_proto::work::CommentAuthor::User,
                "about step two".to_string(),
            );
            replies
                .iter()
                .find_map(|reply| match reply.message() {
                    Message::PlanAnnotations { annotations, .. } => Some(
                        annotations
                            .iter()
                            .find(|annotation| annotation.block_id == block)
                            .unwrap()
                            .id
                            .to_string(),
                    ),
                    _ => None,
                })
                .expect("the second annotation was created")
        };
        answered(&call(
            &url,
            "write_plan",
            json!({"task_id": task_id, "body": "# Plan\n\nStep one."}),
        ));
        let after_edit = answered(&call(
            &url,
            "list_annotations",
            json!({"task_id": task_id, "include_resolved": true}),
        ));
        let orphaned = after_edit["annotations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|annotation| annotation["id"] == orphan_id)
            .unwrap();
        assert_eq!(orphaned["orphaned"], true);
        assert_eq!(orphaned["block_text"], Value::Null);
    }

    /// The edit-provenance half of the same server: the agent writes, a person edits two places
    /// through the interface's own path, and `plan_changes` names exactly those two — defaulting
    /// its watermark to the agent's own last write, with no revision passed in.
    #[test]
    fn plan_changes_reports_where_a_human_edited_what_this_agent_wrote() {
        let (serving, _hub, _host, task_id, plans, _dir) = running_with_plan();
        let url = url(&serving, KEY, "ubiq-plan");
        let task_id = task_id.to_string();
        let project: ubiq_proto::ids::ProjectId = facts().project.id.parse().unwrap();

        let written = answered(&call(
            &url,
            "write_plan",
            json!({"task_id": task_id, "body": "# Plan\n\nStep one.\n\nStep two."}),
        ));
        assert_eq!(written["revision"], 1);

        // Nothing has happened since: the ordinary answer is an empty one, not an error.
        let quiet = answered(&call(&url, "plan_changes", json!({"task_id": task_id})));
        assert_eq!(quiet["regions"].as_array().unwrap().len(), 0);
        assert_eq!(quiet["stats"]["human_revisions"], 0);

        // A person edits two separate lines, through the path a `SavePlan` off the bus takes.
        plans.lock().save(
            &crate::plan::Target::plan(project, task_id.parse().unwrap()),
            "# Plan\n\nStep one, revised.\n\nStep two, also revised.".to_string(),
            &crate::plan::Saver::human(),
            None,
        );

        let changed = answered(&call(&url, "plan_changes", json!({"task_id": task_id})));
        assert_eq!(
            changed["since_revision"], 1,
            "defaulted to this agent's own last write",
        );
        assert_eq!(changed["revision"], 2);

        let regions = changed["regions"].as_array().unwrap();
        assert_eq!(regions.len(), 2, "exactly the two places the human touched");
        assert_eq!(regions[0]["first_line"], 3);
        assert_eq!(regions[0]["last_line"], 3);
        assert_eq!(regions[0]["origin"], "human");
        assert_eq!(regions[0]["text"], "Step one, revised.");
        assert_eq!(
            regions[0]["block_text"], "Step one, revised.",
            "the block is named, never a bare id",
        );
        assert_eq!(regions[1]["first_line"], 5);
        assert_eq!(regions[1]["text"], "Step two, also revised.");

        assert_eq!(changed["stats"]["lines_modified"], 2);
        assert_eq!(changed["stats"]["lines_added"], 0);
        assert_eq!(changed["stats"]["lines_removed"], 0);
        assert_eq!(changed["stats"]["blocks_touched"], 2);
        assert_eq!(changed["stats"]["human_revisions"], 1);
        assert_eq!(changed["stats"]["agent_revisions"], 0);

        // An explicit watermark of 0 takes in the agent's own opening write as well.
        let everything = answered(&call(
            &url,
            "plan_changes",
            json!({"task_id": task_id, "since_revision": 0}),
        ));
        assert_eq!(everything["stats"]["agent_revisions"], 1);
        assert_eq!(everything["stats"]["human_revisions"], 1);
        assert!(
            everything["regions"].as_array().unwrap().len() >= 2,
            "the agent's own untouched lines are in the window too",
        );
    }

    #[test]
    fn a_plan_tool_without_a_plan_store_fails_in_band() {
        let (serving, _hub, _host) = running();
        let response = call(&url(&serving, KEY, "ubiq-plan"), "read_plan", json!({}));
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no plan store")
        );
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

    /// A knowledge base with one read-only folder source and one wiki, on a config root of its
    /// own.
    ///
    /// The list is written through [`crate::kb::Kb::set_sources`] rather than by hand, so the test
    /// reads back exactly what a settings form would have saved — and the `Kb` it was written
    /// through is the one the listener holds, which is what the `Arc` in [`crate::mcp::KbReach`]
    /// exists for.
    struct KbFixture {
        /// Held for the test's life: dropping it takes the config root, and with it the wiki.
        _root: tempfile::TempDir,
        docs: tempfile::TempDir,
        kb: std::sync::Arc<crate::kb::Kb>,
        project: ubiq_proto::ids::ProjectId,
        folder: ubiq_proto::ids::KbSourceId,
    }

    impl KbFixture {
        fn wiki_path(&self, rel: &str) -> std::path::PathBuf {
            self._root
                .path()
                .join("projects")
                .join(self.project.to_string())
                .join("wiki")
                .join(rel)
        }
    }

    fn running_with_kb() -> (Serving, bus::Hub, bus::HostEnd, KbFixture) {
        use ubiq_proto::ids::{KbSourceId, ProjectId};
        use ubiq_proto::kb::{KbAccess, KbOrigin, KbSource};

        let (hub, host) = bus::hub();
        let registry = Registry::new();
        registry.register(facts());

        let root = tempfile::TempDir::new().unwrap();
        let docs = tempfile::TempDir::new().unwrap();
        std::fs::write(docs.path().join("guide.md"), b"how it works").unwrap();
        std::fs::write(docs.path().join("main.rs"), b"fn main() {}").unwrap();
        std::fs::create_dir(docs.path().join("deep")).unwrap();
        std::fs::write(docs.path().join("deep/notes.md"), b"deeper").unwrap();

        let kb = std::sync::Arc::new(crate::kb::Kb::new(root.path().to_path_buf()));
        let project: ProjectId = facts().project.id.parse().unwrap();
        let folder = KbSourceId::generate();
        kb.set_sources(
            project,
            vec![
                KbSource {
                    id: folder,
                    name: "Docs".to_string(),
                    origin: KbOrigin::Folder {
                        path: docs.path().display().to_string(),
                    },
                    filter: "*.md".to_string(),
                    access: KbAccess::ReadOnly,
                },
                KbSource {
                    id: KbSourceId::generate(),
                    name: "Wiki".to_string(),
                    origin: KbOrigin::Internal,
                    filter: String::new(),
                    access: KbAccess::ReadOnly,
                },
            ],
            host.mailbox(ubiq_proto::bus::To::Everyone),
            std::path::Path::new("/tmp/project"),
        );

        let reach = crate::mcp::KbReach {
            kb: kb.clone(),
            everyone: host.mailbox(ubiq_proto::bus::To::Everyone),
        };
        let serving = start(
            registry,
            host.voice(),
            None,
            None,
            None,
            Some(reach),
            None,
            None,
        )
        .expect("the listener binds");
        (
            serving,
            hub,
            host,
            KbFixture {
                _root: root,
                docs,
                kb,
                project,
                folder,
            },
        )
    }

    #[test]
    fn the_kb_server_lists_a_projects_sources() {
        let (serving, _hub, _host, fixture) = running_with_kb();
        let url = url(&serving, KEY, "ubiq-kb");

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
        assert_eq!(
            names,
            [
                "list_kb_sources",
                "list_kb_documents",
                "read_kb_document",
                "write_kb_document",
                "create_kb_entry",
                "rename_kb_entry",
                "delete_kb_entry",
                "sync_kb_source",
            ]
        );

        let sources = answered(&call(&url, "list_kb_sources", json!({})));
        let rows = sources["sources"].as_array().unwrap();
        assert_eq!(rows.len(), 2);

        assert_eq!(rows[0]["name"], "Docs");
        assert_eq!(rows[0]["id"], fixture.folder.to_string());
        assert_eq!(rows[0]["kind"], "folder");
        assert_eq!(rows[0]["writable"], false);
        assert_eq!(rows[0]["filter"], "*.md");
        assert_eq!(rows[0]["state"]["state"], "ready");
        assert_eq!(rows[0]["path"], fixture.docs.path().display().to_string());

        // A wiki is Ubiq's own: always writable, nothing to point at, ready the moment it is
        // asked about — and its directory is never answered.
        assert_eq!(rows[1]["name"], "Wiki");
        assert_eq!(rows[1]["kind"], "wiki");
        assert_eq!(rows[1]["writable"], true);
        assert_eq!(rows[1]["state"]["state"], "ready");
        assert!(rows[1].get("path").is_none());
        assert!(rows[1].get("url").is_none());

        // A folder and a wiki have nothing to fetch, and that is an answer rather than an error.
        let synced = answered(&call(&url, "sync_kb_source", json!({"source": "Docs"})));
        assert_eq!(synced["started"], false);
        assert_eq!(synced["kind"], "folder");
    }

    #[test]
    fn a_kb_address_names_a_source_by_name_or_by_id() {
        let (serving, _hub, host, fixture) = running_with_kb();
        let url = url(&serving, KEY, "ubiq-kb");

        // A bare source is its own top level, and the filter is honoured because it is what the
        // source shows: `main.rs` is not offered, `deep` is, because a folder is never filtered.
        let top = answered(&call(&url, "list_kb_documents", json!({"path": "Docs"})));
        let names: Vec<&str> = top["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["deep", "guide.md"]);
        assert_eq!(top["entries"][0]["is_folder"], true);
        assert_eq!(top["entries"][0]["path"], "Docs/deep");
        assert_eq!(top["entries"][1]["path"], "Docs/guide.md");

        let nested = answered(&call(
            &url,
            "list_kb_documents",
            json!({"path": "Docs/deep"}),
        ));
        assert_eq!(nested["entries"][0]["path"], "Docs/deep/notes.md");

        // The id works where the name does, and the answer names the source the way
        // `list_kb_sources` did, whichever was typed.
        let by_id = answered(&call(
            &url,
            "read_kb_document",
            json!({"path": format!("{}/guide.md", fixture.folder)}),
        ));
        assert_eq!(by_id["contents"], "how it works");
        assert_eq!(by_id["path"], "Docs/guide.md");

        let unknown = call(&url, "list_kb_documents", json!({"path": "Nope/x.md"}));
        assert_eq!(unknown["result"]["isError"], true);
        let said = unknown["result"]["content"][0]["text"].as_str().unwrap();
        assert!(said.contains("not a knowledge-base source"), "{said}");
        assert!(said.contains("Docs"), "{said}");

        // Two sources may share a name — nothing stops the user — so the tools say which two by
        // id rather than picking one.
        use ubiq_proto::ids::KbSourceId;
        use ubiq_proto::kb::{KbAccess, KbOrigin, KbSource};
        let twin = KbSourceId::generate();
        fixture.kb.set_sources(
            fixture.project,
            vec![
                KbSource {
                    id: fixture.folder,
                    name: "Docs".to_string(),
                    origin: KbOrigin::Folder {
                        path: fixture.docs.path().display().to_string(),
                    },
                    filter: String::new(),
                    access: KbAccess::ReadOnly,
                },
                KbSource {
                    id: twin,
                    name: "docs".to_string(),
                    origin: KbOrigin::Internal,
                    filter: String::new(),
                    access: KbAccess::ReadOnly,
                },
            ],
            host.mailbox(ubiq_proto::bus::To::Everyone),
            std::path::Path::new("/tmp/project"),
        );

        let ambiguous = call(&url, "list_kb_documents", json!({"path": "Docs"}));
        assert_eq!(ambiguous["result"]["isError"], true);
        let said = ambiguous["result"]["content"][0]["text"].as_str().unwrap();
        assert!(said.contains("names 2 knowledge-base sources"), "{said}");
        assert!(said.contains(&fixture.folder.to_string()), "{said}");
        assert!(said.contains(&twin.to_string()), "{said}");

        // The id still resolves the one the name no longer can.
        let resolved = answered(&call(
            &url,
            "list_kb_documents",
            json!({"path": twin.to_string()}),
        ));
        assert_eq!(resolved["entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_write_to_a_read_only_kb_source_is_refused_in_band() {
        let (serving, _hub, _host, fixture) = running_with_kb();
        let url = url(&serving, KEY, "ubiq-kb");

        let refused = call(
            &url,
            "write_kb_document",
            json!({"path": "Docs/guide.md", "contents": "rewritten"}),
        );
        assert!(refused.get("error").is_none());
        assert_eq!(refused["result"]["isError"], true);
        let said = refused["result"]["content"][0]["text"].as_str().unwrap();
        assert!(said.contains("read-only"), "{said}");
        assert!(said.contains("Docs/guide.md"), "{said}");

        // The refusal is `kb::ops`', before anything touched disk.
        assert_eq!(
            std::fs::read_to_string(fixture.docs.path().join("guide.md")).unwrap(),
            "how it works"
        );
    }

    #[test]
    fn a_write_to_the_wiki_lands_on_disk_and_is_announced() {
        let (serving, hub, _host, fixture) = running_with_kb();
        let url = url(&serving, KEY, "ubiq-kb");
        // A window has to be attached for a broadcast to reach anybody.
        let window = hub.connect();

        let created = answered(&call(
            &url,
            "create_kb_entry",
            json!({"path": "Wiki/plans", "kind": "folder"}),
        ));
        assert_eq!(created["created"], true);
        assert!(fixture.wiki_path("plans").is_dir());
        match window.from_host().recv_timeout(TIMEOUT) {
            Ok(Message::KbChanged { rel_path, .. }) => assert_eq!(rel_path, ""),
            other => panic!("expected KbChanged, got {other:?}"),
        }

        let written = answered(&call(
            &url,
            "write_kb_document",
            json!({"path": "Wiki/plans/ship.md", "contents": "ship it"}),
        ));
        assert_eq!(written["written"], true);
        assert_eq!(written["path"], "Wiki/plans/ship.md");
        assert_eq!(
            std::fs::read_to_string(fixture.wiki_path("plans/ship.md")).unwrap(),
            "ship it"
        );

        // The parent is what an open panel re-lists, so that is what is named.
        match window.from_host().recv_timeout(TIMEOUT) {
            Ok(Message::KbChanged {
                project_id,
                rel_path,
                ..
            }) => {
                assert_eq!(project_id, fixture.project);
                assert_eq!(rel_path, "plans");
            }
            other => panic!("expected KbChanged, got {other:?}"),
        }

        let read = answered(&call(
            &url,
            "read_kb_document",
            json!({"path": "Wiki/plans/ship.md"}),
        ));
        assert_eq!(read["contents"], "ship it");

        let renamed = answered(&call(
            &url,
            "rename_kb_entry",
            json!({"path": "Wiki/plans/ship.md", "new_name": "shipped.md"}),
        ));
        assert_eq!(renamed["path"], "Wiki/plans/shipped.md");
        assert!(fixture.wiki_path("plans/shipped.md").is_file());

        let deleted = answered(&call(
            &url,
            "delete_kb_entry",
            json!({"path": "Wiki/plans"}),
        ));
        assert_eq!(deleted["deleted"], true);
        assert!(!fixture.wiki_path("plans").exists());
    }

    #[test]
    fn a_kb_tool_without_a_knowledge_base_fails_in_band() {
        let (serving, _hub, _host) = running();
        let response = call(&url(&serving, KEY, "ubiq-kb"), "list_kb_sources", json!({}));
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no knowledge base")
        );
    }

    /// The parking tool, end to end — and the thing `D138` is actually about: while one agent's
    /// ask is waiting for a person, another agent's ordinary tool call is answered as usual.
    #[test]
    fn an_ask_parks_without_holding_up_the_listener() {
        let (hub, host) = bus::hub();
        let asking = ubiq_proto::work::AgentId::generate();
        let registry = Registry::new();
        registry.register(AgentFacts {
            key: asking.to_string(),
            ..facts()
        });
        registry.register(facts());
        let asks = Arc::new(crate::ask::Asks::new());
        let serving = start(
            registry,
            host.voice(),
            None,
            None,
            None,
            None,
            None,
            Some(crate::mcp::AskReach {
                asks: Arc::clone(&asks),
                armed: Arc::new(crate::armed::Armed::new()),
            }),
        )
        .expect("the listener binds");

        let asked = url(&serving, &asking.to_string(), "ubiq-ask");
        let calling = std::thread::spawn(move || {
            call(
                &asked,
                "ask_user_question",
                json!({"questions": [{
                    "question": "Which way?",
                    "header": "Direction",
                    "options": [{"label": "Left"}, {"label": "Right"}],
                    "multiSelect": false,
                }]}),
            )
        });

        // The ask reaches the host, which is where the coordinator addresses it from.
        let ask_id = loop {
            match host.recv_timeout(TIMEOUT).expect("the ask is raised") {
                bus::FromClient::Said {
                    message:
                        Message::AskUser {
                            ask_id, agent_id, ..
                        },
                    ..
                } => {
                    assert_eq!(agent_id, asking);
                    break ask_id;
                }
                _ => continue,
            }
        };

        // The call is parked, and the listener is free: another agent's tool answers now.
        assert_eq!(asks.len(), 1);
        let meanwhile = call(&url(&serving, KEY, "project-info"), "whoami", json!({}));
        assert_eq!(meanwhile["result"]["isError"], false);

        assert!(asks.answer(
            ask_id,
            ubiq_proto::ask::AskOutcome::Answered(vec![ubiq_proto::ask::AskAnswer {
                question: 0,
                chosen: vec!["Right".to_string()],
                other: None,
                notes: None,
            }])
        ));
        let response = calling.join().expect("the calling thread");
        assert_eq!(response["result"]["isError"], false);
        let result = answered(&response);
        assert_eq!(result["answered"][0]["chosen"][0], "Right");
        assert_eq!(result["summary"], "Direction: Right");
        drop(hub);
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
