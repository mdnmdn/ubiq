//! The coordinator: it starts harnesses, supervises them, and answers the bus.
//!
//! It runs on a thread of its own and shares nothing with the window but the channel pair in
//! [`ubiq_proto::bus`]. It renders nothing and has no opinion about layout or colour — everything here
//! is a pane ID, a pseudo-terminal and a process.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use ubiq_proto::bus::{ClientId, FromClient, HostEnd, To};
use ubiq_proto::conversation::{
    ConfigCategory, ConfigChoice, ConfigOption, ConfigValue, ConvUpdate, StopReason,
};
use ubiq_proto::files::FileError;
use ubiq_proto::ids::{PaneId, ProjectId, SearchId, SessionId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use ubiq_proto::projects::ProjectHealth;
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

use crate::agent::{Agents, PendingLogin};
use crate::cli_shortcut;
use crate::config::ConfigRoot;
use crate::connectors::{Answer, Connectors};
use crate::conversation::Conversation;
use crate::files::{self, Files};
use crate::git::{self, Git};
use crate::health;
use crate::projects::Projects;
use crate::pty::{self, Pty};
use crate::reply::Reply;
use crate::search::{self, Search};
use crate::settings::Settings;
use crate::shells;
use crate::store::harness::{CachedModel, FileHarnessCache};
use crate::watch;
use crate::work::Work;

/// The geometry a pane starts at, before the emulator has measured its own bounds and said what it
/// really is. The harness is told the truth a frame later, by [`Message::TerminalResize`].
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// How long the run loop's wait may be while a conversation is live, so a harness that ends on its
/// own is reaped promptly rather than only on the next unrelated message. See the wait computation
/// in `run` for why this is needed at all.
const CONVERSATION_POLL: Duration = Duration::from_millis(500);

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
    /// One live search per project. The flag means two things: a cancel request, set when a
    /// second search for the same project arrives or `CancelSearch` names this one; and "this
    /// search is over", set by the worker itself when it finishes, cancelled or not. `search_job`
    /// reaps entries where the flag is already set, the one place that mints them.
    active_searches: HashMap<ProjectId, (SearchId, Arc<AtomicBool>)>,
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
}

/// A conversation the window asked for and the harness has not yet answered — registered so the
/// UI can draw it with a loader while its harness's models are discovered, instead of starting it.
/// [`Coordinator::conversations`] is where an agent moves once the harness actually exists.
#[derive(Clone)]
struct PendingConversation {
    project_id: ProjectId,
    agent_type: String,
    account: Option<String>,
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
}

/// Whether `agent_type` takes anything after its first turn — the inventory table in
/// `_docs/wip/agent-setup.md` names Claude and Codex as the only two; opencode, Copilot and Grok
/// bridges are one-shot. Known before a bridge exists because it is a fact of the harness, not of
/// the running process — [`Conversation::accepts_input`] reports the identical thing once a bridge
/// is there to ask.
fn accepts_second_turn(agent_type: &str) -> bool {
    matches!(agent_type, "claude-code" | "codex")
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
/// harness::Harness::modes`] list — `None` when the harness offers no choice (opencode, Copilot,
/// Grok today), an absent picker rather than an empty one. `current` is always empty: no mode
/// is a harness default the way a model or a reasoning level is, so nothing is preselected —
/// leaving it unset is what tells `launch_pending` to pass no mode flag at all.
fn mode_config_option(agent_type: &str) -> Option<ConfigOption> {
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
            current: String::new(),
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
) -> Vec<ConfigOption> {
    let mut options = vec![model_config_option(models, chosen_model)];
    options.extend(mode_config_option(agent_type));
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
        projects: Projects,
        work: Work,
        settings: Settings,
        pending: Vec<Reply>,
    ) -> Self {
        // A run directory outlives its pane only when Ubiq did not get to close it, and no pane
        // from a previous process is still running, so the sweep happens once here.
        let agents = Agents::new(root.path.clone(), settings.host().isolate_agents);
        agents.sweep();
        let settings = Arc::new(settings);
        let connectors = Connectors::new(settings.clone(), &root.path);
        let catalogue = Arc::new(FileHarnessCache::new(
            root.path.join("cache").join("harness-models.toml"),
        ));

        Self {
            host,
            root,
            projects,
            work,
            settings,
            connectors,
            agents,
            catalogue,
            files: Files::start(),
            git: Git::start(),
            search: Search::start(),
            active_searches: HashMap::new(),
            watchers: HashMap::new(),
            pending,
            pane_projects: HashMap::new(),
            panes: HashMap::new(),
            owners: HashMap::new(),
            focused: HashMap::new(),
            conversations: HashMap::new(),
            conversation_owners: HashMap::new(),
            pending_conversations: HashMap::new(),
            logins: HashMap::new(),
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
            let wait = if self.conversations.is_empty() {
                due
            } else {
                Some(due.map_or(CONVERSATION_POLL, |due| due.min(CONVERSATION_POLL)))
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
            self.reap_conversations();
            self.projects.flush_due(Instant::now());
        }
        // Nothing is left to say it to, but what the user last did still belongs on disk.
        self.projects.flush();
    }

    /// A window attached. It is told what the host is, and hears anything the catalogue wanted to
    /// say before there was anybody to say it to.
    fn client_here(&mut self, client: ClientId) {
        tracing::debug!("{client} attached");
        self.host.send(
            To::Client(client),
            Message::HostInfo {
                config_root: self.root.path.to_string_lossy().into_owned(),
                is_default: self.root.is_default(),
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
        self.watchers.retain(|(owner, _), _| *owner != client);
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
        // A flow whose window has gone has nobody left to answer its next question.
        self.connectors.client_gone(client);
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
        match message {
            Message::SpawnWorkspace {
                session_id,
                project_id,
                rel_path,
                agent_type,
                args,
            } => self.spawn_workspace(client, session_id, project_id, rel_path, agent_type, args),

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
            Message::ListShells => {
                self.host.send(
                    To::Client(client),
                    Message::ShellList {
                        shells: shells::available(),
                    },
                );
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
                self.answer(client, replies);
            }
            Message::UpdateProject {
                project_id,
                name,
                colour,
                custom_colour,
                search_excludes,
                no_local_index,
            } => {
                let replies = self.projects.update(
                    project_id,
                    name,
                    colour,
                    custom_colour,
                    search_excludes,
                    no_local_index,
                );
                self.answer(client, replies);
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

            Message::ListAccounts => {
                self.send_accounts(client);
            }
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
            } => {
                let (asker, everyone) = self.sinks(client);
                let replies = self.connectors.begin(
                    client, connect_id, provider, instance, label, auth, client_id, asker, everyone,
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
            Message::SetAppSecret {
                provider,
                origin,
                secret,
            } => {
                let replies = self.connectors.set_app_secret(provider, origin, secret);
                self.answer(client, replies);
            }
            Message::ClearAppSecret { provider, origin } => {
                let replies = self.connectors.clear_app_secret(provider, origin);
                self.answer(client, replies);
            }

            // The `ubiq` command on PATH. Every path in the exchange is the host's: the interface
            // says which of the three things to do and is told what is there afterwards.
            Message::CliShortcut { action } => {
                let reply = Reply::Asker(cli_shortcut::handle(action));
                self.answer(client, vec![reply]);
            }

            Message::GetSettings { layer } => {
                let reply = self.settings.get(layer);
                self.answer(client, vec![reply]);
            }
            Message::SetSettings { layer, value } => {
                let replies = self.settings.set(layer, value);
                // Whether an agent is confined is acted on at the next spawn, so the setting is
                // re-read here rather than kept in a copy that could go stale.
                self.agents.set_isolate(self.settings.host().isolate_agents);
                self.answer(client, replies);
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
            } => {
                self.start_conversation(
                    client, agent_id, project_id, session_id, rel_path, agent_type, account,
                );
            }
            Message::PromptAgent { agent_id, text } => {
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
                        pending.chosen_model = Some(value.clone());
                        // ponytail: re-reads the cache rather than holding the probe's result; a
                        // miss would cost one re-probe and cannot happen — the discovery thread
                        // wrote the cache before it ever sent the `ConfigOptions` that made this
                        // pick possible.
                        let account_key = pending.account.clone().unwrap_or_default();
                        pending.catalogue =
                            probe_catalogue(&pending.agent_type, &account_key, &self.catalogue)
                                .unwrap_or_default();
                        // A level is per model: offering one the newly chosen model does not
                        // accept is exactly the lie this design exists to prevent, so the
                        // thinking picker is recomputed for `value`, not the previous model —
                        // and reset to that model's own default rather than carrying over
                        // whatever level was showing before (empty `chosen_thinking`).
                        let options = build_config_options(
                            &pending.agent_type,
                            &pending.catalogue,
                            &value,
                            "",
                        );
                        // Pre-increment, mirroring the pump's own `seq += 1` before it sends: the
                        // discovery thread's message already claimed seq 1, so the first pick's
                        // resend is seq 2, and `next_seq` still names "the last seq used" when
                        // `launch_pending` later hands it to `Conversation::start` as the pump's
                        // own starting point.
                        pending.next_seq += 1;
                        let seq = pending.next_seq;
                        self.host
                            .mailbox(To::Client(client))
                            .send(Message::ConversationUpdate {
                                agent_id,
                                seq,
                                update: Box::new(ConvUpdate::ConfigOptions(options)),
                            });
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
                } else {
                    self.drive(client, agent_id, |conversation| {
                        conversation.set_config(config_id, value)
                    });
                }
            }
            Message::EndConversation { agent_id } => {
                self.end_conversation(agent_id, StopReason::Cancelled);
            }
            Message::UnloadConversation { agent_id } => {
                self.unload_conversation(client, agent_id);
            }
            Message::ResumeConversation { agent_id } => {
                self.resume_conversation(client, agent_id);
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

        // Whether this harness takes a second turn is a fact of the harness, not of a running
        // process, so it is known without a bridge — the same answer `Conversation::accepts_input`
        // would give once one exists.
        let accepts_input = accepts_second_turn(&agent_type);
        // `account` is inert in `discover_models` today, but it is already the cache key's
        // identity leg — captured before `account` moves into the pending record below.
        let account_key = account.clone().unwrap_or_default();
        self.pending_conversations.insert(
            agent_id,
            PendingConversation {
                project_id,
                agent_type: agent_type.clone(),
                account,
                cwd,
                chosen_model: None,
                chosen_thinking: None,
                chosen_mode: None,
                catalogue: Vec::new(),
                // The discovery thread below always sends the first message this agent_id will
                // ever see, and always as seq 1 — nothing else can race ahead of it.
                next_seq: 1,
            },
        );

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
                let chosen_model = if models.iter().any(|m| m.id == last_model) {
                    last_model
                } else {
                    default_model_id(&models)
                };
                let options =
                    build_config_options(&agent_type, &models, &chosen_model, &last_thinking);
                discovery_mailbox.send(Message::ConversationUpdate {
                    agent_id,
                    seq: 1,
                    update: Box::new(ConvUpdate::ConfigOptions(options)),
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
    fn launch(
        &mut self,
        client: ClientId,
        agent_id: AgentId,
        pending: PendingConversation,
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
        let (composed, bridge) = match self.agents.converse(
            agent_id,
            &pending.agent_type,
            &pending.cwd,
            pending.account.clone(),
            model.clone(),
            thinking.clone(),
            mode,
        ) {
            Ok(started) => started,
            Err(error) => {
                tracing::error!(
                    agent = %agent_id,
                    harness = %pending.agent_type,
                    "starting failed: {error:#}"
                );
                self.agents.retire_agent(agent_id);
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
        let conversation = Conversation::start(agent_id, bridge, mailbox, pending.next_seq);
        self.conversations.insert(agent_id, conversation);
        true
    }

    /// Launch a pending agent now that its first prompt has arrived — the moment P3 draws the
    /// line between "asked for" and "running" — then forward that prompt. `ConversationStarted`
    /// already went out when this agent was registered as pending, and `accepts_input` cannot
    /// have changed since — it is the harness's own fact, not the process's.
    fn launch_pending(&mut self, client: ClientId, agent_id: AgentId, text: String) {
        if !self.drives(client, agent_id) {
            return;
        }
        let Some(pending) = self.pending_conversations.get(&agent_id).cloned() else {
            return;
        };
        if !self.launch(client, agent_id, pending) {
            return;
        }
        self.drive(client, agent_id, |conversation| conversation.prompt(text));
    }

    /// Start an unloaded (or never-launched) conversation's harness again, with no prompt to
    /// forward. Already live is left alone — resuming twice must not spawn a second pump.
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
        self.launch(client, agent_id, pending);
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

    /// Reap conversations whose harness ended on its own — a one-shot bridge's stream, or any
    /// bridge's process exiting unasked — rather than by an explicit `EndConversation`.
    ///
    /// Without this, a one-shot harness (`accepts_second_turn` false: everything but
    /// claude-code/codex) leaks its `conversations`/`conversation_owners`/`work.live` rows and its
    /// run directory — credentials included — on every conversation, until the window closes.
    /// The same shape `active_searches` already uses: a flag the worker sets on its way out,
    /// polled here rather than raced against.
    fn reap_conversations(&mut self) {
        let ended: Vec<AgentId> = self
            .conversations
            .iter()
            .filter(|(_, conversation)| conversation.ended())
            .map(|(agent_id, _)| *agent_id)
            .collect();
        for agent_id in ended {
            tracing::debug!("agent {agent_id}'s harness ended on its own; reaping");
            self.end_conversation(agent_id, StopReason::EndTurn);
        }
    }

    /// Stop an agent and take everything it owned with it.
    ///
    /// The pump answers its own `ConversationEnded` on the way out, so nothing is said here: two
    /// endings for one agent would leave a window unsure which it was.
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
        // The agent owned its run's configuration directory — credentials seeded into it included
        // — so that goes when the agent does.
        self.agents.retire_agent(agent_id);
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
            project_id,
            root: PathBuf::from(&record.path),
            request,
            reply_to: self.host.mailbox(To::Client(client)),
        });
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

        match watch::start(watch::Job {
            project_id,
            root,
            excludes,
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
            cancel,
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    fn spawn_workspace(
        &mut self,
        client: ClientId,
        session_id: SessionId,
        project_id: ProjectId,
        rel_path: Option<String>,
        agent_type: Option<String>,
        args: Vec<String>,
    ) {
        // A pane runs in a project's folder, so everything about that folder is settled before a
        // pseudo-terminal exists. A spawn that fails here leaves nothing on screen to close.
        let cwd = match self.resolve_cwd(client, project_id, rel_path.as_deref()) {
            Some(cwd) => cwd,
            None => return,
        };

        let pane_id = PaneId::generate();
        let agent_type = agent_type.unwrap_or_else(shells::default_program);

        // An agent type the library knows is composed — its skills, its throwaway configuration
        // and the policy it runs under all come from there. Anything else is a program name,
        // which is what a shell is.
        let composed = if self.agents.is_agent_type(&agent_type) {
            match self
                .agents
                .compose(pane_id, &agent_type, &cwd, args.clone())
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
        assert!(mode_config_option("opencode").is_none());
    }

    #[test]
    fn mode_config_option_carries_claude_codes_modes() {
        let option = mode_config_option("claude-code").expect("claude-code has modes");
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
        let options = build_config_options("claude-code", &models, "sonnet", "");
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert!(ids.contains(&"model"));
        assert!(ids.contains(&"mode"));
        assert!(ids.contains(&"thinking"));
    }

    #[test]
    fn build_config_options_omits_thinking_for_a_model_with_no_levels() {
        let models = [model_without_levels("gpt-5-chat-latest")];
        let options = build_config_options("codex", &models, "gpt-5-chat-latest", "");
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert!(ids.contains(&"model"));
        assert!(ids.contains(&"mode"));
        assert!(!ids.contains(&"thinking"));
    }

    #[test]
    fn build_config_options_omits_mode_for_a_harness_with_none() {
        let models = [model_without_levels("some-model")];
        let options = build_config_options("opencode", &models, "some-model", "");
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
            role: "agent".to_string(),
            activity: Activity::Thinking,
            note: String::new(),
            branch: String::new(),
            tokens: 0.0,
            harness: "Fake".to_string(),
            account: String::new(),
            model: String::new(),
            context_pct: 0,
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
                cwd: std::path::PathBuf::from("."),
                chosen_model: None,
                chosen_thinking: None,
                chosen_mode: None,
                catalogue: Vec::new(),
                next_seq: last_seq,
            },
        );
        let mailbox = coordinator.host.mailbox(To::Client(client.id()));
        let conversation = Conversation::start(agent_id, Box::new(Idle::new()), mailbox, last_seq);
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
