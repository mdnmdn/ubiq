//! The window registry: the project catalogue, and which window holds which project.
//!
//! This is the one piece of workbench state that is process-wide rather than per window. It has to
//! be: a project is open in exactly one window at a time, so no window can answer "where is this
//! project open?" from its own copy. Every window reads the same registry and redraws when it
//! changes.
//!
//! Two rules the whole feature rests on. **A project is open in at most one window** — opening it
//! somewhere takes it from wherever it was. And **a window with no project open has nothing to
//! show**, so it is closed; the registry drops the slot and returns the window's ID for the caller
//! to close.

use gpui::{App, Global, WindowId};

use super::sample;

/// A project a window can be pointed at.
///
/// `colour` indexes the theme's project swatches; it is the project's identity everywhere it
/// appears. `terminals` is how many terminals it has open, which is what makes closing it a
/// question rather than a click. Openness is not a field here — it is whether some window's slot
/// holds this project.
#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub path: String,
    pub colour: usize,
    pub terminals: usize,
    /// Relative time, as the history list shows it.
    pub when: String,
}

/// One live window: the letter it is named by, and the projects open in it.
#[derive(Clone, Debug)]
pub struct WindowSlot {
    pub id: WindowId,
    /// `A`, `B`, `C`… — what the picker prints beside every project this window holds.
    pub label: char,
    /// The projects open in this window, in the order the picker lists them.
    pub projects: Vec<usize>,
    /// Which of `projects` the window is currently pointed at.
    pub active: usize,
}

impl WindowSlot {
    pub fn active_project(&self) -> Option<usize> {
        self.projects.get(self.active).copied()
    }

    pub fn holds(&self, project: usize) -> bool {
        self.projects.contains(&project)
    }
}

/// The picker's three groups, for one window.
///
/// The rows in `elsewhere` carry the window that holds them, because that row's two actions — go
/// there, or take it from there — both need it.
#[derive(Default)]
pub struct ProjectGroups {
    pub here: Vec<usize>,
    pub elsewhere: Vec<(usize, char, WindowId)>,
    pub history: Vec<usize>,
}

pub struct WindowRegistry {
    /// Every project Ubiq knows about, open or only remembered.
    pub projects: Vec<Project>,
    /// One slot per live window, in the order they were opened.
    pub windows: Vec<WindowSlot>,
}

impl Global for WindowRegistry {}

impl Default for WindowRegistry {
    fn default() -> Self {
        Self {
            projects: sample::projects(),
            windows: Vec::new(),
        }
    }
}

impl WindowRegistry {
    /// Seed the registry, once, before the first window is opened.
    pub fn install(cx: &mut App) {
        if !cx.has_global::<Self>() {
            cx.set_global(Self::default());
        }
    }

    /// Read the registry. Every reader goes through here rather than through `default_global`,
    /// which would notify the observers on a plain read and spin the frame.
    pub fn read(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// The lowest letter no live window is using. Letters are reused once a window closes, so the
    /// set of names stays as short as the set of windows.
    pub fn next_label(&self) -> char {
        ('A'..='Z')
            .find(|c| self.windows.iter().all(|w| w.label != *c))
            .unwrap_or('#')
    }

    pub fn slot(&self, id: WindowId) -> Option<&WindowSlot> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// The window a project is open in, if any.
    pub fn holder(&self, project: usize) -> Option<&WindowSlot> {
        self.windows.iter().find(|w| w.holds(project))
    }

    pub fn project(&self, index: usize) -> Option<&Project> {
        self.projects.get(index)
    }

    /// Add a window, pointed at one project taken from whichever window held it. Answers the
    /// windows that opening it emptied, which the caller closes.
    pub fn register(&mut self, id: WindowId, label: char, project: usize) -> Vec<WindowId> {
        let project = project.min(self.projects.len().saturating_sub(1));
        self.release(project);
        self.windows.push(WindowSlot {
            id,
            label,
            projects: vec![project],
            active: 0,
        });
        self.reap()
    }

    /// Drop a window's slot. Everything it held goes back to history.
    pub fn unregister(&mut self, id: WindowId) {
        self.windows.retain(|w| w.id != id);
    }

    /// Open a project in a window, taking it from any other. Answers the windows this emptied.
    pub fn open_in(&mut self, id: WindowId, project: usize) -> Vec<WindowId> {
        if project >= self.projects.len() {
            return Vec::new();
        }
        self.release(project);
        if let Some(slot) = self.windows.iter_mut().find(|w| w.id == id) {
            slot.projects.push(project);
            slot.active = slot.projects.len() - 1;
        }
        self.reap()
    }

    /// Point a window at a project it already holds.
    pub fn activate(&mut self, id: WindowId, project: usize) {
        if let Some(slot) = self.windows.iter_mut().find(|w| w.id == id)
            && let Some(at) = slot.projects.iter().position(|p| *p == project)
        {
            slot.active = at;
        }
    }

    /// Close a project in a window. Answers the windows this emptied.
    pub fn close(&mut self, id: WindowId, project: usize) -> Vec<WindowId> {
        if let Some(slot) = self.windows.iter_mut().find(|w| w.id == id) {
            remove(slot, project);
        }
        self.reap()
    }

    /// The three groups one window's picker shows, filtered on name and path so a path fragment
    /// finds a project too.
    pub fn groups(&self, id: WindowId, filter: &str) -> ProjectGroups {
        let needle = filter.trim().to_lowercase();
        let matches = |index: usize| {
            let Some(project) = self.projects.get(index) else {
                return false;
            };
            needle.is_empty()
                || project.name.to_lowercase().contains(&needle)
                || project.path.to_lowercase().contains(&needle)
        };

        let mut groups = ProjectGroups::default();

        if let Some(slot) = self.slot(id) {
            groups.here = slot
                .projects
                .iter()
                .copied()
                .filter(|p| matches(*p))
                .collect();
        }

        for slot in self.windows.iter().filter(|w| w.id != id) {
            for project in slot.projects.iter().copied().filter(|p| matches(*p)) {
                groups.elsewhere.push((project, slot.label, slot.id));
            }
        }
        groups
            .elsewhere
            .sort_by_key(|(project, label, _)| (*label, *project));

        groups.history = (0..self.projects.len())
            .filter(|p| self.holder(*p).is_none())
            .filter(|p| matches(*p))
            .collect();

        groups
    }

    /// Take a project out of whatever window holds it. A project is open in one window at a time,
    /// so every open goes through here first.
    fn release(&mut self, project: usize) {
        for slot in &mut self.windows {
            remove(slot, project);
        }
    }

    /// Drop every window left with nothing open, and answer which they were. A window with no
    /// project has nothing to show, so its slot goes now and the caller closes the window itself.
    fn reap(&mut self) -> Vec<WindowId> {
        let emptied: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.projects.is_empty())
            .map(|w| w.id)
            .collect();
        self.windows.retain(|w| !w.projects.is_empty());
        emptied
    }
}

/// Remove a project from one slot, keeping `active` on a project that still exists.
fn remove(slot: &mut WindowSlot, project: usize) {
    let Some(at) = slot.projects.iter().position(|p| *p == project) else {
        return;
    };
    slot.projects.remove(at);
    slot.active = slot.active.min(slot.projects.len().saturating_sub(1));
}
