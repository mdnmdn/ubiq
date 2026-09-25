//! The mission's work breakdown, laid out by dependency level (M20, M21).
//!
//! **A level, not a tree.** A task's prerequisites are a set, so the breakdown is a directed
//! acyclic graph and the only honest arrangement is a topological *layering*: level 0 is
//! everything with no prerequisite inside the mission, and a task's level is one past the deepest
//! prerequisite it names. That is what [`layers`] answers, and it is the whole of the geometry the
//! WBS tab needs — the canvas turns a level and a place in it into a point, and nothing else here
//! knows about pixels.
//!
//! **The two shapes that are not a chain both read.** A mission whose tasks name no prerequisite
//! at all is one level wide and every task is on it; a disconnected graph is several chains laid
//! out against the same levels, which is the arrangement saying they do not depend on each other
//! rather than a broken one. Neither collapses.
//!
//! **A cycle cannot exist** — the host refuses one — but a record read back can still close a
//! loop, so the walk is bounded by the number of tasks rather than trusting the shape.

use std::collections::{HashMap, HashSet};

use ubiq_proto::ids::TaskId;
use ubiq_proto::work::TaskRecord;

/// The breakdown, arranged: which tasks sit on each level, and which of them are on the longest
/// chain through the graph.
#[derive(Clone, Debug, Default)]
pub struct Wbs {
    /// Level 0 first. Each row holds its tasks in the order they were given, so the arrangement is
    /// stable across frames — nothing here sorts by anything the user cannot see.
    pub rows: Vec<Vec<TaskId>>,
    level: HashMap<TaskId, usize>,
    /// The longest prerequisite chain, level 0 first. Empty when there is nothing to draw.
    ///
    /// **Longest by hops, not by estimate.** Nothing on a task measures duration, so a
    /// critical path weighted by time would be a made-up number; the chain that decides how many
    /// rounds the mission takes is the one this highlights.
    pub critical: Vec<TaskId>,
}

impl Wbs {
    pub fn level_of(&self, task: TaskId) -> usize {
        self.level.get(&task).copied().unwrap_or(0)
    }

    pub fn is_critical(&self, task: TaskId) -> bool {
        self.critical.contains(&task)
    }

    /// Where a task sits along its own level.
    pub fn place_of(&self, task: TaskId) -> usize {
        self.rows
            .get(self.level_of(task))
            .and_then(|row| row.iter().position(|held| *held == task))
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// How many tasks the widest level holds — what the canvas sizes itself against.
    pub fn widest(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }
}

/// Lay the tasks out by dependency level.
///
/// Only a prerequisite *inside the set* counts: a mission task waiting on something outside the
/// mission is a root here, and what it waits on is the task detail's to say.
pub fn layers(tasks: &[&TaskRecord]) -> Wbs {
    if tasks.is_empty() {
        return Wbs::default();
    }

    let held: HashSet<TaskId> = tasks.iter().map(|task| task.id).collect();
    let prereqs: HashMap<TaskId, Vec<TaskId>> = tasks
        .iter()
        .map(|task| {
            let mine = task
                .prerequisites
                .iter()
                .copied()
                .filter(|id| held.contains(id) && *id != task.id)
                .collect();
            (task.id, mine)
        })
        .collect();

    // Longest path from a root, relaxed until nothing moves. Bounded by the number of tasks, so a
    // loop that should not exist stops rather than spinning.
    let mut level: HashMap<TaskId, usize> = tasks.iter().map(|task| (task.id, 0usize)).collect();
    for _ in 0..tasks.len() {
        let mut moved = false;
        for task in tasks {
            let want = prereqs[&task.id]
                .iter()
                .map(|id| level[id] + 1)
                .max()
                .unwrap_or(0);
            if want > level[&task.id] {
                level.insert(task.id, want);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    let deepest = level.values().copied().max().unwrap_or(0);
    let mut rows: Vec<Vec<TaskId>> = vec![Vec::new(); deepest + 1];
    for task in tasks {
        rows[level[&task.id]].push(task.id);
    }

    // The chain: start at the deepest task and walk back through the prerequisite that is itself
    // deepest, which is the one the level came from.
    let mut critical = Vec::new();
    if let Some(last) = tasks
        .iter()
        .max_by_key(|task| level[&task.id])
        .map(|task| task.id)
        && level[&last] > 0
    {
        let mut at = last;
        for _ in 0..tasks.len() {
            critical.push(at);
            match prereqs[&at].iter().copied().max_by_key(|id| level[id]) {
                Some(up) => at = up,
                None => break,
            }
        }
        critical.reverse();
    }

    Wbs {
        rows,
        level,
        critical,
    }
}
