//! The `ubiq-archify` server: the Archify diagram engine as tools a hosted agent can call.
//!
//! The agent never edits a document through a tool: it writes the file with its own tools and the
//! host's watcher carries the change. The one write is `archify_new`, which creates a starter
//! `.archify` file. Every path an agent names resolves inside **its own project's folder**
//! (`facts.project.path`) and nowhere else.
//!
//! Every call tells every window that it happened ([`Message::ArchifyToolCall`], the tab's tool chip
//! and the "fixed it" notice), and an `archify_render` of a file that compiled clean, or an
//! `archify_new`, adds [`Message::ArchifyShow`]: open or focus that file in the diagram viewer.
//! Neither carries file bytes: the viewer reads and watches the file itself.
//!
//! A compile can take a second, so this server is served on a thread of its own, like the SQL ones
//! ([`super::server`]).

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};
use ubiq_archify::compile;
use ubiq_archify::diag::{Diagnostic, Receipt, Stage};
use ubiq_archify::guide;
use ubiq_archify::layout_json::{self, LayoutReply};
use ubiq_archify::schema::{self, DOC_TYPES};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;

use super::ArchifyReach;
use super::registry::AgentFacts;

/// Largest document the server reads.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// What a tool call produced.
struct Outcome {
    /// The MCP content text: JSON for `archify_validate`, the schema, or markdown.
    text: String,
    /// The MCP `isError` flag: the call could not be carried out.
    is_error: bool,
    /// The tool did what it was for: not an error and, for validate, the document passed.
    ok: bool,
    /// The file the call named, canonical, so the tab can be told which one it was.
    path: Option<PathBuf>,
    /// The interface should open `path`'s tab (`archify_render` of a file that compiled clean).
    show: bool,
}

impl Outcome {
    fn error(message: impl Into<String>) -> Self {
        Outcome { text: message.into(), is_error: true, ok: false, path: None, show: false }
    }

    fn text(text: impl Into<String>) -> Self {
        Outcome { text: text.into(), is_error: false, ok: true, path: None, show: false }
    }
}

/// Run one tool for `facts`' agent, tell every window, and answer. A receipt that says `ok: false`
/// is an answer, not an error; only a call that could not be carried out is an `Err`.
pub fn call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    reach: &ArchifyReach,
) -> Result<Value, String> {
    let roots = [PathBuf::from(&facts.project.path)];
    let outcome = run(&roots, tool, arguments);
    if let Ok(project_id) = facts.project.id.parse::<ProjectId>() {
        let rel = outcome.path.as_deref().and_then(|file| relative(&roots, file));
        reach.everyone.send(Message::ArchifyToolCall {
            project_id,
            rel: rel.clone(),
            tool: tool.to_string(),
            ok: outcome.ok,
        });
        if outcome.show
            && let Some(rel) = rel
        {
            reach.everyone.send(Message::ArchifyShow { project_id, rel });
        }
    }
    if outcome.is_error { Err(outcome.text) } else { Ok(Value::String(outcome.text)) }
}

/// The `/`-separated path of a canonical `file` relative to the project root it is inside.
fn relative(roots: &[PathBuf], file: &Path) -> Option<String> {
    roots.iter().find_map(|root| {
        let root = std::fs::canonicalize(root).ok()?;
        let rel = file.strip_prefix(&root).ok()?;
        Some(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
    })
}

fn run(roots: &[PathBuf], tool: &str, args: &Value) -> Outcome {
    match tool {
        "archify_schema" => schema_tool(args),
        "archify_validate" => validate_tool(roots, args),
        "archify_layout" => layout_tool(roots, args),
        "archify_render" => render_tool(roots, args),
        "archify_new" => new_tool(roots, args),
        "archify_guide" => guide_tool(args),
        other => Outcome::error(format!(
            "unknown tool {other}; tools: archify_schema, archify_validate, archify_layout, archify_render, archify_new, archify_guide"
        )),
    }
}

/// `name` as a file stem: lowercase ASCII letters and digits, runs of anything else one `-`.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// `archify_new`: write a starter of `type` to `<dir>/<slug>.archify` inside a project root,
/// never over an existing file, and ask the interface to show it.
fn new_tool(roots: &[PathBuf], args: &Value) -> Outcome {
    let doc_type = args.get("type").and_then(Value::as_str).unwrap_or("");
    if !DOC_TYPES.contains(&doc_type) {
        return Outcome::error(format!(
            "unknown type \"{doc_type}\"; use one of {}",
            DOC_TYPES.join(", ")
        ));
    }
    let name = args.get("name").and_then(Value::as_str).unwrap_or("").trim();
    let slug = slug(name);
    if slug.is_empty() {
        return Outcome::error("archify_new needs a name with letters or digits");
    }
    let Some(doc) = guide::starter(doc_type, name, &slug) else {
        return Outcome::error(format!("no starter for {doc_type}"));
    };
    let dir = args.get("dir").and_then(Value::as_str).unwrap_or("");
    let target = match new_target(roots, dir) {
        Ok(target) => target,
        Err(reason) => return Outcome::error(reason),
    };
    let file = target.join(format!("{slug}{}", compile::EXTENSION));
    let text = format!("{}\n", serde_json::to_string_pretty(&doc).unwrap_or_default());
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()));
    match written {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Outcome::error(format!(
                "{} exists already: pick another name or edit that file",
                file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
            ));
        }
        Err(e) => return Outcome::error(format!("cannot write the file: {e}")),
    }
    let Ok(file) = std::fs::canonicalize(&file) else {
        return Outcome::error("the file was written but cannot be resolved");
    };
    let rel = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .find_map(|r| file.strip_prefix(&r).ok().map(|p| p.to_string_lossy().replace('\\', "/")))
        .unwrap_or_default();
    Outcome {
        text: json!({"ok": true, "path": rel, "type": doc_type}).to_string(),
        is_error: false,
        ok: true,
        show: true,
        path: Some(file),
    }
}

/// The canonical folder `dir` names inside a project root, created if need be. A relative `dir`
/// is under the first root; an absolute one must be under some root; `..` is refused.
fn new_target(roots: &[PathBuf], dir: &str) -> Result<PathBuf, String> {
    let Some(first) = roots.first() else {
        return Err("no project is open: the host has not been told of any project root".into());
    };
    let given = Path::new(dir);
    if given.components().any(|c| c == Component::ParentDir) {
        return Err(format!("{dir} is not inside a project root"));
    }
    let (root, wanted) = if given.is_absolute() {
        let root = roots
            .iter()
            .find(|r| given.starts_with(r))
            .ok_or_else(|| format!("{dir} is not inside a project root"))?;
        (root, given.to_path_buf())
    } else {
        (first, first.join(given))
    };
    std::fs::create_dir_all(&wanted).map_err(|e| format!("cannot create {dir}: {e}"))?;
    let canonical = std::fs::canonicalize(&wanted).map_err(|e| e.to_string())?;
    let inside = std::fs::canonicalize(root).is_ok_and(|r| canonical.starts_with(r));
    if inside {
        Ok(canonical)
    } else {
        Err(format!("{dir} is not inside a project root"))
    }
}

fn schema_tool(args: &Value) -> Outcome {
    let name = args.get("type").and_then(Value::as_str).unwrap_or("");
    match schema::source(name) {
        Some(text) => Outcome::text(text),
        None => Outcome::error(format!(
            "unknown type \"{name}\"; use one of {}, common",
            DOC_TYPES.join(", ")
        )),
    }
}

fn guide_tool(args: &Value) -> Outcome {
    let scenario = args.get("scenario").and_then(Value::as_str).filter(|s| !s.trim().is_empty());
    let topic = args.get("topic").and_then(Value::as_str).filter(|s| !s.is_empty());
    if let Some(text) = scenario {
        if topic.is_some() {
            return Outcome::error("give either scenario or topic, not both");
        }
        return Outcome::text(guide::scenario_json(text).to_string());
    }
    match topic {
        None | Some("") => Outcome::text(guide::skill()),
        Some(name) => match guide::topic(name) {
            Some(text) => Outcome::text(text),
            None => Outcome::error(format!(
                "unknown topic \"{name}\"; topics: {}",
                guide::topic_names().join(", ")
            )),
        },
    }
}

/// The document a `path` | `document` call names: its text, what to call it in receipts and, for a
/// path, the canonical file. `Err` is the finished outcome (a refusal or an `input/read` receipt).
fn input_of(
    roots: &[PathBuf],
    args: &Value,
    tool: &str,
) -> Result<(String, String, Option<PathBuf>), Outcome> {
    let type_hint = args.get("type").and_then(Value::as_str);
    let document = args.get("document").or_else(|| args.get("json"));
    let path = args.get("path").and_then(Value::as_str);
    match (path, document) {
        (Some(_), Some(_)) => Err(arguments_receipt(
            "cli/usage",
            "give either path or document, not both",
            type_hint,
        )),
        (None, None) => Err(arguments_receipt(
            "cli/missing-option-value",
            &format!("{tool} needs a path or a document"),
            type_hint,
        )),
        (None, Some(Value::String(s))) => Ok((s.clone(), "<document>".to_owned(), None)),
        (None, Some(value)) => Ok((value.to_string(), "<document>".to_owned(), None)),
        (Some(path), None) => {
            let file = match resolve(roots, path) {
                Ok(file) => file,
                Err(PathError::Outside(reason)) => return Err(Outcome::error(reason)),
                Err(PathError::Missing(reason)) => {
                    return Err(read_failure(path, &reason, type_hint, None));
                }
            };
            match read_text(&file) {
                Ok(text) => Ok((text, path.to_owned(), Some(file))),
                Err(reason) => Err(read_failure(path, &reason, type_hint, Some(file))),
            }
        }
    }
}

fn validate_tool(roots: &[PathBuf], args: &Value) -> Outcome {
    let type_hint = args.get("type").and_then(Value::as_str);
    let quality = args.get("quality").and_then(Value::as_str);
    let (text, input, resolved) = match input_of(roots, args, "archify_validate") {
        Ok(found) => found,
        Err(outcome) => return outcome,
    };
    let evidence = match evidence_of(roots, args, resolved.as_deref(), type_hint) {
        Ok(evidence) => evidence,
        Err(outcome) => return outcome,
    };
    let checked = validate_text_with(&text, &input, type_hint, quality, evidence.as_deref());
    let mut outcome = receipt_outcome(checked.receipt, &checked.stages_run);
    outcome.path = resolved;
    outcome
}

/// `archify_render`: compile the document; a clean one is shown in the interface (the call turns
/// `show` into [`Message::ArchifyShow`]) and summarised: type, profile, counts, viewBox. No
/// HTML. A failure is the ordinary validate receipt, and nothing is shown. An inline `document`
/// has no tab to open, so it is only summarised.
fn render_tool(roots: &[PathBuf], args: &Value) -> Outcome {
    let type_hint = args.get("type").and_then(Value::as_str);
    let quality = args.get("quality").and_then(Value::as_str);
    let (text, input, resolved) = match input_of(roots, args, "archify_render") {
        Ok(found) => found,
        Err(outcome) => return outcome,
    };
    let evidence = match evidence_of(roots, args, resolved.as_deref(), type_hint) {
        Ok(evidence) => evidence,
        Err(outcome) => return outcome,
    };
    let compiled = compile::compile_with(&text, &compile::Opts { input: &input, doc_type: type_hint, quality }, evidence.as_deref());
    if !compiled.ok() {
        let mut outcome = receipt_outcome(compiled.receipt, &compiled.stages_run);
        outcome.path = resolved;
        return outcome;
    }
    let summary = render_summary(&compiled, resolved.is_some());
    Outcome {
        text: summary.to_string(),
        is_error: false,
        ok: true,
        show: resolved.is_some(),
        path: resolved,
    }
}

/// The `archify_render` answer for a document that compiled: what was drawn, in numbers.
fn render_summary(compiled: &compile::Compiled, shown: bool) -> Value {
    use compile::Layout;
    let (nodes, edges, frames, view_box) = match &compiled.layout {
        Some(Layout::Architecture { geometry: g, .. }) => (
            g.placement.components.len(),
            g.connections.len(),
            g.placement.boundaries.len(),
            Some(g.placement.view_box),
        ),
        Some(Layout::Dataflow { geometry: g, .. }) => (g.nodes.len(), g.flows.len(), g.frames.len(), Some(g.view_box)),
        Some(Layout::Sequence { geometry: g, .. }) => (g.participants.len(), g.messages.len(), g.segments.len(), Some(g.view_box)),
        Some(Layout::Lifecycle { geometry: g, .. }) => (g.states.len(), g.transitions.len(), g.bands.len(), Some(g.view_box)),
        // Workflow: nodes, routed edges, lanes (its frames), at the final canvas.
        Some(Layout::Workflow { geometry, .. }) => match geometry.as_ref() {
            compile::WorkflowGeometry::V1(g) => (g.nodes.len(), g.edges.len(), g.lanes.len(), Some(g.view_box)),
            compile::WorkflowGeometry::V2(g) => (g.placement.nodes.len(), g.edges.len(), g.placement.lanes.len(), Some(g.view_box)),
        },
        None => (0, 0, 0, None),
    };
    let mut m = serde_json::Map::new();
    m.insert("ok".into(), true.into());
    m.insert("type".into(), compiled.receipt.doc_type.clone().into());
    m.insert("input".into(), compiled.receipt.input.clone().into());
    m.insert("profile".into(), compiled.profile.into());
    m.insert("counts".into(), json!({"nodes": nodes, "edges": edges, "frames": frames}));
    m.insert("viewBox".into(), view_box.map_or(Value::Null, |vb| json!(vb)));
    let mut diagnostics: Vec<&Diagnostic> = compiled.receipt.diagnostics.iter().collect();
    diagnostics.extend(compiled.warnings.iter());
    m.insert("diagnostics".into(), serde_json::to_value(&diagnostics).unwrap_or(Value::Null));
    m.insert("shown".into(), shown.into());
    Value::Object(m)
}

/// `archify_layout`: the resolved layout of a document, as repair evidence. Architecture is
/// Archify's `--layout-json` report (a rejected layout is returned too, with its diagnostics);
/// dataflow, sequence and lifecycle are a receipt of the same shape
/// ([`layout_json::report`]); workflow is the compiler's own receipt (`fixed-v1` or `readable-v2`,
/// [`layout_json::workflow`]), a rejected layout included.
fn layout_tool(roots: &[PathBuf], args: &Value) -> Outcome {
    let type_hint = args.get("type").and_then(Value::as_str);
    let quality = args.get("quality").and_then(Value::as_str);
    let (text, input, resolved) = match input_of(roots, args, "archify_layout") {
        Ok(found) => found,
        Err(outcome) => return outcome,
    };
    let doc_type = type_hint
        .map(str::to_owned)
        .or_else(|| compile::peek(&text).diagram_type)
        .or_else(|| compile::type_from_name(&input));
    let opts = compile::Opts { input: &input, doc_type: doc_type.as_deref(), quality };
    let evidence = match evidence_of(roots, args, resolved.as_deref(), type_hint) {
        Ok(evidence) => evidence,
        Err(outcome) => return outcome,
    };
    let reply = layout_json::by_type_with(&text, &opts, evidence.as_deref());
    let mut outcome = match (reply, doc_type.as_deref()) {
        (Some(LayoutReply::Layout { json, exit_code }), _) => Outcome {
            text: json.to_string(),
            is_error: false,
            ok: exit_code == 0,
            path: None,
            show: false,
        },
        (Some(LayoutReply::Failed(receipt)), _) => receipt_outcome(*receipt, &[]),
        // No usable type: the validate failure says why (missing or unknown type).
        _ => {
            let checked = validate_text(&text, &input, type_hint, quality);
            receipt_outcome(checked.receipt, &checked.stages_run)
        }
    };
    outcome.path = resolved;
    outcome
}

/// `input/read`: the file is inside a root but cannot be read.
fn read_failure(
    path: &str,
    reason: &str,
    type_hint: Option<&str>,
    file: Option<PathBuf>,
) -> Outcome {
    let receipt = compile::read_failure(type_hint, path, reason);
    let mut outcome = receipt_outcome(receipt, &[]);
    outcome.path = file;
    outcome
}

/// A usage error as a receipt (`stage: arguments`, exit 2 in the CLI).
fn arguments_receipt(code: &str, message: &str, type_hint: Option<&str>) -> Outcome {
    let receipt = Receipt::failure(
        "validate",
        Stage::Arguments,
        type_hint.unwrap_or("unknown"),
        "",
        message,
        vec![Diagnostic::error(code, message)],
    );
    receipt_outcome(receipt, &[])
}

/// The receipt as the tool result, with the two fields that keep a pass honest: the stages that
/// ran, and what is not checked.
fn receipt_outcome(receipt: Receipt, stages_run: &[&str]) -> Outcome {
    let ok = receipt.ok;
    let mut value = serde_json::to_value(&receipt).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        map.insert("stagesRun".into(), json!(stages_run));
        map.insert("notChecked".into(), guide::NOT_CHECKED.into());
    }
    Outcome {
        text: value.to_string(),
        is_error: false,
        ok,
        path: None,
        show: false,
    }
}

fn read_text(file: &Path) -> Result<String, String> {
    use std::io::Read;
    let handle = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    handle
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("the file is larger than {MAX_BYTES} bytes"));
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// The single stage-running call site: [`compile::compile`] (V1-V9, graph part), reduced to the
/// receipt and the stages that ran. The receipt is Archify's (`00` section 3.6); a schema failure
/// is `stage: "render"`, as in the goldens.
fn validate_text(
    text: &str,
    input: &str,
    type_hint: Option<&str>,
    quality: Option<&str>,
) -> Checked {
    validate_text_with(text, input, type_hint, quality, None)
}

/// [`validate_text`] with the repository-evidence verifier of the call's checkout.
fn validate_text_with(
    text: &str,
    input: &str,
    type_hint: Option<&str>,
    quality: Option<&str>,
    evidence: Option<compile::EvidenceCheck<'_>>,
) -> Checked {
    let compiled = compile::compile_with(
        text,
        &compile::Opts {
            input,
            doc_type: type_hint,
            quality,
        },
        evidence,
    );
    Checked {
        receipt: compiled.receipt,
        stages_run: compiled.stages_run,
    }
}

/// The verifier [`evidence_of`] builds: `compile_with`'s [`compile::EvidenceCheck`], owned.
type Evidence = Box<dyn Fn(&ubiq_archify::model::Doc) -> Option<Diagnostic>>;

/// The repository-evidence verifier of a call: the checkout is the call's `repo_root` (a directory
/// inside the project), else the project's root. With no project root there is none, and a document
/// that declares evidence is refused with `root-required`. The host half runs `git`; `compile` only calls the closure.
fn evidence_of(
    roots: &[PathBuf],
    args: &Value,
    file: Option<&Path>,
    type_hint: Option<&str>,
) -> Result<Option<Evidence>, Outcome> {
    let canonical: Vec<PathBuf> = roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect();
    let root = match args.get("repo_root").and_then(Value::as_str) {
        Some(given) => {
            let given_path = Path::new(given);
            let candidates: Vec<PathBuf> = if given_path.is_absolute() {
                vec![given_path.to_path_buf()]
            } else {
                roots.iter().map(|r| r.join(given_path)).collect()
            };
            let found = candidates.iter().filter_map(|c| std::fs::canonicalize(c).ok()).find(|real| {
                real.is_dir() && canonical.iter().any(|r| real.starts_with(r))
            });
            match found {
                Some(real) => Some(real),
                None => {
                    return Err(arguments_receipt(
                        "cli/invalid-option-value",
                        "repo_root must be a directory inside a project root",
                        type_hint,
                    ));
                }
            }
        }
        None => file
            .and_then(|f| canonical.iter().find(|r| f.starts_with(r)).cloned())
            .or_else(|| canonical.first().cloned()),
    };
    Ok(root.map(|root| {
        let verify = crate::archify::verifier(root.to_string_lossy().into_owned());
        Box::new(verify) as Evidence
    }))
}

/// What [`validate_text`] produced: the receipt and the stages (`00` section 3.1) that ran.
struct Checked {
    receipt: Receipt,
    stages_run: Vec<&'static str>,
}

/// Why a `path` argument did not resolve.
#[derive(Debug, PartialEq)]
enum PathError {
    /// Not inside any project root (or no project is open): the call is refused.
    Outside(String),
    /// Inside a root, but not a readable file: an `input/read` diagnostic.
    Missing(String),
}

/// `path` as a canonical file inside one of `roots`: a relative path is tried against each root in
/// turn; `..` is refused outright, and a symlink that leaves the root is outside.
fn resolve(roots: &[PathBuf], path: &str) -> Result<PathBuf, PathError> {
    if roots.is_empty() {
        return Err(PathError::Outside(
            "no project is open: the host has not been told of any project root".into(),
        ));
    }
    let given = Path::new(path);
    if given.components().any(|c| c == Component::ParentDir) {
        return Err(PathError::Outside(format!(
            "{path} is not inside a project root"
        )));
    }
    let canonical_roots: Vec<PathBuf> = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .collect();
    let inside = |file: &Path| canonical_roots.iter().any(|r| file.starts_with(r));
    let candidates: Vec<PathBuf> = if given.is_absolute() {
        vec![given.to_path_buf()]
    } else {
        roots.iter().map(|r| r.join(given)).collect()
    };
    let mut missing = None;
    for candidate in &candidates {
        match std::fs::canonicalize(candidate) {
            Ok(file) if inside(&file) => {
                return if file.is_file() {
                    Ok(file)
                } else {
                    Err(PathError::Missing(format!("{path} is not a file")))
                };
            }
            Ok(_) => {
                return Err(PathError::Outside(format!(
                    "{path} is not inside a project root"
                )));
            }
            Err(e) => missing = Some(e.to_string()),
        }
    }
    // Nothing exists. An absolute path that is not even lexically under a root is outside; the
    // rest is a plain missing file.
    if given.is_absolute() && !roots.iter().any(|r| given.starts_with(r)) && !inside(given) {
        return Err(PathError::Outside(format!(
            "{path} is not inside a project root"
        )));
    }
    Err(PathError::Missing(
        missing.unwrap_or_else(|| "no such file".into()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::registry::ProjectFacts;
    use std::time::Duration;
    use ubiq_proto::bus::{self, To};

    const PROJECT: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    const KINDS: [&str; 5] = ["architecture", "workflow", "sequence", "dataflow", "lifecycle"];

    /// A temp project folder, a facts record pointing at it, and a window listening for the
    /// broadcasts. The hub is held for the test's life: a mailbox holds only a weak sender.
    struct Rig {
        dir: tempfile::TempDir,
        facts: AgentFacts,
        reach: ArchifyReach,
        window: bus::Client,
        _hub: bus::Hub,
        _host: bus::HostEnd,
    }

    impl Rig {
        fn new() -> Rig {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("project/diagrams")).unwrap();
            std::fs::write(dir.path().join("secret.json"), "{}").unwrap();
            let path = dir.path().join("project").to_string_lossy().into_owned();
            let (hub, host) = bus::hub();
            let window = hub.connect();
            let facts = AgentFacts {
                key: "01JBTESTAGENTIDULID000000".to_string(),
                name: "claude 1".to_string(),
                harness: "Claude Code".to_string(),
                account: None,
                model: None,
                mode: None,
                cwd: path.clone(),
                session: None,
                mission: None,
                project: ProjectFacts {
                    id: PROJECT.to_string(),
                    name: "P".to_string(),
                    path,
                    colour: 0,
                },
            };
            let reach = ArchifyReach { everyone: host.mailbox(To::Everyone) };
            Rig { dir, facts, reach, window, _hub: hub, _host: host }
        }

        /// `(is_error, text)`.
        fn call(&self, tool: &str, args: Value) -> (bool, String) {
            match call(tool, &args, &self.facts, &self.reach) {
                Ok(Value::String(text)) => (false, text),
                Ok(other) => panic!("not text: {other}"),
                Err(text) => (true, text),
            }
        }

        fn receipt(&self, args: Value) -> Value {
            let (is_error, text) = self.call("archify_validate", args);
            assert!(!is_error, "{text}");
            serde_json::from_str(&text).unwrap()
        }

        fn write(&self, rel: &str, text: &str) {
            std::fs::write(self.dir.path().join("project").join(rel), text).unwrap();
        }

        fn starter(&self, kind: &str) -> Value {
            guide::starter(kind, "Sample", "sample").expect("a starter for every type")
        }

        fn next(&self) -> Option<Message> {
            self.window.from_host().recv_timeout(Duration::from_millis(300)).ok()
        }

        /// The next tool-call broadcast: `(rel, tool, ok)`.
        fn tool_call(&self) -> (Option<String>, String, bool) {
            match self.next() {
                Some(Message::ArchifyToolCall { rel, tool, ok, .. }) => (rel, tool, ok),
                other => panic!("unexpected {other:?}"),
            }
        }

        fn show(&self) -> String {
            match self.next() {
                Some(Message::ArchifyShow { rel, .. }) => rel,
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    fn codes(receipt: &Value) -> Vec<&str> {
        receipt["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["code"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn schema_and_guide_tools() {
        let rig = Rig::new();
        let (is_error, text) = rig.call("archify_schema", json!({"type": "dataflow"}));
        assert!(!is_error);
        assert!(serde_json::from_str::<Value>(&text).unwrap()["$id"].is_string());
        let (is_error, text) = rig.call("archify_schema", json!({"type": "gantt"}));
        assert!(is_error && text.contains("dataflow"), "{text}");
        assert_eq!(rig.tool_call(), (None, "archify_schema".into(), true));
        assert_eq!(rig.tool_call(), (None, "archify_schema".into(), false));

        let (is_error, text) = rig.call("archify_guide", json!({}));
        assert!(!is_error && !text.is_empty());
        let (is_error, text) = rig.call("archify_guide", json!({"topic": "nope"}));
        assert!(is_error && text.contains("authoring-defaults"), "{text}");
        let (is_error, text) = rig.call("archify_guide", json!({"scenario": "order lifecycle", "topic": "repair"}));
        assert!(is_error && text.contains("not both"), "{text}");
        let (is_error, text) = rig.call("archify_guide", json!({"scenario": "order lifecycle from cart to refund"}));
        assert!(!is_error, "{text}");
        assert!(serde_json::from_str::<Value>(&text).is_ok());

        let (is_error, _) = rig.call("archify_nothing", json!({}));
        assert!(is_error);
    }

    #[test]
    fn validate_by_path_and_inline_and_the_failures_before_the_schema() {
        let rig = Rig::new();
        let good = rig.starter("dataflow");
        rig.write("diagrams/good.archify", &good.to_string());

        let receipt = rig.receipt(json!({"path": "diagrams/good.archify"}));
        assert_eq!(receipt["ok"], true, "{receipt}");
        assert_eq!(receipt["type"], "dataflow");
        assert!(receipt["stagesRun"].as_array().unwrap().len() >= 3);
        assert!(receipt["notChecked"].as_str().is_some());
        // The broadcast names the file the call was about, relative to the project: that is the tab.
        assert_eq!(rig.tool_call(), (Some("diagrams/good.archify".into()), "archify_validate".into(), true));
        assert!(rig.next().is_none(), "validate shows nothing");

        // Inline, as an object, as a string; an inline document names no file.
        assert_eq!(rig.receipt(json!({"document": good}))["ok"], true);
        assert_eq!(rig.receipt(json!({"json": good.to_string()}))["ok"], true);
        assert_eq!(rig.tool_call(), (None, "archify_validate".into(), true));
        let _ = rig.tool_call();
        let wrong = rig.receipt(json!({"document": good, "type": "workflow"}));
        assert_eq!(wrong["ok"], false);
        let _ = rig.tool_call();

        let receipt = rig.receipt(json!({"document": "{\"a\": "}));
        assert_eq!((receipt["ok"].clone(), receipt["stage"].clone()), (json!(false), json!("input")));
        assert_eq!(codes(&receipt), ["input/json-parse"]);
        assert_eq!(codes(&rig.receipt(json!({"document": {"a": 1}}))), ["cli/missing-option-value"]);
        assert_eq!(codes(&rig.receipt(json!({"document": good, "type": "gantt"}))), ["cli/unknown-diagram-type"]);
        assert_eq!(codes(&rig.receipt(json!({"document": good, "quality": "gold"}))), ["cli/invalid-option-value"]);
        assert_eq!(codes(&rig.receipt(json!({}))), ["cli/missing-option-value"]);
        assert_eq!(codes(&rig.receipt(json!({"path": "a.json", "document": "{}"}))), ["cli/usage"]);
        // Inside the project but not there: a diagnostic the agent can act on.
        let receipt = rig.receipt(json!({"path": "diagrams/missing.archify"}));
        assert_eq!(receipt["stage"], "input");
        assert_eq!(codes(&receipt), ["input/read"]);
    }

    #[test]
    fn render_summarises_every_type_and_shows_only_a_file_that_compiled() {
        let rig = Rig::new();
        for kind in KINDS {
            let rel = format!("diagrams/{kind}.archify");
            rig.write(&rel, &rig.starter(kind).to_string());
            let (is_error, text) = rig.call("archify_render", json!({"path": rel}));
            assert!(!is_error, "{kind}: {text}");
            let summary: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(summary["ok"], true, "{kind}: {summary}");
            assert_eq!(summary["type"], kind);
            assert_eq!(summary["shown"], true);
            assert!(summary["counts"]["nodes"].as_u64().unwrap() >= 1, "{kind}");
            assert!(summary["viewBox"].is_array(), "{kind}");
            assert!(!text.contains("<svg") && !text.contains("<html"));
            // The chip first, then the tab to open.
            assert_eq!(rig.tool_call(), (Some(rel.clone()), "archify_render".into(), true));
            assert_eq!(rig.show(), rel);
        }

        // Inline: summarised, but there is no tab to show.
        let (_, out) = rig.call("archify_render", json!({"document": rig.starter("dataflow")}));
        let summary: Value = serde_json::from_str(&out).unwrap();
        assert_eq!((summary["ok"].clone(), summary["shown"].clone()), (json!(true), json!(false)));
        assert_eq!(rig.tool_call(), (None, "archify_render".into(), true));
        assert!(rig.next().is_none());

        // A document that does not compile is the validate receipt, and nothing is shown.
        let mut broken = rig.starter("dataflow");
        broken["nodes"][0]["type"] = json!("toaster");
        rig.write("diagrams/bad.archify", &broken.to_string());
        let (is_error, out) = rig.call("archify_render", json!({"path": "diagrams/bad.archify"}));
        assert!(!is_error);
        let receipt: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(receipt["ok"], false);
        assert_eq!(codes(&receipt), ["schema/enum"]);
        assert_eq!(rig.tool_call(), (Some("diagrams/bad.archify".into()), "archify_render".into(), false));
        assert!(rig.next().is_none(), "no Show after a failure");
    }

    #[test]
    fn layout_returns_a_receipt_with_geometry() {
        let rig = Rig::new();
        for kind in KINDS {
            let (is_error, text) = rig.call("archify_layout", json!({"document": rig.starter(kind)}));
            assert!(!is_error, "{kind}: {text}");
            let layout: Value = serde_json::from_str(&text).unwrap();
            assert!(layout.get("viewBox").is_some() || layout.get("contract").is_some(), "{kind}: {text}");
            assert_eq!(rig.tool_call(), (None, "archify_layout".into(), true));
        }
    }

    #[test]
    fn new_writes_a_starter_that_validates_and_never_overwrites() {
        let rig = Rig::new();
        for kind in KINDS {
            let name = format!("My {kind} flow!");
            let (is_error, text) = rig.call("archify_new", json!({"type": kind, "name": name}));
            assert!(!is_error, "{kind}: {text}");
            let rel = format!("my-{kind}-flow.archify");
            assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), json!({"ok": true, "path": rel, "type": kind}));
            assert_eq!(rig.tool_call(), (Some(rel.clone()), "archify_new".into(), true));
            assert_eq!(rig.show(), rel);
            let written: Value =
                serde_json::from_str(&std::fs::read_to_string(rig.dir.path().join("project").join(&rel)).unwrap()).unwrap();
            assert_eq!(written["diagram_type"], kind);
            assert_eq!(written["meta"]["title"], name);
            let receipt = rig.receipt(json!({"path": rel}));
            assert_eq!((receipt["ok"].clone(), receipt["type"].clone()), (json!(true), json!(kind)), "{receipt}");
            let _ = rig.tool_call();
        }

        // A folder, created on the way; and the second call refuses to overwrite.
        let args = json!({"type": "dataflow", "name": "Orders", "dir": "diagrams/deep"});
        let (is_error, text) = rig.call("archify_new", args.clone());
        assert!(!is_error, "{text}");
        assert!(rig.dir.path().join("project/diagrams/deep/orders.archify").is_file());
        let _ = rig.tool_call();
        assert_eq!(rig.show(), "diagrams/deep/orders.archify");
        rig.write("diagrams/deep/orders.archify", "{\"keep\": true}");
        let (is_error, text) = rig.call("archify_new", args);
        assert!(is_error && text.contains("exists already"), "{text}");
        assert_eq!(rig.tool_call(), (None, "archify_new".into(), false));
        assert_eq!(
            std::fs::read_to_string(rig.dir.path().join("project/diagrams/deep/orders.archify")).unwrap(),
            "{\"keep\": true}"
        );

        // Refusals: a bad type, an empty name, a folder outside the project.
        for (args, expect) in [
            (json!({"type": "gantt", "name": "x"}), "unknown type"),
            (json!({"type": "dataflow", "name": " !! "}), "name"),
            (json!({"type": "dataflow", "name": "x", "dir": "../out"}), "not inside a project root"),
            (json!({"type": "dataflow", "name": "x", "dir": "/etc"}), "not inside a project root"),
        ] {
            let (is_error, text) = rig.call("archify_new", args.clone());
            assert!(is_error && text.contains(expect), "{args}: {text}");
            assert_eq!(rig.tool_call(), (None, "archify_new".into(), false));
        }
        assert!(!rig.dir.path().join("out").exists());
        assert!(rig.next().is_none(), "no Show after a refusal");
    }

    #[test]
    fn paths_outside_the_project_are_refused_and_still_announced() {
        let rig = Rig::new();
        let secret = rig.dir.path().join("secret.json");
        for path in ["../secret.json".to_owned(), secret.to_string_lossy().into_owned(), "/etc/passwd".to_owned()] {
            for tool in ["archify_validate", "archify_render", "archify_layout"] {
                let (is_error, text) = rig.call(tool, json!({"path": path}));
                assert!(is_error && text.contains("not inside a project root"), "{tool} {path}: {text}");
                assert_eq!(rig.tool_call(), (None, tool.into(), false));
            }
        }
        assert!(rig.next().is_none());
    }

    #[test]
    fn a_repo_root_outside_the_project_is_refused() {
        let rig = Rig::new();
        let doc = rig.starter("dataflow");
        let receipt = rig.receipt(json!({"document": doc, "repo_root": "/"}));
        assert_eq!(codes(&receipt), ["cli/invalid-option-value"]);
    }

    #[test]
    fn resolve_stays_inside_the_roots() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        std::fs::create_dir_all(dir.join("root/sub")).unwrap();
        std::fs::write(dir.join("root/sub/a.dataflow.json"), "{}").unwrap();
        std::fs::write(dir.join("outside.json"), "{}").unwrap();
        let roots = [dir.join("root")];
        let canonical = std::fs::canonicalize(dir.join("root/sub/a.dataflow.json")).unwrap();

        assert_eq!(resolve(&roots, "sub/a.dataflow.json"), Ok(canonical.clone()));
        assert_eq!(resolve(&roots, dir.join("root/sub/a.dataflow.json").to_str().unwrap()), Ok(canonical));
        assert!(matches!(resolve(&roots, "../outside.json"), Err(PathError::Outside(_))));
        assert!(matches!(resolve(&roots, dir.join("outside.json").to_str().unwrap()), Err(PathError::Outside(_))));
        assert!(matches!(resolve(&roots, "nope.json"), Err(PathError::Missing(_))));
        assert!(matches!(resolve(&[], "a.json"), Err(PathError::Outside(_))));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("outside.json"), dir.join("root/link.json")).unwrap();
            assert!(matches!(resolve(&roots, "link.json"), Err(PathError::Outside(_))));
        }
    }
}
