//! The coordinator: it starts harnesses, supervises them, and answers the bus.
//!
//! It runs on a thread of its own and shares nothing with the window but the channel pair in
//! [`ubiq_proto::bus`]. It renders nothing and has no opinion about layout or colour — everything here
//! is a pane ID, a pseudo-terminal and a process.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use ubiq_proto::assist::{AssistProvider, SuggestSubject};
use ubiq_proto::bus::{ClientId, FromClient, HostEnd, To};
use ubiq_proto::conversation::{
    ConfigCategory, ConfigChoice, ConfigOption, ConfigValue, ConvUpdate, StopReason,
};
use ubiq_proto::files::FileError;
use ubiq_proto::ids::{PaneId, ProjectId, SearchId, SessionId, SuggestId, ToolId};
use ubiq_proto::messages::{AgentPicks, CatalogueModel, Message, WorkspaceInfo};
use ubiq_proto::projects::{IndexLevel, ProjectHealth, Scope};
use ubiq_proto::stats::{HostStats, UsageRow};
use ubiq_proto::tools::{ListedTool, ToolDef};
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

use crate::agent::{Agents, ConverseOptions, PendingLogin};
use crate::assist::{self, Assist, providers::Providers};
use crate::cli_shortcut;
use crate::config::ConfigRoot;
use crate::connectors::{Answer, Connectors};
use crate::conversation::{ConvFlags, Conversation, UsageMeter};
use crate::conversation_record::{self, ConversationRecord};
use crate::files::{self, Files};
use crate::git::{self, Git};
use crate::health;
use crate::host_meta::{self, HostMeta};
use crate::notifications;
use crate::projects::Projects;
use crate::pty::{self, Pty};
use crate::reply::Reply;
use crate::repos::Repos;
use crate::search::{self, Search};
use crate::settings::Settings;
use crate::shells;
use crate::store::harness::{CachedModel, FileHarnessCache};
use crate::store::usage::Usage;
use crate::watch;
use crate::web_assets::WebAssets;
use crate::work::Work;

/// The geometry a pane starts at, before the emulator has measured its own bounds and said what it
/// really is. The harness is told the truth a frame later, by [`Message::TerminalResize`].
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// How long the run loop's wait may be while a conversation or a clone is live, so work that
/// finishes on another thread is collected promptly rather than only on the next unrelated
/// message. See the wait computation in `run` for why this is needed at all.
const CONVERSATION_POLL: Duration = Duration::from_millis(500);

/// How long one suggestion may take before it is answered with a failure instead. Generation
/// cannot be interrupted, so this bounds the wait rather than the work.
const SUGGEST_DEADLINE: Duration = Duration::from_secs(60);

/// How often a live run's login is reconciled with the account it was seeded from
/// (`Agents::sync_logins`). A harness refreshes its token on the order of hours, so this is not
/// a race being chased — it is a bound on how long a rotated-away credential may sit in a run
/// directory before every other agent on that account can see it. Half a minute of file reads
/// over a handful of small files is cheaper than the watcher the alternative would need.
const LOGIN_SYNC_EVERY: Duration = Duration::from_secs(30);

/// Gather what a subject needs and compose its prompt. Off the coordinator's thread, so the
/// repository read here is allowed to be slow.
fn gather(
    subject: &SuggestSubject,
    root: Option<&std::path::Path>,
    assist: &dyn Assist,
) -> Result<assist::Request, String> {
    match subject {
        SuggestSubject::CommitMessage { .. } => {
            let root = root.ok_or_else(|| "that project is not in the catalogue".to_string())?;
            let observation = git::observe(root, 0, true)
                .map_err(|error| format!("read the repository: {error:?}"))?;
            let entries = observation
                .tree
                .map(|tree| tree.entries)
                .filter(|entries| !entries.is_empty())
                .ok_or_else(|| "there is nothing changed here to name".to_string())?;
            Ok(assist::subject::commit_message(&entries, &assist.limits()))
        }
        // Nothing to gather: a check is about whether the provider answers at all, so its whole
        // material is the prompt the host owns.
        SuggestSubject::ProviderCheck { role, .. } => Ok(assist::subject::provider_check(*role)),
    }
}

/// Start the coordinator on its own thread. One per process, started before the first window: the
/// catalogue it will own is process-wide, and two of them would disagree about what exists.
///
/// It ends when the hub and every client have gone.
pub fn start(
    host: HostEnd,
    root: ConfigRoot,
    projects: Projects,
    work: Work,
    settings: Settings,
    pending: Vec<Reply>,
) {
    thread::Builder::new()
        .name("ubiq-coordinator".to_string())
        .spawn(move || Coordinator::new(host, root, projects, work, settings, pending).run())
        .expect("the coordinator thread");
}

struct Coordinator {
    host: HostEnd,
    /// Where everything is written down, and whether that is the usual place. Each window is told
    /// as it attaches, because the interface cannot look.
    root: ConfigRoot,
    /// The catalogue, the view state, and what is running in each project.
    projects: Projects,
    /// Each project's tasks, and the sessions and agents the two screens over the work draw.
    work: Work,
    /// Application settings: the Ui layer opaque, the Host layer parsed. Shared, because a
    /// connector flow on its own thread writes the same record — [`Settings::update_host`] is what
    /// makes that safe.
    settings: Arc<Settings>,
    /// The identities Ubiq holds at external services. Answers what it can from the settings blob
    /// on this thread; everything that touches a network runs as a flow of its own, for the same
    /// reason a search and a git status do.
    connectors: Connectors,
    /// Cloning a repository into a project, and the listings that find one. Holds a sender per
    /// clone and nothing else — every call it makes is on a thread of its own, for the same reason
    /// a connector flow is.
    repos: Repos,
    /// The vendor bundles a web panel needs, on disk in the shared workarea. Holds a row per fetch
    /// in flight and nothing else — the download is a thread of its own, for the same reason a
    /// clone is.
    web_assets: WebAssets,
    /// The directory the host reserves for the interface, told to each window as it attaches and
    /// never composed by it. Reserved once here, because it belongs to no project and outlives
    /// every window.
    shared_workarea: String,
    /// Which agent types can run here, and the composer that turns one into a launch.
    agents: Agents,
    /// The on-disk cache of what each harness answered about its own models/reasoning levels,
    /// keyed on the harness binary's own version string. `Arc` because the discovery thread
    /// cannot borrow `&self` — the same constraint that made model discovery a free function.
    catalogue: Arc<FileHarnessCache>,
    /// The thread that reads and writes a project's files. Nothing in the file family touches disk
    /// on this thread: a cold `read_dir` here would stall every pane's keystrokes behind it.
    files: Files,
    /// The thread that reads a project's repository. A status walk is seconds on a large tree, and
    /// seconds here would stall every pane behind it.
    git: Git,
    /// The thread that walks a project's files for content search. Long-running by nature, so it
    /// has its own queue: a search behind a slow one would stall every folder expand.
    search: Search,
    /// The full-text index, one open per project that keeps one.
    index: crate::index::Index,
    /// One live search per project. The flag means two things: a cancel request, set when a
    /// second search for the same project arrives or `CancelSearch` names this one; and "this
    /// search is over", set by the worker itself when it finishes, cancelled or not. `search_job`
    /// reaps entries where the flag is already set, the one place that mints them.
    active_searches: HashMap<ProjectId, (SearchId, Arc<AtomicBool>)>,
    /// The backend that writes a suggestion. Held rather than selected per request: a backend can
    /// mean an FFI handle and a loaded model, which is expensive enough to keep, where every other
    /// setting here is re-read from `self.settings.host()` at each use. Rebuilt when a
    /// `SetSettings` write changes the provider, which is the only thing that can change it.
    assist: Arc<dyn Assist>,
    /// The configured API providers: their records, their keys and their cached model lists. Held
    /// beside `assist` rather than inside it, because a provider exists whether or not it is the
    /// one a suggestion currently runs on — and because it is also what lends `select` a key.
    ai_providers: Providers,
    /// One flag per suggestion asked for, flipped by `CancelSuggest`. Reaped the way
    /// `active_searches` is: the worker sets the flag on its way out too, so an entry whose flag is
    /// already set is over and can be dropped when the next suggestion mints one.
    active_suggests: HashMap<SuggestId, Arc<AtomicBool>>,
    /// The bell's history and the mute rules over it. Held here rather than per window because a
    /// rule set in one window governs every window, and because the desktop must be told about an
    /// event once however many windows are open.
    notifications: notifications::Centre,
    /// One filesystem watch per window per open project. Keyed by both because a project is open
    /// in exactly one window and a window shows one project at a time — there is no
    /// `CloseProject` message, so replacing a client's entry when it opens another project is how
    /// the old watch stops.
    watchers: HashMap<(ClientId, ProjectId), watch::Watcher>,
    /// Anything the catalogue wanted said before a window existed to hear it — a corrupt store,
    /// most usefully. Delivered to the first window that attaches.
    pending: Vec<Reply>,
    /// Which project each pane belongs to, so a pane opening or ending changes a count the picker
    /// draws.
    pane_projects: HashMap<PaneId, ProjectId>,
    /// Which session each pane belongs to, so `stats()` can count live sessions. Ubiq's sense of
    /// session — a named grouping of panes — not the harness library's resumable conversation.
    pane_sessions: HashMap<PaneId, SessionId>,
    panes: HashMap<PaneId, Pty>,
    /// Which window owns which pane, recorded when the pane is spawned. This is the whole routing
    /// table: everything a pane emits goes to its owner, and nobody else may drive it.
    owners: HashMap<PaneId, ClientId>,
    /// Which pane each window says has focus. Exactly one per window — two attached windows have
    /// two focused panes, and neither is more focused than the other.
    focused: HashMap<ClientId, PaneId>,
    /// The agents being talked to, keyed by the id that multiplexes them onto the bus. One entry
    /// is one harness on one pump thread.
    conversations: HashMap<AgentId, Conversation>,
    /// Which project and window each conversation belongs to — the same routing table the panes
    /// have, for the same two reasons: a reply goes to the window that asked, and a project's
    /// count changes when one ends.
    conversation_owners: HashMap<AgentId, (ClientId, ProjectId)>,
    /// The launch recipe for every agent this window has asked for, kept for the agent's whole
    /// life rather than only until its first launch. Before a first [`Message::PromptAgent`] it is
    /// what P3's loader is waiting on; after `UnloadConversation` it is what
    /// `ResumeConversation` (or the next `PromptAgent`) relaunches from. Only
    /// [`Message::EndConversation`] removes an entry — a live agent's own row stays put beside its
    /// [`Self::conversations`] one, holding the picks (`chosen_model` and friends) a relaunch must
    /// not lose.
    pending_conversations: HashMap<AgentId, PendingConversation>,
    /// The logins running in a pane, keyed by it. Whether a login captured anything is only
    /// answerable once its process has exited, so what the answer needs is parked here until
    /// then — the same shape `active_searches` uses, and forgotten the same way.
    logins: HashMap<PaneId, PendingLogin>,
    /// When this coordinator started, which is what the stats screen calls uptime. Nothing else
    /// measured elapsed time before, so this is where it comes from.
    // ponytail: measured from the coordinator's thread rather than from `main`. The difference is
    // the few milliseconds spent finding the config root, which nobody reading "uptime 4m" can
    // perceive; move it to `main` and pass it in if a figure in milliseconds ever matters.
    started: Instant,
    /// When the logins were last reconciled with the accounts they were seeded from, which is
    /// what paces [`Self::sync_logins_due`] off a run loop that wakes twice a second.
    logins_synced: Instant,
    /// Every conversation started since this run began, including the ones that have since ended.
    /// A counter rather than a length, because what it counts is gone. A relaunch of the same
    /// agent counts again, deliberately: it is a second harness process on a second pump thread,
    /// which is what the figure beside "live" is comparing against.
    agents_this_run: usize,
    /// What the agents have spent. `None` when the database could not be opened — an unwritable
    /// config root costs the user their token history, never their session.
    usage: Option<Arc<Usage>>,
    /// Machine facts for `HostInfo` / `Stats`, refreshed by the metadata sampler thread every
    /// twenty seconds (`crate::host_meta`). Shared rather than copied because the sampler owns
    /// the write half; both readers take the lock for one clone and never hold it.
    meta: Arc<Mutex<HostMeta>>,
    /// The loopback listener behind Ubiq's own MCP servers, held for the life of the process the
    /// way the worker handles above are: dropping it stops the serving thread, so a host that
    /// goes takes its port with it rather than leaving one answering for agents that are gone.
    /// `None` when the port would not bind, which is a build with no injected servers and not a
    /// failure — the warning was said at startup.
    #[allow(
        dead_code,
        reason = "held for its Drop; the URL is read once, at construction"
    )]
    mcp: Option<crate::mcp::Serving>,
}

/// A conversation the window asked for and the harness has not yet answered — registered so the
/// UI can draw it with a loader while its harness's models are discovered, instead of starting it.
/// [`Coordinator::conversations`] is where an agent moves once the harness actually exists.
#[derive(Clone)]
struct PendingConversation {
    project_id: ProjectId,
    agent_type: String,
    account: Option<String>,
    /// The saved setup this conversation was started from, passed through to the library's
    /// `resolve` at launch. Its model and mode are *also* copied into the `chosen_*` fields
    /// below, because those outrank a profile inside `resolve`.
    profile: Option<String>,
    cwd: PathBuf,
    /// Set by a `SetAgentConfig{config_id: "model", ..}` before launch. `None`, or an empty
    /// string, both mean "whatever this harness defaults to" — no `--model` flag at all.
    chosen_model: Option<String>,
    /// Set by a `SetAgentConfig{config_id: "thinking", ..}` before launch. Same `None`/empty
    /// convention as `chosen_model`: no `--effort`-equivalent flag at all.
    chosen_thinking: Option<String>,
    /// Set by a `SetAgentConfig{config_id: "mode", ..}` before launch. Same `None`/empty
    /// convention as `chosen_model`: no `--permission-mode`-equivalent flag at all.
    chosen_mode: Option<String>,
    /// The model catalogue this agent's discovery answered, refreshed by a synchronous cache
    /// re-read whenever the picked model changes (see the `SetAgentConfig` "model" arm) — a
    /// level is per model, so recomputing the thinking picker needs the whole catalogue, not
    /// just the one id that changed.
    catalogue: Vec<CachedModel>,
    /// Where the real pump's own sequence counter picks up, so a message sent before launch (the
    /// model-discovery thread's `ConfigOptions`, always seq 1) and the harness's first frame after
    /// it are one unbroken sequence.
    next_seq: u64,
    /// What the user first asked this conversation, held for the naming pass and nothing else.
    ///
    /// Set by the first [`Message::PromptAgent`] and cleared when the naming has been asked for.
    /// It lives on this row rather than beside the `Conversation` because a one-shot harness has
    /// no `Conversation` between turns, and the prompt has to outlive that gap.
    opening_prompt: Option<String>,
    /// Whether the naming pass has run for this conversation.
    ///
    /// Set whether it produced a name or not: a conversation is named once, and a provider that
    /// would not answer must not be asked again every half-second for as long as the agent lives.
    named: bool,
    /// The harness's own session id, as the last process to run this conversation reported it.
    ///
    /// **This is what makes a one-shot harness converse.** Such a harness answers once and exits,
    /// so the next turn is a new process — and the only thing joining the two is this id, handed
    /// back to the library as the run's resume. `None` before the first run, and for a harness
    /// that named none.
    resume: Option<String>,
    /// The MCP servers this conversation asked for, from [`Message::StartConversation::mcps`],
    /// carried through to [`ConverseOptions::mcps`] on every launch and relaunch. `compose_run`
    /// does not act on it yet — injecting the server is later work — so this is only where the
    /// pick is remembered between here and there.
    mcps: Vec<String>,
}

/// `base`, then `base 2`, `base 3` … — the first that nothing in `taken` is wearing. A counter
/// from the second occurrence onward, per project, so the first `claude` is just `claude` and a
/// closed `claude 2` is reused rather than skipped.
fn unique_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|name| name == base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base} {n}");
        if !taken.iter().any(|name| name == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Now, in epoch milliseconds — the unit a stored credential's own expiry is in.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The models (with reasoning levels folded in) `agent_type` will answer for `account`, resolved
/// directly against the library rather than through [`crate::agent::Agents::discover_models`] —
/// this runs on a one-off thread with no borrow of `self`, and `Agents` cannot be shared there
/// while `Message::SetSettings` still needs `&mut` access to it for `set_isolate`.
///
/// Cached on the harness binary's own `version()` string: a hit skips both probes entirely; a
/// miss joins `discover_models()` (fatal on failure — no models is nothing to offer) with
/// `discover_thinking()` (**not** fatal — models with no reasoning knob are the normal case, so a
/// failure there just means every model comes back with empty `levels`), then writes the cache.
/// `version()` failing or answering empty bypasses the cache entirely (no read, no write) —
/// an unversioned entry would survive the very upgrade it exists to be invalidated by.
fn probe_catalogue(
    agent_type: &str,
    account: &str,
    cache: &FileHarnessCache,
) -> anyhow::Result<Vec<CachedModel>> {
    let harness = agent_manager::harness::resolve(agent_type)
        .ok_or_else(|| anyhow::anyhow!("unknown agent type '{agent_type}'"))?;

    let version = harness.version().ok().filter(|v| !v.is_empty());
    if let Some(version) = &version
        && let Some(hit) = cache.get(agent_type, account, version)
    {
        return Ok(hit);
    }

    let models = harness.discover_models()?;
    let thinking = harness.discover_thinking().unwrap_or_default();
    let merged: Vec<CachedModel> = models
        .into_iter()
        .map(|model| {
            let levels = thinking.get(&model.id).cloned().unwrap_or_default();
            CachedModel {
                id: model.id,
                description: model.description,
                default: model.default,
                levels: levels
                    .levels
                    .into_iter()
                    .map(|level| crate::store::harness::CachedLevel {
                        value: level.value,
                        label: level.label,
                        description: level.description,
                    })
                    .collect(),
                default_level: levels.default_level,
            }
        })
        .collect();

    if let Some(version) = &version {
        cache.put(agent_type, account, version, merged.clone());
    }
    Ok(merged)
}

/// The default model id: the one flagged `default`, else the first, else empty (no models at
/// all). Shared by [`model_config_option`] (its `current`) and the caller feeding a chosen-model
/// id to [`thinking_config_option`] before the user has picked one.
fn default_model_id(models: &[CachedModel]) -> String {
    models
        .iter()
        .find(|model| model.default)
        .or_else(|| models.first())
        .map(|model| model.id.clone())
        .unwrap_or_default()
}

/// The one `model` [`ConfigOption`] a pending agent's picker gets: a select filled from what the
/// harness discovered, or — when it could not answer — a single "Default" choice. Per
/// `_docs/wip/agent-setup.md`'s P3: "one that cannot answer must offer 'whatever it defaults to'
/// rather than an empty picker." An empty `current`/`value` is what tells `launch_pending` to pass
/// no `--model` flag at all.
///
/// `chosen` is preselected as `current` when it names one of `models` — the caller's job is to
/// resolve that to the harness default (or the newly picked model) beforehand; a `chosen` that
/// matches nothing here (never remembered, or a model the harness dropped) falls back to
/// [`default_model_id`].
fn model_config_option(models: &[CachedModel], chosen: &str) -> ConfigOption {
    if models.is_empty() {
        return ConfigOption {
            id: "model".to_string(),
            name: "Model".to_string(),
            description: Some(
                "This harness's model list could not be read; it will use its own default."
                    .to_string(),
            ),
            category: Some(ConfigCategory::Model),
            value: ConfigValue::Select {
                current: String::new(),
                choices: vec![ConfigChoice {
                    value: String::new(),
                    name: "Default".to_string(),
                    description: None,
                    group: None,
                }],
            },
        };
    }

    let current = if models.iter().any(|model| model.id == chosen) {
        chosen.to_string()
    } else {
        default_model_id(models)
    };
    ConfigOption {
        id: "model".to_string(),
        name: "Model".to_string(),
        description: None,
        category: Some(ConfigCategory::Model),
        value: ConfigValue::Select {
            current,
            choices: models
                .iter()
                .map(|model| ConfigChoice {
                    value: model.id.clone(),
                    name: model.id.clone(),
                    description: model.description.clone(),
                    group: None,
                })
                .collect(),
        },
    }
}

/// The `thinking` [`ConfigOption`] for whichever model `chosen_model` names (falling back to the
/// default model, then the first, when `chosen_model` doesn't match one) — `None` when there is no
/// matching model or it has no reasoning levels at all, an absent picker rather than an empty
/// one, because a lone "Default" choice would claim a knob the harness does not have.
///
/// `chosen_thinking` is preselected as `current` only when the resolved model actually accepts
/// it — offering a level a model does not have is exactly what this design forbids — otherwise
/// `current` falls back to the model's own default (or first) level.
fn thinking_config_option(
    models: &[CachedModel],
    chosen_model: &str,
    chosen_thinking: &str,
) -> Option<ConfigOption> {
    let model = models
        .iter()
        .find(|model| model.id == chosen_model)
        .or_else(|| models.iter().find(|model| model.default))
        .or_else(|| models.first())?;
    if model.levels.is_empty() {
        return None;
    }
    let current = if model
        .levels
        .iter()
        .any(|level| level.value == chosen_thinking)
    {
        chosen_thinking.to_string()
    } else {
        model
            .default_level
            .clone()
            .unwrap_or_else(|| model.levels[0].value.clone())
    };
    Some(ConfigOption {
        id: "thinking".to_string(),
        name: "Thinking".to_string(),
        description: None,
        category: Some(ConfigCategory::ThoughtLevel),
        value: ConfigValue::Select {
            current,
            choices: model
                .levels
                .iter()
                .map(|level| ConfigChoice {
                    value: level.value.clone(),
                    name: level.label.clone(),
                    description: level.description.clone(),
                    group: None,
                })
                .collect(),
        },
    })
}

/// The `mode` [`ConfigOption`] for `agent_type`, from its fixed, non-probed [`agent_manager::
/// harness::Harness::modes`] list — `None` when the harness offers no choice (opencode and
/// Copilot today), an absent picker rather than an empty one. `current` is always empty: no mode
/// is a harness default the way a model or a reasoning level is, so nothing is preselected —
/// leaving it unset is what tells `launch_pending` to pass no mode flag at all.
fn mode_config_option(agent_type: &str, chosen: &str) -> Option<ConfigOption> {
    let modes = agent_manager::harness::resolve(agent_type)?.modes();
    if modes.is_empty() {
        return None;
    }
    Some(ConfigOption {
        id: "mode".to_string(),
        name: "Mode".to_string(),
        description: None,
        category: Some(ConfigCategory::Mode),
        value: ConfigValue::Select {
            current: chosen.to_string(),
            choices: modes
                .into_iter()
                .map(|mode| ConfigChoice {
                    value: mode.id,
                    name: mode.label,
                    description: mode.description,
                    group: None,
                })
                .collect(),
        },
    })
}

/// What a `SetAgentConfig{"model", ..}` on a pending agent has to answer with, or `None` when it
/// has nothing to say.
///
/// **The recomputed thinking picker is the only thing this re-send can tell the window.** A level
/// is per model, so offering one the newly picked model does not accept is exactly the lie this
/// design exists to prevent — and the level is reset to the new model's own default rather than
/// carrying the previous model's over. But the model list and the mode list cannot change by
/// picking a model, and `current` is the value the window itself just sent: when the levels come
/// out identical to what `shown_model`/`shown_thinking` already put on screen, the whole message
/// is a redundant round trip that redraws the picker already there.
///
/// Silent then; the *complete* set whenever anything did change, because a partial one would leave
/// a stale picker up.
fn config_options_after_model_pick(
    agent_type: &str,
    models: &[CachedModel],
    shown_model: &str,
    shown_thinking: &str,
    picked: &str,
) -> Option<Vec<ConfigOption>> {
    let shown = thinking_config_option(models, shown_model, shown_thinking);
    let options = build_config_options(agent_type, models, picked, "", "");
    (options.iter().find(|option| option.id == "thinking") != shown.as_ref()).then_some(options)
}

/// The model id the picker is showing: the user's pick, or — before any pick — the remembered
/// one when the catalogue still has it, and the harness's default otherwise.
///
/// Shared by the discovery thread's first advertisement and the `SetAgentConfig` arm that decides
/// whether a re-advertisement would say anything new; the two must resolve it the same way or the
/// comparison is against a picker that was never on screen.
fn advertised_model(models: &[CachedModel], chosen: Option<&str>, last_used: &str) -> String {
    if let Some(chosen) = chosen {
        return chosen.to_string();
    }
    if models.iter().any(|model| model.id == last_used) {
        last_used.to_string()
    } else {
        default_model_id(models)
    }
}

/// The full set of `ConfigOption`s a pending agent's picker gets, for `chosen_model` (the
/// remembered-or-default model id on the first send, the newly picked one on a
/// `SetAgentConfig{"model", ..}` re-send) and `chosen_thinking` (the remembered level on the
/// first send, empty on a re-send — a model change resets thinking to that model's own default,
/// see the call site): always `model`, plus `mode`/`thinking` whenever the harness/model actually
/// offers one.
fn build_config_options(
    agent_type: &str,
    models: &[CachedModel],
    chosen_model: &str,
    chosen_thinking: &str,
    chosen_mode: &str,
) -> Vec<ConfigOption> {
    let mut options = vec![model_config_option(models, chosen_model)];
    options.extend(mode_config_option(agent_type, chosen_mode));
    options.extend(thinking_config_option(
        models,
        chosen_model,
        chosen_thinking,
    ));
    options
}

/// What model and thinking level a launch actually passes, given what the user picked and what
/// this harness was last launched with.
///
/// **A picker nobody touched still means something.** The option the window drew carried the
/// last-launched model as its `current`, so launching with no flag would run a different model
/// from the one on screen — and only `SetAgentConfig` ever fills `chosen_model`, so a session that
/// simply accepted the proposal would otherwise send nothing at all. The remembered value is
/// resolved here instead, which is what keeps the picker and the launch telling one story.
///
/// Both are validated rather than trusted: a remembered model that has since left the catalogue
/// falls back to no flag — the harness's own default — rather than naming a model that is gone;
/// and a level belongs to a *model*, never to a harness, so one the resolved model does not accept
/// is dropped, the same rule [`thinking_config_option`] follows when it offers them.
fn launch_picks(
    chosen_model: Option<&str>,
    chosen_thinking: Option<&str>,
    catalogue: &[CachedModel],
    last_model: &str,
    last_thinking: &str,
) -> (Option<String>, Option<String>) {
    let model = chosen_model
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .or_else(|| {
            catalogue
                .iter()
                .any(|model| model.id == last_model)
                .then(|| last_model.to_string())
        });
    let thinking = chosen_thinking
        .filter(|level| !level.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let resolved = model.as_deref()?;
            catalogue
                .iter()
                .find(|entry| entry.id == resolved)?
                .levels
                .iter()
                .any(|level| level.value == last_thinking)
                .then(|| last_thinking.to_string())
        });
    (model, thinking)
}

impl Coordinator {
    fn new(
        host: HostEnd,
        root: ConfigRoot,
        mut projects: Projects,
        work: Work,
        settings: Settings,
        pending: Vec<Reply>,
    ) -> Self {
        // Ubiq's own MCP servers, on one loopback port for every agent this process will start.
        // Bound *before* the agents are built, because the URL a run is composed with is written
        // into the harness's configuration and never revisited — there is no later moment to tell
        // a run where the port is. A port that will not bind is said once, here, and every run
        // then composes with no injected servers rather than failing (`crate::mcp`).
        let mcp_agents = crate::mcp::Registry::new();
        let mcp = crate::mcp::start(mcp_agents.clone(), host.voice())
            .inspect_err(|error| {
                tracing::warn!("Ubiq's own MCP servers are not available: {error:#}");
            })
            .ok();

        // A run directory outlives its pane only when Ubiq did not get to close it, and no pane
        // from a previous process is still running, so the sweep happens once here.
        let agents = {
            let host = settings.host();
            let mut agents = Agents::new(root.path.clone(), host.isolate_agents);
            agents.set_policy(host.agent_home.clone(), host.extra_grants.clone());
            agents.set_environment(crate::environment::Environment::load(&root.path));
            agents.set_commands(host.agent_commands.clone());
            if let Some(serving) = &mcp {
                agents.set_mcp(serving.base_url(), mcp_agents.clone());
            }
            agents
        };
        agents.sweep();
        let settings = Arc::new(settings);
        let connectors = Connectors::new(settings.clone(), &root.path);
        let repos = Repos::new(settings.clone(), connectors.store());
        // Reserved once, not per attach: it belongs to no project, and the interface is told a
        // path rather than a maybe. A root that will not take the directory is still named — what
        // is kept there is a cache, so the interface downgrades rather than fails.
        let shared_workarea = crate::projects::reserve_shared_workarea(&root.path);
        let web_assets = WebAssets::new(std::path::PathBuf::from(&shared_workarea));
        // The provider family shares the connector family's keychain — one store, one probe of
        // whether this platform has one — and is built before the backend because it is what
        // lends `select` the key a backend is constructed with.
        let ai_providers = Providers::open(settings.clone(), connectors.store(), &root.path);
        let assist = {
            let held = settings.host();
            assist::select(&held.assist, &held.ai_providers, &ai_providers)
        };
        // The catalogue was opened before the settings were parsed, so the tree it is allowed to
        // delete a project's own folder from is named here, and swept once now: an ephemeral clone
        // whose window went without a clean forget has nobody left to remove it.
        projects.point_ephemeral_at(crate::settings::ephemeral_root(
            &settings.host(),
            &root.path,
        ));
        projects.sweep_ephemeral();
        let catalogue = Arc::new(FileHarnessCache::new(
            root.path.join("cache").join("harness-models.toml"),
        ));
        // A meter that will not open is a stats screen with empty rows, and nothing else. Said
        // once, here, rather than on every sample.
        let usage = Usage::open(&root.path)
            .map(Arc::new)
            .inspect_err(|error| tracing::warn!("the usage meter is not available: {error}"))
            .ok();
        // Machine facts start sampling now so the first attach already has them; the thread keeps
        // them fresh every twenty seconds after that (`crate::host_meta`).
        let meta = host_meta::start(root.path.clone());

        // The conversations the last process left behind, back as launch recipes.
        //
        // **Only the rows marked persistent.** An unmarked row is a leftover — the sweep above has
        // just deleted its run directory, and the run directory *is* the harness's session store,
        // so there is nothing left for a resume to find. Restoring it would offer the user a
        // conversation that comes back empty.
        //
        // What comes back is the recipe and nothing else: no `WorkAgent`, no owner, no `resume`
        // token. A restored row is what [`Self::revive_conversation`] reads its facts out of, not a
        // live agent — `drives` answers false for one, so nothing can be driven into it by accident
        // before a window has asked for it back.
        let pending_conversations = conversation_record::all(&root.path.join("sessions"))
            .into_iter()
            .filter(|(_, row)| row.persistent)
            .map(|(agent_id, row)| {
                (
                    agent_id,
                    PendingConversation {
                        project_id: row.project_id,
                        agent_type: row.agent_type,
                        account: row.account,
                        profile: row.profile,
                        cwd: row.cwd,
                        chosen_model: row.model,
                        chosen_thinking: row.thinking,
                        chosen_mode: row.mode,
                        catalogue: Vec::new(),
                        next_seq: row.next_seq,
                        opening_prompt: None,
                        // A row that carries a title has already been through the naming pass, and
                        // a conversation is named once however many processes it outlives.
                        named: row.title.is_some(),
                        resume: None,
                        // Not part of the persisted row yet: an mcp pick made before a restart
                        // does not survive one, same as `catalogue` above.
                        mcps: Vec::new(),
                    },
                )
            })
            .collect();

        Self {
            host,
            root,
            projects,
            work,
            settings,
            connectors,
            repos,
            web_assets,
            shared_workarea,
            agents,
            catalogue,
            files: Files::start(),
            git: Git::start(),
            search: Search::start(),
            index: crate::index::Index::start(),
            active_searches: HashMap::new(),
            assist,
            ai_providers,
            active_suggests: HashMap::new(),
            notifications: notifications::Centre::new(),
            watchers: HashMap::new(),
            pending,
            pane_projects: HashMap::new(),
            pane_sessions: HashMap::new(),
            panes: HashMap::new(),
            owners: HashMap::new(),
            focused: HashMap::new(),
            conversations: HashMap::new(),
            conversation_owners: HashMap::new(),
            pending_conversations,
            logins: HashMap::new(),
            started: Instant::now(),
            logins_synced: Instant::now(),
            agents_this_run: 0,
            usage,
            meta,
            mcp,
        }
    }

    /// One reading of the host, taken because a window asked. Counts and usage are sampled here
    /// and now; machine resources come from the metadata sampler's cached snapshot
    /// (`crate::host_meta`), refreshed every twenty seconds and never realtime.
    ///
    /// The two usage vectors come back empty when the meter is absent *or* when reading it fails,
    /// and a failure is a log line. A screen that reports how healthy the host is must never be
    /// the thing that takes it down.
    fn stats(&self) -> HostStats {
        let read = |what: &str, rows: Option<Result<Vec<UsageRow>, _>>| match rows {
            Some(Ok(rows)) => rows,
            Some(Err(error)) => {
                tracing::warn!("could not read {what} usage: {error}");
                Vec::new()
            }
            None => Vec::new(),
        };
        let meta = self.meta.lock().ok().map(|guard| guard.clone());

        HostStats {
            rss_bytes: memory_stats::memory_stats().map(|m| m.physical_mem as u64),
            uptime_secs: self.started.elapsed().as_secs(),
            open_projects: self.projects.open_count(),
            agents_live: self.conversations.len(),
            agents_this_run: self.agents_this_run,
            sessions_count: self
                .pane_sessions
                .values()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            cpu_load_pct: meta.as_ref().and_then(|meta| meta.cpu_load_pct),
            mem_free_bytes: meta.as_ref().and_then(|meta| meta.mem_free_bytes),
            mem_total_bytes: meta.as_ref().and_then(|meta| meta.mem_total_bytes),
            disk_free_bytes: meta.as_ref().and_then(|meta| meta.disk_free_bytes),
            this_run: read("this run's", self.usage.as_ref().map(|u| u.this_run())),
            // Everything ever recorded. Bounding the window is the interface's to ask for once it
            // has an opinion about how far back a chart goes.
            history: read("historical", self.usage.as_ref().map(|u| u.history(0))),
        }
    }

    fn run(mut self) {
        loop {
            // A queued preference has to be written even if nobody says anything else, so the wait
            // is bounded whenever something is pending. A harness that ends on its own sets a flag
            // rather than sending anything (see `Conversation::ended`), so nothing would otherwise
            // wake this loop to notice — the wait is bounded the same way whenever a conversation
            // is live, taking the sooner of the two deadlines, or its run directory (credentials
            // seeded into it included) would outlive it until the next thing the user did.
            let due = self.projects.next_due(Instant::now());
            let wait = if self.conversations.is_empty() && !self.repos.busy() {
                due
            } else {
                Some(due.map_or(CONVERSATION_POLL, |due| due.min(CONVERSATION_POLL)))
            };
            // A pane running a harness in passthrough is not a conversation and may say nothing
            // for hours, so neither deadline above would wake this loop — and its harness is
            // rotating an OAuth token the whole time. `sync_logins_due` paces itself; this only
            // has to guarantee it is reached. An empty host still blocks.
            let wait = match wait {
                _ if self.panes.is_empty() => wait,
                Some(wait) => Some(wait.min(LOGIN_SYNC_EVERY)),
                None => Some(LOGIN_SYNC_EVERY),
            };
            let event = match wait {
                Some(wait) => match self.host.recv_timeout(wait) {
                    Ok(event) => Some(event),
                    Err(flume::RecvTimeoutError::Timeout) => None,
                    Err(flume::RecvTimeoutError::Disconnected) => break,
                },
                None => match self.host.recv() {
                    Ok(event) => Some(event),
                    Err(_) => break,
                },
            };

            match event {
                Some(FromClient::Connected(client)) => self.client_here(client),
                Some(FromClient::Said { client, message }) => self.dispatch(client, message),
                Some(FromClient::Gone(client)) => self.client_gone(client),
                None => {}
            }
            self.sync_logins_due();
            self.remember_sessions();
            self.name_conversations();
            self.reap_conversations();
            self.register_clones();
            self.projects.flush_due(Instant::now());
        }
        // Nothing is left to say it to, but what the user last did still belongs on disk.
        self.projects.flush();
    }

    /// A window attached. It is told what the host is, and hears anything the catalogue wanted to
    /// say before there was anybody to say it to.
    fn client_here(&mut self, client: ClientId) {
        tracing::debug!("{client} attached");
        let meta = self.meta.lock().ok().map(|guard| guard.clone());
        self.host.send(
            To::Client(client),
            Message::HostInfo {
                config_root: self.root.path.to_string_lossy().into_owned(),
                is_default: self.root.is_default(),
                hostname: meta.as_ref().and_then(|meta| meta.hostname.clone()),
                os: meta.as_ref().map(|meta| meta.os.clone()),
                arch: meta.as_ref().map(|meta| meta.arch.clone()),
                triplet: meta.as_ref().map(|meta| meta.triplet.clone()),
                cpu_count: meta.as_ref().map(|meta| meta.cpu_count),
                mem_total_bytes: meta.as_ref().and_then(|meta| meta.mem_total_bytes),
                shared_workarea: Some(self.shared_workarea.clone()),
            },
        );
        for reply in std::mem::take(&mut self.pending) {
            self.host.send(To::Client(client), reply.into_message());
        }
    }

    /// The two sinks a flow thread is given: the window that asked, and everybody. A flow reports
    /// its own stages to the asker and a changed record to every window, and it must not have to
    /// learn the routing table to do either.
    fn sinks(&self, client: ClientId) -> (ubiq_proto::bus::Mailbox, ubiq_proto::bus::Mailbox) {
        (
            self.host.mailbox(To::Client(client)),
            self.host.mailbox(To::Everyone),
        )
    }

    /// Say what the catalogue answered, to whoever it is for.
    fn answer(&self, client: ClientId, replies: Vec<Reply>) {
        for reply in replies {
            let to = if reply.is_broadcast() {
                To::Everyone
            } else {
                To::Client(client)
            };
            self.host.send(to, reply.into_message());
        }
    }

    /// A window has gone. Everything it owned goes with it.
    ///
    /// This used to happen by itself: a closed window dropped its whole bus, the coordinator thread
    /// ended, and the pseudo-terminals went with it. One host outlives every window, so nothing
    /// drops now and the reaping has to be deliberate — without this, every closed window leaves a
    /// live harness behind.
    fn client_gone(&mut self, client: ClientId) {
        self.focused.remove(&client);
        // Dropping a watch stops its `notify` handle and ends its debounce thread.
        let dropped: Vec<ProjectId> = self
            .watchers
            .keys()
            .filter(|(owner, _)| *owner == client)
            .map(|(_, project_id)| *project_id)
            .collect();
        self.watchers.retain(|(owner, _), _| *owner != client);
        // An index is worth keeping only while somebody has the project open. A project another
        // window still holds keeps its own — the watch map is the only record of who has what.
        for project_id in dropped {
            let still_open = self
                .watchers
                .keys()
                .any(|(_, watched)| *watched == project_id);
            if !still_open {
                self.index.submit(crate::index::Job::Drop(project_id));
            }
        }
        let owned: Vec<PaneId> = self
            .owners
            .iter()
            .filter(|(_, owner)| **owner == client)
            .map(|(pane_id, _)| *pane_id)
            .collect();

        // Taking the pseudo-terminal out of the map is most of the work: dropping it closes the
        // master, the kernel hangs up the slave, and the harness goes. The kill is the guarantee
        // for anything that ignores the hang-up.
        for pane_id in owned {
            self.owners.remove(&pane_id);
            if let Some(mut pane) = self.panes.remove(&pane_id) {
                tracing::info!("{client} has gone; killing the harness in pane {pane_id}");
                pane.kill();
            }
            self.pane_gone(client, pane_id);
        }

        let talking: Vec<AgentId> = self
            .conversation_owners
            .iter()
            .filter(|(_, (owner, _))| *owner == client)
            .map(|(agent_id, _)| *agent_id)
            .collect();
        for agent_id in talking {
            tracing::info!("{client} has gone; stopping agent {agent_id}");
            self.end_conversation(agent_id, StopReason::Cancelled);
        }
        // A flow whose window has gone has nobody left to answer its next question, and neither
        // has a clone: dropping its sender is what stops the transfer mid-fetch.
        self.connectors.client_gone(client);
        self.repos.client_gone(client);
        // A bundle fetch is not cancelled with the window that asked: another window may be
        // waiting on the same one, and the bytes are worth having whoever wanted them. This only
        // stops it being told.
        self.web_assets.client_gone(client);
    }

    /// Take the folders finished clones left into the catalogue.
    ///
    /// A clone's own thread cannot: [`Projects`] is this thread's, which is why a finished clone
    /// posts a `Registered` and this drains it. The project is added exactly as a folder the user
    /// picked would be, so [`Message::ProjectAdded`] is the success signal and there is no
    /// clone-success message to keep in step with it.
    fn register_clones(&mut self) {
        for done in self.repos.registered() {
            tracing::info!("clone {} landed at {}", done.clone_id, done.path);
            let replies =
                self.projects
                    .add(&done.path, Some(done.name), None, None, done.ephemeral);
            self.answer(done.client, replies);
        }
    }

    /// A pane has ended, however it ended: the project it belonged to has one fewer.
    fn pane_gone(&mut self, client: ClientId, pane_id: PaneId) {
        // The pane owned its run's configuration directory — credentials seeded into it included
        // — so that goes when the pane does. A harness that exited by itself reaches here too:
        // the interface closes the tab on `PaneExited`, and closing a tab is a `CloseWorkspace`.
        self.agents.retire(pane_id);
        // A login pane owns no run directory and no project, so the retire above and the
        // count below both find nothing — what it owns is the answer to whether it captured
        // anything, and this is the only moment that answer exists.
        self.login_gone(client, pane_id);
        self.pane_sessions.remove(&pane_id);
        if let Some(project_id) = self.pane_projects.remove(&pane_id) {
            let replies = self.projects.pane_closed(project_id);
            self.answer(client, replies);
        }
    }

    /// Tell the window that asked that its pane never started.
    ///
    /// A `PaneError` and no `WorkspaceSpawned`: the interface was never told the pane exists, so
    /// there is nothing on screen to close, and the error belongs where the user was looking.
    fn refuse_pane(&self, client: ClientId, pane_id: PaneId, error: String) {
        self.host
            .send(To::Client(client), Message::PaneError { pane_id, error });
    }

    /// Whether this window is the one that owns the pane. A message about somebody else's pane is
    /// a wiring mistake, not something to act on.
    fn owns(&self, client: ClientId, pane_id: PaneId) -> bool {
        match self.owners.get(&pane_id) {
            Some(owner) if *owner == client => true,
            Some(_) => {
                tracing::warn!(
                    "{client} sent a message about pane {pane_id}, which it does not own"
                );
                false
            }
            // The pane has already gone; its last messages are in flight behind it.
            None => false,
        }
    }

    fn dispatch(&mut self, client: ClientId, message: Message) {
        self.capture_inbound(&message);
        match message {
            Message::SpawnWorkspace {
                session_id,
                project_id,
                rel_path,
                agent_type,
                args,
                picks,
            } => self.spawn_workspace(
                client, session_id, project_id, rel_path, agent_type, args, picks,
            ),

            Message::TerminalInput { pane_id, bytes } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                if let Some(pane) = self.panes.get_mut(&pane_id)
                    && let Err(error) = pane.write(&bytes)
                {
                    self.host.send(
                        To::Client(client),
                        Message::PaneError {
                            pane_id,
                            error: error.to_string(),
                        },
                    );
                }
            }

            // A resize for a pane that has gone is ignored: the geometry has nowhere to land.
            Message::TerminalResize {
                pane_id,
                cols,
                rows,
            } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                if let Some(pane) = self.panes.get(&pane_id)
                    && let Err(error) = pane.resize(cols, rows)
                {
                    self.host.send(
                        To::Client(client),
                        Message::PaneError {
                            pane_id,
                            error: error.to_string(),
                        },
                    );
                }
            }

            Message::Focus { pane_id } => {
                if self.owns(client, pane_id) {
                    self.focused.insert(client, pane_id);
                }
            }

            Message::CloseWorkspace { pane_id } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                self.owners.remove(&pane_id);
                if let Some(mut pane) = self.panes.remove(&pane_id) {
                    tracing::info!("closing pane {pane_id}, killing its harness");
                    pane.kill();
                }
                if self.focused.get(&client) == Some(&pane_id) {
                    self.focused.remove(&client);
                }
                self.pane_gone(client, pane_id);
            }

            // What can be started here, asked by the new-pane menu as it opens. Probed on every
            // ask: a shell installed since the window opened is offered without a restart.
            Message::ListStats => {
                let stats = self.stats();
                self.host.send(To::Client(client), Message::Stats { stats });
            }
            Message::ListShells => {
                self.host.send(
                    To::Client(client),
                    Message::ShellList {
                        shells: shells::available(),
                    },
                );
            }
            Message::ListTools { project_id } => {
                self.host.send(
                    To::Client(client),
                    Message::ToolsListed {
                        system: self.system_tools(),
                        project: self.project_tools(project_id),
                    },
                );
            }
            Message::RunTool {
                session_id,
                project_id,
                scope,
                id,
            } => self.run_tool(client, session_id, project_id, scope, id),

            // ── the notification family ─────────────────────────────
            // The centre decides everything: whether a rule silences the request, whether the
            // desktop is told, and who hears the answer. Nothing here draws a conclusion of its
            // own — see `crates/ubiq-host/src/notifications/mod.rs`.
            Message::RaiseNotification { request } => {
                let replies = self.notifications.raise(request);
                self.answer(client, replies);
            }
            Message::ListNotifications => {
                let replies = self.notifications.list();
                self.answer(client, replies);
            }
            Message::ReadNotifications { id } => {
                let replies = self.notifications.read(id);
                self.answer(client, replies);
            }
            Message::DismissNotifications { id } => {
                let replies = self.notifications.dismiss(id);
                self.answer(client, replies);
            }
            Message::MuteNotifications {
                scope,
                max_level,
                duration,
            } => {
                let replies = self.notifications.mute(scope, max_level, duration);
                self.answer(client, replies);
            }
            Message::UnmuteNotifications { scope } => {
                let replies = self.notifications.unmute(scope);
                self.answer(client, replies);
            }

            // ── the project family ──────────────────────────────────
            Message::ListProjects => {
                let reply = self.projects.list_projects();
                self.answer(client, vec![reply]);
            }
            Message::AddProject {
                path,
                name,
                colour,
                custom_colour,
                temporary,
            } => {
                let replies = self
                    .projects
                    .add(&path, name, colour, custom_colour, temporary);
                self.answer(client, replies);
            }
            Message::ForgetProject { project_id } => {
                let replies = self.projects.forget(project_id);
                // The catalogue went first and took the project's directory with it, so the
                // tasks on disk are already gone. This is the memory that was left, and this
                // is the only place that knows both services.
                self.work.forget(project_id);
                self.git_forget(client, project_id);
                // Dropping a watch stops its `notify` handle and ends its debounce thread — the
                // catalogue no longer holds this project, so nothing should still be sending
                // `ProjectFilesChanged` for it.
                self.watchers
                    .retain(|(_, watched), _| *watched != project_id);
                // The index goes with it. Its directory sits under the project's own, which
                // Forget removes, so what is left to do is close the handle holding it open.
                self.index.submit(crate::index::Job::Drop(project_id));
                self.answer(client, replies);
            }
            Message::UpdateProject {
                project_id,
                name,
                colour,
                custom_colour,
                search_excludes,
                index,
                tools,
            } => {
                let replies = self.projects.update(
                    project_id,
                    name,
                    colour,
                    custom_colour,
                    search_excludes,
                    index,
                    tools,
                );
                self.answer(client, replies);
                // A level the user just changed takes effect now, not at the next open: turning
                // indexing off and watching the disk not free up would read as a setting that
                // does nothing.
                if index.is_some() {
                    self.settle_index(project_id);
                }
            }
            Message::LocateProject { project_id, path } => {
                let replies = self.projects.locate(project_id, &path);
                self.git_forget(client, project_id);
                self.answer(client, replies);
            }
            Message::OpenedProject { project_id } => {
                let replies = self.projects.opened(project_id);
                self.answer(client, replies);
                self.watch_project(client, project_id);
            }
            Message::RefreshProject { project_id } => {
                let replies = self.projects.refresh(project_id);
                self.answer(client, replies);
            }
            Message::GetPreferences { scope } => {
                let reply = self.projects.get_preferences(scope);
                self.answer(client, vec![reply]);
            }
            Message::SetPreferences { scope, value } => {
                self.projects.set_preferences(scope, value, Instant::now());
            }
            Message::ListAgentTypes => {
                let agent_types = self.agents.types();
                self.host
                    .send(To::Client(client), Message::AgentTypes { agent_types });
            }
            Message::CheckAgentCommand {
                agent_type,
                command,
            } => {
                self.check_agent_command_job(client, agent_type, command);
            }
            Message::ListHarnessCatalogue {
                agent_type,
                account,
            } => {
                self.list_harness_catalogue_job(client, agent_type, account);
            }

            Message::ListAccounts => {
                self.send_accounts(client);
            }
            Message::ListProfiles => {
                self.send_profiles(client);
            }
            Message::SaveProfile { profile } => match self.agents.save_profile(profile) {
                Ok(()) => self.send_profiles(client),
                Err(error) => self.host.send(
                    To::Client(client),
                    Message::AccountError {
                        error: format!("{error:#}"),
                    },
                ),
            },
            // The catalogue is a constant of this build, so it is answered from the table itself
            // rather than from anything running: the panel's checklist and the tools a harness
            // will be offered come from the one place, and cannot disagree (`crate::mcp`).
            Message::ListMcps => self.host.send(
                To::Client(client),
                Message::Mcps {
                    servers: crate::mcp::catalogue(),
                },
            ),
            Message::BeginHarnessLogin {
                agent_type,
                account,
                probe,
            } => {
                self.begin_harness_login(client, agent_type, account, probe);
            }
            Message::CheckHarnessLogin {
                agent_type,
                account,
            } => {
                self.check_harness_login(client, agent_type, account);
            }
            Message::RenameAccount {
                account,
                new_account,
            } => {
                self.rename_account(client, account, new_account);
            }
            Message::DeleteAccount { account } => {
                self.delete_account(client, account);
            }
            Message::DeleteHarnessLogin {
                agent_type,
                account,
            } => {
                self.delete_harness_login(client, agent_type, account);
            }

            // ── Connector family: the identities an external *service* runs as ──
            // Everything a file can answer is answered here; everything that touches a network is
            // a flow on its own thread, a probe included. Nothing in this block blocks.
            Message::ListConnections => {
                let replies = self.connectors.list();
                self.answer(client, replies);
            }
            Message::BeginConnect {
                connect_id,
                provider,
                instance,
                label,
                auth,
                client_id,
                oauth_app,
            } => {
                let (asker, everyone) = self.sinks(client);
                let replies = self.connectors.begin(
                    client, connect_id, provider, instance, label, auth, client_id, oauth_app,
                    asker, everyone,
                );
                self.answer(client, replies);
            }
            Message::CancelConnect { connect_id } => self.connectors.cancel(connect_id),
            Message::SubmitConnectSecret { connect_id, secret } => {
                let replies = self.connectors.answer(connect_id, Answer::Secret(secret));
                self.answer(client, replies);
            }
            Message::TrustCertificate {
                connect_id,
                origin,
                sha256,
            } => {
                let replies = self
                    .connectors
                    .answer(connect_id, Answer::Certificate { origin, sha256 });
                self.answer(client, replies);
            }
            Message::RenameConnection { connection, label } => {
                let replies = self.connectors.rename(connection, label);
                self.answer(client, replies);
            }
            Message::DeleteConnection { connection } => {
                let replies = self.connectors.delete(connection);
                self.answer(client, replies);
            }
            Message::CheckConnection { connection, probe } => {
                let (asker, everyone) = self.sinks(client);
                let replies = self
                    .connectors
                    .check(client, connection, probe, asker, everyone);
                self.answer(client, replies);
            }
            Message::ForgetCertificate { origin } => {
                let replies = self.connectors.forget_cert(origin);
                self.answer(client, replies);
            }
            Message::SaveOauthApp {
                id,
                provider,
                name,
                origin,
                client_id,
            } => {
                let replies = self
                    .connectors
                    .save_app(id, provider, name, origin, client_id);
                self.answer(client, replies);
            }
            Message::DeleteOauthApp { id } => {
                let replies = self.connectors.delete_app(id);
                self.answer(client, replies);
            }
            Message::SetAppSecret { app, secret } => {
                let replies = self.connectors.set_app_secret(app, secret);
                self.answer(client, replies);
            }
            Message::ClearAppSecret { app } => {
                let replies = self.connectors.clear_app_secret(app);
                self.answer(client, replies);
            }

            // ── Repository family: cloning one into a project ──
            // Every arm here starts a thread and answers nothing itself: a listing is a round
            // trip and a clone is minutes of transfer. A finished clone comes back through
            // `register_clones`, because the catalogue is this thread's.
            Message::ListRepos {
                query_id,
                connection,
                query,
            } => {
                let (asker, _) = self.sinks(client);
                self.repos.list(query_id, connection, query, asker);
            }
            Message::ListRepoBranches { query_id, source } => {
                let (asker, _) = self.sinks(client);
                self.repos.branches(query_id, source, asker);
            }
            Message::CloneRepo { request } => {
                let (asker, _) = self.sinks(client);
                self.repos.clone(client, request, asker);
            }
            Message::CancelClone { clone_id } => self.repos.cancel(clone_id),

            // The vendor bundle a web panel needs. Everything after this — the progress, the
            // answer, the failure — comes from a thread of its own, except the one case that
            // touches no network: a bundle already on disk is answered from here.
            Message::EnsureWebBundle { app } => {
                let (asker, _) = self.sinks(client);
                self.web_assets.ensure(client, &app, asker);
            }

            // The `ubiq` command on PATH. Every path in the exchange is the host's: the interface
            // says which of the three things to do and is told what is there afterwards.
            Message::CliShortcut { action } => {
                let reply = Reply::Asker(cli_shortcut::handle(action));
                self.answer(client, vec![reply]);
            }

            // ── the assist family ───────────────────────────────────
            // Whether a model is there is a fact about this machine, answered from the backend
            // already held — no device is asked twice and no window learns a vendor's words.
            Message::GetAssist => {
                self.answer(client, vec![Reply::Asker(assist::describe(&*self.assist))]);
            }
            // Best effort, and that is the whole contract: the flag stops the reply being
            // forwarded, it does not stop the model. A generation already inside the framework
            // runs to its end and its answer is dropped.
            Message::CancelSuggest { suggest_id } => {
                if let Some(cancel) = self.active_suggests.get(&suggest_id) {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            Message::Suggest {
                suggest_id,
                subject,
            } => self.suggest_job(client, suggest_id, subject),

            // ── the ai provider family ──────────────────────────────
            // Everything but a model refresh is answered from a file and the keychain, on this
            // thread. A refresh is a network call, so `models` puts it on a thread of its own and
            // answers nothing here.
            Message::GetAiProviders => {
                let replies = self.ai_providers.list();
                self.answer(client, replies);
            }
            Message::AddAiProvider { draft, key } => {
                // The mailbox is for the model listing a new provider starts with: the picker the
                // user is about to need cannot be filled before an id and a key exist.
                let asker = self.host.mailbox(To::Client(client));
                let replies = self.ai_providers.add(draft, key, asker);
                self.answer(client, replies);
                self.resettle_assist();
            }
            Message::UpdateAiProvider {
                provider_id,
                draft,
                key,
            } => {
                let replies = self.ai_providers.update(provider_id, draft, key);
                self.answer(client, replies);
                // An edit can change the model or the endpoint the selected provider runs on, so
                // the held backend is rebuilt from what is now on disk.
                self.resettle_assist();
            }
            Message::ForgetAiProvider { provider_id } => {
                let replies = self.ai_providers.forget(provider_id);
                self.answer(client, replies);
                // The delete may have switched assistance off, since a setting cannot point at a
                // record that is gone.
                self.resettle_assist();
            }
            Message::ListAiModels {
                provider_id,
                refresh,
            } => {
                let asker = self.host.mailbox(To::Client(client));
                let replies = self.ai_providers.models(provider_id, refresh, asker);
                self.answer(client, replies);
            }

            Message::GetSettings { layer } => {
                let reply = self.settings.get(layer);
                self.answer(client, vec![reply]);
            }
            Message::SetSettings { layer, value } => {
                let was = self.settings.host().assist;
                let replies = self.settings.set(layer, value);
                // How an agent is confined is acted on at the next spawn, so the settings are
                // re-read here rather than kept in a copy that could go stale.
                let host = self.settings.host();
                self.agents.set_isolate(host.isolate_agents);
                self.agents
                    .set_policy(host.agent_home.clone(), host.extra_grants.clone());
                self.agents.set_commands(host.agent_commands.clone());
                // The assist backend is the exception to re-reading: selecting one can mean
                // opening an FFI handle or reading a key, so it is held and rebuilt only when the
                // provider itself moved.
                if host.assist != was {
                    self.assist =
                        assist::select(&host.assist, &host.ai_providers, &self.ai_providers);
                }
                self.answer(client, replies);
                // The default moved, so every project that never overrode it moved with it. Only
                // open projects are settled: a closed one has no index either way, and builds one
                // when it opens.
                let open: Vec<ProjectId> = self
                    .watchers
                    .keys()
                    .map(|(_, project_id)| *project_id)
                    .collect();
                for project_id in open {
                    self.settle_index(project_id);
                }
            }

            // ── the host browse family ───────────────────────────────
            // No project to look up, and so no syscall here either: the request goes straight to
            // the same worker the file family uses.
            Message::BrowseHostDir { path } => {
                self.browse_job(client, path);
            }

            // ── the file family ─────────────────────────────────────
            // Five arms, no syscall: the record is a lookup in memory and the work goes to the
            // worker with the root it resolved against.
            Message::ProjectTree {
                project_id,
                rel_path,
                depth,
            } => {
                let request = files::Request::Tree {
                    rel_path: rel_path.clone(),
                    depth,
                };
                self.file_job(client, project_id, &rel_path, request);
            }
            Message::ReadProjectFile {
                project_id,
                rel_path,
                max_bytes,
            } => {
                let request = files::Request::Read {
                    rel_path: rel_path.clone(),
                    max_bytes,
                };
                self.file_job(client, project_id, &rel_path, request);
            }
            Message::WriteProjectFile {
                project_id,
                rel_path,
                bytes,
                expected,
            } => {
                let request = files::Request::Write {
                    rel_path: rel_path.clone(),
                    bytes,
                    expected,
                };
                self.file_job(client, project_id, &rel_path, request);
            }
            Message::DiffProjectFile {
                project_id,
                rel_path,
                base,
            } => {
                let request = files::Request::Diff {
                    rel_path: rel_path.clone(),
                    base,
                };
                self.file_job(client, project_id, &rel_path, request);
            }
            Message::EditProjectPath {
                project_id,
                rel_path,
                to,
                op,
            } => {
                let request = files::Request::Edit {
                    rel_path: rel_path.clone(),
                    to,
                    op,
                };
                self.file_job(client, project_id, &rel_path, request);
            }

            // ── the git family ──────────────────────────────────────
            // Two arms, no syscall: the record is a lookup in memory and the work goes to the
            // worker with the root it resolved against. A status walk on this thread would stall
            // every pane behind it.
            Message::ProjectGit { project_id } => {
                self.git_job(client, project_id, git::Request::Overview);
            }
            Message::RefreshProjectGit { project_id, full } => {
                let request = if full {
                    git::Request::Full
                } else {
                    git::Request::Overview
                };
                self.git_job(client, project_id, request);
            }
            Message::ProjectGitRefs {
                project_id,
                with_tracking,
            } => {
                self.git_job(client, project_id, git::Request::Refs { with_tracking });
            }
            Message::ProjectGitLog {
                project_id,
                cursor,
                count,
                rel_path,
                first_parent,
            } => {
                self.git_job(
                    client,
                    project_id,
                    git::Request::Log {
                        cursor,
                        count,
                        rel_path,
                        first_parent,
                    },
                );
            }

            // ── the work family ─────────────────────────────────────
            // Thirteen arms and one helper. Every one names a project, and none of them touches a
            // user's folder — a task file lives under Ubiq's own config root, which the catalogue
            // and the view state already write from this thread.
            Message::ListWork { project_id } => {
                self.work_job(client, project_id, |work| work.list(project_id));
            }
            Message::CreateTask {
                project_id,
                title,
                session,
            } => {
                self.work_job(client, project_id, |work| {
                    work.create(project_id, title, session)
                });
            }
            Message::UpdateTask {
                project_id,
                task_id,
                title,
                description,
                priority,
                shape,
            } => {
                self.work_job(client, project_id, |work| {
                    work.update(project_id, task_id, title, description, priority, shape)
                });
            }
            Message::MoveTask {
                project_id,
                task_id,
                status,
            } => {
                self.work_job(client, project_id, |work| {
                    work.move_task(project_id, task_id, status)
                });
            }
            Message::AssignTask {
                project_id,
                task_id,
                session,
            } => {
                self.work_job(client, project_id, |work| {
                    work.assign(project_id, task_id, session)
                });
            }
            Message::DeleteTask {
                project_id,
                task_id,
            } => {
                self.work_job(client, project_id, |work| work.delete(project_id, task_id));
            }
            Message::AddStep {
                project_id,
                task_id,
                title,
            } => {
                self.work_job(client, project_id, |work| {
                    work.add_step(project_id, task_id, title)
                });
            }
            Message::RenameStep {
                project_id,
                task_id,
                step_id,
                title,
            } => {
                self.work_job(client, project_id, |work| {
                    work.rename_step(project_id, task_id, step_id, title)
                });
            }
            Message::RemoveStep {
                project_id,
                task_id,
                step_id,
            } => {
                self.work_job(client, project_id, |work| {
                    work.remove_step(project_id, task_id, step_id)
                });
            }
            Message::MoveStep {
                project_id,
                task_id,
                step_id,
                to,
            } => {
                self.work_job(client, project_id, |work| {
                    work.move_step(project_id, task_id, step_id, to)
                });
            }
            Message::ToggleStep {
                project_id,
                task_id,
                step_id,
            } => {
                self.work_job(client, project_id, |work| {
                    work.toggle_step(project_id, task_id, step_id)
                });
            }
            Message::AssignAgent {
                project_id,
                agent_id,
                task_id,
            } => {
                self.work_job(client, project_id, |work| {
                    work.assign_agent(project_id, agent_id, task_id)
                });
            }
            Message::SendToAgent {
                project_id,
                agent_id,
                text,
            } => {
                self.work_job(client, project_id, |work| {
                    work.send_to_agent(project_id, agent_id, text)
                });
            }

            Message::StartConversation {
                agent_id,
                project_id,
                session_id,
                rel_path,
                agent_type,
                account,
                profile,
                model,
                thinking,
                mode,
                mcps,
            } => {
                self.start_conversation(
                    client, agent_id, project_id, session_id, rel_path, agent_type, account,
                    profile, model, thinking, mode, mcps,
                );
            }
            Message::PromptAgent { agent_id, text } => {
                // The asked half of the opening exchange, kept before the prompt is handed on
                // and moved. Only the first one: a conversation is named after what it was
                // started for, and a later turn is about where the work has got to.
                if let Some(pending) = self.pending_conversations.get_mut(&agent_id)
                    && !pending.named
                    && pending.opening_prompt.is_none()
                {
                    pending.opening_prompt = Some(text.clone());
                }
                if self.conversations.contains_key(&agent_id) {
                    self.drive(client, agent_id, |conversation| conversation.prompt(text));
                } else {
                    self.launch_pending(client, agent_id, text);
                }
            }
            Message::CancelTurn { agent_id } => {
                self.drive(client, agent_id, |conversation| conversation.cancel());
            }
            Message::AnswerPermission {
                agent_id,
                request_id,
                option_id,
            } => {
                self.drive(client, agent_id, |conversation| {
                    conversation.answer_permission(request_id, option_id)
                });
            }
            Message::SetAgentConfig {
                agent_id,
                config_id,
                value,
            } => {
                // Nothing is running yet: the pick is only remembered, for `launch_pending` to
                // pass as `RunFlags.model` — a model cannot change after launch (see
                // `_docs/wip/agent-setup.md`'s Traps), so a live conversation still goes through
                // `drive`/`Conversation::set_config` exactly as before.
                if self.pending_conversations.contains_key(&agent_id) {
                    if !self.drives(client, agent_id) {
                        return;
                    }
                    let pending = self
                        .pending_conversations
                        .get_mut(&agent_id)
                        .expect("just checked above");
                    if config_id == "model" {
                        // ponytail: re-reads the cache rather than holding the probe's result; a
                        // miss would cost one re-probe and cannot happen — the discovery thread
                        // wrote the cache before it ever sent the `ConfigOptions` that made this
                        // pick possible.
                        let account_key = pending.account.clone().unwrap_or_default();
                        let catalogue =
                            probe_catalogue(&pending.agent_type, &account_key, &self.catalogue)
                                .unwrap_or_default();
                        // The thinking picker the window is *already* showing, rebuilt from the
                        // same inputs that produced it: the previous pick, or — before any pick —
                        // the remembered-or-default resolution the discovery thread used.
                        let (last_model, last_thinking) = self
                            .catalogue
                            .last_used(&pending.agent_type)
                            .unwrap_or_default();
                        let resend = config_options_after_model_pick(
                            &pending.agent_type,
                            &catalogue,
                            &advertised_model(
                                &catalogue,
                                pending.chosen_model.as_deref(),
                                &last_model,
                            ),
                            match pending.chosen_model {
                                // Every re-send resets thinking to the model's own default, so
                                // that is what the last one showed.
                                Some(_) => "",
                                None => &last_thinking,
                            },
                            &value,
                        );

                        pending.chosen_model = Some(value);
                        pending.catalogue = catalogue;
                        if let Some(options) = resend {
                            // Pre-increment, mirroring the pump's own `seq += 1` before it sends:
                            // the discovery thread's message already claimed seq 1, so a resend
                            // is seq 2, and `next_seq` still names "the last seq used" when
                            // `launch_pending` later hands it to `Conversation::start` as the
                            // pump's own starting point.
                            pending.next_seq += 1;
                            let seq = pending.next_seq;
                            self.host.mailbox(To::Client(client)).send(
                                Message::ConversationUpdate {
                                    agent_id,
                                    seq,
                                    update: Box::new(ConvUpdate::ConfigOptions(options)),
                                    raw: None,
                                },
                            );
                        }
                    } else if config_id == "thinking" {
                        pending.chosen_thinking = Some(value);
                    } else if config_id == "mode" {
                        pending.chosen_mode = Some(value);
                    } else {
                        tracing::debug!(
                            agent = %agent_id,
                            config_id,
                            "no such config on an agent that has not launched yet"
                        );
                    }
                    // A pick is part of the recipe, so the row has to hear about it — a relaunch
                    // after a restart runs what the picker last said, not what it started with.
                    self.remember_conversation(agent_id);
                } else {
                    self.drive(client, agent_id, |conversation| {
                        conversation.set_config(config_id, value)
                    });
                }
            }
            Message::EndConversation { agent_id } => {
                // Read before `end_conversation` takes the entry with it: the window that has to
                // hear the conversation is gone for good is whichever one owned it.
                let owner = self.conversation_owners.get(&agent_id).map(|(c, _)| *c);
                self.end_conversation(agent_id, StopReason::Cancelled);
                // The one place the user really means *gone*. `end_conversation` only parks a
                // persistent conversation, so the delete is spelled out here — and it is all three
                // of the run directory, Ubiq's own row and the library's session record. Never one
                // without the others: a row with no directory offers a resume that comes back
                // empty, and a record with no row is a conversation nothing can name. The last two
                // share one directory, so the second call takes both.
                self.agents.retire_agent(agent_id);
                conversation_record::forget(&self.sessions(), agent_id);
                let _ = std::fs::remove_dir_all(self.sessions().join(agent_id.to_string()));
                // `end_conversation` says nothing on its own — `ConversationEnded` would say the
                // transcript stays, which here is exactly wrong. Only the window that had it open
                // has anything to drop.
                if let Some(client) = owner {
                    self.host.send(
                        To::Client(client),
                        Message::ConversationDeleted { agent_id },
                    );
                }
            }
            Message::UnloadConversation { agent_id } => {
                self.unload_conversation(client, agent_id);
            }
            Message::AbortConversation { agent_id } => {
                self.abort_conversation(client, agent_id);
            }
            Message::ResumeConversation { agent_id } => {
                self.resume_conversation(client, agent_id);
            }
            Message::SetConversationPersistent {
                agent_id,
                persistent,
            } => {
                self.set_conversation_persistent(client, agent_id, persistent);
            }
            Message::SetConversationAcceptAll {
                agent_id,
                accept_all,
            } => {
                self.set_conversation_accept_all(client, agent_id, accept_all);
            }
            Message::SetConversationDebugDump {
                agent_id,
                debug_dump,
            } => {
                self.set_conversation_debug_dump(client, agent_id, debug_dump);
            }
            Message::ReviveConversation {
                source,
                agent_id,
                project_id,
                session_id,
            } => {
                self.revive_conversation(client, source, agent_id, project_id, session_id);
            }

            // ── the search family ───────────────────────────────────
            // Two arms. The walk runs on the search worker, not here; this thread only resolves
            // the project and hands over a job.
            Message::SearchProject {
                project_id,
                search_id,
                query,
                scope: _, // v1: `Files` and `Project` search the same thing.
                filter,
            } => {
                self.search_job(client, project_id, search_id, query, filter);
            }
            Message::CancelSearch {
                project_id,
                search_id,
            } => {
                if let Some((active_id, cancel)) = self.active_searches.get(&project_id)
                    && *active_id == search_id
                    && !cancel.load(Ordering::Relaxed)
                {
                    cancel.store(true, Ordering::Relaxed);
                    tracing::info!(search = %search_id, project = %project_id, "search cancelled");
                }
            }

            // Response-direction variants are never received here. Dropping one silently would
            // hide a wiring mistake, so it is named.
            other => {
                tracing::warn!("the coordinator was sent a message only it may send: {other:?}")
            }
        }
    }

    /// Register a conversation the window asked for, without starting its harness.
    ///
    /// P3's ordering: a model reaches a harness only as a launch flag (see
    /// `_docs/wip/agent-setup.md`), so it must be chosen before the harness exists. What used to
    /// be one eager step is now two — this settles the folder, mints the `WorkAgent` and answers
    /// `ConversationStarted` at once, so the window can draw a loader; [`Self::launch_pending`]
    /// is what actually spawns the harness, on the first prompt.
    // One argument per field of `Message::StartConversation` plus `client`: the mirror is the
    // point, and the same shape the rest of this file uses for every family's request.
    #[allow(clippy::too_many_arguments)]
    fn start_conversation(
        &mut self,
        client: ClientId,
        agent_id: AgentId,
        project_id: ProjectId,
        session_id: SessionId,
        rel_path: Option<String>,
        agent_type: String,
        account: Option<String>,
        profile: Option<String>,
        model: Option<String>,
        thinking: Option<String>,
        mode: Option<String>,
        mcps: Vec<String>,
    ) {
        let Some(cwd) = self.resolve_cwd(client, project_id, rel_path.as_deref()) else {
            return;
        };

        if !self.agents.is_agent_type(&agent_type) {
            // A shell is a pane, not a conversation: there is nothing on the other end of a pipe
            // to have a conversation with.
            self.refuse_conversation(
                client,
                agent_id,
                format!("'{agent_type}' is not an agent type"),
            );
            return;
        }
        // A harness the library has no structured bridge for can be *run* — in a pane, where it
        // draws its own screen — but it cannot be conversed with, because nothing turns its
        // output into a transcript. `AgentTypeInfo::chat` keeps it out of every start menu, so
        // reaching here means a stale menu or another client; refusing names the reason rather
        // than leaving a conversation that silently never speaks.
        if !self.agents.converses(&agent_type) {
            self.refuse_conversation(
                client,
                agent_id,
                format!(
                    "'{agent_type}' has no structured bridge, so it cannot hold a conversation — \
                     run it in a terminal pane instead"
                ),
            );
            return;
        }

        let label = self
            .agents
            .types()
            .into_iter()
            .find(|offered| offered.id == agent_type)
            .map(|offered| offered.label)
            .unwrap_or_else(|| agent_type.clone());

        let base = self
            .agents
            .command_of(&agent_type)
            .unwrap_or_else(|| agent_type.clone());
        let name = unique_name(&base, &self.work.live_agent_names(project_id));

        let agent = WorkAgent {
            id: agent_id,
            session: session_id,
            task: None,
            parent: None,
            name,
            summary: None,
            role: "agent".to_string(),
            activity: Activity::Thinking,
            note: String::new(),
            branch: String::new(),
            tokens: 0.0,
            harness: label,
            // No run has happened yet to say which identity actually answered, so this reports
            // what was *asked* for rather than what compose_run resolves — a known, accepted gap
            // while there is no profile UI to make the two differ; see the doc's Traps.
            account: account.clone().unwrap_or_default(),
            // Empty until the harness says which model answered — it is the only thing that
            // knows, and guessing would put a wrong name under a real conversation.
            model: String::new(),
            context_pct: 0,
            persistent: false,
            accept_all: false,
            debug_dump: None,
            thread: Vec::new(),
        };
        // The window's own session, named after the project it is open on: the work's sessions are
        // invented and this one is not, so nothing else in the list would account for this agent.
        let session = WorkSession {
            id: session_id,
            name: self
                .projects
                .record(project_id)
                .map(|record| record.name.clone())
                .unwrap_or_else(|| "session".to_string()),
            branch: String::new(),
            worktree: false,
        };
        self.work
            .add_live_agent(project_id, agent.clone(), session.clone());
        self.conversation_owners
            .insert(agent_id, (client, project_id));

        // Whether this harness can be conversed with is a fact of the harness, not of a running
        // process, so it is known without a bridge — the library answers it.
        //
        // **It is not the same question as "does one process take a second turn".** A one-shot
        // harness accepts every turn it is given; what ends with its answer is the *process*, and
        // the coordinator relaunches it (see `finish_one_shot_turn`). Saying `false` here for one
        // of those is what drew it as `Lifecycle::Ended` before it had spoken at all (`G95`).
        let accepts_input = self.agents.converses(&agent_type);
        // `account` is inert in `discover_models` today, but it is already the cache key's
        // identity leg — captured before `account` moves into the pending record below.
        let account_key = account.clone().unwrap_or_default();
        // A profile's model and mode are seeded into the picks rather than left to `resolve`:
        // `launch_picks` fills `flags.model` from the catalogue's default when `chosen_model`
        // is empty, and a flag outranks the profile — so a profile left unseeded would be
        // shown wrong by the picker *and* launched over. One read, both problems.
        let record = profile.as_deref().and_then(|name| {
            self.agents
                .profiles()
                .unwrap_or_default()
                .into_iter()
                .find(|record| record.id == name)
        });
        // An explicit pick outranks the profile's, the way a pick always does: a flag was the
        // user overriding what the profile set, and the profile's own record is only the
        // fallback when there was no pick.
        let chosen_model = model
            .filter(|value| !value.is_empty())
            .or_else(|| record.as_ref().and_then(|record| record.model.clone()));
        let chosen_thinking = thinking
            .filter(|value| !value.is_empty())
            .or_else(|| record.as_ref().and_then(|record| record.thinking.clone()));
        let chosen_mode = mode
            .filter(|value| !value.is_empty())
            .or_else(|| record.and_then(|record| record.mode.clone()));
        let seeded_model = chosen_model.clone();
        let seeded_mode = chosen_mode.clone().unwrap_or_default();
        let seeded_thinking = chosen_thinking.clone().unwrap_or_default();
        self.pending_conversations.insert(
            agent_id,
            PendingConversation {
                project_id,
                agent_type: agent_type.clone(),
                account,
                profile,
                cwd,
                chosen_model,
                chosen_thinking,
                chosen_mode,
                catalogue: Vec::new(),
                // The discovery thread below always sends the first message this agent_id will
                // ever see, and always as seq 1 — nothing else can race ahead of it.
                next_seq: 1,
                // Nobody has said anything yet, so there is nothing to name it after.
                opening_prompt: None,
                named: false,
                // Nothing has run, so there is no harness session to continue.
                resume: None,
                mcps,
            },
        );
        self.remember_conversation(agent_id);

        let mailbox = self.host.mailbox(To::Client(client));
        mailbox.send(Message::ConversationStarted {
            project_id,
            agent: Box::new(agent),
            session,
            accepts_input,
        });

        // Discover this harness's models (+ reasoning levels) instead of starting it — a one-off
        // thread because probing blocks (it shells out), and the coordinator must keep answering
        // every other window while it runs. `Arc` clone: the thread cannot borrow `&self`.
        let discovery_mailbox = mailbox;
        let cache = self.catalogue.clone();
        thread::Builder::new()
            .name(format!("discover-models-{agent_id}"))
            .spawn(move || {
                let models =
                    probe_catalogue(&agent_type, &account_key, &cache).unwrap_or_else(|error| {
                        // `error:#` is anyhow's full chain — the spawn's own `with_context` names
                        // the binary and asks "is it on PATH?" — plus the resolved `PATH` itself,
                        // so a discovery failure reads as a cause (this binary, this PATH) rather
                        // than the "Default"-only symptom it degrades to below.
                        tracing::warn!(
                            agent = %agent_id,
                            harness = %agent_type,
                            path = %std::env::var("PATH").unwrap_or_default(),
                            "model discovery failed: {error:#}"
                        );
                        Vec::new()
                    });
                let (last_model, last_thinking) = cache.last_used(&agent_type).unwrap_or_default();
                let chosen_model = advertised_model(
                    &models,
                    seeded_model.as_deref().filter(|model| !model.is_empty()),
                    &last_model,
                );
                // A seeded thinking pick (from an explicit `StartConversation` field or the
                // profile record) outranks the remembered last-used level, the same way
                // `chosen_model` outranks `last_model` above.
                let thinking_for_options = if seeded_thinking.is_empty() {
                    &last_thinking
                } else {
                    &seeded_thinking
                };
                let options = build_config_options(
                    &agent_type,
                    &models,
                    &chosen_model,
                    thinking_for_options,
                    &seeded_mode,
                );
                discovery_mailbox.send(Message::ConversationUpdate {
                    agent_id,
                    seq: 1,
                    update: Box::new(ConvUpdate::ConfigOptions(options)),
                    raw: None,
                });
            })
            .ok();
    }

    /// Launch the harness a pending or unloaded agent is still waiting on — the shared core of a
    /// first [`Message::PromptAgent`] (`launch_pending`, below, forwards the prompt afterwards)
    /// and [`Message::ResumeConversation`] (which forwards nothing). `pending` is a clone of the
    /// launch recipe: the row in [`Self::pending_conversations`] itself is left in place, kept for
    /// the next relaunch, and is only ever removed by [`Self::end_conversation`] or by a launch
    /// failure here.
    ///
    /// Returns whether the launch succeeded. A failure has to retract what `start_conversation`
    /// (a first launch) or the previous run (a resume) already made visible — the `WorkAgent`, its
    /// owner, and the recipe itself, since nothing can relaunch a harness that will not compose.
    ///
    /// `first_prompt` is set only for a one-shot harness, whose turn *is* its argv: there is no
    /// pipe to write a prompt into afterwards. A multi-turn harness passes `None` and is prompted
    /// over its bridge once it is running, which is what keeps its launch byte-identical to what
    /// it was.
    /// A project's [`crate::mcp::ProjectFacts`], as both [`Self::launch`] and
    /// [`Self::spawn_workspace`] register them for an agent's MCP row — one reading of the
    /// record is what keeps the two from drifting apart. `Default::default()` for a project
    /// that has gone missing between the spawn and the register, same as either caller lived
    /// with before this was shared.
    fn project_facts(&self, project_id: ProjectId) -> crate::mcp::ProjectFacts {
        self.projects
            .record(project_id)
            .map(|record| crate::mcp::ProjectFacts {
                id: record.id.to_string(),
                name: record.name.clone(),
                path: record.path.clone(),
                colour: record.colour,
            })
            .unwrap_or_default()
    }

    fn launch(
        &mut self,
        client: ClientId,
        agent_id: AgentId,
        pending: PendingConversation,
        first_prompt: Option<String>,
    ) -> bool {
        let (last_model, last_thinking) = self
            .catalogue
            .last_used(&pending.agent_type)
            .unwrap_or_default();
        let (model, thinking) = launch_picks(
            pending.chosen_model.as_deref(),
            pending.chosen_thinking.as_deref(),
            &pending.catalogue,
            &last_model,
            &last_thinking,
        );
        let mode = pending.chosen_mode.filter(|v| !v.is_empty());

        // Tell the MCP listener who this agent is, *before* the harness exists to ask: the run
        // composed below spawns the process, and the first thing a harness does with an injected
        // server is call it. A row here for a launch that then fails is taken out again by the
        // `retire_agent` in the error arm below — the same call that removes its run directory.
        let (name, harness) = self
            .work
            .live_agent_mut(pending.project_id, agent_id)
            .map(|agent| (agent.name.clone(), agent.harness.clone()))
            .unwrap_or_else(|| (pending.agent_type.clone(), pending.agent_type.clone()));
        let project = self.project_facts(pending.project_id);
        self.agents.mcp_agents().register(crate::mcp::AgentFacts {
            key: agent_id.to_string(),
            name,
            harness,
            account: pending.account.clone(),
            model: model.clone(),
            mode: mode.clone(),
            cwd: pending.cwd.display().to_string(),
            // The harness's own session id, and only when this launch is a resume into one — a
            // fresh run mints its id inside the harness and never says it here.
            session: pending.resume.clone(),
            project,
        });

        let (composed, bridge) = match self.agents.converse(
            agent_id,
            &pending.agent_type,
            &pending.cwd,
            ConverseOptions {
                account: pending.account.clone(),
                model: model.clone(),
                thinking: thinking.clone(),
                mode,
                profile: pending.profile.clone(),
                prompt: first_prompt,
                resume: pending.resume.clone(),
                mcps: pending.mcps.clone(),
            },
        ) {
            Ok(started) => started,
            Err(error) => {
                tracing::error!(
                    agent = %agent_id,
                    harness = %pending.agent_type,
                    "starting failed: {error:#}"
                );
                self.agents.retire_agent(agent_id);
                // A run that would not compose has nothing to resume, so its row goes too —
                // otherwise the next boot restores a recipe that is known not to launch.
                conversation_record::forget(&self.sessions(), agent_id);
                self.work.remove_live_agent(pending.project_id, agent_id);
                self.conversation_owners.remove(&agent_id);
                self.pending_conversations.remove(&agent_id);
                self.refuse_conversation(client, agent_id, format!("{error:#}"));
                return false;
            }
        };
        tracing::info!(
            agent = %agent_id,
            harness = %pending.agent_type,
            dir = %composed.dir.display(),
            "conversation started"
        );

        // What actually reached the harness, not what the picker merely showed — a pick the user
        // opened and then abandoned never got here, so it never overwrites what launched.
        self.catalogue.set_last_used(
            &pending.agent_type,
            model.as_deref().unwrap_or(""),
            thinking.as_deref().unwrap_or(""),
        );

        let mailbox = self.host.mailbox(To::Client(client));
        // The launch's own dimensions, resolved once: the pump stamps them on every spend it
        // sees. Without a meter the pump simply records nothing.
        let usage = self.usage.clone().map(|meter| UsageMeter {
            meter,
            project: pending.project_id.to_string(),
            harness: pending.agent_type.clone(),
            account: pending.account.clone().unwrap_or_default(),
        });
        // A one-shot harness's process exits at the end of every turn, so its pump must not
        // announce that as the conversation ending — `finish_one_shot_turn` decides what it
        // means, and says `ConversationUnloaded` instead.
        let quiet = !self.agents.multi_turn(&pending.agent_type);
        // Ubiq's own two overrides, out of the durable row: they are properties of the
        // conversation and not of any one process, so a launch reads them rather than being told
        // them. A conversation with no row yet has neither.
        let held = conversation_record::load(&self.sessions(), agent_id);
        let flags = ConvFlags::new(
            agent_id,
            held.as_ref().is_some_and(|row| row.accept_all),
            held.as_ref().is_some_and(|row| row.debug_dump),
        );
        // The capture the launch just opened, so the window can say where it is. Nothing else
        // reports it: the flag was set while the conversation had no process to open a file.
        let dump_path = flags.dump_path();
        let conversation = Conversation::start(
            agent_id,
            bridge,
            mailbox,
            pending.next_seq,
            usage,
            quiet,
            flags,
        );
        self.conversations.insert(agent_id, conversation);
        if dump_path.is_some() {
            self.publish_conversation_flags(agent_id, |agent| agent.debug_dump = dump_path);
        }
        self.agents_this_run += 1;
        true
    }

    /// Launch a pending agent now that its next prompt has arrived — the moment P3 draws the
    /// line between "asked for" and "running" — then forward that prompt. `ConversationStarted`
    /// already went out when this agent was registered as pending, and `accepts_input` cannot
    /// have changed since — it is the harness's own fact, not the process's.
    ///
    /// **The two harness kinds take their prompt through different doors.** A multi-turn harness
    /// is launched and then written to over its bridge. A one-shot harness has no such door — its
    /// bridge hands out no input sink, so `Conversation::prompt` would no-op into nothing — and
    /// takes the turn in its argv instead. This is also the path a one-shot harness's *second*
    /// turn arrives on, because `finish_one_shot_turn` put it back in `pending_conversations`
    /// with the id its last run resumes from.
    fn launch_pending(&mut self, client: ClientId, agent_id: AgentId, text: String) {
        if !self.drives(client, agent_id) {
            return;
        }
        let Some(pending) = self.pending_conversations.get(&agent_id).cloned() else {
            return;
        };
        let one_shot = !self.agents.multi_turn(&pending.agent_type);
        let argv_prompt = one_shot.then(|| text.clone());
        if !self.launch(client, agent_id, pending, argv_prompt) {
            return;
        }
        if !one_shot {
            self.drive(client, agent_id, |conversation| conversation.prompt(text));
        }
    }

    /// Start an unloaded (or never-launched) conversation's harness again, with no prompt to
    /// forward. Already live is left alone — resuming twice must not spawn a second pump.
    ///
    /// A one-shot harness has nothing to resume *into*: its process is a turn, and a turn with no
    /// prompt is a process that would answer nothing and exit. Its next prompt is what relaunches
    /// it, so this is deliberately a no-op there rather than a run nobody asked for.
    fn resume_conversation(&mut self, client: ClientId, agent_id: AgentId) {
        if self.conversations.contains_key(&agent_id) {
            return;
        }
        if !self.drives(client, agent_id) {
            return;
        }
        let Some(pending) = self.pending_conversations.get(&agent_id).cloned() else {
            return;
        };
        if !self.agents.multi_turn(&pending.agent_type) {
            tracing::debug!(
                agent = %agent_id,
                harness = %pending.agent_type,
                "a one-shot harness resumes with its next prompt, not on its own"
            );
            return;
        };
        self.launch(client, agent_id, pending, None);
    }

    /// Mark a conversation as one to keep, or stop keeping it.
    ///
    /// The flag lives on the durable row and nowhere else, because what it decides happens after
    /// this process has gone: whether closing a window parks the run directory or deletes it, and
    /// whether the next boot's sweep passes it over. The live `WorkAgent` is updated too, but only
    /// so the glyph redraws — nothing reads persistence off it.
    ///
    /// **Refused for a harness with no relocating config lever.** That is grok, which writes its
    /// sessions to the real `~/.grok` whatever `HOME` says, so nothing Ubiq keeps would bring the
    /// conversation back — marking it would promise a resume that cannot happen. The refusal is a
    /// log line naming the reason rather than a silent no-op.
    fn set_conversation_persistent(
        &mut self,
        client: ClientId,
        agent_id: AgentId,
        persistent: bool,
    ) {
        if !self.drives(client, agent_id) {
            return;
        }
        let sessions = self.sessions();
        let Some(mut row) = conversation_record::load(&sessions, agent_id) else {
            tracing::warn!(agent = %agent_id, "no conversation row to mark as persistent");
            return;
        };
        if persistent && !self.agents.forkable(&row.agent_type) {
            tracing::warn!(
                agent = %agent_id,
                harness = %row.agent_type,
                "cannot be kept: it writes its sessions outside the run directory, so nothing \
                 Ubiq keeps would bring it back"
            );
            return;
        }
        row.persistent = persistent;
        conversation_record::save(&sessions, agent_id, &row);

        let Some((_, project_id)) = self.conversation_owners.get(&agent_id).copied() else {
            return;
        };
        let changed = self.work.live_agent_mut(project_id, agent_id).map(|agent| {
            agent.persistent = persistent;
            Box::new(agent.clone())
        });
        if let Some(agent) = changed {
            self.host
                .send(To::Everyone, Message::AgentChanged { project_id, agent });
        }
    }

    /// Answer every permission this conversation asks for, or stop.
    ///
    /// **Ubiq's own override, not the harness's permission mode.** The harness stays in whatever
    /// mode it was launched in and goes on asking; what changes is who answers, and where. A mode
    /// is `Message::SetAgentConfig` and reaches the child process; this never does — which is why
    /// it works the same for a harness whose modes offer nothing like it. The short-circuit itself
    /// is in `conversation.rs`'s pump, on the one path every harness's requests come down.
    ///
    /// Refused for nothing: unlike persistence, this asks nothing of the harness.
    fn set_conversation_accept_all(
        &mut self,
        client: ClientId,
        agent_id: AgentId,
        accept_all: bool,
    ) {
        if !self.drives(client, agent_id) {
            return;
        }
        let sessions = self.sessions();
        let Some(mut row) = conversation_record::load(&sessions, agent_id) else {
            tracing::warn!(agent = %agent_id, "no conversation row to accept permissions on");
            return;
        };
        row.accept_all = accept_all;
        conversation_record::save(&sessions, agent_id, &row);
        // The live harness, where there is one. A conversation with none takes the flag at its
        // next launch, out of the row that was just written.
        if let Some(conversation) = self.conversations.get(&agent_id) {
            conversation.flags().set_accept_all(accept_all);
        }
        self.publish_conversation_flags(agent_id, |agent| agent.accept_all = accept_all);
    }

    /// Write this conversation's traffic to a file, or stop writing it.
    ///
    /// The capture is the process-wide tape narrowed to one agent: the same line shape, in the
    /// same folder, from the first frame rather than the last five hundred. Opening it is the live
    /// conversation's job — `ConvFlags` owns the file and the thread that writes it — and this
    /// only records the wish and reports where it landed.
    ///
    /// **The path is reported on the record, and it is what the menu row reads as the flag.** An
    /// unloaded conversation has no open file, and answering `None` for one would leave the row
    /// marked on disk and drawn off — the toggle would refuse to flip. So the deterministic name
    /// stands in: it is where the traffic lands at the next launch, which is the honest answer to
    /// "where is it going".
    fn set_conversation_debug_dump(
        &mut self,
        client: ClientId,
        agent_id: AgentId,
        debug_dump: bool,
    ) {
        if !self.drives(client, agent_id) {
            return;
        }
        let sessions = self.sessions();
        let Some(mut row) = conversation_record::load(&sessions, agent_id) else {
            tracing::warn!(agent = %agent_id, "no conversation row to dump");
            return;
        };
        row.debug_dump = debug_dump;
        conversation_record::save(&sessions, agent_id, &row);
        let path = match self.conversations.get(&agent_id) {
            // A live harness opens the file here, and answers with the name only if it opened —
            // a capture that was refused must not also claim a path.
            Some(conversation) => conversation.flags().set_debug_dump(debug_dump),
            // No harness to open anything. The name is deterministic, so the window can still say
            // where this conversation's traffic lands the next time it runs — and the row reads
            // as marked, which is what was asked for.
            None => debug_dump.then(|| ConvFlags::dump_path_for(agent_id).display().to_string()),
        };
        if let Some(path) = &path {
            tracing::info!(agent = %agent_id, %path, "conversation capture");
        }
        self.publish_conversation_flags(agent_id, |agent| agent.debug_dump = path);
    }

    /// Mirror a flag onto the live `WorkAgent` and tell every window.
    ///
    /// Shared by the two flags above because the shape is the whole of what they have in common:
    /// the durable row is what any decision reads, and this exists only so the menu that flipped
    /// the flag redraws with it flipped.
    fn publish_conversation_flags(
        &mut self,
        agent_id: AgentId,
        set: impl FnOnce(&mut ubiq_proto::work::WorkAgent),
    ) {
        let Some((_, project_id)) = self.conversation_owners.get(&agent_id).copied() else {
            return;
        };
        let changed = self.work.live_agent_mut(project_id, agent_id).map(|agent| {
            set(agent);
            Box::new(agent.clone())
        });
        if let Some(agent) = changed {
            self.host
                .send(To::Everyone, Message::AgentChanged { project_id, agent });
        }
    }

    /// Put a message the window sent into the capture of the conversation it is about, when that
    /// conversation is recording one.
    ///
    /// **The whole inbound half of a conversation's traffic passes through here**, which is why it
    /// is one call at the top of `dispatch` rather than a line in each handler: a handler that
    /// forgot it would leave a file with answers and no questions. The agent id is lifted out of
    /// the serialised payload the way the tape lifts it, so a message family added later is
    /// captured without this being touched.
    ///
    /// Nothing is serialised unless something is recording, and the terminal families are skipped
    /// outright: a conversation never sends one, and encoding every chunk to find that out is the
    /// stall the bus exists to avoid.
    fn capture_inbound(&self, message: &Message) {
        if matches!(
            message,
            Message::TerminalOutput { .. }
                | Message::TerminalInput { .. }
                | Message::TerminalResize { .. }
        ) {
            return;
        }
        if !self
            .conversations
            .values()
            .any(|conversation| conversation.flags().dumping())
        {
            return;
        }
        let Ok(value) = serde_json::to_value(message) else {
            return;
        };
        let Some(agent_id) = value
            .get("payload")
            .and_then(|payload| payload.get("agent_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(|agent| agent.parse::<AgentId>().ok())
        else {
            return;
        };
        if let Some(conversation) = self.conversations.get(&agent_id) {
            conversation
                .flags()
                .record(ubiq_proto::bus::Direction::Inbound, message);
        }
    }

    /// Bring a conversation back out of a run directory — its own, or a copy of somebody else's.
    ///
    /// `source == agent_id` is a **re-attach**: the same agent, the same kept directory, the same
    /// harness session id. `source != agent_id` is a **fork**: the source's directory is copied and
    /// a second agent resumes the same session inside the copy, so the two share every turn up to
    /// now and diverge from the next one. The source is not touched either way.
    ///
    /// Three things have to be right or the resume silently comes back blank rather than failing:
    ///
    /// - **The directory.** It is the harness's session store, which is why a fork is a copy and
    ///   why `fork_run` refuses a harness that keeps its sessions elsewhere.
    /// - **The cwd.** Claude Code keys its own store by a slug of the working directory, so a
    ///   resume from a different folder finds nothing and starts fresh without saying so. The
    ///   source's `cwd` is turned back into a project-relative path here so it goes through the
    ///   same [`Self::resolve_cwd`] validation a fresh start does.
    /// - **The token.** The harness's own session id, read from the library's `SessionMeta` — the
    ///   one thing [`ConversationRecord`] deliberately does not duplicate.
    ///
    /// A fork does not inherit `persistent`: keeping the original was a decision about the original.
    fn revive_conversation(
        &mut self,
        client: ClientId,
        source: AgentId,
        agent_id: AgentId,
        project_id: ProjectId,
        session_id: SessionId,
    ) {
        // The live recipe first, the row second: a conversation still in this process has picks the
        // row may not have caught up with, and a row is what is left after a restart.
        let facts = self
            .pending_conversations
            .get(&source)
            .map(|pending| {
                (
                    pending.agent_type.clone(),
                    pending.cwd.clone(),
                    pending.account.clone(),
                    pending.profile.clone(),
                    pending.chosen_model.clone(),
                    pending.chosen_thinking.clone(),
                    pending.chosen_mode.clone(),
                )
            })
            .or_else(|| {
                conversation_record::load(&self.sessions(), source).map(|row| {
                    (
                        row.agent_type,
                        row.cwd,
                        row.account,
                        row.profile,
                        row.model,
                        row.thinking,
                        row.mode,
                    )
                })
            });
        let Some((agent_type, cwd, account, profile, model, thinking, mode)) = facts else {
            self.refuse_conversation(
                client,
                agent_id,
                "there is no record of that conversation to bring back".to_string(),
            );
            return;
        };

        let forking = source != agent_id;
        if forking {
            if !self.agents.forkable(&agent_type) {
                self.refuse_conversation(
                    client,
                    agent_id,
                    format!(
                        "{agent_type} keeps its conversations outside the run directory, so a \
                         fork would share one store"
                    ),
                );
                return;
            }
            // ponytail: "is its harness running" stands in for "is it mid-turn" — a `Conversation`
            // exposes no turn state, and copying a store while the harness appends to it tears the
            // last record. Conservative in the safe direction; narrow it to the turn if forking a
            // live, idle conversation is ever asked for.
            if self.conversations.contains_key(&source) {
                self.refuse_conversation(
                    client,
                    agent_id,
                    "that conversation's harness is still running — unload it before forking, or \
                     the copy would catch its session store mid-write"
                        .to_string(),
                );
                return;
            }
            if let Err(error) = self.agents.fork_run(source, agent_id) {
                self.refuse_conversation(client, agent_id, format!("{error:#}"));
                return;
            }
        }

        // Back to a project-relative path, so the start below validates the folder exactly as a
        // fresh one does rather than trusting what a row on disk says.
        let rel_path = match self.projects.record(project_id) {
            Some(record) => match cwd.strip_prefix(&record.path) {
                Ok(rel) if rel.as_os_str().is_empty() => None,
                Ok(rel) => Some(rel.to_string_lossy().into_owned()),
                Err(_) => {
                    self.refuse_conversation(
                        client,
                        agent_id,
                        format!(
                            "{} is not inside this project, and a resume from anywhere else \
                             finds nothing",
                            cwd.display()
                        ),
                    );
                    return;
                }
            },
            None => {
                self.refuse_conversation(client, agent_id, "no such project".to_string());
                return;
            }
        };

        self.start_conversation(
            client,
            agent_id,
            project_id,
            session_id,
            rel_path,
            agent_type,
            account,
            profile,
            model,
            thinking,
            mode,
            Vec::new(),
        );
        // `start_conversation` refuses on its own terms — an unknown harness, an unreadable folder
        // — and says so; there is nothing here to add if it did.
        if !self.pending_conversations.contains_key(&agent_id) {
            return;
        }

        // The token the library holds for the *source*, which is what both paths resume: a
        // re-attach continues its own session, and a fork continues the same session inside its own
        // copy of the store.
        let resume = agent_manager::session::load(&self.sessions(), &source.to_string())
            .ok()
            .and_then(|meta| meta.harness_session_id);
        if resume.is_none() {
            tracing::warn!(
                agent = %agent_id,
                from = %source,
                "no harness session id was recorded; this conversation comes back empty"
            );
        }
        if let Some(pending) = self.pending_conversations.get_mut(&agent_id) {
            pending.resume = resume;
        }
        if forking {
            let sessions = self.sessions();
            if let Some(mut row) = conversation_record::load(&sessions, agent_id) {
                row.forked_from = Some(source);
                conversation_record::save(&sessions, agent_id, &row);
            }
        }

        // Launch it with nothing to forward — already a no-op for a one-shot harness, whose next
        // prompt is what starts its next process.
        self.resume_conversation(client, agent_id);
    }

    /// Kill the harness without ending the conversation. The transcript, the run directory and the
    /// `WorkAgent` all stay; only the pump and its `Conversation` go.
    fn unload_conversation(&mut self, client: ClientId, agent_id: AgentId) {
        if !self.drives(client, agent_id) {
            return;
        }
        let Some(conversation) = self.conversations.remove(&agent_id) else {
            return;
        };
        // `quiet`: the pump skips its own `ConversationEnded` so `ConversationUnloaded`, sent
        // below, is the only lifecycle message this produces.
        let last_seq = conversation.stop(true);
        // One past the last seq actually used, so a relaunched pump's first message continues
        // this conversation's sequence rather than restarting it.
        if let Some(pending) = self.pending_conversations.get_mut(&agent_id) {
            pending.next_seq = last_seq + 1;
        }
        self.remember_conversation(agent_id);
        self.host.send(
            To::Client(client),
            Message::ConversationUnloaded { agent_id },
        );
    }

    /// Unload, without asking the harness first: its process is killed and then reaped, rather
    /// than asked to exit and waited for. Everything else is an unload, down to the
    /// `ConversationUnloaded` — so this shares the whole of that path and differs in one call.
    ///
    /// A harness that does not act on that ask is what this is for. `unload_conversation`'s wait is
    /// this thread's, and this thread answers every window (`G121`), so the difference is felt by
    /// every pane in the application and not only by the conversation being unloaded.
    fn abort_conversation(&mut self, client: ClientId, agent_id: AgentId) {
        if !self.drives(client, agent_id) {
            return;
        }
        let Some(conversation) = self.conversations.remove(&agent_id) else {
            return;
        };
        // Always quiet, for the reason an unload is: `ConversationUnloaded` below is the one
        // lifecycle message an abort produces.
        let last_seq = conversation.abort();
        if let Some(pending) = self.pending_conversations.get_mut(&agent_id) {
            pending.next_seq = last_seq + 1;
        }
        self.remember_conversation(agent_id);
        self.host.send(
            To::Client(client),
            Message::ConversationUnloaded { agent_id },
        );
    }

    /// Hand one conversation-family message to the agent it names.
    ///
    /// A window may only drive an agent it started, on the same terms as a pane; and a harness
    /// that takes no input after launch refuses here rather than swallowing the turn, so a
    /// composer that should not have offered to send finds out.
    fn drive(
        &mut self,
        client: ClientId,
        agent_id: AgentId,
        act: impl FnOnce(&Conversation) -> anyhow::Result<()>,
    ) {
        if !self.drives(client, agent_id) {
            return;
        }
        let Some(conversation) = self.conversations.get(&agent_id) else {
            return;
        };
        if let Err(error) = act(conversation) {
            tracing::warn!("agent {agent_id}: {error:#}");
            self.host.send(
                To::Client(client),
                Message::ConversationError {
                    agent_id,
                    error: format!("{error:#}"),
                },
            );
        }
    }

    /// Whether this window is the one that started the agent.
    fn drives(&self, client: ClientId, agent_id: AgentId) -> bool {
        match self.conversation_owners.get(&agent_id) {
            Some((owner, _)) if *owner == client => true,
            Some(_) => {
                tracing::warn!(
                    "{client} sent a message about agent {agent_id}, which it does not own"
                );
                false
            }
            // The agent has already gone; its last messages are in flight behind it.
            None => false,
        }
    }

    /// Where the durable conversation rows live.
    ///
    /// Composed from the same config root [`Agents`] was built with, because it composes the same
    /// path privately (`Agents::sessions_dir`). **The two must agree**: the row written here is the
    /// one `Agents::is_persistent` reads when the boot sweep decides whether a run directory is
    /// parked or deleted, and a row written somewhere else would have the sweep delete every
    /// conversation the user asked to keep.
    fn sessions(&self) -> PathBuf {
        self.root.path.join("sessions")
    }

    /// Write down one conversation's launch recipe, as it stands now.
    ///
    /// Called from every site that changes what a relaunch would do — the start, a `SetAgentConfig`
    /// pick, and each place `next_seq` moves on — rather than only at teardown, because a `kill -9`
    /// is exactly the case the row exists to survive. One helper rather than a construction at each
    /// site: a field added to [`ConversationRecord`] and filled in four places out of five is a row
    /// that quietly disagrees with itself depending on what happened last.
    ///
    /// Three fields are *not* the coordinator's recipe and are carried over from the row already on
    /// disk rather than rebuilt: `title`, written by the naming thread; `persistent`, written by the
    /// user through [`Message::SetConversationPersistent`]; and `forked_from`, stamped once by a
    /// revive. A sequence bump must not reset any of them.
    fn remember_conversation(&self, agent_id: AgentId) {
        let Some(pending) = self.pending_conversations.get(&agent_id) else {
            return;
        };
        let sessions = self.sessions();
        let held = conversation_record::load(&sessions, agent_id);
        let record = ConversationRecord {
            project_id: pending.project_id,
            agent_type: pending.agent_type.clone(),
            cwd: pending.cwd.clone(),
            account: pending.account.clone(),
            profile: pending.profile.clone(),
            model: pending.chosen_model.clone(),
            thinking: pending.chosen_thinking.clone(),
            mode: pending.chosen_mode.clone(),
            next_seq: pending.next_seq,
            title: held.as_ref().and_then(|row| row.title.clone()),
            persistent: held.as_ref().is_some_and(|row| row.persistent),
            accept_all: held.as_ref().is_some_and(|row| row.accept_all),
            debug_dump: held.as_ref().is_some_and(|row| row.debug_dump),
            forked_from: held.and_then(|row| row.forked_from),
            // What the run was composed under, read from the settings rather than from `Agents`.
            // **The two must agree**: `Agents::set_policy` was handed this same field, and is given
            // it again whenever a `SetSettings` changes it, so a row that named the other one would
            // claim a run answered from a different machine's worth of state than it did.
            agent_home: self.settings.host().agent_home.clone(),
        };
        conversation_record::save(&sessions, agent_id, &record);
    }

    /// Write down each live conversation's harness session id the moment it names one.
    ///
    /// The id is the whole of what a resume needs, and nothing else writes it before teardown —
    /// which makes a `kill -9` the case that loses it, and a crash is precisely when a user wants
    /// their conversation back. So it is recorded here, on the same poll as the naming pass, rather
    /// than at the end.
    ///
    /// This makes [`Self::finish_one_shot_turn`]'s own assignment to `pending.resume` redundant: the
    /// poll that reaps a finished one-shot turn runs this first. It is left in place because it
    /// costs nothing and keeps that method readable on its own.
    /// Every [`LOGIN_SYNC_EVERY`], hand each account's newest credential back to the account and
    /// to every run still holding an older one ([`Agents::sync_logins`]).
    ///
    /// On the coordinator's own thread, which is only allowable because the common tick reads a
    /// few hundred bytes per live run and writes nothing: a credential changes when a harness
    /// refreshes, which is hours apart. If a keychain-backed origin is ever seen making this
    /// stall, it is the write half that would move to a thread, not the read.
    fn sync_logins_due(&mut self) {
        if self.logins_synced.elapsed() < LOGIN_SYNC_EVERY {
            return;
        }
        self.logins_synced = Instant::now();
        self.agents.sync_logins();
    }

    fn remember_sessions(&mut self) {
        let learned: Vec<(AgentId, String)> = self
            .conversations
            .iter()
            .filter_map(|(agent_id, conversation)| {
                let id = conversation.session_id()?;
                let pending = self.pending_conversations.get(agent_id)?;
                (pending.resume.as_deref() != Some(id.as_str())).then_some((*agent_id, id))
            })
            .collect();
        for (agent_id, id) in learned {
            tracing::debug!(agent = %agent_id, session = %id, "the harness named its session");
            self.agents.remember_session(agent_id, &id);
            if let Some(pending) = self.pending_conversations.get_mut(&agent_id) {
                pending.resume = Some(id);
            }
        }
    }

    /// Name every conversation whose agent has now answered its opening prompt.
    ///
    /// **Called from the run loop immediately before [`Self::reap_conversations`], and the order
    /// is the whole of why this works for a one-shot harness.** Such a harness publishes its
    /// first reply and exits in the same breath, so the poll that would reap it is the same poll
    /// that has to take the naming — reaping first would drop the `Conversation` holding the
    /// reply before anything had read it.
    ///
    /// The setting is re-read here rather than cached, the same way every other host setting is:
    /// a user who switches naming off mid-conversation has switched it off.
    fn name_conversations(&mut self) {
        if !self.settings.host().auto_name_conversations {
            return;
        }
        let ready: Vec<(AgentId, String, String)> = self
            .conversations
            .iter()
            .filter_map(|(agent_id, conversation)| {
                let pending = self.pending_conversations.get(agent_id)?;
                if pending.named {
                    return None;
                }
                let asked = pending.opening_prompt.clone()?;
                let answered = conversation.first_reply()?;
                Some((*agent_id, asked, answered))
            })
            .collect();

        for (agent_id, asked, answered) in ready {
            // Marked before the job runs, not after it answers: this is what makes the naming
            // once-per-conversation, and it has to hold for a job that fails as much as for one
            // that succeeds.
            if let Some(pending) = self.pending_conversations.get_mut(&agent_id) {
                pending.named = true;
                pending.opening_prompt = None;
            }
            self.name_job(agent_id, asked, answered);
        }
    }

    /// Ask the selected provider for a title and a five-word summary of one conversation.
    ///
    /// **A failure is a log line and nothing else.** Every other suggestion is something a user
    /// asked for and is owed an answer about; this one is Ubiq's own idea, and the mechanical
    /// name is still on the record either way — so an unreachable provider leaves a conversation
    /// called `claude 2` rather than putting an error where nobody asked a question.
    ///
    /// On its own thread because generation blocks and the coordinator answers every window from
    /// one. It needs no deadline of its own, unlike [`Self::suggest_job`]: nothing is waiting on
    /// this, so a slow model costs a parked thread rather than a spinner that never stops.
    fn name_job(&mut self, agent_id: AgentId, asked: String, answered: String) {
        let backend = self.assist.clone();
        if let Some(unavailable) = backend.availability() {
            tracing::debug!(
                agent = %agent_id,
                reason = unavailable.reason.code(),
                "not naming this conversation",
            );
            return;
        }
        // The window that owns the conversation, which is the one drawing its tab. A naming is
        // not a project-wide fact: another window showing the same project has no tab for it.
        let Some((client, _)) = self.conversation_owners.get(&agent_id).copied() else {
            return;
        };
        let mailbox = self.host.mailbox(To::Client(client));
        // The title lands on this thread and nowhere else — nothing sends it back to the
        // coordinator — so this is where it reaches the durable row. Read-modify-write rather than
        // a rebuild: everything else on the row is the coordinator's, and this thread has none of
        // it. [`Self::remember_conversation`] carries the title over for the same reason.
        let sessions = self.sessions();

        let spawned = thread::Builder::new()
            .name(format!("name-{agent_id}"))
            .spawn(move || {
                let request =
                    assist::subject::conversation_title(&asked, &answered, &backend.limits());
                match backend.generate(request) {
                    Ok(answer) => match assist::subject::naming(&answer) {
                        Some((title, summary)) => {
                            tracing::debug!(agent = %agent_id, %title, "named a conversation");
                            if let Some(mut row) = conversation_record::load(&sessions, agent_id) {
                                row.title = Some(title.clone());
                                conversation_record::save(&sessions, agent_id, &row);
                            }
                            mailbox.send(Message::ConversationNamed {
                                agent_id,
                                title,
                                summary,
                            });
                        }
                        None => tracing::debug!(
                            agent = %agent_id,
                            "the naming answered nothing a title could be read out of",
                        ),
                    },
                    Err(error) => {
                        tracing::debug!(agent = %agent_id, %error, "a naming failed");
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(agent = %agent_id, %error, "could not start a naming thread");
        }
    }

    /// Reap conversations whose harness ended on its own — a one-shot harness finishing its turn,
    /// or any harness's process exiting unasked — rather than by an explicit `EndConversation`.
    ///
    /// Without this, a harness that exits leaks its
    /// `conversations`/`conversation_owners`/`work.live` rows and its run directory — credentials
    /// included — until the window closes. The same shape `active_searches` already uses: a flag
    /// the worker sets on its way out, polled here rather than raced against.
    ///
    /// **What an exit means depends on the harness.** For a multi-turn one it is the conversation
    /// over. For a one-shot one it is a turn over, and the two must not be confused: reaping the
    /// second as the first is what left copilot and opencode looking dead the moment they were
    /// started (`G95`).
    fn reap_conversations(&mut self) {
        let ended: Vec<AgentId> = self
            .conversations
            .iter()
            .filter(|(_, conversation)| conversation.ended())
            .map(|(agent_id, _)| *agent_id)
            .collect();
        for agent_id in ended {
            let one_shot = self
                .pending_conversations
                .get(&agent_id)
                .is_some_and(|pending| !self.agents.multi_turn(&pending.agent_type));
            if one_shot {
                self.finish_one_shot_turn(agent_id);
                continue;
            }
            tracing::debug!("agent {agent_id}'s harness ended on its own; reaping");
            self.end_conversation(agent_id, StopReason::EndTurn);
        }
    }

    /// A one-shot harness answered and exited. Put the conversation back where the *next* prompt
    /// can relaunch it, rather than ending it.
    ///
    /// This is [`Self::unload_conversation`] with nobody having asked: the same three things
    /// happen — the `Conversation` goes, the transcript's sequence is carried forward on the
    /// pending row, and the window is told `ConversationUnloaded` — plus the one thing that makes
    /// the next launch a *continuation*: the harness's own session id, which the next
    /// `PromptAgent` hands back as the run's resume. The pump was started `quiet`, so no
    /// `ConversationEnded` raced this.
    ///
    /// A harness that named no session id is still put back: it will answer the next prompt with
    /// no memory of this one, which is the honest limit of what a harness that reports no
    /// resumable id can do, and better than a conversation that refuses to take another turn.
    fn finish_one_shot_turn(&mut self, agent_id: AgentId) {
        let Some(conversation) = self.conversations.remove(&agent_id) else {
            return;
        };
        let session_id = conversation.session_id();
        // `true` matches what the pump was already started with; a `Conversation::stop` that
        // cleared it would let a pump still on its way out speak after all.
        let last_seq = conversation.stop(true);
        let Some(pending) = self.pending_conversations.get_mut(&agent_id) else {
            return;
        };
        // One past the last seq actually used, so the next process's first message continues this
        // transcript's sequence rather than restarting it — a restarted sequence reads as a gap
        // and the window draws it as one.
        pending.next_seq = last_seq + 1;
        // Only ever overwritten by a newer id: a run that reported none must not erase the one
        // that lets the conversation continue. Redundant since `remember_sessions` — the same poll
        // runs that first — and kept because it costs nothing and says here what a resume needs.
        if session_id.is_some() {
            pending.resume = session_id;
        }
        tracing::debug!(
            agent = %agent_id,
            harness = %pending.agent_type,
            resume = ?pending.resume,
            "a one-shot harness finished its turn; the conversation stays"
        );
        self.remember_conversation(agent_id);
        if let Some((client, _)) = self.conversation_owners.get(&agent_id).copied() {
            self.host.send(
                To::Client(client),
                Message::ConversationUnloaded { agent_id },
            );
        }
    }

    /// Stop an agent and let go of everything the process owned.
    ///
    /// The pump answers its own `ConversationEnded` on the way out, so nothing is said here: two
    /// endings for one agent would leave a window unsure which it was.
    ///
    /// **This is not a delete.** Both ways in are a conversation that may come back —
    /// [`Self::client_gone`] closing a window, and [`Self::reap_conversations`] seeing a harness
    /// exit on its own — so a conversation the user marked persistent is *parked*: its run
    /// directory stays, its login is scrubbed, and its row is left on disk for a
    /// [`Message::ReviveConversation`] to read. Only [`Message::EndConversation`] means gone, and
    /// that arm retires and forgets explicitly on top of this.
    fn end_conversation(&mut self, agent_id: AgentId, reason: StopReason) {
        if let Some(conversation) = self.conversations.remove(&agent_id) {
            tracing::info!("agent {agent_id} ending: {reason:?}");
            conversation.stop(false);
        }
        // A pending agent has no `Conversation` and no run directory yet — closed here, this
        // already covers "closed before it ever launched" without a second path.
        self.pending_conversations.remove(&agent_id);
        if let Some((_, project_id)) = self.conversation_owners.remove(&agent_id) {
            self.work.remove_live_agent(project_id, agent_id);
        }
        // The run directory *is* the harness's session store — Claude's `projects/<slug>/*.jsonl`,
        // Codex's rollouts, opencode's data dir — so keeping it is the whole of what lets this
        // conversation be resumed later. A conversation the user marked persistent keeps it, minus
        // the login it was seeded with; one nobody asked to keep is deleted as it always was, and
        // the credentials seeded into it go with it.
        if conversation_record::load(&self.sessions(), agent_id).is_some_and(|row| row.persistent) {
            self.agents.park_agent(agent_id);
        } else {
            self.agents.retire_agent(agent_id);
        }
    }

    /// Tell the window that asked that its agent never started.
    fn refuse_conversation(&self, client: ClientId, agent_id: AgentId, error: String) {
        self.host.send(
            To::Client(client),
            Message::ConversationError { agent_id, error },
        );
    }

    /// Hand one work-family message to the work.
    ///
    /// The only thing this decides is whether the project exists, and it is not a formality: a task
    /// file written for an id the catalogue does not hold would be collected as an orphan at the
    /// next boot, so the write must never happen. The record is a lookup in memory, so this costs
    /// no syscall — the same as `file_job`.
    fn work_job(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        change: impl FnOnce(&mut Work) -> Vec<Reply>,
    ) {
        if self.projects.record(project_id).is_none() {
            self.host.send(
                To::Client(client),
                Message::WorkError {
                    project_id,
                    task_id: None,
                    error: "no such project".to_string(),
                },
            );
            return;
        }
        let replies = change(&mut self.work);
        self.answer(client, replies);
    }

    /// Hand one file-family request to the worker.
    ///
    /// The only thing this decides is which folder the request is against; a project the catalogue
    /// does not hold is refused here rather than reaching a thread that could not answer it.
    fn file_job(
        &self,
        client: ClientId,
        project_id: ProjectId,
        rel_path: &str,
        request: files::Request,
    ) {
        let Some(record) = self.projects.record(project_id) else {
            self.host.send(
                To::Client(client),
                files::file_error(
                    project_id,
                    rel_path,
                    FileError::Refused("no such project".to_string()),
                ),
            );
            return;
        };

        self.files.submit(files::Job {
            kind: files::JobKind::File {
                project_id,
                root: PathBuf::from(&record.path),
                request,
            },
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    /// Hand one host-browse request to the worker.
    ///
    /// No project to look up first: unlike [`Self::file_job`], the whole point of this family is
    /// browsing before a project's record exists.
    fn browse_job(&self, client: ClientId, path: Option<String>) {
        self.files.submit(files::Job {
            kind: files::JobKind::Browse { path },
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    /// Try a typed command line on its own thread, so a hung or slow binary cannot stall the
    /// coordinator — the same reason [`Self::suggest_job`] runs a backend off this thread.
    /// Answered only to the client that asked; nothing here is persisted or fed back into
    /// `self.agents`, which only ever learns an override through `SetSettings`.
    fn check_agent_command_job(&self, client: ClientId, agent_type: String, command: String) {
        let mailbox = self.host.mailbox(To::Client(client));
        thread::Builder::new()
            .name(format!("check-agent-{agent_type}"))
            .spawn(move || {
                let (ok, detail) = crate::agent::check_command(&command);
                mailbox.send(Message::AgentCommandChecked {
                    agent_type,
                    ok,
                    detail,
                });
            })
            .ok();
    }

    /// Answer [`Message::ListHarnessCatalogue`] on a one-off thread — same shape as the
    /// discovery thread inside [`Self::start_conversation`], because the same probe backs both:
    /// probing shells out and the coordinator must keep answering every other window while it
    /// runs. A probe failure sends an empty `models` list rather than nothing: an unanswerable
    /// harness offers "whatever it defaults to" rather than leaving the asker waiting forever.
    fn list_harness_catalogue_job(
        &self,
        client: ClientId,
        agent_type: String,
        account: Option<String>,
    ) {
        let mailbox = self.host.mailbox(To::Client(client));
        let cache = self.catalogue.clone();
        thread::Builder::new()
            .name(format!("harness-catalogue-{agent_type}"))
            .spawn(move || {
                let account_key = account.clone().unwrap_or_default();
                let models =
                    probe_catalogue(&agent_type, &account_key, &cache).unwrap_or_else(|error| {
                        tracing::warn!(
                            harness = %agent_type,
                            path = %std::env::var("PATH").unwrap_or_default(),
                            "harness catalogue probe failed: {error:#}"
                        );
                        Vec::new()
                    });
                let (last_model, last_thinking) = cache.last_used(&agent_type).unwrap_or_default();
                let models = models
                    .into_iter()
                    .map(|model| CatalogueModel {
                        id: model.id,
                        description: model.description,
                        default: model.default,
                        levels: model
                            .levels
                            .into_iter()
                            .map(|level| ConfigChoice {
                                value: level.value,
                                name: level.label,
                                description: level.description,
                                group: None,
                            })
                            .collect(),
                        default_level: model.default_level,
                    })
                    .collect();
                mailbox.send(Message::HarnessCatalogue {
                    agent_type,
                    account,
                    models,
                    last_model,
                    last_thinking,
                });
            })
            .ok();
    }

    /// Hand one git-family request to the worker.
    ///
    /// The only thing this decides is which folder the request is against; a project the catalogue
    /// does not hold is refused here rather than reaching a thread that could not answer it.
    fn git_job(&self, client: ClientId, project_id: ProjectId, request: git::Request) {
        let Some(record) = self.projects.record(project_id) else {
            self.host.send(
                To::Client(client),
                git::git_error(project_id, ubiq_proto::git::GitError::NotFound),
            );
            return;
        };

        self.git.submit(git::Job {
            project_id,
            root: PathBuf::from(&record.path),
            request,
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    fn git_forget(&self, client: ClientId, project_id: ProjectId) {
        self.git.submit(git::Job {
            project_id,
            root: PathBuf::new(),
            request: git::Request::Forget,
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    /// Start watching a project a window just opened, replacing whatever that window watched
    /// before. A watch that will not start is logged and nothing else: the project is simply not
    /// live, and every other answer still works.
    fn watch_project(&mut self, client: ClientId, project_id: ProjectId) {
        // A window shows one project at a time and there is no `CloseProject`, so opening the next
        // one is the only signal that the previous watch is unwanted.
        self.watchers
            .retain(|(owner, watched), _| *owner != client || *watched == project_id);
        if self.watchers.contains_key(&(client, project_id)) {
            return;
        }

        let Some(record) = self.projects.record(project_id) else {
            return;
        };
        let mut excludes = self.settings.host().search_excludes;
        excludes.extend(record.search_excludes.iter().cloned());
        let root = PathBuf::from(&record.path);

        // The index is built when a project opens, not at boot: an index for a project nobody
        // has open is work nobody asked for. A second open finds it already there, and a level of
        // `none` builds nothing — the watch below still runs, because what a level decides is
        // what is kept and not what is noticed.
        if self.index_level(project_id).keeps_text() {
            self.index.submit(crate::index::Job::Build {
                project_id,
                root: root.clone(),
                home: self.projects.index_dir(project_id),
                excludes: excludes.clone(),
            });
        }

        match watch::start(watch::Job {
            project_id,
            root,
            excludes,
            // The watcher's own batches reach the index directly. The push to the interface goes
            // out on the mailbox below and the coordinator never sees it, so it cannot forward
            // what it is not given.
            index: Some(self.index.sender()),
            reply_to: self.host.mailbox(To::Client(client)),
        }) {
            Ok(watcher) => {
                self.watchers.insert((client, project_id), watcher);
            }
            Err(error) => {
                tracing::warn!(project = %project_id, %error, "no filesystem watch for this project");
            }
        }
    }

    /// The indexing level this project actually gets: its own override, or the application-wide
    /// default when it has none.
    ///
    /// The one place the question is answered, so what the settings dialog promises and what the
    /// index thread does can never come apart.
    fn index_level(&self, project_id: ProjectId) -> IndexLevel {
        self.projects
            .record(project_id)
            .and_then(|record| record.index)
            .unwrap_or_else(|| self.settings.host().index_level)
    }

    /// Make this project's index match its level, now.
    ///
    /// Called when a level could have moved — the project's own override changed, or the
    /// application default did. Building an index that already exists is a no-op on the index
    /// thread, and dropping one that was never built is too, so this is safe to call whenever the
    /// answer might have changed rather than only when it did.
    fn settle_index(&mut self, project_id: ProjectId) {
        let level = self.index_level(project_id);
        if !level.keeps_text() {
            self.index.submit(crate::index::Job::Drop(project_id));
            return;
        }
        let Some(record) = self.projects.record(project_id) else {
            return;
        };
        let mut excludes = self.settings.host().search_excludes;
        excludes.extend(record.search_excludes.iter().cloned());
        self.index.submit(crate::index::Job::Build {
            project_id,
            root: PathBuf::from(&record.path),
            home: self.projects.index_dir(project_id),
            excludes,
        });
    }

    /// Rebuild the held backend from what is now on disk.
    ///
    /// Called after every write to the provider records, because each of them changes what the
    /// selected provider *is*: an edit moves its model, its endpoint or its key, and a delete
    /// switches assistance off, since a setting cannot point at a record that is gone.
    /// Unconditional, unlike the `SetSettings` path that compares first — a provider write is a
    /// deliberate act on the exact thing the backend was built from.
    fn resettle_assist(&mut self) {
        let host = self.settings.host();
        self.assist = assist::select(&host.assist, &host.ai_providers, &self.ai_providers);
    }

    /// Answer one [`Message::Suggest`].
    ///
    /// Generation blocks — an FFI hop into a model, or an HTTP round trip — so it happens on a
    /// one-off named thread holding a mailbox, the same shape model discovery uses: the
    /// coordinator must keep answering every other window while a model writes a sentence. The
    /// repository read happens there too, for the reason the whole git family has its own thread:
    /// a status walk is seconds on a large tree, and seconds here stall every pane's keystrokes.
    ///
    /// The text is forwarded as it arrives, as [`Message::SuggestChunk`], and then whole as the
    /// [`Message::Suggestion`] that ends the id. A backend that cannot stream sends one chunk and
    /// then the same text, so the interface reads one shape either way.
    fn suggest_job(&mut self, client: ClientId, suggest_id: SuggestId, subject: SuggestSubject) {
        // Reap suggestions that are over — the flag means both "cancelled" and "finished", set by
        // the worker on its way out. This is the one place `active_suggests` gains an entry, so it
        // is the one place that drops stale ones.
        self.active_suggests
            .retain(|_, over| !over.load(Ordering::Relaxed));

        let mailbox = self.host.mailbox(To::Client(client));
        let fail = |error: String| {
            mailbox.send(Message::SuggestError { suggest_id, error });
        };

        // Which backend answers, and what it reads. Both are settled here because the catalogue
        // and the settings live on this thread; the material itself is gathered by the worker.
        //
        // A check is the one subject that names its own backend instead of using the held one: a
        // user checking a key they have just typed is asking about *that* provider, not about
        // whichever one the setting points at. It is built for this one request and dropped with
        // it, which is why `self.assist` is left alone.
        let (backend, root) = match &subject {
            SuggestSubject::CommitMessage { project_id } => {
                match self.projects.record(*project_id) {
                    Some(record) => (self.assist.clone(), Some(PathBuf::from(&record.path))),
                    None => return fail("that project is not in the catalogue".to_string()),
                }
            }
            SuggestSubject::ProviderCheck { provider_id, .. } => {
                let host = self.settings.host();
                let named = AssistProvider::Api {
                    provider_id: *provider_id,
                };
                (
                    assist::select(&named, &host.ai_providers, &self.ai_providers),
                    None,
                )
            }
        };

        if let Some(unavailable) = backend.availability() {
            return fail(assist::unavailable_message(&unavailable));
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.active_suggests.insert(suggest_id, cancel.clone());

        thread::Builder::new()
            .name(format!("suggest-{suggest_id}"))
            .spawn(move || {
                let chunks = mailbox.clone();
                let answer = gather(&subject, root.as_deref(), &*backend).and_then(|request| {
                    // A deadline, because nothing below can be interrupted from here: a model
                    // that hangs would otherwise leave the window waiting forever, and a
                    // mechanical name is better than a control that never comes back. The
                    // generation itself runs on one more thread only so this one can stop waiting
                    // for it — and it holds the mailbox, so chunks reach the window while this
                    // thread is still inside `recv_timeout`.
                    let (done, waiting) = std::sync::mpsc::channel();
                    let stop = cancel.clone();
                    thread::Builder::new()
                        .name(format!("suggest-run-{suggest_id}"))
                        .spawn(move || {
                            let mut sink = |text: &str| {
                                // Two reasons to stop, and both mean the same thing to a
                                // backend: the suggestion was given up on, or the window that
                                // asked for it has gone and `send` says so.
                                if stop.load(Ordering::Relaxed) {
                                    return false;
                                }
                                chunks.send(Message::SuggestChunk {
                                    suggest_id,
                                    text: text.to_string(),
                                })
                            };
                            let _ = done.send(backend.stream(request, &mut sink));
                        })
                        .map_err(|error| format!("assist thread: {error}"))?;
                    waiting
                        .recv_timeout(SUGGEST_DEADLINE)
                        .unwrap_or_else(|_| Err("the model did not answer in time".to_string()))
                });

                // Cancelled: the answer is dropped rather than forwarded. Setting the flag here
                // as well is what marks this suggestion over, for the reaping above.
                if !cancel.swap(true, Ordering::Relaxed) {
                    mailbox.send(match answer {
                        Ok(text) => Message::Suggestion {
                            suggest_id,
                            text: text.trim().to_string(),
                        },
                        Err(error) => Message::SuggestError { suggest_id, error },
                    });
                }
            })
            .ok();
    }

    fn search_job(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        search_id: SearchId,
        query: ubiq_proto::search::Query,
        filter: ubiq_proto::search::Filter,
    ) {
        // Reap searches that have finished — the flag also means "this search is over", set by
        // the worker itself. This is the one place `active_searches` gains an entry, so it is the
        // one place that needs to drop stale ones.
        self.active_searches
            .retain(|_, (_, over)| !over.load(Ordering::Relaxed));

        let Some(record) = self.projects.record(project_id) else {
            self.host.send(
                To::Client(client),
                Message::SearchError {
                    project_id,
                    search_id,
                    error: ubiq_proto::search::SearchError::Root,
                },
            );
            return;
        };

        // The application-wide excludes and this project's own, merged here because this is the
        // one place holding both `Settings` and the project record.
        let host_settings = self.settings.host();
        let mut excludes = host_settings.search_excludes;
        excludes.extend(record.search_excludes.iter().cloned());

        // Cancel any active search for this project.
        if let Some((superseded, cancel)) = self.active_searches.remove(&project_id) {
            cancel.store(true, Ordering::Relaxed);
            tracing::info!(
                search = %superseded,
                by = %search_id,
                project = %project_id,
                "search superseded"
            );
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.active_searches
            .insert(project_id, (search_id, cancel.clone()));

        self.search.submit(search::Job {
            project_id,
            search_id,
            root: PathBuf::from(&record.path),
            query,
            filter,
            excludes,
            fallbacks: host_settings.search_fallbacks,
            // Absent until the project's index has been built, and absent for good when its
            // level says to keep none. Either way the worker walks.
            index: self.index.reader(project_id),
            cancel,
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    // One argument per field of `Message::SpawnWorkspace` plus `client`, the same shape
    // `start_conversation`'s own mirror of its message takes.
    #[allow(clippy::too_many_arguments)]
    fn spawn_workspace(
        &mut self,
        client: ClientId,
        session_id: SessionId,
        project_id: ProjectId,
        rel_path: Option<String>,
        agent_type: Option<String>,
        args: Vec<String>,
        picks: AgentPicks,
    ) {
        // A pane runs in a project's folder, so everything about that folder is settled before a
        // pseudo-terminal exists. A spawn that fails here leaves nothing on screen to close.
        let cwd = match self.resolve_cwd(client, project_id, rel_path.as_deref()) {
            Some(cwd) => cwd,
            None => return,
        };

        let pane_id = PaneId::generate();
        let agent_type = agent_type.unwrap_or_else(shells::default_program);

        // Kept past the move below, for the MCP row registered once compose succeeds — that
        // wants the picked model and mode too, and `options` does not outlive the `compose` call.
        let picked_model = picks.model.clone();
        let picked_mode = picks.mode.clone();
        // `prompt` and `resume` stay `None`: a terminal harness is prompted by the user typing
        // into it, not by an argv-carried first turn, and resuming one is not part of this
        // change.
        let options = ConverseOptions {
            account: picks.account,
            model: picks.model,
            thinking: picks.thinking,
            mode: picks.mode,
            profile: picks.profile,
            mcps: picks.mcps,
            prompt: None,
            resume: None,
        };
        // An agent type the library knows is composed — its skills, its throwaway configuration
        // and the policy it runs under all come from there. Anything else is a program name,
        // which is what a shell is.
        let composed = if self.agents.is_agent_type(&agent_type) {
            match self
                .agents
                .compose(pane_id, &agent_type, &cwd, args.clone(), options)
            {
                Ok(composed) => Some(composed),
                Err(error) => {
                    tracing::error!("pane {pane_id}: composing {agent_type} failed: {error:#}");
                    self.refuse_pane(client, pane_id, format!("{error:#}"));
                    return;
                }
            }
        } else {
            None
        };

        // Tell the MCP listener who this pane is, before the harness exists to ask — the same
        // reason `launch` registers a conversation before its own composed run spawns. A pane has
        // no card and no name of its own, so both `name` and `harness` are the resolved agent
        // type; `account` is what the run actually resolved to, not merely what was picked.
        if let Some(composed) = &composed {
            let project = self.project_facts(project_id);
            self.agents.mcp_agents().register(crate::mcp::AgentFacts {
                key: pane_id.to_string(),
                name: agent_type.clone(),
                harness: agent_type.clone(),
                account: composed.account().map(str::to_string),
                model: picked_model.clone(),
                mode: picked_mode.clone(),
                cwd: cwd.display().to_string(),
                // A fresh pane resumes nothing.
                session: None,
                project,
            });
        }

        let program = match &composed {
            Some(composed) => match composed.exec() {
                Ok(launch) => pty::Program {
                    program: launch.program,
                    args: launch.args,
                    env: launch.env,
                    env_remove: launch.env_remove,
                    env_clear: launch.env_clear,
                },
                Err(error) => {
                    tracing::error!("pane {pane_id}: confining {agent_type} failed: {error:#}");
                    self.agents.retire(pane_id);
                    self.refuse_pane(client, pane_id, format!("{error:#}"));
                    return;
                }
            },
            None => pty::Program::plain(&agent_type, args),
        };

        let spawned = pty::spawn(&program, Some(cwd.as_path()), INITIAL_COLS, INITIAL_ROWS);
        let (mut pane, mut child) = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                self.agents.retire(pane_id);
                tracing::error!("pane {pane_id}: starting {agent_type} failed: {error:#}");
                self.host.send(
                    To::Client(client),
                    Message::PaneError {
                        pane_id,
                        error: error.to_string(),
                    },
                );
                return;
            }
        };
        let confined = composed
            .as_ref()
            .is_some_and(|composed| composed.is_confined());
        tracing::info!(
            "pane {pane_id}: started {agent_type}{} in session {session_id} for {client}",
            if confined { " (confined)" } else { "" }
        );

        // The owner is recorded before the pane is announced, so nothing can arrive about a pane
        // the routing table has never heard of.
        self.owners.insert(pane_id, client);
        let mailbox = self.host.mailbox(To::Client(client));

        if let Err(error) = pane.forward_output(pane_id, mailbox.clone(), false) {
            self.owners.remove(&pane_id);
            // The reader never started, so nothing else will ever wait on this child.
            pane.kill();
            let _ = child.wait();
            mailbox.send(Message::PaneError {
                pane_id,
                error: error.to_string(),
            });
            return;
        }
        pty::reap(pane_id, child, mailbox.clone());
        self.panes.insert(pane_id, pane);

        // The picker's terminal count, and the confirmation it puts in front of a close, are only
        // real because of this.
        self.pane_projects.insert(pane_id, project_id);
        self.pane_sessions.insert(pane_id, session_id);
        let replies = self.projects.pane_opened(project_id);
        self.answer(client, replies);

        mailbox.send(Message::WorkspaceSpawned {
            workspace: WorkspaceInfo {
                id: pane_id,
                session_id,
                agent_type,
                project_id,
                rel_path,
                cols: INITIAL_COLS,
                rows: INITIAL_ROWS,
                running: true,
                wait_on_exit: false,
            },
        });
    }

    /// The machine-wide tools as the new-pane menu offers them, stamped with whether each
    /// runs on this host's own platform.
    fn system_tools(&self) -> Vec<ListedTool> {
        let os = std::env::consts::OS;
        self.settings
            .host()
            .tools
            .into_iter()
            .map(|tool| {
                let applicable = tool.applies(os);
                ListedTool {
                    scope: Scope::Interface,
                    tool,
                    applicable,
                }
            })
            .collect()
    }

    /// One project's own tools, or nothing when no project was named or it is unknown.
    fn project_tools(&self, project_id: Option<ProjectId>) -> Vec<ListedTool> {
        let os = std::env::consts::OS;
        project_id
            .and_then(|id| self.projects.record(id))
            .map(|record| {
                record
                    .tools
                    .iter()
                    .cloned()
                    .map(|tool| {
                        let applicable = tool.applies(os);
                        ListedTool {
                            scope: Scope::Project(record.id),
                            tool,
                            applicable,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The tool a run names, from whichever list holds it.
    fn find_tool(&self, scope: &Scope, id: &ToolId) -> Option<ToolDef> {
        match scope {
            Scope::Interface => self
                .settings
                .host()
                .tools
                .into_iter()
                .find(|tool| &tool.id == id),
            Scope::Project(project_id) => self
                .projects
                .record(*project_id)
                .map(|record| record.tools.clone())
                .unwrap_or_default()
                .into_iter()
                .find(|tool| &tool.id == id),
        }
    }

    /// Tell the window that asked that its tool never started. Like [`Self::refuse_pane`],
    /// the interface was never told of a pane, so this answers `ToolError` rather than
    /// `PaneError`: there is nothing on screen to mark stopped.
    fn tool_error(&self, client: ClientId, project_id: Option<ProjectId>, error: String) {
        self.host
            .send(To::Client(client), Message::ToolError { project_id, error });
    }

    /// Run a configured tool: a named command in the project's folder, with its environment.
    ///
    /// Straight where [`Self::spawn_workspace`] branches: a tool is never composed and never
    /// confined — it is a program name with arguments, which is what a shell is, plus the
    /// environment its row carries. The answer names the tool rather than the program, which
    /// is what puts the tool's own title on the tab.
    fn run_tool(
        &mut self,
        client: ClientId,
        session_id: SessionId,
        project_id: ProjectId,
        scope: Scope,
        id: ToolId,
    ) {
        let tool = match self.find_tool(&scope, &id) {
            Some(tool) => tool,
            None => {
                self.tool_error(client, Some(project_id), "no such tool".to_string());
                return;
            }
        };
        if !tool.applies(std::env::consts::OS) {
            self.tool_error(
                client,
                Some(project_id),
                format!("{} is not for {}", tool.name, std::env::consts::OS),
            );
            return;
        }
        let mut words = crate::agent::split_command(&format!("{} {}", tool.command, tool.args));
        if words.is_empty() {
            self.tool_error(
                client,
                Some(project_id),
                format!("{} has nothing to run", tool.name),
            );
            return;
        }
        let program_name = crate::agent::resolve_bare(&words.remove(0));

        // A pane runs in a project's folder, so everything about that folder is settled before
        // a pseudo-terminal exists — the same gate a shell spawn walks through, which refuses
        // with `ProjectError` itself when the folder is gone.
        let cwd = match self.resolve_cwd(client, project_id, None) {
            Some(cwd) => cwd,
            None => return,
        };

        let pane_id = PaneId::generate();
        let name = tool.name.clone();
        let program = pty::Program {
            program: program_name,
            args: words,
            env: tool
                .env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            ..Default::default()
        };

        let spawned = pty::spawn(&program, Some(cwd.as_path()), INITIAL_COLS, INITIAL_ROWS);
        let (mut pane, mut child) = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                tracing::error!("pane {pane_id}: starting tool {name} failed: {error:#}");
                self.refuse_pane(client, pane_id, error.to_string());
                return;
            }
        };
        tracing::info!("pane {pane_id}: started tool {name} in session {session_id} for {client}");

        self.owners.insert(pane_id, client);
        let mailbox = self.host.mailbox(To::Client(client));

        if let Err(error) = pane.forward_output(pane_id, mailbox.clone(), false) {
            self.owners.remove(&pane_id);
            pane.kill();
            let _ = child.wait();
            mailbox.send(Message::PaneError {
                pane_id,
                error: error.to_string(),
            });
            return;
        }
        pty::reap(pane_id, child, mailbox.clone());
        self.panes.insert(pane_id, pane);

        self.pane_projects.insert(pane_id, project_id);
        self.pane_sessions.insert(pane_id, session_id);
        let replies = self.projects.pane_opened(project_id);
        self.answer(client, replies);

        mailbox.send(Message::WorkspaceSpawned {
            workspace: WorkspaceInfo {
                id: pane_id,
                session_id,
                agent_type: name,
                project_id,
                rel_path: None,
                cols: INITIAL_COLS,
                rows: INITIAL_ROWS,
                running: true,
                wait_on_exit: tool.wait_on_exit,
            },
        });
    }

    /// Tell one window which accounts exist. References only — an id and the harnesses it
    /// covers — because a credential has no business on the bus the log sink listens to.
    fn send_accounts(&mut self, client: ClientId) {
        match self.agents.accounts() {
            Ok(accounts) => self
                .host
                .send(To::Client(client), Message::Accounts { accounts }),
            Err(error) => {
                // A store that cannot be read is a real problem, but not one a window can act
                // on, and an empty list is what the screen would draw anyway.
                tracing::warn!("the accounts could not be read: {error:#}");
                self.host.send(
                    To::Client(client),
                    Message::Accounts {
                        accounts: Vec::new(),
                    },
                );
            }
        }
    }

    /// Tell one window which profiles exist. References only, the same rule as
    /// [`Self::send_accounts`] — a profile names an account, it never carries one.
    fn send_profiles(&mut self, client: ClientId) {
        match self.agents.profiles() {
            Ok(profiles) => self
                .host
                .send(To::Client(client), Message::Profiles { profiles }),
            Err(error) => {
                tracing::warn!("the profiles could not be read: {error:#}");
                self.host.send(
                    To::Client(client),
                    Message::Profiles {
                        profiles: Vec::new(),
                    },
                );
            }
        }
    }

    /// Open a pane running a harness's own login flow, and remember what finishing it means.
    ///
    /// A login pane is a pane in every respect but one: it belongs to no project, so it
    /// changes no project's count and closing it is not closing a workspace. What makes it a
    /// login is the entry in `logins` — read when the pane ends, and the only thing that
    /// turns an exited process into a captured account.
    ///
    /// `probe` runs a plain shell under the login's own policy instead of the harness — a
    /// diagnostic for inspecting what that sandbox permits, not a way to sign in. It reaches
    /// [`Self::login_gone`], which is what actually refuses to treat its exit as an outcome.
    fn begin_harness_login(
        &mut self,
        client: ClientId,
        agent_type: String,
        account: String,
        probe: bool,
    ) {
        let refuse = |coordinator: &mut Self, error: String| {
            tracing::warn!(harness = %agent_type, account = %account, "login refused: {error}");
            coordinator.host.send(
                To::Client(client),
                Message::HarnessLoginFailed {
                    agent_type: agent_type.clone(),
                    account: account.clone(),
                    error,
                },
            );
        };

        let pending = match self.agents.begin_login(&agent_type, &account, probe) {
            Ok(pending) => pending,
            Err(error) => return refuse(self, format!("{error:#}")),
        };

        let launch = pending.launch();
        let program = pty::Program {
            program: launch.program.clone(),
            args: launch.args.clone(),
            env: launch.env.clone(),
            env_remove: launch.env_remove.clone(),
            env_clear: launch.env_clear,
        };
        let pane_id = PaneId::generate();
        let (mut pane, mut child) =
            match pty::spawn(&program, Some(pending.home()), INITIAL_COLS, INITIAL_ROWS) {
                Ok(spawned) => spawned,
                Err(error) => return refuse(self, error.to_string()),
            };

        self.owners.insert(pane_id, client);
        let mailbox = self.host.mailbox(To::Client(client));
        if let Err(error) = pane.forward_output(pane_id, mailbox.clone(), true) {
            self.owners.remove(&pane_id);
            // The reader never started, so nothing else will ever wait on this child.
            pane.kill();
            let _ = child.wait();
            return refuse(self, error.to_string());
        }
        pty::reap(pane_id, child, mailbox.clone());
        self.panes.insert(pane_id, pane);
        self.logins.insert(pane_id, pending);

        tracing::info!(
            harness = %agent_type,
            account = %account,
            "login started in pane {pane_id} for {client}"
        );
        mailbox.send(Message::HarnessLoginStarted {
            pane_id,
            agent_type,
            account,
            cols: INITIAL_COLS,
            rows: INITIAL_ROWS,
        });
    }

    /// A login pane has ended: say whether it logged anybody in.
    ///
    /// Three outcomes and one message each, because the difference matters to the user: the
    /// credential appeared and is fresh, so the account exists; it was left untouched, so the
    /// harness exited without logging anyone in; or it is not there, so the flow was
    /// abandoned — which is exactly what pressing abort does, and is not an error.
    ///
    /// A probe pane is none of those three: it never ran the harness, so its credential (if
    /// any) never moved, and reading that as an outcome would be answering a question nobody
    /// asked. Its exit records no account and gets no `HarnessLoginCaptured`/`HarnessLoginFailed`
    /// — the window already knows its own pane closed from the ordinary `PaneExited` it just
    /// forwarded as `CloseWorkspace`, and reads that as done by itself.
    fn login_gone(&mut self, client: ClientId, pane_id: PaneId) {
        let Some(pending) = self.logins.remove(&pane_id) else {
            return;
        };
        if pending.probe {
            tracing::info!(
                harness = %pending.agent_type,
                account = %pending.account,
                "probe shell closed; nothing captured"
            );
            return;
        }
        let agent_type = pending.agent_type.clone();
        let account = pending.account.clone();

        match self.agents.finish_login(&pending) {
            Ok(()) => {
                tracing::info!(harness = %agent_type, account = %account, "login captured");
                self.host.send(
                    To::Client(client),
                    Message::HarnessLoginCaptured {
                        agent_type,
                        account,
                    },
                );
                // The list the settings screen draws has changed, and it is the same answer
                // for every window, so nobody has to ask again.
                self.send_accounts(client);
            }
            Err(error) => {
                tracing::info!(
                    harness = %agent_type,
                    account = %account,
                    "login captured nothing: {error:#}"
                );
                self.host.send(
                    To::Client(client),
                    Message::HarnessLoginFailed {
                        agent_type,
                        account,
                        error: format!("{error:#}"),
                    },
                );
            }
        }
    }

    /// Whether `account` has a login currently mid-capture — the whole of that harness's for a
    /// sign-out, any harness for a rename or a delete, since either would race the filesystem
    /// against a login the pane owning it has not finished writing.
    fn login_running(&self, account: &str, agent_type: Option<&str>) -> bool {
        self.logins.values().any(|pending| {
            pending.account == account && agent_type.is_none_or(|t| pending.agent_type == t)
        })
    }

    /// A stored credential could not be checked, renamed or deleted; say so to the window that
    /// asked, and nobody else — the same routing `refuse` in [`Self::begin_harness_login`] uses.
    fn account_error(&self, client: ClientId, error: String) {
        self.host
            .send(To::Client(client), Message::AccountError { error });
    }

    /// Answer whether `account` has a usable credential for `agent_type`. Always answered —
    /// an unknown harness or account reads as [`ubiq_proto::messages::LoginStatus::Missing`],
    /// not an error.
    fn check_harness_login(&mut self, client: ClientId, agent_type: String, account: String) {
        let now_ms = now_ms();
        let status = self.agents.check_login(&agent_type, &account, now_ms);
        tracing::debug!(harness = %agent_type, %account, ?status, "login checked");
        self.host.send(
            To::Client(client),
            Message::HarnessLoginStatus {
                agent_type,
                account,
                status,
            },
        );
    }

    fn rename_account(&mut self, client: ClientId, account: String, new_account: String) {
        if self.login_running(&account, None) {
            return self.account_error(
                client,
                format!("a sign-in for account '{account}' is still running"),
            );
        }
        match self.agents.rename_account(&account, &new_account) {
            Ok(()) => self.send_accounts(client),
            Err(error) => self.account_error(client, format!("{error:#}")),
        }
    }

    fn delete_account(&mut self, client: ClientId, account: String) {
        if self.login_running(&account, None) {
            return self.account_error(
                client,
                format!("a sign-in for account '{account}' is still running"),
            );
        }
        match self.agents.delete_account(&account) {
            Ok(()) => self.send_accounts(client),
            Err(error) => self.account_error(client, format!("{error:#}")),
        }
    }

    fn delete_harness_login(&mut self, client: ClientId, agent_type: String, account: String) {
        if self.login_running(&account, Some(&agent_type)) {
            return self.account_error(
                client,
                format!("a sign-in for account '{account}' is still running"),
            );
        }
        match self.agents.delete_harness_login(&agent_type, &account) {
            Ok(()) => self.send_accounts(client),
            Err(error) => self.account_error(client, format!("{error:#}")),
        }
    }

    /// Where a pane starts, or nothing at all and the window told why.
    ///
    /// The refusal is a `ProjectError` rather than a `PaneError`, because a `PaneError` names a pane
    /// the interface has never been told about and so has nowhere to put it. An unhealthy project
    /// also gets its fresh snapshot broadcast, so every picker marks the row from the probe that
    /// just happened rather than waiting to be asked.
    fn resolve_cwd(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        rel_path: Option<&str>,
    ) -> Option<PathBuf> {
        let Some(record) = self.projects.record(project_id) else {
            self.refuse_spawn(client, project_id, "no such project".to_string(), false);
            return None;
        };
        let root = PathBuf::from(&record.path);

        let health = health::probe(&root);
        if !health.is_ok() {
            let reason = match &health {
                ProjectHealth::Missing => "its folder is not there".to_string(),
                ProjectHealth::NotADirectory => "its path is not a folder".to_string(),
                ProjectHealth::Unreadable(reason) => reason.clone(),
                ProjectHealth::Ok => unreachable!("the branch above tested for this"),
            };
            self.refuse_spawn(client, project_id, reason, true);
            return None;
        }

        match rel_path {
            None => Some(root),
            Some(rel_path) => match files::path::resolve(&root, rel_path) {
                Ok(cwd) if cwd.is_dir() => Some(cwd),
                Ok(_) => {
                    self.refuse_spawn(
                        client,
                        project_id,
                        format!("{rel_path} is not a folder"),
                        false,
                    );
                    None
                }
                Err(error) => {
                    self.refuse_spawn(client, project_id, format!("{rel_path}: {error}"), false);
                    None
                }
            },
        }
    }

    /// Say why a pane was not started, and re-probe when the folder itself is the reason.
    fn refuse_spawn(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        error: String,
        reprobe: bool,
    ) {
        tracing::warn!("refusing a workspace in project {project_id}: {error}");
        self.host.send(
            To::Client(client),
            Message::ProjectError {
                project_id: Some(project_id),
                error,
            },
        );
        if reprobe {
            let replies = self.projects.refresh(project_id);
            self.answer(client, replies);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::harness::{CachedLevel, CachedModel};
    use ubiq_proto::conversation::{ConfigCategory, ConfigValue};

    #[test]
    fn unique_name_counts_from_the_second_occurrence_and_reuses_the_first_free_one() {
        assert_eq!(unique_name("claude", &[]), "claude");
        assert_eq!(unique_name("claude", &["claude".to_string()]), "claude 2");
        assert_eq!(
            unique_name("claude", &["claude".to_string(), "claude 2".to_string()]),
            "claude 3"
        );
        // A closed "claude 2" is reused rather than skipped to "claude 4".
        assert_eq!(
            unique_name("claude", &["claude".to_string(), "claude 3".to_string()]),
            "claude 2"
        );
        assert_eq!(unique_name("claude", &["codex".to_string()]), "claude");
    }

    fn model_with_levels(id: &str, default: bool) -> CachedModel {
        CachedModel {
            id: id.to_string(),
            description: None,
            default,
            levels: vec![CachedLevel {
                value: "high".to_string(),
                label: "High".to_string(),
                description: None,
            }],
            default_level: None,
        }
    }

    /// A model offering exactly the levels named — the existing `model_with_levels` fixes one
    /// level and a default flag, which the launch-pick rules need to vary independently.
    fn model_offering(id: &str, levels: &[&str]) -> CachedModel {
        CachedModel {
            id: id.to_string(),
            description: None,
            default: false,
            levels: levels
                .iter()
                .map(|value| CachedLevel {
                    value: (*value).to_string(),
                    label: (*value).to_string(),
                    description: None,
                })
                .collect(),
            default_level: None,
        }
    }

    fn model_without_levels(id: &str) -> CachedModel {
        CachedModel {
            id: id.to_string(),
            description: None,
            default: false,
            levels: Vec::new(),
            default_level: None,
        }
    }

    #[test]
    fn model_config_option_with_no_models_offers_a_default_fallback() {
        let option = model_config_option(&[], "");
        assert_eq!(option.id, "model");
        let ConfigValue::Select { current, choices } = option.value else {
            panic!("expected a Select value");
        };
        assert_eq!(current, "");
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].name, "Default");
    }

    #[test]
    fn model_config_option_preselects_a_remembered_model() {
        let models = [
            model_with_levels("sonnet", true),
            model_without_levels("haiku"),
        ];
        let option = model_config_option(&models, "haiku");
        let ConfigValue::Select { current, .. } = option.value else {
            panic!("expected a Select value");
        };
        assert_eq!(current, "haiku");
    }

    #[test]
    fn model_config_option_falls_back_to_the_default_when_the_remembered_model_is_gone() {
        let models = [
            model_with_levels("sonnet", true),
            model_without_levels("haiku"),
        ];
        // "opus" was remembered but this catalogue no longer offers it.
        let option = model_config_option(&models, "opus");
        let ConfigValue::Select { current, .. } = option.value else {
            panic!("expected a Select value");
        };
        assert_eq!(current, "sonnet", "falls back to the harness default");
    }

    #[test]
    fn thinking_config_option_is_none_for_a_model_with_no_levels() {
        let models = [model_without_levels("gpt-5-chat-latest")];
        assert!(thinking_config_option(&models, "gpt-5-chat-latest", "").is_none());
    }

    #[test]
    fn thinking_config_option_is_none_when_there_are_no_models_at_all() {
        assert!(thinking_config_option(&[], "anything", "").is_none());
    }

    #[test]
    fn thinking_config_option_matches_the_chosen_models_levels() {
        let models = [
            model_with_levels("sonnet", true),
            model_without_levels("haiku"),
        ];
        let option = thinking_config_option(&models, "sonnet", "").expect("sonnet has levels");
        assert_eq!(option.id, "thinking");
        assert_eq!(option.category, Some(ConfigCategory::ThoughtLevel));
        let ConfigValue::Select { choices, .. } = option.value else {
            panic!("expected a Select value");
        };
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].value, "high");

        // The other model in the same catalogue has no levels: choosing it produces none.
        assert!(thinking_config_option(&models, "haiku", "").is_none());
    }

    #[test]
    fn thinking_config_option_preselects_a_remembered_level_the_model_accepts() {
        let models = [model_with_levels("sonnet", true)];
        let option = thinking_config_option(&models, "sonnet", "high").expect("sonnet has levels");
        let ConfigValue::Select { current, .. } = option.value else {
            panic!("expected a Select value");
        };
        assert_eq!(current, "high");
    }

    #[test]
    fn thinking_config_option_drops_a_remembered_level_the_model_does_not_accept() {
        let models = [model_with_levels("sonnet", true)];
        // "sonnet" here only accepts "high" (see `model_with_levels`) — a remembered "low" from
        // some other model must not be offered.
        let option = thinking_config_option(&models, "sonnet", "low").expect("sonnet has levels");
        let ConfigValue::Select { current, .. } = option.value else {
            panic!("expected a Select value");
        };
        assert_eq!(
            current, "high",
            "falls back to the model's own default level"
        );
    }

    #[test]
    fn mode_config_option_is_none_for_a_harness_with_no_modes() {
        assert!(mode_config_option("opencode", "").is_none());
    }

    #[test]
    fn mode_config_option_carries_claude_codes_modes() {
        let option = mode_config_option("claude-code", "").expect("claude-code has modes");
        assert_eq!(option.id, "mode");
        assert_eq!(option.category, Some(ConfigCategory::Mode));
        let ConfigValue::Select { current, choices } = option.value else {
            panic!("expected a Select value");
        };
        assert_eq!(current, "", "no mode is preselected");
        assert!(choices.iter().any(|c| c.value == "plan"));
    }

    #[test]
    fn build_config_options_includes_mode_and_thinking_when_the_harness_and_model_offer_them() {
        let models = [model_with_levels("sonnet", true)];
        let options = build_config_options("claude-code", &models, "sonnet", "", "");
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert!(ids.contains(&"model"));
        assert!(ids.contains(&"mode"));
        assert!(ids.contains(&"thinking"));
    }

    #[test]
    fn build_config_options_omits_thinking_for_a_model_with_no_levels() {
        let models = [model_without_levels("gpt-5-chat-latest")];
        let options = build_config_options("codex", &models, "gpt-5-chat-latest", "", "");
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert!(ids.contains(&"model"));
        assert!(ids.contains(&"mode"));
        assert!(!ids.contains(&"thinking"));
    }

    /// The rule that decides whether a model pick is worth answering. A model whose reasoning
    /// levels differ has to be re-advertised — offering the previous model's levels is the lie
    /// this whole design exists to prevent — and the answer is the complete set, not just the
    /// picker that changed.
    #[test]
    fn a_model_pick_that_changes_the_levels_is_re_advertised() {
        let models = [
            model_offering("thinker", &["low", "high"]),
            model_without_levels("chatter"),
        ];
        let options =
            config_options_after_model_pick("codex", &models, "thinker", "high", "chatter")
                .expect("dropping the reasoning knob has to reach the window");
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert!(
            ids.contains(&"model"),
            "the set has to be complete: {ids:?}"
        );
        assert!(
            !ids.contains(&"thinking"),
            "chatter has no levels, so no picker: {ids:?}"
        );
    }

    /// And the other half: two models with the same levels leave nothing to say. The window set
    /// `current` itself when it sent the pick, and the model and mode lists cannot change by
    /// picking a model — so the message would only redraw the picker already on screen.
    #[test]
    fn a_model_pick_that_changes_nothing_says_nothing() {
        let models = [
            model_offering("one", &["low", "high"]),
            model_offering("two", &["low", "high"]),
        ];

        assert!(config_options_after_model_pick("codex", &models, "one", "", "two").is_none());
        // Confirming the model already showing is the same nothing.
        assert!(config_options_after_model_pick("codex", &models, "one", "", "one").is_none());
    }

    /// A model change resets thinking to the new model's own default, so a pick made while a
    /// non-default level was showing has to say so even when the level *lists* match.
    #[test]
    fn a_model_pick_that_resets_a_chosen_level_is_re_advertised() {
        let models = [
            model_offering("one", &["low", "high"]),
            model_offering("two", &["low", "high"]),
        ];

        assert!(config_options_after_model_pick("codex", &models, "one", "high", "two").is_some());
    }

    #[test]
    fn build_config_options_omits_mode_for_a_harness_with_none() {
        let models = [model_without_levels("some-model")];
        let options = build_config_options("opencode", &models, "some-model", "", "");
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert!(ids.contains(&"model"));
        assert!(!ids.contains(&"mode"));
    }

    // ── lifecycle: unload / resume, white-box ────────────────────────
    //
    // These drive `Coordinator`'s methods directly rather than over the bus, because getting a
    // genuinely *live* conversation the honest way needs a real, installed harness binary — the
    // rest of this file's tests never do that even for `claude-code`/`codex` (see
    // `a_launch_that_fails_retracts_the_agent_it_registered` in `tests/coordinator.rs`, which
    // forces a failure with a bad account rather than depend on one). `test_support::Idle`
    // supplies a `Conversation` whose pump is genuinely alive — blocked in `next_event` — without
    // a process behind it, which is all `unload_conversation`/`resume_conversation` ever look at.

    use crate::conversation::test_support::Idle;
    use crate::store::memory::{
        MemoryPreferenceStore, MemoryProjectStore, MemorySettingsStore, MemoryTaskStore,
    };
    use ubiq_proto::bus;
    use ubiq_proto::ids::{ProjectId, SessionId};
    use ubiq_proto::work::{Activity, WorkAgent, WorkSession};

    fn test_coordinator() -> (Coordinator, ubiq_proto::bus::Client) {
        let (hub, host) = bus::hub();
        let root_dir = tempfile::TempDir::new().unwrap();
        let root = ConfigRoot {
            path: root_dir.path().to_path_buf(),
            source: crate::config::RootSource::Flag,
        };
        let (projects, pending) = crate::projects::Projects::open(
            root.path.clone(),
            Box::new(MemoryProjectStore::new()),
            Box::new(MemoryPreferenceStore::new()),
        );
        // The directory has to outlive the coordinator that is writing into it.
        std::mem::forget(root_dir);
        let work = Work::open(Box::new(MemoryTaskStore::new()));
        let settings = Settings::open(Box::new(MemorySettingsStore::new()));
        let coordinator = Coordinator::new(host, root, projects, work, settings, pending);
        let client = hub.connect();
        (coordinator, client)
    }

    /// Register everything a live conversation needs — a `WorkAgent`, an owner, a launch recipe
    /// with an `agent_type` no real harness answers to (so a relaunch attempt fails at once rather
    /// than spawning anything), and a genuinely live `Conversation` pumping `Idle`, seeded to
    /// `last_seq`. Returns the agent and project ids.
    fn seed_live_conversation(
        coordinator: &mut Coordinator,
        client: &ubiq_proto::bus::Client,
        last_seq: u64,
    ) -> (AgentId, ProjectId) {
        let agent_id = AgentId::generate();
        let project_id = ProjectId::generate();
        let session_id = SessionId::generate();

        let agent = WorkAgent {
            id: agent_id,
            session: session_id,
            task: None,
            parent: None,
            name: "fake".to_string(),
            summary: None,
            role: "agent".to_string(),
            activity: Activity::Thinking,
            note: String::new(),
            branch: String::new(),
            tokens: 0.0,
            harness: "Fake".to_string(),
            account: String::new(),
            model: String::new(),
            context_pct: 0,
            persistent: false,
            accept_all: false,
            debug_dump: None,
            thread: Vec::new(),
        };
        let session = WorkSession {
            id: session_id,
            name: "s".to_string(),
            branch: String::new(),
            worktree: false,
        };
        coordinator.work.add_live_agent(project_id, agent, session);
        coordinator
            .conversation_owners
            .insert(agent_id, (client.id(), project_id));
        coordinator.pending_conversations.insert(
            agent_id,
            PendingConversation {
                project_id,
                agent_type: "not-a-real-harness".to_string(),
                account: None,
                profile: None,
                cwd: std::path::PathBuf::from("."),
                chosen_model: None,
                chosen_thinking: None,
                chosen_mode: None,
                catalogue: Vec::new(),
                next_seq: last_seq,
                opening_prompt: None,
                named: false,
                resume: None,
                mcps: Vec::new(),
            },
        );
        let mailbox = coordinator.host.mailbox(To::Client(client.id()));
        let conversation = Conversation::start(
            agent_id,
            Box::new(Idle::new()),
            mailbox,
            last_seq,
            None,
            false,
            ConvFlags::new(agent_id, false, false),
        );
        coordinator.conversations.insert(agent_id, conversation);

        (agent_id, project_id)
    }

    fn drain_all(client: &ubiq_proto::bus::Client) -> Vec<Message> {
        std::iter::from_fn(|| {
            client
                .from_host()
                .recv_timeout(Duration::from_millis(200))
                .ok()
        })
        .collect()
    }

    #[test]
    fn unload_removes_the_live_conversation_and_sends_no_conversation_ended() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, project_id) = seed_live_conversation(&mut coordinator, &client, 5);

        coordinator.unload_conversation(client.id(), agent_id);

        assert!(
            !coordinator.conversations.contains_key(&agent_id),
            "the pump is gone"
        );
        assert!(
            coordinator
                .work
                .live_agent_names(project_id)
                .contains(&"fake".to_string()),
            "the WorkAgent stays: unload is not delete"
        );
        let pending = coordinator
            .pending_conversations
            .get(&agent_id)
            .expect("the launch recipe stays, for a resume or the next PromptAgent");
        assert_eq!(
            pending.next_seq, 6,
            "one past the pump's last seq, so a relaunch continues rather than restarts it"
        );

        let messages = drain_all(&client);
        assert_eq!(
            messages.len(),
            1,
            "exactly one lifecycle message: {messages:?}"
        );
        assert!(matches!(
            &messages[0],
            Message::ConversationUnloaded { agent_id: id } if *id == agent_id
        ));
    }

    #[test]
    fn resume_on_a_live_conversation_changes_nothing_and_spawns_no_second_pump() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project_id) = seed_live_conversation(&mut coordinator, &client, 0);

        coordinator.resume_conversation(client.id(), agent_id);

        assert!(
            coordinator.conversations.contains_key(&agent_id),
            "still exactly the one, live conversation"
        );
        assert_eq!(coordinator.conversations.len(), 1, "no second pump");
        assert!(
            drain_all(&client).is_empty(),
            "resuming an already-live conversation must say nothing"
        );

        coordinator
            .conversations
            .remove(&agent_id)
            .unwrap()
            .stop(true);
    }

    #[test]
    fn prompt_agent_after_an_unload_relaunches_through_the_implicit_path() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project_id) = seed_live_conversation(&mut coordinator, &client, 0);
        coordinator.unload_conversation(client.id(), agent_id);
        drain_all(&client);

        // `agent_type` cannot resolve to a real harness, so the relaunch this triggers fails at
        // once — but it must be *attempted*, which is what this proves: `PromptAgent` after an
        // unload takes the `launch_pending` branch rather than `drive`, since the agent is no
        // longer in `conversations`.
        coordinator.dispatch(
            client.id(),
            Message::PromptAgent {
                agent_id,
                text: "hi".to_string(),
            },
        );

        let error = loop {
            match client.from_host().recv_timeout(Duration::from_millis(500)) {
                Ok(Message::ConversationError {
                    agent_id: id,
                    error,
                }) if id == agent_id => break error,
                Ok(_) => continue,
                Err(_) => panic!("the relaunch attempt never answered"),
            }
        };
        assert!(
            error.contains("unknown agent type"),
            "expected an unknown-harness refusal, said {error:?}"
        );
        assert!(
            !coordinator.pending_conversations.contains_key(&agent_id),
            "a failed relaunch retracts the recipe, the same as a failed first launch"
        );
    }

    /// A picker nobody touched must still launch what it was showing. The option carried the
    /// remembered model as `current`, so sending no flag would run something else — the gap this
    /// closes, and the one the picker cannot report because nothing looks wrong on screen.
    #[test]
    fn an_untouched_picker_launches_the_model_it_was_proposing() {
        let catalogue = vec![model_offering("opus", &["low", "high"])];
        let (model, thinking) = launch_picks(None, None, &catalogue, "opus", "high");
        assert_eq!(model.as_deref(), Some("opus"));
        assert_eq!(thinking.as_deref(), Some("high"));
    }

    /// An explicit pick always wins over the remembered one — otherwise choosing a model would
    /// silently do nothing when it happened to differ from last time.
    #[test]
    fn an_explicit_pick_beats_the_remembered_one() {
        let catalogue = vec![
            model_offering("opus", &["low", "high"]),
            model_offering("haiku", &["low"]),
        ];
        let (model, _) = launch_picks(Some("haiku"), None, &catalogue, "opus", "high");
        assert_eq!(model.as_deref(), Some("haiku"));
    }

    /// A remembered model the harness no longer offers must fall back to no flag at all — naming
    /// a model that is gone is a refused spawn, and the harness's own default is the honest answer.
    #[test]
    fn a_remembered_model_missing_from_the_catalogue_falls_back_to_no_flag() {
        let catalogue = vec![model_offering("opus", &["low"])];
        let (model, thinking) = launch_picks(None, None, &catalogue, "retired-model", "low");
        assert_eq!(model, None);
        assert_eq!(
            thinking, None,
            "no model resolved means no level to validate against"
        );
    }

    /// A level belongs to a model, never to a harness: one the resolved model does not accept is
    /// dropped rather than passed, matching what `thinking_config_option` refuses to offer.
    #[test]
    fn a_remembered_level_the_model_does_not_accept_is_dropped() {
        let catalogue = vec![model_offering("haiku", &["low"])];
        let (model, thinking) = launch_picks(None, None, &catalogue, "haiku", "xhigh");
        assert_eq!(model.as_deref(), Some("haiku"));
        assert_eq!(thinking, None);
    }

    /// A pick made and then abandoned — never launched — must not be remembered: `SetAgentConfig`
    /// on a pending agent only ever touches `pending_conversations`, and a launch attempt that
    /// fails before `Conversation::start` never reaches the `set_last_used` call, so the harness
    /// cache stays untouched either way.
    #[test]
    fn nothing_is_remembered_until_a_launch_actually_happens() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project_id) = seed_live_conversation(&mut coordinator, &client, 0);
        coordinator.unload_conversation(client.id(), agent_id);
        drain_all(&client);

        // A pick on the now-pending agent — recorded only in `pending_conversations`.
        coordinator.dispatch(
            client.id(),
            Message::SetAgentConfig {
                agent_id,
                config_id: "model".to_string(),
                value: "some-model".to_string(),
            },
        );
        drain_all(&client);
        assert_eq!(
            coordinator.catalogue.last_used("not-a-real-harness"),
            None,
            "a pick alone must not reach the cache"
        );

        // The relaunch this triggers fails at once (`not-a-real-harness` resolves to nothing),
        // so the pick is never recorded as a launch either.
        coordinator.dispatch(
            client.id(),
            Message::PromptAgent {
                agent_id,
                text: "hi".to_string(),
            },
        );
        drain_all(&client);
        assert_eq!(
            coordinator.catalogue.last_used("not-a-real-harness"),
            None,
            "a failed launch attempt must not be remembered as a preference"
        );
    }

    #[test]
    fn ending_after_an_unload_still_removes_the_run_directory() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, project_id) = seed_live_conversation(&mut coordinator, &client, 0);

        let run_dir = coordinator.agents.agent_dir(agent_id);
        std::fs::create_dir_all(&run_dir).unwrap();

        coordinator.unload_conversation(client.id(), agent_id);
        assert!(run_dir.exists(), "unload must not touch the run directory");

        coordinator.end_conversation(agent_id, StopReason::Cancelled);
        assert!(
            !run_dir.exists(),
            "a delete after an unload still removes the run directory"
        );
        assert!(
            !coordinator
                .work
                .live_agent_names(project_id)
                .contains(&"fake".to_string()),
            "a delete takes the WorkAgent with it"
        );
    }

    // ── persistence: park, delete, revive ────────────────────────────

    /// Write the durable row a seeded conversation would have, and set its flag. `seed_live_conversation`
    /// inserts the recipe straight into the map, so nothing has written a row for it yet.
    fn mark_persistent(coordinator: &Coordinator, agent_id: AgentId, persistent: bool) {
        coordinator.remember_conversation(agent_id);
        let sessions = coordinator.sessions();
        let mut row = conversation_record::load(&sessions, agent_id).expect("a row was written");
        row.persistent = persistent;
        conversation_record::save(&sessions, agent_id, &row);
    }

    /// A window closing, or a harness exiting on its own, must not take a conversation the user
    /// asked to keep — the run directory *is* the harness's session store, so deleting it is
    /// deleting the conversation. Both cases go through `end_conversation`, which is why one test
    /// covers them.
    #[test]
    fn a_close_parks_a_kept_conversation_and_deletes_one_nobody_asked_to_keep() {
        for persistent in [true, false] {
            let (mut coordinator, client) = test_coordinator();
            let (agent_id, _project_id) = seed_live_conversation(&mut coordinator, &client, 0);
            let run_dir = coordinator.agents.agent_dir(agent_id);
            std::fs::create_dir_all(&run_dir).unwrap();
            mark_persistent(&coordinator, agent_id, persistent);

            coordinator.end_conversation(agent_id, StopReason::Cancelled);

            assert_eq!(
                run_dir.exists(),
                persistent,
                "persistent={persistent}: the run directory should have been \
                 {}",
                if persistent { "parked" } else { "deleted" }
            );
            assert!(
                conversation_record::load(&coordinator.sessions(), agent_id).is_some(),
                "a close never takes the row — only an explicit end does"
            );
        }
    }

    /// Deleting a conversation deletes all three of it: the run directory, Ubiq's row and the
    /// library's session record. A row without a directory would offer a resume that comes back
    /// empty, so none of the three may outlive the others — and this holds even for a conversation
    /// marked persistent, because `EndConversation` is the one place the user means *gone*.
    #[test]
    fn ending_a_conversation_takes_the_directory_the_row_and_the_session_record() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project_id) = seed_live_conversation(&mut coordinator, &client, 0);
        let run_dir = coordinator.agents.agent_dir(agent_id);
        std::fs::create_dir_all(&run_dir).unwrap();
        mark_persistent(&coordinator, agent_id, true);
        let session_dir = coordinator.sessions().join(agent_id.to_string());
        std::fs::write(session_dir.join("meta.json"), b"{}").unwrap();

        coordinator.dispatch(client.id(), Message::EndConversation { agent_id });

        assert!(!run_dir.exists(), "the run directory goes");
        assert!(
            !session_dir.exists(),
            "so do the row and the library's record beside it"
        );
    }

    /// Both of Ubiq's own flags make the same round trip the persistence one does: the durable
    /// row is what any later decision reads, the live conversation is told so it acts on it at
    /// once, and every window hears an `AgentChanged` so the menu that flipped the flag redraws
    /// with it flipped.
    #[test]
    fn accepting_all_permissions_is_written_to_the_row_and_the_live_conversation() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project_id) = seed_live_conversation(&mut coordinator, &client, 0);
        coordinator.remember_conversation(agent_id);
        while client.from_host().try_recv().is_ok() {}

        coordinator.dispatch(
            client.id(),
            Message::SetConversationAcceptAll {
                agent_id,
                accept_all: true,
            },
        );

        let row = conversation_record::load(&coordinator.sessions(), agent_id).expect("a row");
        assert!(row.accept_all, "the durable row is what a launch reads");
        assert!(
            coordinator.conversations[&agent_id].flags().accept_all(),
            "the running harness's requests should be answered from this moment, not the next launch"
        );
        let sent: Vec<Message> =
            std::iter::from_fn(|| client.from_host().try_recv().ok()).collect();
        assert!(
            sent.iter().any(|message| matches!(
                message,
                Message::AgentChanged { agent, .. } if agent.id == agent_id && agent.accept_all
            )),
            "expected an AgentChanged carrying the flag, got {sent:?}"
        );
    }

    /// The capture answers with the file it writes, because a window that cannot say where the
    /// traffic went is a debugging aid nobody can use.
    #[test]
    fn dumping_a_conversation_reports_the_file_it_writes() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project_id) = seed_live_conversation(&mut coordinator, &client, 0);
        coordinator.remember_conversation(agent_id);
        while client.from_host().try_recv().is_ok() {}

        coordinator.dispatch(
            client.id(),
            Message::SetConversationDebugDump {
                agent_id,
                debug_dump: true,
            },
        );

        let row = conversation_record::load(&coordinator.sessions(), agent_id).expect("a row");
        assert!(row.debug_dump);
        let expected = ConvFlags::dump_path_for(agent_id).display().to_string();
        let sent: Vec<Message> =
            std::iter::from_fn(|| client.from_host().try_recv().ok()).collect();
        assert!(
            sent.iter().any(|message| matches!(
                message,
                Message::AgentChanged { agent, .. }
                    if agent.id == agent_id && agent.debug_dump.as_deref() == Some(expected.as_str())
            )),
            "expected an AgentChanged naming {expected}, got {sent:?}"
        );

        // Turning it off closes the capture and takes the path with it.
        coordinator.dispatch(
            client.id(),
            Message::SetConversationDebugDump {
                agent_id,
                debug_dump: false,
            },
        );
        assert!(
            !coordinator.conversations[&agent_id].flags().dumping(),
            "the capture should be closed"
        );
        let _ = std::fs::remove_file(ConvFlags::dump_path_for(agent_id));
    }

    /// Deleting a conversation is the one ending that is not `ConversationEnded` — that message
    /// says the transcript stays, which is exactly wrong once the record is gone. The owning
    /// window has to be told `ConversationDeleted` instead, so it drops its own copy rather than
    /// going on drawing a conversation the host no longer has anything to say about.
    #[test]
    fn deleting_a_conversation_tells_its_window_so() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project_id) = seed_live_conversation(&mut coordinator, &client, 0);
        // Drain whatever `seed_live_conversation` already queued for this client, so the assert
        // below is only about what `EndConversation` itself sends.
        while client.from_host().try_recv().is_ok() {}

        coordinator.dispatch(client.id(), Message::EndConversation { agent_id });

        let sent: Vec<Message> =
            std::iter::from_fn(|| client.from_host().try_recv().ok()).collect();
        assert!(
            sent.iter()
                .any(|message| matches!(message, Message::ConversationDeleted { agent_id: id } if *id == agent_id)),
            "expected a ConversationDeleted for the owning client, got {sent:?}"
        );
    }

    /// A re-attach starts the same conversation again from its own kept directory: the recipe the
    /// source was launched with comes back field for field, and the harness's own session id — the
    /// one thing the row deliberately does not duplicate — is read out of the library's record.
    #[test]
    fn a_re_attach_restores_the_recipe_and_the_harness_session_id() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _seeded) = seed_live_conversation(&mut coordinator, &client, 0);
        let held = tempfile::TempDir::new().unwrap();
        // Canonical, because the catalogue stores what it resolved and the revive turns the
        // source's `cwd` back into a path relative to it.
        let folder = held.path().canonicalize().unwrap();
        let project_id = add_test_project(&mut coordinator, &folder);

        {
            let pending = coordinator
                .pending_conversations
                .get_mut(&agent_id)
                .unwrap();
            pending.project_id = project_id;
            pending.agent_type = "claude-code".to_string();
            pending.cwd = folder.clone();
            pending.account = Some("work".to_string());
            pending.chosen_model = Some("opus".to_string());
        }
        let mut meta = agent_manager::session::SessionMeta::new(
            "claude-code".to_string(),
            folder.clone(),
            Vec::new(),
            None,
            "jsonl".to_string(),
            folder.clone(),
        );
        meta.id = agent_id.to_string();
        meta.harness_session_id = Some("harness-session-1".to_string());
        agent_manager::session::save(&coordinator.sessions(), &meta).unwrap();

        // Source and target are the same agent: a re-attach. Its `Conversation` is still live, so
        // the `resume_conversation` at the end is a no-op and nothing is spawned.
        coordinator.revive_conversation(
            client.id(),
            agent_id,
            agent_id,
            project_id,
            SessionId::generate(),
        );

        // The recipe below would read the same if the start had refused and left the seeded row in
        // place, so the start itself is proved first.
        let said = drain_all(&client);
        assert!(
            said.iter()
                .any(|message| matches!(message, Message::ConversationStarted { .. })),
            "the revived conversation was registered: {said:?}"
        );

        let pending = coordinator
            .pending_conversations
            .get(&agent_id)
            .expect("the revived recipe");
        assert_eq!(pending.agent_type, "claude-code");
        assert_eq!(pending.cwd, folder);
        assert_eq!(pending.account.as_deref(), Some("work"));
        assert_eq!(pending.chosen_model.as_deref(), Some("opus"));
        assert_eq!(
            pending.resume.as_deref(),
            Some("harness-session-1"),
            "the resume token comes from the library's record, not the row"
        );

        coordinator
            .conversations
            .remove(&agent_id)
            .unwrap()
            .stop(true);
    }

    /// Two forks that must not happen. grok writes its sessions to the real `~/.grok` whatever
    /// `HOME` says, so a copied directory would isolate nothing and the two agents would append to
    /// one store; and copying any store while its harness is running catches the last record
    /// mid-write. Both are refused with a reason, before anything is copied.
    #[test]
    fn a_fork_refuses_a_harness_that_keeps_its_sessions_elsewhere_and_one_still_running() {
        for (agent_type, expected) in [
            ("grok", "outside the run directory"),
            ("claude-code", "still running"),
        ] {
            let (mut coordinator, client) = test_coordinator();
            let (source, project_id) = seed_live_conversation(&mut coordinator, &client, 0);
            coordinator
                .pending_conversations
                .get_mut(&source)
                .unwrap()
                .agent_type = agent_type.to_string();

            let forked = AgentId::generate();
            coordinator.revive_conversation(
                client.id(),
                source,
                forked,
                project_id,
                SessionId::generate(),
            );

            assert!(
                !coordinator.pending_conversations.contains_key(&forked),
                "{agent_type}: nothing was registered for the fork"
            );
            assert!(
                !coordinator.agents.agent_dir(forked).exists(),
                "{agent_type}: nothing was copied"
            );
            let said = drain_all(&client);
            let error = said
                .iter()
                .find_map(|message| match message {
                    Message::ConversationError { agent_id, error } if *agent_id == forked => {
                        Some(error.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{agent_type}: no refusal, said {said:?}"));
            assert!(
                error.contains(expected),
                "{agent_type}: expected {expected:?}, said {error:?}"
            );

            coordinator
                .conversations
                .remove(&source)
                .unwrap()
                .stop(true);
        }
    }

    /// Put a real folder in the catalogue, so `resolve_cwd` has something to validate against.
    fn add_test_project(coordinator: &mut Coordinator, path: &std::path::Path) -> ProjectId {
        coordinator
            .projects
            .add(
                &path.to_string_lossy(),
                Some("project".to_string()),
                None,
                None,
                false,
            )
            .into_iter()
            .find_map(|reply| match reply.into_message() {
                Message::ProjectAdded { project } => Some(project.id()),
                _ => None,
            })
            .expect("the project was added")
    }

    // ── probe logins ──────────────────────────────────────────────────

    /// A probe pane's exit must never be read as a login outcome. The fixture is built so a
    /// real login *would* have captured — a fresh credential, and `captured_before: None` (a
    /// first-ever login, the case `finish_login`'s mtime rule always lets through) — precisely
    /// so this proves the skip is `pending.probe`'s doing, not an accident of the fixture.
    #[test]
    fn a_probes_exit_records_no_account_and_sends_no_login_outcome() {
        let (mut coordinator, client) = test_coordinator();
        let home = tempfile::TempDir::new().unwrap();
        let cred = PathBuf::from("cred.json");
        std::fs::write(home.path().join(&cred), b"fresh").unwrap();

        let pane_id = PaneId::generate();
        let pending = PendingLogin::for_test(
            "work",
            "claude-code",
            home.path().to_path_buf(),
            vec![cred],
            None,
            true,
        );
        coordinator.logins.insert(pane_id, pending);

        coordinator.login_gone(client.id(), pane_id);

        assert!(
            !coordinator.logins.contains_key(&pane_id),
            "the pending entry is consumed either way"
        );
        assert!(
            coordinator.agents.accounts().unwrap().is_empty(),
            "a probe must never record an account, even though this fixture would have \
             captured one"
        );
        let messages = drain_all(&client);
        assert!(
            messages.is_empty(),
            "a probe sends neither HarnessLoginCaptured nor HarnessLoginFailed: {messages:?}"
        );
    }
}
