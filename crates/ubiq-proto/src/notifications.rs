//! What a subsystem raises for the bell, where it points, and how it is silenced.
//!
//! A notification is a small owned record with an **origin** — a family, an optional actor inside
//! that family, an optional category of event — and a **level**. The origin is what a mute rule
//! matches on, so the three fields are not decoration: they are the addressing scheme for
//! silencing. Everything here serialises, because the raiser lives in the host and the bell lives
//! in a window.
//!
//! Two flags travel with a request and both default to `false`. `muted` says *arrive quietly*:
//! the badge counts it and the bell does not flash. `os` says *also tell the desktop*, which is
//! the one thing here that reaches outside Ubiq, and is therefore never the default.

use serde::{Deserialize, Serialize};

use crate::ids::{NotificationId, PaneId, ProjectId};
use crate::work::AgentId;

/// How many notifications the host keeps. Older ones fall off the end rather than the store
/// growing for the life of the process; a bell is a recent-events list, not a journal.
pub const HISTORY_CAP: usize = 200;

/// How loud one notification is.
///
/// Ordered on purpose — a mute rule silences everything **at or below** its level, so `Info` is
/// the quietest thing to silence and `Error` silences the lot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    Info,
    Warning,
    Error,
}

impl Level {
    /// Every level, quietest first. What a menu of them is built from.
    pub const ALL: [Level; 3] = [Level::Info, Level::Warning, Level::Error];

    /// The lower-case name, for a log line and a theme token lookup.
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }

    /// The word a person reads in a menu.
    pub fn label(self) -> &'static str {
        match self {
            Level::Info => "Info",
            Level::Warning => "Warning",
            Level::Error => "Error",
        }
    }
}

/// Which part of Ubiq raised it.
///
/// A closed set, because a mute rule names one and a menu lists them all: an open string would
/// make "mute this family" a guess about spelling. Adding a family is one variant here and one
/// row in the feature document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Family {
    /// A harness — a turn finishing, a permission wanted, a tool refused.
    Agents,
    /// A pane and the process inside it.
    Terminal,
    /// The file, search, index and watch workers.
    Files,
    /// A repository the host has read.
    Git,
    /// Tasks, sessions and workspaces.
    Work,
    /// A connection at an external service.
    Connectors,
    /// The host itself — a store that would not write, a worker that died.
    Host,
    /// The application: updates, imports, anything that is Ubiq talking about Ubiq.
    Ubiq,
}

impl Family {
    /// Every family, in menu order.
    pub const ALL: [Family; 8] = [
        Family::Agents,
        Family::Terminal,
        Family::Files,
        Family::Git,
        Family::Work,
        Family::Connectors,
        Family::Host,
        Family::Ubiq,
    ];

    /// The word a person reads in a menu.
    pub fn label(self) -> &'static str {
        match self {
            Family::Agents => "Agents",
            Family::Terminal => "Terminal",
            Family::Files => "Files",
            Family::Git => "Git",
            Family::Work => "Work",
            Family::Connectors => "Connectors",
            Family::Host => "Host",
            Family::Ubiq => "Ubiq",
        }
    }
}

/// Where a notification points once it is clicked.
///
/// Every variant but [`UbiqLink::Url`] names something inside Ubiq by id, never by path — the
/// interface resolves it against what it already holds, and a link naming something that has
/// since gone is dropped rather than drawn. `Url` is the one that leaves: it opens in the
/// user's browser, so a login notice can carry the page it wants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UbiqLink {
    /// Focus this pane.
    Pane(PaneId),
    /// Show this project.
    Project(ProjectId),
    /// Open the chat tab for this agent.
    Agent(AgentId),
    /// Open this file, project-relative.
    File { project: ProjectId, path: String },
    /// Hand this to the operating system's browser.
    Url(String),
}

/// One notification, as the host filed it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub id: NotificationId,
    pub level: Level,
    pub family: Family,
    /// Who inside the family — an agent's name, a pane's title. `None` means the family itself.
    pub actor: Option<String>,
    /// What kind of event — `"tool call"`, `"exit"`. `None` means uncategorised.
    pub category: Option<String>,
    /// What the user reads. One line; the list wraps it rather than truncating.
    pub text: String,
    /// Where clicking it goes, if anywhere.
    pub link: Option<UbiqLink>,
    /// Milliseconds since the Unix epoch, taken by the host when it filed the record.
    pub at: u64,
    /// Whether it arrived silently — either the request asked for that, or a mute rule matched.
    /// A muted notification still counts against the badge; it just does not make the bell flash.
    pub muted: bool,
    /// Whether the user has ticked it off. The badge counts the unread.
    pub read: bool,
}

/// What a raiser hands the host. The id, the timestamp and `read` are the host's to fill in.
///
/// Build one with [`NotificationRequest::new`] and the `with_*` methods rather than a literal, so
/// a new field does not break every call site.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotificationRequest {
    pub level: Level,
    pub family: Family,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    pub text: String,
    #[serde(default)]
    pub link: Option<UbiqLink>,
    /// Arrive without flashing the bell. Default `false`.
    #[serde(default)]
    pub muted: bool,
    /// Also raise it with the operating system. Default `false`, because this is the one thing
    /// here that escapes the window.
    #[serde(default)]
    pub os: bool,
}

impl NotificationRequest {
    /// The three fields every notification has. Everything else is a `with_*` away.
    pub fn new(level: Level, family: Family, text: impl Into<String>) -> Self {
        Self {
            level,
            family,
            actor: None,
            category: None,
            text: text.into(),
            link: None,
            muted: false,
            os: false,
        }
    }

    /// Convenience for the three levels, so a call site reads as what it is.
    pub fn info(family: Family, text: impl Into<String>) -> Self {
        Self::new(Level::Info, family, text)
    }

    pub fn warning(family: Family, text: impl Into<String>) -> Self {
        Self::new(Level::Warning, family, text)
    }

    pub fn error(family: Family, text: impl Into<String>) -> Self {
        Self::new(Level::Error, family, text)
    }

    /// Who inside the family raised it.
    pub fn with_actor(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }

    /// What kind of event it is.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Where clicking it goes.
    pub fn with_link(mut self, link: UbiqLink) -> Self {
        self.link = Some(link);
        self
    }

    /// Arrive quietly: count against the badge, do not flash the bell.
    pub fn quietly(mut self) -> Self {
        self.muted = true;
        self
    }

    /// Also tell the desktop.
    pub fn with_os(mut self) -> Self {
        self.os = true;
        self
    }
}

/// What a mute rule addresses.
///
/// The four shapes are the origin read at four widths — everything, one family, one actor inside
/// a family, one category inside a family. An actor and a category are matched case-sensitively
/// against the strings the raiser sent, because they come from the same code either way.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MuteScope {
    /// Every notification, whatever its origin.
    All,
    /// Everything from one family.
    Family(Family),
    /// One actor inside one family — a single agent.
    Actor { family: Family, actor: String },
    /// One category inside one family — every tool call, say.
    Category { family: Family, category: String },
}

impl MuteScope {
    /// Whether this scope covers that origin. Level is the caller's to check.
    pub fn covers(&self, family: Family, actor: Option<&str>, category: Option<&str>) -> bool {
        match self {
            MuteScope::All => true,
            MuteScope::Family(f) => *f == family,
            MuteScope::Actor {
                family: f,
                actor: a,
            } => *f == family && actor == Some(a.as_str()),
            MuteScope::Category {
                family: f,
                category: c,
            } => *f == family && category == Some(c.as_str()),
        }
    }

    /// The word a person reads where a rule is listed.
    pub fn label(&self) -> String {
        match self {
            MuteScope::All => "Everything".to_string(),
            MuteScope::Family(f) => f.label().to_string(),
            MuteScope::Actor { family, actor } => format!("{} · {actor}", family.label()),
            MuteScope::Category { family, category } => {
                format!("{} · {category}", family.label())
            }
        }
    }
}

/// How long a mute lasts. A closed set, because both halves have to agree on what "15 minutes"
/// means and the menu is the same menu everywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MuteFor {
    Minutes5,
    Minutes15,
    Hour1,
    Hours8,
    /// Until it is lifted by hand.
    Always,
}

impl MuteFor {
    /// Every duration, in menu order.
    pub const ALL: [MuteFor; 5] = [
        MuteFor::Minutes5,
        MuteFor::Minutes15,
        MuteFor::Hour1,
        MuteFor::Hours8,
        MuteFor::Always,
    ];

    /// How many minutes, or `None` for a rule with no end.
    pub fn minutes(self) -> Option<u64> {
        match self {
            MuteFor::Minutes5 => Some(5),
            MuteFor::Minutes15 => Some(15),
            MuteFor::Hour1 => Some(60),
            MuteFor::Hours8 => Some(8 * 60),
            MuteFor::Always => None,
        }
    }

    /// The word a person reads in a menu.
    pub fn label(self) -> &'static str {
        match self {
            MuteFor::Minutes5 => "5 minutes",
            MuteFor::Minutes15 => "15 minutes",
            MuteFor::Hour1 => "1 hour",
            MuteFor::Hours8 => "8 hours",
            MuteFor::Always => "Until I turn it back on",
        }
    }
}

/// One standing rule: this origin, at or below this level, until this moment.
///
/// A scope holds at most one rule — muting the same scope twice replaces the first, which is what
/// makes "mute this agent for an hour" idempotent from a menu.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MuteRule {
    pub scope: MuteScope,
    /// Silence everything **at or below** this level. `Error` silences the whole scope.
    pub max_level: Level,
    /// Milliseconds since the Unix epoch when it lapses; `None` never lapses.
    pub until: Option<u64>,
}

impl MuteRule {
    /// Whether this rule silences that notification at that moment.
    pub fn silences(
        &self,
        level: Level,
        family: Family,
        actor: Option<&str>,
        category: Option<&str>,
        now_ms: u64,
    ) -> bool {
        if self.until.is_some_and(|until| now_ms >= until) {
            return false;
        }
        level <= self.max_level && self.scope.covers(family, actor, category)
    }
}

/// Everything the bell draws: the history, newest first, and the rules in force.
///
/// Sent whole rather than as a diff. The list is capped at [`HISTORY_CAP`] and changes on a click,
/// so a whole-state message is cheaper to reason about than a patch protocol and never leaves two
/// windows disagreeing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notifications {
    /// Newest first.
    pub items: Vec<Notification>,
    pub mutes: Vec<MuteRule>,
}

impl Notifications {
    /// What the badge shows.
    pub fn unread(&self) -> usize {
        self.items.iter().filter(|n| !n.read).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_silences_at_or_below_its_level() {
        let rule = MuteRule {
            scope: MuteScope::All,
            max_level: Level::Warning,
            until: None,
        };
        assert!(rule.silences(Level::Info, Family::Agents, None, None, 0));
        assert!(rule.silences(Level::Warning, Family::Agents, None, None, 0));
        assert!(!rule.silences(Level::Error, Family::Agents, None, None, 0));
    }

    #[test]
    fn a_lapsed_rule_silences_nothing() {
        let rule = MuteRule {
            scope: MuteScope::All,
            max_level: Level::Error,
            until: Some(1_000),
        };
        assert!(rule.silences(Level::Error, Family::Host, None, None, 999));
        assert!(!rule.silences(Level::Error, Family::Host, None, None, 1_000));
    }

    #[test]
    fn a_scope_reads_the_origin_at_four_widths() {
        let agent = MuteScope::Actor {
            family: Family::Agents,
            actor: "claude".to_string(),
        };
        assert!(agent.covers(Family::Agents, Some("claude"), Some("tool call")));
        assert!(!agent.covers(Family::Agents, Some("codex"), None));
        assert!(!agent.covers(Family::Terminal, Some("claude"), None));

        let tools = MuteScope::Category {
            family: Family::Agents,
            category: "tool call".to_string(),
        };
        assert!(tools.covers(Family::Agents, Some("codex"), Some("tool call")));
        assert!(!tools.covers(Family::Agents, Some("codex"), None));

        assert!(MuteScope::Family(Family::Git).covers(Family::Git, None, None));
        assert!(MuteScope::All.covers(Family::Ubiq, None, None));
    }

    #[test]
    fn a_request_defaults_to_loud_and_local() {
        let request = NotificationRequest::info(Family::Agents, "done");
        assert!(!request.muted);
        assert!(!request.os);
        assert_eq!(request.level, Level::Info);
    }
}
