//! The `manage-ubiq-tasks` server: the project's board, as tools a hosted agent can call.
//!
//! Every call takes the [`AgentFacts`]' project and talks to [`crate::work::Work`] through the
//! shared handle. Mutations that the board should redraw are posted to every window as the same
//! `TaskCreated` / `TaskChanged` / `TaskDeleted` a click would have produced, so an agent and a
//! person looking at the same project stay on one list.

use std::cell::RefCell;

use serde_json::{Value, json};
use ubiq_proto::ids::{ProjectId, StepId, TaskId};
use ubiq_proto::messages::{Message, TaskField};
use ubiq_proto::work::{
    Attachment, Comment, CommentAuthor, Complexity, Kind, Label, Priority, Status, Step, TaskRecord,
};

use super::WorkAccess;
use super::registry::AgentFacts;
use crate::reply::Reply;
use crate::work::Work;

/// First-N summaries in [`overview`] per column.
const OVERVIEW_PER_STATUS: usize = 10;
/// Ceiling on [`search_tasks`]' `maxRows`, so a missing bound cannot dump the whole board.
const SEARCH_ROWS_CAP: usize = 100;
const SEARCH_ROWS_DEFAULT: usize = 10;

pub fn manage_call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    access: &WorkAccess,
) -> Result<Value, String> {
    let project = project_id(facts)?;
    match tool {
        "overview" => overview(arguments, project, access),
        "search_tasks" => search_tasks(arguments, project, access),
        "list_tags" => list_tags(project, access),
        "create_tag" => create_tag(arguments, project, access),
        "create_task" => create_task(arguments, project, access),
        "update_task" => update_task(arguments, project, access),
        "delete_task" => delete_task(arguments, project, access),
        "add_todo" => add_todo(arguments, project, access),
        "update_todo" => update_todo(arguments, project, access),
        "delete_todo" => delete_todo(arguments, project, access),
        "get_task" => get_task(arguments, project, access),
        "add_comment" => add_comment(arguments, project, access),
        _ => Err(format!("unknown tool: manage-ubiq-tasks/{tool}")),
    }
}

pub fn use_call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    access: &WorkAccess,
) -> Result<Value, String> {
    let project = project_id(facts)?;
    match tool {
        "search_tasks" => search_tasks(arguments, project, access),
        "get_task" => get_task(arguments, project, access),
        "change_state" => change_state(arguments, project, access),
        "add_comment" => add_comment(arguments, project, access),
        "add_todo" => add_todo(arguments, project, access),
        "update_todo" => update_todo(arguments, project, access),
        "delete_todo" => delete_todo(arguments, project, access),
        _ => Err(format!("unknown tool: use-task/{tool}")),
    }
}

fn overview(arguments: &Value, project: ProjectId, access: &WorkAccess) -> Result<Value, String> {
    let wanted = opt_str_list(arguments, "categories")?
        .unwrap_or_default()
        .into_iter()
        .map(|name| parse_status(&name))
        .collect::<Result<Vec<_>, _>>()?;
    let tasks = load_tasks(access, project)?;
    let labels = load_labels(access, project)?;

    let columns: Vec<Status> = if wanted.is_empty() {
        Status::all().into_iter().collect()
    } else {
        Status::all()
            .into_iter()
            .filter(|status| wanted.contains(status))
            .collect()
    };

    let columns = columns
        .into_iter()
        .map(|status| {
            let in_column: Vec<&TaskRecord> =
                tasks.iter().filter(|task| task.status == status).collect();
            json!({
                "status": status.label(),
                "total": in_column.len(),
                "tasks": in_column
                    .iter()
                    .take(OVERVIEW_PER_STATUS)
                    .map(|task| task_summary(task, &tasks))
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "statuses": Status::all().iter().map(|status| status.label()).collect::<Vec<_>>(),
        "labels": labels.iter().map(label_json).collect::<Vec<_>>(),
        "columns": columns,
    }))
}

fn search_tasks(
    arguments: &Value,
    project: ProjectId,
    access: &WorkAccess,
) -> Result<Value, String> {
    let text = opt_str(arguments, "text")?
        .map(|text| text.trim().to_lowercase())
        .filter(|text| !text.is_empty());
    let statuses = opt_str_list(arguments, "statuses")?
        .unwrap_or_default()
        .into_iter()
        .map(|name| parse_status(&name))
        .collect::<Result<Vec<_>, _>>()?;
    let labels = opt_str_list(arguments, "labels")?
        .unwrap_or_default()
        .into_iter()
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    let priorities = opt_str_list(arguments, "priorities")?
        .unwrap_or_default()
        .into_iter()
        .map(|name| parse_priority(&name))
        .collect::<Result<Vec<_>, _>>()?;
    let max_rows = opt_usize(arguments, "maxRows")?.unwrap_or(SEARCH_ROWS_DEFAULT);
    let max_rows = max_rows.clamp(1, SEARCH_ROWS_CAP);
    let ready_only = opt_bool(arguments, "ready_only")?.unwrap_or(false);

    let tasks = load_tasks(access, project)?;
    let matched: Vec<&TaskRecord> = tasks
        .iter()
        .filter(|task| {
            if !statuses.is_empty() && !statuses.contains(&task.status) {
                return false;
            }
            if !priorities.is_empty() && !priorities.contains(&task.priority) {
                return false;
            }
            if !labels.is_empty() {
                let hit = task.labels.iter().any(|label| {
                    labels
                        .iter()
                        .any(|wanted| wanted == &label.name.to_lowercase())
                });
                if !hit {
                    return false;
                }
            }
            if ready_only && !task.ready(&tasks) {
                return false;
            }
            if let Some(text) = text.as_deref() {
                task_matches_text(task, text)
            } else {
                true
            }
        })
        .collect();

    Ok(json!({
        "total": matched.len(),
        "tasks": matched
            .iter()
            .take(max_rows)
            .map(|task| task_json(task, &tasks))
            .collect::<Vec<_>>(),
    }))
}

fn list_tags(project: ProjectId, access: &WorkAccess) -> Result<Value, String> {
    let labels = load_labels(access, project)?;
    Ok(json!({
        "tags": labels.iter().map(label_json).collect::<Vec<_>>(),
    }))
}

fn create_tag(arguments: &Value, project: ProjectId, access: &WorkAccess) -> Result<Value, String> {
    let name = required_str(arguments, "name")?;
    let colour = opt_usize(arguments, "colour")?.unwrap_or(0);
    let (replies, result) = {
        let mut work = access.work.lock();
        work.create_label(project, name.to_string(), colour)
    };
    warn_only(&replies);
    let label = result?;
    Ok(json!({"tag": label_json(&label)}))
}

fn create_task(
    arguments: &Value,
    project: ProjectId,
    access: &WorkAccess,
) -> Result<Value, String> {
    let title = required_str(arguments, "title")?;
    let description = opt_str(arguments, "description")?.map(str::to_string);
    let status = opt_str(arguments, "status")?
        .map(parse_status)
        .transpose()?;
    let priority = opt_str(arguments, "priority")?
        .map(parse_priority)
        .transpose()?;
    let kind = opt_str(arguments, "kind")?.map(parse_kind).transpose()?;
    let complexity = opt_str(arguments, "complexity")?
        .map(parse_complexity)
        .transpose()?;
    let assigned_to = opt_str(arguments, "assigned_to")?.map(str::to_string);
    let key = opt_str(arguments, "key")?.map(str::to_string);
    let link = opt_str(arguments, "link")?.map(str::to_string);
    let labels = opt_str_list(arguments, "labels")?;
    let attachments = opt_attachments(arguments, "attachments")?;
    let prerequisites = opt_task_id_list(arguments, "prerequisites")?;

    let prereq_error: RefCell<Option<String>> = RefCell::new(None);
    let messages = mutate(access, |work| {
        let mut replies = work.create(project, title.to_string(), None);
        let Some(id) = created_id(&replies) else {
            return replies;
        };
        if description.is_some() || priority.is_some() {
            replies.extend(work.update(project, id, None, description.clone(), priority));
        }
        if let Some(kind) = kind {
            replies.extend(work.set_field(project, id, TaskField::Kind(Some(kind))));
        }
        if let Some(complexity) = complexity {
            replies.extend(work.set_field(project, id, TaskField::Complexity(Some(complexity))));
        }
        if let Some(assigned_to) = assigned_to.clone() {
            replies.extend(work.set_field(project, id, TaskField::AssignedTo(Some(assigned_to))));
        }
        if let Some(key) = key.clone() {
            replies.extend(work.set_field(project, id, TaskField::Key(Some(key))));
        }
        if let Some(link) = link.clone() {
            replies.extend(work.set_field(project, id, TaskField::Link(Some(link))));
        }
        if let Some(names) = labels.clone() {
            let labels = labels_from_names(work, project, names);
            replies.extend(work.set_field(project, id, TaskField::Labels(labels)));
        }
        if let Some(attachments) = attachments.clone() {
            replies.extend(work.set_field(project, id, TaskField::Attachments(attachments)));
        }
        if let Some(prerequisites) = prerequisites.clone() {
            let field_replies =
                work.set_field(project, id, TaskField::Prerequisites(prerequisites));
            capture_work_error(&field_replies, &prereq_error);
            replies.extend(field_replies);
        }
        if let Some(status) = status {
            replies.extend(work.move_task(project, id, status, None));
        }
        replies
    })?;
    if let Some(error) = prereq_error.into_inner() {
        return Err(error);
    }
    let tasks = load_tasks(access, project)?;
    task_result(&messages, &tasks)
}

fn update_task(
    arguments: &Value,
    project: ProjectId,
    access: &WorkAccess,
) -> Result<Value, String> {
    let id = parse_task_id(required_str(arguments, "task_id")?)?;
    let title = opt_str(arguments, "title")?.map(str::to_string);
    let description = opt_str(arguments, "description")?.map(str::to_string);
    let status = opt_str(arguments, "status")?
        .map(parse_status)
        .transpose()?;
    let priority = opt_str(arguments, "priority")?
        .map(parse_priority)
        .transpose()?;
    let kind = opt_str(arguments, "kind")?.map(parse_kind).transpose()?;
    let complexity = opt_str(arguments, "complexity")?
        .map(parse_complexity)
        .transpose()?;
    let assigned_to = opt_str(arguments, "assigned_to")?.map(str::to_string);
    let key = opt_str(arguments, "key")?.map(str::to_string);
    let link = opt_str(arguments, "link")?.map(str::to_string);
    let labels = opt_str_list(arguments, "labels")?;
    let attachments = opt_attachments(arguments, "attachments")?;
    let prerequisites = opt_task_id_list(arguments, "prerequisites")?;

    let prereq_error: RefCell<Option<String>> = RefCell::new(None);
    let messages = mutate(access, |work| {
        let mut replies = Vec::new();
        if title.is_some() || description.is_some() || priority.is_some() {
            replies.extend(work.update(project, id, title.clone(), description.clone(), priority));
        }
        if let Some(kind) = kind {
            replies.extend(work.set_field(project, id, TaskField::Kind(Some(kind))));
        }
        if let Some(complexity) = complexity {
            replies.extend(work.set_field(project, id, TaskField::Complexity(Some(complexity))));
        }
        if let Some(assigned_to) = assigned_to.clone() {
            replies.extend(work.set_field(project, id, TaskField::AssignedTo(Some(assigned_to))));
        }
        if let Some(key) = key.clone() {
            replies.extend(work.set_field(project, id, TaskField::Key(Some(key))));
        }
        if let Some(link) = link.clone() {
            replies.extend(work.set_field(project, id, TaskField::Link(Some(link))));
        }
        if let Some(names) = labels.clone() {
            let labels = labels_from_names(work, project, names);
            replies.extend(work.set_field(project, id, TaskField::Labels(labels)));
        }
        if let Some(attachments) = attachments.clone() {
            replies.extend(work.set_field(project, id, TaskField::Attachments(attachments)));
        }
        if let Some(prerequisites) = prerequisites.clone() {
            let field_replies =
                work.set_field(project, id, TaskField::Prerequisites(prerequisites));
            capture_work_error(&field_replies, &prereq_error);
            replies.extend(field_replies);
        }
        if let Some(status) = status {
            replies.extend(work.move_task(project, id, status, None));
        }
        replies
    })?;
    if let Some(error) = prereq_error.into_inner() {
        return Err(error);
    }
    if messages.is_empty() {
        // Nothing changed, or nothing was sent: still answer the current record so the model
        // sees the task it named.
        let tasks = load_tasks(access, project)?;
        let task = tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or_else(|| "no such task".to_string())?;
        return Ok(json!({"task": task_json(task, &tasks), "changed": false}));
    }
    let tasks = load_tasks(access, project)?;
    let mut result = task_result(&messages, &tasks)?;
    if let Value::Object(ref mut map) = result {
        map.insert("changed".into(), json!(true));
    }
    Ok(result)
}

fn delete_task(
    arguments: &Value,
    project: ProjectId,
    access: &WorkAccess,
) -> Result<Value, String> {
    let id = parse_task_id(required_str(arguments, "task_id")?)?;
    mutate(access, |work| work.delete(project, id))?;
    Ok(json!({"deleted": true, "task_id": id.to_string()}))
}

fn get_task(arguments: &Value, project: ProjectId, access: &WorkAccess) -> Result<Value, String> {
    let id = parse_task_id(required_str(arguments, "task_id")?)?;
    let tasks = load_tasks(access, project)?;
    let task = tasks
        .iter()
        .find(|task| task.id == id)
        .ok_or_else(|| "no such task".to_string())?;
    Ok(json!({"task": task_json(task, &tasks)}))
}

fn add_comment(
    arguments: &Value,
    project: ProjectId,
    access: &WorkAccess,
) -> Result<Value, String> {
    let task_id = parse_task_id(required_str(arguments, "task_id")?)?;
    let text = required_str(arguments, "text")?;
    let messages = mutate(access, |work| {
        work.add_comment(project, task_id, CommentAuthor::Agent, text.to_string())
    })?;
    let task = last_changed(&messages)?;
    let comment = task
        .comments
        .last()
        .ok_or_else(|| "the comment was not added".to_string())?;
    Ok(json!({
        "task_id": task_id.to_string(),
        "comment": comment_json(comment),
    }))
}

fn change_state(
    arguments: &Value,
    project: ProjectId,
    access: &WorkAccess,
) -> Result<Value, String> {
    let id = parse_task_id(required_str(arguments, "task_id")?)?;
    let status = parse_status(required_str(arguments, "status")?)?;
    let messages = mutate(access, |work| work.move_task(project, id, status, None))?;
    // **This is how a worker finishes**, so it is the event a mission's scheduler most needs: the
    // task moving to `InReview` or `Done` frees a slot, and a task moving anywhere changes what
    // the pool holds. Said into the host's own inbox rather than decided here, because the
    // scheduler runs on the coordinator's thread and this is the listener's. Unconditional: a
    // change that wrote nothing is still worth a pass, and a pass on a mission in manual mode is
    // a record read.
    access.wake.say(Message::MissionSchedule {
        project_id: project,
        task_id: id,
    });
    if messages.is_empty() {
        let tasks = load_tasks(access, project)?;
        let task = tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or_else(|| "no such task".to_string())?;
        return Ok(json!({"task": task_json(task, &tasks), "changed": false}));
    }
    let tasks = load_tasks(access, project)?;
    let mut result = task_result(&messages, &tasks)?;
    if let Value::Object(ref mut map) = result {
        map.insert("changed".into(), json!(true));
    }
    Ok(result)
}

fn add_todo(arguments: &Value, project: ProjectId, access: &WorkAccess) -> Result<Value, String> {
    let task_id = parse_task_id(required_str(arguments, "task_id")?)?;
    let title = required_str(arguments, "title")?;
    let messages = mutate(access, |work| {
        work.add_step(project, task_id, title.to_string())
    })?;
    let task = last_changed(&messages)?;
    let todo = task
        .steps
        .last()
        .ok_or_else(|| "the sub-task was not added".to_string())?;
    Ok(json!({"task_id": task_id.to_string(), "todo": todo_json(todo)}))
}

fn update_todo(
    arguments: &Value,
    project: ProjectId,
    access: &WorkAccess,
) -> Result<Value, String> {
    let task_id = parse_task_id(required_str(arguments, "task_id")?)?;
    let todo_id = parse_step_id(required_str(arguments, "todo_id")?)?;
    let title = opt_str(arguments, "title")?.map(str::to_string);
    let done = opt_bool(arguments, "done")?;
    let messages = mutate(access, |work| {
        work.update_step(project, task_id, todo_id, title.clone(), done)
    })?;
    if messages.is_empty() {
        let tasks = load_tasks(access, project)?;
        let task = tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| "no such task".to_string())?;
        let todo = task
            .step(todo_id)
            .ok_or_else(|| "no such sub-task".to_string())?;
        return Ok(json!({
            "task_id": task_id.to_string(),
            "todo": todo_json(todo),
            "changed": false,
        }));
    }
    let task = last_changed(&messages)?;
    let todo = task
        .step(todo_id)
        .ok_or_else(|| "no such sub-task".to_string())?;
    Ok(json!({
        "task_id": task_id.to_string(),
        "todo": todo_json(todo),
        "changed": true,
    }))
}

fn delete_todo(
    arguments: &Value,
    project: ProjectId,
    access: &WorkAccess,
) -> Result<Value, String> {
    let task_id = parse_task_id(required_str(arguments, "task_id")?)?;
    let todo_id = parse_step_id(required_str(arguments, "todo_id")?)?;
    mutate(access, |work| work.remove_step(project, task_id, todo_id))?;
    Ok(json!({
        "deleted": true,
        "task_id": task_id.to_string(),
        "todo_id": todo_id.to_string(),
    }))
}

// ── work handle ─────────────────────────────────────────────────────

fn project_id(facts: &AgentFacts) -> Result<ProjectId, String> {
    facts
        .project
        .id
        .parse()
        .map_err(|_| "this agent has no project".to_string())
}

fn load_tasks(access: &WorkAccess, project: ProjectId) -> Result<Vec<TaskRecord>, String> {
    let (_replies, tasks) = {
        let mut work = access.work.lock();
        work.tasks(project)
    };
    Ok(tasks)
}

fn load_labels(access: &WorkAccess, project: ProjectId) -> Result<Vec<Label>, String> {
    let (_replies, labels) = {
        let mut work = access.work.lock();
        work.labels(project)
    };
    Ok(labels)
}

pub(super) fn labels_from_names(
    work: &mut Work,
    project: ProjectId,
    names: Vec<String>,
) -> Vec<Label> {
    let (_, known) = work.labels(project);
    names
        .into_iter()
        .filter_map(|name| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let colour = known
                .iter()
                .find(|label| label.name == name)
                .map(|label| label.colour)
                .unwrap_or(0);
            Some(Label::new(name, colour))
        })
        .collect()
}

fn mutate(
    access: &WorkAccess,
    run: impl FnOnce(&mut Work) -> Vec<Reply>,
) -> Result<Vec<Message>, String> {
    let replies = {
        let mut work = access.work.lock();
        run(&mut work)
    };
    let mut out = Vec::new();
    let mut error = None;
    for reply in replies {
        let message = reply.into_message();
        match &message {
            Message::WorkError { error: text, .. } => error = Some(text.clone()),
            Message::TaskCreated { .. }
            | Message::TaskChanged { .. }
            | Message::TaskDeleted { .. }
            | Message::AgentChanged { .. } => {
                access.everyone.send(message.clone());
                out.push(message);
            }
            _ => {}
        }
    }
    if let Some(error) = error
        && out.is_empty()
    {
        return Err(error);
    }
    Ok(out)
}

fn warn_only(replies: &[Reply]) {
    for reply in replies {
        if let Message::WorkError { error, .. } = reply.message() {
            tracing::warn!("manage-ubiq-tasks: {error}");
        }
    }
}

/// Records a [`TaskField::Prerequisites`] refusal (self, cross-project or cyclic) into `slot`, so
/// the caller can surface it even when other field writes in the same batch succeed and would
/// otherwise leave [`mutate`]'s aggregated error swallowed — see its `out.is_empty()` guard.
fn capture_work_error(replies: &[Reply], slot: &RefCell<Option<String>>) {
    for reply in replies {
        if let Message::WorkError { error, .. } = reply.message() {
            *slot.borrow_mut() = Some(error.clone());
        }
    }
}

fn created_id(replies: &[Reply]) -> Option<TaskId> {
    replies.iter().find_map(|reply| match reply.message() {
        Message::TaskCreated { task, .. } => Some(task.id),
        _ => None,
    })
}

fn task_result(messages: &[Message], tasks: &[TaskRecord]) -> Result<Value, String> {
    for message in messages.iter().rev() {
        match message {
            Message::TaskCreated { task, .. } | Message::TaskChanged { task, .. } => {
                return Ok(json!({"task": task_json(task, tasks)}));
            }
            _ => {}
        }
    }
    Err("the task was not written".to_string())
}

fn last_changed(messages: &[Message]) -> Result<TaskRecord, String> {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::TaskChanged { task, .. } => Some(task.clone()),
            _ => None,
        })
        .ok_or_else(|| "the task was not written".to_string())
}

// ── json ────────────────────────────────────────────────────────────

fn task_json(task: &TaskRecord, tasks: &[TaskRecord]) -> Value {
    json!({
        "id": task.id.to_string(),
        "title": task.title,
        "description": task.description,
        "status": task.status.label(),
        "priority": priority_name(task.priority),
        "kind": task.kind.map(|kind| kind.label()),
        "complexity": task.complexity.map(|complexity| complexity.label()),
        "assigned_to": task.assigned_to,
        "key": task.key,
        "link": task.link,
        "labels": task.labels.iter().map(label_json).collect::<Vec<_>>(),
        "attachments": task.attachments.iter().map(attachment_json).collect::<Vec<_>>(),
        "prerequisites": task.prerequisites.iter().map(|id| task_ref(tasks, *id)).collect::<Vec<_>>(),
        "ready": task.ready(tasks),
        "waiting_on": task.waiting_on(tasks).iter().map(|id| task_ref(tasks, *id)).collect::<Vec<_>>(),
        "todos": task.steps.iter().map(todo_json).collect::<Vec<_>>(),
        "comments": task.comments.iter().map(comment_json).collect::<Vec<_>>(),
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

fn task_summary(task: &TaskRecord, tasks: &[TaskRecord]) -> Value {
    json!({
        "id": task.id.to_string(),
        "key": task.key,
        "title": task.title,
        "status": task.status.label(),
        "priority": priority_name(task.priority),
        "complexity": task.complexity.map(|complexity| complexity.label()),
        "assigned_to": task.assigned_to,
        "labels": task.labels.iter().map(|label| &label.name).collect::<Vec<_>>(),
        "todos_done": task.done(),
        "todos_total": task.steps.len(),
        "ready": task.ready(tasks),
        "waiting_on": task.waiting_on(tasks).iter().map(|id| task_ref(tasks, *id)).collect::<Vec<_>>(),
    })
}

/// A prerequisite id as an agent can act on it: the other task's key if it has one, its raw id
/// otherwise. Mirrors how [`task_matches_text`] treats a key as the human-facing name.
fn task_ref(tasks: &[TaskRecord], id: TaskId) -> String {
    tasks
        .iter()
        .find(|task| task.id == id)
        .and_then(|task| task.key.clone())
        .unwrap_or_else(|| id.to_string())
}

fn todo_json(step: &Step) -> Value {
    json!({
        "id": step.id.to_string(),
        "title": step.title,
        "done": step.done(),
        "state": step.state.label(),
    })
}

fn comment_json(comment: &Comment) -> Value {
    json!({
        "id": comment.id.to_string(),
        "author": comment.author.label(),
        "text": comment.text,
        "created_at": comment.created_at.to_rfc3339(),
    })
}

/// One attachment as a model reads it. `kind` is the one thing the JSON says that the target does
/// not spell out for a reader skimming it: whether to open the path or ask `ubiq-kb` for it.
fn attachment_json(attachment: &Attachment) -> Value {
    json!({
        "target": attachment.target,
        "label": attachment.label,
        "kind": if attachment.is_kb() { "kb" } else { "file" },
    })
}

fn label_json(label: &Label) -> Value {
    json!({"name": label.name, "colour": label.colour})
}

fn priority_name(priority: Priority) -> &'static str {
    priority.label().unwrap_or("normal")
}

fn task_matches_text(task: &TaskRecord, text: &str) -> bool {
    let in_title = task.title.to_lowercase().contains(text);
    let in_description = task.description.to_lowercase().contains(text);
    let in_key = task
        .key
        .as_deref()
        .is_some_and(|key| key.to_lowercase().contains(text));
    let in_kind = task.kind.is_some_and(|kind| kind.label().contains(text));
    let in_assigned = task
        .assigned_to
        .as_deref()
        .is_some_and(|who| who.to_lowercase().contains(text));
    let in_labels = task
        .labels
        .iter()
        .any(|label| label.name.to_lowercase().contains(text));
    let in_todos = task
        .steps
        .iter()
        .any(|step| step.title.to_lowercase().contains(text));
    let in_comments = task
        .comments
        .iter()
        .any(|comment| comment.text.to_lowercase().contains(text));
    in_title
        || in_description
        || in_key
        || in_kind
        || in_assigned
        || in_labels
        || in_todos
        || in_comments
}

// ── arguments ───────────────────────────────────────────────────────

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    opt_str(arguments, key)?.ok_or_else(|| format!("{key} is required"))
}

fn opt_str<'a>(arguments: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn opt_bool(arguments: &Value, key: &str) -> Result<Option<bool>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("{key} must be a boolean")),
    }
}

fn opt_usize(arguments: &Value, key: &str) -> Result<Option<usize>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .map(|value| Some(value as usize))
            .ok_or_else(|| format!("{key} must be a non-negative integer")),
        Some(_) => Err(format!("{key} must be a number")),
    }
}

fn opt_str_list(arguments: &Value, key: &str) -> Result<Option<Vec<String>>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut values = Vec::new();
            for item in items {
                match item {
                    Value::String(value) => values.push(value.clone()),
                    Value::Object(object) => {
                        let name = object
                            .get("name")
                            .and_then(Value::as_str)
                            .ok_or_else(|| format!("{key} entries need a name"))?;
                        values.push(name.to_string());
                    }
                    _ => return Err(format!("{key} must be an array of names")),
                }
            }
            Ok(Some(values))
        }
        Some(Value::String(value)) => Ok(Some(vec![value.clone()])),
        Some(_) => Err(format!("{key} must be an array of names")),
    }
}

/// The whole attachment list a model sent, in either of the two shapes it may write it.
///
/// A bare string is the target; an object may carry a `label` beside it. `None` is "leave them
/// alone" and `Some(vec![])` clears them, the same distinction `labels` draws. Nothing here
/// validates the target — a project path and a `kb:{source}:{path}` address are both strings the
/// host stores and never resolves (see [`Attachment`]).
fn opt_attachments(arguments: &Value, key: &str) -> Result<Option<Vec<Attachment>>, String> {
    let malformed = || format!("{key} must be an array of paths or {{target, label}} objects");
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(vec![Attachment::new(value.clone())])),
        Some(Value::Array(items)) => {
            let mut values = Vec::new();
            for item in items {
                match item {
                    Value::String(value) => values.push(Attachment::new(value.clone())),
                    Value::Object(object) => {
                        let target = object
                            .get("target")
                            .or_else(|| object.get("path"))
                            .and_then(Value::as_str)
                            .ok_or_else(|| format!("{key} entries need a target"))?;
                        values.push(Attachment {
                            target: target.to_string(),
                            label: object
                                .get("label")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        });
                    }
                    _ => return Err(malformed()),
                }
            }
            Ok(Some(values))
        }
        Some(_) => Err(malformed()),
    }
}

/// A list of task ids, in either shape [`opt_str_list`] accepts. `task_id` elsewhere in this file
/// is a raw id too, never a key — this matches that, rather than inventing a second convention.
fn opt_task_id_list(arguments: &Value, key: &str) -> Result<Option<Vec<TaskId>>, String> {
    opt_str_list(arguments, key)?
        .map(|values| values.iter().map(|value| parse_task_id(value)).collect())
        .transpose()
}

fn parse_task_id(value: &str) -> Result<TaskId, String> {
    value.parse().map_err(|_| format!("not a task id: {value}"))
}

fn parse_step_id(value: &str) -> Result<StepId, String> {
    value.parse().map_err(|_| format!("not a todo id: {value}"))
}

fn parse_status(value: &str) -> Result<Status, String> {
    let compact: String = value
        .trim()
        .to_lowercase()
        .chars()
        .filter(|ch| *ch != ' ' && *ch != '-' && *ch != '_')
        .collect();
    match compact.as_str() {
        "backlog" => Ok(Status::Backlog),
        "ready" => Ok(Status::Ready),
        "blocked" => Ok(Status::Blocked),
        "inprogress" => Ok(Status::InProgress),
        "inreview" => Ok(Status::InReview),
        "done" => Ok(Status::Done),
        "abandoned" => Ok(Status::Abandoned),
        _ => Err(format!(
            "unknown status '{value}': use backlog, ready, blocked, in progress, in review, done, or abandoned"
        )),
    }
}

fn parse_priority(value: &str) -> Result<Priority, String> {
    match value.trim().to_lowercase().as_str() {
        "low" => Ok(Priority::Low),
        "normal" => Ok(Priority::Normal),
        "high" => Ok(Priority::High),
        _ => Err(format!(
            "unknown priority '{value}': use low, normal, or high"
        )),
    }
}

fn parse_complexity(value: &str) -> Result<Complexity, String> {
    match value.trim().to_lowercase().as_str() {
        "low" | "l" => Ok(Complexity::Low),
        "medium" | "m" => Ok(Complexity::Medium),
        "high" | "h" => Ok(Complexity::High),
        _ => Err(format!(
            "unknown complexity '{value}': use low, medium, or high"
        )),
    }
}

fn parse_kind(value: &str) -> Result<Kind, String> {
    match value.trim().to_lowercase().as_str() {
        "bug" => Ok(Kind::Bug),
        "feature" => Ok(Kind::Feature),
        "chore" => Ok(Kind::Chore),
        "docs" => Ok(Kind::Docs),
        _ => Err(format!(
            "unknown kind '{value}': use bug, feature, chore, or docs"
        )),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use ubiq_proto::bus;
    use ubiq_proto::ids::ProjectId;

    use super::*;
    use crate::mcp::WorkAccess;
    use crate::mcp::registry::{AgentFacts, ProjectFacts};
    use crate::store::memory::MemoryTaskStore;
    use crate::work::{Handle, Work};

    const PROJECT: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

    fn facts() -> AgentFacts {
        AgentFacts {
            key: "01JBTESTAGENTIDULID000000".to_string(),
            name: "claude 1".to_string(),
            harness: "Claude Code".to_string(),
            account: Some("work".to_string()),
            model: Some("opus".to_string()),
            mode: Some("default".to_string()),
            cwd: "/tmp/project".to_string(),
            mission: None,
            session: None,
            project: ProjectFacts {
                id: PROJECT.to_string(),
                name: "Ubiq".to_string(),
                path: "/tmp/project".to_string(),
                colour: 3,
            },
        }
    }

    /// A `WorkAccess` backed by an in-memory store, and the hub it broadcasts through — held for
    /// the test's life, the same discipline `mcp::server`'s own fixtures follow, since a `Voice`
    /// or a `Mailbox` holds only a weak sender.
    fn access() -> (WorkAccess, bus::Hub, bus::HostEnd) {
        let (hub, host) = bus::hub();
        let work = Handle::new(Work::open(Box::new(MemoryTaskStore::new())));
        let access = WorkAccess {
            work,
            everyone: host.mailbox(bus::To::Everyone),
            wake: host.voice(),
        };
        (access, hub, host)
    }

    fn project() -> ProjectId {
        PROJECT.parse().unwrap()
    }

    fn task_id(value: &Value) -> String {
        value["task"]["id"].as_str().unwrap().to_string()
    }

    /// A task is minted with an auto-assigned `T-<n>` key the moment it has none of its own
    /// (`Work::create`), so a prerequisite naming it by id is still reported back by that key —
    /// see [`super::task_ref`]. Tests that read `waiting_on`/`prerequisites` need this, not the
    /// raw id `task_id` returns.
    fn task_key(value: &Value) -> String {
        value["task"]["key"].as_str().unwrap().to_string()
    }

    #[test]
    fn create_task_sets_prerequisites_and_reports_readiness() {
        let (access, _hub, _host) = access();
        let facts = facts();

        let a = manage_call("create_task", &json!({"title": "a"}), &facts, &access).unwrap();
        let a_id = task_id(&a);
        let a_key = task_key(&a);

        let b = manage_call(
            "create_task",
            &json!({"title": "b", "prerequisites": [a_id]}),
            &facts,
            &access,
        )
        .unwrap();
        assert_eq!(b["task"]["prerequisites"], json!([a_key]));
        assert_eq!(b["task"]["ready"], json!(false));
        assert_eq!(b["task"]["waiting_on"], json!([a_key]));
    }

    #[test]
    fn waiting_on_reports_keys_not_ids_when_a_key_is_set() {
        let (access, _hub, _host) = access();
        let facts = facts();

        let a = manage_call(
            "create_task",
            &json!({"title": "a", "key": "UBQ-1"}),
            &facts,
            &access,
        )
        .unwrap();
        let a_id = task_id(&a);

        let b = manage_call(
            "create_task",
            &json!({"title": "b", "prerequisites": [a_id]}),
            &facts,
            &access,
        )
        .unwrap();
        assert_eq!(b["task"]["waiting_on"], json!(["UBQ-1"]));
    }

    #[test]
    fn task_becomes_ready_once_its_prerequisite_is_in_review() {
        let (access, _hub, _host) = access();
        let facts = facts();

        let a = manage_call("create_task", &json!({"title": "a"}), &facts, &access).unwrap();
        let a_id = task_id(&a);
        let b = manage_call(
            "create_task",
            &json!({"title": "b", "prerequisites": [a_id]}),
            &facts,
            &access,
        )
        .unwrap();
        let b_id = task_id(&b);

        manage_call(
            "update_task",
            &json!({"task_id": a_id, "status": "in review"}),
            &facts,
            &access,
        )
        .unwrap();

        let refreshed =
            manage_call("get_task", &json!({"task_id": b_id}), &facts, &access).unwrap();
        assert_eq!(refreshed["task"]["ready"], json!(true));
        assert_eq!(refreshed["task"]["waiting_on"], json!([]));
    }

    #[test]
    fn update_task_replaces_the_whole_prerequisite_list() {
        let (access, _hub, _host) = access();
        let facts = facts();

        let a = manage_call("create_task", &json!({"title": "a"}), &facts, &access).unwrap();
        let a_id = task_id(&a);
        let b = manage_call("create_task", &json!({"title": "b"}), &facts, &access).unwrap();
        let b_id = task_id(&b);
        let b_key = task_key(&b);
        let c = manage_call(
            "create_task",
            &json!({"title": "c", "prerequisites": [a_id]}),
            &facts,
            &access,
        )
        .unwrap();
        let c_id = task_id(&c);

        let updated = manage_call(
            "update_task",
            &json!({"task_id": c_id, "prerequisites": [b_id]}),
            &facts,
            &access,
        )
        .unwrap();
        assert_eq!(updated["task"]["prerequisites"], json!([b_key]));
    }

    #[test]
    fn a_self_prerequisite_is_refused_and_the_refusal_is_not_swallowed() {
        let (access, _hub, _host) = access();
        let facts = facts();

        let a = manage_call("create_task", &json!({"title": "a"}), &facts, &access).unwrap();
        let a_id = task_id(&a);

        // Mixed with another field that would otherwise succeed and produce a `TaskChanged`,
        // masking the refusal behind `mutate`'s "some message came back" check.
        let result = manage_call(
            "update_task",
            &json!({"task_id": a_id, "title": "renamed", "prerequisites": [a_id]}),
            &facts,
            &access,
        );
        let error = result.expect_err("a self-prerequisite must be refused");
        assert!(
            error.contains("own prerequisite"),
            "unexpected refusal text: {error}"
        );
    }

    #[test]
    fn search_tasks_ready_only_filters_out_waiting_tasks() {
        let (access, _hub, _host) = access();
        let facts = facts();

        let a = manage_call("create_task", &json!({"title": "a"}), &facts, &access).unwrap();
        let a_id = task_id(&a);
        manage_call(
            "create_task",
            &json!({"title": "b", "prerequisites": [a_id]}),
            &facts,
            &access,
        )
        .unwrap();

        let ready = manage_call(
            "search_tasks",
            &json!({"ready_only": true}),
            &facts,
            &access,
        )
        .unwrap();
        let titles: Vec<&str> = ready["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| task["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, vec!["a"]);
    }

    #[test]
    fn use_task_server_also_reports_readiness() {
        let (access, _hub, _host) = access();
        let facts = facts();

        let a = manage_call("create_task", &json!({"title": "a"}), &facts, &access).unwrap();
        let a_id = task_id(&a);
        let a_key = task_key(&a);
        let b = manage_call(
            "create_task",
            &json!({"title": "b", "prerequisites": [a_id]}),
            &facts,
            &access,
        )
        .unwrap();
        let b_id = task_id(&b);

        let via_use_task =
            use_call("get_task", &json!({"task_id": b_id}), &facts, &access).unwrap();
        assert_eq!(via_use_task["task"]["ready"], json!(false));
        assert_eq!(via_use_task["task"]["waiting_on"], json!([a_key]));

        let searched = use_call(
            "search_tasks",
            &json!({"ready_only": true}),
            &facts,
            &access,
        )
        .unwrap();
        assert_eq!(searched["total"], json!(1));
    }

    /// Sanity check on [`project`] itself: the fixture id parses to the project [`facts`] names,
    /// which everything above relies on silently.
    #[test]
    fn fixture_project_id_matches_facts() {
        assert_eq!(project().to_string(), facts().project.id);
    }
}
