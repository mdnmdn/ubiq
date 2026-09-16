//! The `ubiq-kb` server: the project's knowledge base, as tools a hosted agent can call.
//!
//! An agent Ubiq started is given a folder to work in, and the project's documents are usually not
//! under it — a knowledge-base source is a folder somewhere else on the machine, a repository the
//! host cloned into its own config root, or a wiki that exists only inside Ubiq. None of those are
//! reachable by the agent's ordinary file tools, and none of them have a path the agent could be
//! told without Ubiq handing out absolute paths it deliberately keeps to itself. These tools are
//! the way in.
//!
//! **One string addresses every document: `<source>/path/to/file`.** `<source>` is the source's
//! **name**, as the user typed it in the settings form, because a name is the thing a model can
//! read in one listing and type back in the next call — a ULID is 26 characters of noise that a
//! model will transpose, and it is not what the person and the agent would use when talking about
//! the same document. The id still works: a `<source>` that matches no name is retried as a
//! [`KbSourceId`], so an address copied out of a message or a log resolves too. A bare `<source>`
//! with no slash is that source's own top level, the same meaning an empty `rel_path` carries on
//! the wire. Two sources sharing a name is a real configuration — nothing stops the user — so it
//! is answered as an in-band error naming both candidates by id rather than by picking one.
//!
//! [`resolve`] is the single place that parse and that lookup happen, so every tool below agrees
//! about what an address means by construction rather than by seven implementations staying in
//! step.
//!
//! **Every mutation goes through [`crate::kb::ops`]**, the same free functions the coordinator
//! calls for `WriteKbFile` and its siblings. Writability and containment are checked there and
//! nowhere here: a second check would be a second thing to get wrong, and the interesting failure
//! — a write against a read-only source — is precisely the one `ops` already refuses. Its
//! [`FileError`] becomes the sentence the model reads.
//!
//! **Facts, never material.** A git source's URL and branch are facts about where a source came
//! from and are answered; nothing that would authenticate to it exists in this family to answer
//! with, on [`super::registry`]'s own rule.
//!
//! After any mutation, [`Message::KbChanged`] naming the changed entry's parent goes to every
//! window, exactly as [`super::tasks`] posts its work-family messages — an open knowledge-base
//! panel re-lists that folder without the coordinator being asked a question.

use std::path::PathBuf;

use serde_json::{Value, json};
use ubiq_proto::files::{EntryKind, FileError};
use ubiq_proto::ids::{KbSourceId, ProjectId};
use ubiq_proto::kb::{KbOrigin, KbSource, KbSourceState, KbSourceStatus, KbStore};
use ubiq_proto::messages::Message;

use super::KbReach;
use super::registry::AgentFacts;
use crate::files;
use crate::kb::ops;

/// How deep a single `list_kb_documents` call walks. One level: the tool answers a folder's own
/// entries, and a model that wants what is below asks again with that folder's address.
const LIST_DEPTH: u8 = 1;

pub fn call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    reach: &KbReach,
) -> Result<Value, String> {
    match tool {
        "list_kb_sources" => list_sources(facts, reach),
        "list_kb_documents" => list_documents(arguments, facts, reach),
        "read_kb_document" => read_document(arguments, facts, reach),
        "write_kb_document" => write_document(arguments, facts, reach),
        "create_kb_entry" => create_entry(arguments, facts, reach),
        "rename_kb_entry" => rename_entry(arguments, facts, reach),
        "delete_kb_entry" => delete_entry(arguments, facts, reach),
        "sync_kb_source" => sync_source(arguments, facts, reach),
        _ => Err(format!(
            "unknown tool: {}/{tool}",
            super::catalogue::UBIQ_KB
        )),
    }
}

// ── the tools ───────────────────────────────────────────────────────

/// Every source of this agent's project, with what a model needs in order to address the rest of
/// the tools: the name it types, the id that name falls back to, and whether a write will be
/// refused before it tries one.
fn list_sources(facts: &AgentFacts, reach: &KbReach) -> Result<Value, String> {
    let (project, project_path) = project_of(facts)?;
    let statuses = reach.kb.sources(project, &project_path);
    Ok(json!({
        "project_id": project.to_string(),
        "sources": statuses.iter().map(source_json).collect::<Vec<_>>(),
    }))
}

/// One folder's entries, each already addressed so the answer can be fed straight back into any
/// other tool.
///
/// The source's filter is honoured, because it is what the source *shows* — a document the panel
/// hides is not one the agent should be told about either. Folders are never filtered out, on
/// [`ubiq_proto::kb::KbSource::admits`]'s own rule.
fn list_documents(arguments: &Value, facts: &AgentFacts, reach: &KbReach) -> Result<Value, String> {
    let at = resolve(required_str(arguments, "path")?, facts, reach)?;
    let listings =
        files::listing(&at.base, &at.rel_path, LIST_DEPTH).map_err(|error| at.say(error))?;
    let listing = listings
        .first()
        .ok_or_else(|| format!("{} is not a folder", at.address()))?;

    let entries = listing
        .entries
        .iter()
        .filter(|entry| entry.kind != EntryKind::File || at.source.admits(&entry.name))
        .map(|entry| {
            json!({
                "name": entry.name,
                "is_folder": entry.kind == EntryKind::Dir,
                "path": at.address_of(&entry.rel_path),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "path": at.address(),
        "source": at.source.name,
        "truncated": listing.truncated,
        "entries": entries,
    }))
}

/// The document's text.
///
/// [`files::contents`] is the function the host already answers `ReadKbFile` with — `files`'
/// `kb_answer` calls exactly this, with exactly this base path — so the two cannot read a
/// document differently. Its answer is bytes plus a length, because the file family is opaque
/// about what it read; a model can only be given text, so a binary or invalid-UTF-8 document is
/// refused by name rather than handed back as replacement characters.
fn read_document(arguments: &Value, facts: &AgentFacts, reach: &KbReach) -> Result<Value, String> {
    let at = resolve(required_str(arguments, "path")?, facts, reach)?;
    let contents = files::contents(&at.base, &at.rel_path, None).map_err(|error| at.say(error))?;
    if contents.is_binary {
        return Err(format!(
            "{} is not a text document, so there is nothing to read as text",
            at.address()
        ));
    }
    let text = String::from_utf8(contents.bytes)
        .map_err(|_| format!("{} is not valid UTF-8 text", at.address()))?;

    Ok(json!({
        "path": at.address(),
        "contents": text,
        "len": contents.len,
        "truncated": contents.truncated,
    }))
}

/// Write a document whole. A read-only source is refused by [`ops::write_file`], not here.
fn write_document(arguments: &Value, facts: &AgentFacts, reach: &KbReach) -> Result<Value, String> {
    let at = resolve(required_str(arguments, "path")?, facts, reach)?;
    let contents = required_str(arguments, "contents")?;
    ops::write_file(&at.base, &at.source, &at.rel_path, contents).map_err(|error| at.say(error))?;
    at.changed(reach, &at.rel_path);
    Ok(json!({"written": true, "path": at.address(), "bytes": contents.len()}))
}

fn create_entry(arguments: &Value, facts: &AgentFacts, reach: &KbReach) -> Result<Value, String> {
    let at = resolve(required_str(arguments, "path")?, facts, reach)?;
    let kind = match required_str(arguments, "kind")?
        .trim()
        .to_lowercase()
        .as_str()
    {
        "file" => EntryKind::File,
        "folder" | "dir" | "directory" => EntryKind::Dir,
        other => return Err(format!("unknown kind '{other}': use file or folder")),
    };
    ops::create(&at.base, &at.source, &at.rel_path, kind).map_err(|error| at.say(error))?;
    at.changed(reach, &at.rel_path);
    Ok(json!({
        "created": true,
        "path": at.address(),
        "kind": if kind == EntryKind::Dir { "folder" } else { "file" },
    }))
}

/// Rename one entry in place. `new_name` is a name and never a path — [`ops::rename`] is what
/// refuses a separator in it, and it answers the new relative path, which is turned back into an
/// address so the model can carry on addressing the entry it just moved.
fn rename_entry(arguments: &Value, facts: &AgentFacts, reach: &KbReach) -> Result<Value, String> {
    let at = resolve(required_str(arguments, "path")?, facts, reach)?;
    let new_name = required_str(arguments, "new_name")?;
    let new_rel =
        ops::rename(&at.base, &at.source, &at.rel_path, new_name).map_err(|error| at.say(error))?;
    at.changed(reach, &at.rel_path);
    Ok(json!({
        "renamed": true,
        "from": at.address(),
        "path": at.address_of(&new_rel),
    }))
}

/// Delete one entry. A folder goes with everything under it, on [`ops::delete`]'s own rule.
fn delete_entry(arguments: &Value, facts: &AgentFacts, reach: &KbReach) -> Result<Value, String> {
    let at = resolve(required_str(arguments, "path")?, facts, reach)?;
    ops::delete(&at.base, &at.source, &at.rel_path).map_err(|error| at.say(error))?;
    at.changed(reach, &at.rel_path);
    Ok(json!({"deleted": true, "path": at.address()}))
}

/// Fetch or refresh a git source.
///
/// The fetch runs on a thread of its own and reports its progress to every window, so this answers
/// the moment it is started rather than when it finishes — a model that wants to know whether it
/// landed calls `list_kb_sources` again and reads the state. A folder or a wiki has nothing to
/// fetch: that is said as an ordinary answer with `started` false, not as an error, because
/// nothing went wrong and nothing needs correcting.
fn sync_source(arguments: &Value, facts: &AgentFacts, reach: &KbReach) -> Result<Value, String> {
    let at = resolve(required_str(arguments, "source")?, facts, reach)?;
    let fetched = at.source.origin.is_fetched();
    if fetched {
        reach.kb.sync(
            at.project,
            at.source.id,
            reach.everyone.clone(),
            &at.project_path,
        );
    }
    Ok(json!({
        "source": at.source.name,
        "kind": kind_of(&at.source.origin),
        "started": fetched,
        "detail": if fetched {
            "the fetch has started; call list_kb_sources to see how it ended"
        } else {
            "this source has nothing to fetch: it is read where it lies"
        },
    }))
}

// ── addressing ──────────────────────────────────────────────────────

/// One resolved address: which source it named, where that source lives, and the path inside it.
struct Address {
    project: ProjectId,
    project_path: PathBuf,
    source: KbSource,
    /// [`crate::kb::Kb::base_path`]'s answer for this source, resolved once. Never answered to a
    /// model: an absolute path is exactly what these tools exist to avoid handing out.
    base: PathBuf,
    /// Forward-slashed and without a leading separator, the wire's own convention. Empty is the
    /// source's own top level.
    rel_path: String,
}

impl Address {
    /// The address as the model typed it, rebuilt from what it resolved to — so an answer always
    /// names the source the way `list_kb_sources` does, whether the call used a name or an id.
    fn address(&self) -> String {
        self.address_of(&self.rel_path)
    }

    /// Another path in the same source, addressed.
    fn address_of(&self, rel_path: &str) -> String {
        if rel_path.is_empty() {
            self.source.name.clone()
        } else {
            format!("{}/{rel_path}", self.source.name)
        }
    }

    /// A [`FileError`] as the sentence a model reads, with the address it was about — the error
    /// itself names no source, and a model calling seven tools needs to know which one failed.
    fn say(&self, error: FileError) -> String {
        format!("{}: {error}", self.address())
    }

    /// Tell every window the folder holding `rel_path` changed, naming its parent — the directory
    /// an open panel has to re-list, on [`Message::KbChanged`]'s own reasoning.
    fn changed(&self, reach: &KbReach, rel_path: &str) {
        reach.everyone.send(Message::KbChanged {
            project_id: self.project,
            source: self.source.id,
            rel_path: ops::parent_of(rel_path),
        });
    }
}

/// Parse `<source>/path/to/file` and find the source it names.
///
/// The one place both halves happen. A name is tried first and an id second, so a source
/// deliberately named after another's ULID — which nothing forbids — still reaches the source the
/// user actually named. Every refusal is a sentence saying what went wrong; the ambiguous case
/// lists the candidates by id, since that is the only thing that tells them apart.
fn resolve(address: &str, facts: &AgentFacts, reach: &KbReach) -> Result<Address, String> {
    let (project, project_path) = project_of(facts)?;
    let trimmed = address.trim().trim_start_matches('/');
    let (name, rest) = match trimmed.split_once('/') {
        Some((name, rest)) => (name, rest),
        None => (trimmed, ""),
    };
    if name.is_empty() {
        return Err(
            "a knowledge-base path starts with a source, as in 'Wiki/notes/plan.md'".to_string(),
        );
    }

    let statuses = reach.kb.sources(project, &project_path);
    let source = pick(name, &statuses)?;
    let rel_path = rest.trim_matches('/').to_string();
    let base = reach.kb.base_path(project, &source, &project_path);

    Ok(Address {
        project,
        project_path,
        source,
        base,
        rel_path,
    })
}

/// Which source a `<source>` segment names: by name, ignoring case, and failing that by id.
fn pick(name: &str, statuses: &[KbSourceStatus]) -> Result<KbSource, String> {
    let by_name: Vec<&KbSourceStatus> = statuses
        .iter()
        .filter(|status| status.source.name.eq_ignore_ascii_case(name))
        .collect();
    match by_name.as_slice() {
        [only] => return Ok(only.source.clone()),
        [] => {}
        many => {
            let candidates = many
                .iter()
                .map(|status| status.source.id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "'{name}' names {} knowledge-base sources of this project; address it by id instead: {candidates}",
                many.len()
            ));
        }
    }

    if let Ok(id) = name.parse::<KbSourceId>()
        && let Some(status) = statuses.iter().find(|status| status.source.id == id)
    {
        return Ok(status.source.clone());
    }

    let known = statuses
        .iter()
        .map(|status| status.source.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if known.is_empty() {
        Err(format!(
            "'{name}' is not a knowledge-base source: this project has none configured"
        ))
    } else {
        Err(format!(
            "'{name}' is not a knowledge-base source of this project; it has: {known}"
        ))
    }
}

/// The project this agent was started in, and its folder — the knowledge base it reaches.
///
/// A project id that will not parse is a launch that recorded nothing usable, which is an answer a
/// model can act on (it is in the wrong place) rather than something to panic the listener over.
fn project_of(facts: &AgentFacts) -> Result<(ProjectId, PathBuf), String> {
    let project: ProjectId = facts
        .project
        .id
        .parse()
        .map_err(|_| "this agent has no project, so it has no knowledge base".to_string())?;
    Ok((project, PathBuf::from(&facts.project.path)))
}

// ── the answers ─────────────────────────────────────────────────────

fn source_json(status: &KbSourceStatus) -> Value {
    let source = &status.source;
    let mut value = json!({
        "name": source.name,
        "id": source.id.to_string(),
        "kind": kind_of(&source.origin),
        "writable": source.is_writable(),
        "filter": source.filter,
        "state": state_of(&status.state),
    });

    // Where it came from. A URL is a fact about the source and is answered; nothing that would
    // authenticate to it exists here to answer with.
    let map = value.as_object_mut().expect("a json object");
    match &source.origin {
        KbOrigin::Folder { path } => {
            map.insert("path".to_string(), json!(path));
        }
        KbOrigin::Git { url, branch, store } => {
            map.insert("url".to_string(), json!(url));
            map.insert("branch".to_string(), json!(branch));
            map.insert("store".to_string(), json!(store_of(*store)));
        }
        KbOrigin::Internal => {}
    }
    value
}

fn kind_of(origin: &KbOrigin) -> &'static str {
    match origin {
        KbOrigin::Folder { .. } => "folder",
        KbOrigin::Git { .. } => "git",
        KbOrigin::Internal => "wiki",
    }
}

fn store_of(store: KbStore) -> &'static str {
    match store {
        KbStore::Cache => "cache",
        KbStore::Internal => "internal",
        KbStore::Project => "project",
    }
}

/// A source's state, with the one sentence that goes with it where there is one — a syncing phase
/// or the error the last attempt left.
fn state_of(state: &KbSourceState) -> Value {
    match state {
        KbSourceState::Pending => json!({"state": "pending"}),
        KbSourceState::Syncing { detail } => json!({"state": "syncing", "detail": detail}),
        KbSourceState::Ready => json!({"state": "ready"}),
        KbSourceState::Failed { error } => json!({"state": "failed", "error": error}),
    }
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    match arguments.get(key) {
        Some(Value::String(value)) => Ok(value.as_str()),
        None | Some(Value::Null) => Err(format!("{key} is required")),
        Some(_) => Err(format!("{key} must be a string")),
    }
}
