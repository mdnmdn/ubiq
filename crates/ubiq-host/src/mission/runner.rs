//! The mission runner's decisions, as pure functions over a mission's task tree.
//!
//! **Nothing here writes.** [`pass`] reads the parent, the project's tasks and the liveness of
//! their agents and answers a list of [`Step`]s; [`super::Missions`] carries them out on the
//! coordinator's thread. [`needs_you`] and [`snapshot`] derive what a window draws from the same
//! inputs. The log the runner reads back — who was spawned, whether a reviewer gave its verdict —
//! is the [`RunEvent`] comments on the tasks themselves (contract §2).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::mission::{
    Driver, LaunchRole, MissionSettings, MissionSnapshot, NeedsYou, Phase, RunEvent, RunningWorker,
    is_human, worker_gone,
};
use ubiq_proto::work::{AgentId, Comment, Priority, Status, TaskRecord};

/// After this many automatic review fails in a row the reviewed task stops looping and waits for
/// the user (contract §3.6). Always compared with `>=`.
pub const AUTO_FAIL_CAP: u8 = 2;

/// What a pass reads.
pub struct World<'a> {
    pub parent: &'a TaskRecord,
    /// The project's tasks, the parent among them.
    pub tasks: &'a [TaskRecord],
    pub live: &'a dyn Fn(AgentId) -> bool,
    /// Tasks with a launch asked of a window and not yet answered.
    pub in_flight: &'a HashSet<TaskId>,
    /// Tasks whose launch failed this session, until a manual Start or Retry.
    pub launch_failed: &'a HashSet<TaskId>,
    /// The agent definition ids the project can see — what tells a definition from a person.
    pub definitions: &'a [String],
}

impl World<'_> {
    pub fn settings(&self) -> MissionSettings {
        settings_of(self.parent)
    }

    pub fn children(&self) -> impl Iterator<Item = &TaskRecord> {
        let parent = self.parent.id;
        self.tasks.iter().filter(move |t| t.parent == Some(parent))
    }

    fn task(&self, id: TaskId) -> Option<&TaskRecord> {
        self.tasks.iter().find(|t| t.id == id)
    }
}

/// A mission parent's settings; `None` reads as [`MissionSettings::legacy`].
pub fn settings_of(parent: &TaskRecord) -> MissionSettings {
    parent
        .mission
        .clone()
        .unwrap_or_else(MissionSettings::legacy)
}

/// One thing the runner decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Launch an agent on a ready task.
    Launch { task: TaskId, role: LaunchRole },
    /// Move the parent — the mission's phase.
    ParentTo(Status),
    /// A reviewer gave its verdict through `submit_review`; act on it.
    Verdict {
        review: TaskId,
        pass: bool,
        findings: String,
    },
    /// The worker moved its task to review: write the `Finished` line.
    Finished { task: TaskId, agent: AgentId },
    /// The worker is no longer live and the task is still in progress: write the `Gone` line.
    Gone { task: TaskId, agent: AgentId },
    /// The task is closed and its worker still loaded: stop it (Q3).
    Unload { task: TaskId, agent: AgentId },
    /// A review task whose reviewed task was abandoned goes with it.
    Abandon { task: TaskId },
}

// ── reading the log back ────────────────────────────────────────────────

/// The index and the comment of the last `Spawned` line on a task.
fn last_spawned(task: &TaskRecord) -> Option<(usize, &Comment)> {
    task.comments
        .iter()
        .enumerate()
        .rev()
        .find(|(_, c)| matches!(c.event, Some(RunEvent::Spawned { .. })))
}

/// The events written since the last `Spawned`, oldest first (everything, when nothing was
/// spawned).
pub fn since_spawn(task: &TaskRecord) -> impl DoubleEndedIterator<Item = &Comment> {
    let from = last_spawned(task).map_or(0, |(i, _)| i + 1);
    task.comments[from..].iter()
}

/// The role and the time of the last launch on a task.
pub fn spawned_role(task: &TaskRecord) -> Option<(AgentId, LaunchRole, DateTime<Utc>)> {
    last_spawned(task).and_then(|(_, c)| match c.event {
        Some(RunEvent::Spawned { agent, role, .. }) => Some((agent, role, c.created_at)),
        _ => None,
    })
}

/// The live mission reviewer on the parent, if one was launched and is still there.
pub fn mission_reviewer(parent: &TaskRecord, live: &dyn Fn(AgentId) -> bool) -> Option<AgentId> {
    parent
        .comments
        .iter()
        .rev()
        .find_map(|c| match c.event {
            Some(RunEvent::Spawned {
                agent,
                role: LaunchRole::MissionReviewer,
                ..
            }) => Some(agent),
            _ => None,
        })
        .filter(|agent| live(*agent))
}

/// A verdict written since the reviewer was launched.
fn verdict(task: &TaskRecord) -> Option<(bool, String)> {
    since_spawn(task).rev().find_map(|c| match c.event {
        Some(RunEvent::ReviewVerdict { pass }) => Some((pass, c.text.clone())),
        _ => None,
    })
}

fn noted_since_spawn(task: &TaskRecord, test: impl Fn(&RunEvent) -> bool) -> bool {
    since_spawn(task).any(|c| c.event.as_ref().is_some_and(&test))
}

/// The pending ask the planner made, if it is newer than the parent's last phase move.
pub fn pending_request(parent: &TaskRecord) -> Option<Phase> {
    for c in parent.comments.iter().rev() {
        match &c.event {
            Some(RunEvent::Phase { .. }) => return None,
            Some(RunEvent::PhaseRequested { to, .. }) => return Some(*to),
            _ => {}
        }
    }
    None
}

/// When the latest breakdown that followed a Running phase began — the baseline the replan
/// marks read against.
pub fn replan_at(parent: &TaskRecord) -> Option<DateTime<Utc>> {
    let mut ran = false;
    let mut at = None;
    for c in &parent.comments {
        match c.event {
            Some(RunEvent::Phase { to: Phase::Running }) => ran = true,
            Some(RunEvent::Phase {
                to: Phase::Breakdown,
            }) if ran => at = Some(c.created_at),
            _ => {}
        }
    }
    at
}

// ── readiness ───────────────────────────────────────────────────────────

/// Which definition runs a task: `assigned_to` when it names one, else the mission's worker,
/// else the window's default.
pub fn definition_for(
    task: &TaskRecord,
    settings: &MissionSettings,
    defs: &[String],
) -> Option<String> {
    task.assigned_to
        .as_deref()
        .map(str::trim)
        .filter(|who| defs.iter().any(|d| d == who))
        .map(str::to_string)
        .or_else(|| settings.worker.clone())
}

/// The role a task's agent is launched as.
pub fn role_for(task: &TaskRecord) -> LaunchRole {
    if task.review_of.is_some() {
        LaunchRole::Reviewer
    } else {
        LaunchRole::Worker
    }
}

/// Whether a review task's reviewed task has hit the auto-fail cap — the reviewer must not be
/// relaunched until the user decides.
fn capped(world: &World, task: &TaskRecord) -> bool {
    task.review_of
        .and_then(|of| world.task(of))
        .is_some_and(|reviewed| reviewed.auto_fails >= AUTO_FAIL_CAP)
}

/// Every prerequisite satisfied, ignoring what the task's own status says.
fn unblocked(world: &World, task: &TaskRecord) -> bool {
    task.ready(world.tasks)
}

/// The tasks the runner may start now, in the order it starts them (contract §3.2, 1–2).
pub fn ready(world: &World) -> Vec<TaskId> {
    let mut ready: Vec<&TaskRecord> = world
        .children()
        .filter(|t| {
            t.status == Status::Ready
                && t.worker.is_none()
                && !is_human(t, world.definitions)
                && !world.in_flight.contains(&t.id)
                && !world.launch_failed.contains(&t.id)
                && !capped(world, t)
                && unblocked(world, t)
        })
        .collect();
    let children: Vec<&TaskRecord> = world.children().collect();
    let levels = wbs_levels(&children);
    let level = |id: TaskId| {
        levels
            .iter()
            .find(|(held, _)| *held == id)
            .map_or(usize::MAX, |(_, level)| *level)
    };
    ready.sort_by(|a, b| {
        priority_rank(b.priority)
            .cmp(&priority_rank(a.priority))
            .then(level(a.id).cmp(&level(b.id)))
            .then(a.created_at.cmp(&b.created_at))
    });
    ready.into_iter().map(|t| t.id).collect()
}

/// Each child's depth in the prerequisite graph inside the mission: 0 for one that waits on no
/// sibling, one more than its deepest sibling prerequisite otherwise. A cycle leaves its members
/// out (they sort last).
fn wbs_levels(children: &[&TaskRecord]) -> Vec<(TaskId, usize)> {
    let mut levels: Vec<(TaskId, usize)> = Vec::new();
    for _ in 0..children.len() {
        let mut settled = false;
        for task in children {
            if levels.iter().any(|(id, _)| *id == task.id) {
                continue;
            }
            let deepest = task
                .prerequisites
                .iter()
                .filter(|id| children.iter().any(|child| child.id == **id))
                .map(|id| {
                    levels
                        .iter()
                        .find(|(held, _)| held == id)
                        .map(|(_, level)| *level)
                })
                .collect::<Option<Vec<_>>>();
            if let Some(deepest) = deepest {
                let level = deepest.into_iter().max().map_or(0, |level| level + 1);
                levels.push((task.id, level));
                settled = true;
            }
        }
        if !settled {
            break;
        }
    }
    levels
}

fn priority_rank(priority: Priority) -> u8 {
    match priority {
        Priority::Low => 0,
        Priority::Normal => 1,
        Priority::High => 2,
    }
}

// ── the pass ────────────────────────────────────────────────────────────

/// One pass of the runner (contract §3.2).
///
/// The bookkeeping lines (`Finished`, `Gone`), the unloads of closed tasks and the abandon
/// cascade run whenever the mission is open; verdicts, launches and completion only while it is
/// Running; the step back from Review only in Review. A pass that answers a [`Step::Verdict`]
/// answers nothing else: the caller applies it and passes again over the new state.
pub fn pass(world: &World) -> Vec<Step> {
    let mut steps = Vec::new();
    let status = world.parent.status;
    if matches!(status, Status::Done | Status::Abandoned) {
        return steps;
    }

    for child in world.children() {
        if let Some(of) = child.review_of
            && world
                .task(of)
                .is_some_and(|t| t.status == Status::Abandoned)
            && !matches!(child.status, Status::Done | Status::Abandoned)
        {
            steps.push(Step::Abandon { task: child.id });
        }
        let Some(agent) = child.worker else {
            continue;
        };
        let live = (world.live)(agent);
        match child.status {
            Status::Done | Status::Abandoned if live => {
                steps.push(Step::Unload {
                    task: child.id,
                    agent,
                });
            }
            Status::InReview
                if !noted_since_spawn(child, |e| matches!(e, RunEvent::Finished { .. })) =>
            {
                steps.push(Step::Finished {
                    task: child.id,
                    agent,
                });
            }
            Status::InProgress
                if worker_gone(child, world.live)
                    && !world.in_flight.contains(&child.id)
                    && !noted_since_spawn(child, |e| matches!(e, RunEvent::Gone { .. })) =>
            {
                steps.push(Step::Gone {
                    task: child.id,
                    agent,
                });
            }
            _ => {}
        }
    }

    match status {
        Status::InProgress => {
            let verdicts: Vec<Step> = world
                .children()
                // Whatever the review task's status now — a reviewer that gave its verdict and
                // then moved its task still gave it. Applying the verdict clears `worker`, which
                // is what marks it consumed.
                .filter(|t| {
                    t.review_of.is_some() && t.worker.is_some() && t.status != Status::Abandoned
                })
                .filter_map(|t| {
                    verdict(t).map(|(pass, findings)| Step::Verdict {
                        review: t.id,
                        pass,
                        findings,
                    })
                })
                .collect();
            if !verdicts.is_empty() {
                steps.extend(verdicts);
                return steps;
            }

            let settings = world.settings();
            if settings.driver == Driver::Auto {
                let mut running = world
                    .children()
                    .filter(|t| {
                        t.status == Status::InProgress && t.worker.is_some_and(|a| (world.live)(a))
                    })
                    .count()
                    + world
                        .children()
                        .filter(|t| world.in_flight.contains(&t.id))
                        .count();
                for task in ready(world) {
                    if running >= settings.parallelism() {
                        break;
                    }
                    let role = world.task(task).map_or(LaunchRole::Worker, role_for);
                    steps.push(Step::Launch { task, role });
                    running += 1;
                }
            }

            if complete(world) {
                steps.push(Step::ParentTo(Status::InReview));
            }
        }
        Status::InReview => {
            if world
                .children()
                .any(|t| matches!(t.status, Status::Ready | Status::InProgress))
            {
                steps.push(Step::ParentTo(Status::InProgress));
            }
        }
        _ => {}
    }
    steps
}

/// Contract §3.2, 5: nothing left open, nothing waiting on a confirmation, no review capped.
fn complete(world: &World) -> bool {
    !world.children().any(|t| {
        matches!(
            t.status,
            Status::Backlog | Status::Ready | Status::InProgress | Status::Blocked
        ) || (t.status == Status::InReview && (t.approve || t.auto_fails >= AUTO_FAIL_CAP))
    })
}

// ── what a window draws ─────────────────────────────────────────────────

/// What needs the user, derived (contract §1.1 `NeedsYou`).
pub fn needs_you(world: &World, open_threads: usize) -> Vec<NeedsYou> {
    let mut out = Vec::new();
    let phase = Phase::of(world.parent.status);
    let settings = world.settings();

    match (phase, pending_request(world.parent)) {
        (Phase::Planning, Some(Phase::Breakdown)) => out.push(NeedsYou::PlanReady { open_threads }),
        (Phase::Breakdown, Some(Phase::Running)) => out.push(NeedsYou::TasksDrafted),
        _ => {}
    }
    if matches!(phase, Phase::Planning | Phase::Breakdown)
        && !world.in_flight.contains(&world.parent.id)
        && !settings.planner_agent.is_some_and(|a| (world.live)(a))
    {
        out.push(NeedsYou::PlannerStopped);
    }

    if matches!(phase, Phase::Running | Phase::Paused | Phase::Review) {
        for t in world.children() {
            if t.status == Status::InReview && t.auto_fails >= AUTO_FAIL_CAP {
                out.push(NeedsYou::ReviewFailedTwice { task: t.id });
            } else if t.status == Status::InReview && t.approve {
                out.push(NeedsYou::AwaitingApproval { task: t.id });
            }
            if matches!(t.status, Status::Ready | Status::InProgress)
                && is_human(t, world.definitions)
                && unblocked(world, t)
            {
                out.push(NeedsYou::HumanTask { task: t.id });
            }
            // In progress with no live agent on it — a dead worker, or none at all (a hand
            // move) — is gone: Retry is the button that gets it going again.
            let unworked = t.status == Status::InProgress
                && !t.worker.is_some_and(|a| (world.live)(a))
                && !is_human(t, world.definitions);
            if !world.in_flight.contains(&t.id)
                && (worker_gone(t, world.live)
                    || unworked
                    || (t.status == Status::Ready && world.launch_failed.contains(&t.id)))
            {
                out.push(NeedsYou::WorkerGone { task: t.id });
            }
        }
    }
    if phase == Phase::Review {
        out.push(NeedsYou::ReadyForReview {
            left_in_review: world
                .children()
                .filter(|t| t.status == Status::InReview)
                .map(|t| t.id)
                .collect(),
        });
    }
    out
}

/// The live agents working tasks of the mission.
pub fn running(world: &World) -> Vec<RunningWorker> {
    let mut out: Vec<RunningWorker> = world
        .children()
        .filter(|t| t.status == Status::InProgress)
        .filter_map(|t| {
            let agent = t.worker.filter(|a| (world.live)(*a))?;
            let (_, role, since) = spawned_role(t).unwrap_or((agent, role_for(t), t.updated_at));
            Some(RunningWorker {
                task: t.id,
                agent,
                role,
                since,
            })
        })
        .collect();
    if let Some(agent) = mission_reviewer(world.parent, world.live) {
        let since = spawned_role(world.parent).map_or(world.parent.updated_at, |(_, _, at)| at);
        out.push(RunningWorker {
            task: world.parent.id,
            agent,
            role: LaunchRole::MissionReviewer,
            since,
        });
    }
    out
}

/// Every live agent of the mission: the planner, the workers and reviewers (idle ones
/// included), the mission reviewer.
pub fn agents(world: &World) -> Vec<AgentId> {
    let mut out = Vec::new();
    let mut push = |a: AgentId| {
        if (world.live)(a) && !out.contains(&a) {
            out.push(a);
        }
    };
    if let Some(planner) = world.settings().planner_agent {
        push(planner);
    }
    for t in world.children() {
        if let Some(a) = t.worker {
            push(a);
        }
    }
    if let Some(a) = mission_reviewer(world.parent, world.live) {
        push(a);
    }
    out
}

/// The mission as a window draws it. `documents` and `open_threads` are the two facts read off
/// disk, by the caller.
pub fn snapshot(
    project: ProjectId,
    world: &World,
    documents: Vec<String>,
    open_threads: usize,
) -> MissionSnapshot {
    let settings = world.settings();
    let phase = Phase::of(world.parent.status);
    let next = if settings.driver == Driver::Manual && phase == Phase::Running {
        ready(world)
    } else {
        Vec::new()
    };
    MissionSnapshot {
        project_id: project,
        task_id: world.parent.id,
        phase,
        needs_you: needs_you(world, open_threads),
        running: running(world),
        next,
        agents: agents(world),
        replan_at: replan_at(world.parent),
        documents,
        open_threads,
        settings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::work::Level;

    struct Fixture {
        parent: TaskRecord,
        tasks: Vec<TaskRecord>,
        live: HashSet<AgentId>,
        in_flight: HashSet<TaskId>,
        failed: HashSet<TaskId>,
        defs: Vec<String>,
    }

    impl Fixture {
        fn new(status: Status, settings: MissionSettings) -> Self {
            let mut parent = TaskRecord::new("mission".into(), None, Utc::now());
            parent.level = Some(Level::Mission);
            parent.status = status;
            parent.mission = Some(settings);
            Self {
                tasks: vec![parent.clone()],
                parent,
                live: HashSet::new(),
                in_flight: HashSet::new(),
                failed: HashSet::new(),
                defs: vec!["worker".into()],
            }
        }

        fn child(&mut self, title: &str, status: Status) -> TaskId {
            let mut t = TaskRecord::new(title.into(), None, Utc::now());
            t.parent = Some(self.parent.id);
            t.status = status;
            let id = t.id;
            self.tasks.push(t);
            id
        }

        fn edit(&mut self, id: TaskId, f: impl FnOnce(&mut TaskRecord)) {
            f(self.tasks.iter_mut().find(|t| t.id == id).unwrap());
        }

        fn run(&self) -> Vec<Step> {
            let live = |a: AgentId| self.live.contains(&a);
            pass(&World {
                parent: &self.parent,
                tasks: &self.tasks,
                live: &live,
                in_flight: &self.in_flight,
                launch_failed: &self.failed,
                definitions: &self.defs,
            })
        }

        fn needs(&self) -> Vec<NeedsYou> {
            let live = |a: AgentId| self.live.contains(&a);
            needs_you(
                &World {
                    parent: &self.parent,
                    tasks: &self.tasks,
                    live: &live,
                    in_flight: &self.in_flight,
                    launch_failed: &self.failed,
                    definitions: &self.defs,
                },
                0,
            )
        }
    }

    fn auto(parallel: usize) -> MissionSettings {
        MissionSettings {
            parallel,
            ..MissionSettings::default()
        }
    }

    fn launches(steps: &[Step]) -> Vec<TaskId> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::Launch { task, .. } => Some(*task),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nothing_runs_unless_the_parent_is_running() {
        for status in [Status::Backlog, Status::Ready, Status::Blocked] {
            let mut f = Fixture::new(status, auto(3));
            f.child("a", Status::Ready);
            assert!(launches(&f.run()).is_empty(), "{status:?}");
        }
    }

    #[test]
    fn auto_launches_ready_tasks_up_to_parallel_in_order() {
        let mut f = Fixture::new(Status::InProgress, auto(2));
        let low = f.child("low", Status::Ready);
        let high = f.child("high", Status::Ready);
        let first = f.child("first", Status::Ready);
        let _waits = f.child("waits", Status::Ready);
        f.edit(low, |t| t.priority = Priority::Low);
        f.edit(high, |t| t.priority = Priority::High);
        f.edit(_waits, |t| t.prerequisites = vec![first]);
        assert_eq!(launches(&f.run()), vec![high, first]);
    }

    #[test]
    fn manual_launches_nothing() {
        let mut settings = auto(3);
        settings.driver = Driver::Manual;
        let mut f = Fixture::new(Status::InProgress, settings);
        f.child("a", Status::Ready);
        f.child("b", Status::InProgress);
        assert!(launches(&f.run()).is_empty());
    }

    #[test]
    fn a_legacy_mission_never_auto_launches() {
        let mut f = Fixture::new(Status::InProgress, auto(3));
        f.parent.mission = None;
        f.tasks[0].mission = None;
        f.child("a", Status::Ready);
        assert!(launches(&f.run()).is_empty());
    }

    #[test]
    fn running_and_in_flight_count_against_parallel() {
        let mut f = Fixture::new(Status::InProgress, auto(2));
        let busy = f.child("busy", Status::InProgress);
        let agent = AgentId::generate();
        f.edit(busy, |t| t.worker = Some(agent));
        f.live.insert(agent);
        let flying = f.child("flying", Status::Ready);
        f.in_flight.insert(flying);
        f.child("waiting", Status::Ready);
        assert!(launches(&f.run()).is_empty());
    }

    #[test]
    fn human_failed_and_capped_tasks_are_not_ready() {
        let mut f = Fixture::new(Status::InProgress, auto(5));
        let human = f.child("check", Status::Ready);
        f.edit(human, |t| t.assigned_to = Some("human".into()));
        let failed = f.child("failed", Status::Ready);
        f.failed.insert(failed);
        let reviewed = f.child("reviewed", Status::InReview);
        f.edit(reviewed, |t| t.auto_fails = AUTO_FAIL_CAP);
        let review = f.child("review", Status::Ready);
        f.edit(review, |t| {
            t.review_of = Some(reviewed);
            t.prerequisites = vec![reviewed];
        });
        let defined = f.child("defined", Status::Ready);
        f.edit(defined, |t| t.assigned_to = Some("worker".into()));
        assert_eq!(launches(&f.run()), vec![defined]);
        let needs = f.needs();
        assert!(needs.contains(&NeedsYou::HumanTask { task: human }));
        assert!(needs.contains(&NeedsYou::WorkerGone { task: failed }));
        assert!(needs.contains(&NeedsYou::ReviewFailedTwice { task: reviewed }));
    }

    #[test]
    fn a_review_task_runs_against_an_approve_task_in_review() {
        let mut f = Fixture::new(Status::InProgress, auto(5));
        let reviewed = f.child("reviewed", Status::InReview);
        f.edit(reviewed, |t| t.approve = true);
        let review = f.child("review", Status::Ready);
        f.edit(review, |t| {
            t.review_of = Some(reviewed);
            t.prerequisites = vec![reviewed];
        });
        let dependent = f.child("dependent", Status::Ready);
        f.edit(dependent, |t| t.prerequisites = vec![reviewed]);
        let steps = f.run();
        assert_eq!(launches(&steps), vec![review]);
        assert!(steps.contains(&Step::Launch {
            task: review,
            role: LaunchRole::Reviewer
        }));
        assert!(
            f.needs()
                .contains(&NeedsYou::AwaitingApproval { task: reviewed })
        );
    }

    #[test]
    fn completion_moves_the_parent_to_review_and_fix_tasks_move_it_back() {
        let mut f = Fixture::new(Status::InProgress, auto(1));
        f.child("done", Status::Done);
        let left = f.child("left", Status::InReview);
        assert!(f.run().contains(&Step::ParentTo(Status::InReview)));

        f.edit(left, |t| t.approve = true);
        assert!(!f.run().contains(&Step::ParentTo(Status::InReview)));

        let mut f = Fixture::new(Status::InReview, auto(1));
        f.child("fix", Status::Ready);
        assert_eq!(f.run(), vec![Step::ParentTo(Status::InProgress)]);
    }

    #[test]
    fn a_dead_worker_is_written_gone_once_and_never_relaunched() {
        let mut f = Fixture::new(Status::InProgress, auto(3));
        let task = f.child("t", Status::InProgress);
        let agent = AgentId::generate();
        f.edit(task, |t| {
            t.worker = Some(agent);
            t.comments.push(Comment::event(
                String::new(),
                RunEvent::Spawned {
                    agent,
                    role: LaunchRole::Worker,
                    retry: false,
                },
                Utc::now(),
            ));
        });
        assert_eq!(f.run(), vec![Step::Gone { task, agent }]);
        assert!(f.needs().contains(&NeedsYou::WorkerGone { task }));
        f.edit(task, |t| {
            t.comments.push(Comment::event(
                String::new(),
                RunEvent::Gone { agent },
                Utc::now(),
            ))
        });
        assert!(f.run().is_empty());
    }

    #[test]
    fn a_closed_task_unloads_its_live_worker_and_a_finished_one_is_noted() {
        let mut f = Fixture::new(Status::InProgress, auto(1));
        let done = f.child("done", Status::Done);
        let a = AgentId::generate();
        f.edit(done, |t| t.worker = Some(a));
        f.live.insert(a);
        let review = f.child("review", Status::InReview);
        let b = AgentId::generate();
        f.edit(review, |t| {
            t.worker = Some(b);
            t.approve = true;
        });
        f.live.insert(b);
        let steps = f.run();
        assert!(steps.contains(&Step::Unload {
            task: done,
            agent: a
        }));
        assert!(steps.contains(&Step::Finished {
            task: review,
            agent: b
        }));
    }

    #[test]
    fn a_verdict_since_the_spawn_is_acted_on_alone() {
        let mut f = Fixture::new(Status::InProgress, auto(3));
        let reviewed = f.child("reviewed", Status::InReview);
        let review = f.child("review", Status::InProgress);
        f.child("other", Status::Ready);
        let agent = AgentId::generate();
        f.live.insert(agent);
        f.edit(review, |t| {
            t.review_of = Some(reviewed);
            t.worker = Some(agent);
            t.comments.push(Comment::event(
                "fix x".into(),
                RunEvent::ReviewVerdict { pass: false },
                Utc::now(),
            ));
            t.comments.push(Comment::event(
                String::new(),
                RunEvent::Spawned {
                    agent,
                    role: LaunchRole::Reviewer,
                    retry: false,
                },
                Utc::now(),
            ));
        });
        // The old verdict predates the spawn: not acted on.
        assert!(!f.run().iter().any(|s| matches!(s, Step::Verdict { .. })));
        f.edit(review, |t| {
            t.comments.push(Comment::event(
                "fix y".into(),
                RunEvent::ReviewVerdict { pass: false },
                Utc::now(),
            ))
        });
        assert_eq!(
            f.run(),
            vec![Step::Verdict {
                review,
                pass: false,
                findings: "fix y".into()
            }]
        );
    }

    #[test]
    fn a_verdict_is_applied_after_the_reviewer_moved_its_task() {
        let mut f = Fixture::new(Status::InProgress, auto(1));
        let reviewed = f.child("reviewed", Status::InReview);
        let review = f.child("review", Status::InReview);
        let agent = AgentId::generate();
        f.edit(review, |t| {
            t.review_of = Some(reviewed);
            t.worker = Some(agent);
            t.comments.push(Comment::event(
                String::new(),
                RunEvent::ReviewVerdict { pass: true },
                Utc::now(),
            ));
        });
        assert!(f.run().contains(&Step::Verdict {
            review,
            pass: true,
            findings: String::new()
        }));
        // Applied, the verdict clears `worker` and is not acted on again.
        f.edit(review, |t| t.worker = None);
        assert!(!f.run().iter().any(|s| matches!(s, Step::Verdict { .. })));
    }

    #[test]
    fn in_progress_with_no_agent_reads_as_gone() {
        let mut f = Fixture::new(Status::InProgress, auto(1));
        let task = f.child("moved by hand", Status::InProgress);
        assert!(f.needs().contains(&NeedsYou::WorkerGone { task }));
        f.in_flight.insert(task);
        assert!(!f.needs().contains(&NeedsYou::WorkerGone { task }));
    }

    #[test]
    fn abandoning_a_task_abandons_its_reviews() {
        let mut f = Fixture::new(Status::InProgress, auto(1));
        let gone = f.child("gone", Status::Abandoned);
        let review = f.child("review", Status::Ready);
        f.edit(review, |t| t.review_of = Some(gone));
        assert!(f.run().contains(&Step::Abandon { task: review }));
    }

    #[test]
    fn planner_items_follow_the_phase_and_the_request() {
        let mut f = Fixture::new(Status::Backlog, auto(1));
        assert_eq!(f.needs(), vec![NeedsYou::PlannerStopped]);
        let planner = AgentId::generate();
        f.parent.mission.as_mut().unwrap().planner_agent = Some(planner);
        f.live.insert(planner);
        assert!(f.needs().is_empty());
        f.parent.comments.push(Comment::event(
            String::new(),
            RunEvent::PhaseRequested {
                to: Phase::Breakdown,
                summary: "ready".into(),
            },
            Utc::now(),
        ));
        assert_eq!(f.needs(), vec![NeedsYou::PlanReady { open_threads: 0 }]);
        f.parent.status = Status::Ready;
        f.parent.comments.push(Comment::event(
            String::new(),
            RunEvent::Phase {
                to: Phase::Breakdown,
            },
            Utc::now(),
        ));
        assert!(f.needs().is_empty());
        assert_eq!(replan_at(&f.parent), None);
    }

    #[test]
    fn replan_at_is_the_breakdown_after_a_run() {
        let mut parent = TaskRecord::new("m".into(), None, Utc::now());
        let event = |to| Comment::event(String::new(), RunEvent::Phase { to }, Utc::now());
        parent.comments.push(event(Phase::Breakdown));
        assert_eq!(replan_at(&parent), None);
        parent.comments.push(event(Phase::Running));
        let again = event(Phase::Breakdown);
        let at = again.created_at;
        parent.comments.push(again);
        assert_eq!(replan_at(&parent), Some(at));
    }
}
