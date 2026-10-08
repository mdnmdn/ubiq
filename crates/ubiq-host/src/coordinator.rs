//! The coordinator: it starts harnesses, supervises them, and answers the bus.
//!
//! It runs on a thread of its own and shares nothing with the window but the channel pair in
//! [`ubiq_proto::bus`]. It renders nothing and has no opinion about layout or colour — everything here
//! is a pane ID, a pseudo-terminal and a process.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use ubiq_proto::ask::AskClosed;
use ubiq_proto::assist::{AssistProvider, SuggestSubject};
use ubiq_proto::bus::{ClientId, FromClient, HostEnd, MovingAddress, To};
use ubiq_proto::catalog::SkillAdd;
use ubiq_proto::conversation::{
    ConfigCategory, ConfigChoice, ConfigOption, ConfigValue, ConvUpdate, StopReason,
};
use ubiq_proto::files::{FileError, FileVersion};
use ubiq_proto::ids::{
    AskId, KbSourceId, PaneId, ProjectId, SearchId, SessionId, SshProfileId, SuggestId, ToolId,
};
use ubiq_proto::messages::{AgentPicks, CatalogueModel, Message, Secret, TaskField, WorkspaceInfo};
use ubiq_proto::notifications::{Family, NotificationRequest};
use ubiq_proto::projects::{IndexLevel, ProjectHealth, Scope};
use ubiq_proto::settings::{SettingsLayer, SshProfile};
use ubiq_proto::stats::{HostStats, UsageRow};
use ubiq_proto::tools::{ListedTool, ToolDef, ToolOrigin, ToolRun};
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

use crate::agent::{Agents, ConverseOptions, PendingLogin};
use crate::assist::{self, Assist, providers::Providers};
use crate::catalog::Catalog;
use crate::cli_shortcut;
use crate::config::ConfigRoot;
use crate::connectors::{Answer, Connectors};
use crate::conversation::{ConvFlags, Conversation, UsageMeter};
use crate::conversation_record::{self, ConversationRecord};
use crate::files::{self, Files};
use crate::git::{self, Git};
use crate::health;
use crate::help::Help;
use crate::host_meta::{self, HostMeta};
use crate::kb::{self, Kb};
use crate::notifications;
use crate::projects::Projects;
use crate::pty::{self, Pty};
use crate::reply::Reply;
use crate::repos::Repos;
use crate::search::{self, Search};
use crate::settings::Settings;
use crate::shell_integration;
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
/// The first wait after a doc prompt the conversation refused, doubled per refusal up to
/// [`DOC_RETRY_MAX`] (`D207`).
const DOC_RETRY_FIRST: Duration = Duration::from_secs(2);
const DOC_RETRY_MAX: Duration = Duration::from_secs(60);

/// How long one suggestion may take before it is answered with a failure instead. Generation
/// cannot be interrupted, so this bounds the wait rather than the work.
const SUGGEST_DEADLINE: Duration = Duration::from_secs(60);

/// How often a project's `tasks.toml` is re-read so an edit that happened outside this process
/// reaches a window that already has the board open.
const TASK_SYNC_EVERY: Duration = Duration::from_secs(2);

/// How old a project's discovered runner targets may be before an ask for its tools starts another
/// scan. The ask is still answered at once from what is held; this only paces the re-reads.
const RUNNER_FRESH: Duration = Duration::from_secs(3);

/// One knowledge-base source resolved for a mutation: the record, where it lives, and — for a
/// protected wiki — the key its documents are sealed with.
type KbResolved = (
    ubiq_proto::kb::KbSource,
    PathBuf,
    Option<Arc<kb::vault::Key>>,
);

/// What one finished runner scan hands back to the coordinator's thread.
struct Scanned {
    project_id: ProjectId,
    tools: Vec<(String, ToolDef)>,
}

/// A project's discovered runner targets and when they were read.
struct Discovered {
    tools: Vec<(String, ToolDef)>,
    at: Instant,
}

/// Gather what a subject needs and compose its prompt. Off the coordinator's thread, so the
/// repository read here is allowed to be slow.
fn gather(
    subject: &SuggestSubject,
    root: Option<&std::path::Path>,
    held: Option<String>,
    assist: &dyn Assist,
) -> Result<assist::Request, String> {
    match subject {
        SuggestSubject::CommitMessage { .. } => {
            let root = root.ok_or_else(|| "that project is not in the catalogue".to_string())?;
            // No managed set: a commit message names what the project's *own* repository is about
            // to commit, and a repository below it commits on its own.
            let observation = git::observe(root, 0, true, &[])
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
        // The one subject whose material is not on disk here: a task's description is in the work
        // store, which lives on the coordinator's own thread, so `suggest_job` read it there and
        // this only has to compose it.
        SuggestSubject::TaskTitle { .. } => {
            let description = held.unwrap_or_default();
            if description.trim().is_empty() {
                return Err("that task has no description to name it from".to_string());
            }
            Ok(assist::subject::task_title(&description, &assist.limits()))
        }
    }
}

/// Start the coordinator on its own thread. One per process, started before the first window: the
/// catalogue it will own is process-wide, and two of them would disagree about what exists.
///
/// It ends when the hub and every client have gone.
/// `providers` is the task-source registry this build ships, **handed in rather than built here**
/// (`D189`): it arrives seeded with the base's own providers and with whatever a second edition
/// registered before the first window, which is the whole of how a Studio provider reaches
/// `ListRemoteContainers`. The base binary hands in `Registry::with_defaults()`, unchanged.
/// `runners` is the same for the runner sources a project's tools are discovered from.
pub fn start(
    host: HostEnd,
    root: ConfigRoot,
    projects: Projects,
    work: Work,
    settings: Settings,
    providers: crate::tasksrc::Registry,
    runners: crate::runners::Registry,
    pending: Vec<Reply>,
) {
    thread::Builder::new()
        .name("ubiq-coordinator".to_string())
        .spawn(move || {
            Coordinator::new(
                host, root, projects, work, settings, providers, runners, pending,
            )
            .run()
        })
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
    /// Shared with the MCP listener as [`crate::work::Handle`].
    work: crate::work::Handle,
    /// A task's plan, one markdown document per mission. Shared with the MCP listener's
    /// `ubiq-plan` server as [`crate::plan::Handle`], on [`Self::work`]'s own footing.
    plans: crate::plan::Handle,
    /// A mission's own record, one directory per mission beside its plan. Shared on
    /// [`Self::plans`]'s footing, because the stage that adds `ubiq-mission` hands the same
    /// instance to the MCP listener.
    missions: crate::mission::Handle,
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
    /// The task-source sync worker: one thread, walking whatever projects it was last told about,
    /// pulling each bound board on that binding's own interval. Holds a clone of [`Self::work`],
    /// which is the whole of how it writes (`D120`). Dropping this handle ends the thread.
    tasksrc: crate::tasksrc::sync::Sync,
    /// The §3.6 wire family's host half (`D188`): providers, containers, lanes, the Test, the
    /// import dialog's listing, the force buttons, drift resolution and the binding file. Holds a
    /// clone of [`Self::work`] and the same sidecar gate the worker above does, and answers
    /// nothing that touches a network on this thread.
    tasksrc_service: crate::tasksrc::service::TaskSrc,
    /// The vendor bundles a web panel needs, on disk in the shared workarea. Holds a row per fetch
    /// in flight and nothing else — the download is a thread of its own, for the same reason a
    /// clone is.
    web_assets: WebAssets,
    /// Ubiq's own documentation: where the bundle this build ships was found, and the catalogue
    /// parsed out of it once. Answers every ask itself — no thread, no watcher list, unlike
    /// [`Self::web_assets`], for the reason [`crate::help`]'s module doc gives. `Arc` because the
    /// MCP listener's `ubiq-help` server holds the same instance on its own thread, on
    /// [`Self::kb`]'s own footing.
    help: Arc<Help>,
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
    /// A project's knowledge-base sources, and a git one's fetch. Listing and reading still go
    /// through [`Self::files`] — nothing here touches disk either — but the source list and its
    /// state are this family's own, on no project's catalogue record.
    ///
    /// `Arc` because the MCP listener's `ubiq-kb` server holds the same service on its own thread:
    /// the `Syncing`/`Failed` overrides live in memory, so a second `Kb` would answer a state this
    /// one has never heard of.
    kb: Arc<Kb>,
    /// A project's databases: the saved connections, their sealed passwords, and the sessions that
    /// run SQL. Every driver call blocks, so none runs here — a tab's work is on that tab's own
    /// thread, and this only hands the message over with the addresses to answer to.
    db: crate::db::Db,
    /// The thread that reads a project's repository. A status walk is seconds on a large tree, and
    /// seconds here would stall every pane behind it.
    git: Git,
    /// The thread that walks a project's files for content search. Long-running by nature, so it
    /// has its own queue: a search behind a slow one would stall every folder expand.
    search: Search,
    /// The full-text index, one open per project that keeps one.
    index: crate::index::Index,
    /// The thread that asks a provider how much of an account's plan is left. A probe is a
    /// blocking HTTPS call to an endpoint that rate-limits, so it never happens on this thread.
    quota: crate::quota::Quota,
    /// What each account last read. In memory and never written down: unlike spend, what is left
    /// is re-derivable by asking again, so a restart re-probes — see [`crate::quota`].
    quotas: crate::quota::Quotas,
    /// The thread that posts the user's report to whatever destination this build was compiled
    /// with. A blocking HTTPS call, so it is a thread for [`crate::quota`]'s reason.
    feedback: crate::feedback::Feedback,
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
    /// Each harness pane's handle (`claude 2`), minted by `unique_name` over the same namespace
    /// as the project's conversations — see `taken_names`. A shell or a tool pane has none here.
    pane_handles: HashMap<PaneId, String>,
    /// Which session each pane belongs to, so `stats()` can count live sessions. Ubiq's sense of
    /// session — a named grouping of panes — not the harness library's resumable conversation.
    pane_sessions: HashMap<PaneId, SessionId>,
    /// Which tool each pane started by [`Message::RunTool`] is running, so a tool marked
    /// `single_instance` can be refused a second pane. Only tool panes have an entry, and it
    /// goes with the pane in [`Self::pane_gone`].
    ///
    /// Here rather than in a window, because a window only sees its own panes and the rule is
    /// "one run at a time" on the machine, not one per window.
    pane_tools: HashMap<PaneId, ToolId>,
    /// The runner sources a scan asks, shared with the scan's worker thread. Handed in at boot.
    runners: Arc<crate::runners::Registry>,
    /// Each project's discovered runner targets, as the last finished scan read them. Never
    /// written to a store: a target is read again, and [`Self::find_tool`] resolves a run of one
    /// from here.
    discovered: HashMap<ProjectId, Discovered>,
    /// Projects with a scan in flight — one each, so a window asking in a loop starts one thread.
    scanning: HashSet<ProjectId>,
    /// The project each window last asked the tools of, so a scan that finishes later can say its
    /// answer again to exactly the windows still looking at that project.
    tool_askers: HashMap<ClientId, ProjectId>,
    /// Where a scan's worker leaves its answer; drained by [`Self::collect_scans`] every turn of
    /// the run loop, and woken by a voice message so it is not left until the next tick.
    scans: (flume::Sender<Scanned>, flume::Receiver<Scanned>),
    panes: HashMap<PaneId, Pty>,
    /// Which window owns which pane, recorded when the pane is spawned. This is the whole routing
    /// table: everything a pane emits goes to its owner, and nobody else may drive it.
    owners: HashMap<PaneId, ClientId>,
    /// The handle that re-addresses a pane's own reader and reaper, for every pane that belongs to
    /// a project.
    ///
    /// `owners` decides what the coordinator sends and what it accepts, both resolved per message;
    /// a pane's reader has neither, because it holds one mailbox for the life of the harness and
    /// runs on its own thread. So a project moving between windows has two halves, and this is the
    /// other one — see [`Self::adopt_project`]. A login pane has no entry: it belongs to no
    /// project and there is nothing that could move it.
    pane_addresses: HashMap<PaneId, MovingAddress>,
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
    /// What the user said on annotated documents, waiting for the agent bound to them — at most
    /// one pending prompt per agent, delivered when the agent is live and idle (`D207`).
    doc_queue: crate::plan::queue::DocQueue,
    /// When each agent whose doc prompt was refused may be tried again, and the wait after that —
    /// doubled per refusal, so a broken input is not retried, logged and broadcast every poll.
    doc_backoff: HashMap<AgentId, (Instant, Duration)>,
    /// Every question an agent has put to the user and not yet had an answer to. `Arc` because
    /// the MCP listener parks its tool calls on the same table this thread answers into — the two
    /// halves of an ask never meet anywhere else (`D138`, [`crate::ask`]).
    asks: Arc<crate::ask::Asks>,
    /// Every dialog an agent registered for the end of its turn and not yet had an answer to.
    /// `Arc` for the reason [`Self::asks`] is, and between the same threads: the MCP listener arms
    /// a row, the conversation's pump raises it at the turn boundary, and this thread turns the
    /// answer into the next turn's prompt (`D175`, [`crate::armed`]).
    armed: Arc<crate::armed::Armed>,
    /// The launch recipe for every agent this window has asked for, kept for the agent's whole
    /// life rather than only until its first launch. Before a first [`Message::PromptAgent`] it is
    /// what P3's loader is waiting on; after `UnloadConversation` it is what
    /// `ResumeConversation` (or the next `PromptAgent`) relaunches from. Only
    /// [`Message::EndConversation`] removes an entry — a live agent's own row stays put beside its
    /// [`Self::conversations`] one, holding the picks (`chosen_model` and friends) a relaunch must
    /// not lose.
    pending_conversations: HashMap<AgentId, PendingConversation>,
    /// The sign-ins running in a pane, keyed by it. Whether one worked is only answerable once
    /// its process has exited, so what the answer needs is parked here until then — the same
    /// shape `active_searches` uses, and forgotten the same way.
    logins: HashMap<PaneId, PendingLogin>,
    /// The device-code sign-ins under way, by harness and account, and the window that asked —
    /// the one their outcome goes to. A second ask for the same pair replaces the first's window.
    device_logins: HashMap<(String, String), ClientId>,
    /// When this coordinator started, which is what the stats screen calls uptime. Nothing else
    /// measured elapsed time before, so this is where it comes from.
    // ponytail: measured from the coordinator's thread rather than from `main`. The difference is
    // the few milliseconds spent finding the config root, which nobody reading "uptime 4m" can
    // perceive; move it to `main` and pass it in if a figure in milliseconds ever matters.
    started: Instant,
    /// When the loaded task files were last compared to disk, which is what paces
    /// [`Self::sync_tasks_due`].
    tasks_synced: Instant,
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
    /// below, because those outrank a definition inside `resolve`.
    definition: Option<String>,
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
    /// The last title the harness named itself with that was put on the live `WorkAgent`.
    ///
    /// What lets a harness retitle itself without ever overwriting a rename or a generated
    /// title: its next title is adopted only while the agent's current one is still this (or
    /// there is none). In memory only, so after a restart the restored title holds.
    harness_title: Option<String>,
    /// The harness's own session id, as the last process to run this conversation reported it.
    ///
    /// **This is what makes a one-shot harness converse.** Such a harness answers once and exits,
    /// so the next turn is a new process — and the only thing joining the two is this id, handed
    /// back to the library as the run's resume. `None` before the first run, and for a harness
    /// that named none.
    resume: Option<String>,
    /// Whether the next launch forks [`Self::resume`] rather than continuing it — set by a fork,
    /// cleared the moment the fork's own harness names its session, so a relaunch after that
    /// continues the fork rather than forking the source again.
    fork: bool,
    /// The MCP servers this conversation asked for, from [`Message::StartConversation::mcps`],
    /// carried through to [`ConverseOptions::mcps`] on every launch and relaunch.
    mcps: Vec<String>,
    /// The skills this conversation asked for, from [`Message::StartConversation::skills`],
    /// carried through to [`ConverseOptions::skills`] the same way.
    skills: Vec<String>,
    /// The extra read-write folders this launch asked for, from
    /// [`Message::StartConversation::grants`], carried through to [`ConverseOptions::extra_rw`].
    extra_rw: Vec<String>,
}

/// The paths of the read-write grants a launch carried. A grant on a start is a write grant by
/// contract; a read-only one is dropped rather than widened or honoured differently.
fn rw_paths(grants: Vec<ubiq_proto::settings::Grant>) -> Vec<String> {
    grants
        .into_iter()
        .filter(|grant| grant.write)
        .map(|grant| grant.path)
        .collect()
}

/// Whether a mutation's replies carry a refusal.
fn failed(replies: &[Reply]) -> bool {
    replies
        .iter()
        .any(|reply| matches!(reply.message(), Message::PlanError { .. }))
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

/// Change one field of a live conversation's `WorkAgent`, refresh its MCP identity to match, and
/// tell every window with an `AgentChanged`.
///
/// A free function rather than a method because the naming thread calls it too: the title it
/// writes has to land on the live record (or the next `AgentChanged` from anywhere reverts it,
/// T-283's B1), and that thread holds no `&self`. The identity is refreshed on every call, not
/// only on a naming — it is one map write, and a field that changed is the only reason to be here.
fn publish_live_agent(
    work: &crate::work::Handle,
    registry: &crate::mcp::Registry,
    everyone: &ubiq_proto::bus::Mailbox,
    project_id: ProjectId,
    agent_id: AgentId,
    set: impl FnOnce(&mut WorkAgent),
) {
    let changed = work
        .lock()
        .live_agent_mut(project_id, agent_id)
        .map(|agent| {
            set(agent);
            Box::new(agent.clone())
        });
    if let Some(agent) = changed {
        registry.set_name(&agent_id.to_string(), agent.title_or_handle());
        everyone.send(Message::AgentChanged { project_id, agent });
    }
}

/// A refusal in the host settings layer, which is the layer the ssh profiles live in.
fn settings_error(error: String) -> Reply {
    Reply::Asker(Message::SettingsError {
        layer: SettingsLayer::Host,
        error,
    })
}

/// The host layer's whole record, to every window: settings are drawn wherever one is open, and a
/// flag that moved here moved for all of them.
fn host_settings(host: &ubiq_proto::settings::HostSettings) -> Reply {
    Reply::Everyone(Message::Settings {
        layer: SettingsLayer::Host,
        value: serde_json::to_string(host).ok(),
    })
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
        providers: crate::tasksrc::Registry,
        runners: crate::runners::Registry,
        pending: Vec<Reply>,
    ) -> Self {
        let runners = Arc::new(runners);
        // Shared with the MCP listener so an agent can read and write the same board a window
        // does, without asking this thread a question (`D120`).
        let work = crate::work::Handle::new(work);
        // The plan store has exactly one implementation, so it is built here rather than handed
        // in — [`Kb`] and [`Help`] below are on the same footing. It holds its own clone of `work`
        // to check a task's level before it hands out or accepts a plan; see `crate::plan`'s
        // module doc for why that does not mean `crate::mcp::PlanReach` holds one too.
        let plans = crate::plan::Handle::new(crate::plan::Plans::open(
            crate::store::plan::FilePlanStore::new(root.path.clone()),
            work.clone(),
        ));
        // The missions, on the plans' own footing and for the same reasons: one implementation, so
        // it is built here; a work handle of its own, so a mission's refusals are the anchor
        // task's; a plan store of its own, so a record made lazily can infer its phase from
        // whether a plan has been written (`crate::mission`).
        let missions = crate::mission::Handle::new(crate::mission::Missions::open(
            crate::store::mission::MissionStore::new(root.path.clone()),
            work.clone(),
            crate::store::plan::FilePlanStore::new(root.path.clone()),
            // How a mission line reaches a *live* agent rather than only its row: half of
            // `Missions` runs on the MCP listener's thread, where the conversation map is out of
            // reach, so it says `PromptAgent` into this same inbox (`crate::mission`).
            host.voice(),
        ));
        // Ubiq's own MCP servers, on one loopback port for every agent this process will start.
        // Bound *before* the agents are built, because the URL a run is composed with is written
        // into the harness's configuration and never revisited — there is no later moment to tell
        // a run where the port is. A port that will not bind is said once, here, and every run
        // then composes with no injected servers rather than failing (`crate::mcp`).
        // Shared with the MCP listener for the same reason the board is: the `ubiq-kb` server
        // reads and writes the same sources a window does, and the in-flight sync states live in
        // this one object's memory.
        // A protected wiki's password is filed in the OS keychain through the connector family's
        // store, the one place Ubiq files secrets (`D206`).
        let kb = Arc::new(Kb::with_keychain(
            root.path.clone(),
            Arc::new(crate::connectors::store::Store::open(&root.path)),
            kb::vault::KdfParams::default(),
        ));
        // Reserved here rather than beside `web_assets` below: the MCP listener's `ubiq-help`
        // server needs it started before it can be handed to `mcp::start`, on the same footing as
        // the knowledge base above. It belongs to no project, and the interface is told a path
        // rather than a maybe — a root that will not take the directory is still named, because
        // what is kept there is a cache and downgrades rather than fails.
        let shared_workarea_path = crate::projects::reserve_shared_workarea(&root.path);
        let help = Arc::new(Help::new(std::path::PathBuf::from(&shared_workarea_path)));
        let mcp_agents = crate::mcp::Registry::new();
        // The parked questions, shared with the listener for the reason the knowledge base is:
        // `ubiq-ask`'s tool waits on this table from its own thread and this one answers into it.
        let asks = Arc::new(crate::ask::Asks::new());
        // The registered dialogs, shared the same way and with the same two halves — armed on the
        // listener's thread, raised on a pump's, answered on this one.
        let armed = Arc::new(crate::armed::Armed::new());
        // The database family, built here so the SQL servers can be handed a handle on it: they run
        // on the listener's own threads and reach the same sessions and editors a window does
        // (`D202`).
        let db = crate::db::Db::new(root.path.clone());
        let mcp = crate::mcp::start(
            mcp_agents.clone(),
            host.voice(),
            Some(crate::mcp::WorkAccess {
                work: work.clone(),
                everyone: host.mailbox(To::Everyone),
                wake: host.voice(),
            }),
            Some(crate::mcp::PlanReach {
                plans: plans.clone(),
                everyone: host.mailbox(To::Everyone),
                agents: mcp_agents.clone(),
            }),
            // The missions, on the plans' own footing: one handle, shared with the listener, so
            // `ubiq-mission` writes the same records and the same journal a window reads (`D120`).
            Some(crate::mcp::MissionReach {
                missions: missions.clone(),
                everyone: host.mailbox(To::Everyone),
                agent_definitions_root: root.path.clone(),
            }),
            Some(crate::mcp::KbReach {
                kb: kb.clone(),
                everyone: host.mailbox(To::Everyone),
            }),
            Some(crate::mcp::HelpReach { help: help.clone() }),
            Some(crate::mcp::AskReach {
                asks: asks.clone(),
                armed: armed.clone(),
            }),
            Some(crate::mcp::SqlReach::new(
                db.agent_handle(host.mailbox(To::Everyone)),
            )),
            Some(crate::mcp::ArchifyReach {
                everyone: host.mailbox(To::Everyone),
            }),
        )
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
        // A fresh install gets its three agent definitions here, once, before any window asks
        // for the list — and only when the root holds none, so nobody's own list is added to.
        if let Err(error) = agents.ensure_default_definitions() {
            tracing::warn!("the default agent definitions could not be written: {error:#}");
        }
        let settings = Arc::new(settings);
        let connectors = Connectors::new(settings.clone(), &root.path);
        let repos = Repos::new(settings.clone(), connectors.store());
        // The task-source sync worker: the **third** holder of the work handle, after this thread
        // and the MCP listener, and for the same reason (`D120`). It polls each bound project's
        // board on that binding's own interval and writes what it pulls through `Work`, so a
        // pulled task is an ordinary task and every window redraws without this thread answering a
        // question. It holds no catalogue — `sync_tasks_due` hands it the project list.
        // Whatever this edition registered, seeded with the base's own (`D189`). Nothing is added
        // here: a provider this thread built itself could not be substituted or removed, and the
        // seam would have a base-side user and no contributor — which is the shape the extension
        // pattern exists to refuse.
        let tasksrc_registry = Arc::new(providers);
        // Two instances over one root rather than one shared: `ProjectDirs` holds a memo of the
        // pointer files it has read, not state, so a second one costs a second lookup at worst and
        // spares the family an `Arc` it would otherwise need for nothing.
        let tasksrc_dirs = crate::store::project_dir::ProjectDirs::new(root.path.clone());
        // One gate over every `tasksrc.toml`, shared by the poll and by the wire family's own
        // threads. Two passes over one project's sidecar — the five-minute tick and a force button
        // pressed during it — are otherwise last-writer-wins over the link table.
        let tasksrc_gate = Arc::new(std::sync::Mutex::new(()));
        let tasksrc = crate::tasksrc::sync::start(crate::tasksrc::sync::Job {
            work: work.clone(),
            everyone: host.mailbox(To::Everyone),
            dirs: tasksrc_dirs,
            registry: tasksrc_registry.clone(),
            settings: settings.clone(),
            connections: connectors.store(),
            gate: tasksrc_gate.clone(),
        });
        // The same registry and the same gate, answering the §3.6 family M7 drew a page against
        // (`D188`, `G364`). Every arm that touches a network runs on a thread of its own.
        let tasksrc_service = crate::tasksrc::service::TaskSrc::new(
            tasksrc_registry,
            settings.clone(),
            connectors.store(),
            crate::store::project_dir::ProjectDirs::new(root.path.clone()),
            work.clone(),
            tasksrc_gate,
        );
        // `shared_workarea_path` was reserved above, before `mcp::start`, because `help` had to
        // exist by then; kept under this name for every use from here on, the way `web_assets`
        // has always been told it.
        let shared_workarea = shared_workarea_path;
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
        // Taken before the root is moved into the struct: the quota worker probes from a thread of
        // its own and everything it needs is a path.
        let quota_root = root.path.clone();
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
                        definition: row.definition,
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
                        harness_title: None,
                        resume: None,
                        fork: false,
                        // Not part of the persisted row yet: an mcp or skill pick made before a
                        // restart does not survive one, same as `catalogue` above.
                        mcps: Vec::new(),
                        skills: Vec::new(),
                        extra_rw: Vec::new(),
                    },
                )
            })
            .collect();

        Self {
            host,
            root,
            projects,
            work,
            plans,
            missions,
            settings,
            connectors,
            repos,
            tasksrc,
            tasksrc_service,
            web_assets,
            help,
            shared_workarea,
            agents,
            catalogue,
            files: Files::start(),
            kb,
            db,
            git: Git::start(),
            quota: crate::quota::Quota::start(quota_root),
            quotas: crate::quota::Quotas::new(),
            feedback: crate::feedback::Feedback::start(),
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
            pane_handles: HashMap::new(),
            pane_sessions: HashMap::new(),
            pane_tools: HashMap::new(),
            runners,
            discovered: HashMap::new(),
            scanning: HashSet::new(),
            tool_askers: HashMap::new(),
            scans: flume::unbounded(),
            panes: HashMap::new(),
            owners: HashMap::new(),
            pane_addresses: HashMap::new(),
            focused: HashMap::new(),
            conversations: HashMap::new(),
            conversation_owners: HashMap::new(),
            doc_queue: Default::default(),
            doc_backoff: HashMap::new(),
            asks,
            armed,
            pending_conversations,
            logins: HashMap::new(),
            device_logins: HashMap::new(),
            started: Instant::now(),
            tasks_synced: Instant::now(),
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
            let wait = match wait {
                Some(wait) => Some(wait.min(TASK_SYNC_EVERY)),
                None => Some(TASK_SYNC_EVERY),
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
            self.collect_scans();
            self.sync_tasks_due();
            self.remember_sessions();
            self.name_conversations();
            self.adopt_harness_titles();
            self.reap_conversations();
            self.flush_doc_queue();
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
        self.tool_askers.remove(&client);
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
        // Its database sessions close, and what they were running is stopped.
        self.db.client_gone(client);
    }

    /// A project has moved to another window: everything running in it is now that window's.
    ///
    /// Only the routing changes. No pseudo-terminal is opened or closed, no harness is signalled
    /// and no conversation is stopped — `owners` and `conversation_owners` are the whole of what
    /// a pane or a conversation belongs to, so re-homing them is the whole of the move. From here
    /// the pane's bytes are addressed to `client`, and the keystrokes, resizes and closes it sends
    /// are the ones [`Self::owns`] lets through.
    ///
    /// A focus the previous owner recorded goes with the pane rather than staying behind: the
    /// window that no longer draws the pane cannot be the one typing into it, and the new owner
    /// says where its own keyboard is with its own [`Message::Focus`].
    fn adopt_project(&mut self, client: ClientId, project_id: ProjectId) {
        let moved: Vec<PaneId> = self
            .pane_projects
            .iter()
            .filter(|(_, owned_by_project)| **owned_by_project == project_id)
            .map(|(pane_id, _)| *pane_id)
            .collect();
        for pane_id in &moved {
            let previous = self.owners.insert(*pane_id, client);
            if previous == Some(client) {
                continue;
            }
            // The reader and the reaper hold a mailbox rather than consulting `owners`, so the
            // bytes only follow the pane because of this.
            if let Some(address) = self.pane_addresses.get(pane_id) {
                address.point_at(To::Client(client));
            }
            if let Some(previous) = previous {
                if self.focused.get(&previous) == Some(pane_id) {
                    self.focused.remove(&previous);
                }
                tracing::info!("pane {pane_id} moved from {previous} to {client}");
            }
        }

        let mut agents = 0usize;
        for (owner, owned_by_project) in self.conversation_owners.values_mut() {
            if *owned_by_project == project_id && *owner != client {
                *owner = client;
                agents += 1;
            }
        }
        tracing::info!(
            "{client} adopted project {project_id}: {} panes, {agents} conversations",
            moved.len()
        );
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
            // A clone is Ubiq-managed: nothing in the clone request says otherwise, and writing
            // a `.ubiq/` into a tree that was just fetched from somebody else's remote is not
            // something the user asked for.
            let replies = self.projects.add(
                &done.path,
                Some(done.name),
                None,
                None,
                done.ephemeral,
                ubiq_proto::projects::StorageMode::UbiqManaged,
            );
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
        self.pane_addresses.remove(&pane_id);
        self.pane_sessions.remove(&pane_id);
        // A single-instance tool's next run is allowed the moment its pane is gone, so this goes
        // with the pane rather than being swept later.
        self.pane_tools.remove(&pane_id);
        // Its handle is free for the next pane or conversation the moment it is gone.
        self.pane_handles.remove(&pane_id);
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

    /// Every handle in use in one project: its live conversations' names and its harness panes'
    /// handles. One namespace, so `unique_name` never hands a pane and a conversation the same one.
    fn taken_names(&self, project_id: ProjectId) -> Vec<String> {
        let mut taken = self.work.lock().live_agent_names(project_id);
        taken.extend(
            self.pane_handles
                .iter()
                .filter(|(pane, _)| self.pane_projects.get(pane) == Some(&project_id))
                .map(|(_, handle)| handle.clone()),
        );
        taken
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

            // Stop the process, keep the pane: the reaper's `PaneExited` follows, and the pane then
            // closes or waits the way its tool row says. `Pty::kill` is the same SIGHUP / terminate
            // a close sends — it is what `CloseWorkspace` does minus the removal.
            Message::StopPane { pane_id } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                match self.panes.get_mut(&pane_id) {
                    Some(pane) => {
                        tracing::info!("stopping pane {pane_id}");
                        pane.kill();
                    }
                    None => tracing::debug!("stop for pane {pane_id}, which holds no process"),
                }
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
            // A finished runner scan waking the loop (see `scan_runners`): `collect_scans` runs
            // after this turn and does the whole of it.
            Message::ListTools { .. } if client.is_host_voice() => {}
            Message::ListTools { project_id } => {
                match project_id {
                    Some(project_id) => {
                        self.tool_askers.insert(client, project_id);
                        self.scan_runners(project_id);
                    }
                    None => {
                        self.tool_askers.remove(&client);
                    }
                }
                self.send_tools(client, project_id);
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
                storage,
            } => {
                let replies =
                    self.projects
                        .add(&path, name, colour, custom_colour, temporary, storage);
                self.answer(client, replies);
            }
            Message::ForgetProject { project_id } => {
                let replies = self.projects.forget(project_id);
                // The catalogue went first and took the project's directory with it, so the
                // tasks on disk are already gone. This is the memory that was left, and this
                // is the only place that knows both services.
                self.work.lock().forget(project_id);
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
                mission_term,
                tools,
                managed_repos,
                lanes,
                runs_on,
                definitions_use_global,
                definitions_allowed,
            } => {
                let managed_changed = managed_repos.is_some();
                let replies = self.projects.update(
                    project_id,
                    name,
                    colour,
                    custom_colour,
                    search_excludes,
                    index,
                    mission_term,
                    tools,
                    managed_repos,
                    lanes,
                    runs_on,
                    definitions_use_global,
                    definitions_allowed,
                );
                self.answer(client, replies);
                // A level the user just changed takes effect now, not at the next open: turning
                // indexing off and watching the disk not free up would read as a setting that
                // does nothing.
                if index.is_some() {
                    self.settle_index(project_id);
                }
                // The same for a repository just taken on or let go: the badges and the Git screen
                // read the observation, so it is redone now rather than at the next restart.
                if managed_changed {
                    self.git_job(client, project_id, String::new(), git::Request::Full);
                }
                // `runs_on` gets no such settle: per D116 the coordinator never learns a drone
                // exists. It is stored and rebroadcast as part of the record above, and that is
                // the whole of the coordinator's involvement — no reader is spawned, no relay is
                // dialed, nothing here treats the project as remote. The absence is the decision.
            }
            Message::SetProjectInitials {
                project_id,
                initials,
            } => {
                let replies = self.projects.set_initials(project_id, &initials);
                self.answer(client, replies);
            }
            Message::SetProjectStorage {
                project_id,
                storage,
            } => {
                // A conversation has no pane, so `Projects` cannot see it: its own refusal
                // counts panes, and this one counts the agents in the chat panel. Together they
                // are the whole of "something is running in this project" — the two routing
                // tables the coordinator already keeps to decide where a message goes.
                if self
                    .conversation_owners
                    .values()
                    .any(|(_, owner)| *owner == project_id)
                {
                    self.host.send(
                        To::Client(client),
                        Message::ProjectStorageError {
                            project_id,
                            storage,
                            error: "end the agents running in this project first".to_string(),
                        },
                    );
                    return;
                }
                let replies = self.projects.set_storage(project_id, storage);
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
            Message::AdoptProject { project_id } => self.adopt_project(client, project_id),
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
            // Answered inline, with no thread and no process: this is a read of what a past
            // handshake recorded, not a probe. A harness nothing has conversed with answers
            // `None`, which is the truth rather than a failure.
            Message::ListAcpCapabilities { agent_type } => {
                let capabilities = self.catalogue.acp_capabilities(&agent_type);
                self.host.send(
                    To::Client(client),
                    Message::AcpCapabilities {
                        agent_type,
                        capabilities,
                    },
                );
            }

            Message::ListAccounts => {
                self.send_accounts(client);
            }
            Message::ListAgentDefinitions => {
                self.send_definitions(client);
            }
            Message::SaveAgentDefinition { definition } => {
                match self.agents.save_definition(definition) {
                    Ok(()) => self.send_definitions(client),
                    Err(error) => self.host.send(
                        To::Client(client),
                        Message::AccountError {
                            error: format!("{error:#}"),
                        },
                    ),
                }
            }
            Message::CloneAgentDefinition {
                id,
                new_id,
                project,
            } => match self.agents.clone_definition(&id, &new_id, project) {
                Ok(()) => self.send_definitions(client),
                Err(error) => self.host.send(
                    To::Client(client),
                    Message::AccountError {
                        error: format!("{error:#}"),
                    },
                ),
            },
            Message::DeleteAgentDefinition { id, project } => {
                match self.agents.delete_definition(&id, project) {
                    Ok(()) => self.send_definitions(client),
                    Err(error) => self.host.send(
                        To::Client(client),
                        Message::AccountError {
                            error: format!("{error:#}"),
                        },
                    ),
                }
            }
            // The catalogue is a constant of this build, so it is answered from the table itself
            // rather than from anything running: the panel's checklist and the tools a harness
            // will be offered come from the one place, and cannot disagree (`crate::mcp`).
            Message::ListMcps => self.host.send(
                To::Client(client),
                Message::Mcps {
                    servers: crate::mcp::catalogue(),
                },
            ),
            // ── the skill and MCP catalog ───────────────────────────
            // A layer is a directory the library reads and writes (`crate::catalog`). What can
            // reach the network — an install, a source search, a registry search — runs on a
            // thread of its own; the rest is a file edit and is answered where it is asked.
            Message::ListCatalog { scope } => {
                let answer = if !self.catalog_scope_known(scope) {
                    Message::CatalogError {
                        error: "no such project".to_string(),
                    }
                } else {
                    Catalog::new(self.root.path.clone())
                        .snapshot(scope)
                        .unwrap_or_else(|error| Message::CatalogError {
                            error: format!("{error:#}"),
                        })
                };
                self.host.send(To::Client(client), answer);
            }
            Message::AddSkill {
                scope,
                from: SkillAdd::Remote { source, path, id },
            } => self.catalog_change(client, scope, true, move |catalog| {
                catalog.install_remote(scope, &source, &path, id.as_deref())
            }),
            Message::AddSkill { scope, from } => {
                self.catalog_change(client, scope, false, move |catalog| {
                    catalog.add_skill(scope, &from)
                })
            }
            Message::RemoveSkill { scope, id } => {
                self.catalog_change(client, scope, false, move |catalog| {
                    catalog.remove_skill(scope, &id)
                })
            }
            Message::RemoveSkillFolder { scope, path } => {
                self.catalog_change(client, scope, false, move |catalog| {
                    catalog.remove_skill_folder(scope, &path)
                })
            }
            Message::SaveSkillSources { sources } => {
                self.catalog_change(client, None, false, move |catalog| {
                    catalog.save_sources(&sources)
                })
            }
            Message::SaveCatalogMcp {
                scope,
                mcp,
                previous_id,
            } => self.catalog_change(client, scope, false, move |catalog| {
                catalog.save_mcp(scope, &mcp, previous_id.as_deref())
            }),
            Message::RemoveCatalogMcp { scope, id } => {
                self.catalog_change(client, scope, false, move |catalog| {
                    catalog.remove_mcp(scope, &id)
                })
            }
            Message::SearchSkills { query, source } => {
                let catalog = Catalog::new(self.root.path.clone());
                self.catalog_answer(client, "skill-search", move || {
                    catalog.search_skills(&query, source.as_deref())
                });
            }
            Message::ParseMcpConfig { text } => {
                self.host
                    .send(To::Client(client), Catalog::parse_config(&text));
            }
            Message::SearchMcpRegistry { query, cursor } => {
                self.catalog_answer(client, "mcp-registry", move || {
                    Catalog::search_mcp_registry(&query, cursor.as_deref())
                });
            }
            Message::BeginHarnessLogin {
                agent_type,
                account,
            } => {
                self.begin_harness_login(client, agent_type, account);
            }
            Message::BeginDeviceLogin {
                agent_type,
                account,
            } => {
                self.begin_device_login(client, agent_type, account);
            }
            // A device-code sign-in's thread reporting its outcome through the host's voice (see
            // `begin_device_login`): the outcome goes to the window that asked, and a success is
            // recorded exactly as a pane sign-in's clean exit is.
            Message::HarnessHomeSignedIn {
                agent_type,
                account,
            } if client.is_host_voice() => {
                self.device_login_ended(agent_type, account, None);
            }
            Message::HarnessLoginFailed {
                agent_type,
                account,
                error,
            } if client.is_host_voice() => {
                self.device_login_ended(agent_type, account, Some(error));
            }
            Message::CheckHarnessLogin {
                agent_type,
                account,
            } => {
                let status = self.agents.check_login(&agent_type, &account, now_ms());
                self.host.send(
                    To::Client(client),
                    Message::HarnessLoginStatus {
                        agent_type,
                        account,
                        status,
                    },
                );
            }
            Message::DeleteHarnessLogin {
                agent_type,
                account,
            } => match self.agents.delete_harness_login(&agent_type, &account) {
                Ok(()) => self.send_accounts(client),
                Err(error) => self.account_error(client, format!("{error:#}")),
            },
            Message::RenameAccount {
                account,
                new_account,
            } => {
                self.rename_account(client, account, new_account);
            }
            Message::DeleteAccount { account } => {
                self.delete_account(client, account);
            }

            // ── the feedback family ─────────────────────────────────
            // Where a build reports to is compiled in, so the offer is read rather than asked;
            // the report itself is a blocking POST and goes to the worker, which answers the
            // window that sent it.
            Message::QueryFeedback => {
                self.host.send(
                    To::Client(client),
                    Message::FeedbackOffered {
                        offer: self.feedback.offer(),
                    },
                );
            }
            Message::SendFeedback { report } => {
                self.feedback.submit(crate::feedback::Job {
                    report,
                    reply_to: self.host.mailbox(To::Client(client)),
                });
            }

            // ── Quota family: how much of an account's plan is left ──
            Message::QueryQuota {
                account,
                harness,
                fresh,
            } => {
                self.query_quota(client, account, harness, fresh);
            }
            // Filed by a host-side thread rather than asked by a window: the quota worker says
            // this when a probe worked, and a conversation's pump says it when a running harness
            // pushed a reading mid-turn. Both reach here through `bus::Voice`, because `quotas`
            // is this thread's — the same rule a finished clone's `Registered` obeys.
            Message::QuotaChanged {
                account,
                harness,
                snapshot,
            } => {
                if self.quotas.put(snapshot.clone()) {
                    // Every window showing that account is looking at the same fact, so all of
                    // them hear it — the precedent `ProjectFilesChanged` sets.
                    self.host.send(
                        To::Everyone,
                        Message::QuotaChanged {
                            account,
                            harness,
                            snapshot,
                        },
                    );
                }
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

            // ── Task-source family: binding a board somewhere else ──
            // `D188`, and the half `G364` recorded as missing: M7 drew every surface and this
            // thread answered none of them. The split is the repository family's — what a file
            // answers is answered here, and everything that touches a network is a thread of its
            // own. A listing goes back to its asker with the query id it was minted under; a pass
            // changes the project's tasks, so it goes to everyone.
            Message::ListTaskProviders { query_id } => {
                let (asker, _) = self.sinks(client);
                self.tasksrc_service.providers(query_id, &asker);
            }
            Message::ListRemoteContainers {
                query_id,
                provider,
                connection,
            } => {
                let (asker, _) = self.sinks(client);
                self.tasksrc_service
                    .containers(query_id, provider, connection, asker);
            }
            Message::ListRemoteLanes { query_id, binding } => {
                let (asker, _) = self.sinks(client);
                self.tasksrc_service.lanes(query_id, *binding, asker);
            }
            Message::TestTaskSource { query_id, binding } => {
                let (asker, _) = self.sinks(client);
                self.tasksrc_service.test(query_id, *binding, asker);
            }
            Message::GetTaskSource { project_id } => {
                let (asker, _) = self.sinks(client);
                self.tasksrc_service.get(project_id, &asker);
            }
            Message::SetTaskSource {
                project_id,
                binding,
            } => {
                let (asker, everyone) = self.sinks(client);
                self.tasksrc_service
                    .set(project_id, binding.map(|held| *held), asker, everyone);
            }
            Message::ListRemoteItems {
                query_id,
                project_id,
                binding,
            } => {
                let (asker, _) = self.sinks(client);
                self.tasksrc_service
                    .items(query_id, project_id, *binding, asker);
            }
            Message::ImportRemoteItems { project_id, items } => {
                let (_, everyone) = self.sinks(client);
                self.tasksrc_service.import(project_id, items, everyone);
            }
            Message::SyncTaskSource { project_id, scope } => {
                let (_, everyone) = self.sinks(client);
                self.tasksrc_service.sync(project_id, scope, everyone);
            }
            Message::ResolveTaskDrift {
                project_id,
                task_id,
                side,
            } => {
                let (_, everyone) = self.sinks(client);
                self.tasksrc_service
                    .resolve(project_id, task_id, side, everyone);
            }
            Message::UnlinkTask {
                project_id,
                task_id,
            } => {
                let (_, everyone) = self.sinks(client);
                self.tasksrc_service.unlink(project_id, task_id, &everyone);
            }

            // The vendor bundle a web panel needs. Everything after this — the progress, the
            // answer, the failure — comes from a thread of its own, except the one case that
            // touches no network: a bundle already on disk is answered from here.
            Message::EnsureWebBundle { app } => {
                let (asker, _) = self.sinks(client);
                self.web_assets.ensure(client, &app, asker);
            }

            // Ubiq's own documentation. Resolved and, if needed, unpacked inline — no thread and
            // no watcher list, because this is nothing like the web bundle above: a help bundle
            // is already local, and the answer is cached after the first ask (`crate::help`'s
            // module doc says why).
            Message::EnsureHelp => {
                let (asker, _) = self.sinks(client);
                self.help.ensure(&asker);
            }

            // The `ubiq` command on PATH. Every path in the exchange is the host's: the interface
            // says which of the three things to do and is told what is there afterwards.
            Message::CliShortcut { action } => {
                let reply = Reply::Asker(cli_shortcut::handle(action));
                self.answer(client, vec![reply]);
            }

            // *Open in Ubiq* in the file manager's context menu, on CliShortcut's terms: every key
            // and path is the host's, the interface names one of three actions, and the answer is
            // read back from the machine rather than remembered. A handful of registry calls on
            // Windows, nothing at all elsewhere — inline, like the shortcut's directory listing.
            Message::ShellIntegration { action } => {
                let reply = Reply::Asker(shell_integration::handle(action));
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
            // ── an ssh profile's material ───────────────────────────
            // The profile itself rides `SetSettings`; only the secret comes this way, and it
            // never goes back — what the interface is told is the flag.
            Message::SetSshSecret { profile_id, secret } => {
                let replies = self.set_ssh_secret(profile_id, secret);
                self.answer(client, replies);
            }
            Message::ClearSshSecret { profile_id } => {
                let replies = self.clear_ssh_secret(profile_id);
                self.answer(client, replies);
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
                let before = self.settings.host();
                let was = before.assist;
                let mut replies = self.settings.set(layer, value);
                // The ssh profiles rode the blob whole, flags and all, and the interface is not
                // the half that knows what is filed. A write that landed is reconciled against
                // the secret store: stale material pruned, every flag re-stamped from what is
                // actually there.
                if layer == SettingsLayer::Host && replies.is_empty() {
                    replies.extend(self.reconcile_ssh_secrets(&before.ssh_profiles));
                }
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

            // ── the host file family ─────────────────────────────────
            // The write half of a guest tab: an absolute path, no project to look up either, so
            // straight to the worker on `browse_job`'s own reasoning.
            Message::WriteHostFile {
                path,
                bytes,
                expected,
            } => {
                self.host_write_job(client, path, bytes, expected);
            }
            // The export half: the same worker, the same atomic write, no version to check.
            Message::SaveHostFileAs { path, bytes } => {
                self.files.submit(files::Job {
                    kind: files::JobKind::HostSaveAs { path, bytes },
                    reply_to: self.host.mailbox(To::Client(client)),
                });
            }

            // ── the file family ─────────────────────────────────────
            // Five arms, no syscall: the record is a lookup in memory and the work goes to the
            // worker with the root it resolved against.
            Message::ProjectTree {
                project_id,
                rel_path,
                depth,
                prefetch,
            } => {
                let request = files::Request::Tree {
                    rel_path: rel_path.clone(),
                    depth,
                    prefetch,
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
                overwrite,
            } => {
                // An annotated markdown file is written under the plans lock and stamped as the
                // user's (`D208`), so an agent's merge cannot land between this write's version
                // check and its write, and the threads and provenance follow the save.
                if let Some((root, target)) = self.annotated_file(project_id, &rel_path) {
                    let outcome = self.plans.lock().file_save(
                        &target, &root, &rel_path, &bytes, expected, overwrite,
                    );
                    match outcome {
                        Ok((version, replies)) => {
                            self.host.send(
                                To::Client(client),
                                Message::ProjectFileWritten {
                                    project_id,
                                    rel_path,
                                    version,
                                },
                            );
                            self.answer(client, replies);
                        }
                        Err(error) => self.host.send(
                            To::Client(client),
                            files::file_error(project_id, &rel_path, error),
                        ),
                    }
                } else {
                    let request = files::Request::Write {
                        rel_path: rel_path.clone(),
                        bytes,
                        expected,
                        overwrite,
                    };
                    self.file_job(client, project_id, &rel_path, request);
                }
            }
            Message::DiffProjectFile {
                project_id,
                rel_path,
                base,
                old,
                new,
            } => {
                let request = files::Request::Diff {
                    rel_path: rel_path.clone(),
                    base,
                    old,
                    new,
                };
                self.file_job(client, project_id, &rel_path, request);
            }
            Message::EditProjectPath {
                project_id,
                rel_path,
                to,
                op,
                carry_related,
            } => {
                let request = files::Request::Edit {
                    rel_path: rel_path.clone(),
                    to,
                    op,
                    carry_related,
                };
                self.file_job(client, project_id, &rel_path, request);
            }
            Message::RelatedProjectFiles {
                project_id,
                rel_path,
            } => {
                let request = files::Request::Related {
                    rel_path: rel_path.clone(),
                };
                self.file_job(client, project_id, &rel_path, request);
            }
            Message::ImportIntoProject {
                project_id,
                into,
                sources,
                mode,
            } => {
                let request = files::Request::Import {
                    into: into.clone(),
                    sources,
                    mode,
                };
                self.file_job(client, project_id, &into, request);
            }

            // ── the knowledge-base family ────────────────────────────
            // A source list is this family's own, not the catalogue's, so unlike the file family
            // there is no project record to look up first — a project ulid with no list written
            // is simply an empty one. Only `KbTree` and `ReadKbFile` reach the worker; the other
            // three settle in memory or on a sync thread and answer directly. The project's own
            // path still has to be found, for `KbOrigin::Git { store: KbStore::Project, .. }`'s
            // sake — a project ulid with no record has no such source to resolve either, so an
            // empty path is exactly as unused as the lookup that produced it.
            Message::KbSources { project_id } => {
                // A protected wiki whose password the keychain holds is opened before the list is
                // drawn, so it reads `Ready` rather than prompting (`D206`).
                self.kb.auto_unlock(project_id);
                let sources = self
                    .kb
                    .sources(project_id, &self.kb_project_path(project_id));
                self.host.send(
                    To::Client(client),
                    Message::KbSourcesListed {
                        project_id,
                        sources,
                    },
                );
            }
            Message::SetKbSources {
                project_id,
                sources,
            } => {
                let asker = self.host.mailbox(To::Client(client));
                let project_path = self.kb_project_path(project_id);
                // Listed again after the auto-unlock below, so a wiki the keychain opens reads
                // `Ready` in this very answer.
                self.kb
                    .set_sources(project_id, sources, asker, &project_path);
                self.kb.auto_unlock(project_id);
                let sources = self.kb.sources(project_id, &project_path);
                self.host.send(
                    To::Client(client),
                    Message::KbSourcesListed {
                        project_id,
                        sources,
                    },
                );
            }
            Message::KbTree {
                project_id,
                source,
                rel_path,
                depth,
            } => {
                let request = files::Request::Tree {
                    rel_path: rel_path.clone(),
                    depth,
                    prefetch: false,
                };
                self.kb_job(client, project_id, source, &rel_path, request);
            }
            Message::ReadKbFile {
                project_id,
                source,
                rel_path,
                max_bytes,
            } => {
                let request = files::Request::Read {
                    rel_path: rel_path.clone(),
                    max_bytes,
                };
                self.kb_job(client, project_id, source, &rel_path, request);
            }
            Message::SyncKbSource { project_id, source } => {
                let asker = self.host.mailbox(To::Client(client));
                let project_path = self.kb_project_path(project_id);
                self.kb.sync(project_id, source, asker, &project_path);
            }
            // A protected wiki's password (`D206`). Answered inline: one Argon2 derivation for an
            // unlock, and a re-seal of every document for a change — a wiki of hand-written pages,
            // not a tree worth a thread. The password is never logged; it leaves its `Secret` only
            // to be stretched into a key and, if asked, filed in the keychain.
            Message::UnlockKbSource {
                project_id,
                source,
                password,
                remember,
            } => {
                let result = self
                    .kb
                    .unlock(project_id, source, password.expose(), remember);
                self.kb_password_answer(client, project_id, source, result);
            }
            Message::ChangeKbPassword {
                project_id,
                source,
                old,
                new,
                remember,
            } => {
                let result = self.kb.change_password(
                    project_id,
                    source,
                    old.expose(),
                    new.expose(),
                    remember,
                );
                self.kb_password_answer(client, project_id, source, result);
            }
            // The six below are writes, a rename or a lookup against one already-resolved
            // source — cheap enough, and rare enough, to answer inline rather than through
            // `Files`: unlike `ProjectFileContents`, a knowledge-base document is markdown or a
            // diagram, never the multi-megabyte read `Files` exists to keep off this thread.
            Message::WriteKbFile {
                project_id,
                source,
                rel_path,
                contents,
            } => match self.kb_resolve(project_id, source) {
                Ok((found, base, key)) => {
                    match kb::ops::write_file(&base, &found, &rel_path, &contents, key.as_deref()) {
                        Ok(()) => self.kb_changed(client, project_id, source, &rel_path),
                        Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
                    }
                }
                Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
            },
            Message::CreateKbEntry {
                project_id,
                source,
                rel_path,
                kind,
            } => match self.kb_resolve(project_id, source) {
                Ok((found, base, key)) => {
                    match kb::ops::create(&base, &found, &rel_path, kind, key.as_deref()) {
                        Ok(()) => self.kb_changed(client, project_id, source, &rel_path),
                        Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
                    }
                }
                Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
            },
            Message::RenameKbEntry {
                project_id,
                source,
                rel_path,
                new_name,
            } => match self.kb_resolve(project_id, source) {
                Ok((found, base, key)) => {
                    match kb::ops::rename(&base, &found, &rel_path, &new_name, key.as_deref()) {
                        Ok(_new_rel) => self.kb_changed(client, project_id, source, &rel_path),
                        Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
                    }
                }
                Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
            },
            Message::DeleteKbEntry {
                project_id,
                source,
                rel_path,
            } => match self.kb_resolve(project_id, source) {
                Ok((found, base, key)) => {
                    match kb::ops::delete(&base, &found, &rel_path, key.as_deref()) {
                        Ok(()) => self.kb_changed(client, project_id, source, &rel_path),
                        Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
                    }
                }
                Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
            },
            Message::RevealKbPath {
                project_id,
                source,
                rel_path,
            } => match self.kb_resolve(project_id, source) {
                Ok((found, base, _key)) => match kb::ops::absolute(&base, &found, &rel_path) {
                    Ok(path) => {
                        if let Err(error) = kb::ops::reveal(&path) {
                            self.kb_failed(
                                client,
                                project_id,
                                source,
                                &rel_path,
                                FileError::Failed(error.to_string()),
                            );
                        }
                    }
                    Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
                },
                Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
            },
            Message::AskKbPath {
                project_id,
                source,
                rel_path,
            } => match self.kb_resolve(project_id, source) {
                Ok((found, base, _key)) => match kb::ops::absolute(&base, &found, &rel_path) {
                    Ok(path) => self.host.send(
                        To::Client(client),
                        Message::KbPath {
                            project_id,
                            source,
                            rel_path: rel_path.clone(),
                            path: path.to_string_lossy().into_owned(),
                        },
                    ),
                    Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
                },
                Err(error) => self.kb_failed(client, project_id, source, &rel_path, error),
            },

            // ── the git family ──────────────────────────────────────
            // Two arms, no syscall: the record is a lookup in memory and the work goes to the
            // worker with the root it resolved against. A status walk on this thread would stall
            // every pane behind it.
            Message::ProjectGit { project_id, repo } => {
                self.git_job(client, project_id, repo, git::Request::Overview);
            }
            Message::RefreshProjectGit { project_id, full } => {
                let request = if full {
                    git::Request::Full
                } else {
                    git::Request::Overview
                };
                self.git_job(client, project_id, String::new(), request);
            }
            Message::ProjectGitRefs {
                project_id,
                repo,
                with_tracking,
            } => {
                self.git_job(
                    client,
                    project_id,
                    repo,
                    git::Request::Refs { with_tracking },
                );
            }
            Message::ProjectGitLog {
                project_id,
                repo,
                cursor,
                count,
                rel_path,
                first_parent,
                rev,
            } => {
                self.git_job(
                    client,
                    project_id,
                    repo,
                    git::Request::Log {
                        cursor,
                        count,
                        rel_path,
                        first_parent,
                        rev,
                    },
                );
            }
            Message::WriteProjectGit {
                project_id,
                repo,
                op,
            } => {
                self.git_job(client, project_id, repo, git::Request::Write { op });
            }
            Message::ProjectGitChanged {
                project_id,
                repo,
                from,
                to,
            } => {
                self.git_job(client, project_id, repo, git::Request::Changed { from, to });
            }

            // ── the work family ─────────────────────────────────────
            // Sixteen arms and one helper. Every one names a project, and none of them touches a
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
            } => {
                self.work_job(client, project_id, |work| {
                    work.update(project_id, task_id, title, description, priority)
                });
            }
            Message::SetTaskField {
                project_id,
                task_id,
                field,
            } => {
                // A mission is its anchor task, so the level control is where one comes into being
                // and where one is thrown away (M3). Demoting a mission that has been worked is
                // refused here, before anything is written: `Work` knows nothing about missions
                // and the refusal has to happen above it.
                let known = self.projects.record(project_id).is_some();
                let demoted = matches!(field, TaskField::Level(None));
                let promoted = matches!(field, TaskField::Level(Some(_)));
                if demoted
                    && known
                    && let Some(refusal) =
                        self.missions.lock().demotion_refusal(project_id, task_id)
                {
                    self.host.send(
                        To::Client(client),
                        Message::MissionError {
                            project_id,
                            task_id: Some(task_id),
                            error: refusal,
                        },
                    );
                    return;
                }
                self.work_job(client, project_id, |work| {
                    work.set_field(project_id, task_id, field)
                });
                if known && (demoted || promoted) {
                    let replies = if demoted {
                        self.missions.lock().delete(project_id, task_id)
                    } else {
                        self.missions.lock().create(project_id, task_id)
                    };
                    self.answer(client, replies);
                }
            }
            Message::MoveTask {
                project_id,
                task_id,
                status,
                before,
            } => {
                // **A mission card's column is its phase** (M4). Dragging one to another column is
                // a phase move under M5's rules — gate and all — not a raw status write, and the
                // host is where that rule lives whatever the window sends. A drag that lands in
                // the column the mission's phase already means is a reorder and nothing else, so
                // it falls through to `move_task` and keeps its `before`.
                let phase = ubiq_proto::mission::Phase::for_status(status);
                let mission = self.projects.record(project_id).is_some()
                    && self.missions.lock().is_mission(project_id, task_id);
                if mission
                    && self
                        .missions
                        .lock()
                        .record(project_id, task_id)
                        .is_none_or(|record| record.phase != phase)
                {
                    let replies = self.missions.lock().set_phase(project_id, task_id, phase);
                    self.answer(client, replies);
                    return;
                }
                self.work_job(client, project_id, |work| {
                    work.move_task(project_id, task_id, status, before)
                });
                self.wake_scheduler(project_id, task_id);
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
                // A deleted task must not leave its plan orphaned on disk. Guarded the same way
                // `work_job` guards every other plan-family arm below: a project the catalogue
                // does not hold gets nothing written under it, plan included.
                if self.projects.record(project_id).is_some() {
                    let replies = self
                        .plans
                        .lock()
                        .delete(&crate::plan::Target::plan(project_id, task_id));
                    self.answer(client, replies);
                    // The mission's directory rides the deletion the same way, for the same
                    // reason: a record keyed by a task that is gone can never be reached again.
                    let replies = self.missions.lock().delete(project_id, task_id);
                    self.answer(client, replies);
                }
            }
            Message::ArchiveTasks { project_id } => {
                self.work_job(client, project_id, |work| work.archive(project_id));
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
            Message::AddComment {
                project_id,
                task_id,
                text,
            } => {
                self.work_job(client, project_id, |work| {
                    work.add_comment(
                        project_id,
                        task_id,
                        ubiq_proto::work::CommentAuthor::User,
                        text,
                    )
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
                // The assignment is what membership is read from, so the roster and the agent's
                // own facts are settled straight after it — an agent moved onto a mission's child
                // task is in that mission from its next tool call (M11).
                let was = self
                    .agents
                    .mcp_agents()
                    .facts(&agent_id.to_string())
                    .and_then(|facts| facts.mission);
                let now = self.settle_mission(project_id, agent_id);
                if was != now {
                    if let Some(was) = was {
                        let replies = self.missions.lock().leave(
                            project_id,
                            was,
                            agent_id,
                            ubiq_proto::mission::Actor::User,
                        );
                        for reply in replies {
                            self.host.send(To::Everyone, reply.into_message());
                        }
                    }
                    self.agents
                        .mcp_agents()
                        .set_mission(&agent_id.to_string(), now);
                }
                // **A hand assignment removes that task from the scheduler's pool** (M22), and
                // taking a task off an agent puts it back. Either way the pool changed.
                if let Some(mission) = now {
                    self.wake_scheduler(project_id, mission);
                }
            }
            Message::SendToAgent {
                project_id,
                agent_id,
                text,
            } => {
                // **The user talking to a mission's coordinator is the mission's feedback** (M12).
                // Journaled here, where it already happens, rather than through a second path: the
                // panel's *Feedback* line is this message, and `read_feedback` reads these lines.
                let mission = self
                    .agents
                    .mcp_agents()
                    .facts(&agent_id.to_string())
                    .and_then(|facts| facts.mission)
                    .filter(|mission| {
                        self.missions
                            .lock()
                            .record(project_id, *mission)
                            .is_some_and(|record| record.coordinator == Some(agent_id))
                    });
                if let Some(mission) = mission {
                    let replies = self
                        .missions
                        .lock()
                        .feedback(project_id, mission, text.clone());
                    for reply in replies {
                        self.host.send(To::Everyone, reply.into_message());
                    }
                }
                self.work_job(client, project_id, |work| {
                    work.send_to_agent(project_id, agent_id, text)
                });
            }
            // Named boards (`T-360`): the registry, and the envelope a board's task edits travel in.
            Message::OnBoard { board, message } => self.board_job(client, board, *message),
            Message::ListBoards { project_id } => {
                self.work_job(client, project_id, |work| work.list_boards(project_id));
            }
            Message::CreateBoard { project_id, name } => {
                self.work_job(client, project_id, |work| {
                    work.create_board(project_id, name)
                });
            }
            Message::SetBoardEnabled {
                project_id,
                board,
                enabled,
            } => {
                self.work_job(client, project_id, |work| {
                    work.set_board_enabled(project_id, &board, enabled)
                });
            }
            Message::DeleteBoard { project_id, board } => {
                self.work_job(client, project_id, |work| {
                    work.delete_board(project_id, &board)
                });
            }

            // ── the annotated-document family ───────────────────────
            // Every arm here names a `DocumentHandle` and nothing else. What this decides is only
            // whether the project exists and where the document's body is — `plan_job` resolves
            // the handle to a `plan::Target` once, at the edge, and the level check for a plan
            // stays inside `crate::plan::Plans` on `work_job`'s own footing.
            Message::LoadPlan { doc } => {
                self.plan_job(client, &doc, |plans, target| plans.load(target));
            }
            Message::SavePlan {
                doc,
                body,
                expected,
            } => {
                // `SavePlan` is the interface's message and nothing else sends it, so the origin
                // is settled here and never looked at again. An agent's save arrives through
                // `ubiq-plan`'s `write_plan` and never becomes this message.
                //
                // `expected` is mandatory on the wire and is passed straight through: the window
                // never gets to skip the check, so a save made against a watermark somebody has
                // moved past is refused here with `PlanConflict` rather than landing on top of a
                // copy its author never read.
                self.plan_job(client, &doc, |plans, target| {
                    plans.save(target, body, &crate::plan::Saver::human(), Some(expected))
                });
            }
            Message::DeletePlan { doc } => {
                self.plan_job(client, &doc, |plans, target| plans.delete(target));
            }
            Message::ExportPlan { doc, rel_path } => {
                self.export_plan(client, &doc, rel_path);
            }
            Message::ListPlanAnnotations { doc, open } => {
                self.plan_job(client, &doc, |plans, target| {
                    if open {
                        plans.reopen_lineage(target);
                    }
                    plans.annotations(target)
                });
            }
            // The author is stamped from the path the message arrived on and never read off the
            // wire — `D121`'s rule for a task's comments, and a window is a user by construction.
            Message::AnnotatePlan {
                doc,
                block_id,
                quote,
                text,
                marks,
                to,
            } => {
                let mut thread = None;
                self.plan_job(client, &doc, |plans, target| {
                    let replies = plans.annotate(
                        target,
                        block_id,
                        quote,
                        ubiq_proto::work::CommentAuthor::User,
                        text,
                        marks,
                        to,
                    );
                    thread = crate::plan::queued_from(&replies, None);
                    replies
                });
                self.deliver_to_agent(&doc, thread, to);
            }
            // A window's disk merge kept the user's words over another writer's (`D208`): the
            // other side's passages become threads, checked against the disk and authored by
            // whoever provenance says moved it — `Plans::disk_conflicts`.
            Message::DocMergeConflicts { doc, conflicts } => {
                self.plan_job(client, &doc, |plans, target| {
                    plans.disk_conflicts(target, conflicts)
                });
            }
            Message::ReplyToAnnotation {
                doc,
                annotation_id,
                text,
                to,
            } => {
                let mut thread = None;
                self.plan_job(client, &doc, |plans, target| {
                    let replies = plans.reply_to(
                        target,
                        annotation_id,
                        ubiq_proto::work::CommentAuthor::User,
                        text,
                        to,
                    );
                    thread = crate::plan::queued_from(&replies, Some(annotation_id));
                    replies
                });
                self.deliver_to_agent(&doc, thread, to);
            }
            Message::MarkAnnotation {
                doc,
                annotation_id,
                mark,
                on,
            } => {
                self.plan_job(client, &doc, |plans, target| {
                    plans.set_mark(target, annotation_id, mark, on)
                });
            }
            Message::SetBlockHighlight {
                doc,
                block_ids,
                colour,
            } => {
                self.plan_job(client, &doc, |plans, target| {
                    plans.set_highlights(target, &block_ids, colour)
                });
            }
            Message::ResolveAnnotation {
                doc,
                annotation_id,
                resolved,
            } => {
                self.plan_job(client, &doc, |plans, target| {
                    plans.resolve(target, annotation_id, resolved)
                });
            }
            Message::ListPlanChanges {
                doc,
                since_revision,
            } => {
                self.plan_job(client, &doc, |plans, target| {
                    plans.changes(target, since_revision)
                });
            }
            Message::EditAnnotationComment {
                doc,
                annotation_id,
                comment_id,
                text,
            } => {
                self.plan_job(client, &doc, |plans, target| {
                    plans.edit_comment(target, annotation_id, comment_id, text)
                });
            }
            // ── the doc binding (`D207`) ──
            Message::SetDocAgent { doc, agent_id } => {
                let mut bound = false;
                self.plan_job(client, &doc, |plans, target| {
                    let replies = plans.set_agent(target, agent_id);
                    bound = !failed(&replies);
                    replies
                });
                if bound {
                    // A thread queued for the old agent is no longer its to answer.
                    for agent in self.doc_queue.drop_doc(&doc, agent_id) {
                        self.deliver_doc_queue(agent, doc.project_id());
                    }
                }
            }
            Message::SetDocAutoSend { doc, auto_send } => {
                self.plan_job(client, &doc, |plans, target| {
                    plans.set_auto_send(target, auto_send)
                });
            }
            Message::AskDocAgent {
                doc,
                annotation_ids,
            } => {
                let mut found = None;
                self.plan_job(client, &doc, |plans, target| {
                    match plans.awaiting_threads(target, &annotation_ids) {
                        Ok((binding, _)) if binding.agent.is_none() => vec![Reply::Asker(
                            crate::plan::doc_error(target, "no agent is bound to this document"),
                        )],
                        Ok((binding, threads)) => {
                            found = binding.agent.map(|agent| (agent, threads));
                            Vec::new()
                        }
                        Err(error) => vec![Reply::Asker(crate::plan::doc_error(target, error))],
                    }
                });
                // Nothing waiting on the agent is nothing to say: no message, no empty report.
                if let Some((agent, threads)) = found.filter(|(_, threads)| !threads.is_empty()) {
                    for thread in threads {
                        self.doc_queue.push(agent, thread);
                    }
                    self.deliver_doc_queue(agent, doc.project_id());
                }
            }

            Message::ListMissions { project_id } => {
                self.mission_job(client, project_id, |missions| missions.list(project_id));
            }
            Message::CreateMission {
                project_id,
                task_id,
            } => {
                self.mission_job(client, project_id, |missions| {
                    missions.create(project_id, task_id)
                });
            }
            Message::SetMissionField {
                project_id,
                task_id,
                field,
            } => {
                self.mission_job(client, project_id, |missions| {
                    missions.set_field(project_id, task_id, field)
                });
                // The mode and the parallelism are two of the four things the scheduler runs on,
                // and switching into auto is the one that has to act immediately (M22).
                self.wake_scheduler(project_id, task_id);
            }
            Message::RequestPhase {
                project_id,
                task_id,
                phase,
                summary,
            } => {
                self.mission_job(client, project_id, |missions| {
                    missions.request_phase(project_id, task_id, phase, summary)
                });
            }
            Message::SetPhase {
                project_id,
                task_id,
                phase,
            } => {
                self.mission_job(client, project_id, |missions| {
                    missions.set_phase(project_id, task_id, phase)
                });
                // **Auto only runs in the In-progress phase** (M22). Entering it is what starts
                // the scheduler; leaving it pauses it, and the pass this makes is the no-op that
                // proves it.
                self.wake_scheduler(project_id, task_id);
            }
            Message::LoadJournal {
                project_id,
                task_id,
                before,
                limit,
            } => {
                self.mission_job(client, project_id, |missions| {
                    missions.load_journal(project_id, task_id, before, limit)
                });
            }
            // The window has launched, or refused to launch, an agent a member asked for (M13).
            // The host only relayed the request, so this is where the outcome becomes a fact: the
            // roster, the journal, and the requester's next prompt.
            Message::AnswerSpawn {
                project_id,
                task_id,
                request_id,
                outcome,
            } => {
                self.mission_job(client, project_id, |missions| {
                    missions.answer_spawn(project_id, task_id, request_id, outcome)
                });
                // A declined auto spawn gives its slot straight back, and an allowed one may have
                // freed another: either way the next pass is due now rather than at whatever
                // happens next.
                self.wake_scheduler(project_id, task_id);
            }
            Message::MissionSchedule {
                project_id,
                task_id,
            } => {
                self.wake_scheduler(project_id, task_id);
            }

            Message::StartConversation {
                agent_id,
                project_id,
                session_id,
                rel_path,
                agent_type,
                account,
                definition,
                model,
                thinking,
                mode,
                mcps,
                skills,
                spawned_by,
                grants,
            } => {
                self.start_conversation(
                    client,
                    agent_id,
                    project_id,
                    session_id,
                    rel_path,
                    agent_type,
                    account,
                    definition,
                    model,
                    thinking,
                    mode,
                    mcps,
                    skills,
                    spawned_by,
                    rw_paths(grants),
                );
            }
            Message::PromptAgent { agent_id, text } => {
                // **A prompt may come from the host's own voice**, not only from a window: a
                // mission line to a roster member, or the outcome of a spawn reaching the agent
                // that asked for it (`crate::mission::Missions::tell_agent`). The voice owns no
                // conversation, so it borrows the owning window's id for the ownership check —
                // the same re-addressing the `AskUser` arm does in the other direction (`D138`).
                //
                // **And it only ever drives a conversation that is already live.** A voice prompt
                // is a line about the mission, never a reason to start a harness the user has not
                // started: for an agent that has not launched, the line is already in its thread
                // and that is the whole of the delivery.
                let voiced = client.is_host_voice();
                // **The user typed instead of answering.** A registered dialog *is* the question
                // this turn ended on, and answering it is what opens the next one — so a prompt
                // from the window closes whatever is on screen for it, and there is no path where
                // the same turn is both answered and spoken to (`D175`). Only a window's own
                // prompt: the host's voice is a mission line, not the user.
                if !voiced {
                    self.close_armed_dialogs(agent_id);
                }
                let client = if voiced {
                    let Some((owner, _)) = self.conversation_owners.get(&agent_id) else {
                        return;
                    };
                    *owner
                } else {
                    client
                };
                // The asked half of the opening exchange, kept before the prompt is handed on
                // and moved. Only the first one: a conversation is named after what it was
                // started for, and a later turn is about where the work has got to.
                if !voiced
                    && let Some(pending) = self.pending_conversations.get_mut(&agent_id)
                    && !pending.named
                    && pending.opening_prompt.is_none()
                {
                    pending.opening_prompt = Some(text.clone());
                }
                if self.conversations.contains_key(&agent_id) {
                    self.drive(client, agent_id, |conversation| conversation.prompt(text));
                } else if !voiced {
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
            // ── the ask family ──────────────────────────────────────
            // Raised by the `ubiq-ask` tool through the host's own voice, not by a window: the
            // coordinator is the only half that knows which window owns the conversation, so it
            // does the addressing and the tool call knows nothing about clients (`D138`).
            Message::AskUser {
                agent_id,
                ask_id,
                questions,
                batch_at,
                batch_of,
            } => {
                match self.conversation_owners.get(&agent_id) {
                    Some((owner, _)) => self.host.send(
                        To::Client(*owner),
                        Message::AskUser {
                            agent_id,
                            ask_id,
                            questions,
                            batch_at,
                            batch_of,
                        },
                    ),
                    // Nobody to ask. Ended here rather than left to the timeout: a tool call that
                    // waits an hour for a window that does not exist is a wedged agent.
                    None => {
                        tracing::debug!(
                            agent = %agent_id,
                            ask = %ask_id,
                            "an ask was raised by a conversation nobody owns",
                        );
                        self.asks.end(ask_id, AskClosed::Gone);
                        // The same for the other mode, where there is no call to end and only a
                        // row to forget. Whatever that does to the row's batch is moot: there is
                        // no window to have answered the rest and no conversation to prompt.
                        let _ = self.armed.close(ask_id);
                    }
                }
            }
            Message::AnswerAsk {
                agent_id,
                ask_id,
                outcome,
            } => {
                // The same owner gate every conversation-family message passes, then whichever
                // table is holding this ask. An answer naming an ask nobody is holding — one that
                // timed out while the dialog was still open, or a second answer — is dropped
                // quietly by both.
                if !self.drives(client, agent_id) {
                    return;
                }
                // The two modes differ only here. A parked call is released with the outcome and
                // reads it as its own tool result; a registered dialog has no call left to
                // release, so the outcome becomes the next turn instead (`D175`). `Chat` sends
                // nothing either way: the user would rather type, and that is what they do next.
                //
                // **The armed table is asked first, and that is what makes a late answer land.**
                // A parked call that gave up on its bound hands its row over to the armed table
                // under the same id (`D191`), so an answer arriving after the tool result has
                // gone finds it here rather than falling through to a call nobody is holding.
                //
                // **And an armed answer may not be the whole of what is waiting.** Everything one
                // turn registered is raised together, so answering the first of three settles a
                // row and submits nothing — the prompt is opened once, by whichever answer
                // completes the set, and carries all of them (`G362`). A set nobody said anything
                // to opens no turn at all.
                match self.armed.answer(ask_id, &outcome) {
                    crate::armed::Answering::Settled(settled) => {
                        if let Some(text) = crate::armed::prose(&settled) {
                            self.drive(client, agent_id, |conversation| conversation.prompt(text));
                        }
                    }
                    crate::armed::Answering::Waiting { .. } => {}
                    crate::armed::Answering::NotOurs => {
                        self.asks.answer(ask_id, outcome);
                    }
                }
            }
            // The one ask message that travels both ways, so the sender is what tells them apart.
            // The owning window is the only client that says it: it holds no conversation to draw
            // the ask in, and the parked call is freed here rather than left to the timeout.
            // Anything else on this arm came from the `ubiq-ask` thread's own voice — a wait that
            // gave up — and the host's voice owns no conversation, so the two never read as each
            // other. That one is addressed exactly as `AskUser` above is.
            Message::AskEnded {
                agent_id,
                ask_id,
                why,
            } => match self.conversation_owners.get(&agent_id).copied() {
                Some((owner, _)) if owner == client => {
                    // An id the host is no longer holding — one that timed out first — is
                    // dropped quietly by the table. Both tables: a window that cannot draw a
                    // registered dialog leaves nothing to answer either.
                    //
                    // **Giving one dialog up settles its row rather than forgetting it**, so a
                    // set raised together is not wedged by the one the user dismissed: it
                    // contributes nothing, and if it was the last one still open the rest is
                    // submitted here (`G362`).
                    self.asks.end(ask_id, why);
                    if let crate::armed::Answering::Settled(settled) = self.armed.close(ask_id)
                        && let Some(text) = crate::armed::prose(&settled)
                    {
                        self.drive(client, agent_id, |conversation| conversation.prompt(text));
                    }
                }
                Some((owner, _)) => self.host.send(
                    To::Client(owner),
                    Message::AskEnded {
                        agent_id,
                        ask_id,
                        why,
                    },
                ),
                // No owner: nobody to tell, and nothing the table is still holding for it — the
                // `AskUser` arm above ended it as `Gone` when it was raised.
                None => {}
            },

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
            Message::RenameConversation { agent_id, name } => {
                self.rename_conversation(client, agent_id, name);
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

            // ── the database family ─────────────────────────────────
            // Every driver call blocks, so none is made here: `Db` queues each on a session thread
            // of its own and answers through the mailboxes handed over with it.
            message @ (Message::DbConnections { .. }
            | Message::SaveDbConnection { .. }
            | Message::DeleteDbConnection { .. }
            | Message::TestDbConnection { .. }
            | Message::DbPassword { .. }
            | Message::DbTree { .. }
            | Message::DbExportDbml { .. }
            | Message::DbTablePage { .. }
            | Message::DbQuery { .. }
            | Message::DbApplyEdits { .. }
            | Message::DbCancel { .. }
            | Message::DbCloseSession { .. }
            | Message::DbDisconnect { .. }
            | Message::DbEditors { .. }
            | Message::DbEditorEdit { .. }
            | Message::CreateDbFile { .. }) => {
                let project_path = message
                    .project_id()
                    .map(|project_id| self.kb_project_path(project_id))
                    .unwrap_or_default();
                self.db.handle(
                    crate::db::Ctx {
                        client,
                        asker: self.host.mailbox(To::Client(client)),
                        everyone: self.host.mailbox(To::Everyone),
                        project_path,
                    },
                    message,
                );
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
        definition: Option<String>,
        model: Option<String>,
        thinking: Option<String>,
        mode: Option<String>,
        mcps: Vec<String>,
        skills: Vec<String>,
        spawned_by: Option<AgentId>,
        extra_rw: Vec<String>,
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
        let name = unique_name(&base, &self.taken_names(project_id));

        let agent = WorkAgent {
            id: agent_id,
            session: session_id,
            task: None,
            // The **only** production writer of `parent`: an agent the window launched to answer
            // a `MissionSpawnRequest` names the agent that asked for it. Every other start has
            // nobody above it, and `Work::assign_agent` clears this on every reassignment — which
            // is why a mission's roster is written down at the spawn rather than read back off it.
            parent: spawned_by,
            name,
            // Nothing has named it yet; a revive restores the row's title right after this.
            title: None,
            definition: definition.clone(),
            summary: None,
            activity: Activity::Thinking,
            branch: String::new(),
            tokens: 0.0,
            harness: label,
            // No run has happened yet to say which identity actually answered, so this reports
            // what was *asked* for rather than what compose_run resolves — a known, accepted gap
            // while there is no definition UI to make the two differ; see the doc's Traps.
            account: account.clone().unwrap_or_default(),
            // Empty until the harness says which model answered — it is the only thing that
            // knows, and guessing would put a wrong name under a real conversation.
            model: String::new(),
            context_pct: 0,
            persistent: false,
            accept_all: false,
            debug_dump: None,
            // Known already, and without composing anything: a run's directory is named after the
            // agent's own id, which was minted before this message was sent. The configuration
            // directory is the library's answer and there is no run yet to ask — `launch_pending`
            // reports it once one has been composed.
            run_dir: Some(self.agents.agent_dir(agent_id).display().to_string()),
            config_dir: None,
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
            .lock()
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
        // A definition's model and mode are seeded into the picks rather than left to `resolve`:
        // `launch_picks` fills `flags.model` from the catalogue's default when `chosen_model`
        // is empty, and a flag outranks the definition — so a definition left unseeded would be
        // shown wrong by the picker *and* launched over. One read, both problems.
        let record = definition.as_deref().and_then(|name| {
            self.agents
                .definitions()
                .unwrap_or_default()
                .into_iter()
                .find(|record| record.id == name)
        });
        // An explicit pick outranks the definition's, the way a pick always does: a flag was the
        // user overriding what the definition set, and the definition's own record is only the
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
                definition,
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
                harness_title: None,
                // Nothing has run, so there is no harness session to continue.
                resume: None,
                fork: false,
                mcps,
                skills,
                extra_rw,
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
                // definition record) outranks the remembered last-used level, the same way
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
    /// the next relaunch, and is only ever removed by [`Self::end_conversation`]; a launch
    /// failure leaves it.
    ///
    /// Returns whether the launch succeeded. A failure retracts nothing: the `WorkAgent`, its
    /// owner, the recipe and the MCP row all stay, so the agent shows as failed and a Resume or a
    /// prompt retries it, while Close ends it the ordinary way (`end_conversation`).
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
        // server is call it. A row here for a launch that then fails stays, like everything else, for a
        // retry or for the close that retires it.
        let (name, harness) = self
            .work
            .lock()
            .live_agent_mut(pending.project_id, agent_id)
            .map(|agent| (agent.title_or_handle().to_string(), agent.harness.clone()))
            .unwrap_or_else(|| (pending.agent_type.clone(), pending.agent_type.clone()));
        let project = self.project_facts(pending.project_id);
        // Which mission this run is in, settled before the harness exists to ask — the mission
        // servers resolve it from the URL identity alone and there is no later moment to fill it
        // for the first call (M11).
        let mission = self.settle_mission(pending.project_id, agent_id);
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
            mission,
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
                definition: pending.definition.clone(),
                // Which definitions this run can even see: the project's own, over the global
                // ones. A conversation always belongs to a project, so this is never absent.
                project: Some(pending.project_id),
                prompt: first_prompt,
                resume: pending.resume.clone(),
                fork: pending.fork,
                mcps: pending.mcps.clone(),
                skills: pending.skills.clone(),
                extra_rw: pending.extra_rw.clone(),
            },
        ) {
            Ok(started) => started,
            Err(error) => {
                tracing::error!(
                    agent = %agent_id,
                    harness = %pending.agent_type,
                    "starting failed: {error:#}"
                );
                // **Put back, never forgotten** — `reap_conversations`' rule (`T-296`) applied one
                // step earlier (`T-327`). The window already holds this conversation, so dropping
                // the owner and the pending row here left Close, Resume and the next prompt all
                // hitting `drives` and vanishing: an errored agent nothing could unload or retry.
                // Kept pending, a Resume or a prompt retries the launch (with the harness's own
                // session id when there is one), and Close ends it the ordinary way. The MCP row
                // registered above stays too: `wake_agent_scheduler` finds the mission through it
                // when a failed mission worker is closed, a retry's `register` replaces it, and
                // `retire_agent`/`park_agent` forget it on close. The run directory stays as well,
                // because a resume's session store lives in it.
                self.close_asks(agent_id, AskClosed::Gone);
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
        // A stale reference in the definition this run resolved does not stop it launching (see
        // `Composed::problems`) — it still has to reach the person who set the definition up, as a
        // dismissable entry in the bell rather than nothing at all.
        self.report_run_problems(client, &pending.agent_type, &composed.problems);

        // The one directory the library actually provisioned this run's configuration into, said
        // back to the window now that there is a run to say it about. Ubiq pins that directory to
        // the run's own (`Agents::compose_run`), so the two paths agree today — they are reported
        // separately because it is the library that decides the second one, and a day when it
        // decides differently must not be a day the window starts opening the wrong folder.
        let config_dir = composed.dir.display().to_string();
        self.publish_conversation_flags(agent_id, |agent| {
            agent.config_dir = Some(config_dir);
        });

        // An ACP agent states what it can do in the `initialize` it has just answered, and there
        // is no second way to ask: no probe, no flag, nothing to shell out to. So the handshake is
        // the discovery, and this is the one place it is recorded — keyed on the harness, because
        // every session behind the same harness hears the same answer. Both surfaces that show it
        // read the record rather than a live bridge, which is what lets them show it with nothing
        // running. `None` for every harness on a wire of its own.
        if let Some(capabilities) = bridge.acp_capabilities() {
            self.catalogue.set_acp_capabilities(
                &pending.agent_type,
                now_ms(),
                crate::agent::acp_capabilities(capabilities),
            );
        }

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
        // Where a reading this harness pushes mid-turn is filed. Only for a run with an identity
        // behind it: what is left belongs to an account, and a run as the harness's own default
        // identity has none to key one by.
        let quota = pending
            .account
            .clone()
            .filter(|account| !account.is_empty())
            .map(|account| crate::conversation::QuotaVoice {
                voice: self.host.voice(),
                harness: pending.agent_type.clone(),
                account,
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
            quota,
            quiet,
            flags,
            // Where a dialog this agent registers mid-turn is raised from: the pump sees the turn
            // end, and the coordinator is what addresses the question it says (`D175`).
            Some(crate::conversation::AskFire {
                armed: self.armed.clone(),
                voice: self.host.voice(),
            }),
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
        let changed = self
            .work
            .lock()
            .live_agent_mut(project_id, agent_id)
            .map(|agent| {
                agent.persistent = persistent;
                Box::new(agent.clone())
            });
        if let Some(agent) = changed {
            self.host
                .send(To::Everyone, Message::AgentChanged { project_id, agent });
        }
    }

    /// Give a conversation a new name, typed by the user over whatever it was called.
    ///
    /// Written to the durable row's `title` — the same field [`Self::name_job`] writes from its
    /// own idea of one — and to the live `WorkAgent::title` through
    /// [`Self::publish_conversation_flags`], the same broadcast [`Self::set_conversation_persistent`]
    /// and its two siblings use. One field, so a rename shows up on every surface that reads a
    /// conversation's name rather than only the one it was typed into, and the MCP identity with it.
    ///
    /// **Counts as named**, the same flag [`Self::name_conversations`] checks before ever running
    /// the naming pass: a title the user just typed is not overwritten the moment the opening
    /// reply lands.
    fn rename_conversation(&mut self, client: ClientId, agent_id: AgentId, name: String) {
        if !self.drives(client, agent_id) {
            return;
        }
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let sessions = self.sessions();
        if let Some(mut row) = conversation_record::load(&sessions, agent_id) {
            row.title = Some(name.clone());
            conversation_record::save(&sessions, agent_id, &row);
        }
        if let Some(pending) = self.pending_conversations.get_mut(&agent_id) {
            pending.named = true;
            pending.opening_prompt = None;
        }
        // The title, never the handle: `name` stays the mechanical one it was minted with.
        self.publish_conversation_flags(agent_id, |agent| agent.title = Some(name));
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
        publish_live_agent(
            &self.work,
            &self.agents.mcp_agents(),
            &self.host.mailbox(To::Everyone),
            project_id,
            agent_id,
            set,
        );
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
                    pending.definition.clone(),
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
                        row.definition,
                        row.model,
                        row.thinking,
                        row.mode,
                    )
                })
            });
        let Some((agent_type, cwd, account, definition, model, thinking, mode)) = facts else {
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
            definition,
            model,
            thinking,
            mode,
            Vec::new(),
            Vec::new(),
            // A relaunch from a stored recipe, not a spawn: who once asked for this agent is not
            // on the row, and re-stamping a parent a reassignment has since cleared would invent
            // a link nobody made.
            None,
            Vec::new(),
        );
        // `start_conversation` refuses on its own terms — an unknown harness, an unreadable folder
        // — and says so; there is nothing here to add if it did.
        if !self.pending_conversations.contains_key(&agent_id) {
            return;
        }

        // What it was called, back onto the live record (T-283's B3): a fresh start mints only
        // the handle, and the row's title is what the conversation was named before. A title on
        // the row also means it has been named, so the naming pass does not run a second time.
        // A fork copies both onto its own row, which `start_conversation` has just written blank.
        let label = conversation_record::load(&self.sessions(), source)
            .map(|row| (row.title, row.summary))
            .filter(|(title, _)| title.is_some());
        if let Some((title, summary)) = label {
            if let Some(pending) = self.pending_conversations.get_mut(&agent_id) {
                pending.named = true;
            }
            if forking {
                let sessions = self.sessions();
                if let Some(mut row) = conversation_record::load(&sessions, agent_id) {
                    row.title = title.clone();
                    row.summary = summary.clone();
                    conversation_record::save(&sessions, agent_id, &row);
                }
            }
            self.publish_conversation_flags(agent_id, |agent| {
                agent.title = title;
                agent.summary = summary;
            });
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
            // A harness whose conversations live in a shared home (Codex on an account home) has
            // to be told: the copied directory isolates nothing there, and resuming the source's
            // id would put both conversations on one thread. A harness whose store *is* the
            // directory ignores it.
            pending.fork = forking;
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
        self.forget_doc_delivery(agent_id);
        let Some(conversation) = self.conversations.remove(&agent_id) else {
            return;
        };
        // The harness is about to go, so nothing can answer what it asked: release the parked
        // calls now rather than leaving a thread waiting out the hour on a dead conversation.
        self.close_asks(agent_id, AskClosed::Gone);
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
        self.forget_doc_delivery(agent_id);
        let Some(conversation) = self.conversations.remove(&agent_id) else {
            return;
        };
        // As in `unload_conversation`: the harness is going, so its questions are closed here.
        self.close_asks(agent_id, AskClosed::Gone);
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

    /// Close every question this conversation left waiting, and tell the window that drew them.
    ///
    /// Called wherever a conversation stops being something an answer could reach — ended,
    /// unloaded, or its harness gone. The parked tool call is released first, so the harness gets
    /// its error rather than waiting out the hour, and the owning window is told per ask so a
    /// dialog still on screen stops offering an answer that can no longer land. The owner is read
    /// before it is removed, which is why this runs ahead of the removal at each site.
    /// Close the registered dialogs this conversation has on screen, leaving its parked calls
    /// alone. What a prompt from the window does: the user answered by typing.
    fn close_armed_dialogs(&mut self, agent_id: AgentId) {
        let closed = self.armed.close_for_agent(agent_id);
        self.tell_asks_closed(agent_id, closed, AskClosed::Gone);
    }

    fn close_asks(&mut self, agent_id: AgentId, why: AskClosed) {
        let mut closed = self.asks.end_for_agent(agent_id, why);
        // The registered dialogs go the same way: there is no call to release, but a dialog on
        // screen must stop offering an answer that would open a turn on a conversation that has
        // gone. A row still armed is simply forgotten — nothing was ever drawn for it.
        closed.extend(self.armed.close_for_agent(agent_id));
        self.tell_asks_closed(agent_id, closed, why);
    }

    /// Tell the owning window that these asks stopped waiting, so a dialog still on screen for one
    /// of them stops offering an answer that can no longer land.
    fn tell_asks_closed(&mut self, agent_id: AgentId, closed: Vec<AskId>, why: AskClosed) {
        if closed.is_empty() {
            return;
        }
        let Some((client, _)) = self.conversation_owners.get(&agent_id).copied() else {
            return;
        };
        for ask_id in closed {
            self.host.send(
                To::Client(client),
                Message::AskEnded {
                    agent_id,
                    ask_id,
                    why,
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
    /// disk rather than rebuilt: `title` and `summary`, written by the naming thread, a rename or
    /// the harness's own title; `persistent`, written by the
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
            definition: pending.definition.clone(),
            model: pending.chosen_model.clone(),
            thinking: pending.chosen_thinking.clone(),
            mode: pending.chosen_mode.clone(),
            next_seq: pending.next_seq,
            title: held.as_ref().and_then(|row| row.title.clone()),
            summary: held.as_ref().and_then(|row| row.summary.clone()),
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
    /// Every [`TASK_SYNC_EVERY`], re-read each loaded project's `tasks.toml` and tell every
    /// window when the disk has moved on.
    fn sync_tasks_due(&mut self) {
        if self.tasks_synced.elapsed() < TASK_SYNC_EVERY {
            return;
        }
        self.tasks_synced = Instant::now();
        for reply in self.work.lock().sync_from_disk() {
            self.host.send(To::Everyone, reply.into_message());
        }
        // The sync worker has no catalogue of its own — `Projects` never leaves this thread — so
        // the project list is handed over on the tick that already exists for the task files.
        self.tasksrc
            .watch(self.projects.records().iter().map(|held| held.id).collect());
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
                pending.fork = false;
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

    /// Put each title a harness named itself with (`ConvUpdate::Title`) on the live `WorkAgent`
    /// and the durable row, so it survives the next `AgentChanged` and a revive.
    ///
    /// **Never over a rename or a generated title.** A harness title is adopted only while the
    /// agent has no title, or still wears the last one the harness gave it — the precedence
    /// T-283 sets: the user's rename, then whichever naming came first, then the harness's later
    /// retitles of its own.
    fn adopt_harness_titles(&mut self) {
        let arrived: Vec<(AgentId, String)> = self
            .conversations
            .iter()
            .filter_map(|(agent_id, conversation)| {
                Some((*agent_id, conversation.take_harness_title()?))
            })
            .collect();
        for (agent_id, title) in arrived {
            let Some((_, project_id)) = self.conversation_owners.get(&agent_id).copied() else {
                continue;
            };
            let Some(pending) = self.pending_conversations.get_mut(&agent_id) else {
                continue;
            };
            let current = self
                .work
                .lock()
                .live_agent_mut(project_id, agent_id)
                .and_then(|agent| agent.title.clone());
            let ours = current.is_none() || current == pending.harness_title;
            if !ours || current.as_deref() == Some(title.as_str()) {
                continue;
            }
            pending.harness_title = Some(title.clone());
            let sessions = self.sessions();
            if let Some(mut row) = conversation_record::load(&sessions, agent_id) {
                row.title = Some(title.clone());
                conversation_record::save(&sessions, agent_id, &row);
            }
            self.publish_conversation_flags(agent_id, |agent| agent.title = Some(title));
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
        // The project the conversation lives in. The naming goes to every window as an
        // `AgentChanged` (T-283): it is the record's title now, not a fact about one tab.
        let Some((_, project_id)) = self.conversation_owners.get(&agent_id).copied() else {
            return;
        };
        // What lands the naming on the live record from this thread — see `publish_live_agent`.
        let everyone = self.host.mailbox(To::Everyone);
        let work = self.work.clone();
        let registry = self.agents.mcp_agents();
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
                                row.summary = summary.clone();
                                conversation_record::save(&sessions, agent_id, &row);
                            }
                            publish_live_agent(
                                &work,
                                &registry,
                                &everyone,
                                project_id,
                                agent_id,
                                |agent| {
                                    agent.title = Some(title.clone());
                                    agent.summary = summary;
                                },
                            );
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
    /// **Neither kind is ended; both are parked** (`finish_one_shot_turn`). For a one-shot harness
    /// an exit is a turn over (`G95`); for a multi-turn one it is a crash or error, and ending it
    /// would drop the owner so the window's Unload, Resume and Close all vanished (`T-296`).
    fn reap_conversations(&mut self) {
        let ended: Vec<AgentId> = self
            .conversations
            .iter()
            .filter(|(_, conversation)| conversation.ended())
            .map(|(agent_id, _)| *agent_id)
            .collect();
        for agent_id in ended {
            // **Both kinds are put back, never ended.** A multi-turn harness that exits unasked —
            // a crash, an auth or API error — used to be ended here, which dropped the owner and
            // the pending row: the window still held a conversation, but every Unload, Resume and
            // Close it then sent hit `drives` and vanished (`T-296`). Parked like an unload
            // instead, the same three controls work from the errored state, and the transcript
            // (with the pump's own `ConversationEnded`, already sent) stays readable.
            self.finish_one_shot_turn(agent_id);
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
        self.forget_doc_delivery(agent_id);
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
            pending.fork = false;
        }
        tracing::debug!(
            agent = %agent_id,
            harness = %pending.agent_type,
            resume = ?pending.resume,
            "a one-shot harness finished its turn; the conversation stays"
        );
        self.remember_conversation(agent_id);
        // The harness behind the parked call has gone, so nothing can answer it now.
        self.close_asks(agent_id, AskClosed::Gone);
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
        self.forget_doc_delivery(agent_id);
        if let Some(conversation) = self.conversations.remove(&agent_id) {
            tracing::info!("agent {agent_id} ending: {reason:?}");
            conversation.stop(false);
        }
        // A pending agent has no `Conversation` and no run directory yet — closed here, this
        // already covers "closed before it ever launched" without a second path.
        self.pending_conversations.remove(&agent_id);
        self.close_asks(agent_id, AskClosed::Gone);
        if let Some((_, project_id)) = self.conversation_owners.remove(&agent_id) {
            self.work.lock().remove_live_agent(project_id, agent_id);
            // **An agent that ends frees its slot** (M23). This is the lifecycle event the
            // scheduler cares about most: the agent is off the list the sweep reads, so the task
            // it was holding either counts an attempt or is already finished, and the slot is
            // filled again on this same pass. Before `retire_agent`, which takes the facts that
            // say which mission it was in.
            self.wake_agent_scheduler(project_id, agent_id);
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

    /// A run composed with a stale catalog reference still launches — see
    /// [`crate::agent::Composed::problems`] — but the person who set up that definition still gets
    /// to hear about the entry that was dropped, as a dismissable notification rather than a run
    /// that silently came up short. `harness` and `actor` name what the bell attributes it to;
    /// several problems on one run collapse into one notification rather than one per line, so a
    /// definition with three stale ids does not flash the bell three times for a single launch.
    fn report_run_problems(&mut self, client: ClientId, harness: &str, problems: &[String]) {
        if problems.is_empty() {
            return;
        }
        let text = if problems.len() == 1 {
            format!("{harness}: {}", problems[0])
        } else {
            format!(
                "{harness}: {} entries in this definition were dropped — {}",
                problems.len(),
                problems.join("; ")
            )
        };
        let replies = self.notifications.raise(
            NotificationRequest::warning(Family::Agents, text)
                .with_actor(harness)
                .with_category("agent-definition"),
        );
        self.answer(client, replies);
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
        let replies = change(&mut self.work.lock());
        self.answer(client, replies);
    }

    /// One work message aimed at a named board (`T-360`). The default board's envelope is the bare
    /// message. Only the task-editing variants are a board's; everything else — missions, agents,
    /// plans, task sync — is the default board's alone, and is refused here, in the envelope, so
    /// the window hears which board said no.
    fn board_job(&mut self, client: ClientId, board: ubiq_proto::work::BoardId, message: Message) {
        if board.is_default() {
            self.dispatch(client, message);
            return;
        }
        let Some(project_id) = message.project_id() else {
            tracing::warn!("a board envelope around a message naming no project: {message:?}");
            return;
        };
        let refuse = |error: &str| Message::OnBoard {
            board: board.clone(),
            message: Box::new(Message::WorkError {
                project_id,
                task_id: None,
                error: error.to_string(),
            }),
        };
        if self.projects.record(project_id).is_none() {
            self.host
                .send(To::Client(client), refuse("no such project"));
            return;
        }
        type Run = Box<dyn FnOnce(&mut Work) -> Vec<Reply>>;
        let run: Run = match message {
            Message::ListWork { .. } => Box::new(move |work| work.list(project_id)),
            Message::CreateTask { title, session, .. } => {
                Box::new(move |work| work.create(project_id, title, session))
            }
            Message::UpdateTask {
                task_id,
                title,
                description,
                priority,
                ..
            } => {
                Box::new(move |work| work.update(project_id, task_id, title, description, priority))
            }
            // A level is what makes a task a mission, and missions live on the default board.
            Message::SetTaskField {
                field: TaskField::Level(Some(_)),
                ..
            } => {
                self.host.send(
                    To::Client(client),
                    refuse("a task on a named board cannot be given a level"),
                );
                return;
            }
            Message::SetTaskField { task_id, field, .. } => {
                Box::new(move |work| work.set_field(project_id, task_id, field))
            }
            Message::MoveTask {
                task_id,
                status,
                before,
                ..
            } => Box::new(move |work| work.move_task(project_id, task_id, status, before)),
            Message::AssignTask {
                task_id, session, ..
            } => Box::new(move |work| work.assign(project_id, task_id, session)),
            Message::DeleteTask { task_id, .. } => {
                Box::new(move |work| work.delete(project_id, task_id))
            }
            Message::ArchiveTasks { .. } => Box::new(move |work| work.archive(project_id)),
            Message::AddStep { task_id, title, .. } => {
                Box::new(move |work| work.add_step(project_id, task_id, title))
            }
            Message::RenameStep {
                task_id,
                step_id,
                title,
                ..
            } => Box::new(move |work| work.rename_step(project_id, task_id, step_id, title)),
            Message::RemoveStep {
                task_id, step_id, ..
            } => Box::new(move |work| work.remove_step(project_id, task_id, step_id)),
            Message::MoveStep {
                task_id,
                step_id,
                to,
                ..
            } => Box::new(move |work| work.move_step(project_id, task_id, step_id, to)),
            Message::ToggleStep {
                task_id, step_id, ..
            } => Box::new(move |work| work.toggle_step(project_id, task_id, step_id)),
            Message::AddComment { task_id, text, .. } => Box::new(move |work| {
                work.add_comment(
                    project_id,
                    task_id,
                    ubiq_proto::work::CommentAuthor::User,
                    text,
                )
            }),
            other => {
                tracing::warn!("refused on board {board}: {other:?}");
                self.host.send(
                    To::Client(client),
                    refuse("only the default board takes that"),
                );
                return;
            }
        };
        let replies = self
            .work
            .lock()
            .on_board(project_id, &board, |work| run(work));
        self.answer(client, replies);
    }

    /// Hand one mission message to the missions, [`Self::work_job`]'s own shape and for its
    /// reason: a project the catalogue does not hold gets no directory written under it.
    /// Which mission an agent is in, with its roster entry written down if it is a new member
    /// (M11) — the one place that decides, called at launch and again on every `AssignAgent`.
    ///
    /// **Two ways in, and an assignment wins.** An agent assigned to a mission's anchor or to one
    /// of its children is in that mission; otherwise it inherits its spawner's, read off the
    /// spawner's own [`crate::mcp::AgentFacts`]. The second is fixed here, at the moment it is
    /// asked, because `Work::assign_agent` clears `WorkAgent::parent` on every reassignment and
    /// the link is gone the next time anybody looks — which is why the roster is stored.
    ///
    /// Harness-native subagents are not this: a Claude Code `Task` delegate is read off the stream
    /// and never becomes a `WorkAgent`, so it is drawn as a delegate ring and counted in spend,
    /// and it is never rostered (`D47`).
    ///
    /// Returns the mission, and leaves every reply it made on the bus.
    fn settle_mission(
        &mut self,
        project_id: ProjectId,
        agent_id: ubiq_proto::work::AgentId,
    ) -> Option<ubiq_proto::ids::TaskId> {
        // The board's lock is taken and released before the missions' is: `Missions` takes the
        // board's for its own refusals, and holding both in this order would be the one place they
        // could deadlock.
        let (task, parent) = {
            let (_, agents) = self.work.lock().agents(project_id);
            let agent = agents.iter().find(|agent| agent.id == agent_id)?;
            (agent.task, agent.parent)
        };
        let assigned = task.and_then(|task| self.missions.lock().mission_of_task(project_id, task));
        let mission = match assigned {
            Some(mission) => mission,
            None => parent
                .and_then(|parent| self.agents.mcp_agents().facts(&parent.to_string()))
                .and_then(|facts| facts.mission)?,
        };

        let role = self
            .missions
            .lock()
            .record(project_id, mission)
            .filter(|record| record.coordinator == Some(agent_id))
            .map_or(ubiq_proto::mission::MissionRole::Worker, |_| {
                ubiq_proto::mission::MissionRole::Coordinator
            });
        let replies = self.missions.lock().join(
            project_id,
            mission,
            agent_id,
            role,
            ubiq_proto::mission::Actor::Host,
        );
        for reply in replies {
            self.host.send(To::Everyone, reply.into_message());
        }
        Some(mission)
    }

    /// Run the scheduler of whatever mission `task_id` belongs to (M23).
    ///
    /// **This is where the scheduler runs**: the coordinator's own thread, on an event it was
    /// already handling, with no thread and no timer of its own. It is a record read for every
    /// mission that is not in auto *and* in the In-progress phase, so calling it from more places
    /// than strictly necessary is cheap and missing one is not.
    ///
    /// Everything it decides goes out broadcast, [`Self::settle_mission`]'s own posture: nobody
    /// asked for any of it, so there is no asker to answer.
    fn wake_scheduler(&mut self, project_id: ProjectId, task_id: ubiq_proto::ids::TaskId) {
        if self.projects.record(project_id).is_none() {
            return;
        }
        let Some(mission) = self.missions.lock().mission_of_task(project_id, task_id) else {
            return;
        };
        let replies = self.missions.lock().schedule(project_id, mission);
        for reply in replies {
            self.host.send(To::Everyone, reply.into_message());
        }
    }

    /// The same, for an event about an **agent** rather than a task: the mission is read off the
    /// agent's own facts, which is the only thing that still knows it once its task is gone.
    fn wake_agent_scheduler(&mut self, project_id: ProjectId, agent_id: AgentId) {
        if self.projects.record(project_id).is_none() {
            return;
        }
        let Some(mission) = self
            .agents
            .mcp_agents()
            .facts(&agent_id.to_string())
            .and_then(|facts| facts.mission)
        else {
            return;
        };
        let replies = self.missions.lock().schedule(project_id, mission);
        for reply in replies {
            self.host.send(To::Everyone, reply.into_message());
        }
    }

    fn mission_job(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        change: impl FnOnce(&mut crate::mission::Missions) -> Vec<Reply>,
    ) {
        if self.projects.record(project_id).is_none() {
            self.host.send(
                To::Client(client),
                Message::MissionError {
                    project_id,
                    task_id: None,
                    error: "no such project".to_string(),
                },
            );
            return;
        }
        let replies = change(&mut self.missions.lock());
        self.answer(client, replies);
    }

    /// Hand one annotated-document message to the plans, [`Self::work_job`]'s own shape: a project
    /// the catalogue does not hold gets no file written under it, plan included.
    ///
    /// **This is where a [`ubiq_proto::plan::DocumentHandle`] becomes a [`crate::plan::Target`]**
    /// — the one place a project-relative path is resolved and contained, so nothing below ever
    /// sees one. A path that leaves the project, or names something that is not markdown, is
    /// refused here and nothing is read or written.
    fn plan_job(
        &mut self,
        client: ClientId,
        doc: &ubiq_proto::plan::DocumentHandle,
        change: impl FnOnce(&mut crate::plan::Plans, &crate::plan::Target) -> Vec<Reply>,
    ) {
        let project_id = doc.project_id();
        let Some(record) = self.projects.record(project_id) else {
            self.host.send(
                To::Client(client),
                Message::PlanError {
                    project_id,
                    doc: None,
                    error: "no such project".to_string(),
                },
            );
            return;
        };
        let root = PathBuf::from(&record.path);
        let target = match crate::plan::Target::resolve(doc, &root) {
            Ok(target) => target,
            Err(error) => {
                self.host.send(
                    To::Client(client),
                    Message::PlanError {
                        project_id,
                        doc: Some(doc.clone()),
                        error,
                    },
                );
                return;
            }
        };
        let replies = change(&mut self.plans.lock(), &target);
        self.answer(client, replies);
    }

    /// Queue a user's comment for the agent it goes to, after the mutation succeeded (`D207`).
    ///
    /// **A bound document goes to its agent only with auto-send on**; with it off every thread
    /// waits on the document, addressed or not, for the user's `AskDocAgent`. An unbound plan or
    /// mission document keeps the older rule for an addressed comment: the mission's coordinator,
    /// else the task's assignee. Anything else — an unbound file document — goes nowhere, and the
    /// comment and its `Agent` mark stand on the thread.
    fn deliver_to_agent(
        &mut self,
        doc: &ubiq_proto::plan::DocumentHandle,
        found: Option<(ubiq_proto::plan::DocBinding, crate::plan::queue::QueuedRef)>,
        to: Option<ubiq_proto::work::Addressee>,
    ) {
        let Some((binding, queued)) = found else {
            return;
        };
        let project = doc.project_id();
        let addressed = to == Some(ubiq_proto::work::Addressee::Agent);
        let agent = match binding.agent {
            // Auto-send off: the thread waits on the document, addressed or not, until the
            // user's Ask agent (`AskDocAgent`) sends everything waiting in one prompt.
            Some(agent) => binding.auto_send.then_some(agent),
            None if addressed => doc.task_id().and_then(|task| {
                let coordinator = self
                    .missions
                    .lock()
                    .record(project, task)
                    .and_then(|record| record.coordinator);
                let (_, tasks) = self.work.lock().tasks(project);
                let assigned = tasks
                    .iter()
                    .find(|record| record.id == task)
                    .and_then(|record| record.assigned_to.clone());
                crate::plan::choose_agent(coordinator, assigned.as_deref())
            }),
            None => None,
        };
        let Some(agent) = agent else {
            tracing::debug!("comment on {doc:?}: nobody to deliver to");
            return;
        };
        self.doc_queue.push(agent, queued);
        self.deliver_doc_queue(agent, project);
    }

    /// Deliver the agent's pending doc prompt if it can take one now — a live conversation that
    /// takes input and is between turns, and not backing off a refused send — and tell the windows
    /// where its queue stands either way. `project` is where to report when nothing is left
    /// queued to say it.
    ///
    /// The prompt is composed from the sidecars as they read now (`Plans::thread_view`), goes in
    /// front of the model *and* into the agent's thread row, as `Missions::tell_agent` does. An
    /// agent that is not running is started first ([`Self::autostart_doc_agent`]) — the user
    /// bound it and asked; one that cannot be started keeps its entry until it runs.
    fn deliver_doc_queue(&mut self, agent: AgentId, project: ProjectId) {
        use crate::plan::queue::Delivery;
        use ubiq_proto::plan::DocDelivery;
        let project = self.doc_queue.project_of(agent).unwrap_or(project);
        self.autostart_doc_agent(agent);
        let backing_off = self
            .doc_backoff
            .get(&agent)
            .is_some_and(|(until, _)| Instant::now() < *until);
        let mut state = self.doc_state(agent);
        let delivery = match self.conversations.get(&agent) {
            Some(conversation) if state == DocDelivery::Delivered && !backing_off => {
                let projects = &self.projects;
                let mut plans = self.plans.lock();
                self.doc_queue.deliver(
                    agent,
                    |queued| {
                        // Only a file document resolves against the project's root.
                        let root = projects
                            .record(queued.doc.project_id())
                            .map(|record| PathBuf::from(&record.path))
                            .unwrap_or_default();
                        let target = crate::plan::Target::resolve(&queued.doc, &root).ok()?;
                        plans.thread_view(&target, queued)
                    },
                    |text| conversation.prompt(text.to_string()),
                )
            }
            _ => Delivery::Nothing,
        };
        match delivery {
            Delivery::Sent(text) => {
                self.doc_backoff.remove(&agent);
                if let Some((_, owner_project)) = self.conversation_owners.get(&agent).copied() {
                    let replies = self.work.lock().send_to_agent(owner_project, agent, text);
                    for reply in replies {
                        self.host.send(To::Everyone, reply.into_message());
                    }
                }
            }
            Delivery::Failed(error) => {
                // The entry stays queued (`DocQueue::deliver`); the next try waits, doubling.
                let wait = self
                    .doc_backoff
                    .get(&agent)
                    .map_or(DOC_RETRY_FIRST, |(_, wait)| (*wait * 2).min(DOC_RETRY_MAX));
                self.doc_backoff
                    .insert(agent, (Instant::now() + wait, wait));
                tracing::warn!(
                    "agent {agent}: the doc prompt was not delivered, retrying in {wait:?}: \
                     {error:#}"
                );
                state = DocDelivery::NotRunning;
            }
            // Nothing went out. An idle agent with nothing left queued (every thread resolved or
            // gone) stays `Delivered` — nothing is held back; one backing off is not taking input.
            Delivery::Nothing if backing_off && state == DocDelivery::Delivered => {
                state = DocDelivery::NotRunning;
            }
            Delivery::Nothing => {}
        }
        self.host.send(
            To::Everyone,
            Message::DocAgentQueue {
                project_id: project,
                agent_id: agent,
                pending: self.doc_queue.pending(agent),
                state,
            },
        );
    }

    /// Start the bound agent when its doc queue has something for it and it is not running —
    /// exactly what the owning window's `ResumeConversation` does, on that window's behalf, so the
    /// one launcher stays the host's and two windows showing the document cannot both start it.
    /// An agent no window owns, or never set up here, stays `NotRunning`; a one-shot harness is
    /// left alone (`resume_conversation`'s own rule). After the launch the conversation is idle,
    /// so the delivery that follows goes out at once.
    fn autostart_doc_agent(&mut self, agent: AgentId) {
        if self.conversations.contains_key(&agent) || !self.doc_queue.has(agent) {
            return;
        }
        let Some((owner, _)) = self.conversation_owners.get(&agent).copied() else {
            return;
        };
        tracing::debug!(agent = %agent, "starting the agent its doc queue is waiting on");
        self.resume_conversation(owner, agent);
    }

    /// The conversation is going: its next one has been sent no document yet, and a refused send
    /// against this one says nothing about the next.
    fn forget_doc_delivery(&mut self, agent: AgentId) {
        self.doc_queue.forget(agent);
        self.doc_backoff.remove(&agent);
    }

    /// Where the agent stands for the doc queue: `Delivered` means it could take a prompt now.
    fn doc_state(&self, agent: AgentId) -> ubiq_proto::plan::DocDelivery {
        use ubiq_proto::plan::DocDelivery;
        match self.conversations.get(&agent) {
            None => DocDelivery::NotRunning,
            Some(conversation) if !conversation.accepts_input() => DocDelivery::NotRunning,
            Some(conversation) if conversation.busy() => DocDelivery::Waiting,
            Some(_) => DocDelivery::Delivered,
        }
    }

    /// The run loop's half of the doc queue: an agent whose turn has ended since its entry was
    /// queued — or whose conversation has since come up, or whose back-off has run out — gets it
    /// now. Polled, because the turn end is seen by the pump thread, which counts it and says
    /// nothing to this one; the loop wakes every `CONVERSATION_POLL` while a conversation is live.
    fn flush_doc_queue(&mut self) {
        let now = Instant::now();
        for agent in self.doc_queue.waiting() {
            let ready = self.doc_state(agent) == ubiq_proto::plan::DocDelivery::Delivered
                && self
                    .doc_backoff
                    .get(&agent)
                    .is_none_or(|(until, _)| now >= *until);
            if ready && let Some(project) = self.doc_queue.project_of(agent) {
                self.deliver_doc_queue(agent, project);
            }
        }
    }

    /// Write a copy of a task's plan into the project's own working tree, at `rel_path`.
    ///
    /// An explicit, one-shot action — never a continuous mirror — so it happens synchronously on
    /// this thread rather than through [`Self::files`]'s worker: the body is already in hand from
    /// [`crate::plan::Plans`] (itself an in-process read, the same footing [`Self::work_job`]'s
    /// direct calls into `Work` stand on), and one small write costs nothing the coordinator's own
    /// stores do not already pay on every save. `resolve_for_create` both contains the path inside
    /// the project and creates the folders it names, because the export is what brings the file
    /// into being — [`crate::files::path::resolve_for_create`]'s own reasoning for a save-as.
    fn export_plan(
        &mut self,
        client: ClientId,
        doc: &ubiq_proto::plan::DocumentHandle,
        rel_path: String,
    ) {
        let project_id = doc.project_id();
        let error = |error: String| Message::PlanError {
            project_id,
            doc: Some(doc.clone()),
            error,
        };

        let Some(record) = self.projects.record(project_id) else {
            self.host
                .send(To::Client(client), error("no such project".to_string()));
            return;
        };
        let root = PathBuf::from(&record.path);
        let target = match crate::plan::Target::resolve(doc, &root) {
            Ok(target) => target,
            Err(message) => {
                self.host.send(To::Client(client), error(message));
                return;
            }
        };

        let body = match self.plans.lock().body(&target) {
            Ok(body) => body,
            Err(message) => {
                self.host.send(To::Client(client), error(message));
                return;
            }
        };

        let dest = match files::path::resolve_for_create(&root, &rel_path) {
            Ok(dest) => dest,
            Err(file_error) => {
                self.host
                    .send(To::Client(client), error(file_error.to_string()));
                return;
            }
        };

        match crate::store::plan::export_to(&dest, &body) {
            Ok(()) => self.host.send(
                To::Client(client),
                Message::PlanExported {
                    doc: doc.clone(),
                    rel_path,
                },
            ),
            Err(store_error) => self
                .host
                .send(To::Client(client), error(store_error.to_string())),
        }
    }

    /// Hand one file-family request to the worker.
    ///
    /// The only thing this decides is which folder the request is against; a project the catalogue
    /// does not hold is refused here rather than reaching a thread that could not answer it.
    /// The project root and annotation target for `rel_path`, when it names a markdown file that
    /// carries a sidecar — the files whose tab saves go through the plans lock (`D208`).
    fn annotated_file(
        &self,
        project_id: ProjectId,
        rel_path: &str,
    ) -> Option<(PathBuf, crate::plan::Target)> {
        if !rel_path.to_ascii_lowercase().ends_with(".md") {
            return None;
        }
        let root = PathBuf::from(&self.projects.record(project_id)?.path);
        let doc = ubiq_proto::plan::DocumentHandle::File {
            project_id,
            rel_path: rel_path.to_string(),
        };
        let target = crate::plan::Target::resolve(&doc, &root).ok()?;
        self.plans
            .lock()
            .is_annotated(&target)
            .then_some((root, target))
    }

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

    /// A project's own folder, for the one knowledge-base origin that reads or writes inside it —
    /// `KbOrigin::Git { store: KbStore::Project, .. }` today. A project id with no catalogue
    /// record has no such source configured either, so the empty path this falls back to is never
    /// actually resolved against.
    fn kb_project_path(&self, project_id: ProjectId) -> PathBuf {
        self.projects
            .record(project_id)
            .map(|record| PathBuf::from(&record.path))
            .unwrap_or_default()
    }

    /// Hand one knowledge-base request to the worker.
    ///
    /// The only thing this decides is which source the request resolves against; a project with
    /// no such source — never written, or written without this id — is refused here rather than
    /// reaching a thread that could not answer it, on [`Self::file_job`]'s own reasoning.
    fn kb_job(
        &self,
        client: ClientId,
        project_id: ProjectId,
        source_id: KbSourceId,
        rel_path: &str,
        request: files::Request,
    ) {
        let Some(source) = self.kb.find(project_id, source_id) else {
            self.host.send(
                To::Client(client),
                files::kb_error(
                    project_id,
                    source_id,
                    rel_path,
                    FileError::Refused("no such knowledge-base source".to_string()),
                ),
            );
            return;
        };

        let key = match self.kb.vault_key(project_id, &source) {
            Ok(key) => key,
            Err(error) => {
                self.host.send(
                    To::Client(client),
                    files::kb_error(project_id, source_id, rel_path, error),
                );
                return;
            }
        };
        let project_path = self.kb_project_path(project_id);
        let base = self.kb.base_path(project_id, &source, &project_path);
        self.files.submit(files::Job {
            kind: files::JobKind::Kb {
                project_id,
                source,
                base,
                request,
                key,
            },
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    /// Resolve a knowledge-base source and its base path, for one of the six mutating arms above.
    /// `Kb::find` and `Kb::base_path`, exactly as [`Self::kb_job`] resolves them for a read — the
    /// two never diverge, because a write against a source a read could not find would be a
    /// contract nobody could reason about.
    fn kb_resolve(
        &self,
        project_id: ProjectId,
        source_id: KbSourceId,
    ) -> Result<KbResolved, FileError> {
        let Some(source) = self.kb.find(project_id, source_id) else {
            return Err(FileError::Refused(
                "no such knowledge-base source".to_string(),
            ));
        };
        let key = self.kb.vault_key(project_id, &source)?;
        let project_path = self.kb_project_path(project_id);
        let base = self.kb.base_path(project_id, &source, &project_path);
        Ok((source, base, key))
    }

    /// Answer an unlock or a password change to the window that asked, and on success tell every
    /// window the wiki is open.
    fn kb_password_answer(
        &self,
        client: ClientId,
        project_id: ProjectId,
        source: KbSourceId,
        result: Result<(), String>,
    ) {
        let error = result.err();
        if error.is_none() {
            self.host.send(
                To::Everyone,
                Message::KbSourceChanged {
                    project_id,
                    source,
                    state: ubiq_proto::kb::KbSourceState::Ready,
                },
            );
        }
        self.host.send(
            To::Client(client),
            Message::KbPasswordAnswer {
                project_id,
                source,
                error,
            },
        );
    }

    /// Say a directory in one knowledge-base source changed, naming `rel_path`'s own parent — the
    /// folder whoever is drawing it has to re-list, on [`Message::KbChanged`]'s own reasoning.
    fn kb_changed(
        &self,
        client: ClientId,
        project_id: ProjectId,
        source: KbSourceId,
        rel_path: &str,
    ) {
        self.host.send(
            To::Client(client),
            Message::KbChanged {
                project_id,
                source,
                rel_path: kb::ops::parent_of(rel_path),
            },
        );
    }

    /// One path's failure in one knowledge-base source, on [`files::kb_error`]'s own reasoning.
    fn kb_failed(
        &self,
        client: ClientId,
        project_id: ProjectId,
        source: KbSourceId,
        rel_path: &str,
        error: FileError,
    ) {
        self.host.send(
            To::Client(client),
            files::kb_error(project_id, source, rel_path, error),
        );
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

    /// Hand one host-file write to the worker, on [`Self::browse_job`]'s own reasoning: no
    /// project to look up, so nothing to refuse here before it reaches the thread that answers it.
    fn host_write_job(
        &self,
        client: ClientId,
        path: String,
        bytes: Vec<u8>,
        expected: FileVersion,
    ) {
        self.files.submit(files::Job {
            kind: files::JobKind::HostWrite {
                path,
                bytes,
                expected,
            },
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    /// Answer how much of an account's plan is left: from the cache where that is honest, from
    /// the provider where it is not.
    ///
    /// Three answers, in order. A harness whose provider states nothing is refused in place,
    /// because a probe would be a network call that can only fail. A reading already held is the
    /// answer unless `fresh` asks for another one — the endpoint behind a probe is unofficial and
    /// rate-limits, so asking again is what a user's own refresh means and not what a panel
    /// opening means. Otherwise the worker asks, and [`Message::QuotaRead`] arrives when it has.
    fn query_quota(&self, client: ClientId, account: String, harness: String, fresh: bool) {
        let refusal = match self.agents.quota_source(&harness) {
            None => Some(format!("'{harness}' is not an agent type Ubiq knows")),
            Some(source) if !source.reports() => {
                Some(format!("{harness} states no limit anybody can read"))
            }
            Some(_) => None,
        };
        if let Some(error) = refusal {
            self.host.send(
                To::Client(client),
                Message::QuotaRead {
                    account,
                    harness,
                    snapshot: None,
                    error: Some(error),
                },
            );
            return;
        }

        if !fresh && let Some(snapshot) = self.quotas.get(&account, &harness) {
            self.host.send(
                To::Client(client),
                Message::QuotaRead {
                    account,
                    harness,
                    snapshot: Some(snapshot.clone()),
                    error: None,
                },
            );
            return;
        }

        self.quota.submit(crate::quota::Job {
            account,
            harness,
            reply_to: self.host.mailbox(To::Client(client)),
            voice: self.host.voice(),
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

    /// Whether a catalog scope names a layer that exists: the application's, or a project the
    /// catalogue holds. A layer for a project nobody knows would be a directory nothing can
    /// forget.
    fn catalog_scope_known(&self, scope: Option<ProjectId>) -> bool {
        scope.is_none_or(|project| self.projects.record(project).is_some())
    }

    /// Apply one catalog mutation, then tell **every** window the layer as it now stands — or
    /// tell the asker why not.
    ///
    /// `background` puts the work on a thread of its own, for the one mutation that fetches over
    /// the network (`AddSkill::Remote`); the answer goes out from there through mailboxes, so the
    /// coordinator never waits on a clone.
    fn catalog_change(
        &self,
        client: ClientId,
        scope: Option<ProjectId>,
        background: bool,
        work: impl FnOnce(&Catalog) -> anyhow::Result<()> + Send + 'static,
    ) {
        if !self.catalog_scope_known(scope) {
            self.host.send(
                To::Client(client),
                Message::CatalogError {
                    error: "no such project".to_string(),
                },
            );
            return;
        }
        let catalog = Catalog::new(self.root.path.clone());
        let asker = self.host.mailbox(To::Client(client));
        let everyone = self.host.mailbox(To::Everyone);
        let run = move || match work(&catalog).and_then(|()| catalog.snapshot(scope)) {
            Ok(layer) => everyone.send(layer),
            Err(error) => asker.send(Message::CatalogError {
                error: format!("{error:#}"),
            }),
        };
        if background {
            thread::Builder::new()
                .name("catalog-install".to_string())
                .spawn(run)
                .ok();
        } else {
            run();
        }
    }

    /// Answer the asker with what `work` produces, on a thread of its own: a search that clones a
    /// repository or calls a registry.
    fn catalog_answer(
        &self,
        client: ClientId,
        name: &str,
        work: impl FnOnce() -> Message + Send + 'static,
    ) {
        let asker = self.host.mailbox(To::Client(client));
        thread::Builder::new()
            .name(format!("catalog-{name}"))
            .spawn(move || asker.send(work()))
            .ok();
    }

    /// Hand one git-family request to the worker.
    ///
    /// The only thing this decides is which folder the request is against; a project the catalogue
    /// does not hold is refused here rather than reaching a thread that could not answer it.
    fn git_job(
        &self,
        client: ClientId,
        project_id: ProjectId,
        repo: String,
        request: git::Request,
    ) {
        let Some(record) = self.projects.record(project_id) else {
            self.host.send(
                To::Client(client),
                git::git_error(project_id, &repo, ubiq_proto::git::GitError::NotFound),
            );
            return;
        };

        self.git.submit(git::Job {
            project_id,
            root: PathBuf::from(&record.path),
            managed_repos: record.managed_repos.clone(),
            repo,
            request,
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    fn git_forget(&self, client: ClientId, project_id: ProjectId) {
        self.git.submit(git::Job {
            project_id,
            root: PathBuf::new(),
            // Forgetting drops a cached handle and reads nothing, so there is no set to carry.
            managed_repos: Vec::new(),
            repo: String::new(),
            request: git::Request::Forget,
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    /// Start watching a project a window just opened, replacing whatever that window watched
    /// before. A watch that will not start is logged and nothing else: the project is simply not
    /// live, and every other answer still works.
    fn watch_project(&mut self, client: ClientId, project_id: ProjectId) {
        // Opening a project is the moment its runner targets are worth knowing, so the first ask
        // for its tools already has them.
        self.scan_runners(project_id);
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

    /// File an SSH profile's passphrase or password, and stamp the flag that says there is one.
    ///
    /// Refused rather than filed for a profile that is not there or whose auth method reads no
    /// secret: material stored against a profile that will never ask for it is a leak with no
    /// user-visible way back to it. The flag and the secret move together, so a row on screen
    /// never claims a secret the store does not hold.
    fn set_ssh_secret(&self, profile_id: SshProfileId, secret: Secret) -> Vec<Reply> {
        // What the record says is checked before whether the keychain works, so the refusal names
        // the specific thing that is wrong: a profile that holds no secret is refused the same way
        // on a machine with no secret service as on one with.
        if let Err(reason) = self.ssh_profile_takes_secret(profile_id) {
            return vec![settings_error(reason)];
        }
        if secret.expose().trim().is_empty() {
            return vec![settings_error("that secret is blank".to_string())];
        }
        let store = self.connectors.store();
        if let Err(reason) = store.usable() {
            return vec![settings_error(reason)];
        }
        if let Err(reason) = store.set_ssh_secret(profile_id, secret.expose()) {
            return vec![settings_error(reason)];
        }
        self.stamp_ssh_flag(profile_id, true)
    }

    /// Forget an SSH profile's stored secret, leaving the profile itself alone.
    fn clear_ssh_secret(&self, profile_id: SshProfileId) -> Vec<Reply> {
        if let Err(reason) = self.ssh_profile_takes_secret(profile_id) {
            return vec![settings_error(reason)];
        }
        let store = self.connectors.store();
        if let Err(reason) = store.usable() {
            return vec![settings_error(reason)];
        }
        if let Err(reason) = store.clear_ssh_secret(profile_id) {
            return vec![settings_error(reason)];
        }
        self.stamp_ssh_flag(profile_id, false)
    }

    /// That the profile exists and can hold a secret at all.
    fn ssh_profile_takes_secret(&self, profile_id: SshProfileId) -> Result<(), String> {
        match self
            .settings
            .host()
            .ssh_profiles
            .iter()
            .find(|profile| profile.id == profile_id)
        {
            None => Err("no such ssh profile".to_string()),
            Some(profile) if !profile.auth.takes_secret() => {
                Err("that profile's auth method holds no secret".to_string())
            }
            Some(_) => Ok(()),
        }
    }

    /// Write one profile's host-derived flag, and answer with the whole record — the interface
    /// draws the flag and is never sent the material it stands for.
    fn stamp_ssh_flag(&self, profile_id: SshProfileId, filed: bool) -> Vec<Reply> {
        match self.settings.update_host(|host| {
            if let Some(profile) = host
                .ssh_profiles
                .iter_mut()
                .find(|profile| profile.id == profile_id)
            {
                profile.auth.set_has_secret(filed);
            }
        }) {
            Err(reason) => vec![settings_error(reason)],
            Ok(host) => vec![host_settings(&host)],
        }
    }

    /// Reconcile the ssh profiles the interface just wrote against what the secret store holds.
    ///
    /// Two directions, and the interface is authoritative for neither: material nothing names any
    /// more is deleted, and every remaining flag is re-stamped from the store rather than believed
    /// from the blob. Answers nothing when nothing moved — the common case is a settings write
    /// that never touched a profile.
    fn reconcile_ssh_secrets(&self, before: &[SshProfile]) -> Vec<Reply> {
        let store = self.connectors.store();
        let host = self.settings.host();
        for profile_id in crate::settings::stale_ssh_secrets(before, &host.ssh_profiles) {
            if store.has_ssh_secret(profile_id)
                && let Err(reason) = store.clear_ssh_secret(profile_id)
            {
                tracing::warn!("an ssh profile's secret was not deleted: {reason}");
            }
        }
        let restamp: Vec<(SshProfileId, bool)> = host
            .ssh_profiles
            .iter()
            .filter_map(|profile| {
                let filed = store.has_ssh_secret(profile.id);
                (profile.auth.has_secret() != filed).then_some((profile.id, filed))
            })
            .collect();
        if restamp.is_empty() {
            return Vec::new();
        }
        match self.settings.update_host(|host| {
            for profile in &mut host.ssh_profiles {
                if let Some((_, filed)) = restamp.iter().find(|(id, _)| *id == profile.id) {
                    profile.auth.set_has_secret(*filed);
                }
            }
        }) {
            Err(reason) => vec![settings_error(reason)],
            Ok(host) => vec![host_settings(&host)],
        }
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
        let (backend, root, held) = match &subject {
            SuggestSubject::CommitMessage { project_id } => {
                match self.projects.record(*project_id) {
                    Some(record) => (self.assist.clone(), Some(PathBuf::from(&record.path)), None),
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
                    None,
                )
            }
            // The material is a task's description, which is in the work store on this thread —
            // so it is read here and carried, rather than handing the worker the store's lock.
            SuggestSubject::TaskTitle {
                project_id,
                task_id,
            } => {
                if self.projects.record(*project_id).is_none() {
                    return fail("that project is not in the catalogue".to_string());
                }
                let (_, tasks) = self.work.lock().tasks(*project_id);
                let Some(task) = tasks.into_iter().find(|task| task.id == *task_id) else {
                    return fail("no such task".to_string());
                };
                (self.assist.clone(), None, Some(task.description))
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
                let answer =
                    gather(&subject, root.as_deref(), held, &*backend).and_then(|request| {
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
            definition: picks.definition,
            // A terminal pane runs in a project's folder, so a definition scoped to that project
            // resolves here exactly as it does for a conversation.
            project: Some(project_id),
            mcps: picks.mcps,
            skills: picks.skills,
            extra_rw: rw_paths(picks.grants),
            prompt: None,
            resume: None,
            fork: false,
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
        // Same rule `launch` follows for a conversation: a stale definition reference does not stop
        // this pane from opening, but the person who set that definition up still gets to hear
        // about the entry that was dropped.
        if let Some(composed) = &composed {
            self.report_run_problems(client, &agent_type, &composed.problems);
        }

        // Tell the MCP listener who this pane is, before the harness exists to ask — the same
        // reason `launch` registers a conversation before its own composed run spawns. Its `name`
        // is the pane's handle, minted from the harness's command over the same namespace a
        // conversation's is (T-291), so `claude 2` means one thing whichever kind holds it;
        // `account` is what the run actually resolved to, not merely what was picked.
        let handle = composed.as_ref().map(|_| {
            let base = self
                .agents
                .command_of(&agent_type)
                .unwrap_or_else(|| agent_type.clone());
            unique_name(&base, &self.taken_names(project_id))
        });
        if let (Some(composed), Some(handle)) = (&composed, &handle) {
            let project = self.project_facts(project_id);
            self.agents.mcp_agents().register(crate::mcp::AgentFacts {
                key: pane_id.to_string(),
                name: handle.clone(),
                harness: agent_type.clone(),
                account: composed.account().map(str::to_string),
                model: picked_model.clone(),
                mode: picked_mode.clone(),
                cwd: cwd.display().to_string(),
                // A fresh pane resumes nothing.
                session: None,
                project,
                // A pane is not an agent card: it has no `WorkAgent`, so nothing can assign it to
                // a task and it is never on a mission's roster.
                mission: None,
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
        // Re-addressable, because this pane's project may move to another window and the reader
        // below outlives that. See `Coordinator::adopt_project`.
        let (mailbox, address) = self.host.moving_mailbox(To::Client(client));

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

        self.pane_addresses.insert(pane_id, address);

        // The picker's terminal count, and the confirmation it puts in front of a close, are only
        // real because of this.
        self.pane_projects.insert(pane_id, project_id);
        self.pane_sessions.insert(pane_id, session_id);
        if let Some(handle) = &handle {
            self.pane_handles.insert(pane_id, handle.clone());
        }
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
                wait_on_error: false,
                tool: None,
                handle,
            },
        });
    }

    /// The machine-wide tools as the run menu offers them, stamped with whether each
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
                    origin: ToolOrigin::Defined,
                }
            })
            .collect()
    }

    /// One project's own tools, or nothing when no project was named or it is unknown: the rows
    /// the user wrote, then the runner targets the last scan found.
    fn project_tools(&self, project_id: Option<ProjectId>) -> Vec<ListedTool> {
        let os = std::env::consts::OS;
        let Some(record) = project_id.and_then(|id| self.projects.record(id)) else {
            return Vec::new();
        };
        let defined = record.tools.iter().cloned().map(|tool| {
            let applicable = tool.applies(os);
            ListedTool {
                scope: Scope::Project(record.id),
                tool,
                applicable,
                origin: ToolOrigin::Defined,
            }
        });
        let discovered = self
            .discovered
            .get(&record.id)
            .into_iter()
            .flat_map(|found| found.tools.iter())
            .map(|(runner, tool)| ListedTool {
                scope: Scope::Project(record.id),
                applicable: tool.applies(os),
                tool: tool.clone(),
                origin: ToolOrigin::Discovered {
                    runner: runner.clone(),
                },
            });
        defined.chain(discovered).collect()
    }

    /// Answer a window's ask for the run menu's tools, from what is held.
    fn send_tools(&self, client: ClientId, project_id: Option<ProjectId>) {
        self.host.send(
            To::Client(client),
            Message::ToolsListed {
                system: self.system_tools(),
                project: self.project_tools(project_id),
            },
        );
    }

    /// Start a scan of this project's runner files unless one is running or the last is fresh.
    ///
    /// The scan runs `just` or `mise` and takes however long they do, so it is a thread of its own;
    /// it leaves its answer in [`Self::scans`] and wakes the loop with a message from the host's
    /// voice, and [`Self::collect_scans`] takes it from there.
    fn scan_runners(&mut self, project_id: ProjectId) {
        if self.runners.is_empty() || self.scanning.contains(&project_id) {
            return;
        }
        if self
            .discovered
            .get(&project_id)
            .is_some_and(|found| found.at.elapsed() < RUNNER_FRESH)
        {
            return;
        }
        let Some(record) = self.projects.record(project_id) else {
            return;
        };
        let root = PathBuf::from(&record.path);
        let runners = self.runners.clone();
        let done = self.scans.0.clone();
        let voice = self.host.voice();
        let started = thread::Builder::new()
            .name("ubiq-runners".to_string())
            .spawn(move || {
                let tools = runners.discover(&root);
                let _ = done.send(Scanned { project_id, tools });
                voice.say(Message::ListTools {
                    project_id: Some(project_id),
                });
            });
        match started {
            Ok(_) => {
                self.scanning.insert(project_id);
            }
            Err(error) => tracing::warn!(project = %project_id, %error, "no runner scan thread"),
        }
    }

    /// Take the finished runner scans into the cache, and say the new list again to the windows
    /// looking at that project when it differs from the one they were last given.
    fn collect_scans(&mut self) {
        while let Ok(done) = self.scans.1.try_recv() {
            self.scanning.remove(&done.project_id);
            // A project forgotten while its scan ran leaves nothing behind.
            if self.projects.record(done.project_id).is_none() {
                self.discovered.remove(&done.project_id);
                continue;
            }
            let changed = match self.discovered.get(&done.project_id) {
                Some(held) => held.tools != done.tools,
                None => !done.tools.is_empty(),
            };
            self.discovered.insert(
                done.project_id,
                Discovered {
                    tools: done.tools,
                    at: Instant::now(),
                },
            );
            if changed {
                let askers: Vec<ClientId> = self
                    .tool_askers
                    .iter()
                    .filter(|(_, asked)| **asked == done.project_id)
                    .map(|(client, _)| *client)
                    .collect();
                for client in askers {
                    self.send_tools(client, Some(done.project_id));
                }
            }
        }
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
                .find(|tool| &tool.id == id)
                // A discovered target is not in the record; the last scan holds it.
                .or_else(|| {
                    self.discovered
                        .get(project_id)?
                        .tools
                        .iter()
                        .find(|(_, tool)| &tool.id == id)
                        .map(|(_, tool)| tool.clone())
                }),
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
        // One run at a time, when the row says so. The check is here rather than in the window
        // because only the host knows every pane: two windows on the same project would each
        // think theirs was the only one.
        // A discovered id is derived from the runner and the target alone, so `just build` in two
        // projects share one — a project-scope run is therefore one-at-a-time per project.
        let running = self.pane_tools.iter().any(|(pane, held)| {
            *held == id
                && (matches!(scope, Scope::Interface)
                    || self.pane_projects.get(pane) == Some(&project_id))
        });
        if tool.single_instance && running {
            self.tool_error(
                client,
                Some(project_id),
                format!("{} is already running", tool.name),
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
        // with `ProjectError` itself when the folder is gone. A row's own `starting_folder`
        // overrides that resolution outright, unset leaves it exactly as it always was.
        let cwd = match self.resolve_tool_cwd(client, project_id, &tool) {
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
        // Re-addressable for the same reason a harness pane's is: a tool runs in a project, and a
        // project moves between windows with everything running in it.
        let (mailbox, address) = self.host.moving_mailbox(To::Client(client));

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

        self.pane_addresses.insert(pane_id, address);
        self.pane_projects.insert(pane_id, project_id);
        self.pane_sessions.insert(pane_id, session_id);
        self.pane_tools.insert(pane_id, id);
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
                wait_on_error: tool.wait_on_error,
                // What started it, so a stopped pane can be restarted with the same two values
                // this call was addressed with.
                tool: Some(ToolRun { scope, id }),
                // A tool pane is named by the interface, after the tool.
                handle: None,
            },
        });
    }

    /// Tell one window which accounts exist. References only — an id — because a credential
    /// has no business on the bus the log sink listens to.
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

    /// Tell every window which accounts exist, for the one thing that changes the set rather
    /// than reads it: a sign-in that made a new identity (`D194`). An account belongs to the
    /// machine, not to the window that happened to sign it in, so a second window's list would
    /// otherwise be missing it until something asked again.
    fn broadcast_accounts(&mut self) {
        match self.agents.accounts() {
            Ok(accounts) => self.host.send(To::Everyone, Message::Accounts { accounts }),
            // Nothing to broadcast and nothing a window can do about it: every list already on
            // screen is the one from before, which is better than emptying them all.
            Err(error) => tracing::warn!("the accounts could not be read: {error:#}"),
        }
    }

    /// Tell one window which definitions exist. References only, the same rule as
    /// [`Self::send_accounts`] — a definition names an account, it never carries one.
    ///
    /// Both scopes, in one list: the global definitions, then every project's own, each stamped
    /// with the project it belongs to. The window already holds every project, so a scope is a
    /// field to filter on rather than a second ask — and a project whose definitions cannot be
    /// read contributes none rather than emptying the list.
    fn send_definitions(&mut self, client: ClientId) {
        match self.agents.definitions() {
            Ok(mut definitions) => {
                let ids: Vec<ProjectId> = self.projects.records().iter().map(|it| it.id).collect();
                for project in ids {
                    match self.agents.project_definitions(project) {
                        Ok(scoped) => definitions.extend(scoped),
                        Err(error) => {
                            tracing::warn!(
                                "the definitions of project {project} could not be read: {error:#}"
                            );
                        }
                    }
                }
                self.host.send(
                    To::Client(client),
                    Message::AgentDefinitions { definitions },
                )
            }
            Err(error) => {
                tracing::warn!("the definitions could not be read: {error:#}");
                self.host.send(
                    To::Client(client),
                    Message::AgentDefinitions {
                        definitions: Vec::new(),
                    },
                );
            }
        }
    }

    /// Open a pane running a harness's own sign-in into an account's config home
    /// (`D194`, [`Agents::begin_home_login`]), and remember which account it is for.
    ///
    /// A login pane is a pane in every respect but one: it belongs to no project, so it
    /// changes no project's count and closing it is not closing a workspace. What makes it a
    /// login is the entry in `logins` — read when the pane ends, whose exit is the outcome.
    fn begin_harness_login(&mut self, client: ClientId, agent_type: String, account: String) {
        let refuse = |coordinator: &mut Self, error: String| {
            tracing::warn!(harness = %agent_type, account = %account, "sign-in refused: {error}");
            coordinator.host.send(
                To::Client(client),
                Message::HarnessLoginFailed {
                    agent_type: agent_type.clone(),
                    account: account.clone(),
                    error,
                },
            );
        };

        let pending = match self.agents.begin_home_login(&agent_type, &account) {
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
        pty::reap_noting(pane_id, child, mailbox.clone(), Some(pending.exit_slot()));
        self.panes.insert(pane_id, pane);
        self.logins.insert(pane_id, pending);

        tracing::info!(
            harness = %agent_type,
            account = %account,
            "sign-in started in pane {pane_id} for {client}"
        );
        mailbox.send(Message::HarnessLoginStarted {
            pane_id,
            agent_type,
            cols: INITIAL_COLS,
            rows: INITIAL_ROWS,
        });
    }

    /// A sign-in pane has ended: say whether it signed the account's home in. Nothing is
    /// captured or read back — the harness either finished its own login there, or it did not,
    /// and how the process exited is the answer.
    ///
    /// A clean exit is also the one thing that makes an account (`D194`): the record is written
    /// if it is not there yet, so the name the dialog took becomes an identity definitions can
    /// name. The refreshed list then goes to **every** window, because an identity is not this
    /// window's — the same rule the catalogue follows. A failed sign-in writes nothing.
    fn login_gone(&mut self, client: ClientId, pane_id: PaneId) {
        let Some(pending) = self.logins.remove(&pane_id) else {
            return;
        };
        let agent_type = pending.agent_type.clone();
        let account = pending.account.clone();
        match pending.exit_code() {
            Some(0) => {
                tracing::info!(harness = %agent_type, account = %account, "home signed in");
                if let Err(error) = self.agents.record_sign_in(&agent_type, &account) {
                    // The login is in the home either way; only Ubiq's note of it is missing, and
                    // saying so is more use than turning a finished sign-in into a failure.
                    tracing::warn!(
                        harness = %agent_type,
                        account = %account,
                        "the sign-in finished but could not be recorded: {error:#}"
                    );
                }
                self.host.send(
                    To::Client(client),
                    Message::HarnessHomeSignedIn {
                        agent_type,
                        account,
                    },
                );
                self.broadcast_accounts();
            }
            code => {
                let error = match code {
                    Some(code) => format!("the sign-in exited with code {code}"),
                    None => "the sign-in was closed before it finished".to_string(),
                };
                tracing::info!(harness = %agent_type, account = %account, "{error}");
                self.host.send(
                    To::Client(client),
                    Message::HarnessLoginFailed {
                        agent_type,
                        account,
                        error,
                    },
                );
            }
        }
    }

    /// Sign an account's home in by device code (`BeginDeviceLogin`): no pane — the harness's own
    /// sign-in hands over a page and a code, which go to the window, and then waits for the person
    /// to finish in a browser. All of that blocks on the network and on a person, so it runs on a
    /// thread of its own; the code goes straight to the asking window, and the outcome comes back
    /// through the host's voice to [`Self::device_login_ended`], which records it as
    /// [`Self::login_gone`] records a pane's clean exit.
    fn begin_device_login(&mut self, client: ClientId, agent_type: String, account: String) {
        let home = match self.agents.device_login_home(&agent_type, &account) {
            Ok(home) => home,
            Err(error) => {
                self.host.send(
                    To::Client(client),
                    Message::HarnessLoginFailed {
                        agent_type,
                        account,
                        error: format!("{error:#}"),
                    },
                );
                return;
            }
        };
        self.device_logins
            .insert((agent_type.clone(), account.clone()), client);
        let asker = self.host.mailbox(To::Client(client));
        let voice = self.host.voice();
        let started = thread::Builder::new()
            .name("ubiq-device-login".to_string())
            .spawn(move || {
                let outcome = agent_manager::harness::resolve(&agent_type)
                    .ok_or_else(|| anyhow::anyhow!("unknown agent type '{agent_type}'"))
                    .and_then(|harness| harness.begin_device_login(&home))
                    .and_then(|login| {
                        asker.send(Message::HarnessDeviceCode {
                            agent_type: agent_type.clone(),
                            account: account.clone(),
                            verification_url: login.verification_url.clone(),
                            user_code: login.user_code.clone(),
                        });
                        login.wait()
                    });
                voice.say(match outcome {
                    Ok(()) => Message::HarnessHomeSignedIn {
                        agent_type,
                        account,
                    },
                    Err(error) => Message::HarnessLoginFailed {
                        agent_type,
                        account,
                        error: format!("{error:#}"),
                    },
                });
            });
        if let Err(error) = started {
            tracing::warn!("no device sign-in thread: {error}");
        }
    }

    /// A device-code sign-in has finished, one way or the other: say so to the window that asked,
    /// and — when it worked — record the sign-in and tell every window the accounts changed.
    fn device_login_ended(&mut self, agent_type: String, account: String, error: Option<String>) {
        let Some(client) = self
            .device_logins
            .remove(&(agent_type.clone(), account.clone()))
        else {
            return;
        };
        match error {
            None => {
                tracing::info!(harness = %agent_type, account = %account, "home signed in by code");
                if let Err(error) = self.agents.record_sign_in(&agent_type, &account) {
                    tracing::warn!(
                        harness = %agent_type,
                        account = %account,
                        "the sign-in finished but could not be recorded: {error:#}"
                    );
                }
                self.host.send(
                    To::Client(client),
                    Message::HarnessHomeSignedIn {
                        agent_type,
                        account,
                    },
                );
                self.broadcast_accounts();
            }
            Some(error) => {
                tracing::info!(harness = %agent_type, account = %account, "{error}");
                self.host.send(
                    To::Client(client),
                    Message::HarnessLoginFailed {
                        agent_type,
                        account,
                        error,
                    },
                );
            }
        }
    }

    /// An account could not be renamed or deleted; say so to the window that asked, and nobody
    /// else — the same routing `refuse` in [`Self::begin_harness_login`] uses.
    fn account_error(&self, client: ClientId, error: String) {
        self.host
            .send(To::Client(client), Message::AccountError { error });
    }

    fn rename_account(&mut self, client: ClientId, account: String, new_account: String) {
        match self.agents.rename_account(&account, &new_account) {
            Ok(()) => self.send_accounts(client),
            Err(error) => self.account_error(client, format!("{error:#}")),
        }
    }

    fn delete_account(&mut self, client: ClientId, account: String) {
        match self.agents.delete_account(&account) {
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

    /// The folder a tool run starts in: its own `starting_folder` when the row names one and it
    /// still holds up, [`Self::resolve_cwd`]'s own resolution — the project's root — otherwise.
    /// An unset or blank `starting_folder` changes nothing about today's behaviour.
    ///
    /// A folder gone missing or unreadable answers `ToolError` rather than `refuse_spawn`'s
    /// `ProjectError`: the project itself is fine, it is the row's own setting that no longer
    /// resolves.
    fn resolve_tool_cwd(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        tool: &ToolDef,
    ) -> Option<PathBuf> {
        let Some(folder) = tool
            .starting_folder
            .as_deref()
            .filter(|folder| !folder.is_empty())
        else {
            return self.resolve_cwd(client, project_id, None);
        };
        let path = PathBuf::from(folder);
        let health = health::probe(&path);
        if !health.is_ok() {
            let reason = match &health {
                ProjectHealth::Missing => format!("{folder} is not there"),
                ProjectHealth::NotADirectory => format!("{folder} is not a folder"),
                ProjectHealth::Unreadable(reason) => reason.clone(),
                ProjectHealth::Ok => unreachable!("the branch above tested for this"),
            };
            self.tool_error(client, Some(project_id), reason);
            return None;
        }
        Some(path)
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
    // `a_launch_that_fails_keeps_the_agent_closable_and_retryable` in `tests/coordinator.rs`, which
    // forces a failure with a bad account rather than depend on one). `test_support::Idle`
    // supplies a `Conversation` whose pump is genuinely alive — blocked in `next_event` — without
    // a process behind it, which is all `unload_conversation`/`resume_conversation` ever look at.

    use crate::conversation::test_support::Idle;
    use crate::store::memory::{
        MemoryPreferenceStore, MemoryProjectStore, MemorySettingsStore, MemoryTaskStore,
    };
    use ubiq_proto::ask::AskOutcome;
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
        let coordinator = Coordinator::new(
            host,
            root,
            projects,
            work,
            settings,
            crate::tasksrc::Registry::with_defaults(),
            crate::runners::Registry::with_defaults(),
            pending,
        );
        let client = hub.connect();
        (coordinator, client)
    }

    #[test]
    fn a_pane_handle_and_a_conversation_name_share_one_namespace() {
        let (mut coordinator, client) = test_coordinator();
        let (_, project_id) = seed_live_conversation(&mut coordinator, &client, 0);
        // A harness pane in the same project, and one in another that must not count.
        let pane = PaneId::generate();
        coordinator.pane_projects.insert(pane, project_id);
        coordinator.pane_handles.insert(pane, "fake 2".to_string());
        let elsewhere = PaneId::generate();
        coordinator
            .pane_projects
            .insert(elsewhere, ProjectId::generate());
        coordinator
            .pane_handles
            .insert(elsewhere, "fake 3".to_string());

        let mut taken = coordinator.taken_names(project_id);
        taken.sort();
        assert_eq!(taken, vec!["fake".to_string(), "fake 2".to_string()]);
        assert_eq!(unique_name("fake", &taken), "fake 3");
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
        let (agent_id, project_id, _) =
            seed_live_conversation_writing(coordinator, client, last_seq);
        (agent_id, project_id)
    }

    /// The same, handing back the far end of the fake harness: what a test that has to prove
    /// something was *written to the agent* — a prompt out of an answered dialog — reads.
    fn seed_live_conversation_writing(
        coordinator: &mut Coordinator,
        client: &ubiq_proto::bus::Client,
        last_seq: u64,
    ) -> (
        AgentId,
        ProjectId,
        crate::conversation::test_support::Written,
    ) {
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
            title: None,
            definition: None,
            activity: Activity::Thinking,
            branch: String::new(),
            tokens: 0.0,
            harness: "Fake".to_string(),
            account: String::new(),
            model: String::new(),
            context_pct: 0,
            persistent: false,
            accept_all: false,
            debug_dump: None,
            run_dir: None,
            config_dir: None,
            thread: Vec::new(),
        };
        let session = WorkSession {
            id: session_id,
            name: "s".to_string(),
            branch: String::new(),
            worktree: false,
        };
        coordinator
            .work
            .lock()
            .add_live_agent(project_id, agent, session);
        coordinator
            .conversation_owners
            .insert(agent_id, (client.id(), project_id));
        coordinator.pending_conversations.insert(
            agent_id,
            PendingConversation {
                project_id,
                agent_type: "not-a-real-harness".to_string(),
                account: None,
                definition: None,
                cwd: std::path::PathBuf::from("."),
                chosen_model: None,
                chosen_thinking: None,
                chosen_mode: None,
                catalogue: Vec::new(),
                next_seq: last_seq,
                opening_prompt: None,
                named: false,
                harness_title: None,
                resume: None,
                fork: false,
                mcps: Vec::new(),
                skills: Vec::new(),
                extra_rw: Vec::new(),
            },
        );
        let mailbox = coordinator.host.mailbox(To::Client(client.id()));
        let bridge = Idle::new();
        let written = bridge.written();
        let conversation = Conversation::start(
            agent_id,
            Box::new(bridge),
            mailbox,
            last_seq,
            None,
            None,
            false,
            ConvFlags::new(agent_id, false, false),
            Some(crate::conversation::AskFire {
                armed: coordinator.armed.clone(),
                voice: coordinator.host.voice(),
            }),
        );
        coordinator.conversations.insert(agent_id, conversation);

        (agent_id, project_id, written)
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

    /// A project in the catalogue with `spec.md` in it, and its file document bound to `agent`.
    fn bound_spec(
        coordinator: &mut Coordinator,
        client: &ubiq_proto::bus::Client,
        agent: AgentId,
    ) -> ubiq_proto::plan::DocumentHandle {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::mem::forget(dir);
        std::fs::write(root.join("spec.md"), "# Spec\n\nOne.\n\nTwo.\n").unwrap();
        let replies = coordinator.projects.add(
            &root.to_string_lossy(),
            None,
            None,
            None,
            false,
            Default::default(),
        );
        let project_id = replies
            .iter()
            .find_map(|reply| match reply.message() {
                Message::ProjectAdded { project } => Some(project.record.id),
                _ => None,
            })
            .expect("the project was added");
        let doc = ubiq_proto::plan::DocumentHandle::File {
            project_id,
            rel_path: "spec.md".to_string(),
        };
        coordinator.dispatch(
            client.id(),
            Message::SetDocAgent {
                doc: doc.clone(),
                agent_id: Some(agent),
            },
        );
        doc
    }

    fn block_ids(
        coordinator: &Coordinator,
        doc: &ubiq_proto::plan::DocumentHandle,
    ) -> Vec<ubiq_proto::ids::BlockId> {
        let root = PathBuf::from(&coordinator.projects.record(doc.project_id()).unwrap().path);
        let target = crate::plan::Target::resolve(doc, &root).unwrap();
        let (blocks, _, _) = coordinator.plans.lock().annotation_list(&target).unwrap();
        blocks.into_iter().map(|block| block.id).collect()
    }

    fn doc_prompts(written: &crate::conversation::test_support::Written) -> Vec<String> {
        written
            .lock()
            .unwrap()
            .iter()
            .filter(|input| matches!(input, agent_manager::io::AgentInput::Prompt { .. }))
            .map(|input| format!("{input:?}"))
            .collect()
    }

    /// Auto-send off: an addressed comment and a reply queue nothing and reach nobody — the
    /// threads wait on the document — and the user's Ask agent then sends both in one prompt.
    #[test]
    fn with_auto_send_off_threads_wait_until_the_user_asks_and_go_in_one_prompt() {
        let (mut coordinator, client) = test_coordinator();
        let (agent, _, written) = seed_live_conversation_writing(&mut coordinator, &client, 0);
        let doc = bound_spec(&mut coordinator, &client, agent);
        let blocks = block_ids(&coordinator, &doc);
        let addressed = Some(ubiq_proto::work::Addressee::Agent);
        for (block, text) in [(blocks[1], "first point"), (blocks[2], "second point")] {
            coordinator.dispatch(
                client.id(),
                Message::AnnotatePlan {
                    doc: doc.clone(),
                    block_id: block,
                    quote: None,
                    text: text.to_string(),
                    marks: Vec::new(),
                    to: addressed,
                },
            );
        }
        coordinator.flush_doc_queue();
        assert!(!coordinator.doc_queue.has(agent), "nothing queued with auto-send off");
        assert!(doc_prompts(&written).is_empty(), "nothing delivered");

        coordinator.dispatch(
            client.id(),
            Message::AskDocAgent {
                doc: doc.clone(),
                annotation_ids: Vec::new(),
            },
        );
        let sent = doc_prompts(&written);
        assert_eq!(sent.len(), 1, "one coalesced prompt");
        assert!(sent[0].contains("first point") && sent[0].contains("second point"));
        assert!(!coordinator.doc_queue.has(agent));
    }

    /// An agent that is not running keeps what was asked of it, honestly reported, and gets it the
    /// moment its conversation is back up and idle — the launch `autostart_doc_agent` asks for.
    #[test]
    fn a_queue_for_an_agent_not_running_waits_and_flushes_once_it_is_up() {
        use ubiq_proto::plan::DocDelivery;
        let (mut coordinator, client) = test_coordinator();
        let (agent, _) = seed_live_conversation(&mut coordinator, &client, 0);
        let doc = bound_spec(&mut coordinator, &client, agent);
        let blocks = block_ids(&coordinator, &doc);
        coordinator.unload_conversation(client.id(), agent);
        drain_all(&client);

        coordinator.dispatch(
            client.id(),
            Message::AnnotatePlan {
                doc: doc.clone(),
                block_id: blocks[1],
                quote: None,
                text: "while you were away".to_string(),
                marks: Vec::new(),
                to: None,
            },
        );
        coordinator.dispatch(
            client.id(),
            Message::AskDocAgent {
                doc: doc.clone(),
                annotation_ids: Vec::new(),
            },
        );
        // The test harness is not one the host can launch, so the autostart leaves it down.
        assert!(coordinator.doc_queue.has(agent), "the entry waits");
        assert!(drain_all(&client).iter().any(|message| matches!(
            message,
            Message::DocAgentQueue { pending, state: DocDelivery::NotRunning, .. }
                if pending.len() == 1
        )));

        // What a successful launch leaves behind: a live, idle conversation.
        let bridge = crate::conversation::test_support::Idle::new();
        let written = bridge.written();
        let mailbox = coordinator.host.mailbox(To::Client(client.id()));
        let conversation = Conversation::start(
            agent,
            Box::new(bridge),
            mailbox,
            0,
            None,
            None,
            false,
            ConvFlags::new(agent, false, false),
            None,
        );
        coordinator.conversations.insert(agent, conversation);
        coordinator.flush_doc_queue();
        let sent = doc_prompts(&written);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("while you were away"));
    }

    /// The doc queue (`D207`) holds a comment while the agent is mid-turn, and the run loop's
    /// flush delivers it once the turn ends — composed from the sidecar as it reads then.
    #[test]
    fn a_queued_doc_comment_waits_for_the_turn_and_is_delivered_when_it_ends() {
        use ubiq_proto::plan::DocDelivery;
        let (mut coordinator, client) = test_coordinator();
        let (agent, project, written) =
            seed_live_conversation_writing(&mut coordinator, &client, 0);
        let task = {
            let mut work = coordinator.work.lock();
            let replies = work.create(project, "a mission".to_string(), None);
            let id = replies
                .iter()
                .find_map(|reply| match reply.message() {
                    Message::TaskCreated { task, .. } => Some(task.id),
                    _ => None,
                })
                .unwrap();
            work.set_field(
                project,
                id,
                ubiq_proto::messages::TaskField::Level(Some(ubiq_proto::work::Level::Mission)),
            );
            id
        };
        let target = crate::plan::Target::plan(project, task);
        let replies = {
            let mut plans = coordinator.plans.lock();
            plans.save(
                &target,
                "# Plan\n\nShip it.".to_string(),
                &crate::plan::Saver::human(),
                None,
            );
            let (blocks, _, _) = plans.annotation_list(&target).unwrap();
            plans.set_agent(&target, Some(agent));
            plans.annotate(
                &target,
                blocks[1].id,
                None,
                ubiq_proto::work::CommentAuthor::User,
                "first draft".to_string(),
                Vec::new(),
                Some(ubiq_proto::work::Addressee::Agent),
            )
        };
        let (_, queued) = crate::plan::queued_from(&replies, None).unwrap();
        let prompts = |written: &crate::conversation::test_support::Written| {
            written
                .lock()
                .unwrap()
                .iter()
                .filter(|input| matches!(input, agent_manager::io::AgentInput::Prompt { .. }))
                .map(|input| format!("{input:?}"))
                .collect::<Vec<_>>()
        };

        // The agent is mid-turn: the comment waits.
        coordinator.conversations[&agent]
            .prompt("the user's own turn".to_string())
            .unwrap();
        coordinator.doc_queue.push(agent, queued.clone());
        coordinator.deliver_doc_queue(agent, project);
        coordinator.flush_doc_queue();
        assert_eq!(prompts(&written).len(), 1, "nothing delivered mid-turn");
        let told = drain_all(&client);
        assert!(told.iter().any(|message| matches!(
            message,
            Message::DocAgentQueue { pending, state: DocDelivery::Waiting, .. } if pending.len() == 1
        )));

        // Edited while it waited, then the turn ends: the flush delivers it as it reads now.
        coordinator.plans.lock().edit_comment(
            &target,
            queued.annotation_id,
            queued.comment_ids[0],
            "final words".to_string(),
        );
        coordinator.conversations[&agent].end_turn_for_test();
        coordinator.flush_doc_queue();
        let sent = prompts(&written);
        assert_eq!(sent.len(), 2, "delivered once the turn ended");
        assert!(sent[1].contains("final words") && !sent[1].contains("first draft"));
        assert!(!coordinator.doc_queue.has(agent));
        assert!(
            coordinator.conversations[&agent].busy(),
            "the delivery opened a turn"
        );
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
                .lock()
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
            coordinator.pending_conversations.contains_key(&agent_id),
            "a failed launch keeps the recipe, so a Resume or the next prompt retries it (T-327)"
        );
        assert!(
            coordinator.drives(client.id(), agent_id),
            "and keeps the owner, so the window can still close it"
        );

        // Close after the failure is answered, not swallowed by `drives`.
        coordinator.dispatch(client.id(), Message::EndConversation { agent_id });
        assert!(
            drain_all(&client).iter().any(|message| matches!(
                message,
                Message::ConversationDeleted { agent_id: id } if *id == agent_id
            )),
            "an errored agent can be closed"
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
                .lock()
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

    /// The live record, as the last `AgentChanged` about `agent_id` carried it.
    fn last_changed(sent: &[Message], agent_id: AgentId) -> Option<WorkAgent> {
        sent.iter().rev().find_map(|message| match message {
            Message::AgentChanged { agent, .. } if agent.id == agent_id => Some((**agent).clone()),
            _ => None,
        })
    }

    /// T-283's B1: a rename lands on the live record's `title` — never on the handle — and so
    /// survives the next `AgentChanged` some unrelated flag sends; the MCP identity follows it.
    #[test]
    fn a_rename_is_on_the_live_record_and_survives_the_next_agent_changed() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project_id) = seed_live_conversation(&mut coordinator, &client, 0);
        coordinator.remember_conversation(agent_id);
        coordinator
            .agents
            .mcp_agents()
            .register(crate::mcp::AgentFacts {
                key: agent_id.to_string(),
                name: "fake".to_string(),
                ..Default::default()
            });
        while client.from_host().try_recv().is_ok() {}

        coordinator.dispatch(
            client.id(),
            Message::RenameConversation {
                agent_id,
                name: "Reviewer".to_string(),
            },
        );
        coordinator.dispatch(
            client.id(),
            Message::SetConversationAcceptAll {
                agent_id,
                accept_all: true,
            },
        );

        let sent: Vec<Message> =
            std::iter::from_fn(|| client.from_host().try_recv().ok()).collect();
        let agent = last_changed(&sent, agent_id).expect("an AgentChanged");
        assert!(agent.accept_all, "the later change is the one read");
        assert_eq!(agent.title.as_deref(), Some("Reviewer"));
        assert_eq!(agent.name, "fake", "the handle is never overwritten");
        assert_eq!(
            coordinator
                .agents
                .mcp_agents()
                .facts(&agent_id.to_string())
                .map(|facts| facts.name),
            Some("Reviewer".to_string()),
            "whoami says what the window says"
        );
        let row = conversation_record::load(&coordinator.sessions(), agent_id).expect("a row");
        assert_eq!(row.title.as_deref(), Some("Reviewer"));
    }

    /// A harness's own title lands on the live record like a naming does, and never over a
    /// rename.
    #[test]
    fn a_harness_title_is_adopted_until_the_user_renames() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, project_id) = seed_live_conversation(&mut coordinator, &client, 0);
        coordinator.remember_conversation(agent_id);
        let title_of = |coordinator: &Coordinator| {
            coordinator
                .work
                .lock()
                .live_agent_mut(project_id, agent_id)
                .and_then(|agent| agent.title.clone())
        };

        coordinator.conversations[&agent_id].offer_harness_title("Harness one");
        coordinator.adopt_harness_titles();
        assert_eq!(title_of(&coordinator).as_deref(), Some("Harness one"));

        coordinator.conversations[&agent_id].offer_harness_title("Harness two");
        coordinator.adopt_harness_titles();
        assert_eq!(
            title_of(&coordinator).as_deref(),
            Some("Harness two"),
            "the harness may retitle what it titled"
        );

        coordinator.dispatch(
            client.id(),
            Message::RenameConversation {
                agent_id,
                name: "Mine".to_string(),
            },
        );
        coordinator.conversations[&agent_id].offer_harness_title("Harness three");
        coordinator.adopt_harness_titles();
        assert_eq!(title_of(&coordinator).as_deref(), Some("Mine"));

        coordinator
            .conversations
            .remove(&agent_id)
            .unwrap()
            .stop(true);
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
        assert!(
            !pending.fork,
            "a re-attach continues its session, never forks it"
        );

        coordinator
            .conversations
            .remove(&agent_id)
            .unwrap()
            .stop(true);
    }

    /// T-283's B3: a revived conversation comes back under the title and summary its row kept,
    /// on the live record rather than only on disk — and, being named, is not named again.
    #[test]
    fn a_re_attach_restores_the_title_and_summary() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _seeded) = seed_live_conversation(&mut coordinator, &client, 0);
        let held = tempfile::TempDir::new().unwrap();
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
            pending.definition = Some("Reviewer".to_string());
        }
        coordinator.remember_conversation(agent_id);
        let sessions = coordinator.sessions();
        let mut row = conversation_record::load(&sessions, agent_id).expect("a row");
        row.title = Some("Fix tab truncation".to_string());
        row.summary = Some("Tabs cut their labels short".to_string());
        conversation_record::save(&sessions, agent_id, &row);

        coordinator.revive_conversation(
            client.id(),
            agent_id,
            agent_id,
            project_id,
            SessionId::generate(),
        );

        let said = drain_all(&client);
        let agent = last_changed(&said, agent_id).expect("an AgentChanged after the revive");
        assert_eq!(agent.title.as_deref(), Some("Fix tab truncation"));
        assert_eq!(
            agent.summary.as_deref(),
            Some("Tabs cut their labels short")
        );
        assert_eq!(agent.definition.as_deref(), Some("Reviewer"));
        assert_ne!(
            agent.name, "Fix tab truncation",
            "the handle stays the handle"
        );
        assert!(coordinator.pending_conversations[&agent_id].named);
        let row = conversation_record::load(&sessions, agent_id).expect("a row");
        assert_eq!(row.summary.as_deref(), Some("Tabs cut their labels short"));

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
                ubiq_proto::projects::StorageMode::UbiqManaged,
            )
            .into_iter()
            .find_map(|reply| match reply.into_message() {
                Message::ProjectAdded { project } => Some(project.id()),
                _ => None,
            })
            .expect("the project was added")
    }

    // ── account sign-ins ────────────────────────────────────────────────

    /// A sign-in into an account's home (`D194`) is judged by how it exited and nothing else: a
    /// failing code, or a pane closed before the login ended, is `HarnessLoginFailed` and writes
    /// nothing — no account appears for a sign-in that did not finish.
    #[test]
    fn a_sign_in_that_did_not_finish_records_no_account() {
        let (mut coordinator, client) = test_coordinator();
        for code in [Some(1), None] {
            let pane_id = PaneId::generate();
            coordinator.logins.insert(
                pane_id,
                PendingLogin::for_home_test("claude-code", "work", code),
            );

            coordinator.login_gone(client.id(), pane_id);

            let messages = drain_all(&client);
            assert_eq!(messages.len(), 1, "one outcome: {messages:?}");
            match &messages[0] {
                Message::HarnessLoginFailed {
                    agent_type,
                    account,
                    ..
                } => assert_eq!(
                    (agent_type.as_str(), account.as_str()),
                    ("claude-code", "work")
                ),
                other => panic!("unexpected outcome {other:?}"),
            }
        }
        assert!(coordinator.agents.accounts().unwrap().is_empty());
    }

    /// A clean exit is the one thing that makes an account (`D194`): the outcome goes to the
    /// window that asked, the record is written, and the refreshed list is broadcast so every
    /// window's accounts grow the new identity rather than the one that happened to sign it in.
    #[test]
    fn a_clean_sign_in_records_the_account_and_tells_every_window() {
        let (mut coordinator, client) = test_coordinator();
        let pane_id = PaneId::generate();
        coordinator.logins.insert(
            pane_id,
            PendingLogin::for_home_test("claude-code", "work", Some(0)),
        );

        coordinator.login_gone(client.id(), pane_id);

        let messages = drain_all(&client);
        assert_eq!(messages.len(), 2, "the outcome and the list: {messages:?}");
        match &messages[0] {
            Message::HarnessHomeSignedIn {
                agent_type,
                account,
            } => assert_eq!(
                (agent_type.as_str(), account.as_str()),
                ("claude-code", "work")
            ),
            other => panic!("unexpected outcome {other:?}"),
        }
        match &messages[1] {
            Message::Accounts { accounts } => {
                assert_eq!(accounts.len(), 1);
                assert_eq!(accounts[0].id, "work");
            }
            other => panic!("expected the refreshed accounts, got {other:?}"),
        }
        let recorded = coordinator.agents.accounts().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].id, "work");
    }

    /// A sign-in by code reports through the host's voice; the outcome reaches the window that
    /// asked and is recorded as a pane's clean exit is. An outcome nobody asked for is dropped.
    #[test]
    fn a_device_sign_in_reports_to_its_asker_and_records_the_account() {
        let (mut coordinator, client) = test_coordinator();
        coordinator.device_login_ended("codex".to_string(), "nobody".to_string(), None);
        assert!(drain_all(&client).is_empty(), "nobody asked for that one");

        coordinator
            .device_logins
            .insert(("codex".to_string(), "work".to_string()), client.id());
        coordinator.device_login_ended("codex".to_string(), "work".to_string(), None);

        let messages = drain_all(&client);
        assert!(
            matches!(&messages[..], [Message::HarnessHomeSignedIn { account, .. }, Message::Accounts { .. }] if account == "work"),
            "the outcome, then the list: {messages:?}"
        );
        assert_eq!(coordinator.agents.accounts().unwrap()[0].id, "work");
        assert!(coordinator.device_logins.is_empty());
    }

    /// Signing one harness out shortens the account's `logged_in` and leaves the identity: the
    /// answer is the refreshed list, which is what the settings screen redraws from.
    #[test]
    fn deleting_a_harness_login_shortens_logged_in_and_keeps_the_account() {
        let (mut coordinator, client) = test_coordinator();
        let harness = agent_manager::harness::resolve("claude-code").expect("claude-code");
        coordinator.agents.record_account("work").unwrap();
        let home = coordinator
            .agents
            .home_store()
            .home("work", &harness.id())
            .unwrap();
        let login = home.join(harness.login_files().first().expect("a login file"));
        std::fs::create_dir_all(login.parent().unwrap()).unwrap();
        std::fs::write(&login, b"{}").unwrap();
        assert_eq!(
            coordinator.agents.accounts().unwrap()[0].logged_in,
            ["claude-code"]
        );

        coordinator.dispatch(
            client.id(),
            Message::DeleteHarnessLogin {
                agent_type: "claude-code".to_string(),
                account: "work".to_string(),
            },
        );

        let messages = drain_all(&client);
        match messages.last() {
            Some(Message::Accounts { accounts }) => {
                assert_eq!(accounts.len(), 1, "the account survives");
                assert_eq!(accounts[0].id, "work");
                assert!(accounts[0].logged_in.is_empty(), "its login is gone");
            }
            other => panic!("expected the refreshed accounts, got {other:?}"),
        }
        assert!(!home.exists());
    }

    // ── the registered dialogs (`D175`) ─────────────────────────────

    /// One well-formed question, as `register_question` would have filed it.
    fn armed_question() -> ubiq_proto::ask::AskQuestion {
        ubiq_proto::ask::AskQuestion {
            question: "Which way?".to_string(),
            header: "Direction".to_string(),
            options: vec![
                ubiq_proto::ask::AskOption {
                    label: "Left".to_string(),
                    description: String::new(),
                    preview: None,
                },
                ubiq_proto::ask::AskOption {
                    label: "Right".to_string(),
                    description: String::new(),
                    preview: None,
                },
            ],
            multi_select: false,
        }
    }

    fn prompts(written: &crate::conversation::test_support::Written) -> Vec<String> {
        written
            .lock()
            .unwrap()
            .iter()
            .filter_map(|input| match input {
                agent_manager::io::AgentInput::Prompt { content } => Some(
                    content
                        .iter()
                        .map(|chunk| match chunk {
                            agent_manager::io::Content::Text { text } => text.clone(),
                            _ => String::new(),
                        })
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect()
    }

    /// The whole of the second ask mode's answer: a dialog raised at the turn boundary is
    /// answered, and what the user picked opens the next turn as a prompt — there is no parked
    /// call left to give it to.
    #[test]
    fn answering_a_registered_dialog_opens_the_next_turn() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project, written) =
            seed_live_conversation_writing(&mut coordinator, &client, 0);

        let ask_id = coordinator.armed.arm(agent_id, vec![armed_question()]);
        assert_eq!(coordinator.armed.fire(agent_id).len(), 1, "the turn ended");

        coordinator.dispatch(
            client.id(),
            Message::AnswerAsk {
                agent_id,
                ask_id,
                outcome: AskOutcome::Answered(vec![ubiq_proto::ask::AskAnswer {
                    question: 0,
                    chosen: vec!["Left".to_string()],
                    other: None,
                    notes: None,
                }]),
            },
        );

        let said = prompts(&written);
        assert_eq!(
            said.len(),
            1,
            "exactly one turn, out of the answer: {said:?}"
        );
        assert!(said[0].contains("Direction — Which way?"), "{}", said[0]);
        assert!(said[0].contains("- Answer: Left"), "{}", said[0]);
        assert!(coordinator.armed.is_empty(), "the row is answered and gone");

        coordinator
            .conversations
            .remove(&agent_id)
            .unwrap()
            .stop(true);
    }

    /// `G362`: two dialogs registered in one turn are raised together and answered one at a time,
    /// and the first answer must not open a turn while the second is still on screen. One prompt,
    /// carrying both, when the last of them is in.
    #[test]
    fn two_dialogs_registered_in_one_turn_open_one_turn_between_them() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project, written) =
            seed_live_conversation_writing(&mut coordinator, &client, 0);

        let first = coordinator.armed.arm(agent_id, vec![armed_question()]);
        let second = coordinator.armed.arm(agent_id, vec![armed_question()]);
        assert_eq!(coordinator.armed.fire(agent_id).len(), 2, "the turn ended");

        coordinator.dispatch(
            client.id(),
            Message::AnswerAsk {
                agent_id,
                ask_id: first,
                outcome: AskOutcome::Answered(vec![ubiq_proto::ask::AskAnswer {
                    question: 0,
                    chosen: vec!["Left".to_string()],
                    other: None,
                    notes: None,
                }]),
            },
        );
        assert!(
            prompts(&written).is_empty(),
            "the second dialog is still on screen",
        );

        coordinator.dispatch(
            client.id(),
            Message::AnswerAsk {
                agent_id,
                ask_id: second,
                outcome: AskOutcome::Answered(vec![ubiq_proto::ask::AskAnswer {
                    question: 0,
                    chosen: vec!["Right".to_string()],
                    other: None,
                    notes: None,
                }]),
            },
        );

        let said = prompts(&written);
        assert_eq!(said.len(), 1, "one turn, out of both answers: {said:?}");
        assert!(said[0].contains("- Answer: Left"), "{}", said[0]);
        assert!(said[0].contains("- Answer: Right"), "{}", said[0]);
        assert!(coordinator.armed.is_empty(), "both rows are gone");

        coordinator
            .conversations
            .remove(&agent_id)
            .unwrap()
            .stop(true);
    }

    /// The dismissal half of `G362`: a dialog the window gives up on settles its row rather than
    /// wedging the set, so the answers that were given still reach the agent.
    #[test]
    fn dismissing_one_of_a_registered_set_still_submits_the_rest() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project, written) =
            seed_live_conversation_writing(&mut coordinator, &client, 0);

        let first = coordinator.armed.arm(agent_id, vec![armed_question()]);
        let second = coordinator.armed.arm(agent_id, vec![armed_question()]);
        coordinator.armed.fire(agent_id);

        coordinator.dispatch(
            client.id(),
            Message::AnswerAsk {
                agent_id,
                ask_id: first,
                outcome: AskOutcome::Answered(vec![ubiq_proto::ask::AskAnswer {
                    question: 0,
                    chosen: vec!["Left".to_string()],
                    other: None,
                    notes: None,
                }]),
            },
        );
        assert!(prompts(&written).is_empty());

        // The window says it cannot draw the second one any more.
        coordinator.dispatch(
            client.id(),
            Message::AskEnded {
                agent_id,
                ask_id: second,
                why: AskClosed::Gone,
            },
        );

        let said = prompts(&written);
        assert_eq!(said.len(), 1, "the answer that was given: {said:?}");
        assert!(said[0].contains("- Answer: Left"), "{}", said[0]);
        assert!(coordinator.armed.is_empty());

        coordinator
            .conversations
            .remove(&agent_id)
            .unwrap()
            .stop(true);
    }

    /// "Chat about this" arms nothing and sends nothing: the user would rather type, and typing is
    /// what they do next.
    #[test]
    fn chatting_about_a_registered_dialog_sends_nothing() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project, written) =
            seed_live_conversation_writing(&mut coordinator, &client, 0);

        let ask_id = coordinator.armed.arm(agent_id, vec![armed_question()]);
        coordinator.armed.fire(agent_id);
        coordinator.dispatch(
            client.id(),
            Message::AnswerAsk {
                agent_id,
                ask_id,
                outcome: AskOutcome::Chat,
            },
        );

        assert!(prompts(&written).is_empty(), "nothing went to the harness");
        assert!(coordinator.armed.is_empty());

        coordinator
            .conversations
            .remove(&agent_id)
            .unwrap()
            .stop(true);
    }

    /// The user typed instead of answering. The dialog on screen stops offering an answer, so the
    /// same turn cannot be both answered and spoken to — and a later answer for it lands nowhere.
    #[test]
    fn a_prompt_closes_the_registered_dialog_on_screen() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project, written) =
            seed_live_conversation_writing(&mut coordinator, &client, 0);

        let ask_id = coordinator.armed.arm(agent_id, vec![armed_question()]);
        coordinator.armed.fire(agent_id);
        coordinator.dispatch(
            client.id(),
            Message::PromptAgent {
                agent_id,
                text: "never mind, do it the other way".to_string(),
            },
        );

        assert!(coordinator.armed.is_empty(), "the dialog is over");
        let closed = drain_all(&client);
        assert!(
            closed.iter().any(|message| matches!(
                message,
                Message::AskEnded { ask_id: id, .. } if *id == ask_id
            )),
            "the window is told the dialog stopped waiting: {closed:?}"
        );

        // And the answer that raced it changes nothing.
        coordinator.dispatch(
            client.id(),
            Message::AnswerAsk {
                agent_id,
                ask_id,
                outcome: AskOutcome::Answered(vec![ubiq_proto::ask::AskAnswer {
                    question: 0,
                    chosen: vec!["Left".to_string()],
                    other: None,
                    notes: None,
                }]),
            },
        );
        let said = prompts(&written);
        assert_eq!(said, vec!["never mind, do it the other way".to_string()]);

        coordinator
            .conversations
            .remove(&agent_id)
            .unwrap()
            .stop(true);
    }

    /// A conversation going takes its registrations with it, raised or not, and the window is told
    /// about the one it has on screen.
    #[test]
    fn unloading_a_conversation_closes_its_registered_dialogs() {
        let (mut coordinator, client) = test_coordinator();
        let (agent_id, _project) = seed_live_conversation(&mut coordinator, &client, 0);

        let raised = coordinator.armed.arm(agent_id, vec![armed_question()]);
        coordinator.armed.fire(agent_id);
        coordinator.armed.arm(agent_id, vec![armed_question()]);

        coordinator.unload_conversation(client.id(), agent_id);

        assert!(coordinator.armed.is_empty());
        let messages = drain_all(&client);
        assert!(
            messages.iter().any(|message| matches!(
                message,
                Message::AskEnded { ask_id, why: AskClosed::Gone, .. } if *ask_id == raised
            )),
            "only the dialog that was drawn is worth a message: {messages:?}"
        );
    }
}
