//! The `ubiq-mission` server: a mission as tools its planner (and its final reviewer) can call.
//!
//! **No tool here takes a mission id.** Which mission the caller is in is
//! [`AgentFacts::mission`], filled at launch from the agent's assignment and moved by
//! `AssignAgent` — so the mission is resolved from the URL identity alone (`D102`), and an agent
//! cannot address a mission it is not in by naming one.
//!
//! **An agent outside a mission gets a sentence, not an error.** None of the ways to get there is
//! something the model did wrong, and the sentence says what is true.
//!
//! Workers have no mission server: the runner puts the task, the plan and the guidance in their
//! prompt, and they report through `use-task`. Mutations broadcast to every window (`D120`), and
//! every write that changes what the runner sees says [`Message::MissionSchedule`] to wake it.

use serde_json::{Value, json};
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::messages::{Message, TaskField};
use ubiq_proto::mission::Phase;
use ubiq_proto::work::{Attachment, Level, Status};

use super::registry::AgentFacts;
use super::{MissionReach, PlanReach, WorkAccess};
use crate::plan::{Saver, Target};
use crate::reply::Reply;

/// The mission server's tools.
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
    match tool {
        "mission_overview" => overview(project, mission, reach, work),
        "read_brief" => read_brief(project, mission, work),
        "list_documents" => Ok(list_documents(project, mission, reach)),
        "attach_document" => attach_document(arguments, project, mission, work),
        "detach_document" => detach_document(arguments, project, mission, work),
        "read_document" => read_document(arguments, project, mission, plan),
        "write_document" => write_document(arguments, facts, project, mission, reach, plan),
        "create_mission_task" => create_mission_task(arguments, project, mission, work),
        "create_mission_tasks" => create_mission_tasks(arguments, project, mission, work),
        "request_phase" => request_phase(arguments, project, mission, reach),
        _ => Err(format!("unknown tool: ubiq-mission/{tool}")),
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

/// The sentence every tool answers an agent outside a mission with — a result rather than an
/// `Err`, so a model does not retry it; `in_mission: false` is the machine-readable half.
fn outside() -> Value {
    json!({
        "in_mission": false,
        "message": "You are not in a mission, so there is nothing here to read or change. \
                    Carry on with the task you were given.",
    })
}

// ── reading ─────────────────────────────────────────────────────────

fn overview(
    project: ProjectId,
    mission: TaskId,
    reach: &MissionReach,
    work: Option<&WorkAccess>,
) -> Result<Value, String> {
    let access = work.ok_or_else(|| "this host has no task board".to_string())?;
    let (_, tasks) = access.work.lock().tasks(project);
    let parent = tasks
        .iter()
        .find(|task| task.id == mission && task.level == Some(Level::Mission))
        .ok_or_else(|| "this mission's task is gone".to_string())?;
    let settings = crate::mission::runner::settings_of(parent);
    let mut counts = serde_json::Map::new();
    let mut children = 0usize;
    for task in tasks.iter().filter(|task| task.parent == Some(mission)) {
        children += 1;
        let entry = counts
            .entry(task.status.label().to_string())
            .or_insert_with(|| json!(0));
        *entry = json!(entry.as_u64().unwrap_or(0) + 1);
    }
    let documents = reach.missions.lock().documents(project, mission);
    let pending = crate::mission::runner::pending_request(parent);
    Ok(json!({
        "in_mission": true,
        "task_id": mission.to_string(),
        "title": parent.title,
        "description": parent.description,
        "phase": Phase::of(parent.status).label(),
        "settings": {
            "driver": format!("{:?}", settings.driver).to_lowercase(),
            "planner": settings.planner,
            "worker": settings.worker,
            "parallel": settings.parallelism(),
            "approve": settings.approve,
        },
        "documents": documents,
        "tasks": {"total": children, "by_status": counts},
        "waiting_on_user": pending.map(|phase| format!("you asked for {}", phase.label())),
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
        // The same shape `attach_document` answers with and the tasks server writes, `kind` and
        // all: one reading of an attachment across every surface a model sees.
        "attachments": anchor
            .attachments
            .iter()
            .map(super::tasks::attachment_json)
            .collect::<Vec<_>>(),
        "linked_tasks": linked,
    }))
}

// ── attached documents ────────────────────────────────────────────────
//
// **A mission's attached documents are its anchor task's attachments, and nothing else** (M7). The
// brief is the anchor's fields — `read_brief` already reads them back and `ui/mission/full.rs`
// already draws them — so attaching one is writing `TaskField::Attachments` on the anchor, through
// the board every other task write goes through. A second set on `MissionRecord` would be a
// second shape for one idea and a second thing for the mission panel to merge.
//
// **Not the same as a mission *document*.** `write_document` creates one of the mission's own,
// in the mission store; these two only hold a reference to something that already exists
// somewhere else. The two tool descriptions in [`super::catalogue`] carry that distinction.

/// The reference an attachment carries, checked as far as anything here can check it.
///
/// **A target is stored, never resolved** — [`ubiq_proto::work::Attachment`]'s own discipline, and
/// the tasks server's. So the only wrongness this can see is wrongness in the *shape*: nothing at
/// all, or a `kb:` address that is not `kb:{source}:{path}` and so names no entry any reader could
/// ask `ubiq-kb` for. A path that does not exist is not refused here: the project it is relative
/// to is not this call's to read, and an attachment written before the file is a real thing to want.
fn attach_target(arguments: &Value) -> Result<Attachment, String> {
    let target = arguments
        .get("target")
        .or_else(|| arguments.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .ok_or_else(|| {
            "target must be a project-relative path or a kb:{source}:{path} address".to_string()
        })?;
    let label = arguments
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string);
    let attachment = Attachment {
        target: target.to_string(),
        label,
    };
    if attachment.is_kb()
        && attachment
            .kb_address()
            .is_none_or(|(source, path)| source.trim().is_empty() || path.trim().is_empty())
    {
        return Err(format!(
            "`{target}` is not a knowledge-base address; those are kb:{{source}}:{{path}}, as list_kb_documents gives them"
        ));
    }
    Ok(attachment)
}

/// The anchor's attachments as they stand, for a caller about to rewrite them.
fn attached(
    project: ProjectId,
    mission: TaskId,
    access: &WorkAccess,
) -> Result<Vec<Attachment>, String> {
    let (_, tasks) = access.work.lock().tasks(project);
    tasks
        .iter()
        .find(|task| task.id == mission)
        .map(|anchor| anchor.attachments.clone())
        .ok_or_else(|| "this mission's task is gone".to_string())
}

/// What both tools answer with: the whole set, in `read_brief`'s own shape, so a model never has
/// to re-read the brief to know where it stands.
fn attach_result(documents: &[Attachment], did: &str, target: &str) -> Value {
    json!({
        "in_mission": true,
        "attached": did,
        "target": target,
        "documents": documents
            .iter()
            .map(super::tasks::attachment_json)
            .collect::<Vec<_>>(),
    })
}

/// Attach an existing document to the mission's brief.
///
/// **Idempotent on the target.** Attaching something already attached replaces its label rather
/// than writing the target twice — a duplicate is never what was meant, and refusing would make a
/// coordinator that lost track of what it attached handle an error over nothing.
fn attach_document(
    arguments: &Value,
    project: ProjectId,
    mission: TaskId,
    work: Option<&WorkAccess>,
) -> Result<Value, String> {
    let access =
        work.ok_or_else(|| "this host has no task board to attach a document on".to_string())?;
    let attachment = attach_target(arguments)?;
    let mut documents = attached(project, mission, access)?;
    match documents
        .iter_mut()
        .find(|held| held.target == attachment.target)
    {
        Some(held) => held.label = attachment.label.clone(),
        None => documents.push(attachment.clone()),
    }
    let written = documents.clone();
    super::tasks::mutate(access, |board| {
        board.set_field(project, mission, TaskField::Attachments(written))
    })?;
    Ok(attach_result(&documents, "attached", &attachment.target))
}

/// Drop an attachment. The document itself is not this call's to touch — only the reference is.
fn detach_document(
    arguments: &Value,
    project: ProjectId,
    mission: TaskId,
    work: Option<&WorkAccess>,
) -> Result<Value, String> {
    let access =
        work.ok_or_else(|| "this host has no task board to detach a document on".to_string())?;
    let target = arguments
        .get("target")
        .or_else(|| arguments.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .ok_or_else(|| "target must be the target read_brief gives".to_string())?
        .to_string();
    let mut documents = attached(project, mission, access)?;
    let before = documents.len();
    documents.retain(|held| held.target != target);
    if documents.len() == before {
        return Err(format!(
            "no document is attached to this mission as `{target}`; read_brief lists what is"
        ));
    }
    let written = documents.clone();
    super::tasks::mutate(access, |board| {
        board.set_field(project, mission, TaskField::Attachments(written))
    })?;
    Ok(attach_result(&documents, "detached", &target))
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
fn write_document(
    arguments: &Value,
    facts: &AgentFacts,
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

    let target = super::plan::owned(doc_target(project, mission, &name), facts, plan_reach)?;
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

    // The runner's snapshot carries the document list, so a wake is how the name reaches a window.
    reach.missions.lock().document_written(project, mission);
    Ok(result)
}

/// What one entry of a breakdown says, singular or batched — [`create_mission_task`] and
/// [`create_mission_tasks`] read the same shape so the two tools can never drift.
struct TaskSpec {
    title: String,
    description: String,
    todos: Vec<String>,
    labels: Vec<String>,
    /// Ids, existing keys, or a `ref` named by an earlier entry of the same batch.
    prerequisites: Vec<String>,
    /// `None` takes the mission's default.
    approve: Option<bool>,
    assigned_to: Option<String>,
    /// The task this one reviews: an id, a key or an earlier `ref`.
    review_of: Option<String>,
    /// A name this entry answers to **inside this call only**. Never written down.
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
        prerequisites: string_list(arguments, "prerequisites"),
        approve: arguments.get("approve").and_then(Value::as_bool),
        assigned_to: optional_str(arguments, "assigned_to"),
        review_of: optional_str(arguments, "review_of"),
        reference: optional_str(arguments, "ref"),
    })
}

fn create_mission_task(
    arguments: &Value,
    project: ProjectId,
    mission: TaskId,
    work: Option<&WorkAccess>,
) -> Result<Value, String> {
    let access = work.ok_or_else(|| "this host has no task board".to_string())?;
    let spec = spec_from(arguments)?;
    let created = write_task(&spec, project, mission, access, &[])?;
    wake(access, project, mission);
    Ok(json!({
        "in_mission": true,
        "task_id": created.id.to_string(),
        "key": created.key,
        "parent": mission.to_string(),
        "todos": created.todos,
        "prerequisites": created.prerequisites.len(),
    }))
}

/// The whole breakdown in one call. An entry may wait on (or review) one created earlier in the
/// same call, named by the `ref` it gave itself. **Not transactional**: every entry that
/// succeeded is named in `created` and every one that did not in `failed`, so a second call can
/// fill the gaps.
fn create_mission_tasks(
    arguments: &Value,
    project: ProjectId,
    mission: TaskId,
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
    wake(access, project, mission);

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
}

/// Write one entry of a breakdown. Created in backlog while the mission is planning or in
/// breakdown (the runner ignores the draft), ready once it runs — so a task added mid-run is
/// picked up (contract §4).
fn write_task(
    spec: &TaskSpec,
    project: ProjectId,
    mission: TaskId,
    access: &WorkAccess,
    made: &[Made],
) -> Result<Made, String> {
    let (created, messages) = {
        let mut board = access.work.lock();
        let (_, tasks) = board.tasks(project);
        let parent = tasks
            .iter()
            .find(|task| task.id == mission)
            .ok_or_else(|| "this mission's task is gone".to_string())?;
        let settings = crate::mission::runner::settings_of(parent);
        let status = match Phase::of(parent.status) {
            Phase::Planning | Phase::Breakdown => Status::Backlog,
            _ => Status::Ready,
        };
        // Resolved before anything is written, so an entry naming a task nobody can find is
        // refused rather than created without the link.
        let mut prerequisites = spec
            .prerequisites
            .iter()
            .map(|named| resolve_reference(named, &tasks, made))
            .collect::<Result<Vec<_>, _>>()?;
        let review_of = spec
            .review_of
            .as_deref()
            .map(|named| resolve_reference(named, &tasks, made))
            .transpose()?;
        // A review waits on what it reviews: without the edge it would be ready at once and run
        // against work nobody has done yet.
        if let Some(of) = review_of
            && !prerequisites.contains(&of)
        {
            prerequisites.push(of);
        }

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
        all.extend(board.set_field(
            project,
            id,
            TaskField::Approve(spec.approve.unwrap_or(settings.approve)),
        ));
        if review_of.is_some() {
            all.extend(board.set_field(project, id, TaskField::ReviewOf(review_of)));
        }
        if spec.assigned_to.is_some() {
            all.extend(board.set_field(
                project,
                id,
                TaskField::AssignedTo(spec.assigned_to.clone()),
            ));
        }
        for todo in &spec.todos {
            all.extend(board.add_step(project, id, todo.clone()));
        }
        if status != Status::Backlog {
            all.extend(board.move_task(project, id, status, None));
        }
        (
            Made {
                id,
                key: None,
                title: spec.title.clone(),
                todos: spec.todos.len(),
                prerequisites,
                reference: spec.reference.clone(),
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
fn resolve_reference(
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
        "no task called '{named}' — name a task by id, by the key of a task that already exists, \
         or by the 'ref' of a task earlier in this same call"
    ))
}

/// Tell the user a stage is ready for them. Never moves the phase (contract §4).
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
        .request_phase(project, mission, phase, summary)?;
    for reply in replies {
        if let Message::TaskChanged { .. } = reply.message() {
            reach.everyone.send(reply.into_message());
        }
    }
    Ok(json!({
        "in_mission": true,
        "asked_for": phase.label(),
        "waiting_for_user": true,
        "message": "The user has been told. Stop here and wait for their answer.",
    }))
}

/// Wake the runner after a write it should see.
fn wake(access: &WorkAccess, project: ProjectId, mission: TaskId) {
    access.wake.say(Message::MissionSchedule {
        project_id: project,
        task_id: mission,
    });
}

fn optional_str(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
    match raw.trim().to_ascii_lowercase().as_str() {
        "breakdown" => Ok(Phase::Breakdown),
        "running" => Ok(Phase::Running),
        _ => Err(format!(
            "unknown phase '{raw}': use 'breakdown' or 'running'"
        )),
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
