//! Application settings as they cross the bus: which half owns the schema, and the host's own record.
//!
//! View state is a different store — [`crate::projects::Scope`] and the preference messages. Settings
//! are how the application behaves. Two layers, because the host must never parse what it does
//! not own, and must parse what it does.

use serde::{Deserialize, Serialize};

use crate::connectors::{Connection, OauthApp, TrustedCert};
use crate::projects::IndexLevel;

/// Which half owns the schema of a settings blob.
///
/// The Ui layer is opaque to the host: a string it writes down and hands back. The Host layer is
/// the host's to parse and act on. Harness definitions are neither — they belong to agent-manager.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingsLayer {
    /// Schema owned by the interface. The host stores the blob and never looks inside.
    Ui,
    /// Schema owned by the host. The host parses it; a blob it cannot read is an error, not a
    /// discarded default.
    Host,
}

/// The host-owned settings record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSettings {
    pub schema: u32,
    /// Whether an agent runs confined: a policy that grants its project's folder and its own
    /// throwaway configuration, and denies the rest of the machine.
    ///
    /// On, because an agent that edits files is exactly what a deny-by-default policy is for, and
    /// a default the user has to find is a default nobody has. Which harnesses opt out of it, and
    /// under which policy, belongs to the harness library rather than here — this is the one bit
    /// Ubiq owns, because Ubiq is what spawns the pane.
    #[serde(default = "isolate_agents_default")]
    pub isolate_agents: bool,
    /// Globs every project search and every filename index skip, whatever a project record says.
    #[serde(default = "search_excludes_default")]
    pub search_excludes: Vec<String>,
    /// External tools a search may fall back to, in the order they are tried, and only when the
    /// built-in walk could not answer. Empty means there is no fallback.
    #[serde(default = "search_fallbacks_default")]
    pub search_fallbacks: Vec<String>,
    /// How much of a project Ubiq indexes, for every project that does not say otherwise in its
    /// own record. `Light` by default: a content search that reads only the files that could
    /// match is what most projects want, and the symbol half costs a parse of every file.
    #[serde(default)]
    pub index_level: IndexLevel,
    /// The folder a clone lands in. `None` is the built-in default, which the host resolves — the
    /// contract does not name a path, and this one is no exception.
    #[serde(default)]
    pub projects_root: Option<String>,
    /// The folder an ephemeral clone lands in, and the only tree an ephemeral project may be
    /// deleted from. A second root rather than a flag on the first, because "may Ubiq remove this
    /// folder" is answered by where it is, not by what a record claims about it.
    #[serde(default)]
    pub ephemeral_root: Option<String>,

    /// The authenticated identities at external services — see [`crate::connectors`].
    ///
    /// **The host owns this field and the two below, and that is unlike everything above them.**
    /// They ride this record because it is already persisted, versioned and round-tripped, but the
    /// interface writes the whole blob back on `SetSettings`, and a flow completing while a
    /// settings dialog is open would otherwise be lost to that write. So the host discards whatever
    /// the interface sent for these three and keeps what is on disk. The rule is "the half that
    /// mutates a field owns it", and no other field here works that way.
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// The named OAuth application registrations Ubiq authenticates *as*, where one was registered
    /// rather than built in. Keyed by id: one provider and one instance may carry several.
    #[serde(default)]
    pub oauth_apps: Vec<OauthApp>,
    /// Certificates the user has vouched for, keyed by origin. A second list rather than a field on
    /// a connection, which is what makes a pin instance-wide for free: two connections to one
    /// server find the same row.
    #[serde(default)]
    pub trusted_certs: Vec<TrustedCert>,

    /// The remote hosts the interface knows how to reach, by name and address alone.
    ///
    /// **Deliberately not a token.** `Bus::register_remote`'s `Client` and the token that dials it
    /// live only in the window's own memory, never here — a bearer token is credential material
    /// exactly as `AGENTS.md` rules ("Accounts carry credential references, never credential
    /// material"), and this is a plaintext file on disk. The `connections`/`oauth_apps` precedent
    /// above keeps material off this record too, but by putting it in the OS-level `SecretStore`
    /// the harness library already has; a remote host's token has no such home to go to short of
    /// adding a keychain dependency this phase was told not to take on, so the honest answer is
    /// not to persist it at all. Reconnecting to a saved host asks for the token again, the same
    /// as the first dial did.
    ///
    /// **Why this field is UI-mutated, unlike the three above it.** Each of those exists because a
    /// background flow — a login polling a device code, a certificate confirmation — can complete
    /// while a settings dialog sits open with a stale copy, so `Settings::set` re-overwrites them
    /// from disk on every `SetSettings`. Nothing here runs unattended: a saved host is added or
    /// forgotten only by a person editing this exact list on this exact settings page, so there is
    /// no concurrent writer for a UI write to clobber, and this rides `SetSettings` whole like
    /// `search_excludes` or `projects_root` above it.
    #[serde(default)]
    pub remote_hosts: Vec<SavedRemoteHost>,
}

/// One remembered remote host: enough to offer a reconnect, never enough to perform one alone.
///
/// See [`HostSettings::remote_hosts`] for why the token is not here. Reconnecting from this record
/// means the interface still has to ask for one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRemoteHost {
    /// What the user called it when they saved it. Freely renamable, and shown instead of the
    /// address wherever there is room for only one string.
    pub name: String,
    /// `host:port`, exactly as typed or pasted — the same shape
    /// `state::remote::with_default_port` already normalises for a live dial.
    pub address: String,
}

/// The shape this host writes and understands.
///
/// A record from an older schema still parses — every field added since carries a default — and
/// only a *newer* one is refused, because that is the one this build cannot be trusted to read.
///
/// Six because an [`OauthApp`] gained an id and a name. A build that predates them reads such a
/// record without complaint and drops both on the next write, which would strand the client
/// secrets filed under those ids — so the refusal a newer schema earns is exactly what is wanted.
///
/// Seven adds [`HostSettings::remote_hosts`]. A build that predates it reads such a record fine —
/// the field defaults to empty — but would silently drop every saved host on its next write, which
/// is exactly the "an older build should not overwrite a newer field with nothing" case the schema
/// bump exists to prevent.
pub const HOST_SETTINGS_SCHEMA: u32 = 7;

fn isolate_agents_default() -> bool {
    true
}

/// Kept consistent with [`crate::files::WALK_SKIP`] and [`crate::files::LIST_HIDE`] — these are
/// globs for `ignore`'s `Override`, not bare name tests, so a leaf like `.gitkeep` still matches
/// wherever it sits.
fn search_excludes_default() -> Vec<String> {
    [
        "node_modules",
        ".git",
        "target",
        "dist",
        "build",
        ".venv",
        "__pycache__",
        ".cache",
        ".direnv",
        ".DS_Store",
        ".gitkeep",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn search_fallbacks_default() -> Vec<String> {
    ["ag", "grep"].into_iter().map(String::from).collect()
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            schema: HOST_SETTINGS_SCHEMA,
            isolate_agents: isolate_agents_default(),
            search_excludes: search_excludes_default(),
            search_fallbacks: search_fallbacks_default(),
            index_level: IndexLevel::default(),
            projects_root: None,
            ephemeral_root: None,
            connections: Vec::new(),
            oauth_apps: Vec::new(),
            trusted_certs: Vec::new(),
            remote_hosts: Vec::new(),
        }
    }
}
