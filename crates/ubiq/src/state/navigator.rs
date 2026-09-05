//! The ⌘K navigator: one field over everywhere the window can go.
//!
//! Flat, single-select, and it answers nobody — which is why it is a **sibling** of the file
//! picker rather than a second use of it. There is no picked set, no commit and no owner waiting
//! for a reply: a row is pressed and the window is somewhere else.
//!
//! The rows are not held anywhere. They are the query's answer, built afresh from the window's own
//! lists by [`rows`] — a free function that names neither `AppState` nor a window, which is what
//! makes it testable and costs nothing.

use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::repos::parse_repo_url;
use ubiq_proto::work::AgentId;

use crate::state::nav::{Bookmark, Destination, View};

/// How many rows one group offers. Enough to recognise the one you meant, short enough that the
/// list is read rather than scrolled.
const GROUP_MAX: usize = 8;

/// How many places a project remembers arriving at.
pub const RECENTS_MAX: usize = 32;

/// Which list a row came from. **The order here is the order the groups are drawn in.**
///
/// There is no `Commands` group: a command is not a place, and an action catalogue is a pass over
/// every action in the crate for one consumer. Backlogged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Uri,
    /// A repository URL, offered as a clone rather than as a place. The only group whose row goes
    /// nowhere: it opens the clone modal instead — see [`NavAction`].
    Clone,
    Recent,
    Bookmark,
    File,
    Task,
    Agent,
}

impl Group {
    /// The heading this group is drawn under.
    pub fn label(self) -> &'static str {
        match self {
            Group::Uri => "Link",
            Group::Clone => "Clone",
            Group::Recent => "Recent",
            Group::Bookmark => "Bookmarks",
            Group::File => "Files",
            Group::Task => "Tasks",
            Group::Agent => "Agents",
        }
    }
}

/// One offer: one line, and where it goes.
#[derive(Clone, PartialEq, Debug)]
pub struct NavRow {
    pub group: Group,
    pub label: String,
    pub detail: String,
    /// Where the row goes. `None` is a row that only *says* something — a link to a project this
    /// catalogue does not hold — and it is not clickable.
    pub dest: Option<Destination>,
    /// Whether the bookmark behind the row has lost its line. Drawn, never repaired.
    pub adrift: bool,
    /// What pressing the row does when it is not a place. Checked before `dest`, because a row
    /// carrying an action never carries a destination too.
    pub action: Option<NavAction>,
}

/// What a row does instead of going somewhere.
///
/// One variant, and an enum rather than a bool: the navigator is where every "type this and
/// something happens" lands, and the next one is a variant rather than a second field.
#[derive(Clone, PartialEq, Debug)]
pub enum NavAction {
    /// Open the clone modal with this URL already in its field.
    Clone(String),
}

/// The navigator, while it is up: what was typed, and which row the keyboard is on.
#[derive(Default, Debug)]
pub struct NavigatorState {
    pub query: String,
    pub cursor: usize,
}

/// What the navigator offers for a query.
///
/// Empty query is "where was I": recents, then what was written down, and **no files** — a file
/// list with nothing typed is just the explorer. A non-empty one offers every group, capped and
/// stopped at [`GROUP_MAX`] each.
#[allow(clippy::too_many_arguments)]
pub fn rows(
    query: &str,
    project: ProjectId,
    recents: &[Destination],
    bookmarks: &[Bookmark],
    // Name, path, and whether it is a folder — the explorer's rows, already filtered by it.
    files: &[(String, String, bool)],
    tasks: &[(TaskId, String)],
    // Id, name, role.
    agents: &[(AgentId, String, String)],
    name_of: &dyn Fn(ProjectId) -> Option<String>,
) -> Vec<NavRow> {
    let query = query.trim();

    // A pasted link is an answer rather than a filter: it names one place, so the list collapses
    // to it.
    if let Ok(dest) = query.parse::<Destination>() {
        return vec![match name_of(dest.project) {
            // The project's **name**, never its ULID: a link is read, not decoded.
            Some(name) => NavRow {
                group: Group::Uri,
                label: format!("{name} · {}", dest.label()),
                detail: query.to_string(),
                dest: Some(dest),
                adrift: false,
                action: None,
            },
            None => NavRow {
                group: Group::Uri,
                label: "Unknown project".to_string(),
                detail: query.to_string(),
                dest: None,
                adrift: false,
                action: None,
            },
        }];
    }

    // A repository URL is likewise an answer rather than a filter: it names one repository, so
    // the list collapses to the one thing that can be done with it.
    if let Some(parsed) = parse_repo_url(query) {
        return vec![NavRow {
            group: Group::Clone,
            label: format!("Clone {}/{}", parsed.owner, parsed.name),
            detail: parsed.host.clone(),
            dest: None,
            adrift: false,
            action: Some(NavAction::Clone(query.to_string())),
        }];
    }

    let needle = query.to_lowercase();
    let mut out = Vec::new();

    keep(
        &mut out,
        recents.iter().map(|dest| NavRow {
            group: Group::Recent,
            label: dest.label(),
            detail: kind_of(&dest.view).to_string(),
            dest: Some(dest.clone()),
            adrift: false,
            action: None,
        }),
        &needle,
    );
    keep(
        &mut out,
        bookmarks.iter().map(|mark| NavRow {
            group: Group::Bookmark,
            label: mark.name.clone(),
            detail: mark.dest.label(),
            dest: Some(mark.dest.clone()),
            adrift: mark.adrift,
            action: None,
        }),
        &needle,
    );

    if needle.is_empty() {
        return out;
    }

    keep(
        &mut out,
        files.iter().map(|(name, path, is_dir)| NavRow {
            group: Group::File,
            label: name.clone(),
            detail: path.clone(),
            dest: Some(Destination::new(
                project,
                match is_dir {
                    true => View::Explorer { path: path.clone() },
                    // An unprefixed tab key *is* the path, which is what a plain file opens under.
                    false => View::Ide { key: path.clone() },
                },
            )),
            adrift: false,
            action: None,
        }),
        &needle,
    );
    keep(
        &mut out,
        tasks.iter().map(|(task, title)| NavRow {
            group: Group::Task,
            label: title.clone(),
            detail: String::new(),
            dest: Some(Destination::new(project, View::Tasks { task: *task })),
            adrift: false,
            action: None,
        }),
        &needle,
    );
    keep(
        &mut out,
        agents.iter().map(|(agent, name, role)| NavRow {
            group: Group::Agent,
            label: name.clone(),
            detail: role.clone(),
            dest: Some(Destination::new(project, View::Agents { agent: *agent })),
            adrift: false,
            action: None,
        }),
        &needle,
    );

    out
}

/// Keep a group's first [`GROUP_MAX`] matches, and stop there.
fn keep(out: &mut Vec<NavRow>, rows: impl Iterator<Item = NavRow>, needle: &str) {
    out.extend(rows.filter(|row| matches(needle, row)).take(GROUP_MAX));
}

/// Whether a row answers the query: every character of it, in order, somewhere in the row's own
/// two strings. No scorer and no fuzzy library — a subsequence is what a person typing an
/// abbreviation means, and ranking six short lists is ceremony.
fn matches(needle: &str, row: &NavRow) -> bool {
    subsequence(needle, &format!("{} {}", row.label, row.detail))
}

/// The matcher itself, over one already-lowercased needle and any haystack.
///
/// Shared with the clone modal's repository filter so "does this answer what was typed?" has one
/// answer in the window rather than two that drift apart.
pub(crate) fn subsequence(needle: &str, hay: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hay = hay.to_lowercase();
    let mut have = hay.chars();
    needle.chars().all(|want| have.any(|seen| seen == want))
}

/// What a place is called as a kind, for the second half of a row that already reads as a name.
fn kind_of(view: &View) -> &'static str {
    match view {
        View::Control => "Control",
        View::Kb => "Knowledge",
        View::Git => "Git",
        View::Logs => "Logs",
        View::Ide { .. } => "File",
        View::Explorer { .. } => "Folder",
        View::Terminal { .. } => "Terminal",
        View::Graph { .. } => "Graph",
        View::Agents { .. } => "Agents",
        View::Tasks { .. } => "Task",
        View::Chat { .. } => "Chat",
    }
}

/// Put a place at the front of what a project has just visited.
///
/// Deduped on the **place** rather than on the whole destination: a locus is not part of where you
/// are, so equality would leave thirty-two copies of one file behind a single scroll.
pub fn remember(recents: &mut Vec<String>, dest: &Destination) {
    if !dest.persistable() {
        return;
    }
    recents.retain(|held| {
        !held
            .parse::<Destination>()
            .is_ok_and(|held| held.same_place(dest))
    });
    recents.insert(0, dest.to_string());
    recents.truncate(RECENTS_MAX);
}

/// Read a recents list, keeping what still parses — one lost row rather than a lost list, the same
/// bargain the stored bookmarks make.
pub fn kept_recents(recents: &[String]) -> Vec<Destination> {
    recents
        .iter()
        .filter_map(|text| text.parse().ok())
        .collect()
}
