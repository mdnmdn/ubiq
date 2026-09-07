//! Application settings as they cross the bus: which half owns the schema, and the host's own record.
//!
//! View state is a different store — [`crate::projects::Scope`] and the preference messages. Settings
//! are how the application behaves. Two layers, because the host must never parse what it does
//! not own, and must parse what it does.

use serde::{Deserialize, Serialize};

use crate::assist::AssistProvider;
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
    /// Which backend answers a suggestion — a commit message the user asked Ubiq to write.
    ///
    /// [`AssistProvider::Off`] by default, and that default is today's behaviour: Ubiq calls no
    /// model at all, so an existing user's behaviour is unchanged by upgrading and nothing leaves
    /// the machine until they say so.
    #[serde(default)]
    pub assist: AssistProvider,
    /// Which `$HOME` a confined agent runs with.
    ///
    /// [`AgentHome::Inherit`] by default, and that is not a soft default: a replaced home aims
    /// every toolchain grant the policy carries — `~/.cargo`, `~/.npm`, `~/.dotnet` — at an
    /// empty directory, so the agent holds `cargo` in its `PATH` and cannot build. Inheriting
    /// opens nothing on its own; only the paths the policy names are reachable inside it.
    #[serde(default)]
    pub agent_home: AgentHome,
    /// Directories a confined agent may reach beyond what its policy already grants.
    ///
    /// The escape hatch for a toolchain installed somewhere the policy does not expect — a
    /// relocated `CARGO_HOME`, an SDK on another volume, a vendored dependency tree outside
    /// the project. Empty by default, because a default nobody can explain is worse than a
    /// denial somebody can fix.
    #[serde(default)]
    pub extra_grants: Vec<Grant>,
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

/// Which `$HOME` a confined agent runs with.
///
/// The three answers are "mine", "a fresh one" and "a named one I keep". Only the first works
/// with a toolchain out of the box; the other two are for a user who wants an agent kept away
/// from their own dotfiles and is willing to populate a home to get there.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentHome {
    /// The user's own home, unreplaced. Paths are still restricted to what the policy grants.
    #[default]
    Inherit,
    /// A scratch home per run, discarded with it. Nothing an agent leaves behind survives —
    /// and nothing it needs is there to begin with.
    Ephemeral,
    /// A home kept under Ubiq's own state, by name, so a second run finds what the first left.
    Named(String),
}

/// One directory a confined agent may reach, and whether it may write there.
///
/// A reference, like everything else on this record: a path the host resolves, never its
/// contents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// The directory, absolute or `~`-prefixed as the user typed it.
    pub path: String,
    /// Whether the agent may write there. Read-only is the safer half of the choice and the
    /// one a shared cache usually wants.
    #[serde(default)]
    pub write: bool,
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
///
/// Eight adds [`HostSettings::agent_home`] and [`HostSettings::extra_grants`], and earns the bump
/// for the same reason: an older build drops both on its next write, and the one it would drop
/// silently is the grant list a user added to make their toolchain reachable — a setting whose
/// absence shows up as a build failing inside an agent, nowhere near this file.
///
/// Nine adds [`HostSettings::assist`], and earns the bump on the same footing: an older build
/// reads the record fine — the field defaults to off — but drops it on its next write, silently
/// reverting a user's provider choice to calling no model at all.
pub const HOST_SETTINGS_SCHEMA: u32 = 9;

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
            assist: AssistProvider::default(),
            agent_home: AgentHome::default(),
            extra_grants: Vec::new(),
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
