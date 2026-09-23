//! The `ubiq-plan` server: read and write a task's plan, and answer its annotations, as tools a
//! hosted agent can call.
//!
//! `read_plan` and `write_plan` are staging slice 3. `list_annotations`, `reply_annotation` and
//! `resolve_annotation` are slice 4 — see `_docs/wip/planning-system.md`'s "The annotation MCP
//! surface". **Not built here**: an `annotate_plan`-style tool that opens a new annotation (this
//! slice answers existing ones, it does not add the human's kind), and the `ubiq-ask` parking
//! trick that would let an agent post an annotation and wait for the reply inside one turn — see
//! the module-level note in the staging card. Every call talks to [`crate::plan::Plans`] through
//! the shared [`super::PlanReach`], which is where the level check lives: a plan belongs to any
//! task carrying a [`ubiq_proto::work::Level`], and every [`crate::plan::Plans`] method refuses
//! anything else on its own, the same [`ubiq_proto::messages::Message::PlanError`] path a
//! window's own request would be refused on.

use serde_json::{Value, json};
use ubiq_proto::ids::{AnnotationId, ProjectId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::plan::{Annotation, PlanBlock};
use ubiq_proto::work::CommentAuthor;

use super::PlanReach;
use super::registry::AgentFacts;
use crate::plan::{Saver, Target};
use crate::reply::Reply;

pub fn call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    reach: &PlanReach,
) -> Result<Value, String> {
    let project = project_id(facts)?;
    match tool {
        "read_plan" => read_plan(arguments, project, reach),
        "write_plan" => write_plan(arguments, facts, project, reach),
        "plan_changes" => plan_changes(arguments, facts, project, reach),
        "list_annotations" => list_annotations(arguments, project, reach),
        "reply_annotation" => reply_annotation(arguments, project, reach),
        "resolve_annotation" => resolve_annotation(arguments, project, reach),
        _ => Err(format!("unknown tool: ubiq-plan/{tool}")),
    }
}

fn project_id(facts: &AgentFacts) -> Result<ProjectId, String> {
    facts
        .project
        .id
        .parse()
        .map_err(|_| "this agent has no project".to_string())
}

fn read_plan(arguments: &Value, project: ProjectId, reach: &PlanReach) -> Result<Value, String> {
    let task = task_id(arguments)?;
    let replies = reach.plans.lock().load(&Target::plan(project, task));
    plan_result(task, &replies)
}

/// Replace the plan's body, stamped as **this agent's** save.
///
/// [`Saver::agent`] is handed the agent's own key, which is what later lets its `plan_changes`
/// default to "since I last wrote this" rather than to some other agent's write. This is the only
/// path to [`ubiq_proto::plan::SaveOrigin::Agent`]; the interface's [`Message::SavePlan`] is the
/// only path to the other, and neither can reach the other's stamp.
///
/// **`expected_revision` is optional here and mandatory on the wire**, which is the one place the
/// two write paths differ. A window always holds a watermark — it cannot have a body without
/// having been told the revision it came at — so naming one costs it nothing and the check is free
/// to be unconditional. An agent may legitimately have none: writing a plan for a mission nobody
/// has planned is a first call with nothing read beforehand, and refusing it for want of a number
/// it was never given would be a worse failure than the race it guards. An agent that *did* read
/// the plan first should pass the revision `read_plan` handed it, and then gets the same refusal a
/// window gets.
fn write_plan(
    arguments: &Value,
    facts: &AgentFacts,
    project: ProjectId,
    reach: &PlanReach,
) -> Result<Value, String> {
    let task = task_id(arguments)?;
    let body = required_str(arguments, "body")?;
    let expected =
        match arguments.get("expected_revision") {
            None | Some(Value::Null) => None,
            Some(Value::Number(number)) => Some(number.as_u64().ok_or_else(|| {
                "expected_revision must be a whole number, zero or more".to_string()
            })?),
            Some(_) => return Err("expected_revision must be a number".to_string()),
        };
    let replies = {
        let mut plans = reach.plans.lock();
        plans.save(
            &Target::plan(project, task),
            body.to_string(),
            &Saver::agent(facts.key.clone()),
            expected,
        )
    };
    for reply in &replies {
        if let Reply::Everyone(message) = reply {
            reach.everyone.send(message.clone());
        }
    }
    plan_result(task, &replies)
}

/// Where the plan has been edited since a revision, and by how much.
///
/// **The default watermark is this agent's own last `write_plan`**, which is the question an
/// agent actually has: *what has a human done to my plan since I wrote it?* An agent that has
/// never written this plan has no watermark to default to, so it gets everything that is known —
/// which for a plan written before the provenance layer existed is nothing, and that is the
/// honest answer rather than a fabricated one.
fn plan_changes(
    arguments: &Value,
    facts: &AgentFacts,
    project: ProjectId,
    reach: &PlanReach,
) -> Result<Value, String> {
    let task = task_id(arguments)?;
    let asked =
        match arguments.get("since_revision") {
            None | Some(Value::Null) => None,
            Some(Value::Number(number)) => Some(number.as_u64().ok_or_else(|| {
                "since_revision must be a whole number, zero or more".to_string()
            })?),
            Some(_) => return Err("since_revision must be a number".to_string()),
        };

    let mut plans = reach.plans.lock();
    let since = match asked {
        Some(since) => Some(since),
        None => plans.last_written_by(&Target::plan(project, task), &facts.key),
    };
    let report = plans.change_report(&Target::plan(project, task), since)?;
    let stats = report.stats;

    let regions: Vec<Value> = report
        .regions
        .iter()
        .map(|region| {
            let block = region
                .block_id
                .and_then(|id| report.blocks.iter().find(|block| block.id == id));
            json!({
                "first_line": region.first_line,
                "last_line": region.last_line,
                "line_count": region.line_count(),
                "revision": region.revision,
                "origin": region.origin.label(),
                "block_id": region.block_id.map(|id| id.to_string()),
                "block_kind": block.map(|block| block.kind.as_str()),
                "block_text": block.map(|block| block.text.as_str()),
                "text": region.text,
            })
        })
        .collect();

    Ok(json!({
        "task_id": task.to_string(),
        "revision": stats.revision,
        "since_revision": stats.since_revision,
        "regions": regions,
        "stats": {
            "lines_added": stats.lines_added,
            "lines_removed": stats.lines_removed,
            "lines_modified": stats.lines_modified,
            "blocks_touched": stats.blocks_touched,
            "human_revisions": stats.human_revisions,
            "agent_revisions": stats.agent_revisions,
        },
    }))
}

/// Every annotation on a task's plan, each carrying the text of the block it names — the agent is
/// never handed a bare [`ubiq_proto::ids::BlockId`] and left to guess what it is about. Orphaned
/// annotations (`_docs/wip/planning-system.md`, decision 4) come back with `"block_text": null`
/// and `"orphaned": true`, so an agent does not try to answer a comment whose passage is gone.
///
/// Open by default — `include_resolved: true` asks for the closed ones too, since "list the
/// open annotations" is the ordinary loop and a resolved thread is rarely what an agent came for.
fn list_annotations(
    arguments: &Value,
    project: ProjectId,
    reach: &PlanReach,
) -> Result<Value, String> {
    let task = task_id(arguments)?;
    let include_resolved = matches!(arguments.get("include_resolved"), Some(Value::Bool(true)));
    let (blocks, annotations) = reach
        .plans
        .lock()
        .annotation_list(&Target::plan(project, task))?;
    let annotations: Vec<Value> = annotations
        .iter()
        .filter(|annotation| include_resolved || annotation.is_open())
        .map(|annotation| annotation_json(annotation, &blocks))
        .collect();
    Ok(json!({"task_id": task.to_string(), "annotations": annotations}))
}

fn reply_annotation(
    arguments: &Value,
    project: ProjectId,
    reach: &PlanReach,
) -> Result<Value, String> {
    let task = task_id(arguments)?;
    let annotation = annotation_id(arguments)?;
    let text = required_str(arguments, "text")?;
    let replies = {
        let mut plans = reach.plans.lock();
        plans.reply_to(
            &Target::plan(project, task),
            annotation,
            CommentAuthor::Agent,
            text.to_string(),
        )
    };
    annotation_result(annotation, &replies, reach)
}

/// Close an annotation, or reopen one — `resolved` defaults to `true`, so calling this with only
/// `annotation_id` is the ordinary "I answered it" case. **No author check**: any agent may
/// resolve any annotation, including one it did not open (`_docs/wip/planning-system.md`).
fn resolve_annotation(
    arguments: &Value,
    project: ProjectId,
    reach: &PlanReach,
) -> Result<Value, String> {
    let task = task_id(arguments)?;
    let annotation = annotation_id(arguments)?;
    let resolved = match arguments.get("resolved") {
        None => true,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err("resolved must be a boolean".to_string()),
    };
    let replies = {
        let mut plans = reach.plans.lock();
        plans.resolve(&Target::plan(project, task), annotation, resolved)
    };
    annotation_result(annotation, &replies, reach)
}

/// The `PlanAnnotations` reply, broadcast to every window and turned into just the one annotation
/// the caller asked about — an agent that replied or resolved one does not need the whole list
/// back, and does not need to re-derive which entry in it is theirs.
fn annotation_result(
    annotation: AnnotationId,
    replies: &[Reply],
    reach: &PlanReach,
) -> Result<Value, String> {
    for reply in replies {
        if let Reply::Everyone(message) = reply {
            reach.everyone.send(message.clone());
        }
    }
    for reply in replies {
        match reply.message() {
            Message::PlanAnnotations {
                blocks,
                annotations,
                ..
            } => {
                let found = annotations
                    .iter()
                    .find(|existing| existing.id == annotation)
                    .ok_or_else(|| "the annotation was not found after the change".to_string())?;
                return Ok(annotation_json(found, blocks));
            }
            Message::PlanError { error, .. } => return Err(error.clone()),
            _ => {}
        }
    }
    Err("the annotation was not answered".to_string())
}

/// One annotation, with the text of the block it names folded in — `None` when the block is gone,
/// which is exactly the `orphaned` case.
fn annotation_json(annotation: &Annotation, blocks: &[PlanBlock]) -> Value {
    let block = blocks.iter().find(|block| block.id == annotation.block_id);
    json!({
        "id": annotation.id.to_string(),
        "block_id": annotation.block_id.to_string(),
        "quote": annotation.quote,
        "state": annotation.state.label(),
        "orphaned": annotation.orphaned,
        "block_kind": block.map(|block| block.kind.as_str()),
        "block_text": block.map(|block| block.text.as_str()),
        "thread": annotation.thread.iter().map(|comment| json!({
            "author": comment.author.label(),
            "text": comment.text,
            "created_at": comment.created_at,
        })).collect::<Vec<_>>(),
        "created_at": annotation.created_at,
    })
}

/// The `Plan` reply as JSON, or the `PlanError` it carries instead.
fn plan_result(task: TaskId, replies: &[Reply]) -> Result<Value, String> {
    for reply in replies {
        match reply.message() {
            // `revision` rides back so the agent can hold a watermark and later ask
            // `plan_changes` what happened while it was away, without a second call to find out
            // where it stood.
            Message::Plan { body, revision, .. } => {
                return Ok(json!({
                    "task_id": task.to_string(),
                    "body": body,
                    "revision": revision,
                }));
            }
            // A refused save is a sentence here rather than a structured refusal: an agent has no
            // second press to make, and what it has to do — re-read the plan, redo the edit
            // against what is there now, and write again naming that revision — is a sentence's
            // worth of instruction. The revision it needs is in it.
            Message::PlanConflict { revision, .. } => {
                return Err(format!(
                    "the plan has moved on and now stands at revision {revision}: nothing was \
                     written. Read it again, redo your edit against what is there now, and write \
                     with expected_revision {revision}."
                ));
            }
            Message::PlanError { error, .. } => return Err(error.clone()),
            _ => {}
        }
    }
    Err("the plan was not answered".to_string())
}

fn task_id(arguments: &Value) -> Result<TaskId, String> {
    required_str(arguments, "task_id")?
        .parse()
        .map_err(|_| "not a task id".to_string())
}

fn annotation_id(arguments: &Value) -> Result<AnnotationId, String> {
    required_str(arguments, "annotation_id")?
        .parse()
        .map_err(|_| "not an annotation id".to_string())
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    match arguments.get(key) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value.as_str()),
        Some(Value::String(_)) => Err(format!("{key} must not be empty")),
        _ => Err(format!("{key} is required")),
    }
}
