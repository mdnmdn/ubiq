//! What a panel is, and where it is allowed to sit.
//!
//! The window's arrangement is a tree the user rearranges — the dock — and the movable unit in it
//! is a **panel**. A [`PanelKind`] is what identifies one: a terminal names its pane, a file names
//! its tab, and everything else is one of a kind. Nothing here draws, and nothing here knows what a
//! group or a split is: the tree is the component library's, and this is the policy over it.
//!
//! **Placement is a property of the kind, not a special case.** An explorer squeezed into the
//! bottom region is a sixty-pixel-tall tree and a chat squeezed into a centre column stops being a
//! conversation, so each kind answers one function — [`PanelKind::class`] — and the dock consults
//! it in one place. Widening the policy later is a row in the table rather than a branch somewhere.
//!
//! The same holds for **whether a panel is drawn**. [`PanelKind::is_drawn`] is asked against one
//! [`Visibility`] — everything the window knows about itself that a panel could care about — so a
//! new rule is a field on that struct rather than another argument threaded through the dock.

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use ubiq_proto::ids::PaneId;

/// One chat tab's identity, minted the way [`ubiq_proto::work::AgentId::generate`] mints
/// one — the counter is local rather than the contract's because a chat tab is UI arrangement,
/// never a fact the host is told. Carried in the dock's payload exactly as a pane's id is, so a
/// panel round-trips through a saved layout, but never meant to survive past the process: a leaf
/// naming one this window never minted names nothing, the way a saved terminal leaf does.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChatId(u64);

impl ChatId {
    /// Mint the next one. There is no `default()`: an id that was not minted is a nil id that
    /// looks real.
    pub fn generate() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for ChatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ChatId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}

/// One of the window's regions. There is no top: the component library's dock places edge regions
/// left, right and bottom only, so "docked on top" is a split at the top of the centre.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Region {
    Centre,
    Left,
    Right,
    Bottom,
}

/// Where a kind of panel may go.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelClass {
    /// The left or right region only — what "the side panel stays on the border" turns into.
    Edge,
    /// Any region at all. A terminal was never squeezed into an edge because nothing asked for
    /// that; a chat tab is, so this now means what its name always said and both kinds share it
    /// rather than the chat inventing a class of its own.
    Free,
    /// The centre region only.
    Centre,
}

impl PanelClass {
    /// Whether a panel of this class may be dropped in `region`.
    pub fn allows(self, region: Region) -> bool {
        match self {
            PanelClass::Edge => matches!(region, Region::Left | Region::Right),
            PanelClass::Free => true,
            PanelClass::Centre => region == Region::Centre,
        }
    }
}

/// What one window's panel is drawn about, and what the window asks of it out of band.
///
/// Every question a panel answers about itself is asked against the whole of this, in one place,
/// so widening a rule is a field here rather than another argument threaded through the dock.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Visibility {
    /// The rail is on IDE, which is what the explorer and the chat are furniture for.
    pub is_ide: bool,
    /// The window is pointed at a project.
    pub has_project: bool,
    /// This panel's pane belongs to the project on screen. Meaningless for anything but a terminal.
    pub pane_on_screen: bool,
    /// This panel's file is one of the project's open tabs. Meaningless for anything but a file.
    pub file_open: bool,
    /// The project on screen has at least one file open, which is what the centre steps aside for.
    pub any_file_open: bool,
}

/// What one panel is. `Logs`, `Explorer` and `Centre` are one panel per window; a terminal is one
/// per pane, and carries the id every message about it is keyed by; a file is one per open tab,
/// and carries the tab's key; a chat is one per open tab the same way, and carries a
/// [`ChatId`] the host never sees.
///
/// **A file's key is the tab's, not the path's.** A file and its diff are two tabs looking at the
/// same file, so `state/editor.rs`'s `tab_key` is what identifies a panel and a bare path is not.
/// A chat tab's id is the same idea one step further: nothing about it is derived from what it is
/// looking at, because it may be looking at nothing at all.
///
/// `Centre` is the one panel whose body follows the rail mode: the columns in Agents mode, the
/// graph in Orchestration mode, the board in Tasks mode, and the empty page otherwise. In IDE mode
/// it is the page that says no file is open — as soon as one is, the file panels are the centre and
/// it steps aside.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum PanelKind {
    Terminal(PaneId),
    Logs,
    Explorer,
    /// One chat tab, named by its id. Many may exist at once, each attached to a conversation of
    /// its own (or to none) — see `state::chat::ChatTab`, which is where that attachment lives.
    /// The id is what tells two tabs apart; the name below is the same for all of them.
    Chat(ChatId),
    Centre,
    /// One open file, named by its tab key.
    File(String),
    /// Project content search.
    Search,
}

impl PanelKind {
    /// The name every terminal panel answers. Named as a constant because a saved leaf has to be
    /// recognised as a terminal *before* there is a pane id to build the kind with — see
    /// `ui::dock::leaf`.
    pub const TERMINAL: &'static str = "ubiq.terminal";

    /// The name every chat panel answers, for the same reason [`Self::TERMINAL`] is a constant: a
    /// saved leaf is recognised as a chat tab before there is an id to build one with.
    pub const CHAT: &'static str = "ubiq.chat";

    /// Where this kind may sit. One function, consulted in one place.
    pub fn class(&self) -> PanelClass {
        match self {
            PanelKind::Terminal(_) | PanelKind::Logs | PanelKind::Search | PanelKind::Chat(_) => {
                PanelClass::Free
            }
            PanelKind::Explorer => PanelClass::Edge,
            PanelKind::Centre | PanelKind::File(_) => PanelClass::Centre,
        }
    }

    /// Where a panel of this kind opens, and where one dropped somewhere its class forbids is put
    /// back. Every kind's home satisfies its own class.
    pub fn home(&self) -> Region {
        match self {
            PanelKind::Terminal(_) | PanelKind::Logs | PanelKind::Search => Region::Bottom,
            PanelKind::Explorer => Region::Left,
            PanelKind::Chat(_) => Region::Right,
            PanelKind::Centre | PanelKind::File(_) => Region::Centre,
        }
    }

    /// The permanent name a saved layout is rebuilt from. **It never changes**: it is the key the
    /// rebuild looks a panel up by, and a renamed panel is a panel a saved layout has lost.
    ///
    /// Every file panel answers the same name. What tells one from another is its **payload**, not
    /// its name — a name is a `&'static str` and a tab key is not, and a saved layout's panels are
    /// looked up by name and rebuilt from what they carried.
    pub fn name(&self) -> &'static str {
        match self {
            PanelKind::Terminal(_) => Self::TERMINAL,
            PanelKind::Logs => "ubiq.logs",
            PanelKind::Explorer => "ubiq.explorer",
            PanelKind::Chat(_) => Self::CHAT,
            PanelKind::Centre => "ubiq.centre",
            PanelKind::File(_) => "ubiq.file",
            PanelKind::Search => "ubiq.search",
        }
    }

    /// The kind a saved layout's panel name means, or nothing for a name this build cannot rebuild
    /// from a name alone.
    ///
    /// Three kinds have no answer. A terminal is dropped on purpose — **layout persists and
    /// harnesses do not**, so a saved terminal panel goes and the tree normalises around the gap.
    /// A file is not dropped but is *not rebuilt from its name either*: it is rebuilt from the
    /// payload beside it, which is where its tab key travels. A chat tab is the same as the file:
    /// its id travels in the payload, because the name below is the same for every one of them.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ubiq.logs" => Some(PanelKind::Logs),
            "ubiq.explorer" => Some(PanelKind::Explorer),
            "ubiq.centre" => Some(PanelKind::Centre),
            "ubiq.search" => Some(PanelKind::Search),
            _ => None,
        }
    }

    /// The pane this panel is the terminal of, if it is one. Every focus and resize rule keys off
    /// this being `Some`.
    pub fn pane(&self) -> Option<PaneId> {
        match self {
            PanelKind::Terminal(pane_id) => Some(*pane_id),
            _ => None,
        }
    }

    /// The tab this panel is the file of, if it is one. What `state/editor.rs` calls a tab key: a
    /// path for the file itself, and a prefixed path for something the host made from it.
    pub fn tab_key(&self) -> Option<&str> {
        match self {
            PanelKind::File(key) => Some(key.as_str()),
            _ => None,
        }
    }

    /// The tab this panel is the chat instance of, if it is one.
    pub fn chat_id(&self) -> Option<ChatId> {
        match self {
            PanelKind::Chat(id) => Some(*id),
            _ => None,
        }
    }

    /// Whether a panel of this kind has anything to show, given what the window is doing.
    ///
    /// A panel with nothing to show is **hidden, not removed**: it keeps its place in the
    /// arrangement and its tab slot, and comes back where it was left. The explorer and the chat
    /// are IDE furniture and leave with the mode; the chat also wants a project, because a
    /// conversation about nothing is a fiction. A terminal belongs to a project, so a pane of one
    /// this window is not pointed at keeps running, keeps its scrollback, and stays off screen.
    ///
    /// The last two rules are one rule from two sides: **in IDE mode the open files are the
    /// centre**. A file panel is drawn while its tab is open, and the centre panel — which in that
    /// mode is only ever the page saying no file is open — steps aside for as long as one is. It is
    /// the same hidden-not-removed machinery, so the centre comes back where it was left when the
    /// last tab closes rather than being rebuilt somewhere else.
    pub fn is_drawn(&self, at: Visibility) -> bool {
        match self {
            PanelKind::Explorer => at.is_ide,
            PanelKind::Chat(_) => at.is_ide && at.has_project,
            PanelKind::Terminal(_) => at.pane_on_screen,
            PanelKind::Logs => true,
            PanelKind::Centre => !at.is_ide || !at.any_file_open,
            PanelKind::File(_) => at.is_ide && at.file_open,
            PanelKind::Search => at.is_ide && at.has_project,
        }
    }

    /// Whether the panel's tab offers a close. A terminal's close kills its harness, a file's
    /// closes its tab, and a chat tab's closes the view — the conversation it was attached to, if
    /// any, keeps running, because the tab was never anything but a perspective on it. Every other
    /// panel is the window's own furniture and is hidden rather than closed.
    pub fn closable(&self) -> bool {
        matches!(
            self,
            PanelKind::Terminal(_)
                | PanelKind::File(_)
                | PanelKind::Logs
                | PanelKind::Search
                | PanelKind::Chat(_)
        )
    }
}
