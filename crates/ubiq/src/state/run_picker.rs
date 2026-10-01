//! The run picker's rows: which tools the titlebar's run chevron offers, and in what order.
//!
//! Not held anywhere — the rows are the query's answer, built afresh by [`rows`] from the host's
//! listing and the project's favourites and recents, a free function that names neither
//! `AppState` nor a window. A row is an **index into the listing** (`WorkbenchState::tools`),
//! because the listing is where the tool is and a second copy of it would be one more thing to
//! fall out of step.

use std::collections::HashSet;

use ubiq_proto::projects::Scope;
use ubiq_proto::tools::{ListedTool, ToolOrigin};

use crate::state::navigator::subsequence;

/// How many runs a project remembers.
pub const RECENTS_MAX: usize = 20;

/// How many of those the picker's Recent section shows.
pub const RECENT_SHOWN: usize = 3;

/// Which part of the picker a row is drawn in. **The order here is the order they are drawn.**
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    /// Starred, in the order they were starred.
    Favorites,
    /// The last few run and not starred, newest first.
    Recent,
    /// Rows the user wrote, the project's own before the machine's.
    Defined,
    /// Targets the host found in a runner file: those run before first, then the rest.
    Discovered,
}

impl Section {
    /// The small heading over a section, or `None` for one drawn without.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Section::Favorites => Some("Favorites"),
            Section::Recent => Some("Recent"),
            Section::Defined | Section::Discovered => None,
        }
    }
}

/// One offer: a tool of the listing, and the section it is drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PickerRow {
    pub section: Section,
    /// Index into the listing the rows were built from.
    pub index: usize,
}

/// What the picker offers for a filter.
///
/// No tool appears twice: each section skips what an earlier one took. An inapplicable tool is
/// never offered, the host's own stamp being what says so. A non-empty filter keeps every section
/// and filters inside it, by the same subsequence match the navigator uses.
pub fn rows(
    tools: &[ListedTool],
    favorites: &[String],
    recents: &[String],
    filter: &str,
) -> Vec<PickerRow> {
    let needle = filter.trim().to_lowercase();
    let keys: Vec<String> = tools
        .iter()
        .map(|listed| listed.tool.id.to_string())
        .collect();
    let offered = |at: usize| tools[at].applicable && subsequence(&needle, &tools[at].tool.name);
    let find = |key: &String| keys.iter().position(|k| k == key).filter(|&at| offered(at));
    let discovered = |at: usize| matches!(tools[at].origin, ToolOrigin::Discovered { .. });

    let mut taken: HashSet<usize> = HashSet::new();
    let mut out = Vec::new();
    let mut push = |section, at: usize, taken: &mut HashSet<usize>| {
        if taken.insert(at) {
            out.push(PickerRow { section, index: at });
            true
        } else {
            false
        }
    };

    for at in favorites.iter().filter_map(find) {
        push(Section::Favorites, at, &mut taken);
    }

    let mut shown = 0;
    for at in recents.iter().filter_map(find) {
        if shown == RECENT_SHOWN {
            break;
        }
        if push(Section::Recent, at, &mut taken) {
            shown += 1;
        }
    }

    // The project's own rows before the machine's: the closer list is the likelier reach.
    for project in [true, false] {
        for (at, listed) in tools.iter().enumerate() {
            if offered(at)
                && !discovered(at)
                && matches!(listed.scope, Scope::Project(_)) == project
            {
                push(Section::Defined, at, &mut taken);
            }
        }
    }

    for at in recents.iter().filter_map(find).filter(|&at| discovered(at)) {
        push(Section::Discovered, at, &mut taken);
    }
    for at in (0..tools.len()).filter(|&at| offered(at) && discovered(at)) {
        push(Section::Discovered, at, &mut taken);
    }
    out
}

/// Put a run at the front of a project's recents: deduped, newest first, capped.
pub fn remember(recents: &mut Vec<String>, key: String) {
    recents.retain(|seen| *seen != key);
    recents.insert(0, key);
    recents.truncate(RECENTS_MAX);
}

/// Star a tool, or unstar it. A new star goes to the end: favourites keep the order they were
/// chosen in.
pub fn toggle_favorite(favorites: &mut Vec<String>, key: &str) {
    match favorites.iter().position(|seen| seen == key) {
        Some(at) => {
            favorites.remove(at);
        }
        None => favorites.push(key.to_string()),
    }
}
