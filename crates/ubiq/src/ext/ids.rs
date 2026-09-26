//! Every container and item id the base declares, as a `const` `SlotId` (`D178`).
//!
//! A string-literal container or item id anywhere else in base code is a lint violation —
//! `just slots-check` finds it, wired into `verify` beside the icon recipes. A second edition has
//! no equivalent module: it registers by string literal and gets `D177`'s boot assertion in place
//! of the compile error a base rename produces here.
//!
//! Two naming conventions, and they mean different things. A **container** and its **groups** are
//! base-owned paths with slashes (`settings/app`, `settings/app/interface`); an **item** carries
//! its owner first, dotted (`ubiq.settings.editor`), so a second edition's own items read as its
//! own at a glance.

use super::SlotId;

// ── The settings containers (`D180`) ────────────────────────────────

/// The application settings overlay.
pub const SETTINGS_APP: SlotId = SlotId::new("settings/app");
/// The project settings dialog — the live one and the sink's fixture are one container.
pub const SETTINGS_PROJECT: SlotId = SlotId::new("settings/project");

// ── The application overlay's groups, in drawn order ────────────────
//
// No separator is drawn between them today: they exist because ordering is by declared group and
// never by integer (`D177`), and they are where a contributed section lands. They name the
// clusters the nav already reads as.

/// Appearance, Size, File explorer, Editor, Search.
pub const SETTINGS_APP_INTERFACE: SlotId = SlotId::new("settings/app/interface");
/// Harnesses, Agent definitions, Isolation, Assistance.
pub const SETTINGS_APP_AGENTS: SlotId = SlotId::new("settings/app/agents");
/// Connectors, Hosts, SSH profiles, Drones.
pub const SETTINGS_APP_CONNECTIVITY: SlotId = SlotId::new("settings/app/connectivity");
/// Tools, Command line.
pub const SETTINGS_APP_SYSTEM: SlotId = SlotId::new("settings/app/system");

// ── The project dialog's groups, in drawn order ─────────────────────

/// General, Tools, Agent definitions, Tasks, Remote — what the project *is* and what runs in it.
pub const SETTINGS_PROJECT_CORE: SlotId = SlotId::new("settings/project/core");
/// Knowledge base, Documentation, Integrations — what the project is wired to.
pub const SETTINGS_PROJECT_CONTENT: SlotId = SlotId::new("settings/project/content");

// ── The application overlay's own sections ──────────────────────────

pub const APPEARANCE: SlotId = SlotId::new("ubiq.settings.appearance");
pub const SIZE: SlotId = SlotId::new("ubiq.settings.size");
pub const FILE_EXPLORER: SlotId = SlotId::new("ubiq.settings.file-explorer");
pub const EDITOR: SlotId = SlotId::new("ubiq.settings.editor");
pub const SEARCH: SlotId = SlotId::new("ubiq.settings.search");
pub const HARNESSES: SlotId = SlotId::new("ubiq.settings.harnesses");
pub const AGENT_DEFINITIONS: SlotId = SlotId::new("ubiq.settings.agent-definitions");
pub const ISOLATION: SlotId = SlotId::new("ubiq.settings.isolation");
pub const ASSIST: SlotId = SlotId::new("ubiq.settings.assist");
pub const CONNECTORS: SlotId = SlotId::new("ubiq.settings.connectors");
pub const HOSTS: SlotId = SlotId::new("ubiq.settings.hosts");
pub const SSH: SlotId = SlotId::new("ubiq.settings.ssh");
pub const DRONES: SlotId = SlotId::new("ubiq.settings.drones");
pub const TOOLS: SlotId = SlotId::new("ubiq.settings.tools");
pub const COMMAND_LINE: SlotId = SlotId::new("ubiq.settings.command-line");

// ── The project dialog's own sections ───────────────────────────────

pub const PROJECT_GENERAL: SlotId = SlotId::new("ubiq.project.general");
pub const PROJECT_TOOLS: SlotId = SlotId::new("ubiq.project.tools");
pub const PROJECT_AGENT_DEFINITIONS: SlotId = SlotId::new("ubiq.project.agent-definitions");
pub const PROJECT_TASKS: SlotId = SlotId::new("ubiq.project.tasks");
pub const PROJECT_REMOTE: SlotId = SlotId::new("ubiq.project.remote");
pub const PROJECT_KB: SlotId = SlotId::new("ubiq.project.kb");
pub const PROJECT_DOCUMENTATION: SlotId = SlotId::new("ubiq.project.documentation");
pub const PROJECT_INTEGRATIONS: SlotId = SlotId::new("ubiq.project.integrations");
/// Task sync (`D187`): the provider, the board, the **rendered** `ConfigField` filter and the
/// fields every binding has. Registered from `ui::tasksrc` through `ext::settings::register`,
/// exactly the way a second edition registers its own and with neither container module touched —
/// the second item on either container to arrive as a genuine contribution rather than as a
/// conversion of a former enum variant.
pub const PROJECT_TASK_SYNC: SlotId = SlotId::new("ubiq.project.task-sync");

// ── The rail container (`D184`) ─────────────────────────────────────

/// The activity rail. One container; its groups are the two headings the rail draws.
pub const RAIL: SlotId = SlotId::new("rail");

/// What the window is, rather than what one project is: Control, All Teams, Sink.
pub const RAIL_APP: SlotId = SlotId::new("rail/app");
/// The views onto the one project the window is pointed at.
pub const RAIL_PROJECT: SlotId = SlotId::new("rail/project");

// ── The rail's own modes ────────────────────────────────────────────
//
// The item id is what a saved layout is keyed by on disk (`D184`), so renaming one here renames
// the key and needs an alias beside it in `state::prefs` — the same trade `D178` already takes.

pub const RAIL_CONTROL: SlotId = SlotId::new("ubiq.rail.control");
pub const RAIL_IDE: SlotId = SlotId::new("ubiq.rail.ide");
pub const RAIL_GIT: SlotId = SlotId::new("ubiq.rail.git");
pub const RAIL_AGENTS: SlotId = SlotId::new("ubiq.rail.agents");
pub const RAIL_TEAMS: SlotId = SlotId::new("ubiq.rail.teams");
pub const RAIL_TEAMS_ALL: SlotId = SlotId::new("ubiq.rail.teams-all");
pub const RAIL_TEAMS_OLD: SlotId = SlotId::new("ubiq.rail.teams-old");
pub const RAIL_KB: SlotId = SlotId::new("ubiq.rail.kb");
pub const RAIL_TASKS: SlotId = SlotId::new("ubiq.rail.tasks");
pub const RAIL_SINK: SlotId = SlotId::new("ubiq.rail.sink");

// ── The kitchen sink's own demo registrations (`X11`, invariant 9, M4) ──────
//
// Not a conversion of anything that used to be a closed enum's variant — the first item on either
// container that arrived as a genuine contribution, registered exactly the way a second edition
// would register its own, through `ext::settings::register`/`ext::rail::register`, with nothing
// in either container's own module touched to add it. See `crate::ui::sink::ext_demo`.

/// The demo settings section, in the application overlay.
pub const EXT_DEMO_SETTINGS: SlotId = SlotId::new("ubiq.settings.ext-demo");
/// The demo rail mode, gated by `Availability::When` reading the switch the section above draws.
pub const EXT_DEMO_RAIL: SlotId = SlotId::new("ubiq.rail.ext-demo");
