//! The `ubiq-plan` and `ubiq-doc` servers: read and write an annotated markdown document, and
//! answer its annotations, as tools a hosted agent can call.
//!
//! **One set of handlers, two ways of naming the document.** Every handler below takes a resolved
//! [`Target`] and never asks which kind it is: `ubiq-plan` resolves one from a `task_id` (a task's
//! plan), `ubiq-doc` from a `path` (any markdown file in the agent's project — `D161`'s
//! `DocumentHandle::File`). The block matcher, the orphaning rule, the conflict check and the
//! provenance layer are [`crate::plan::Plans`]'s and serve both, so a collaboration on a spec and
//! a collaboration on a plan cannot drift apart. What differs is only the field each answer echoes
//! back to say which document it is about: `task_id` for a plan, `path` for a file.
//!
//! `read_plan` and `write_plan` are staging slice 3. `list_annotations`, `reply_annotation` and
//! `resolve_annotation` are slice 4 — see `_docs/wip/planning-system.md`'s "The annotation MCP
//! surface"; `annotate_plan` opens a thread as the agent, anchored by quote or block id.
//! **Not built here**: the `ubiq-ask` parking trick that would let an agent post an annotation
//! and wait for the reply inside one turn. Every call talks to [`crate::plan::Plans`] through
//! the shared [`super::PlanReach`], which is where the level check lives: a plan belongs to any
//! task carrying a [`ubiq_proto::work::Level`], and every [`crate::plan::Plans`] method refuses
//! anything else on its own, the same [`ubiq_proto::messages::Message::PlanError`] path a
//! window's own request would be refused on. A file document has no such check; being markdown
//! and inside the project is settled when its [`Target`] is resolved.

use std::path::Path;

use serde_json::{Map, Value, json};
use ubiq_proto::ids::{AnnotationId, ProjectId};
use ubiq_proto::messages::Message;
use ubiq_proto::plan::{Annotation, AnnotationMark, DocumentHandle, PlanBlock};
use ubiq_proto::work::CommentAuthor;

use super::PlanReach;
use super::registry::AgentFacts;
use crate::plan::Target;
use crate::reply::Reply;

/// How many annotated documents `list_annotated_docs` reports before it stops walking. A project
/// with more than this many annotated files is not one an agent reads a list of.
const MAX_ANNOTATED_DOCS: usize = 500;

/// The `ubiq-plan` server: the document is a task's plan, named by `task_id`.
pub fn call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    reach: &PlanReach,
) -> Result<Value, String> {
    let project = project_id(facts)?;
    let plan = || -> Result<Target, String> {
        let task = required_str(arguments, "task_id")?
            .parse()
            .map_err(|_| "not a task id".to_string())?;
        Ok(Target::plan(project, task))
    };
    match tool {
        "read_plan" => read(&plan()?, facts, reach),
        "write_plan" => write(arguments, facts, &owned(plan()?, facts, reach)?, reach),
        "plan_changes" => changes(arguments, facts, &plan()?, reach),
        "list_annotations" => list_annotations(arguments, &plan()?, reach),
        "annotate_plan" => annotate(arguments, &owned(plan()?, facts, reach)?, reach),
        "reply_annotation" => reply_annotation(arguments, &owned(plan()?, facts, reach)?, reach),
        "resolve_annotation" => {
            resolve_annotation(arguments, &owned(plan()?, facts, reach)?, reach)
        }
        _ => Err(format!("unknown tool: ubiq-plan/{tool}")),
    }
}

/// The `ubiq-doc` server: the document is a markdown file in the agent's project, named by a
/// project-relative `path` and resolved against the project's own root — the same containment and
/// markdown check a window's [`DocumentHandle::File`] goes through in `Coordinator::plan_job`.
pub fn doc_call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    reach: &PlanReach,
) -> Result<Value, String> {
    let project = project_id(facts)?;
    let root = Path::new(&facts.project.path);
    if tool == "list_annotated_docs" {
        return list_annotated_docs(arguments, project, root, reach);
    }
    if tool == "list_my_docs" {
        return list_my_docs(facts, project, root, reach);
    }
    let file = || -> Result<Target, String> {
        let rel_path = required_str(arguments, "path")?;
        file_target(project, root, rel_path)
    };
    match tool {
        "read_doc" => read(&file()?, facts, reach),
        "write_doc" => write(arguments, facts, &owned(file()?, facts, reach)?, reach),
        "doc_changes" => changes(arguments, facts, &file()?, reach),
        "list_annotations" => list_annotations(arguments, &file()?, reach),
        "annotate_doc" => annotate(arguments, &owned(file()?, facts, reach)?, reach),
        "reply_annotation" => reply_annotation(arguments, &owned(file()?, facts, reach)?, reach),
        "resolve_annotation" => {
            resolve_annotation(arguments, &owned(file()?, facts, reach)?, reach)
        }
        _ => Err(format!("unknown tool: ubiq-doc/{tool}")),
    }
}

fn file_target(project: ProjectId, root: &Path, rel_path: &str) -> Result<Target, String> {
    // Forward slashes and no leading `./` or `/`: the same spelling the explorer and the sidecar
    // walk below use, so one file is one `DocumentHandle` key whoever named it.
    let rel_path = rel_path
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string();
    Target::resolve(
        &DocumentHandle::File {
            project_id: project,
            rel_path,
        },
        root,
    )
}

fn project_id(facts: &AgentFacts) -> Result<ProjectId, String> {
    facts
        .project
        .id
        .parse()
        .map_err(|_| "this agent has no project".to_string())
}

/// Which document an answer is about: `{"task_id": …}` for a plan, `{"path": …}` for a file.
fn ident(target: &Target) -> Map<String, Value> {
    let mut map = Map::new();
    match target.handle() {
        DocumentHandle::Plan { task_id, .. } => {
            map.insert("task_id".into(), json!(task_id.to_string()));
        }
        DocumentHandle::File { rel_path, .. } => {
            map.insert("path".into(), json!(rel_path));
        }
        DocumentHandle::MissionDoc { task_id, name, .. } => {
            map.insert("task_id".into(), json!(task_id.to_string()));
            map.insert("name".into(), json!(name));
        }
    }
    map
}

/// `fields` with the document's identity in front.
fn with_ident(target: &Target, fields: Value) -> Value {
    let mut map = ident(target);
    if let Value::Object(fields) = fields {
        map.extend(fields);
    }
    Value::Object(map)
}

/// What the answers call the document in a sentence.
fn noun(target: &Target) -> &'static str {
    match target {
        Target::Plan { .. } => "plan",
        _ => "document",
    }
}

/// Every markdown file in the project that carries an annotation sidecar
/// (`<file>.md.annotation.json`, `D161`), with how many threads are open on it — the "what did
/// the user leave for me" question an agent asks before it knows a path.
///
/// The walk keeps the project's own ignore rules, as the search does: a sidecar is committed with
/// its file, so one the repository ignores is one nobody meant to share.
fn list_annotated_docs(
    arguments: &Value,
    project: ProjectId,
    root: &Path,
    reach: &PlanReach,
) -> Result<Value, String> {
    let include_resolved = matches!(arguments.get("include_resolved"), Some(Value::Bool(true)));
    let (paths, truncated) = annotated_paths(root);

    let mut docs = Vec::new();
    for path in paths {
        let Ok(target) = file_target(project, root, &path) else {
            continue;
        };
        let Ok((_, annotations, _)) = reach.plans.lock().annotation_list(&target) else {
            continue;
        };
        let open = annotations.iter().filter(|a| a.is_open()).count();
        if open == 0 && !include_resolved {
            continue;
        }
        docs.push(json!({
            "path": path,
            "open": open,
            "total": annotations.len(),
        }));
    }
    Ok(json!({"documents": docs, "truncated": truncated}))
}

/// The caller's working documents: every annotated markdown file of the project bound to this
/// agent (`D207`), with how many threads are open on each and whether auto-send is on.
fn list_my_docs(
    facts: &AgentFacts,
    project: ProjectId,
    root: &Path,
    reach: &PlanReach,
) -> Result<Value, String> {
    let (paths, truncated) = annotated_paths(root);
    let mut docs = Vec::new();
    for path in paths {
        let Ok(target) = file_target(project, root, &path) else {
            continue;
        };
        let mut plans = reach.plans.lock();
        let Ok(binding) = plans.binding(&target) else {
            continue;
        };
        if binding.agent.map(|agent| agent.to_string()).as_deref() != Some(facts.key.as_str()) {
            continue;
        }
        let Ok((_, annotations, _)) = plans.annotation_list(&target) else {
            continue;
        };
        docs.push(json!({
            "path": path,
            "open": annotations.iter().filter(|a| a.is_open()).count(),
            "total": annotations.len(),
            "auto_send": binding.auto_send,
        }));
    }
    Ok(json!({"documents": docs, "truncated": truncated}))
}

/// The target, if this agent may change it: refused when the document is bound to another agent
/// (`D207`), and every window is told so it can offer to reassign it. An unbound document, or one
/// bound to the caller, passes. Every agent write to an annotated document — `ubiq-doc`,
/// `ubiq-plan` and `ubiq-mission`'s `write_document` — comes through here.
///
/// **Advisory, not a security boundary.** It keeps a cooperating agent off a document the user
/// gave to another; an agent's own file tools can still edit the markdown underneath.
pub(crate) fn owned(
    target: Target,
    facts: &AgentFacts,
    reach: &PlanReach,
) -> Result<Target, String> {
    // A refusal reading the binding (no such task, say) is the call's own to report.
    let binding = reach.plans.lock().binding(&target).unwrap_or_default();
    let Some(owner) = binding.agent else {
        return Ok(target);
    };
    if owner.to_string() == facts.key {
        return Ok(target);
    }
    if let Ok(requester) = facts.key.parse() {
        reach.everyone.send(Message::DocOwnershipConflict {
            doc: target.handle(),
            requester,
            requester_name: facts.name.clone(),
            owner,
        });
    }
    // Named the way the interface names it; an owner not running now has only its id to go by.
    let owner_name = reach
        .agents
        .facts(&owner.to_string())
        .map(|facts| facts.name)
        .unwrap_or_else(|| format!("agent {owner}"));
    Err(format!(
        "this {} is bound to another agent, {owner_name}, so nothing was changed. Do not work \
         around this: ask the user to reassign the {} to you — they have been shown your request.",
        noun(&target),
        noun(&target),
    ))
}

/// Every markdown file under `root` with an annotation sidecar beside it, project-relative and
/// sorted, and whether the walk stopped at [`MAX_ANNOTATED_DOCS`].
fn annotated_paths(root: &Path) -> (Vec<String>, bool) {
    let mut paths: Vec<String> = Vec::new();
    let mut truncated = false;
    for entry in ignore::WalkBuilder::new(root).build().flatten() {
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        let Some(body) = name.strip_suffix(".annotation.json") else {
            continue;
        };
        if !body.to_ascii_lowercase().ends_with(".md") {
            continue;
        }
        let Ok(rel) = entry.path().with_file_name(body).strip_prefix(root).map(Path::to_path_buf)
        else {
            continue;
        };
        if paths.len() == MAX_ANNOTATED_DOCS {
            truncated = true;
            break;
        }
        paths.push(rel.to_string_lossy().replace('\\', "/"));
    }
    paths.sort();
    (paths, truncated)
}

/// The body, and — for the merge `write` does — a note of exactly what this agent was shown.
fn read(target: &Target, facts: &AgentFacts, reach: &PlanReach) -> Result<Value, String> {
    let mut plans = reach.plans.lock();
    let replies = plans.load(target);
    for reply in &replies {
        if let Message::Plan { body, .. } = reply.message() {
            plans.note_agent_read(target, &facts.key, body);
        }
    }
    // The blocks' lineage codes (`D208`), for a document that is annotated already — asking an
    // unannotated one would index it and write its sidecar as a side effect of a read.
    let blocks = if plans.is_annotated(target) {
        plans.annotation_list(target).ok().map(|(blocks, _, _)| blocks)
    } else {
        None
    };
    drop(plans);
    let mut answer = body_result(target, &replies)?;
    if let Some(blocks) = blocks
        && let Some(object) = answer.as_object_mut()
    {
        object.insert(
            "blocks".to_string(),
            json!(
                blocks
                    .iter()
                    .map(|block| json!({
                        "code": block.lineage,
                        "id": block.id.to_string(),
                        "kind": block.kind,
                        "starts": block.text.chars().take(60).collect::<String>(),
                    }))
                    .collect::<Vec<_>>()
            ),
        );
    }
    Ok(answer)
}

/// Replace the document's body, stamped as **this agent's** save.
///
/// [`crate::plan::Saver::agent`] is handed the agent's own key, which is what later lets its `plan_changes`
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
/// the plan first is **merged, not refused** ([`crate::plan::Plans::agent_save`], `D208`): the
/// host remembers what `read` showed it, and a write over a body that moved since is a three-way
/// merge in which the user's newer text wins and the agent's contested passages become threads.
/// The revision check is left for a write with nothing on record to merge from.
fn write(
    arguments: &Value,
    facts: &AgentFacts,
    target: &Target,
    reach: &PlanReach,
) -> Result<Value, String> {
    let body = required_str(arguments, "body")?;
    let expected = optional_revision(arguments, "expected_revision")?;
    let saved = reach.plans.agent_save(target, body, &facts.key, expected);
    let threads = saved.threads;
    broadcast(&saved.replies, reach);
    let mut answer = body_result(target, &saved.replies)?;
    // What stands is not what was sent: the agent is told so, and told to read before it writes
    // again, rather than left to believe its own text is the document.
    if saved.merged
        && let Some(object) = answer.as_object_mut()
    {
        object.insert("merged".to_string(), json!(true));
        object.insert(
            "note".to_string(),
            json!(
                "Your write was merged with newer edits by the user: the body above is what now \
                 stands, not what you sent. Read the document again (or continue from this body) \
                 before your next write."
            ),
        );
    }
    // The merge said where the agent's text lost to the user's: the agent is told, by thread.
    if !threads.is_empty()
        && let Some(object) = answer.as_object_mut()
    {
        object.insert(
            "conflicts".to_string(),
            json!({
                "threads": threads.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                "note": "Part of your write overlapped newer edits by the user. Their text was \
                         kept; each of your overlapping passages was posted as a thread on its \
                         block instead. The body above is what now stands — work from it.",
            }),
        );
    }
    Ok(answer)
}

/// Where the document has been edited since a revision, and by how much.
///
/// **The default watermark is this agent's own last write**, which is the question an agent
/// actually has: *what has a human done to my document since I wrote it?* An agent that has never
/// written it has no watermark to default to, so it gets everything that is known — which for a
/// document written before the provenance layer existed is nothing, and that is the honest answer
/// rather than a fabricated one.
fn changes(
    arguments: &Value,
    facts: &AgentFacts,
    target: &Target,
    reach: &PlanReach,
) -> Result<Value, String> {
    let asked = optional_revision(arguments, "since_revision")?;
    let mut plans = reach.plans.lock();
    let since = match asked {
        Some(since) => Some(since),
        None => plans.last_written_by(target, &facts.key),
    };
    let report = plans.change_report(target, since)?;
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

    Ok(with_ident(
        target,
        json!({
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
        }),
    ))
}

/// Every annotation on the document, each carrying the text of the block it names — the agent is
/// never handed a bare [`ubiq_proto::ids::BlockId`] and left to guess what it is about. Orphaned
/// annotations (`_docs/wip/planning-system.md`, decision 4) come back with `"block_text": null`
/// and `"orphaned": true`, so an agent does not try to answer a comment whose passage is gone.
///
/// Open by default — `include_resolved: true` asks for the closed ones too, since "list the
/// open annotations" is the ordinary loop and a resolved thread is rarely what an agent came for.
fn list_annotations(
    arguments: &Value,
    target: &Target,
    reach: &PlanReach,
) -> Result<Value, String> {
    let include_resolved = matches!(arguments.get("include_resolved"), Some(Value::Bool(true)));
    let (blocks, annotations, _) = reach.plans.lock().annotation_list(target)?;
    let mark = match arguments.get("mark") {
        None | Some(Value::Null) => None,
        Some(value) => Some(parse_mark(value)?),
    };
    let annotations: Vec<Value> = annotations
        .iter()
        .filter(|annotation| include_resolved || annotation.is_open())
        .filter(|annotation| mark.is_none_or(|mark| annotation.marks.contains(&mark)))
        .map(|annotation| annotation_json(annotation, &blocks))
        .collect();
    Ok(with_ident(target, json!({"annotations": annotations})))
}

fn parse_mark(value: &Value) -> Result<AnnotationMark, String> {
    serde_json::from_value(value.clone())
        .map_err(|_| "a mark is one of \"agent\", \"todo\", \"question\"".to_string())
}

/// Open a thread on a block of the document, as this agent. The block is `block_id` when given,
/// else the one block containing `quote`; neither is refused, and so is a quote that is ambiguous.
fn annotate(arguments: &Value, target: &Target, reach: &PlanReach) -> Result<Value, String> {
    let text = required_str(arguments, "text")?;
    let quote = match arguments.get("quote") {
        None | Some(Value::Null) => None,
        Some(Value::String(quote)) => Some(quote.as_str()),
        Some(_) => return Err("quote must be a string".to_string()),
    };
    let marks = match arguments.get("marks") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().map(parse_mark).collect::<Result<_, _>>()?,
        Some(_) => return Err("marks must be an array".to_string()),
    };
    let replies = {
        let mut plans = reach.plans.lock();
        let block = match arguments.get("block_id") {
            None | Some(Value::Null) => match quote {
                Some(quote) => plans.block_for_quote(target, quote)?,
                None => return Err("give a quote or a block_id to say where".to_string()),
            },
            Some(_) => required_str(arguments, "block_id")?
                .parse()
                .map_err(|_| "not a block id".to_string())?,
        };
        plans.annotate(
            target,
            block,
            quote.map(str::to_string),
            CommentAuthor::Agent,
            text.to_string(),
            marks,
            None,
        )
    };
    broadcast(&replies, reach);
    for reply in &replies {
        match reply.message() {
            Message::PlanAnnotations {
                blocks,
                annotations,
                ..
            } => {
                let newest = annotations
                    .iter()
                    .max_by_key(|annotation| annotation.created_at)
                    .ok_or_else(|| "the annotation was not found after the change".to_string())?;
                return Ok(annotation_json(newest, blocks));
            }
            Message::PlanError { error, .. } => return Err(error.clone()),
            _ => {}
        }
    }
    Err("the annotation was not created".to_string())
}

fn reply_annotation(
    arguments: &Value,
    target: &Target,
    reach: &PlanReach,
) -> Result<Value, String> {
    let annotation = annotation_id(arguments)?;
    let text = required_str(arguments, "text")?;
    let replies = {
        let mut plans = reach.plans.lock();
        plans.reply_to(
            target,
            annotation,
            CommentAuthor::Agent,
            text.to_string(),
            None,
        )
    };
    annotation_result(annotation, &replies, reach)
}

/// **Propose** an annotation resolved, or reopen one — `resolved` defaults to `true`, so calling
/// this with only `annotation_id` is the ordinary "I answered it" case. **An agent never closes a
/// thread** (`D208`): the thread stays open with the `review` mark and the optional `note` as the
/// agent's comment, and only the user's Accept resolves it. No author check on either.
fn resolve_annotation(
    arguments: &Value,
    target: &Target,
    reach: &PlanReach,
) -> Result<Value, String> {
    let annotation = annotation_id(arguments)?;
    let resolved = match arguments.get("resolved") {
        None => true,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err("resolved must be a boolean".to_string()),
    };
    let replies = {
        let mut plans = reach.plans.lock();
        if resolved {
            let note = arguments
                .get("note")
                .and_then(Value::as_str)
                .map(str::to_string);
            plans.propose_resolution(target, annotation, note)
        } else {
            plans.resolve(target, annotation, false)
        }
    };
    annotation_result(annotation, &replies, reach)
}

/// Every `Everyone` reply, to every window — what keeps a window holding the document in step
/// with what the agent just did to it.
fn broadcast(replies: &[Reply], reach: &PlanReach) {
    for reply in replies {
        if let Reply::Everyone(message) = reply {
            reach.everyone.send(message.clone());
        }
    }
}

/// The `PlanAnnotations` reply, broadcast to every window and turned into just the one annotation
/// the caller asked about — an agent that replied or resolved one does not need the whole list
/// back, and does not need to re-derive which entry in it is theirs.
fn annotation_result(
    annotation: AnnotationId,
    replies: &[Reply],
    reach: &PlanReach,
) -> Result<Value, String> {
    broadcast(replies, reach);
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
        "marks": annotation.marks,
        // Proposed resolved by an agent and waiting on the user's Accept or Reopen (`D208`).
        "review": annotation.review,
        "block_kind": block.map(|block| block.kind.as_str()),
        // The block's lineage code (`D208`): where it came from, `AAB.AA` being the first part of
        // `AAB` split. Informative only — anchor by `block_id` or a quote.
        "block_code": block.map(|block| block.lineage.as_str()),
        "block_text": block.map(|block| block.text.as_str()),
        "thread": annotation.thread.iter().map(|comment| json!({
            "id": comment.id.to_string(),
            "author": comment.author.label(),
            "text": comment.text,
            "to": comment.to,
            "created_at": comment.created_at,
            "edited_at": comment.edited_at,
        })).collect::<Vec<_>>(),
        "created_at": annotation.created_at,
    })
}

/// The `Plan` reply as JSON, or the `PlanError` it carries instead.
fn body_result(target: &Target, replies: &[Reply]) -> Result<Value, String> {
    for reply in replies {
        match reply.message() {
            // `revision` rides back so the agent can hold a watermark and later ask what changed
            // while it was away, without a second call to find out where it stood.
            Message::Plan { body, revision, .. } => {
                return Ok(with_ident(
                    target,
                    json!({"body": body, "revision": revision}),
                ));
            }
            // A refused save is a sentence here rather than a structured refusal: an agent has no
            // second press to make, and what it has to do — re-read the document, redo the edit
            // against what is there now, and write again naming that revision — is a sentence's
            // worth of instruction. The revision it needs is in it.
            Message::PlanConflict { revision, .. } => {
                let noun = noun(target);
                return Err(format!(
                    "the {noun} has moved on and now stands at revision {revision}: nothing was \
                     written. Read it again, redo your edit against what is there now, and write \
                     with expected_revision {revision}."
                ));
            }
            Message::PlanError { error, .. } => return Err(error.clone()),
            _ => {}
        }
    }
    Err(format!("the {} was not answered", noun(target)))
}

fn optional_revision(arguments: &Value, key: &str) -> Result<Option<u64>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{key} must be a whole number, zero or more")),
        Some(_) => Err(format!("{key} must be a number")),
    }
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
