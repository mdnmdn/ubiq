//! Application settings the interface owns: the schema, the overlay's nav, and how a blob is read.
//!
//! The host stores the Ui layer as an opaque string it never parses, so **the interface owns this
//! schema and versions it**. A blob that fails to parse, or that carries a schema this build does
//! not know, is discarded and the window opens on defaults.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::{AccountInfo, CliDir, LoginStatus};
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
    FileExplorer,
    Editor,
    Search,
    Harnesses,
    CommandLine,
}

impl SettingsSection {
    pub fn all() -> &'static [SettingsSection] {
        &[
            SettingsSection::FileExplorer,
            SettingsSection::Editor,
            SettingsSection::Search,
            SettingsSection::Harnesses,
            SettingsSection::CommandLine,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::FileExplorer => "File explorer",
            SettingsSection::Editor => "Editor",
            SettingsSection::Search => "Search",
            SettingsSection::Harnesses => "Harnesses",
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
    /// The layout a new markdown tab opens in.
    #[serde(default)]
    pub markdown_open: MarkdownOpen,
    /// Modal editing in the code editor and every multi-line box. Off is the default, and off
    /// means the interceptor in `app/vim.rs` returns before it looks at anything.
    #[serde(default)]
    pub vim_mode: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            explorer_preview: true,
            markdown_open: MarkdownOpen::Preview,
            vim_mode: false,
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
    /// The login modal, while one is up.
    pub login: Option<LoginState>,
    /// The rename, delete or sign-out question over one account, while one is up.
    pub dialog: Option<AccountDialog>,
    /// What `CheckHarnessLogin` last answered for a harness on an account, keyed
    /// `(agent_type, account)`. An absent entry means never checked, not `Missing` — those
    /// read differently. Pruned whenever `Accounts` arrives, so a renamed or deleted pair
    /// cannot linger here.
    pub statuses: HashMap<(String, String), LoginStatus>,
    /// The `ubiq` command's shortcut, as the host last reported it. Absent until it answers.
    pub cli: Option<CliShortcut>,
    /// What the host last refused — a rename, a delete, a sign-out. Cleared the next time the
    /// user acts: opens a dialog, starts a login, or dismisses it.
    pub error: Option<String>,
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
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            nav: SettingsSection::FileExplorer,
            ui: UiSettings::default(),
            host: HostSettings::default(),
            accounts: Vec::new(),
            login: None,
            dialog: None,
            statuses: HashMap::new(),
            cli: None,
            error: None,
        }
    }
}

/// Whole days, hours or minutes between two timestamps, worded singular or plural: `3 days`,
/// `1 hour`, `12 minutes`. Kept to whole units — a fractional one nobody reads precisely.
///
/// Private because the only caller is [`describe_status`]; a second use is what promotes this
/// to a shared helper.
fn magnitude(diff_ms: i64) -> String {
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
