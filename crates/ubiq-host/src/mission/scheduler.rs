//! The mission scheduler (M23): who works on what, in [`ExecutionMode::Auto`].
//!
//! **It decides; it does not act.** [`plan`] is a pure function of the mission's record, the
//! project's tasks and its agents, and it answers a list of [`Decision`]s.
//! [`super::Missions::schedule`] is the half that carries them out — the assignment, the prompt,
//! the spawn request, the journal line. Splitting it that way is what makes an autonomous
//! scheduler testable at all: every rule below is a table of inputs and an expected list, with no
//! bus, no store and no clock in the way.
//!
//! **It runs on the coordinator's own loop, on events it already receives** — a task changed, an
//! agent's activity or lifecycle changed, the mode or the parallelism changed. No thread of its
//! own and no timer: the events that come from an MCP tool instead of a window reach the loop as
//! [`ubiq_proto::messages::Message::MissionSchedule`], said into the host's own inbox.
//!
//! **Launches still go through the window** (M13), so the one-minter rule for an
//! [`AgentId`] holds: what the scheduler emits is a [`Decision::Spawn`], which becomes a
//! [`ubiq_proto::mission::PendingSpawn`] with `auto` set — the flag that bypasses
//! [`ubiq_proto::mission::SpawnPolicy::Ask`], because choosing auto *is* the consent. The cap is
//! [`MissionRecord::parallelism`] instead.
//!
//! **Auto only runs in the In-progress phase.** Leaving it pauses the scheduler rather than
//! switching the mode off, so the setting is still what the user chose when the mission comes
//! back — [`MissionRecord::scheduling`] is that rule, and [`plan`] answers nothing at all while it
//! is false.

use chrono::{DateTime, Duration, Utc};
use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{ExecutionMode, MissionRecord};
use ubiq_proto::work::{Activity, AgentId, Priority, Status, TaskRecord, WorkAgent};

/// How long an agent may hold a task without visibly working on it before the scheduler takes it
/// back (M23's "sits idle past a grace period").
///
/// **Five minutes**, because the thing it must never mistake for a stall is a cold start: a
/// harness launching, resolving its configuration, reading the brief and taking its first turn is
/// comfortably inside it, and an agent that has done none of that in five minutes is not going to.
/// Shorter and auto mode would release tasks out from under agents that were about to begin;
/// longer and a stalled slot is lost for most of a working session, which at the default
/// parallelism of two is half the mission.
///
/// It is measured from [`ubiq_proto::mission::RosterEntry::held_since`] and only ever *checked* —
/// there is no timer, so a grace that has expired is noticed at the next wake and not before.
pub const IDLE_GRACE: Duration = Duration::minutes(5);

/// How much overlap a task and an agent need before the scheduler will hand the one to the other
/// (M24). One shared label: **zero overlap never reuses an agent**, and that is the whole rule —
/// anything above one would be a second knob nobody asked for.
pub const AFFINITY_THRESHOLD: usize = 1;

/// One thing the scheduler has decided to do. Every variant becomes a journal line, so auto mode
/// can always be read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// A task the scheduler was holding is finished — it moved to `InReview` or `Done`. The slot
    /// is free and the agent is idle; whether it is reused or retired is the next pass's.
    Finished { task: TaskId, agent: AgentId },
    /// An attempt failed: the agent ended, or held the task past the grace without starting it.
    /// The task goes back to `Ready` and the attempt is counted.
    Release {
        task: TaskId,
        agent: Option<AgentId>,
        /// The attempt this failure *is* — 1 for the first.
        attempt: usize,
    },
    /// The task has run out of attempts. It goes to `Blocked`, the coordinator is told, and it
    /// appears under *Needs you*.
    Block { task: TaskId, attempts: usize },
    /// Hand a task to an agent already in the mission.
    Assign {
        task: TaskId,
        agent: AgentId,
        /// The labels the two share — the *why* the journal line carries.
        affinity: Vec<String>,
    },
    /// Ask the window for a new agent, with the task in its briefing.
    Spawn { task: TaskId, kind: String },
    /// Stop one of the scheduler's own agents: nothing left with enough affinity, or
    /// [`ubiq_proto::mission::OnFinish::Stop`]. Unloaded, never deleted.
    Retire { agent: AgentId },
    /// Every child is in review or done. The scheduler stops and the coordinator is prompted to
    /// request Completed.
    Done,
}

/// What the scheduler would do now.
///
/// The order of the list is the order it decided in, and it matters: the failure sweep runs before
/// anything is handed out, so a task released this pass is in the same pass's pool, and the
/// retirements run last, after every free slot has had its chance to reuse an agent.
pub fn plan(
    record: &MissionRecord,
    tasks: &[TaskRecord],
    agents: &[WorkAgent],
    now: DateTime<Utc>,
) -> Vec<Decision> {
    if !record.scheduling() {
        return Vec::new();
    }
    let mut decisions = Vec::new();

    // ── the failure sweep (M23, *Finish* and *Failure*) ─────────────
    //
    // What the scheduler thinks it is holding, against what the board and the agent list actually
    // say. Three answers: the task is finished, the attempt failed, or it is still being worked.
    // `released` and `finished` are kept so the pool and the slot count below read the world as it
    // is *after* this sweep rather than as it was before — a task released here is available in
    // the same pass, which is the difference between recovering in one wake and recovering in two.
    let mut released: Vec<TaskId> = Vec::new();
    let mut freed: Vec<AgentId> = Vec::new();
    for entry in record.scheduler_members() {
        let Some(task) = entry.current_task else {
            continue;
        };
        let held = tasks.iter().find(|candidate| candidate.id == task);
        let live = agents.iter().find(|agent| agent.id == entry.agent);

        // The task is gone from the board: nothing to release and nothing to count against it.
        let Some(held) = held else {
            freed.push(entry.agent);
            continue;
        };
        if matches!(held.status, Status::InReview | Status::Done) {
            decisions.push(Decision::Finished {
                task,
                agent: entry.agent,
            });
            freed.push(entry.agent);
            continue;
        }
        // A person moved the task onto somebody else. The scheduler lets go rather than fighting
        // over it — a hand assignment always wins (M22).
        if agents
            .iter()
            .any(|agent| agent.task == Some(task) && agent.id != entry.agent)
        {
            freed.push(entry.agent);
            continue;
        }

        // **An agent waiting on a person keeps its slot** and raises *Needs you*; it has not
        // failed and taking its task away would throw away the answer it is waiting for.
        let waiting = live.is_some_and(|agent| agent.activity == Activity::NeedsYou);
        if waiting {
            continue;
        }
        let ended =
            live.is_none_or(|agent| matches!(agent.activity, Activity::Ended | Activity::Failed));
        let stalled = held.status != Status::InProgress
            && entry
                .held_since
                .is_some_and(|since| now - since > IDLE_GRACE);
        if !ended && !stalled {
            continue;
        }

        let attempt = record.attempts_of(task) + 1;
        freed.push(entry.agent);
        decisions.push(Decision::Release {
            task,
            agent: Some(entry.agent),
            attempt,
        });
        if attempt >= record.max_attempts {
            decisions.push(Decision::Block {
                task,
                attempts: attempt,
            });
        } else {
            released.push(task);
        }
    }

    // ── the pool (M23) ──────────────────────────────────────────────
    let children: Vec<&TaskRecord> = tasks
        .iter()
        .filter(|task| task.parent == Some(record.task_id))
        .collect();
    // Readiness is read against the board as this pass leaves it: a task the sweep blocked is not
    // `InReview` either way, so only the releases change anything here.
    let mut pool: Vec<&TaskRecord> = children
        .iter()
        .copied()
        .filter(|task| {
            // **`Backlog` is never picked.** Promoting to `Ready` is how work is released.
            let ready_column =
                task.status == Status::Ready || released.contains(&task.id);
            ready_column
                && task.ready(tasks)
                // Not hand-assigned, and not already held — one question, because an agent
                // holding a task is what both look like from here.
                && !agents.iter().any(|agent| agent.task == Some(task.id))
                && !record
                    .scheduler_members()
                    .any(|entry| entry.current_task == Some(task.id) && !freed.contains(&entry.agent))
        })
        .collect();
    let levels = wbs_levels(&children);
    pool.sort_by_key(|task| {
        (
            // Priority first, highest first.
            std::cmp::Reverse(priority_rank(task.priority)),
            // Then WBS level, shallowest first — the work that unblocks the most goes out first.
            levels
                .iter()
                .find(|(id, _)| *id == task.id)
                .map_or(0, |(_, level)| *level),
            // Then board order, which is the order the list is already in.
            tasks
                .iter()
                .position(|held| held.id == task.id)
                .unwrap_or(0),
        )
    });

    // ── the slots (M23) ─────────────────────────────────────────────
    //
    // A pending auto spawn holds a slot: the window has not answered yet, but the scheduler has
    // already committed that slot to a task and a second pass must not commit it again.
    let held_slots = record
        .scheduler_members()
        .filter(|entry| entry.current_task.is_some() && !freed.contains(&entry.agent))
        .count()
        + record
            .pending_spawns
            .iter()
            .filter(|spawn| spawn.auto)
            .count();
    let mut slots = record.parallelism.saturating_sub(held_slots);

    // ── the assignment (M23, M24) ───────────────────────────────────
    //
    // Who has been given something in *this* pass, so one pass cannot hand two tasks to the same
    // agent or count the same idle agent twice.
    let mut taken: Vec<AgentId> = Vec::new();
    for task in &pool {
        if slots == 0 {
            break;
        }
        let labels: Vec<String> = task.labels.iter().map(|label| label.name.clone()).collect();

        // (1) An idle agent of the scheduler's, in the mission, with the best affinity and still
        //     under `max_tasks_per_agent`. Ties go to the most recent holder.
        let reusable = record.on_finish == ubiq_proto::mission::OnFinish::ReuseOrStop;
        let best = reusable
            .then(|| {
                record
                    .scheduler_members()
                    .filter(|entry| {
                        (entry.current_task.is_none() || freed.contains(&entry.agent))
                            && !taken.contains(&entry.agent)
                            && entry.tasks_held < record.max_tasks_per_agent
                            // A dead agent is not an idle one — and neither is one a person has
                            // given work to. The roster only knows what the *scheduler* handed
                            // over; the board is what knows about a hand assignment, and an agent
                            // already carrying one must not be handed a second task on top of it.
                            // The one board task that does not count is the one this pass has just
                            // taken off it: the assignment is cleared when the decisions are
                            // applied, which is after this.
                            && agents.iter().any(|agent| {
                                agent.id == entry.agent
                                    && (agent.task.is_none() || agent.task == entry.current_task)
                                    && !matches!(
                                        agent.activity,
                                        Activity::Ended | Activity::Failed
                                    )
                            })
                    })
                    .map(|entry| (entry.affinity(&labels), entry.last_held_at, entry.agent))
                    .filter(|(affinity, _, _)| *affinity >= AFFINITY_THRESHOLD)
                    .max_by_key(|(affinity, last, _)| (*affinity, *last))
            })
            .flatten();

        if let Some((_, _, agent)) = best {
            let shared = shared_labels(record, agent, &labels);
            decisions.push(Decision::Assign {
                task: task.id,
                agent,
                affinity: shared,
            });
            taken.push(agent);
            slots -= 1;
            continue;
        }

        // (2) A new agent: the task's own kind, else the kind whose labels match best, else the
        //     mission's default. A mission with no kinds at all can spawn nothing, and says so in
        //     `mission_overview` rather than inventing one.
        let kind = record
            .task_kind(task.id)
            .map(str::to_string)
            .or_else(|| record.kind_for(&labels));
        if let Some(kind) = kind {
            decisions.push(Decision::Spawn {
                task: task.id,
                kind,
            });
            slots -= 1;
        }
    }

    // ── retiring (M23, *Finish*) ────────────────────────────────────
    //
    // Last, so an agent that could have been handed something has already been. What is left is an
    // agent of the scheduler's that has finished work and has nothing to go on to.
    for entry in record.scheduler_members() {
        let idle = entry.current_task.is_none() || freed.contains(&entry.agent);
        let gone = agents
            .iter()
            .find(|agent| agent.id == entry.agent)
            .is_none_or(|agent| matches!(agent.activity, Activity::Ended | Activity::Failed));
        if idle && !taken.contains(&entry.agent) && entry.tasks_held > 0 && !gone {
            decisions.push(Decision::Retire { agent: entry.agent });
        }
    }

    // ── done (M23) ──────────────────────────────────────────────────
    //
    // Said once. `scheduler_done` is cleared the moment anything is handed out again, so a mission
    // that gains a task after it finished says it again when *that* is done.
    let all_done = !children.is_empty()
        && children
            .iter()
            .all(|task| matches!(task.status, Status::InReview | Status::Done));
    if all_done && !record.scheduler_done {
        decisions.push(Decision::Done);
    }
    decisions
}

/// Which labels an agent and a task actually share — the *(affinity ui, api)* half of the journal
/// line. Read off the roster rather than recomputed from the tasks it held, because the roster is
/// the only thing that still knows: those tasks may since have been relabelled or deleted.
fn shared_labels(record: &MissionRecord, agent: AgentId, labels: &[String]) -> Vec<String> {
    let Some(entry) = record.member(agent) else {
        return Vec::new();
    };
    labels
        .iter()
        .filter(|label| {
            entry
                .labels
                .iter()
                .any(|held| held.eq_ignore_ascii_case(label))
        })
        .cloned()
        .collect()
}

/// Each child's depth in the breakdown: 0 for a task with no prerequisites among its siblings,
/// and one more than the deepest prerequisite otherwise (M20's topological layering).
///
/// Bounded by the number of children rather than by recursion: prerequisites are a DAG the work
/// store already refuses a cycle in, and a walk that trusted that and was wrong would not
/// terminate. This one always does — each pass can only settle more tasks, and it stops when a
/// pass settles none.
fn wbs_levels(children: &[&TaskRecord]) -> Vec<(TaskId, usize)> {
    let mut levels: Vec<(TaskId, usize)> = Vec::new();
    for _ in 0..children.len() {
        let mut settled = false;
        for task in children {
            if levels.iter().any(|(id, _)| *id == task.id) {
                continue;
            }
            let within: Vec<TaskId> = task
                .prerequisites
                .iter()
                .copied()
                .filter(|id| children.iter().any(|child| child.id == *id))
                .collect();
            let deepest = within
                .iter()
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

/// Highest first, so a `Reverse` on this sorts a pool the way a board reads.
fn priority_rank(priority: Priority) -> u8 {
    match priority {
        Priority::Low => 0,
        Priority::Normal => 1,
        Priority::High => 2,
    }
}

/// Whether a mission is in auto at all, whatever phase it is in — what a surface reads to draw the
/// mode, as against [`MissionRecord::scheduling`], which says whether it is *running*.
pub fn is_auto(record: &MissionRecord) -> bool {
    record.execution == ExecutionMode::Auto
}
