//! The `ubiq-mission` and `use-mission` servers: a mission as tools the agents in it can call.
//!
//! **No tool here takes a mission id.** Which mission the caller is in is
//! [`AgentFacts::mission`], filled at launch from the agent's assignment or its spawner and moved
//! by `AssignAgent` — so the mission is resolved from the URL identity alone (M11, `D102`), and an
//! agent cannot address a mission it is not in by naming one.
//!
//! **An agent outside a mission gets a sentence, not an error.** A coordinator whose mission was
//! demoted, a worker unassigned mid-run, a run launched with the server ticked and no mission
//! behind it — all three are ordinary, and none of them is something the model did wrong. The
//! sentence says what is true and what to do instead, which is what a model can act on.
//!
//! The `manage` / `use` split is the task servers' own (`manage-ubiq-tasks` / `use-task`): the
//! read tools and `report_progress` answer on both, everything that runs the mission answers only
//! on `ubiq-mission`. The split is in the catalogue's two [`super::catalogue::ServerSpec`]s and
//! enforced here by [`use_call`] listing what a worker may call, so a tool added to one server is
//! never quietly reachable on the other.
//!
//! Mutations broadcast to every window (`D120`), the way every other tool family's do.

use serde_json::{Value, json};
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::messages::{Message, TaskField};
use ubiq_proto::mission::{Actor, MissionRecord, MissionRole, Phase};
use ubiq_proto::work::AgentId;

use super::registry::AgentFacts;
use super::{MissionReach, PlanReach, WorkAccess};
use crate::plan::{Saver, Target};
use crate::reply::Reply;

/// How many journal lines `mission_overview` folds in.
const OVERVIEW_JOURNAL: usize = 5;

/// The coordinator's server: everything.
pub fn call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    reach: &MissionReach,
    work: Option<&WorkAccess>,
    plan: Option<&PlanReach>,
) -> Result<Value, String> {
    let (project, mission) = match resolve(facts)? {
        Some(found) => found,
        None => return Ok(outside()),
    };
    let by = actor(facts);
    match tool {
        "mission_overview" => overview(project, mission, facts, reach, work),
        "read_brief" => read_brief(project, mission, work),
        "list_documents" => Ok(list_documents(project, mission, reach)),
        "read_document" => read_document(arguments, project, mission, plan),
        "write_document" => write_document(arguments, facts, by, project, mission, reach, plan),
        "create_mission_task" => create_mission_task(arguments, by, project, mission, reach, work),
        "create_mission_tasks" => {
            create_mission_tasks(arguments, by, project, mission, reach, work)
        }
        "report_progress" => report_progress(arguments, by, project, mission, reach),
        "request_phase" => request_phase(arguments, project, mission, reach),
        "list_agent_kinds" => list_agent_kinds(project, mission, reach),
        "spawn_agent" => spawn_agent(arguments, by, project, mission, reach),
        "list_agents" => list_agents(project, mission, reach, work),
        "message_agent" => message_agent(arguments, project, mission, reach),
        "read_feedback" => Ok(read_feedback(project, mission, reach)),
        _ => Err(format!("unknown tool: ubiq-mission/{tool}")),
    }
}

/// The workers' server: the read tools and the one line a worker writes about its own work.
pub fn use_call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    reach: &MissionReach,
    work: Option<&WorkAccess>,
    plan: Option<&PlanReach>,
) -> Result<Value, String> {
    match tool {
        "mission_overview" | "read_brief" | "list_documents" | "read_document"
        | "report_progress" | "list_agents" => call(tool, arguments, facts, reach, work, plan),
        _ => Err(format!("unknown tool: use-mission/{tool}")),
    }
}

// ── who is calling ──────────────────────────────────────────────────

/// The project and mission this call is inside, or `None` for an agent in no mission.
fn resolve(facts: &AgentFacts) -> Result<Option<(ProjectId, TaskId)>, String> {
    let project: ProjectId = facts
        .project
        .id
        .parse()
        .map_err(|_| "this agent has no project".to_string())?;
    Ok(facts.mission.map(|mission| (project, mission)))
}

/// The sentence every tool answers an agent outside a mission with.
///
/// A result rather than an `Err`: nothing went wrong, and a model that reads an error here will
/// retry it. `in_mission: false` is the machine-readable half, so a harness can branch without
/// matching on prose.
fn outside() -> Value {
    json!({
        "in_mission": false,
        "message": "You are not in a mission, so there is nothing here to read or change. \
                    Carry on with the task you were given. If you believe you should be in one, \
                    say so — a person has to put you on its roster.",
    })
}

/// An agent's own key as a mission [`Actor`]. A key that is a pane id rather than an agent id
/// leaves [`Actor::Host`] behind: a pane is not on any roster, so there is no agent to name.
fn actor(facts: &AgentFacts) -> Actor {
    facts
        .key
        .parse::<AgentId>()
        .map(Actor::Agent)
        .unwrap_or(Actor::Host)
}

/// The mission's record, made if nobody has opened this mission yet.
///
/// An agent's first call is as much "somebody opened the mission" as a board listing is (M3), so
/// this goes through [`crate::mission::Missions::touch`] rather than reading past it — otherwise
/// a coordinator launched onto a freshly promoted task would be told its own mission has no
/// record. A creation is broadcast the way every other one is.
fn record(
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
) -> Result<MissionRecord, String> {
    let (record, replies) = reach.missions.lock().touch(project, mission)?;
    broadcast(&reach.everyone, replies);
    Ok(record)
}

// ── reading ─────────────────────────────────────────────────────────

fn overview(
    project: ProjectId,
    mission: TaskId,
    facts: &AgentFacts,
    reach: &MissionReach,
    work: Option<&WorkAccess>,
) -> Result<Value, String> {
    let record = record(project, mission, reach)?;
    let journal = reach
        .missions
        .lock()
        .journal_tail(project, mission, OVERVIEW_JOURNAL);

    // The task counts are the mission's children by status — the one number a coordinator asks
    // for before anything else, and the reason the board handle is threaded in here at all.
    let (title, counts, children) = match work {
        Some(access) => {
            let (_, tasks) = access.work.lock().tasks(project);
            let title = tasks
                .iter()
                .find(|task| task.id == mission)
                .map(|task| task.title.clone())
                .unwrap_or_default();
            let mut counts = serde_json::Map::new();
            let mut children = 0usize;
            for task in tasks.iter().filter(|task| task.parent == Some(mission)) {
                children += 1;
                let entry = counts
                    .entry(task.status.label().to_string())
                    .or_insert_with(|| json!(0));
                *entry = json!(entry.as_u64().unwrap_or(0) + 1);
            }
            (title, Value::Object(counts), children)
        }
        None => (String::new(), json!({}), 0),
    };

    // **The execution mode, the pool, the slots and who holds what** (M25) — the block that makes
    // auto mode legible to the agents inside it as well as to the panel.
    let view = reach.missions.lock().scheduler_view(project, mission);
    // A worker's own current task, which is the one thing `use-mission::mission_overview` exists
    // to tell it: read off the roster, so it is what the scheduler believes rather than what the
    // board's free-text assignee says.
    let mine = facts.key.parse::<AgentId>().ok().and_then(|agent| {
        view.holding
            .iter()
            .find(|(held, _)| *held == agent)
            .map(|(_, task)| task.to_string())
    });

    Ok(json!({
        "in_mission": true,
        "task_id": mission.to_string(),
        "title": title,
        "phase": record.phase.label(),
        "execution": if view.auto { "auto" } else { "manual" },
        "scheduler": {
            "running": view.running,
            "parallelism": view.parallelism,
            "free_slots": view.free_slots,
            "pool": view.pool.iter().map(|(id, title)| json!({
                "task_id": id.to_string(),
                "title": title,
            })).collect::<Vec<_>>(),
            "holding": view.holding.iter().map(|(agent, task)| json!({
                "agent": agent.to_string(),
                "task_id": task.to_string(),
            })).collect::<Vec<_>>(),
        },
        "your_task": mine,
        "documents": record.documents,
        "tasks": {"total": children, "by_status": counts},
        "roster": roster_json(&record),
        "pending": record.pending_phase.as_ref().map(|pending| json!({
            "phase": pending.phase.label(),
            "summary": pending.summary,
            "asked_at": pending.at,
        })),
        "coordinator": record.coordinator.map(|agent| agent.to_string()),
        "require_plan": record.require_plan,
        "journal": journal.iter().map(journal_json).collect::<Vec<_>>(),
    }))
}

fn read_brief(
    project: ProjectId,
    mission: TaskId,
    work: Option<&WorkAccess>,
) -> Result<Value, String> {
    let access =
        work.ok_or_else(|| "this host has no task board to read a brief from".to_string())?;
    let (_, tasks) = access.work.lock().tasks(project);
    let anchor = tasks
        .iter()
        .find(|task| task.id == mission)
        .ok_or_else(|| "this mission's task is gone".to_string())?;
    // The referenced tasks by title and key rather than by id: an id is not something a model can
    // do anything with, and the point of a brief is to be read.
    let linked: Vec<Value> = anchor
        .references
        .iter()
        .filter_map(|id| tasks.iter().find(|task| task.id == *id))
        .map(|task| json!({"task_id": task.id.to_string(), "key": task.key, "title": task.title}))
        .collect();
    Ok(json!({
        "in_mission": true,
        "task_id": anchor.id.to_string(),
        "key": anchor.key,
        "title": anchor.title,
        "description": anchor.description,
        "attachments": anchor.attachments.iter().map(|attachment| json!({
            "target": attachment.target,
            "label": attachment.label,
        })).collect::<Vec<_>>(),
        "linked_tasks": linked,
    }))
}

fn list_documents(project: ProjectId, mission: TaskId, reach: &MissionReach) -> Value {
    let names = reach.missions.lock().documents(project, mission);
    json!({
        "in_mission": true,
        "documents": names,
        "note": "The plan is not listed here — read it with ubiq-plan's read_plan.",
    })
}

fn read_document(
    arguments: &Value,
    project: ProjectId,
    mission: TaskId,
    plan: Option<&PlanReach>,
) -> Result<Value, String> {
    let reach = plan.ok_or_else(|| "this host has no document store".to_string())?;
    let name = document_name(arguments)?;
    let target = doc_target(project, mission, &name);
    let replies = reach.plans.lock().load(&target);
    document_result(&name, &replies)
}

// ── writing ─────────────────────────────────────────────────────────

/// Replace a mission document, stamped as **this agent's** save — `write_plan`'s own discipline,
/// including its `expected_revision` posture: optional here because a first write has nothing to
/// have read, and checked exactly like a window's once a revision is named.
#[allow(clippy::too_many_arguments)]
fn write_document(
    arguments: &Value,
    facts: &AgentFacts,
    by: Actor,
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
    plan: Option<&PlanReach>,
) -> Result<Value, String> {
    let plan_reach = plan.ok_or_else(|| "this host has no document store".to_string())?;
    let name = document_name(arguments)?;
    let body = required_str(arguments, "body")?.to_string();
    let expected =
        match arguments.get("expected_revision") {
            None | Some(Value::Null) => None,
            Some(Value::Number(number)) => Some(number.as_u64().ok_or_else(|| {
                "expected_revision must be a whole number, zero or more".to_string()
            })?),
            Some(_) => return Err("expected_revision must be a number".to_string()),
        };

    let target = doc_target(project, mission, &name);
    let replies = {
        let mut plans = plan_reach.plans.lock();
        plans.save(&target, body, &Saver::agent(facts.key.clone()), expected)
    };
    for reply in &replies {
        if let Reply::Everyone(message) = reply {
            plan_reach.everyone.send(message.clone());
        }
    }
    let result = document_result(&name, &replies)?;

    // The journal line and the refreshed document list, in one act — this is how the name reaches
    // a window, which otherwise has no way to discover that a document exists.
    let noted = reach
        .missions
        .lock()
        .document_written(project, mission, by, &name);
    broadcast(&reach.everyone, noted);
    Ok(result)
}

/// What one entry of a breakdown says, singular or batched — [`create_mission_task`] and
/// [`create_mission_tasks`] read the same shape so the two tools can never drift.
struct TaskSpec {
    title: String,
    description: String,
    todos: Vec<String>,
    labels: Vec<String>,
    /// The agent kind the planner suggests for this task (M21). Kept on the mission record, not
    /// on the card — see [`ubiq_proto::mission::TaskKind`].
    kind: Option<String>,
    /// Ids, existing keys, or a `ref` named by an earlier entry of the same batch.
    prerequisites: Vec<String>,
    /// A name this entry answers to **inside this call only**, so a later entry can wait on it
    /// without anybody threading an id back to the model. Never written down.
    reference: Option<String>,
}

fn spec_from(arguments: &Value) -> Result<TaskSpec, String> {
    Ok(TaskSpec {
        title: required_str(arguments, "title")?.to_string(),
        description: arguments
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        todos: string_list(arguments, "todos"),
        labels: string_list(arguments, "labels"),
        kind: arguments
            .get("kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(str::to_string),
        prerequisites: string_list(arguments, "prerequisites"),
        reference: arguments
            .get("ref")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string),
    })
}

fn create_mission_task(
    arguments: &Value,
    by: Actor,
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
    work: Option<&WorkAccess>,
) -> Result<Value, String> {
    let access = work.ok_or_else(|| "this host has no task board".to_string())?;
    let spec = spec_from(arguments)?;
    let made: Vec<Made> = Vec::new();
    let created = write_task(&spec, project, mission, access, &made)?;
    finish_batch(by, project, mission, reach, std::slice::from_ref(&created));
    Ok(json!({
        "in_mission": true,
        "task_id": created.id.to_string(),
        "key": created.key,
        "parent": mission.to_string(),
        "todos": created.todos,
        "prerequisites": created.prerequisites.len(),
    }))
}

/// The whole breakdown in one call (M21).
///
/// **In-batch references are the point.** An entry may wait on one created earlier in the same
/// call, named by the `ref` it gave itself — so a planner writes a dependency graph in one message
/// instead of threading a dozen ids back through a dozen round trips, which is where a model loses
/// the shape of what it was building.
///
/// **It is not transactional**, and saying so is better than pretending: a board write that fails
/// halfway leaves what it already made. Every entry that succeeded is named in `created` and every
/// one that did not is named in `failed` with its reason, so a second call can fill the gaps
/// rather than starting again — and an entry whose prerequisite failed is reported as such rather
/// than created without it.
fn create_mission_tasks(
    arguments: &Value,
    by: Actor,
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
    work: Option<&WorkAccess>,
) -> Result<Value, String> {
    let access = work.ok_or_else(|| "this host has no task board".to_string())?;
    let entries = arguments
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or_else(|| "tasks must be an array of task objects".to_string())?;
    if entries.is_empty() {
        return Err("tasks is empty — name at least one task to create".to_string());
    }

    let mut made: Vec<Made> = Vec::new();
    let mut failed = Vec::new();
    for entry in entries {
        let spec = match spec_from(entry) {
            Ok(spec) => spec,
            Err(reason) => {
                failed.push(json!({"title": entry.get("title"), "error": reason}));
                continue;
            }
        };
        let title = spec.title.clone();
        match write_task(&spec, project, mission, access, &made) {
            Ok(created) => made.push(created),
            Err(reason) => failed.push(json!({"title": title, "error": reason})),
        }
    }
    finish_batch(by, project, mission, reach, &made);

    Ok(json!({
        "in_mission": true,
        "parent": mission.to_string(),
        "created": made.iter().map(|made| json!({
            "ref": made.reference,
            "task_id": made.id.to_string(),
            "key": made.key,
            "title": made.title,
            "prerequisites": made.prerequisites.iter().map(ToString::to_string).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "failed": failed,
    }))
}

/// One task this call has written, as the answer and the next entry's references read it.
struct Made {
    id: TaskId,
    key: Option<String>,
    title: String,
    todos: usize,
    prerequisites: Vec<TaskId>,
    reference: Option<String>,
    /// The agent kind the planner suggested, to be written onto the mission's record once the
    /// board is settled.
    kind: Option<String>,
}

/// Write one entry of a breakdown, resolving its prerequisites against the board and against what
/// this call has already made.
fn write_task(
    spec: &TaskSpec,
    project: ProjectId,
    mission: TaskId,
    access: &WorkAccess,
    made: &[Made],
) -> Result<Made, String> {
    let (created, messages) = {
        let mut board = access.work.lock();
        // Resolved before anything is written, so an entry naming a prerequisite nobody can find
        // is refused rather than created without it — a task silently missing a dependency is
        // exactly the bug a scheduler then acts on.
        let (_, tasks) = board.tasks(project);
        let prerequisites = spec
            .prerequisites
            .iter()
            .map(|named| resolve_prerequisite(named, &tasks, made))
            .collect::<Result<Vec<_>, _>>()?;

        // The task, then its parent, then everything else — the parent is what makes it part of
        // the mission, so it goes on before any todo does and a failure there is a failure of the
        // whole call rather than an orphan left on the board.
        let replies = board.create(project, spec.title.clone(), None);
        let Some(id) = replies.iter().find_map(|reply| match reply.message() {
            Message::TaskCreated { task, .. } => Some(task.id),
            _ => None,
        }) else {
            let error = replies
                .iter()
                .find_map(|reply| match reply.message() {
                    Message::WorkError { error, .. } => Some(error.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "the task was not created".to_string());
            return Err(error);
        };
        let mut all = replies;
        all.extend(board.set_field(project, id, TaskField::Parent(Some(mission))));
        if !spec.description.is_empty() {
            all.extend(board.update(project, id, None, Some(spec.description.clone()), None));
        }
        if !spec.labels.is_empty() {
            // The board's own reading of a label name — an existing colour where the project
            // already knows the label, so an agent never invents a second swatch for one name.
            let labels = super::tasks::labels_from_names(&mut board, project, spec.labels.clone());
            all.extend(board.set_field(project, id, TaskField::Labels(labels)));
        }
        if !prerequisites.is_empty() {
            all.extend(board.set_field(
                project,
                id,
                TaskField::Prerequisites(prerequisites.clone()),
            ));
        }
        for todo in &spec.todos {
            all.extend(board.add_step(project, id, todo.clone()));
        }
        (
            Made {
                id,
                key: None,
                title: spec.title.clone(),
                todos: spec.todos.len(),
                prerequisites,
                reference: spec.reference.clone(),
                kind: spec.kind.clone(),
            },
            all,
        )
    };

    // The board redraws in every window, the same as a click would have made it.
    let mut created = created;
    for reply in messages {
        let message = reply.into_message();
        match &message {
            Message::TaskCreated { task: record, .. }
            | Message::TaskChanged { task: record, .. } => {
                created.key = record.key.clone();
                created.todos = record.steps.len();
                access.everyone.send(message.clone());
            }
            Message::WorkError { error, .. } => {
                tracing::warn!("ubiq-mission/create_mission_task: {error}");
            }
            _ => {}
        }
    }
    Ok(created)
}

/// What a prerequisite string names: a task id, an existing task's key, or a `ref` given by an
/// earlier entry of this same batch. Checked in that order, and a name that matches none of the
/// three is a refusal rather than a dropped dependency.
fn resolve_prerequisite(
    named: &str,
    tasks: &[ubiq_proto::work::TaskRecord],
    made: &[Made],
) -> Result<TaskId, String> {
    let named = named.trim();
    if let Ok(id) = named.parse::<TaskId>() {
        return Ok(id);
    }
    if let Some(task) = tasks
        .iter()
        .find(|task| task.key.as_deref().is_some_and(|key| key == named))
    {
        return Ok(task.id);
    }
    if let Some(earlier) = made
        .iter()
        .find(|made| made.reference.as_deref() == Some(named))
    {
        return Ok(earlier.id);
    }
    Err(format!(
        "no task called '{named}' — a prerequisite is a task id, the key of a task that already \
         exists, or the 'ref' of a task earlier in this same call"
    ))
}

/// The journal lines and the suggested kinds a batch leaves behind, written once the board is
/// settled: a mission's record is not the place a half-written breakdown belongs.
fn finish_batch(
    by: Actor,
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
    made: &[Made],
) {
    for created in made {
        let noted =
            reach
                .missions
                .lock()
                .task_created(project, mission, by, created.id, &created.title);
        broadcast(&reach.everyone, noted);
    }
    let kinds: Vec<(TaskId, Option<String>)> = made
        .iter()
        .filter(|created| created.kind.is_some())
        .map(|created| (created.id, created.kind.clone()))
        .collect();
    if !kinds.is_empty() {
        let noted = reach
            .missions
            .lock()
            .set_task_kinds(project, mission, kinds);
        broadcast(&reach.everyone, noted);
    }
}

fn report_progress(
    arguments: &Value,
    by: Actor,
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
) -> Result<Value, String> {
    let text = required_str(arguments, "text")?.to_string();
    let about = match arguments.get("task_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => Some(
            raw.parse::<TaskId>()
                .map_err(|_| "task_id is not a task id".to_string())?,
        ),
        Some(_) => return Err("task_id must be a string".to_string()),
    };
    let replies = reach
        .missions
        .lock()
        .report_progress(project, mission, by, about, text.clone());
    broadcast(&reach.everyone, replies);
    Ok(json!({"in_mission": true, "reported": text}))
}

/// Ask for a phase move. The host decides whether it happens now or waits for the user, so the
/// answer says which — an agent that assumed the move happened would go on to work a phase the
/// mission is not in.
fn request_phase(
    arguments: &Value,
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
) -> Result<Value, String> {
    let phase = phase_named(required_str(arguments, "phase")?)?;
    let summary = arguments
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let replies = reach
        .missions
        .lock()
        .request_phase(project, mission, phase, summary);
    let refusal = replies.iter().find_map(|reply| match reply.message() {
        Message::MissionError { error, .. } => Some(error.clone()),
        _ => None,
    });
    broadcast(&reach.everyone, replies);
    if let Some(refusal) = refusal {
        return Err(refusal);
    }
    let now = record(project, mission, reach)?;
    let pending = now
        .pending_phase
        .as_ref()
        .is_some_and(|pending| pending.phase == phase);
    Ok(json!({
        "in_mission": true,
        "phase": now.phase.label(),
        "waiting_for_user": pending,
        "applied": now.phase == phase,
    }))
}

// ── the roster ──────────────────────────────────────────────────────

fn list_agents(
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
    work: Option<&WorkAccess>,
) -> Result<Value, String> {
    let record = record(project, mission, reach)?;
    let agents = match work {
        Some(access) => access.work.lock().agents(project).1,
        None => Vec::new(),
    };
    let members: Vec<Value> = record
        .roster
        .iter()
        .filter(|entry| entry.left_at.is_none())
        .map(|entry| {
            let live = agents.iter().find(|agent| agent.id == entry.agent);
            json!({
                "agent_id": entry.agent.to_string(),
                "role": role_name(entry.role),
                "labels": entry.labels,
                "joined_at": entry.joined_at,
                "name": live.map(|agent| agent.name.clone()),
                "task_id": live.and_then(|agent| agent.task).map(|task| task.to_string()),
                "activity": live.map(|agent| agent.activity.label()),
                "note": live.map(|agent| agent.note.clone()),
            })
        })
        .collect();
    Ok(json!({"in_mission": true, "agents": members}))
}

/// The mission's agent kinds, for an agent choosing one (M13).
///
/// **A definition name is the most this ever says about a launch.** An agent kind names a saved
/// setup; it never carries an account's credential material, and there is nothing here for one to
/// leak into. Its `description` is prose, not a launch fact — resolved from the named definition
/// itself (`AgentDefinition::description`), it is the one addition to that rule: what the saved
/// setup is for and which MCP servers it carries, so the caller can tell two kinds with the same
/// harness apart before spawning either.
///
/// An empty table is reported as empty, with the sentence that says what to do about it: the table
/// is seeded from the project's definitions by the dialog and edited in the panel, so a mission whose
/// table nobody has filled has genuinely nothing to offer and pretending otherwise would send the
/// agent into `spawn_agent` to be refused.
fn list_agent_kinds(
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
) -> Result<Value, String> {
    let record = record(project, mission, reach)?;
    let kinds: Vec<Value> = record
        .agent_kinds
        .iter()
        .map(|kind| {
            // The kind's own blurb is whoever set up the mission's words; the definition's own
            // description is the saved setup's own, written for exactly this — another agent
            // deciding what to spawn. Both are handed over: the first is why this kind exists in
            // this mission, the second is what it actually carries.
            let definition_description = kind.definition.as_deref().and_then(|id| {
                crate::agent::Agents::definition_description(
                    &reach.agent_definitions_root,
                    id,
                    Some(project),
                )
            });
            json!({
                "name": kind.name,
                "description": kind.description,
                "definition": kind.definition,
                "definition_description": definition_description,
                "labels": kind.labels,
                "default": record.default_kind.as_deref() == Some(kind.name.as_str()),
            })
        })
        .collect();
    let empty = kinds.is_empty();
    Ok(json!({
        "in_mission": true,
        "kinds": kinds,
        "message": empty.then_some(
            "This mission has no agent kinds set up yet, so there is nothing to spawn. Say what \
             kind of agent you need and why — a person adds them in the mission panel."
        ),
    }))
}

/// Ask the mission for another agent, and answer with the request id at once (M13).
///
/// **This does not wait and cannot.** The host relays; the window applies the spawn policy,
/// composes the launch and mints the [`AgentId`], and the outcome comes back to this agent later
/// as a prompt. A tool that blocked here would hold the listener's thread on a person's decision,
/// which is exactly what `ubiq-ask` needs a thread of its own for (`D138`) — and a spawn is not a
/// question, so it does not get one.
fn spawn_agent(
    arguments: &Value,
    by: Actor,
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
) -> Result<Value, String> {
    let kind = arguments
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let definition = arguments
        .get("definition")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let task = match arguments.get("task_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => Some(
            raw.parse::<TaskId>()
                .map_err(|_| "task_id is not a task id".to_string())?,
        ),
        Some(_) => return Err("task_id must be a string".to_string()),
    };
    let prompt = required_str(arguments, "prompt")?.to_string();
    let reason = required_str(arguments, "reason")?.to_string();

    let (id, replies) = reach.missions.lock().request_spawn(
        project, mission, by, &kind, definition, task, prompt, reason,
    )?;
    broadcast(&reach.everyone, replies);
    Ok(json!({
        "in_mission": true,
        "request_id": id.to_string(),
        "waiting_for_user": true,
        "message": "The request is with the person watching the mission. You will be told what \
                    happened, and which kind was used, as a later message. Do not wait for it — \
                    carry on with something else.",
    }))
}

/// Put a line in another member's thread, and in front of it if it is running.
///
/// **Roster members only**, and the check is [`crate::mission::Missions::message_member`]'s so the
/// rule has one reading. The delivery is both halves — the row and a prompt to the live
/// conversation — which is why this goes through the missions rather than straight to the board.
fn message_agent(
    arguments: &Value,
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
) -> Result<Value, String> {
    let agent: AgentId = required_str(arguments, "agent_id")?
        .parse()
        .map_err(|_| "agent_id is not an agent id".to_string())?;
    let text = required_str(arguments, "text")?.to_string();

    let replies = reach
        .missions
        .lock()
        .message_member(project, mission, agent, text)?;
    let delivered = replies
        .iter()
        .any(|reply| matches!(reply.message(), Message::AgentChanged { .. }));
    broadcast(&reach.everyone, replies);
    Ok(json!({"in_mission": true, "delivered": delivered}))
}

fn read_feedback(project: ProjectId, mission: TaskId, reach: &MissionReach) -> Value {
    let lines = reach.missions.lock().take_feedback(project, mission);
    json!({
        "in_mission": true,
        "feedback": lines.iter().map(|entry| json!({
            "at": entry.at,
            "text": entry.text,
        })).collect::<Vec<_>>(),
    })
}

// ── shared shapes ───────────────────────────────────────────────────

fn roster_json(record: &MissionRecord) -> Vec<Value> {
    record
        .roster
        .iter()
        .filter(|entry| entry.left_at.is_none())
        .map(|entry| {
            json!({
                "agent_id": entry.agent.to_string(),
                "role": role_name(entry.role),
                "labels": entry.labels,
            })
        })
        .collect()
}

fn journal_json(entry: &ubiq_proto::mission::JournalEntry) -> Value {
    json!({
        "seq": entry.seq,
        "at": entry.at,
        "kind": entry.event.kind(),
        "text": entry.text,
        "agent_id": entry.agent().map(|agent| agent.to_string()),
    })
}

fn role_name(role: MissionRole) -> &'static str {
    match role {
        MissionRole::Coordinator => "coordinator",
        MissionRole::Worker => "worker",
    }
}

fn doc_target(project: ProjectId, mission: TaskId, name: &str) -> Target {
    Target::MissionDoc {
        project,
        task: mission,
        name: name.to_string(),
    }
}

/// A bare document name — no path and no extension, which is what the mission's flat `docs/`
/// holds (M8). The separator check is [`Target::resolve`]'s own and is repeated here so the
/// refusal names the argument the agent passed rather than a handle it never saw.
fn document_name(arguments: &Value) -> Result<String, String> {
    let raw = required_str(arguments, "name")?.trim();
    let name = raw.strip_suffix(".md").unwrap_or(raw);
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(
            "name must be a bare document name — no folders, no path separators".to_string(),
        );
    }
    Ok(name.to_string())
}

/// The `Plan` reply as a document, or the refusal it carries instead — [`super::plan`]'s own
/// `plan_result`, saying "document" where that one says "plan".
fn document_result(name: &str, replies: &[Reply]) -> Result<Value, String> {
    for reply in replies {
        match reply.message() {
            Message::Plan { body, revision, .. } => {
                return Ok(json!({
                    "in_mission": true,
                    "name": name,
                    "body": body,
                    "revision": revision,
                }));
            }
            Message::PlanConflict { revision, .. } => {
                return Err(format!(
                    "the document has moved on and now stands at revision {revision}: nothing was \
                     written. Read it again, redo your edit against what is there now, and write \
                     with expected_revision {revision}."
                ));
            }
            Message::PlanError { error, .. } => return Err(error.clone()),
            _ => {}
        }
    }
    Err("the document was not answered".to_string())
}

fn phase_named(raw: &str) -> Result<Phase, String> {
    Phase::all()
        .into_iter()
        .find(|phase| phase.label() == raw.trim().to_ascii_lowercase())
        .ok_or_else(|| {
            format!(
                "unknown phase '{raw}': use one of {}",
                Phase::all()
                    .iter()
                    .map(|phase| phase.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn broadcast(everyone: &ubiq_proto::bus::Mailbox, replies: Vec<Reply>) {
    for reply in replies {
        if let Reply::Everyone(message) = reply {
            everyone.send(message);
        }
    }
}

fn string_list(arguments: &Value, key: &str) -> Vec<String> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    match arguments.get(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.as_str()),
        Some(Value::String(_)) => Err(format!("{key} must not be empty")),
        _ => Err(format!("{key} is required")),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use tempfile::TempDir;
    use ubiq_proto::bus;
    use ubiq_proto::work::{Level, Status, TaskRecord};

    use super::*;
    use crate::mcp::registry::ProjectFacts;
    use crate::store::memory::MemoryTaskStore;
    use crate::store::mission::MissionStore;
    use crate::store::plan::FilePlanStore;
    use crate::work::{Handle, Work};

    const PROJECT: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

    fn project() -> ProjectId {
        PROJECT.parse().unwrap()
    }

    fn facts(mission: Option<TaskId>) -> AgentFacts {
        AgentFacts {
            key: AgentId::generate().to_string(),
            name: "claude 1".to_string(),
            harness: "Claude Code".to_string(),
            cwd: "/tmp/project".to_string(),
            mission,
            project: ProjectFacts {
                id: PROJECT.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn anchor() -> TaskRecord {
        let mut task = TaskRecord::new(
            "Ship the thing".to_string(),
            None,
            Utc.with_ymd_and_hms(2026, 9, 24, 9, 0, 0).unwrap(),
        );
        task.level = Some(Level::Mission);
        task.status = Status::InProgress;
        task.description = "Everything it takes.".to_string();
        task
    }

    /// The three reaches over one temp config root and one in-memory board, plus the bus hub that
    /// has to stay alive for the mailboxes to carry anything.
    #[allow(clippy::type_complexity)]
    fn reaches(
        tasks: Vec<TaskRecord>,
    ) -> (
        MissionReach,
        WorkAccess,
        PlanReach,
        TempDir,
        bus::Hub,
        bus::HostEnd,
    ) {
        let dir = TempDir::new().unwrap();
        let (hub, host) = bus::hub();
        let work = Handle::new(Work::open(Box::new(MemoryTaskStore::with(
            project(),
            tasks,
        ))));
        let missions = crate::mission::Handle::new(crate::mission::Missions::open(
            MissionStore::new(dir.path().to_path_buf()),
            work.clone(),
            FilePlanStore::new(dir.path().to_path_buf()),
            host.voice(),
        ));
        let plans = crate::plan::Handle::new(crate::plan::Plans::open(
            FilePlanStore::new(dir.path().to_path_buf()),
            work.clone(),
        ));
        (
            MissionReach {
                missions,
                everyone: host.mailbox(bus::To::Everyone),
                agent_definitions_root: dir.path().to_path_buf(),
            },
            WorkAccess {
                work,
                everyone: host.mailbox(bus::To::Everyone),
                wake: host.voice(),
            },
            PlanReach {
                plans,
                everyone: host.mailbox(bus::To::Everyone),
            },
            dir,
            hub,
            host,
        )
    }

    /// An agent outside a mission is told so in a sentence, and nothing it calls is an error —
    /// a model that reads an error retries it, and there is nothing here to retry.
    #[test]
    fn a_call_from_an_agent_in_no_mission_answers_with_a_sentence() {
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![anchor()]);
        let facts = facts(None);

        for tool in [
            "mission_overview",
            "read_brief",
            "list_documents",
            "list_agents",
            "read_feedback",
        ] {
            let answer = call(tool, &json!({}), &facts, &mission, Some(&work), Some(&plan))
                .unwrap_or_else(|error| panic!("{tool} answered an error: {error}"));
            assert_eq!(answer["in_mission"], json!(false), "{tool}");
            assert!(
                answer["message"]
                    .as_str()
                    .is_some_and(|text| text.contains("not in a mission")),
                "{tool} said: {}",
                answer["message"]
            );
        }
    }

    /// A worker's server answers the read tools and refuses the ones that run the mission — the
    /// `manage` / `use` split, enforced where the tools are rather than only in the catalogue.
    #[test]
    fn a_worker_cannot_call_what_only_the_coordinator_may() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        assert!(
            use_call(
                "mission_overview",
                &json!({}),
                &facts,
                &mission,
                Some(&work),
                Some(&plan)
            )
            .is_ok()
        );
        for tool in ["write_document", "create_mission_task", "request_phase"] {
            let refused = use_call(tool, &json!({}), &facts, &mission, Some(&work), Some(&plan));
            assert_eq!(
                refused.unwrap_err(),
                format!("unknown tool: use-mission/{tool}"),
                "{tool}"
            );
        }
    }

    /// A stale `expected_revision` is refused with the revision to redo the edit against, and
    /// nothing is written — `write_plan`'s own contract, said about a document.
    #[test]
    fn write_document_refuses_a_stale_revision_and_writes_nothing() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        let first = call(
            "write_document",
            &json!({"name": "architecture", "body": "one"}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .expect("a first write needs no revision");
        let revision = first["revision"].as_u64().unwrap();

        let conflict = call(
            "write_document",
            &json!({"name": "architecture", "body": "two", "expected_revision": revision}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        );
        assert!(conflict.is_ok(), "writing at the revision read must work");

        let stale = call(
            "write_document",
            &json!({"name": "architecture", "body": "three", "expected_revision": revision}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap_err();
        assert!(stale.contains("moved on"), "{stale}");

        let body = call(
            "read_document",
            &json!({"name": "architecture"}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(
            body["body"],
            json!("two"),
            "the refused write wrote nothing"
        );

        // The document's name reached the record, which is how a window learns it exists.
        let listed = call(
            "list_documents",
            &json!({}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(listed["documents"], json!(["architecture"]));
    }

    /// A task an agent makes inside a mission is a child of the mission's anchor, keeps its todos,
    /// and is journaled.
    #[test]
    fn create_mission_task_parents_to_the_mission() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        let made = call(
            "create_mission_task",
            &json!({"title": "Wire the bus", "todos": ["read it", "write it"]}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(made["parent"], json!(id.to_string()));
        assert_eq!(made["todos"], json!(2));

        let child: TaskId = made["task_id"].as_str().unwrap().parse().unwrap();
        let (_, tasks) = work.work.lock().tasks(project());
        let record = tasks.iter().find(|task| task.id == child).unwrap();
        assert_eq!(record.parent, Some(id));
        assert_eq!(record.steps.len(), 2);

        // And the mission's own journal says it happened.
        let overview = call(
            "mission_overview",
            &json!({}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(overview["tasks"]["total"], json!(1));
        let kinds: Vec<&str> = overview["journal"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"task_created"), "{kinds:?}");
    }

    /// **The batch resolves in-batch references** (M21) — the whole reason it exists. Three tasks
    /// in one call, the second waiting on the first by the `ref` it gave itself and the third on
    /// both, with no id ever going back to the caller in between.
    #[test]
    fn create_mission_tasks_resolves_references_inside_the_batch() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        let made = call(
            "create_mission_tasks",
            &json!({"tasks": [
                {"ref": "schema", "title": "Write the schema", "labels": ["api"], "kind": "worker"},
                {"ref": "wire", "title": "Wire it up", "prerequisites": ["schema"], "labels": ["api", "ui"]},
                {"title": "Draw it", "prerequisites": ["schema", "wire"], "labels": ["ui"]},
            ]}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        let created = made["created"].as_array().unwrap();
        assert_eq!(created.len(), 3);
        assert_eq!(made["failed"], json!([]));

        let ids: Vec<TaskId> = created
            .iter()
            .map(|entry| entry["task_id"].as_str().unwrap().parse().unwrap())
            .collect();
        let (_, tasks) = work.work.lock().tasks(project());
        let of = |id: TaskId| tasks.iter().find(|task| task.id == id).unwrap();
        assert!(of(ids[0]).prerequisites.is_empty());
        assert_eq!(of(ids[1]).prerequisites, vec![ids[0]]);
        assert_eq!(of(ids[2]).prerequisites, vec![ids[0], ids[1]]);
        // Every one of them is a child of the mission, and readiness follows the graph.
        assert!(tasks.iter().filter(|t| t.parent == Some(id)).count() == 3);
        assert!(of(ids[0]).ready(&tasks));
        assert!(!of(ids[2]).ready(&tasks));

        // The suggested kind is on the mission's record, not on the card.
        let record = mission.missions.lock().record(project(), id).unwrap();
        assert_eq!(record.task_kind(ids[0]), Some("worker"));
        assert_eq!(record.task_kind(ids[1]), None);
    }

    /// A prerequisite naming nothing is a refusal for **that entry**, not a silent drop: a task
    /// created without the dependency it asked for is exactly what a scheduler then acts on.
    #[test]
    fn a_batch_entry_with_an_unknown_prerequisite_is_reported_not_dropped() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);

        let made = call(
            "create_mission_tasks",
            &json!({"tasks": [
                {"title": "Fine", "ref": "fine"},
                {"title": "Broken", "prerequisites": ["nothing-called-this"]},
            ]}),
            &facts(Some(id)),
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(made["created"].as_array().unwrap().len(), 1);
        let failed = made["failed"].as_array().unwrap();
        assert_eq!(failed.len(), 1);
        assert!(
            failed[0]["error"]
                .as_str()
                .unwrap()
                .contains("nothing-called-this"),
            "{failed:?}"
        );
        let (_, tasks) = work.work.lock().tasks(project());
        assert!(
            !tasks.iter().any(|task| task.title == "Broken"),
            "the entry that could not be resolved was not created at all"
        );
    }

    /// `report_progress` is one journal line and reads back on the overview — the whole of what a
    /// worker writes.
    #[test]
    fn report_progress_lands_in_the_journal() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        use_call(
            "report_progress",
            &json!({"text": "Read the contract."}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();

        let overview = call(
            "mission_overview",
            &json!({}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        let newest = &overview["journal"][0];
        assert_eq!(newest["kind"], json!("progress"));
        assert_eq!(newest["text"], json!("Read the contract."));
        // The line is attributed to the agent that wrote it, not to the host.
        assert_eq!(newest["agent_id"], json!(facts.key));
    }

    /// `message_agent` reaches roster members and nobody else — the one rule that keeps a
    /// mission's identity from being a way to talk to the whole project.
    #[test]
    fn message_agent_refuses_an_agent_off_the_roster() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        let stranger = AgentId::generate();
        let refused = call(
            "message_agent",
            &json!({"agent_id": stranger.to_string(), "text": "hello"}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap_err();
        assert!(
            refused.contains("not on this mission's roster"),
            "{refused}"
        );
    }

    /// Every tool the catalogue advertises on each server actually dispatches, and nothing else
    /// does. The catalogue is what a harness calls from, so a name only it knows is a tool the
    /// model is offered and then cannot use.
    #[test]
    fn the_catalogue_and_the_dispatch_offer_the_same_tools() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        for (slug, dispatch) in [
            (
                crate::mcp::catalogue::UBIQ_MISSION,
                call as fn(
                    &str,
                    &Value,
                    &AgentFacts,
                    &MissionReach,
                    Option<&WorkAccess>,
                    Option<&PlanReach>,
                ) -> Result<Value, String>,
            ),
            (crate::mcp::catalogue::USE_MISSION, use_call),
        ] {
            let spec = crate::mcp::catalogue::server(slug).expect("the server is in the catalogue");
            for tool in spec.tools {
                // Called with no arguments: a tool that needs some answers a missing-argument
                // sentence, which is still proof that the name reached a handler.
                let answer = dispatch(
                    tool.name,
                    &json!({}),
                    &facts,
                    &mission,
                    Some(&work),
                    Some(&plan),
                );
                if let Err(error) = &answer {
                    assert!(
                        !error.starts_with("unknown tool"),
                        "{slug}/{} is advertised and does not dispatch",
                        tool.name
                    );
                }
            }
        }

        // And the worker's server is a strict subset of the coordinator's.
        let manage = crate::mcp::catalogue::server(crate::mcp::catalogue::UBIQ_MISSION).unwrap();
        let worker = crate::mcp::catalogue::server(crate::mcp::catalogue::USE_MISSION).unwrap();
        for tool in worker.tools {
            assert!(
                manage.tools.iter().any(|other| other.name == tool.name),
                "use-mission offers {} and ubiq-mission does not",
                tool.name
            );
        }
    }

    /// `spawn_agent` answers a request id **and does not wait**: nothing about a launch is in the
    /// reply, because the launch has not happened and is not the host's to make happen.
    #[test]
    fn spawn_agent_answers_a_request_id_without_waiting() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        // An empty table is reported as empty rather than guessed at.
        let empty = call(
            "list_agent_kinds",
            &json!({}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(empty["kinds"], json!([]));
        assert!(
            empty["message"]
                .as_str()
                .is_some_and(|text| text.contains("no agent kinds")),
            "{}",
            empty["message"]
        );

        // Seeded, as the dialog will seed it from the project's definitions.
        mission.missions.lock().set_field(
            project(),
            id,
            ubiq_proto::mission::MissionField::AgentKinds(vec![ubiq_proto::mission::AgentKind {
                name: "worker".to_string(),
                description: "Does one task and stops.".to_string(),
                definition: Some("worker-definition".to_string()),
                ..Default::default()
            }]),
        );

        let kinds = call(
            "list_agent_kinds",
            &json!({}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(kinds["kinds"][0]["name"], json!("worker"));
        assert_eq!(
            kinds["kinds"][0]["description"],
            json!("Does one task and stops.")
        );
        // A kind names a definition and nothing else — never an account, never a credential.
        assert_eq!(kinds["kinds"][0]["definition"], json!("worker-definition"));

        let spawned = call(
            "spawn_agent",
            &json!({"kind": "worker", "prompt": "Wire the bus.", "reason": "It splits in two."}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        let request_id: ubiq_proto::ids::SpawnId =
            spawned["request_id"].as_str().unwrap().parse().unwrap();
        assert_eq!(spawned["waiting_for_user"], json!(true));
        assert!(spawned.get("agent_id").is_none(), "no agent exists yet");

        // And it is pending on the record, which is what the window draws the row from.
        let record = mission.missions.lock().record(project(), id).unwrap();
        assert_eq!(record.pending_spawns.len(), 1);
        assert_eq!(record.pending_spawn(request_id).unwrap().kind, "worker");
        assert_eq!(
            record.pending_spawn(request_id).unwrap().prompt,
            "Wire the bus."
        );

        // An unknown kind is a sentence the model can act on, not a silent default.
        let unknown = call(
            "spawn_agent",
            &json!({"kind": "architect", "prompt": "go", "reason": "because"}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap_err();
        assert!(unknown.contains("'worker'"), "{unknown}");
    }

    /// `list_agent_kinds` hands back not just the kind's own blurb but the description of the
    /// definition it names — the point of `AgentDefinition::description`: a spawning agent reads
    /// what the saved setup actually carries, not just what the mission's kind table calls it.
    #[test]
    fn list_agent_kinds_resolves_the_named_definitions_description() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        let mut agents = crate::agent::Agents::new(dir.path(), false);
        agents.set_commands(std::collections::BTreeMap::from([(
            "claude-code".to_string(),
            "echo".to_string(),
        )]));
        agents
            .save_definition(ubiq_proto::messages::AgentDefinition {
                id: "worker-definition".to_string(),
                description: Some("Reads a task and reports progress.".to_string()),
                agent_type: "claude-code".to_string(),
                account: None,
                model: None,
                mode: None,
                thinking: None,
                max_subagents: None,
                prompt: None,
                mcps: Vec::new(),
                mission_assistant: None,
                mission_coordinator: false,
                mission_worker: true,
                disabled: false,
                project: None,
            })
            .unwrap();

        mission.missions.lock().set_field(
            project(),
            id,
            ubiq_proto::mission::MissionField::AgentKinds(vec![
                ubiq_proto::mission::AgentKind {
                    name: "worker".to_string(),
                    description: "Does one task and stops.".to_string(),
                    definition: Some("worker-definition".to_string()),
                    ..Default::default()
                },
                ubiq_proto::mission::AgentKind {
                    name: "ghost".to_string(),
                    description: "Names nothing on disk.".to_string(),
                    definition: Some("no-such-definition".to_string()),
                    ..Default::default()
                },
            ]),
        );

        let kinds = call(
            "list_agent_kinds",
            &json!({}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(
            kinds["kinds"][0]["definition_description"],
            json!("Reads a task and reports progress.")
        );
        assert_eq!(
            kinds["kinds"][1]["definition_description"],
            json!(null),
            "a kind naming a definition that is not there gets null, not an error"
        );
    }

    /// The brief is the anchor task, read back in the words it was written with.
    #[test]
    fn read_brief_answers_the_anchor_task() {
        let task = anchor();
        let id = task.id;
        let (mission, work, plan, _dir, _hub, _host) = reaches(vec![task]);
        let facts = facts(Some(id));

        let brief = call(
            "read_brief",
            &json!({}),
            &facts,
            &mission,
            Some(&work),
            Some(&plan),
        )
        .unwrap();
        assert_eq!(brief["title"], json!("Ship the thing"));
        assert_eq!(brief["description"], json!("Everything it takes."));
    }
}
