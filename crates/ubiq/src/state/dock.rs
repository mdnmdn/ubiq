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

use ubiq_proto::ids::{PaneId, TaskId};

use crate::state::RailMode;

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
    /// The current rail mode (for git panels).
    pub rail_mode: Option<crate::state::RailMode>,
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
/// graph in Teams and [Teams] mode, the board in Tasks mode, and the empty page otherwise. In IDE mode
/// it is the page that says no file is open — as soon as one is, the file panels are the centre and
/// it steps aside.
///
/// Git panels: refs explorer, changes panel, history panel, diff panel. These are movable panels
/// that replace the monolithic git screen, allowing IDE-like arrangement.
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
    /// The definitions in the file on screen. Answered from the buffer, so unlike `Search` it
    /// wants no project — only a file.
    Outline,
    /// Git refs explorer: branches, remotes, tags, stashes, submodules.
    GitRefs,
    /// Git changes panel: modified/untracked/conflicted files and commit message box.
    GitChanges,
    /// Git commit history panel: commit graph with search and filters.
    GitHistory,
    /// Git diff viewer panel: shows diff for selected file/commit.
    GitDiff,
    /// The knowledge base's explorer: one row per configured root, and the documents under each.
    /// KB mode's side panel — the centre is the documents it opens.
    KbExplorer,
    /// One open knowledge-base document, named by its tab key — `state/kb.rs`'s `kb_tab_key`,
    /// which is the source and the path together because a path alone names nothing there.
    ///
    /// **A document is a tab exactly as a file is**, so the centre of KB mode is a strip of them
    /// with the dock's own chrome — drag, split, pin, dirty dot, close. It is a kind of its own
    /// rather than a `File` carrying a KB key because the two are drawn in different rail modes
    /// and are populated from different halves of the project: a `File` is one of
    /// `EditorPaneState::open`, and a document is one of `KbState::docs`.
    Kb(String),
    /// The board's one task: the report for whatever is selected, or the form for one being
    /// written. Tasks mode's side panel — the columns are the centre, and this is what they select.
    ///
    /// **One panel for both**, because they are one slot: a draft answers `open_task` as nothing,
    /// so the form and the report can never be on screen together. `BoardState::popup` pops the
    /// same body out of this panel into a modal; it is a choice of shape, not a second panel.
    Task,
    /// The agents screen's list: every session, every conversation in it, and what each is doing.
    /// Agents mode's side panel — the columns are the centre, and this is what fills them.
    AgentsExplorer,
    /// One mission's side panel — the fast overview of §6.1, named by the anchor task the
    /// mission *is* (there is no `MissionId`, `ubiq_proto::mission` M1). Several may be open at
    /// once, and like a chat tab it is `Free`: a mission is read beside whatever the reader is
    /// doing, in any region and in any rail mode.
    Mission(TaskId),
    /// One mission's **full view**, as a document in the centre region — §6.2's second shape,
    /// named by the same anchor task the side panel is. A document rather than a free panel for
    /// the reason the proposal gives: it stays while the user works, can be split beside a file,
    /// and is restored with the arrangement.
    ///
    /// The same [`TaskId`] may be open as both shapes at once; they read one
    /// `state::mission::MissionView`, so the two are the same view twice rather than two views.
    MissionView(TaskId),
    /// Ubiq's own documentation. **Not mode-owned**: the reader opens it to understand the screen
    /// they are looking at, so it has to survive the rail-mode change that takes them there.
    Help,
}

impl PanelKind {
    /// The name every terminal panel answers. Named as a constant because a saved leaf has to be
    /// recognised as a terminal *before* there is a pane id to build the kind with — see
    /// `ui::dock::leaf`.
    pub const TERMINAL: &'static str = "ubiq.terminal";

    /// The name every chat panel answers, for the same reason [`Self::TERMINAL`] is a constant: a
    /// saved leaf is recognised as a chat tab before there is an id to build one with.
    pub const CHAT: &'static str = "ubiq.chat";

    /// The name every mission panel answers, for the reason [`Self::CHAT`] is a constant: its
    /// anchor task travels in the payload, so a saved leaf is recognised before there is a
    /// [`TaskId`] to build one with.
    pub const MISSION: &'static str = "ubiq.mission";

    /// The name every mission **full view** document answers, for [`Self::MISSION`]'s reason: its
    /// anchor task travels in the same payload.
    pub const MISSION_VIEW: &'static str = "ubiq.mission.view";

    /// Where this kind may sit. One function, consulted in one place.
    pub fn class(&self) -> PanelClass {
        match self {
            PanelKind::Terminal(_)
            | PanelKind::Logs
            | PanelKind::Search
            | PanelKind::Outline
            | PanelKind::Chat(_)
            | PanelKind::Mission(_)
            | PanelKind::GitRefs
            | PanelKind::GitChanges
            | PanelKind::GitHistory
            | PanelKind::GitDiff
            | PanelKind::Help => PanelClass::Free,
            PanelKind::Explorer
            | PanelKind::KbExplorer
            | PanelKind::Task
            | PanelKind::AgentsExplorer => PanelClass::Edge,
            PanelKind::Centre
            | PanelKind::File(_)
            | PanelKind::Kb(_)
            | PanelKind::MissionView(_) => PanelClass::Centre,
        }
    }

    /// Where a panel of this kind opens, and where one dropped somewhere its class forbids is put
    /// back. Every kind's home satisfies its own class.
    pub fn home(&self) -> Region {
        match self {
            PanelKind::Terminal(_) | PanelKind::Logs | PanelKind::Search => Region::Bottom,
            PanelKind::Explorer
            | PanelKind::Outline
            | PanelKind::KbExplorer
            | PanelKind::AgentsExplorer => Region::Left,
            PanelKind::Chat(_) | PanelKind::Task | PanelKind::Mission(_) | PanelKind::Help => {
                Region::Right
            }
            PanelKind::Centre
            | PanelKind::File(_)
            | PanelKind::Kb(_)
            | PanelKind::MissionView(_) => Region::Centre,
            // Git panels default to left/right edges for IDE-like layout
            PanelKind::GitRefs => Region::Left,
            PanelKind::GitChanges => Region::Right,
            PanelKind::GitHistory => Region::Centre,
            PanelKind::GitDiff => Region::Centre,
        }
    }

    /// [`Self::home`], asked about one rail mode. **The mode is part of the placement policy**, not
    /// a branch at the site that opens a panel: Tasks has the task in its right region, so a chat
    /// there goes where it always goes, and Teams draws its graph in the centre with the inspector
    /// beside it — the chat is the conversation with the agents, and that stays on the right in
    /// every mode, Teams included.
    ///
    /// Everything else answers the same in every mode, which is why this delegates rather than
    /// repeating the table.
    pub fn home_in(&self, mode: RailMode) -> Region {
        match self {
            PanelKind::Chat(_) => Self::chat_home(mode),
            _ => self.home(),
        }
    }

    /// [`Self::home_in`] for a chat tab, asked without one in hand — what a window filling an
    /// emptied side region needs to know *before* it has minted the tab to put there. A chat is the
    /// agents' side of the conversation, and it holds its region — the right dock — whatever the
    /// mode.
    pub fn chat_home(_mode: RailMode) -> Region {
        Region::Right
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
            PanelKind::Mission(_) => Self::MISSION,
            PanelKind::MissionView(_) => Self::MISSION_VIEW,
            PanelKind::Centre => "ubiq.centre",
            PanelKind::File(_) => "ubiq.file",
            PanelKind::Search => "ubiq.search",
            PanelKind::Outline => "ubiq.outline",
            PanelKind::GitRefs => "ubiq.git.refs",
            PanelKind::GitChanges => "ubiq.git.changes",
            PanelKind::GitHistory => "ubiq.git.history",
            PanelKind::GitDiff => "ubiq.git.diff",
            PanelKind::KbExplorer => "ubiq.kb.explorer",
            PanelKind::Kb(_) => "ubiq.kb.doc",
            PanelKind::Task => "ubiq.task",
            PanelKind::AgentsExplorer => "ubiq.agents.explorer",
            PanelKind::Help => "ubiq.help",
        }
    }

    /// The kind a saved layout's panel name means, or nothing for a name this build cannot rebuild
    /// from a name alone.
    ///
    /// Five kinds have no answer. A terminal is dropped on purpose — **layout persists and
    /// harnesses do not**, so a saved terminal panel goes and the tree normalises around the gap.
    /// A file is not dropped but is *not rebuilt from its name either*: it is rebuilt from the
    /// payload beside it, which is where its tab key travels. A chat tab is the same as the file:
    /// its id travels in the payload, because the name below is the same for every one of them.
    /// A mission panel and a mission full view are the chat tab's case again: the anchor task
    /// travels in the payload beside them.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ubiq.logs" => Some(PanelKind::Logs),
            "ubiq.explorer" => Some(PanelKind::Explorer),
            "ubiq.centre" => Some(PanelKind::Centre),
            "ubiq.search" => Some(PanelKind::Search),
            "ubiq.outline" => Some(PanelKind::Outline),
            "ubiq.git.refs" => Some(PanelKind::GitRefs),
            "ubiq.git.changes" => Some(PanelKind::GitChanges),
            "ubiq.git.history" => Some(PanelKind::GitHistory),
            "ubiq.git.diff" => Some(PanelKind::GitDiff),
            "ubiq.kb.explorer" => Some(PanelKind::KbExplorer),
            "ubiq.task" => Some(PanelKind::Task),
            "ubiq.agents.explorer" => Some(PanelKind::AgentsExplorer),
            "ubiq.help" => Some(PanelKind::Help),
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

    /// The document this panel is the knowledge-base tab of, if it is one.
    ///
    /// Deliberately *not* folded into [`Self::tab_key`]: that one answers "which of the project's
    /// open files is this", and every caller of it — the panel sync, the file activation, the
    /// bookmark count — would go looking for a document among them and find nothing.
    pub fn kb_key(&self) -> Option<&str> {
        match self {
            PanelKind::Kb(key) => Some(key.as_str()),
            _ => None,
        }
    }

    /// The mission this panel is about, if it is one — the anchor task's id, for either shape.
    ///
    /// Both shapes answer because both write the same payload into a saved layout; which of the
    /// two a saved leaf rebuilds as is [`Self::name`]'s answer, not this one's.
    pub fn mission_id(&self) -> Option<TaskId> {
        match self {
            PanelKind::Mission(id) | PanelKind::MissionView(id) => Some(*id),
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
            // **A conversation is furniture on every screen about a project**, not a list of
            // three: the agents are how work gets done in all of them, and the right region is
            // where they stay. Agents is the exception and the reason is the screen itself — the
            // columns *are* the conversation there, and a chat panel beside them is the same
            // thing twice. Control and the sink are not about a project at all. It still wants a
            // project everywhere it is drawn — a conversation about nothing is a fiction.
            PanelKind::Chat(_) => {
                at.has_project
                    && !matches!(
                        at.rail_mode,
                        Some(RailMode::AGENTS | RailMode::CONTROL | RailMode::SINK)
                    )
            }
            PanelKind::Terminal(_) => at.pane_on_screen,
            PanelKind::Logs => true,
            // In Git the history and the diff are the centre, so this panel — the mode page —
            // steps aside while a project is on screen. Without a project the git panels have
            // nothing to show, and this is the empty page that says so.
            PanelKind::Centre => {
                if matches!(at.rail_mode, Some(RailMode::GIT)) && at.has_project {
                    false
                } else {
                    !at.is_ide || !at.any_file_open
                }
            }
            PanelKind::File(_) => at.is_ide && at.file_open,
            // The same rule one mode along: a document is drawn while its tab is open, and only on
            // the screen the documents are the centre of.
            PanelKind::Kb(_) => {
                at.has_project && matches!(at.rail_mode, Some(RailMode::KB)) && at.file_open
            }
            PanelKind::Search => at.is_ide && at.has_project,
            // No project clause: a file dropped in from outside every project still has an
            // outline, because the buffer is the whole input.
            PanelKind::Outline => at.is_ide && at.any_file_open,
            // Git panels are drawn in Git rail mode with a project
            PanelKind::GitRefs
            | PanelKind::GitChanges
            | PanelKind::GitHistory
            | PanelKind::GitDiff => at.has_project && matches!(at.rail_mode, Some(RailMode::GIT)),
            // The same rule one mode along: the knowledge base's explorer is KB's own furniture,
            // and a project is what it lists.
            PanelKind::KbExplorer => at.has_project && matches!(at.rail_mode, Some(RailMode::KB)),
            // And again for the two screens that gained a side panel of their own: the board's
            // task and the agents list are their mode's furniture, and a project is what either
            // is about.
            PanelKind::Task => at.has_project && matches!(at.rail_mode, Some(RailMode::TASKS)),
            PanelKind::AgentsExplorer => {
                at.has_project && matches!(at.rail_mode, Some(RailMode::AGENTS))
            }
            // No clause at all: help is about the application, so it is drawn wherever the reader
            // opened it — including in a window with no project, which is one of the places a
            // reader most needs it.
            // No clause either: a mission is read in every mode, the way a chat tab is, and it
            // belongs to a project — a window pointed elsewhere has nothing to draw it from, and
            // `ui::mission::panel` says so rather than the panel disappearing.
            PanelKind::Mission(_) => at.has_project,
            // The full view is a document rather than furniture, so it is drawn wherever it was
            // opened, in any mode — the same "no clause at all" the side panel has, one region
            // along. It wants a project for the panel's reason: the record is the project's.
            PanelKind::MissionView(_) => at.has_project,
            PanelKind::Help => true,
        }
    }

    /// Whether this panel belongs to Git mode rather than to the window.
    pub fn is_git(&self) -> bool {
        matches!(
            self,
            PanelKind::GitRefs | PanelKind::GitChanges | PanelKind::GitHistory | PanelKind::GitDiff
        )
    }

    /// Whether this panel belongs to a rail mode rather than to the window.
    ///
    /// Git's four, the IDE's file explorer, the knowledge base's explorer, the board's task and
    /// the agents list travel with their own mode's saved
    /// arrangement. Putting one back into another mode's tree would open that mode's edges for a
    /// panel it hides — so leftover restore skips them, and a first visit to the mode asks for
    /// them again. A second mode with side panels of its own is a name in this list, not a second
    /// branch in the restore.
    ///
    /// **The IDE's explorer is one of them**, even though [`Self::is_drawn`] already hides it
    /// outside IDE: it is that mode's left-hand furniture (`AppState::queue_mode_furniture`), so a
    /// leftover restore was leaking it into Git's, the knowledge base's and the agents' left
    /// regions and then into their blobs.
    pub fn is_mode_owned(&self) -> bool {
        self.is_git()
            || matches!(
                self,
                PanelKind::Explorer
                    | PanelKind::KbExplorer
                    | PanelKind::Task
                    | PanelKind::AgentsExplorer
            )
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
                | PanelKind::Kb(_)
                | PanelKind::Logs
                | PanelKind::Search
                | PanelKind::Outline
                | PanelKind::Chat(_)
                | PanelKind::Mission(_)
                | PanelKind::MissionView(_)
                | PanelKind::GitRefs
                | PanelKind::GitChanges
                | PanelKind::GitHistory
                | PanelKind::GitDiff
                | PanelKind::Help
        )
    }
}
