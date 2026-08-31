//! The window registry: a projection of the host's catalogue, and which window holds which project.
//!
//! This is the one piece of workbench state that is process-wide rather than per window. It has to
//! be: a project is open in exactly one window at a time, so no window can answer "where is this
//! project open?" from its own copy. Every window reads the same registry and redraws when it
//! changes.
//!
//! **The catalogue is not here.** It belongs to the host, and what this holds is a projection of
//! it, keyed by id and replaced wholesale or one snapshot at a time as the host says so. Because
//! project messages are broadcast, every window applies the same snapshot and the projection is
//! idempotent by construction.
//!
//! Two rules the whole feature rests on. **A project is open in at most one window** — opening it
//! somewhere takes it from wherever it was. And **a window with no project open has nothing to
//! show**, so it is closed — except when the catalogue is empty, where a window with nothing open
//! still has an "Add a project…" to offer, and closing it would quit the application at boot.

use std::collections::BTreeMap;

use gpui::{App, Global, WindowId};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::ProjectSnapshot;

/// One live window: the letter it is named by, and the projects open in it.
#[derive(Clone, Debug)]
pub struct WindowSlot {
    pub id: WindowId,
    /// `A`, `B`, `C`… — what the picker prints beside every project this window holds.
    pub label: char,
    /// The projects open in this window, in the order the picker lists them.
    pub projects: Vec<ProjectId>,
    /// Which of `projects` the window is currently pointed at.
    pub active: usize,
}

impl WindowSlot {
    pub fn active_project(&self) -> Option<ProjectId> {
        self.projects.get(self.active).copied()
    }

    pub fn holds(&self, project: ProjectId) -> bool {
        self.projects.contains(&project)
    }
}

/// The picker's three groups, for one window.
///
/// The rows in `elsewhere` carry the window that holds them, because that row's two actions — go
/// there, or take it from there — both need it.
#[derive(Default)]
pub struct ProjectGroups {
    pub here: Vec<ProjectId>,
    pub elsewhere: Vec<(ProjectId, char, WindowId)>,
    pub history: Vec<ProjectId>,
}

#[derive(Default)]
pub struct WindowRegistry {
    /// The host's catalogue, as this process last heard it. A `BTreeMap` because a ULID sorts by
    /// creation time, so iteration is the order projects were added, at no cost.
    projects: BTreeMap<ProjectId, ProjectSnapshot>,
    /// One slot per live window, in the order they were opened.
    pub windows: Vec<WindowSlot>,
}

impl Global for WindowRegistry {}

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

    /// Whether the host has told us about any project at all.
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    pub fn len(&self) -> usize {
        self.projects.len()
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
    pub fn holder(&self, project: ProjectId) -> Option<&WindowSlot> {
        self.windows.iter().find(|w| w.holds(project))
    }

    pub fn project(&self, id: ProjectId) -> Option<&ProjectSnapshot> {
        self.projects.get(&id)
    }

    /// Every project, in creation order.
    pub fn all(&self) -> impl Iterator<Item = &ProjectSnapshot> {
        self.projects.values()
    }

    /// The project a window should open on when it is given no choice: the one used most recently,
    /// falling back to the most recently created.
    pub fn most_recent(&self) -> Option<ProjectId> {
        self.projects
            .values()
            .max_by_key(|p| (p.record.last_opened_at, p.record.created_at))
            .map(|p| p.record.id)
    }

    // ── the projection ──────────────────────────────────────────────

    /// Replace the whole catalogue, as `ProjectList` says it is.
    ///
    /// A window holding a project that no longer exists loses it, but is **not** reaped here: the
    /// catalogue arriving is not the user closing anything.
    pub fn replace_all(&mut self, projects: Vec<ProjectSnapshot>) {
        self.projects = projects.into_iter().map(|p| (p.record.id, p)).collect();
        let known: Vec<ProjectId> = self.projects.keys().copied().collect();
        for slot in &mut self.windows {
            slot.projects.retain(|id| known.contains(id));
            slot.active = slot.active.min(slot.projects.len().saturating_sub(1));
        }
    }

    /// Apply one snapshot, whether it is new or a change to one already held.
    pub fn apply(&mut self, project: ProjectSnapshot) {
        self.projects.insert(project.record.id, project);
    }

    /// The host has forgotten a project. Answers the windows this emptied.
    pub fn forget(&mut self, id: ProjectId) -> Vec<WindowId> {
        self.projects.remove(&id);
        for slot in &mut self.windows {
            remove(slot, id);
        }
        self.reap()
    }

    // ── which window holds what ─────────────────────────────────────

    /// Add a window, optionally pointed at one project taken from whichever window held it.
    /// Answers the windows that opening it emptied, which the caller closes.
    pub fn register(
        &mut self,
        id: WindowId,
        label: char,
        project: Option<ProjectId>,
    ) -> Vec<WindowId> {
        let project = project.filter(|p| self.projects.contains_key(p));
        if let Some(project) = project {
            self.release(project);
        }
        self.windows.push(WindowSlot {
            id,
            label,
            projects: project.into_iter().collect(),
            active: 0,
        });
        self.reap()
    }

    /// Drop a window's slot. Everything it held goes back to history.
    pub fn unregister(&mut self, id: WindowId) {
        self.windows.retain(|w| w.id != id);
    }

    /// Open a project in a window, taking it from any other. Answers the windows this emptied.
    pub fn open_in(&mut self, id: WindowId, project: ProjectId) -> Vec<WindowId> {
        if !self.projects.contains_key(&project) {
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
    pub fn activate(&mut self, id: WindowId, project: ProjectId) {
        if let Some(slot) = self.windows.iter_mut().find(|w| w.id == id)
            && let Some(at) = slot.projects.iter().position(|p| *p == project)
        {
            slot.active = at;
        }
    }

    /// Close a project in a window. Answers the windows this emptied.
    pub fn close(&mut self, id: WindowId, project: ProjectId) -> Vec<WindowId> {
        if let Some(slot) = self.windows.iter_mut().find(|w| w.id == id) {
            remove(slot, project);
        }
        self.reap()
    }

    /// The three groups one window's picker shows, filtered on name and path so a path fragment
    /// finds a project too.
    pub fn groups(&self, id: WindowId, filter: &str) -> ProjectGroups {
        let needle = filter.trim().to_lowercase();
        let matches = |project: ProjectId| {
            let Some(snapshot) = self.projects.get(&project) else {
                return false;
            };
            needle.is_empty()
                || snapshot.record.name.to_lowercase().contains(&needle)
                || snapshot.record.path.to_lowercase().contains(&needle)
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

        // Most recently used first, and a project never opened last. With a real catalogue the
        // order things were added is not the order anybody wants to see them in.
        let mut history: Vec<&ProjectSnapshot> = self
            .projects
            .values()
            .filter(|p| self.holder(p.record.id).is_none())
            .filter(|p| matches(p.record.id))
            .collect();
        history.sort_by(|a, b| {
            b.record
                .last_opened_at
                .cmp(&a.record.last_opened_at)
                .then_with(|| b.record.created_at.cmp(&a.record.created_at))
        });
        groups.history = history.into_iter().map(|p| p.record.id).collect();

        groups
    }

    /// Take a project out of whatever window holds it. A project is open in one window at a time,
    /// so every open goes through here first.
    fn release(&mut self, project: ProjectId) {
        for slot in &mut self.windows {
            remove(slot, project);
        }
    }

    /// Drop every window left with nothing open, and answer which they were.
    ///
    /// **Except when the catalogue is empty.** A window with no project normally has nothing to
    /// show, but with nothing to open it still offers "Add a project…", and reaping it would quit
    /// the application on a first run.
    fn reap(&mut self) -> Vec<WindowId> {
        if self.projects.is_empty() {
            return Vec::new();
        }
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
fn remove(slot: &mut WindowSlot, project: ProjectId) {
    let Some(at) = slot.projects.iter().position(|p| *p == project) else {
        return;
    };
    slot.projects.remove(at);
    slot.active = slot.active.min(slot.projects.len().saturating_sub(1));
}
