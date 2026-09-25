//! Application settings as they cross the bus: which half owns the schema, and the host's own record.
//!
//! View state is a different store — [`crate::projects::Scope`] and the preference messages. Settings
//! are how the application behaves. Two layers, because the host must never parse what it does
//! not own, and must parse what it does.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::assist::{AiProvider, AssistProvider};
use crate::connectors::{Connection, OauthApp, TrustedCert};
use crate::ids::SshProfileId;
use crate::projects::IndexLevel;
use crate::tools::ToolDef;

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
    /// Whether a conversation names itself once its agent has answered the opening prompt.
    ///
    /// On by default, and that default changes nothing on its own: naming runs through
    /// [`Self::assist`], which is [`AssistProvider::Off`] until a user picks a provider, so a host
    /// with no provider configured names nothing however this reads. It is a separate setting
    /// because the two questions are separate — a user may want a commit message written by hand
    /// on request and still not want every conversation to cost a call it did not ask for.
    #[serde(default = "auto_name_conversations_default")]
    pub auto_name_conversations: bool,
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
    /// What to run for a harness, when the harness's own name is not what to run. Keyed by
    /// harness id; the value is a command line — a bare name (`claudex`), an absolute path
    /// (`/opt/bin/claude`, `C:\tools\claude.exe`) or a launcher and its arguments
    /// (`mise exec -- opencode`).
    ///
    /// **This is the one launch fact Ubiq owns**, and it is here rather than in the harness
    /// library because it is a property of this machine, not of the harness: the library says
    /// what a harness is called, the user says where this machine keeps it. The host splits the
    /// string, uses the first word as the program and puts the rest in front of the arguments
    /// the library composed. An id with no entry resolves exactly as before.
    #[serde(default)]
    pub agent_commands: BTreeMap<String, String>,
    /// Runnable tools defined for this machine — named commands the new-pane menu offers and
    /// [`Message::RunTool`] runs in a project's folder. Rides `SetSettings` whole like
    /// `search_excludes` above: nothing runs unattended from this list, so there is no
    /// concurrent writer for a UI write to clobber.
    ///
    /// [`Message::RunTool`]: crate::messages::Message::RunTool
    #[serde(default)]
    pub tools: Vec<ToolDef>,
    /// Globs every project search and every filename index skip, whatever a project record says.
    /// How many days a conversation nobody marked persistent is kept before its record is
    /// collected. Thirty by default — long enough that last month's work is still there to go
    /// back to, short enough that the store is not a transcript of the year.
    ///
    /// `0` never collects, and that is a real answer rather than a degenerate one: a user who
    /// wants everything kept says so with a number, not by finding a switch somewhere else.
    ///
    /// A *persistent* conversation is exempt entirely, whatever this reads. Marking one is the
    /// user saying keep it, and a timer must not overrule that — such a conversation goes only by
    /// an explicit unmark or delete.
    #[serde(default = "retain_conversations_days_default")]
    pub retain_conversations_days: u32,
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
    /// The display word for a task at [`crate::work::Level::Mission`] — *mission*, *epic*, *user
    /// story*, whatever the user calls it. Display text alone: nothing but the interface reads it,
    /// and a project may override it in its own [`crate::projects::ProjectRecord::mission_term`].
    #[serde(default = "mission_term_default")]
    pub mission_term: String,
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
    /// **The host owns this field and the three below, and that is unlike everything above them.**
    /// They ride this record because it is already persisted, versioned and round-tripped, but the
    /// interface writes the whole blob back on `SetSettings`, and a flow completing while a
    /// settings dialog is open would otherwise be lost to that write. So the host discards whatever
    /// the interface sent for these four and keeps what is on disk. The rule is "the half that
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
    /// The API providers assistance may be pointed at — see [`crate::assist`].
    ///
    /// Host-mutated for the same reason the three above are, and for one more of its own: a
    /// provider's key lives in the OS secret store under the record's id, so a record the
    /// interface wrote directly could name a key that was never filed. Every change comes in as
    /// `AddAiProvider`, `UpdateAiProvider` or `ForgetAiProvider`, which is what keeps a row and
    /// its key inseparable. **No key is ever on this record**, and none is ever in this file.
    #[serde(default)]
    pub ai_providers: Vec<AiProvider>,

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
    /// **Why this field is UI-mutated, unlike the four above it.** Each of those exists because a
    /// background flow — a login polling a device code, a certificate confirmation, a provider's
    /// key being filed — can complete while a settings dialog sits open with a stale copy, so
    /// `Settings::set` re-overwrites them from disk on every `SetSettings`. Nothing here runs
    /// unattended: a saved host is added or
    /// forgotten only by a person editing this exact list on this exact settings page, so there is
    /// no concurrent writer for a UI write to clobber, and this rides `SetSettings` whole like
    /// `search_excludes` or `projects_root` above it.
    #[serde(default)]
    pub remote_hosts: Vec<SavedRemoteHost>,

    /// The SSH targets the interface knows how to dial, by address and auth method alone.
    ///
    /// Three things want this and none of them is the drone alone: a drone's carrier, a remote
    /// `ubiq-host` reached over a tunnel, and an ssh clone's credential callback. It is a list of
    /// *references* — a key's path rides the record, a passphrase never does, and the `has_*`
    /// flags on [`SshAuth`] are re-derived from the secret store on every write rather than
    /// believed from the blob that arrives.
    ///
    /// **UI-mutated, like [`Self::remote_hosts`] above and unlike [`Self::ai_providers`].**
    /// Nothing writes a profile unattended: a profile is added, edited or forgotten only by a
    /// person on this exact settings page, so there is no concurrent writer for a UI write to
    /// clobber and this rides `SetSettings` whole. What the host still owns is the *material* —
    /// it arrives only in a [`crate::messages::Secret`], and a profile that leaves this list has
    /// its secret pruned with it, so no key is ever stranded under an id nothing names.
    #[serde(default)]
    pub ssh_profiles: Vec<SshProfile>,
}

/// One SSH target: enough to dial it, never enough to unlock it.
///
/// The passphrase or password lives in the OS secret store under the profile's `id`, never in
/// this file — the rule `AGENTS.md` states as "accounts carry credential references, never
/// credential material", and this is a plaintext file on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshProfile {
    /// Stable id, minted UI-side when the row is added. The name and the address are the user's
    /// and change; this is what the secret store references.
    pub id: SshProfileId,
    /// What the user called it. Freely renamable, and shown instead of the address wherever
    /// there is room for only one string.
    pub name: String,
    /// The hostname or address to dial. For [`SshAuth::ConfigAlias`] this is the alias in the
    /// user's `~/.ssh/config`, and the three fields below are left to that file.
    pub host: String,
    /// The port, `22` unless the user said otherwise. Ignored for [`SshAuth::ConfigAlias`].
    #[serde(default = "ssh_port_default")]
    pub port: u16,
    /// The remote user. Empty means "whatever `ssh` would pick" — the local username, or what
    /// the config file says.
    #[serde(default)]
    pub user: String,
    /// How the connection authenticates.
    #[serde(default)]
    pub auth: SshAuth,
}

/// How an [`SshProfile`] authenticates.
///
/// Each variant's `has_*` flag is written by the host from the secret store, not by the
/// interface: the interface is never sent the material, so it cannot be the half that says
/// whether there is any.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SshAuth {
    /// The running `ssh-agent`, which is what an already-working setup usually is. Nothing to
    /// store and nothing to ask for.
    #[default]
    Agent,
    /// A private key on this machine. The path is a reference and rides the record; the
    /// passphrase, if the key has one, goes to the secret store.
    KeyFile {
        /// The key file, absolute or `~`-prefixed as the user typed it.
        path: String,
        /// Whether a passphrase is filed for it. Host-derived.
        #[serde(default)]
        has_passphrase: bool,
    },
    /// A password. Never on this record, and never in `argv` — OpenSSH reads one from
    /// `/dev/tty`, so it reaches `ssh` through an askpass helper or not at all.
    Password {
        /// Whether a password is filed. Host-derived.
        #[serde(default)]
        has_password: bool,
    },
    /// Defer wholly to the user's `~/.ssh/config`: `host` is an alias there and every other
    /// field on the record is ignored. The escape hatch for a setup Ubiq should not try to
    /// re-describe — jump hosts, certificates, per-host identity files.
    ConfigAlias,
}

impl SshAuth {
    /// Whether this variant has a secret filed for it, whichever one it is.
    pub fn has_secret(&self) -> bool {
        match self {
            Self::KeyFile { has_passphrase, .. } => *has_passphrase,
            Self::Password { has_password } => *has_password,
            Self::Agent | Self::ConfigAlias => false,
        }
    }

    /// Whether this variant can have a secret at all. An [`Self::Agent`] definition with a
    /// passphrase filed against it is a leak, not a feature, so the host prunes one.
    pub fn takes_secret(&self) -> bool {
        matches!(self, Self::KeyFile { .. } | Self::Password { .. })
    }

    /// Restamp the host-derived flag from what the secret store actually holds.
    pub fn set_has_secret(&mut self, filed: bool) {
        match self {
            Self::KeyFile { has_passphrase, .. } => *has_passphrase = filed,
            Self::Password { has_password } => *has_password = filed,
            Self::Agent | Self::ConfigAlias => {}
        }
    }
}

fn ssh_port_default() -> u16 {
    22
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
/// The bearer token lives in the OS keychain under the entry's `id`, never in this file.
/// Reconnecting from this record means the interface still has to unlock one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRemoteHost {
    /// Stable id, minted UI-side on first save. The name and address are the user's and change;
    /// this is what the secret store and the live-connection table reference.
    #[serde(default)]
    pub id: String,
    /// What the user called it. Freely renamable, and shown instead of the
    /// address wherever there is room for only one string.
    pub name: String,
    /// `host:port`, exactly as typed or pasted — the same shape
    /// `state::remote::with_default_port` already normalises for a live dial.
    pub address: String,
    /// `http` (plaintext upgrade) or `https` (TLS). Defaults to `http` for records
    /// written before the scheme was captured.
    #[serde(default)]
    pub scheme: RemoteScheme,
    /// Only meaningful for `https`: skip chain validation. The panel shows it as
    /// untrusted. Auto-reconnect does not apply this flag, so a later MITM is not
    /// accepted without a click.
    #[serde(default)]
    pub trust_insecure: bool,
    /// What carries the frames to this host: a socket Ubiq dials, or an `ssh` it spawns.
    ///
    /// The discriminant rather than a second list, because everything around a saved host —
    /// the Hosts section, the picker, Disconnect, the live-connection table — is about a host
    /// and not about how its bytes arrive (`D116`). Defaults to [`RemoteCarrier::Socket`], which
    /// is what every record written before a drone existed is.
    #[serde(default)]
    pub carrier: RemoteCarrier,
}

/// How a saved remote host's frames are carried.
///
/// Both ends speak the same length-prefixed MessagePack; what differs is what the bytes travel
/// over and what failing to reach the far end looks like.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum RemoteCarrier {
    /// A TCP socket Ubiq dials, with [`SavedRemoteHost::scheme`] deciding whether TLS wraps it
    /// and [`SavedRemoteHost::address`] naming where. What a `ubiq --serve` host is.
    #[default]
    Socket,
    /// An `ssh` Ubiq spawns, whose standard input and output are the stream. The address and
    /// scheme are unused; the profile says where to connect and the root says which folder the
    /// drone serves.
    Ssh {
        /// The [`SshProfile`] to dial with. A record naming a profile the user has since
        /// deleted cannot connect, and says so rather than falling back to another.
        profile: SshProfileId,
        /// The folder on the far machine the drone is launched against, as the user typed it.
        /// Empty means the drone's own default — the login directory.
        #[serde(default)]
        root: String,
        /// Whether the drone detaches, and for how long it survives a dropped link. Defaults to
        /// [`DronePreset::Attached`], which is the only shape a record from before phase 7 could
        /// have meant.
        #[serde(default)]
        preset: DronePreset,
        /// Where the drone binary sits on the far machine, if it is not on the remote `PATH` —
        /// either typed once by hand, or the cache path phase 9's deployer wrote back after it
        /// last uploaded one. `remote_command` runs this in place of the bare `ubiq-drone` when
        /// set. `None` is a bare `PATH` lookup, which is every record from before a deployer
        /// existed.
        #[serde(default)]
        drone_path: Option<String>,
    },
}

/// The drone's lifetime, picked once on the connect path and carried on the saved host.
///
/// Two axes — does the drone detach, and what `--linger` does it launch with — collapse to three
/// presets rather than staying two knobs, because the useful combinations are exactly these three
/// and the fourth (detach with `--linger 0`) is just [`Self::Attached`] with an extra hop: it would
/// still die the moment this window's `ssh` line drops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DronePreset {
    /// `--stdio`, no socket: the drone is this session's `ssh` and nothing survives it. What the
    /// interface has always done, and still the right shape for a one-off look at a machine.
    #[default]
    Attached,
    /// Detaches with the default ten-minute linger. A dropped link — a laptop's Wi-Fi, a closed
    /// lid — is outlived; Ubiq quitting for the day is not, so the drone still cleans up after
    /// itself rather than becoming a thing the user has to remember is running.
    Session,
    /// Detaches with `--linger never`: a managed drone, meant to survive this Ubiq closing
    /// entirely. `--list` and the Drones settings section are how it is found again, and `--stop`
    /// is the only thing that ends it short of the far machine going down.
    Managed,
}

/// Which protocol a saved remote host dials with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteScheme {
    /// Plaintext TCP + HTTP upgrade. What the host has always spoken.
    #[default]
    Http,
    /// TLS first, then the same HTTP upgrade inside it. Needs a server cert on the host.
    Https,
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
///
/// Ten adds [`HostSettings::ai_providers`] and gives [`AssistProvider`] its `Api` variant. This is
/// the strongest case in the list: an older build drops the provider records on its next write and
/// **strands their keys in the OS secret store**, filed under ids nothing on disk names any more —
/// a leak of exactly the kind a keychain exists to prevent. It also cannot read `assist` at all
/// when the variant is `Api`, which by the rule above is a refusal rather than a silent default.
///
/// Eleven adds [`HostSettings::agent_commands`]. An older build drops the overrides on its next
/// write, and every harness they pointed at goes back to being looked up by its own name — a
/// harness that is only reachable through one stops starting until it is set again.
///
/// Twelve adds [`HostSettings::auto_name_conversations`]. It defaults to on, so an older build
/// dropping it on its next write turns automatic naming back on for a user who had switched it
/// off — a setting that reverts to *calling a model* rather than to not calling one, which is the
/// direction that costs something.
///
/// Thirteen adds [`HostSettings::retain_conversations_days`], and it is the sharpest case yet. The
/// field decides when a conversation nobody marked persistent has its record collected, and it
/// defaults to thirty days. An older build dropping it returns a user who asked for `0` — never
/// collect — or for a longer window to that default, and the next collector run then deletes
/// records they had said to keep. Every other field on this record reverts a *setting*; this one
/// reverts and then deletes the user's own data, which is why the guard matters here even though
/// the field is additive and defaulted.
///
/// Fourteen widens [`SavedRemoteHost`] with `id`, `scheme` and `trust_insecure`. An older build
/// drops all three on its next write — saved hosts lose their keychain linkage, scheme and
/// trust flag — so the bump refuses rather than silently downgrading them.
///
/// Fifteen adds [`HostSettings::tools`]. An older build drops the rows on its next write, and
/// the new-pane menu goes back to offering shells and harnesses only until they are set again.
///
/// Sixteen adds [`HostSettings::ssh_profiles`], and earns the bump on `ai_providers`' footing
/// rather than `tools`': an older build drops the rows on its next write and **strands their
/// passphrases in the OS secret store**, filed under ids nothing on disk names any more. The
/// profiles themselves are re-typable; a secret nothing can reach to delete is not.
///
/// Seventeen adds [`SavedRemoteHost::carrier`]. An older build drops it on its next write and
/// every saved drone silently becomes a socket host pointed at an address it never had — a row
/// that looks connectable and cannot connect, which is worse than one that is gone.
///
/// Eighteen adds [`DronePreset`] to [`RemoteCarrier::Ssh`]. Additive on the same footing as
/// fourteen's trio: an older build drops the field on its next write and every saved drone comes
/// back as [`DronePreset::Attached`] — a real answer, the one every record before this schema
/// already meant, rather than a guess. What is lost is only the *choice*, not a fact the record
/// needs to stay correct, which is why this earns the bump on the lighter footing rather than
/// fourteen's or sixteen's: nothing is stranded in the keychain, and nothing a user set reverts to
/// the wrong direction — it reverts to the one shape that was always safe to assume.
///
/// Nineteen adds `drone_path` to [`RemoteCarrier::Ssh`]. An older build drops it on its next
/// write and a drone deployed off the remote `PATH` goes back to a bare `ubiq-drone` lookup that
/// cannot find it — on the same lighter footing as eighteen: what reverts is a location phase 9's
/// deployer can re-learn and re-save on the next connect, not a fact stranded anywhere a user
/// cannot get back.
pub const HOST_SETTINGS_SCHEMA: u32 = 19;

fn isolate_agents_default() -> bool {
    true
}

fn auto_name_conversations_default() -> bool {
    true
}

fn retain_conversations_days_default() -> u32 {
    30
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

fn mission_term_default() -> String {
    "Mission".to_string()
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            schema: HOST_SETTINGS_SCHEMA,
            isolate_agents: isolate_agents_default(),
            assist: AssistProvider::default(),
            auto_name_conversations: auto_name_conversations_default(),
            agent_home: AgentHome::default(),
            extra_grants: Vec::new(),
            agent_commands: BTreeMap::new(),
            tools: Vec::new(),
            retain_conversations_days: retain_conversations_days_default(),
            search_excludes: search_excludes_default(),
            search_fallbacks: search_fallbacks_default(),
            index_level: IndexLevel::default(),
            mission_term: mission_term_default(),
            projects_root: None,
            ephemeral_root: None,
            connections: Vec::new(),
            oauth_apps: Vec::new(),
            trusted_certs: Vec::new(),
            ai_providers: Vec::new(),
            remote_hosts: Vec::new(),
            ssh_profiles: Vec::new(),
        }
    }
}
