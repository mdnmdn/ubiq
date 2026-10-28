//! The `ubiq-browser` server: an agent driving the project's internal browser tabs.
//!
//! The host holds no web view, so every tool is a round trip to a window: the call files a row in
//! [`crate::browser::Commands`], says [`Message::BrowserCommand`] through the host's own voice —
//! the coordinator addresses it to the window that owns the calling agent, or one that has the
//! project open — and waits for that window's [`Message::BrowserReply`], at most
//! [`BROWSER_TIMEOUT_SECS`]. Always on a thread [`super::server`] spawned for the call, never the
//! listener's own.
//!
//! The window's answer is JSON text; it is handed to the model as JSON when it parses, as text
//! when it does not. An `Err` from the window is the tool's in-band error, as written.
//!
//! `browser_wait_load` is the one tool with no command of its own: it polls
//! [`BrowserCmd::Info`] until the tab says it is no longer loading.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use ubiq_proto::browser::{BROWSER_TIMEOUT_SECS, BrowserCmd, BrowserContentFormat};
use ubiq_proto::bus::Voice;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;

use super::BrowserReach;
use super::registry::AgentFacts;

/// The most browser calls served at once; the next is refused as busy.
pub const MAX_CALLS: usize = 8;

/// A claimed place among the [`MAX_CALLS`]; released on drop.
pub struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl BrowserReach {
    pub fn new(commands: Arc<crate::browser::Commands>) -> Self {
        Self {
            commands,
            inflight: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// A place for one more call, or `None` when [`MAX_CALLS`] are running.
    pub fn claim(&self) -> Option<Slot> {
        let taken = self.inflight.fetch_add(1, Ordering::SeqCst);
        if taken >= MAX_CALLS {
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Slot(Arc::clone(&self.inflight)))
    }
}

/// How often `browser_wait_load` asks again.
const POLL: Duration = Duration::from_millis(250);

/// `browser_wait_load`'s default and ceiling.
const WAIT_DEFAULT_MS: u64 = 30_000;
const WAIT_MAX_MS: u64 = 120_000;

pub fn call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    voice: &Voice,
    reach: &BrowserReach,
) -> Result<Value, String> {
    let project_id: ProjectId = facts
        .project
        .id
        .parse()
        .map_err(|_| "this agent is not in a project with a browser".to_string())?;
    let tab = || optional(arguments, "tab");
    let cmd = match tool {
        "browser_list_tabs" => BrowserCmd::ListTabs,
        "browser_open" => BrowserCmd::Open {
            url: required(arguments, "url")?,
            new_tab: arguments
                .get("new_tab")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "browser_navigate" => BrowserCmd::Navigate {
            tab: tab(),
            url: required(arguments, "url")?,
        },
        "browser_back" => BrowserCmd::Back { tab: tab() },
        "browser_forward" => BrowserCmd::Forward { tab: tab() },
        "browser_reload" => BrowserCmd::Reload { tab: tab() },
        "browser_stop" => BrowserCmd::Stop { tab: tab() },
        "browser_close" => BrowserCmd::Close { tab: tab() },
        "browser_get_content" => BrowserCmd::GetContent {
            tab: tab(),
            format: match arguments.get("format").and_then(Value::as_str) {
                None | Some("text") => BrowserContentFormat::Text,
                Some("html") => BrowserContentFormat::Html,
                Some(other) => {
                    return Err(format!("unknown format '{other}': use text or html"));
                }
            },
        },
        "browser_eval" => BrowserCmd::Eval {
            tab: tab(),
            script: required(arguments, "script")?,
        },
        "browser_info" => BrowserCmd::Info { tab: tab() },
        "browser_wait_load" => return wait_load(arguments, facts, project_id, voice, reach),
        _ => return Err(format!("unknown tool: ubiq-browser/{tool}")),
    };
    send(
        facts,
        project_id,
        cmd,
        voice,
        reach,
        Duration::from_secs(BROWSER_TIMEOUT_SECS),
    )
}

/// One round trip: file the row, say the command, wait for the window at most `bound`.
fn send(
    facts: &AgentFacts,
    project_id: ProjectId,
    cmd: BrowserCmd,
    voice: &Voice,
    reach: &BrowserReach,
    bound: Duration,
) -> Result<Value, String> {
    let name = cmd.name();
    let (req_id, waiting) = reach.commands.raise(&facts.key);
    tracing::debug!(agent = %facts.key, req = %req_id, cmd = name, "browser command");
    voice.say(Message::BrowserCommand {
        project_id,
        req_id,
        cmd,
    });
    let text = reach.commands.wait(req_id, &waiting, bound)?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// Poll `Info` until the tab is no longer loading, or the caller's bound passes.
fn wait_load(
    arguments: &Value,
    facts: &AgentFacts,
    project_id: ProjectId,
    voice: &Voice,
    reach: &BrowserReach,
) -> Result<Value, String> {
    let bound = arguments
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(WAIT_DEFAULT_MS)
        .clamp(100, WAIT_MAX_MS);
    let tab = optional(arguments, "tab");
    let started = Instant::now();
    let deadline = started + Duration::from_millis(bound);
    loop {
        // Each poll waits no longer than what is left of the caller's bound, so the whole call
        // never overruns `timeout_ms` by more than a lock.
        let left = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(BROWSER_TIMEOUT_SECS));
        if left.is_zero() {
            return Err(format!("the page was still loading after {bound}ms"));
        }
        let mut info = send(
            facts,
            project_id,
            BrowserCmd::Info { tab: tab.clone() },
            voice,
            reach,
            left,
        )?;
        let loading = info
            .get("loading")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !loading {
            let waited = started.elapsed().as_millis() as u64;
            if let Some(object) = info.as_object_mut() {
                object.insert("waited_ms".to_string(), json!(waited));
            }
            return Ok(info);
        }
        if Instant::now() + POLL >= deadline {
            return Err(format!("the page was still loading after {bound}ms"));
        }
        std::thread::sleep(POLL);
    }
}

fn required(arguments: &Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("needs a non-empty '{key}' argument"))
}

fn optional(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}
