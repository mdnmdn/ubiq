//! Application settings the interface owns: the schema, the overlay's nav, and how a blob is read.
//!
//! The host stores the Ui layer as an opaque string it never parses, so **the interface owns this
//! schema and versions it**. A blob that fails to parse, or that carries a schema this build does
//! not know, is discarded and the window opens on defaults.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use ubiq_proto::assist::{
    AiModelList, AiProviderInfo, AiProviderKind, AssistLimits, AssistReason, ModelRole,
};
use ubiq_proto::connectors::{AuthKind, CertInfo, ConnectError, OauthApp, ProviderId};
use ubiq_proto::ids::{AiProviderId, ConnectId, ConnectionId, OauthAppId, PaneId, SuggestId};
use ubiq_proto::messages::{AccountInfo, CliDir, LoginStatus, ProfileInfo};
use ubiq_proto::settings::HostSettings;

use crate::state::editor::ViewLayout;

/// A running login's own output has offered this many links without one being clicked or
/// copied yet. Capped so a misbehaving host cannot grow this state without bound — the host
/// already dedupes, this is the belt to its braces.
pub const MAX_LOGIN_LINKS: usize = 8;

/// The shape this build writes and understands. Bump it and older blobs are discarded.
pub const SCHEMA: u32 = 1;

/// The left nav of the application settings overlay, in the order it is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SettingsSection {
    #[default]
    Appearance,
    FileExplorer,
    Editor,
    Search,
    Harnesses,
    Isolation,
    Assist,
    Connectors,
    Hosts,
    CommandLine,
}

impl SettingsSection {
    pub fn all() -> &'static [SettingsSection] {
        &[
            SettingsSection::Appearance,
            SettingsSection::FileExplorer,
            SettingsSection::Editor,
            SettingsSection::Search,
            SettingsSection::Harnesses,
            SettingsSection::Isolation,
            SettingsSection::Assist,
            SettingsSection::Connectors,
            SettingsSection::Hosts,
            SettingsSection::CommandLine,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::Appearance => "Appearance",
            SettingsSection::FileExplorer => "File explorer",
            SettingsSection::Editor => "Editor",
            SettingsSection::Search => "Search",
            SettingsSection::Harnesses => "Harnesses",
            SettingsSection::Isolation => "Isolation",
            SettingsSection::Assist => "Assistance",
            SettingsSection::Connectors => "Connectors",
            SettingsSection::Hosts => "Hosts",
            SettingsSection::CommandLine => "Command line",
        }
    }
}

/// What the host last said about the `ubiq` command on the shell's `PATH`.
///
/// Every field is the host's answer and none of it is decided here: the interface draws what it
/// was told and sends one of three actions back. `None` on the state means nothing has answered
/// yet, which reads differently from "no shortcut installed".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliShortcut {
    pub installed: Option<String>,
    pub stale: bool,
    pub target: Option<String>,
    pub candidates: Vec<CliDir>,
    pub error: Option<String>,
}

/// What the host last said about assistance: whether it can run, and what is running.
///
/// Every field is the host's answer, `detail` included — the interface renders a reason and a
/// sentence it was given rather than composing either, so no platform or vendor is ever named
/// here. `None` on the state means nothing has answered yet, which reads differently from a
/// negative answer: the same rule `accounts` and `cli` above follow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssistInfo {
    pub available: bool,
    pub reason: Option<AssistReason>,
    pub detail: Option<String>,
    pub limits: Option<AssistLimits>,
}

impl AssistInfo {
    /// Whether the user may still pick a backend. A negative answer that is only the setting
    /// being off is the one the user can undo from this panel; anything else is the machine's
    /// answer, and offering a switch would offer a backend that is not there.
    pub fn switchable(&self) -> bool {
        self.available || self.reason == Some(AssistReason::DisabledBySetting)
    }
}

/// How a newly opened markdown file is drawn. Split is a per-tab choice, not a default.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkdownOpen {
    #[default]
    Preview,
    Source,
}

impl MarkdownOpen {
    pub fn all() -> [MarkdownOpen; 2] {
        [MarkdownOpen::Preview, MarkdownOpen::Source]
    }

    pub fn label(self) -> &'static str {
        match self {
            MarkdownOpen::Preview => "Preview",
            MarkdownOpen::Source => "Source",
        }
    }

    pub fn layout(self) -> ViewLayout {
        match self {
            MarkdownOpen::Preview => ViewLayout::Preview,
            MarkdownOpen::Source => ViewLayout::Source,
        }
    }
}

/// What the interface remembers about how it behaves, as opposed to where it was left.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiSettings {
    pub schema: u32,
    /// Single click and Enter open a temporary preview tab. Off, they open permanently.
    #[serde(default = "default_true")]
    pub explorer_preview: bool,
    /// The window-capture control in the titlebar, and its keystroke. Off removes both;
    /// the editor stays reachable by opening a PNG and by pasting one.
    #[serde(default = "default_true")]
    pub capture_enabled: bool,
    /// The open projects, as coloured badges at the bottom of the activity rail.
    #[serde(default = "default_true")]
    pub rail_projects: bool,
    /// The layout a new markdown tab opens in.
    #[serde(default)]
    pub markdown_open: MarkdownOpen,
    /// Modal editing in the code editor and every multi-line box. Off is the default, and off
    /// means the interceptor in `app/vim.rs` returns before it looks at anything.
    #[serde(default)]
    pub vim_mode: bool,
    /// A second ring beside a conversation footer's total-token readout, comparing the cached
    /// tokens to that total. Off by default: it is a reading only a user watching cache
    /// behaviour wants, and the one ring is what the footer says today.
    #[serde(default)]
    pub show_cache_ring: bool,
    /// Which connection the clone modal opened on last, as its id in text. A string rather than a
    /// `ConnectionId` because this blob is written by the interface and read back by a build that
    /// may no longer hold that connection — an id nothing matches is simply not preselected.
    #[serde(default)]
    pub last_connection: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            explorer_preview: true,
            capture_enabled: true,
            rail_projects: true,
            markdown_open: MarkdownOpen::Preview,
            vim_mode: false,
            show_cache_ring: false,
            last_connection: None,
        }
    }
}

/// Where a login has got to. A modal shows exactly one of these at a time, and the user can
/// leave any of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginStep {
    /// Pick a harness and name the identity. Nothing has been started, so leaving costs
    /// nothing.
    Choosing {
        /// The harness id picked, or none while the user has not chosen.
        agent_type: Option<String>,
    },
    /// `BeginHarnessLogin` is on its way to the host and nothing has answered yet — a first
    /// login past the picker, or a re-authentication that skips the picker entirely. Without
    /// this step the picker (or nothing at all) sat on screen until `HarnessLoginStarted`
    /// arrived, which read as the button having done nothing.
    Starting { agent_type: String },
    /// The harness's own login is running in this pane. Leaving abandons it, which is what
    /// the abort button does and is always safe — an unfinished login writes no credential.
    Running { pane: PaneId },
    /// It ended. `captured` is whether an account came of it; `message` says which harness,
    /// or why not.
    Done { captured: bool, message: String },
}

/// The login modal: which account is being logged in, and how far it has got.
#[derive(Clone, Debug)]
pub struct LoginState {
    /// The identity being logged in. Typed by the user before the flow starts, and kept
    /// afterwards so the outcome can name it.
    pub account: String,
    pub step: LoginStep,
    /// URLs the running login's own output has printed, oldest first and capped at
    /// [`MAX_LOGIN_LINKS`]. Offered as buttons below the terminal, because a terminal is a
    /// poor place to click text — the bytes themselves are untouched.
    pub links: Vec<String>,
    /// A shell in this login's sandbox rather than the harness's own flow — set from the
    /// `Shell` button, carried through so the modal's copy stays honest about what was asked
    /// for, and read by `PaneExited`'s handler: a probe's outcome is decided locally, since the
    /// host sends none for it.
    pub probe: bool,
    /// Whether the custom-command field is showing. Opened by the button, and opened on its own
    /// when the picked harness already has an override to show.
    pub command_open: bool,
    /// What `CheckAgentCommand` last answered: whether it ran, and the line to print under the
    /// field. Cleared whenever the harness changes, because the answer was about the old one.
    pub command_check: Option<(bool, String)>,
}

/// A question asked about one account, over the harnesses section. Only one is up at a time —
/// opening another replaces it, the same rule the login modal follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountDialog {
    /// Seeded with the current id; confirming sends `RenameAccount`.
    Rename { account: String },
    /// Deletes the account and every harness logged in under it. Confirming sends
    /// `DeleteAccount`.
    Delete { account: String },
    /// Signs one harness out, leaving the account and its other harnesses alone. Confirming
    /// sends `DeleteHarnessLogin`.
    SignOut { agent_type: String, account: String },
}

/// Where a connect flow has got to. One at a time, and the user can leave any of them —
/// leaving sends `CancelConnect`, and a flow that stored no token left nothing behind.
///
/// Mirrors [`ConnectStage`](ubiq_proto::connectors::ConnectStage) one for one, plus the two ends
/// the wire reports as their own messages and the picker the wire never sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectStep {
    /// Pick a provider and a flow, and name the identity. Nothing has been sent, so leaving
    /// costs nothing.
    Choosing {
        /// The provider picked, or none while the user has not chosen.
        provider: Option<ProviderId>,
        /// The flow picked, or none. Cleared whenever the provider changes, because the flows
        /// on offer are a property of the provider and the instance.
        auth: Option<AuthKind>,
    },
    /// `BeginConnect` is on its way and nothing has answered yet. Without this step the picker
    /// sat on screen until the first stage arrived, which read as the button having done nothing.
    Starting,
    /// [`ConnectStage::Opening`](ubiq_proto::connectors::ConnectStage::Opening).
    Opening,
    /// The user types `user_code` at `verification_url`. `expires_in` is seconds.
    DeviceCode {
        user_code: String,
        verification_url: String,
        expires_in: u64,
    },
    /// The browser has been sent to `url`, and the loopback listener is bound on `port`.
    AwaitingCallback { port: u16, url: String },
    /// Trading a code, or a pasted token, for an identity.
    Exchanging,
    /// Waiting for the user to paste something. `prompt` says what.
    NeedSecret { prompt: String },
    /// Stopped on a certificate; the confirmation modal is up, or on its way.
    AwaitingCertificate,
    /// The flow ended with nothing stored. Kept as a step rather than closing the modal, so the
    /// user can read why and retry without retyping.
    Failed { error: ConnectError },
}

/// Which application a connect flow is to authenticate as.
///
/// The one choice that replaces typing an instance URL, because a registration already knows where
/// it lives: picking one sets the instance and the client id together, and neither can then
/// disagree with the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectApp {
    /// The application this build ships. Offered only where the host says there is one.
    Bundled,
    /// One of the user's own registrations.
    Registration(OauthAppId),
    /// Neither — a base URL typed by hand. Kept because a pasted token needs no application at
    /// all, and refusing to reach a self-hosted install without one registered would be a
    /// regression rather than a rule.
    Instance,
}

/// The connect modal: which flow is running, and what it is for.
#[derive(Clone, Debug)]
pub struct ConnectState {
    /// Minted here, and the only thing that lets a late stage be told from the flow on screen —
    /// anything naming another id is discarded rather than drawn.
    pub connect_id: ConnectId,
    /// The user's name for the identity being made. Read out of the field when the flow starts
    /// and kept afterwards so the outcome can name it.
    pub label: String,
    pub provider: ProviderId,
    /// The base URL the identity lives at, as typed. `None` is the provider's public cloud.
    pub instance: Option<String>,
    /// Which application the flow authenticates as. Seeded whenever the provider is picked, so the
    /// modal never opens on a choice that provider does not offer.
    pub app: ConnectApp,
    /// Whether the application picker's list is down. Held here because a picker over a modal
    /// draws its own layer and the modal is redrawn from state on every frame.
    pub app_open: bool,
    pub step: ConnectStep,
}

/// The application-registration form, while it is up.
///
/// The four fields it draws are the connect modal's own inputs — the two modals occlude the
/// settings page that raises them, so neither can be on screen while the other is, and a fifth
/// buffer for the same four questions is a buffer that goes stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppForm {
    /// The registration being rewritten, or `None` for one being created. The host mints the id,
    /// so a form the user abandoned leaves nothing behind.
    pub id: Option<OauthAppId>,
    pub provider: ProviderId,
    /// Whether the provider picker's list is down.
    pub open: bool,
}

/// A client secret typed into the form for a registration that did not exist yet.
///
/// The id is the host's to mint, so there is nothing to file a secret under until the written
/// record comes back. This is what the interface holds in the meantime, matched against the next
/// settings broadcast by the two fields the user just typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSecret {
    pub name: String,
    pub client_id: String,
    pub secret: String,
}

impl PendingSecret {
    /// The registration this secret was meant for, in a list the host has since sent back.
    pub fn matches(&self, app: &OauthApp) -> bool {
        app.name == self.name && app.client_id == self.client_id
    }
}

/// A question asked about one connection or one pinned certificate, over the connectors section.
/// Only one is up at a time — opening another replaces it, the rule every dialog here follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectorDialog {
    /// Seeded with the current label; confirming sends `RenameConnection`.
    Rename {
        connection: ConnectionId,
        label: String,
    },
    /// Deletes the connection and its token. Confirming sends `DeleteConnection`; the pinned
    /// certificate is not touched, which is `ForgetCert`'s job.
    Disconnect {
        connection: ConnectionId,
        label: String,
    },
    /// Drops a pinned certificate. `uses` is how many connections live at that origin, so the
    /// question can say what else stops trusting it.
    ForgetCert { origin: String, uses: usize },
    /// Deletes a registration and the client secret filed under it. Confirming sends
    /// `DeleteOauthApp`; the connections made under it are left alone, since each holds its own
    /// client id.
    DeleteApp { app: OauthAppId, name: String },
}

/// A certificate a running flow stopped on, held until the user answers.
///
/// The `connect_id` is carried so the answer goes back to the flow that asked, and the `sha256`
/// sent back is the one out of `cert` — never one recomputed here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertPrompt {
    pub connect_id: ConnectId,
    pub origin: String,
    pub cert: CertInfo,
}

/// The API-provider form, while one is up.
///
/// It carries only what is chosen rather than what is typed: the name, the base URL, the key and
/// the two model ids are read out of their fields at save time, the way [`AppForm`] reads its
/// four. A picked model is typed too, in the end — the picker writes the id into the field it sits
/// beside — so there is nothing here to keep in step with it. **No key is ever held here**: the
/// field is the only place one exists, and it is cleared when the form closes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiProviderForm {
    /// The provider being rewritten, or `None` for one being added. The host mints the id, so an
    /// abandoned form leaves nothing behind — and a form with no id has no models to offer, which
    /// is why an add asks no model question at all.
    pub id: Option<AiProviderId>,
    pub kind: AiProviderKind,
    /// Which of the two model pickers has its list down, if either. Held here because a picker
    /// over a modal draws its own layer and the modal is redrawn from state on every frame — the
    /// same reason [`AppForm::open`] is a field.
    pub open: Option<ModelRole>,
}

/// The provider test, while its modal is up: which provider is being checked, and what the host
/// has said so far.
///
/// The answer arrives as chunks, so the modal shows a first token instead of a spinner — which is
/// the whole point of the subject. `suggest_id` is what tells a chunk for the run on screen from
/// one for a run the user has already restarted, and nothing naming another id is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiTest {
    pub provider_id: AiProviderId,
    /// The provider's name, kept so the title reads right even if the list is redrawn under it.
    pub name: String,
    /// Whether the provider configured a smart model at all. Kept so the Smart pill can be
    /// offered dead rather than dropped — a row that vanishes reads as gone, not as unset.
    pub has_smart: bool,
    pub role: ModelRole,
    /// The request in flight, or `None` before the first run and after one ends.
    pub suggest_id: Option<SuggestId>,
    /// What has arrived so far: every chunk concatenated, replaced by the final text when the
    /// whole answer lands. Cleared when a run starts, because a rerun is a new answer.
    pub answer: String,
    /// Whether the answer is complete. What turns the "waiting" note off for a backend that
    /// streamed nothing and simply answered.
    pub done: bool,
    /// Why the check failed, in the host's words. Chunks already drawn are discarded with it —
    /// a partial answer to a failed request is not an answer.
    pub error: Option<String>,
}

/// The settings overlay, and the values it is showing.
#[derive(Clone, Debug)]
pub struct SettingsState {
    pub open: bool,
    pub nav: SettingsSection,
    pub ui: UiSettings,
    /// The Host layer's own record. Owned and parsed by the host — this is only ever what the
    /// host last said it held, or the default while nothing has answered yet.
    pub host: HostSettings,
    /// The accounts the host holds. References only — an id and the harnesses it covers — and
    /// only ever what the host last said, like `host` above.
    pub accounts: Vec<AccountInfo>,
    /// The saved setups the host holds — a harness plus the identity, model and mode to start it
    /// with. References only, like `accounts`, and only ever what the host last said.
    pub profiles: Vec<ProfileInfo>,
    /// The profile form, while one is up. It carries the profile being edited; the id and model
    /// are read out of their fields at save time, the way the login modal reads its name.
    pub profile_form: Option<ProfileInfo>,
    /// The login modal, while one is up.
    pub login: Option<LoginState>,
    /// The rename, delete or sign-out question over one account, while one is up.
    pub dialog: Option<AccountDialog>,
    /// The providers this build ships an application for, as the host last said. Empty until it
    /// answers, which reads correctly: a flow offers a "Default" only where there is one.
    pub bundled: Vec<ProviderId>,
    /// The application-registration form, while one is up.
    pub app_form: Option<AppForm>,
    /// A secret waiting for the registration it belongs to to come back with an id.
    pub pending_secret: Option<PendingSecret>,
    /// The connect modal, while a flow is running. There is no list of connections beside it:
    /// those ride `host.connections`, which the host owns.
    pub connect: Option<ConnectState>,
    /// The rename, disconnect or forget-certificate question, while one is up.
    pub connector: Option<ConnectorDialog>,
    /// The certificate a flow is waiting on. Its own field rather than a `ConnectorDialog`,
    /// because it interrupts a running flow instead of being raised from the list.
    pub cert: Option<CertPrompt>,
    /// What `CheckHarnessLogin` last answered for a harness on an account, keyed
    /// `(agent_type, account)`. An absent entry means never checked, not `Missing` — those
    /// read differently. Pruned whenever `Accounts` arrives, so a renamed or deleted pair
    /// cannot linger here.
    pub statuses: HashMap<(String, String), LoginStatus>,
    /// What the host last said about one connection's token, keyed by id.
    ///
    /// A map rather than a field on the record, because the records themselves ride
    /// `host.connections` — which the host owns — and a status is an answer *about* a record
    /// rather than part of it. Absent means never asked, which reads differently from `Missing`.
    /// Whether an instance has a pinned certificate is not here at all: it is
    /// `host.trusted_certs` matched by origin, and deriving it keeps one truth.
    pub connection_status: HashMap<ConnectionId, LoginStatus>,
    /// The `ubiq` command's shortcut, as the host last reported it. Absent until it answers.
    pub cli: Option<CliShortcut>,
    /// What the host last said about assistance. Absent until it answers — see [`AssistInfo`].
    pub assist: Option<AssistInfo>,
    /// The configured API providers, as the host last said, each with whether a key is filed
    /// under it. Its own list rather than `host.ai_providers`, because that field is host-mutated
    /// — the interface changes providers through the four provider messages and never through
    /// `SetSettings` — and because `has_key` is an answer *about* a record rather than part of
    /// one, the same split `connection_status` makes.
    pub ai_providers: Vec<AiProviderInfo>,
    /// The model list each provider last answered with, keyed by provider. Absent means never
    /// listed, which the form says out loud rather than drawing an empty picker.
    pub ai_models: HashMap<AiProviderId, AiModelList>,
    /// The add-or-edit provider form, while one is up.
    pub ai_form: Option<AiProviderForm>,
    /// The provider test, while its modal is up.
    pub ai_test: Option<AiTest>,
    /// The provider a removal is being confirmed for. Its own field rather than a variant of
    /// [`ConnectorDialog`], because that dialog belongs to the connectors section and this
    /// question is raised from the assistance one.
    pub ai_remove: Option<AiProviderId>,
    /// Whether the Hosts section's dropdown list is down.
    pub host_picker_open: bool,
    /// Addresses a reconnect started from the Hosts section most recently failed to reach —
    /// see [`crate::app::hosts::host_menu_rows`]'s `failed` parameter. An address leaves this set
    /// the moment it attaches, so the dropdown never shows a stale failure for a host that is now
    /// up.
    pub failed_hosts: HashSet<String>,
    /// Reconnects still running for saved remote hosts, keyed by the keychain key
    /// (`host_secrets::key_for`). The socket dropped and the loop below owns the retries —
    /// manual `Reconnect` bumps the generation and dials at once, `Disconnect`/`Forget` removes
    /// the entry and every scheduled retry aborts on the missing key.
    pub reconnects: HashMap<String, ReconnectState>,
    /// What the host last refused — a rename, a delete, a sign-out. Cleared the next time the
    /// user acts: opens a dialog, starts a login, or dismisses it.
    pub error: Option<String>,
}

/// One saved host the interface is trying to get back to without being asked again.
///
/// Kept beside `failed_hosts` rather than on the `Bus`: the connection is gone, so there is no
/// `HostId` to key it by — only the saved entry it dials from. `generation` invalidates timers
/// a manual retry or a stop left behind: a scheduled wake for an older generation does nothing.
#[derive(Clone, Debug)]
pub struct ReconnectState {
    /// Attempts since the drop, starting at 0. The delay before the next one backs off from it.
    pub attempt: u32,
    /// What the last try said. Shown in the manager panel; empty before the first try.
    pub error: String,
    /// Bumped by every manual retry and every stop, so a timer scheduled earlier aborts instead
    /// of dialling twice.
    pub generation: u64,
}

impl SettingsState {
    /// The accounts that can run `agent_type`, for a picker that must not offer an identity
    /// which would start the harness logged out.
    pub fn accounts_for(&self, agent_type: &str) -> Vec<&AccountInfo> {
        self.accounts
            .iter()
            .filter(|account| account.logged_in.iter().any(|id| id == agent_type))
            .collect()
    }

    /// One configured provider by id, for a form, a test or a removal that holds only the id.
    /// Absent for a provider another window has since removed, which is what keeps a stale
    /// dialog from drawing a record that no longer exists.
    pub fn ai_provider(&self, id: AiProviderId) -> Option<&AiProviderInfo> {
        self.ai_providers.iter().find(|info| info.provider.id == id)
    }
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            nav: SettingsSection::Appearance,
            ui: UiSettings::default(),
            host: HostSettings::default(),
            accounts: Vec::new(),
            profiles: Vec::new(),
            profile_form: None,
            bundled: Vec::new(),
            app_form: None,
            pending_secret: None,
            login: None,
            dialog: None,
            connect: None,
            connector: None,
            cert: None,
            statuses: HashMap::new(),
            connection_status: HashMap::new(),
            cli: None,
            assist: None,
            ai_providers: Vec::new(),
            ai_models: HashMap::new(),
            ai_form: None,
            ai_test: None,
            ai_remove: None,
            host_picker_open: false,
            failed_hosts: HashSet::new(),
            reconnects: HashMap::new(),
            error: None,
        }
    }
}

/// Whole days, hours or minutes between two timestamps, worded singular or plural: `3 days`,
/// `1 hour`, `12 minutes`. Kept to whole units — a fractional one nobody reads precisely.
///
/// Shared rather than private now that there is a second caller: [`describe_status`] words a
/// token's expiry with it and the provider form words a model list's age with it, and two callers
/// wording a duration two ways would be two vocabularies for one fact.
pub fn magnitude(diff_ms: i64) -> String {
    let diff_ms = diff_ms.abs();
    let minutes = diff_ms / 60_000;
    let hours = minutes / 60;
    let days = hours / 24;

    if days >= 1 {
        format!("{days} day{}", if days == 1 { "" } else { "s" })
    } else if hours >= 1 {
        format!("{hours} hour{}", if hours == 1 { "" } else { "s" })
    } else if minutes >= 1 {
        format!("{minutes} minute{}", if minutes == 1 { "" } else { "s" })
    } else {
        "a moment".to_string()
    }
}

/// How a [`LoginStatus`] reads in the accounts section, at `now_ms`.
pub fn describe_status(status: &LoginStatus, now_ms: i64) -> String {
    match status {
        LoginStatus::Valid { expires_at_ms } => {
            format!(
                "valid \u{b7} expires in {}",
                magnitude(expires_at_ms - now_ms)
            )
        }
        LoginStatus::Expired { expires_at_ms } => {
            format!("expired {} ago", magnitude(now_ms - expires_at_ms))
        }
        LoginStatus::Unknown => "signed in \u{b7} no expiry recorded".to_string(),
        LoginStatus::Missing => "no credential stored".to_string(),
    }
}

/// How a [`ConnectError`] reads in the connect modal.
///
/// Here rather than in the drawing code so the wording is one testable place, the same reason
/// [`describe_status`] is. Every sentence says what happened, and names what the user can do
/// about it where there is anything.
pub fn connect_error_note(error: &ConnectError) -> String {
    match error {
        ConnectError::Denied => {
            "The provider refused, or the sign-in was declined in the browser.".to_string()
        }
        ConnectError::Expired => {
            "The code ran out before it was used. Starting again issues a new one.".to_string()
        }
        ConnectError::PortBusy => {
            "Port 47821 is in use, and it is the only port a browser flow can come back on — \
             close whatever holds it, or use a token instead."
                .to_string()
        }
        ConnectError::StateMismatch => {
            "The answer did not match what was sent. Nothing was exchanged and nothing stored; \
             start again."
                .to_string()
        }
        ConnectError::NoSecureStore => {
            "Connectors need a secure credential store, and this machine has none available — \
             Ubiq never writes a token to a plaintext file."
                .to_string()
        }
        ConnectError::BadInstance => {
            "That address is not a URL, or is not this product. Give the base URL of the \
             install, scheme included."
                .to_string()
        }
        ConnectError::NoApplication => {
            "No application is registered for this provider, so there is no browser flow to \
             open. Configure one below, or use a token."
                .to_string()
        }
        ConnectError::Tls(detail) => format!("The connection itself failed: {detail}"),
        ConnectError::Http(detail) => format!("The provider answered: {detail}"),
    }
}

/// Read a blob back, or nothing at all.
///
/// The schema is probed before anything else is trusted, so a blob from a newer build is discarded
/// whole rather than half-applied.
pub fn decode(blob: &str) -> Option<UiSettings> {
    #[derive(Deserialize)]
    struct JustTheSchema {
        schema: u32,
    }

    match serde_json::from_str::<JustTheSchema>(blob) {
        Ok(JustTheSchema { schema }) if schema == SCHEMA => {}
        Ok(JustTheSchema { schema }) => {
            tracing::debug!("discarding ui settings written for schema {schema}");
            return None;
        }
        Err(error) => {
            tracing::debug!("discarding unreadable ui settings: {error}");
            return None;
        }
    }

    serde_json::from_str(blob)
        .inspect_err(|error| tracing::debug!("discarding ui settings: {error}"))
        .ok()
}

/// Write one out. Infallible in practice; an unserialisable value becomes an empty blob, which
/// decodes to nothing and opens on defaults.
pub fn encode(value: &UiSettings) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;
    const MINUTE: i64 = 60_000;

    #[test]
    fn magnitude_days_plural_and_singular() {
        assert_eq!(magnitude(3 * DAY), "3 days");
        assert_eq!(magnitude(DAY), "1 day");
    }

    #[test]
    fn magnitude_hours_plural_and_singular() {
        assert_eq!(magnitude(4 * HOUR), "4 hours");
        assert_eq!(magnitude(HOUR), "1 hour");
    }

    #[test]
    fn magnitude_minutes_plural_and_singular() {
        assert_eq!(magnitude(12 * MINUTE), "12 minutes");
        assert_eq!(magnitude(MINUTE), "1 minute");
    }

    #[test]
    fn magnitude_ignores_sign() {
        assert_eq!(magnitude(-2 * DAY), "2 days");
    }

    #[test]
    fn magnitude_under_a_minute() {
        assert_eq!(magnitude(0), "a moment");
    }

    #[test]
    fn describe_status_valid_future() {
        assert_eq!(
            describe_status(
                &LoginStatus::Valid {
                    expires_at_ms: 3 * DAY
                },
                0
            ),
            "valid \u{b7} expires in 3 days"
        );
    }

    #[test]
    fn describe_status_expired_past() {
        assert_eq!(
            describe_status(
                &LoginStatus::Expired {
                    expires_at_ms: -2 * DAY
                },
                0
            ),
            "expired 2 days ago"
        );
    }

    #[test]
    fn describe_status_unknown_and_missing() {
        assert_eq!(
            describe_status(&LoginStatus::Unknown, 0),
            "signed in \u{b7} no expiry recorded"
        );
        assert_eq!(
            describe_status(&LoginStatus::Missing, 0),
            "no credential stored"
        );
    }
}
