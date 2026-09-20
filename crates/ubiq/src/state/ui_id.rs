//! Names for places on the screen, and the per-frame record of where each one was drawn.
//!
//! A [`UiId`] is a stable, hierarchical, human-readable name for a region of the interface —
//! `rail`, `rail.mode.tasks`, `titlebar.help`. It is deliberately *not* the kebab-case string
//! every interactive element already passes to GPUI: those exist to key hover and scroll state,
//! they are unique only among their siblings, and nothing outside the frame they were built in
//! may depend on them. A `UiId` is the opposite — it is a public name, it outlives the element
//! that carries it, and other things key data to it.
//!
//! The first of those things is in-place help: point at a control and read what it is. How the
//! sentence is bound to the name is an open question — `_docs/wip/in-place-help.md` §2.5 names the
//! two candidates — and nothing here decides it.
//! Personalisation and extensions are the reason the name is a scheme rather than a help detail:
//! "hide this control", "put an extension's badge here", "this is the control the tour points at"
//! all need the same answer to *which place*.
//!
//! The second half of the module is the machinery an overlay needs to turn a cursor position back
//! into a name. Per-element mouse handlers cannot do it — an overlay drawn on top occludes every
//! element under it, so no child ever sees the move — so marked elements *report* their rectangle
//! as they are laid out, and the overlay hit-tests the resulting list itself. [`MarkRegistry`]
//! holds that list, [`deepest_hit`] is the lookup, and both are plain data with no GPUI in them,
//! which is what makes the hit-test testable without a window. The element side that fills the
//! registry is [`crate::ui::ident`].

use std::borrow::Cow;
use std::fmt;

use super::RailMode;

/// The separator between levels of a [`UiId`].
pub const SEPARATOR: char = '.';

/// The namespace a [`UiId`] carries when it is written down outside this crate —
/// `ui.rail.mode.tasks`. It says which kind of name this is, so that a file holding several
/// (a help page's keys, a preference, an extension manifest) can tell them apart.
pub const NAMESPACE: &str = "ui.";

/// The most levels an id may have. A limit exists so that "how deep can this go" has an answer;
/// eight is far past anything the interface has needed.
pub const MAX_DEPTH: usize = 8;

/// The most characters an id may have, separators included.
pub const MAX_LEN: usize = 96;

/// Why a string is not a [`UiId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiIdError {
    /// The whole string was empty.
    Empty,
    /// Longer than [`MAX_LEN`].
    TooLong(usize),
    /// More than [`MAX_DEPTH`] levels.
    TooDeep(usize),
    /// A level between two separators was empty — a leading, trailing or doubled `.`.
    EmptySegment,
    /// A level held something outside `a-z`, `0-9` and `-`, started with something other than a
    /// letter, or ended with a `-`.
    BadSegment(String),
}

impl fmt::Display for UiIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiIdError::Empty => write!(f, "a ui id may not be empty"),
            UiIdError::TooLong(n) => write!(f, "a ui id may not exceed {MAX_LEN} characters ({n})"),
            UiIdError::TooDeep(n) => write!(f, "a ui id may not exceed {MAX_DEPTH} levels ({n})"),
            UiIdError::EmptySegment => write!(f, "a ui id may not have an empty level"),
            UiIdError::BadSegment(s) => write!(
                f,
                "`{s}` is not a valid level: lowercase letters, digits and inner hyphens, \
                 starting with a letter"
            ),
        }
    }
}

impl std::error::Error for UiIdError {}

/// A stable name for a place on the screen.
///
/// # Grammar
///
/// ```text
/// ui-id   := segment ( "." segment )*        1..=MAX_DEPTH segments, <= MAX_LEN chars
/// segment := [a-z] ( [a-z0-9] | "-" )*       must not end in "-", must not contain "--"
/// ```
///
/// The dots *are* the hierarchy: `rail.mode.tasks` is a child of `rail.mode`, which is a child of
/// `rail`. Nothing else encodes parentage, and an ancestor need not itself be a marked element —
/// `rail.mode` names a level that exists in the scheme whether or not anything draws it.
///
/// A `UiId` built from a literal is `const`; one parsed at runtime owns its string. Both compare
/// and hash by their text, so the two forms of the same name are the same id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UiId(Cow<'static, str>);

impl UiId {
    /// A name from a literal, checked by the catalogue test rather than at this call.
    ///
    /// `const` so that the catalogue below is a table of constants and a call site can name one
    /// without allocating. Nothing validates the grammar here — a `const fn` cannot — so
    /// `ui_id::catalogue_is_well_formed` (and the test beside it) is what keeps the literals
    /// honest.
    pub const fn new(path: &'static str) -> Self {
        UiId(Cow::Borrowed(path))
    }

    /// A name from a string that arrived at runtime — a help context key, a preference file, an
    /// extension's manifest. This one checks the grammar.
    pub fn parse(path: &str) -> Result<Self, UiIdError> {
        validate(path)?;
        Ok(UiId(Cow::Owned(path.to_owned())))
    }

    /// The name, as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The levels, outermost first. `rail.mode.tasks` yields `rail`, `mode`, `tasks`.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split(SEPARATOR)
    }

    /// The last level — the part that names this place among its siblings.
    pub fn leaf(&self) -> &str {
        self.0.rsplit(SEPARATOR).next().unwrap_or(&self.0)
    }

    /// How many levels deep this is. A root like `rail` is 1.
    pub fn depth(&self) -> usize {
        self.0.split(SEPARATOR).count()
    }

    /// The name one level up, or `None` for a root.
    pub fn parent(&self) -> Option<UiId> {
        let cut = self.0.rfind(SEPARATOR)?;
        Some(UiId(Cow::Owned(self.0[..cut].to_owned())))
    }

    /// Every name above this one, nearest first. `rail.mode.tasks` yields `rail.mode`, `rail`.
    pub fn ancestors(&self) -> impl Iterator<Item = UiId> {
        let mut next = self.parent();
        std::iter::from_fn(move || {
            let here = next.take()?;
            next = here.parent();
            Some(here)
        })
    }

    /// This name with one more level below it.
    pub fn child(&self, segment: &str) -> UiId {
        UiId(Cow::Owned(format!("{}{SEPARATOR}{segment}", self.0)))
    }

    /// Whether `other` sits below this name. A name is not its own ancestor.
    ///
    /// Compared level by level, so `rail` is an ancestor of `rail.mode` but not of `railing`.
    pub fn is_ancestor_of(&self, other: &UiId) -> bool {
        other.0.len() > self.0.len()
            && other.0.starts_with(self.0.as_ref())
            && other.0.as_bytes()[self.0.len()] == SEPARATOR as u8
    }

    /// The name written down for anything outside this crate — [`NAMESPACE`] plus the path.
    ///
    /// Which file carries it, and which field in that file binds a sentence to it, is not settled:
    /// `_docs/wip/in-place-help.md` §2.5 names the two candidates. This is only the spelling, and
    /// it is the same spelling either way.
    pub fn qualified(&self) -> String {
        format!("{NAMESPACE}{}", self.0)
    }

    /// The inverse of [`qualified`](Self::qualified): the name a `ui.*` string addresses, or
    /// `None` for a string in some other namespace or one that is not a legal name.
    pub fn from_qualified(key: &str) -> Option<UiId> {
        UiId::parse(key.strip_prefix(NAMESPACE)?).ok()
    }
}

impl fmt::Display for UiId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for UiId {
    type Err = UiIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        UiId::parse(s)
    }
}

/// The grammar, once, so [`UiId::parse`] and the catalogue check apply the same rule.
pub fn validate(path: &str) -> Result<(), UiIdError> {
    if path.is_empty() {
        return Err(UiIdError::Empty);
    }
    if path.len() > MAX_LEN {
        return Err(UiIdError::TooLong(path.len()));
    }
    let depth = path.split(SEPARATOR).count();
    if depth > MAX_DEPTH {
        return Err(UiIdError::TooDeep(depth));
    }
    for segment in path.split(SEPARATOR) {
        if segment.is_empty() {
            return Err(UiIdError::EmptySegment);
        }
        let bytes = segment.as_bytes();
        let ok = bytes[0].is_ascii_lowercase()
            && bytes[bytes.len() - 1] != b'-'
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
            && !segment.contains("--");
        if !ok {
            return Err(UiIdError::BadSegment(segment.to_owned()));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The catalogue
// ---------------------------------------------------------------------------

/// The rail: the mode switcher down the window's left edge.
pub const RAIL: UiId = UiId::new("rail");
/// The brand mark at the rail's head.
pub const RAIL_MARK: UiId = UiId::new("rail.mark");
/// The column of mode buttons.
pub const RAIL_MODES: UiId = UiId::new("rail.modes");
/// The project badges below the modes.
pub const RAIL_PROJECTS: UiId = UiId::new("rail.projects");

/// One name per [`RailMode`]. Written out rather than derived from the variant, because a
/// `UiId` is a published name: renaming `RailMode::TeamsOld` must not silently move the place a
/// help page and a user's personalisation both point at. The match in [`rail_mode`] is
/// exhaustive, so a *new* mode is a compile error here until it is named.
pub const RAIL_MODE_CONTROL: UiId = UiId::new("rail.mode.control");
pub const RAIL_MODE_IDE: UiId = UiId::new("rail.mode.ide");
pub const RAIL_MODE_GIT: UiId = UiId::new("rail.mode.git");
pub const RAIL_MODE_AGENTS: UiId = UiId::new("rail.mode.agents");
pub const RAIL_MODE_TEAMS: UiId = UiId::new("rail.mode.teams");
pub const RAIL_MODE_TEAMS_ALL: UiId = UiId::new("rail.mode.teams-all");
pub const RAIL_MODE_TEAMS_OLD: UiId = UiId::new("rail.mode.teams-old");
pub const RAIL_MODE_KB: UiId = UiId::new("rail.mode.kb");
pub const RAIL_MODE_TASKS: UiId = UiId::new("rail.mode.tasks");
pub const RAIL_MODE_SINK: UiId = UiId::new("rail.mode.sink");

/// The top row.
pub const TITLEBAR: UiId = UiId::new("titlebar");
/// The window's letter and the project picker at the row's head.
pub const TITLEBAR_PROJECT: UiId = UiId::new("titlebar.project");
/// Add a project, and the chevron offering clone and remote beside it.
pub const TITLEBAR_NEW_PROJECT: UiId = UiId::new("titlebar.new-project");
pub const TITLEBAR_NEW_PROJECT_MENU: UiId = UiId::new("titlebar.new-project-menu");
/// Back and forward through the window's navigation history.
pub const TITLEBAR_NAV_BACK: UiId = UiId::new("titlebar.nav-back");
pub const TITLEBAR_NAV_FORWARD: UiId = UiId::new("titlebar.nav-forward");
/// The middle field: find a file, run a command.
pub const TITLEBAR_COMMAND: UiId = UiId::new("titlebar.command");
/// The right cluster, and each switch and action in it.
pub const TITLEBAR_ACTIONS: UiId = UiId::new("titlebar.actions");
pub const TITLEBAR_REGION_LEFT: UiId = UiId::new("titlebar.region-left");
pub const TITLEBAR_REGION_BOTTOM: UiId = UiId::new("titlebar.region-bottom");
pub const TITLEBAR_REGION_RIGHT: UiId = UiId::new("titlebar.region-right");
pub const TITLEBAR_NEW_AGENT: UiId = UiId::new("titlebar.new-agent");
pub const TITLEBAR_NEW_TERMINAL: UiId = UiId::new("titlebar.new-terminal");
pub const TITLEBAR_NEW_TERMINAL_MENU: UiId = UiId::new("titlebar.new-terminal-menu");
pub const TITLEBAR_SEARCH: UiId = UiId::new("titlebar.search");
pub const TITLEBAR_NOTIFICATIONS: UiId = UiId::new("titlebar.notifications");
pub const TITLEBAR_FEEDBACK: UiId = UiId::new("titlebar.feedback");
pub const TITLEBAR_HELP: UiId = UiId::new("titlebar.help");
pub const TITLEBAR_REMOTE_HOSTS: UiId = UiId::new("titlebar.remote-hosts");
pub const TITLEBAR_OVERFLOW: UiId = UiId::new("titlebar.overflow");
pub const TITLEBAR_THEME: UiId = UiId::new("titlebar.theme");

/// Every name the interface knows.
///
/// This is the answer to "is that a real target?" — for `_tools/helpbundle.py` checking a `ui.*`
/// context key, for a personalisation file naming a control, and for anything that has to list
/// the places a user can be pointed at. A name not in here is a typo, and nothing resolves it.
pub const CATALOGUE: &[UiId] = &[
    RAIL,
    RAIL_MARK,
    RAIL_MODES,
    RAIL_PROJECTS,
    RAIL_MODE_CONTROL,
    RAIL_MODE_IDE,
    RAIL_MODE_GIT,
    RAIL_MODE_AGENTS,
    RAIL_MODE_TEAMS,
    RAIL_MODE_TEAMS_ALL,
    RAIL_MODE_TEAMS_OLD,
    RAIL_MODE_KB,
    RAIL_MODE_TASKS,
    RAIL_MODE_SINK,
    TITLEBAR,
    TITLEBAR_PROJECT,
    TITLEBAR_NEW_PROJECT,
    TITLEBAR_NEW_PROJECT_MENU,
    TITLEBAR_NAV_BACK,
    TITLEBAR_NAV_FORWARD,
    TITLEBAR_COMMAND,
    TITLEBAR_ACTIONS,
    TITLEBAR_REGION_LEFT,
    TITLEBAR_REGION_BOTTOM,
    TITLEBAR_REGION_RIGHT,
    TITLEBAR_NEW_AGENT,
    TITLEBAR_NEW_TERMINAL,
    TITLEBAR_NEW_TERMINAL_MENU,
    TITLEBAR_SEARCH,
    TITLEBAR_NOTIFICATIONS,
    TITLEBAR_FEEDBACK,
    TITLEBAR_HELP,
    TITLEBAR_REMOTE_HOSTS,
    TITLEBAR_OVERFLOW,
    TITLEBAR_THEME,
];

/// Whether a name is in the [`CATALOGUE`].
pub fn is_known(id: &UiId) -> bool {
    CATALOGUE.contains(id)
}

// ---------------------------------------------------------------------------
// What a name means — the one seam the overlay reads
// ---------------------------------------------------------------------------

/// What in-place help says about a place: a short human name and one sentence.
///
/// `label` is what the thing is called out loud — "Tasks", "Back" — which is not the leaf of its
/// path and must not be derived from one: `new-terminal-menu` is "More terminals", and a path is
/// an id rather than a caption. `blurb` is a single sentence saying what the place is *for*,
/// because the unit an inspector balloon shows is a sentence and a manual page is the wrong size
/// for it (`_docs/wip/in-place-help.md` §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetInfo {
    pub label: &'static str,
    pub blurb: &'static str,
}

/// The sentence bound to each name.
///
/// **A `const` table today and not necessarily tomorrow.** §2.5 of the design leaves the binding
/// open and `inbox/ui-id-proposal.md` argues for a `ui-targets.toml` with a packer behind it, so
/// this is deliberately the *only* place the text lives and [`describe`] is the only way to it.
/// Nothing outside this module reads `DESCRIPTIONS`; replacing it with a generated table, or with
/// a file read at startup, is then a change to this module and to nothing else. That boundary is
/// the point — the overlay is throwaway and the names are not.
const DESCRIPTIONS: &[(UiId, TargetInfo)] = &[
    (
        RAIL,
        TargetInfo {
            label: "Activity rail",
            blurb: "The column of destinations down the window's left edge; exactly one mode is \
                    active, and the badges under it switch project.",
        },
    ),
    (
        RAIL_MARK,
        TargetInfo {
            label: "Project mark",
            blurb: "The Ubiq logo on the open project's colour — click it to pick another \
                    project or open a new one.",
        },
    ),
    (
        RAIL_MODES,
        TargetInfo {
            label: "Modes",
            blurb: "Every destination this project offers, grouped; ⌃1 to ⌃9 jump to them in \
                    the order they are drawn.",
        },
    ),
    (
        RAIL_PROJECTS,
        TargetInfo {
            label: "Project badges",
            blurb: "One badge per project open in this window, newest last; ⌘1 to ⌘9 switch \
                    between them.",
        },
    ),
    (
        RAIL_MODE_CONTROL,
        TargetInfo {
            label: "Control",
            blurb: "The window's dashboard: what is running, what needs attention, and the way \
                    back into any of it.",
        },
    ),
    (
        RAIL_MODE_IDE,
        TargetInfo {
            label: "IDE",
            blurb: "The code: the file explorer, the editor and the terminal panes, with a chat \
                    strip beside them.",
        },
    ),
    (
        RAIL_MODE_GIT,
        TargetInfo {
            label: "Git",
            blurb: "The repository — status, diffs, history and branches for the project and \
                    the repos it manages.",
        },
    ),
    (
        RAIL_MODE_AGENTS,
        TargetInfo {
            label: "Agents",
            blurb: "Every conversation in this project side by side, one column each, with the \
                    composer under whichever is being read.",
        },
    ),
    (
        RAIL_MODE_TEAMS,
        TargetInfo {
            label: "Teams",
            blurb: "Groups of agents working a shared brief, and the lanes their work is split \
                    across.",
        },
    ),
    (
        RAIL_MODE_TEAMS_ALL,
        TargetInfo {
            label: "All Teams",
            blurb: "The same canvas over every open project at once, so agents from all of them \
                    are arranged and compared side by side.",
        },
    ),
    (
        RAIL_MODE_TEAMS_OLD,
        TargetInfo {
            label: "Teams (previous)",
            blurb: "The earlier teams screen, kept while the new one settles; it will go when \
                    nothing needs it.",
        },
    ),
    (
        RAIL_MODE_KB,
        TargetInfo {
            label: "Knowledge",
            blurb: "The project's knowledge base: the sources it was built from, and search \
                    across them.",
        },
    ),
    (
        RAIL_MODE_TASKS,
        TargetInfo {
            label: "Tasks",
            blurb: "The project's cards — what is queued, in flight and done, and which agent \
                    has each one.",
        },
    ),
    (
        RAIL_MODE_SINK,
        TargetInfo {
            label: "Kitchen sink",
            blurb: "The developer's reference screen: every control, colour and primitive the \
                    interface is built from.",
        },
    ),
    (
        TITLEBAR,
        TargetInfo {
            label: "Title bar",
            blurb: "The window's top row: which project is open on the left, the command field \
                    in the middle, and the window's actions on the right.",
        },
    ),
    (
        TITLEBAR_PROJECT,
        TargetInfo {
            label: "Project",
            blurb: "The window's letter and the name of the project it is on — click to switch \
                    project, or to open one that is not here yet.",
        },
    ),
    (
        TITLEBAR_NEW_PROJECT,
        TargetInfo {
            label: "Add project",
            blurb: "Open a folder already on this machine as a project in this window.",
        },
    ),
    (
        TITLEBAR_NEW_PROJECT_MENU,
        TargetInfo {
            label: "More ways to add",
            blurb: "The other two ways in: clone a repository, or open a project on a remote \
                    host.",
        },
    ),
    (
        TITLEBAR_NAV_BACK,
        TargetInfo {
            label: "Back",
            blurb: "Return to where this window was standing before — the mode, the tab and the \
                    place in it (⌃-).",
        },
    ),
    (
        TITLEBAR_NAV_FORWARD,
        TargetInfo {
            label: "Forward",
            blurb: "Undo a Back: return to where the window was before it stepped back (⌃⇧-).",
        },
    ),
    (
        TITLEBAR_COMMAND,
        TargetInfo {
            label: "Command field",
            blurb: "One field for finding a file, searching the project and running a command \
                    — ⌘P to go to a file, ⌘⏎ to search.",
        },
    ),
    (
        TITLEBAR_ACTIONS,
        TargetInfo {
            label: "Window actions",
            blurb: "The right-hand cluster: what this window can start, show and put away.",
        },
    ),
    (
        TITLEBAR_REGION_LEFT,
        TargetInfo {
            label: "Left panels",
            blurb: "Show or hide everything docked down the window's left side.",
        },
    ),
    (
        TITLEBAR_REGION_BOTTOM,
        TargetInfo {
            label: "Bottom panels",
            blurb: "Show or hide the strip under the workspace — terminals, logs and the rest.",
        },
    ),
    (
        TITLEBAR_REGION_RIGHT,
        TargetInfo {
            label: "Right panels",
            blurb: "Show or hide everything docked down the window's right side.",
        },
    ),
    (
        TITLEBAR_NEW_AGENT,
        TargetInfo {
            label: "New agent",
            blurb: "Start a conversation: pick the harness, the identity, the model and the \
                    permission level in one question.",
        },
    ),
    (
        TITLEBAR_NEW_TERMINAL,
        TargetInfo {
            label: "New terminal",
            blurb: "Open a shell in the project's folder, in a pane of its own.",
        },
    ),
    (
        TITLEBAR_NEW_TERMINAL_MENU,
        TargetInfo {
            label: "More terminals",
            blurb: "The other shells and harnesses this machine offers, rather than the default \
                    one.",
        },
    ),
    (
        TITLEBAR_SEARCH,
        TargetInfo {
            label: "Search",
            blurb: "Search the whole project and show the results in their own panel (⌘⇧F).",
        },
    ),
    (
        TITLEBAR_NOTIFICATIONS,
        TargetInfo {
            label: "Notifications",
            blurb: "The bell: everything that happened while you were elsewhere, newest first.",
        },
    ),
    (
        TITLEBAR_FEEDBACK,
        TargetInfo {
            label: "Send feedback",
            blurb: "Say what is wrong or missing, with a picture of this window attached.",
        },
    ),
    (
        TITLEBAR_HELP,
        TargetInfo {
            label: "Help",
            blurb: "Open the manual on the page for wherever you are standing (F1).",
        },
    ),
    (
        TITLEBAR_REMOTE_HOSTS,
        TargetInfo {
            label: "Remote hosts",
            blurb: "The machines this window can run panes on, and the state of each connection.",
        },
    ),
    (
        TITLEBAR_OVERFLOW,
        TargetInfo {
            label: "More",
            blurb: "The commands that are wanted occasionally rather than every session — \
                    connect, export, capture, help and settings.",
        },
    ),
    (
        TITLEBAR_THEME,
        TargetInfo {
            label: "Light and dark",
            blurb: "Switch the whole application between the light and the dark palette.",
        },
    ),
];

/// What in-place help says about a name, or `None` for a name nothing has described.
///
/// **This is the seam.** The overlay calls this and never touches the table behind it, so the
/// backing store can become a data file without the overlay changing a line. `None` is a real
/// answer and not a bug: marking is incremental, and a name with no sentence yet is a name the
/// balloon reports as itself rather than refuses to show.
pub fn describe(id: &UiId) -> Option<TargetInfo> {
    DESCRIPTIONS
        .iter()
        .find(|(name, _)| name == id)
        .map(|(_, info)| *info)
}

/// The name of a rail mode's button.
pub fn rail_mode(mode: RailMode) -> UiId {
    match mode {
        RailMode::Control => RAIL_MODE_CONTROL,
        RailMode::Ide => RAIL_MODE_IDE,
        RailMode::Git => RAIL_MODE_GIT,
        RailMode::Agents => RAIL_MODE_AGENTS,
        RailMode::Teams => RAIL_MODE_TEAMS,
        RailMode::TeamsAll => RAIL_MODE_TEAMS_ALL,
        RailMode::TeamsOld => RAIL_MODE_TEAMS_OLD,
        RailMode::Kb => RAIL_MODE_KB,
        RailMode::Tasks => RAIL_MODE_TASKS,
        RailMode::Sink => RAIL_MODE_SINK,
    }
}

/// Every catalogue entry obeys the grammar, is unique, and names a root the catalogue also holds.
///
/// The last of those is what keeps the tree rooted: every name belongs to an area — `rail`,
/// `titlebar` — that is itself a place a reader can be shown, so widening a highlight from a
/// control to the area around it always lands on something named.
pub fn catalogue_is_well_formed() -> Result<(), String> {
    for (i, id) in CATALOGUE.iter().enumerate() {
        validate(id.as_str()).map_err(|e| format!("{id}: {e}"))?;
        if CATALOGUE[..i].contains(id) {
            return Err(format!("{id}: listed twice"));
        }
        let root = id.segments().next().unwrap_or_default();
        if !is_known(&UiId::parse(root).map_err(|e| format!("{id}: {e}"))?) {
            return Err(format!("{id}: root `{root}` is not in the catalogue"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Where things were drawn
// ---------------------------------------------------------------------------

/// A rectangle in window coordinates, in logical pixels.
///
/// Its own type rather than GPUI's `Bounds<Pixels>` so the registry and the hit-test are plain
/// arithmetic that a test can drive with no window behind it. The element side converts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl MarkRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        MarkRect {
            x,
            y,
            width,
            height,
        }
    }

    /// Whether a point falls inside. Half-open on the far edges, so two rectangles sharing an edge
    /// never both claim the same point.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.width > 0.
            && self.height > 0.
            && x >= self.x
            && y >= self.y
            && x < self.x + self.width
            && y < self.y + self.height
    }

    /// The area, for comparing two rectangles at the same depth.
    pub fn area(&self) -> f32 {
        (self.width * self.height).max(0.)
    }
}

/// One marked place, as it was laid out this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct UiMark {
    pub id: UiId,
    pub rect: MarkRect,
}

/// Every mark from the last complete frame, plus the one being built.
///
/// Two buffers, because of when each is used. Marks are recorded during layout, which happens
/// *after* the whole element tree — the overlay included — has been built. So an overlay asking
/// "what is under the cursor" while it is being built can only be answered from the previous
/// frame. [`begin_frame`](Self::begin_frame) retires `current` into `last` and starts a fresh
/// one; [`hit`](Self::hit) reads `last`. One frame of lag on a highlight that follows the mouse
/// is not visible; reading a half-filled list would be.
#[derive(Debug, Default)]
pub struct MarkRegistry {
    last: Vec<UiMark>,
    current: Vec<UiMark>,
}

impl MarkRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Retire the frame that just finished and start collecting the next one. Called once per
    /// frame, from the root of the element tree, before anything is marked.
    pub fn begin_frame(&mut self) {
        std::mem::swap(&mut self.last, &mut self.current);
        self.current.clear();
    }

    /// Record where a marked element was laid out.
    pub fn record(&mut self, id: UiId, rect: MarkRect) {
        self.current.push(UiMark { id, rect });
    }

    /// The marks the last complete frame produced, in the order they were laid out.
    pub fn marks(&self) -> &[UiMark] {
        &self.last
    }

    /// The most specific marked place under a point, or `None` if the cursor is over nothing
    /// marked. See [`deepest_hit`].
    pub fn hit(&self, x: f32, y: f32) -> Option<&UiMark> {
        deepest_hit(&self.last, x, y)
    }

    /// Forget everything. For a window closing, and for a test.
    pub fn clear(&mut self) {
        self.last.clear();
        self.current.clear();
    }
}

/// The most specific mark containing a point.
///
/// "Most specific" is decided in three steps, and the order matters:
///
/// 1. **Depth.** `rail.mode.tasks` beats `rail`, because the deeper name is the more precise
///    description of the same pixel. This is the whole point of the scheme being hierarchical.
/// 2. **Area.** Two names at the same depth that both contain the point are overlapping siblings;
///    the smaller one is the one the user is pointing at.
/// 3. **Order.** Still tied, so they are the same size in the same place — the later record wins,
///    because layout order is paint order and the later one is the one drawn on top.
///
/// Nothing here assumes a mark's ancestors were also marked, and nothing assumes a child's
/// rectangle sits inside its parent's: an anchored menu is a child of its trigger by name and
/// nowhere near it on screen, and it still hit-tests correctly.
pub fn deepest_hit<'a>(marks: &'a [UiMark], x: f32, y: f32) -> Option<&'a UiMark> {
    let mut best: Option<&UiMark> = None;
    for mark in marks {
        if !mark.rect.contains(x, y) {
            continue;
        }
        let better = match best {
            None => true,
            Some(current) => {
                let (d, c) = (mark.id.depth(), current.id.depth());
                d > c || (d == c && mark.rect.area() <= current.rect.area())
            }
        };
        if better {
            best = Some(mark);
        }
    }
    best
}
