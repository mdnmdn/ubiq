//! Where the user is, as a value.
//!
//! A [`Destination`] names any place the window can show: a project, one of its screens, and
//! optionally a spot inside that screen. It is what a link, a card's button, a history entry and a
//! bookmark all reduce to, so a surface can send the user somewhere without knowing the way — the
//! router in `app::nav` is the one thing that knows how to arrive.
//!
//! The **locus is not part of the place**. Arriving where you already are, only at a different
//! line, is not an arrival, which is why [`Destination::same_place`] compares everything but.

mod text;

use std::ops::Range;

use serde::{Deserialize, Serialize};

use ubiq_proto::ids::{PaneId, ProjectId, TaskId};
use ubiq_proto::work::AgentId;

pub use text::{NotALink, resolve_relative};

use crate::state::dock::ChatId;
use crate::state::orchestration::{InspectorTab, Selection};

/// One place: a project, a screen of it, and where in that screen.
///
/// No `Eq` — [`Locus::Viewport`] holds floats — and no serde: the serialised form is the `ubiq://`
/// text, so a view arm with no printable id is never offered a bookmark by construction.
#[derive(Clone, PartialEq, Debug)]
pub struct Destination {
    pub project: ProjectId,
    pub view: View,
    pub locus: Option<Locus>,
}

impl Destination {
    pub fn new(project: ProjectId, view: View) -> Self {
        Self {
            project,
            view,
            locus: None,
        }
    }

    /// Whether two destinations are the same *place*, the locus ignored. What "already there"
    /// means, and what recents dedupe on.
    pub fn same_place(&self, other: &Destination) -> bool {
        self.project == other.project && self.view == other.view
    }

    /// Whether this place will still exist tomorrow, and so may be written down.
    ///
    /// Only a chat is not: its id is minted by this window and means nothing after a restart.
    pub fn persistable(&self) -> bool {
        !matches!(self.view, View::Chat { .. })
    }

    /// The line this place names, where it names one. `Span` answers its first.
    pub fn line(&self) -> Option<u32> {
        match self.locus {
            Some(Locus::Line { line }) => Some(line),
            Some(Locus::Span { from, .. }) => Some(from),
            _ => None,
        }
    }

    /// What to call this place in one line, for a control's tooltip. A path where there is one:
    /// a file is read by its path, not by the arm that holds it.
    pub fn label(&self) -> String {
        match &self.view {
            View::Control => "Control".into(),
            View::Kb => "Knowledge".into(),
            View::Git => "Git".into(),
            View::Logs => "Logs".into(),
            View::Ide { key } => crate::state::editor::from_tab_key(key).0,
            View::Explorer { path } => path.clone(),
            View::Terminal { .. } => "Terminal".into(),
            View::Graph { .. } => "Graph".into(),
            View::Agents { .. } => "Agents".into(),
            View::Tasks { .. } => "Task".into(),
            View::Chat { .. } => "Chat".into(),
        }
    }
}

/// Which screen, and what it is pointed at.
///
/// A file is named by its **tab key** rather than its path, because a file and its diff are two
/// tabs over one path — see [`crate::state::editor::tab_key`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum View {
    Control,
    Kb,
    Git,
    Logs,
    Ide {
        key: String,
    },
    Explorer {
        path: String,
    },
    Terminal {
        pane: PaneId,
    },
    /// The orchestration graph.
    Graph {
        selection: Selection,
        tab: InspectorTab,
    },
    /// The agents columns — the transcript, not the map.
    Agents {
        agent: AgentId,
    },
    Tasks {
        task: TaskId,
    },
    /// Process-local: a chat tab's id is minted by this window and means nothing tomorrow.
    Chat {
        chat: ChatId,
    },
}

/// Where in a screen. Serialised, because it rides a bookmark record beside its anchor text.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Locus {
    Line { line: u32 },
    Span { from: u32, to: u32 },
    Anchor { slug: String },
    Viewport { x: f32, y: f32, scale: f32 },
    Node { key: String },
}

/// The byte offset a one-based line starts at. Total: a line past the end is the end.
pub fn offset_of_line(text: &str, line: u32) -> usize {
    let mut at = 0;
    for _ in 1..line.max(1) {
        match text[at..].find('\n') {
            Some(cut) => at += cut + 1,
            None => return text.len(),
        }
    }
    at
}

/// The byte range covering lines `from` to `to` inclusive, without the last one's newline.
///
/// Reversed bounds are taken the right way round, and both ends are total — the units
/// `set_selected_range` wants.
pub fn line_range(text: &str, from: u32, to: u32) -> Range<usize> {
    let (from, to) = if from <= to { (from, to) } else { (to, from) };
    let start = offset_of_line(text, from);
    let last = offset_of_line(text, to);
    let end = last + text[last..].find('\n').unwrap_or(text.len() - last);
    start..end.max(start)
}

/// A locus as a range in a buffer, where the locus is one the editor owns. A kind it does not —
/// a viewport, a graph node — is `None` and is dropped, never refused.
///
/// An anchor is answered by scanning the **source** for its heading: the rendered preview exposes
/// no scroll-to-anchor, so `#a-heading` lands as a line on the editor side and is simply dropped
/// where only the preview is up.
pub fn range_for(text: &str, locus: &Locus) -> Option<Range<usize>> {
    match locus {
        Locus::Line { line } => Some(line_range(text, *line, *line)),
        Locus::Span { from, to } => Some(line_range(text, *from, *to)),
        Locus::Anchor { slug } => {
            let line = line_of_slug(text, slug)?;
            Some(line_range(text, line, line))
        }
        Locus::Viewport { .. } | Locus::Node { .. } => None,
    }
}

/// The one-based line of the first Markdown heading whose text slugs to this one.
///
/// *First*, because a document with two headings of a name has no better answer than the one a
/// reader following the link would scroll to.
pub fn line_of_slug(text: &str, slug: &str) -> Option<u32> {
    let want = slugify(slug);
    text.lines()
        .position(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with('#') && slugify(trimmed.trim_start_matches('#').trim()) == want
        })
        .map(|ix| ix as u32 + 1)
}

/// The usual lowercase-hyphenate: letters, digits, `-` and `_` survive, a space becomes a hyphen
/// and everything else is dropped. Idempotent, so a slug compares equal to itself.
fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars().flat_map(char::to_lowercase) {
        match ch {
            ' ' => out.push('-'),
            ch if ch.is_alphanumeric() || ch == '-' || ch == '_' => out.push(ch),
            _ => {}
        }
    }
    out
}

/// How many arrivals a window remembers. Old enough to get back to this morning's file, short
/// enough that the list is never worth a data structure.
const HISTORY_MAX: usize = 64;

/// Where a remembered destination stands now, as the window that would go there sees it.
///
/// `Elsewhere` is another window's project: back never moves a project between windows, so the
/// entry is stepped over and left where it is. `Gone` is a project the catalogue no longer holds
/// and there is nothing to keep.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fate {
    Here,
    Elsewhere,
    Gone,
}

/// Every place this window has drawn, and which of them it is standing on.
///
/// Per window, spanning its projects, and not persisted: history is where you have just been, and
/// that does not survive the window that was there.
#[derive(Default, Debug)]
pub struct History {
    pub entries: Vec<Destination>,
    pub at: usize,
}

impl History {
    /// An arrival. The same place again is not one — its locus is refreshed in place, which is how
    /// scrolling a file leaves one entry rather than thirty.
    pub fn record(&mut self, dest: Destination) {
        if let Some(current) = self.entries.get_mut(self.at)
            && current.same_place(&dest)
        {
            current.locus = dest.locus;
            return;
        }
        self.entries.truncate(self.at + 1);
        self.entries.push(dest);
        if self.entries.len() > HISTORY_MAX {
            self.entries.remove(0);
        }
        self.at = self.entries.len() - 1;
    }

    pub fn back(&mut self, live: &dyn Fn(&Destination) -> Fate) -> Option<Destination> {
        self.step(true, live)
    }

    pub fn forward(&mut self, live: &dyn Fn(&Destination) -> Fate) -> Option<Destination> {
        self.step(false, live)
    }

    /// The next live entry in one direction, the cursor moved onto it.
    ///
    /// Removing an entry on the way shifts everything above it down, so the cursor and the probe
    /// are each fixed for the direction they are travelling: going back the cursor is above the
    /// hole, going forward the probe is.
    fn step(&mut self, back: bool, live: &dyn Fn(&Destination) -> Fate) -> Option<Destination> {
        let mut probe = self.at;
        loop {
            if back {
                if probe == 0 {
                    return None;
                }
                probe -= 1;
            } else {
                probe += 1;
                if probe >= self.entries.len() {
                    return None;
                }
            }
            match live(&self.entries[probe]) {
                Fate::Here => {
                    self.at = probe;
                    return Some(self.entries[probe].clone());
                }
                Fate::Elsewhere => {}
                Fate::Gone => {
                    self.entries.remove(probe);
                    if back {
                        self.at -= 1;
                    } else {
                        probe -= 1;
                    }
                }
            }
        }
    }

    /// Where the window is standing, as history has it.
    pub fn current(&self) -> Option<&Destination> {
        self.entries.get(self.at)
    }

    pub fn current_mut(&mut self) -> Option<&mut Destination> {
        self.entries.get_mut(self.at)
    }

    /// What a press in one direction would land on, for the control's label. The neighbour, not
    /// the outcome of a walk: a fate is a frame-time question and a tooltip is not worth it.
    pub fn peek(&self, back: bool) -> Option<&Destination> {
        let at = if back {
            self.at.checked_sub(1)?
        } else {
            self.at + 1
        };
        self.entries.get(at)
    }
}

/// How much of a line is kept as a bookmark's anchor. Long enough to be one line and not another,
/// short enough that a blob of them is still a preference. **Both sides of every comparison are
/// capped to this**, or a line longer than it never matches its own stored anchor.
pub const ANCHOR_CHARS: usize = 120;

/// How far either way a moved line is looked for before the bookmark is called lost.
const ANCHOR_SCAN: u32 = 200;

/// A place written down to come back to.
///
/// The destination is serialised as its `ubiq://` text, so what keeps an old bookmark readable is
/// the parser in [`text`] rather than a set of serde variant names — and one that no longer parses
/// is dropped on load instead of poisoning the whole blob. See [`kept_bookmarks`].
#[derive(Clone, PartialEq, Debug)]
pub struct Bookmark {
    pub name: String,
    pub dest: Destination,
    pub note: String,
    /// The line's own text when the bookmark was made, trimmed and capped, so the mark can find
    /// its line again after the file has been edited above it.
    pub anchor: Option<String>,
    /// Whether the last resolution lost the line. Not written down: it is what the file says now,
    /// and it is re-answered every time the file loads.
    pub adrift: bool,
}

/// The stored shape: the destination as text and nothing else that needs a version.
#[derive(Serialize, Deserialize)]
struct Stored {
    name: String,
    dest: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    anchor: Option<String>,
}

impl Serialize for Bookmark {
    fn serialize<S: serde::Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        Stored {
            name: self.name.clone(),
            dest: self.dest.to_string(),
            note: self.note.clone(),
            anchor: self.anchor.clone(),
        }
        .serialize(out)
    }
}

impl<'de> Deserialize<'de> for Bookmark {
    fn deserialize<D: serde::Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        Stored::deserialize(input)?
            .into_bookmark()
            .ok_or_else(|| serde::de::Error::custom("not a link"))
    }
}

impl Stored {
    fn into_bookmark(self) -> Option<Bookmark> {
        Some(Bookmark {
            name: self.name,
            dest: self.dest.parse().ok()?,
            note: self.note,
            anchor: self.anchor,
            adrift: false,
        })
    }
}

/// Read a bookmark list, keeping what still parses.
///
/// A destination this build no longer understands is one lost bookmark, not a lost preferences
/// blob — which is the whole reason the destination is stored as text.
pub fn kept_bookmarks<'de, D: serde::Deserializer<'de>>(
    input: D,
) -> Result<Vec<Bookmark>, D::Error> {
    Ok(Vec::<Stored>::deserialize(input)?
        .into_iter()
        .filter_map(Stored::into_bookmark)
        .collect())
}

/// Write a place down, or take it away if it is already written down. Twice leaves the list as it
/// was found.
///
/// Free rather than a method on the window so the rule is testable without one.
pub fn toggle_mark(marks: &mut Vec<Bookmark>, mark: Bookmark) {
    match marks.iter().position(|held| held.dest == mark.dest) {
        Some(at) => {
            marks.remove(at);
        }
        None => marks.push(mark),
    }
}

/// What a stored line number turned out to mean once the file was read again.
///
/// `Moved` is a line found somewhere else and re-stamped; `Adrift` is one that could not be found,
/// and **nothing is written for it** — a bookmark quietly pointing at the wrong line is worse than
/// one that says it has lost its place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchored {
    Exact(u32),
    Moved(u32),
    Adrift(u32),
}

impl Anchored {
    pub fn line(self) -> u32 {
        match self {
            Anchored::Exact(line) | Anchored::Moved(line) | Anchored::Adrift(line) => line,
        }
    }
}

/// A line as it compares: trimmed, and cut to [`ANCHOR_CHARS`] like the anchor it is matched with.
fn capped(line: &str) -> String {
    line.trim().chars().take(ANCHOR_CHARS).collect()
}

/// What a bookmark's line has become in the file as it now reads.
///
/// The number is trusted first, so a line that appears twice does not have its bookmark stolen by
/// the other copy; then the line is looked for outward from where it was, nearest first.
pub fn resolve_anchor(text: &str, line: u32, anchor: &str) -> Anchored {
    let lines: Vec<&str> = text.lines().collect();
    let last = lines.len().max(1) as u32;
    let want = capped(anchor);
    let at = |number: u32| {
        lines
            .get(number.checked_sub(1)? as usize)
            .map(|l| capped(l))
    };

    // A number the file no longer reaches has no neighbourhood to search, so the whole file is.
    if line == 0 || line > last {
        return match lines.iter().position(|l| capped(l) == want) {
            Some(index) => Anchored::Moved(index as u32 + 1),
            None => Anchored::Adrift(last),
        };
    }
    if at(line).as_deref() == Some(want.as_str()) {
        return Anchored::Exact(line);
    }
    for step in 1..=ANCHOR_SCAN {
        if let Some(above) = line.checked_sub(step)
            && at(above).as_deref() == Some(want.as_str())
        {
            return Anchored::Moved(above);
        }
        let below = line + step;
        if below <= last && at(below).as_deref() == Some(want.as_str()) {
            return Anchored::Moved(below);
        }
    }
    Anchored::Adrift(line.min(last))
}
