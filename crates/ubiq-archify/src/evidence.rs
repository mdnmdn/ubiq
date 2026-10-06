//! Repository evidence (V7). Port of `renderers/shared/repository-evidence.mjs` and
//! `repository-location.mjs`: `meta.repository` and the nodes' `sources`, in two halves.
//!
//! **Shape**, pure ([`check_header`], [`check_shape`]): nothing runs `git`, reads a repository or
//! resolves a path on disk. **Verification** ([`verify`], P9.2): the whole of
//! `verifyRepositoryEvidence`, in Archify's order, over a [`Git`] that answers what the checks ask of
//! a checkout. This module is still pure: the process half (`git --no-replace-objects`, realpath,
//! physical identity) is the host's verifier, and `compile` takes the result as an optional
//! verifier ([`crate::compile::compile_with`]).
//!
//! Archify throws the first failure, in this order: `repository-required`, `revision-invalid`,
//! `url-invalid`, `provider-invalid`, `links-unsupported`; then `root-required` without a
//! repository root; then `root-unreadable`, `git-command` (not a repository), `root-not-top-level`
//! (`root-identity-indeterminate`), `git-command` (no origin), `origin-mismatch`,
//! `revision-unavailable`; then per source in document order `path-invalid`, `path-escape`,
//! `line-required`, `line-range-invalid`, `file-missing`, `file-unreadable`, `line-out-of-range`;
//! finally `source-required`. (`link-mode-invalid` is the schema's `enum` and `revision-invalid`
//! its `pattern`; both are kept only as defence, since the typed model cannot carry an unknown link
//! mode.) Without a root, `root-required` therefore comes before any per-source check, which is what
//! [`check_unverified`] does; [`check_shape`] keeps the shape checks together for the P1.4 tests.

// A `Diagnostic` is this module's error: Archify throws the first failure, and that diagnostic is the
// whole answer, so it is returned as it is rather than boxed at every `Err`.
#![allow(clippy::result_large_err)]

use serde_json::{Map, Value, json};

use crate::diag::{Diagnostic, Subject, json_num};
use crate::model::Doc;
use crate::model::common::{LinkMode, RepoProvider, Repository, SourceRef};

/// A node id and its `sources`.
type Authored<'a> = (&'a str, Option<&'a [SourceRef]>);

/// `(collection, nodes)`: where each type authors source evidence.
fn nodes(doc: &Doc) -> (&'static str, Vec<Authored<'_>>) {
    fn of<'a, T: 'a>(items: &'a [T], pick: impl Fn(&'a T) -> Authored<'a>) -> Vec<Authored<'a>> {
        items.iter().map(pick).collect()
    }
    match doc {
        Doc::Architecture(d) => (
            "components",
            of(&d.components, |n| (&n.id, n.sources.as_deref())),
        ),
        Doc::Workflow(d) => ("nodes", of(&d.nodes, |n| (&n.id, n.sources.as_deref()))),
        Doc::Sequence(d) => (
            "participants",
            of(&d.participants, |n| (&n.id, n.sources.as_deref())),
        ),
        Doc::Dataflow(d) => ("nodes", of(&d.nodes, |n| (&n.id, n.sources.as_deref()))),
        Doc::Lifecycle(d) => ("states", of(&d.states, |n| (&n.id, n.sources.as_deref()))),
    }
}

fn repository(doc: &Doc) -> Option<&Repository> {
    match doc {
        Doc::Architecture(d) => d.meta.repository.as_ref(),
        Doc::Workflow(d) => d.meta.repository.as_ref(),
        Doc::Sequence(d) => d.meta.repository.as_ref(),
        Doc::Dataflow(d) => d.meta.repository.as_ref(),
        Doc::Lifecycle(d) => d.meta.repository.as_ref(),
    }
}

/// `hasRepositoryEvidence`: `meta.repository`, or any node with a non-empty `sources`.
pub fn declares_evidence(doc: &Doc) -> bool {
    repository(doc).is_some()
        || nodes(doc)
            .1
            .iter()
            .any(|(_, sources)| sources.is_some_and(|s| !s.is_empty()))
}

/// `repository-evidence/root-required`: what Archify says to a diagram with evidence when it is
/// not given a repository to verify against. Shape must have passed first.
pub fn root_required(doc: &Doc) -> Option<Diagnostic> {
    declares_evidence(doc).then(|| {
        failure(
            "repository-evidence/root-required",
            "This diagram declares source evidence. Pass --repo-root <repository> so Archify can verify it before rendering.",
            Subject::default().with_path("/meta/repository"),
            Map::new(),
            ["pass --repo-root with the matching local Git checkout"],
        )
    })
}

/// V7 with no repository to verify against: the header checks, then `root-required`. At most one
/// diagnostic, empty when the document declares no evidence.
pub fn check_unverified(doc: &Doc) -> Vec<Diagnostic> {
    match header(doc) {
        Err(d) => vec![d],
        Ok(None) => Vec::new(),
        Ok(Some(_)) => root_required(doc).into_iter().collect(),
    }
}

/// The shape checks of V7: at most one diagnostic (the first failure), empty when the document
/// declares no evidence or its evidence is well formed.
pub fn check_shape(doc: &Doc, _raw: &Value) -> Vec<Diagnostic> {
    check_first(doc).into_iter().collect()
}

/// The checks of `meta.repository` alone (everything before the root and the sources).
pub fn check_header(doc: &Doc) -> Option<Diagnostic> {
    header(doc).err()
}

fn check_first(doc: &Doc) -> Option<Diagnostic> {
    if let Err(d) = header(doc) {
        return Some(d);
    }
    if !declares_evidence(doc) {
        return None;
    }
    let (collection, authored) = nodes(doc);
    let diagram_type = doc.diagram_type();
    let mut references = 0;
    for (node_index, (node_id, sources)) in authored.iter().enumerate() {
        for (source_index, source) in sources.unwrap_or_default().iter().enumerate() {
            if let Some(d) = source_shape(diagram_type, collection, node_index, node_id, source_index, source) {
                return Some(d);
            }
            references += 1;
        }
    }
    (references == 0).then(|| source_required(diagram_type, collection))
}

fn source_required(diagram_type: &str, collection: &str) -> Diagnostic {
    failure(
        "repository-evidence/source-required",
        &format!("/meta/repository requires at least one /{collection} source reference."),
        Subject::of(diagram_type)
            .with_path("/meta/repository")
            .with_collection(collection),
        Map::new(),
        [format!(
            "add at least one verified /{collection} source or remove repository metadata"
        )],
    )
}

/// What the header checks learned, for what comes after them.
struct Header<'a> {
    repo: &'a Repository,
    location: Remote,
}

/// `repository-required` .. `links-unsupported`. `Ok(None)` when the document declares no evidence.
fn header(doc: &Doc) -> Result<Option<Header<'_>>, Diagnostic> {
    if !declares_evidence(doc) {
        return Ok(None);
    }
    let (collection, _) = nodes(doc);
    let diagram_type = doc.diagram_type();
    let Some(repo) = repository(doc) else {
        return Err(failure(
            "repository-evidence/repository-required",
            "Repository evidence requires /meta/repository.",
            Subject::of(diagram_type)
                .with_path("/meta/repository")
                .with_collection(collection),
            Map::new(),
            [format!(
                "add the pinned repository metadata or remove /{collection} sources"
            )],
        ));
    };

    if !is_full_sha(&repo.revision) {
        return Err(failure(
            "repository-evidence/revision-invalid",
            "/meta/repository/revision must be a full 40-character commit SHA.",
            Subject::default().with_path("/meta/repository/revision"),
            obj(json!({ "revision": repo.revision })),
            ["pin one full 40-character commit SHA"],
        ));
    }
    let Some(location) = parse_remote(&repo.url, true) else {
        // A filesystem path is the common authoring mistake.
        let looks_like_path = looks_like_filesystem_path(&repo.url);
        return Err(failure(
            "repository-evidence/url-invalid",
            "/meta/repository/url must be a credential-free HTTP(S) or Git SSH repository address without query, fragment, or dot segments.",
            Subject::default().with_path("/meta/repository/url"),
            if looks_like_path {
                obj(
                    json!({ "authoredValueLooksLike": "local filesystem path; the expected value is the remote origin address" }),
                )
            } else {
                Map::new()
            },
            [
                "run `git remote get-url origin` inside --repo-root and declare that credential-free address",
                "use link_mode: local-only for internal repositories",
            ],
        ));
    };
    let link_mode = repo.link_mode.unwrap_or(LinkMode::Web);
    if let Some(provider) = repo.provider {
        let want = match provider {
            RepoProvider::Github => "github",
            RepoProvider::Gitee => "gitee",
        };
        if location.provider != Some(want) {
            return Err(failure(
                "repository-evidence/provider-invalid",
                "Repository provider must match its supported public host (github.com or gitee.com).",
                Subject::default().with_path("/meta/repository/provider"),
                Map::new(),
                ["use the matching provider or omit provider and select link_mode: local-only"],
            ));
        }
    }
    if link_mode == LinkMode::Web
        && (location.provider.is_none()
            || location.protocol != "https:"
            || !location.standard_endpoint
            || !is_owner_repo(&location.path))
    {
        return Err(failure(
            "repository-evidence/links-unsupported",
            "Web source links require a canonical GitHub or Gitee HTTPS owner/repository URL.",
            Subject::default().with_path("/meta/repository/url"),
            Map::new(),
            [
                "use a canonical GitHub or Gitee URL, or select link_mode: local-only to retain local verification without web links",
            ],
        ));
    }
    Ok(Some(Header { repo, location }))
}

/// The subject of a failure about one node's source.
fn node_subject(diagram_type: &str, collection: &str, node_id: &str) -> Subject {
    let mut s = Subject::of(diagram_type).with_collection(collection);
    s = s.with_extra("nodeId", json!(node_id));
    if collection == "components" {
        s = s.with_extra("componentId", json!(node_id));
    }
    s
}

/// The shape of one source, in Archify's order: `path-invalid`, `path-escape`, `line-required`,
/// `line-range-invalid`.
fn source_shape(
    diagram_type: &str,
    collection: &str,
    node_index: usize,
    node_id: &str,
    source_index: usize,
    source: &SourceRef,
) -> Option<Diagnostic> {
    let at = format!("/{collection}/{node_index}/sources/{source_index}");
    if let Some(d) = source_path_failure(&source.path, &format!("{at}/path")) {
        return Some(d);
    }
    let line = source.line.filter(|l| *l != 0.0);
    let end_line = source.end_line.filter(|l| *l != 0.0);
    if let (Some(_), None) = (end_line, line) {
        return Some(failure(
            "repository-evidence/line-required",
            &format!("{at}/end_line requires line."),
            node_subject(diagram_type, collection, node_id).with_path(format!("{at}/end_line")),
            Map::new(),
            ["add line or remove end_line"],
        ));
    }
    if let (Some(line), Some(end)) = (line, end_line)
        && end < line
    {
        return Some(failure(
            "repository-evidence/line-range-invalid",
            &format!("{at}/end_line must be greater than or equal to line."),
            node_subject(diagram_type, collection, node_id).with_path(at.clone()),
            obj(json!({ "line": json_num(line), "endLine": json_num(end) })),
            ["use an end_line greater than or equal to line"],
        ));
    }
    None
}

/// `verifiedSourcePath`: `path-invalid` then `path-escape`.
fn source_path_failure(path: &str, at: &str) -> Option<Diagnostic> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(is_control)
    {
        return Some(failure(
            "repository-evidence/path-invalid",
            &format!("{at} must be a repo-relative POSIX path."),
            Subject::default().with_path(at),
            obj(json!({ "authoredPath": path })),
            ["use a repository-relative path with forward slashes"],
        ));
    }
    let mut segments = path.split('/');
    let first = segments.clone().next().unwrap_or_default();
    if first == ".git" || segments.any(|s| s.is_empty() || s == "." || s == "..") {
        return Some(failure(
            "repository-evidence/path-escape",
            &format!("{at} must stay inside the repository and may not address .git."),
            Subject::default().with_path(at),
            obj(json!({ "authoredPath": path })),
            ["remove empty, dot, parent, or .git path segments"],
        ));
    }
    None
}

fn failure<S: AsRef<str>>(
    code: &str,
    message: &str,
    subject: Subject,
    evidence: Map<String, Value>,
    fixes: impl IntoIterator<Item = S>,
) -> Diagnostic {
    Diagnostic::error(code, message)
        .with_subject(
            subject
                .with_extra("surface", json!("repository-evidence"))
                .with_rule(code),
        )
        .with_evidence(evidence)
        .with_fixes(fixes)
}

fn obj(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

trait WithCollection {
    fn with_collection(self, collection: &str) -> Self;
}

impl WithCollection for Subject {
    fn with_collection(mut self, collection: &str) -> Self {
        self.collection = Some(collection.to_owned());
        self
    }
}

// ---- verification ------------------------------------------------------------------------------

/// The result of one `git` call (`spawnSync`): it ran and exited, or it could not run at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRun {
    Done { status: i32, stdout: String },
    /// `result.error.message`: Git is not installed, or the output outgrew the 16 MB buffer.
    Unavailable(String),
}

/// `sameEntry`: the physical identity of two existing filesystem entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Same,
    Different,
    /// Missing or unidentifiable, with the reason Archify prints (`entry-missing`, ...).
    Unknown(String),
}

/// A repository root that did not resolve: `path.resolve(input)` and the system error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootError {
    pub requested: String,
    pub reason: String,
}

/// What the checks ask of a checkout. the host's verifier answers with the `git` executable; a test
/// can answer with a fake.
pub trait Git {
    /// `path.resolve(input)` then `fs.realpathSync`.
    fn resolve_root(&self, input: &str) -> Result<String, RootError>;
    /// `git --no-replace-objects -C <root> <args>`: a pinned SHA must read the original objects,
    /// never what a local replace ref swaps in.
    fn run(&self, root: &str, args: &[&str]) -> GitRun;
    fn same_entry(&self, left: &str, right: &str) -> Entry;
}

fn git_unavailable(reason: &str) -> Diagnostic {
    failure(
        "repository-evidence/git-unavailable",
        &format!("Could not run Git: {reason}"),
        Subject::default(),
        obj(json!({ "reason": reason })),
        ["install Git and ensure it is available on PATH"],
    )
}

/// `runGit`: the call, or `git-unavailable`.
fn run(git: &dyn Git, root: &str, args: &[&str]) -> Result<(i32, String), Diagnostic> {
    match git.run(root, args) {
        GitRun::Done { status, stdout } => Ok((status, stdout)),
        GitRun::Unavailable(reason) => Err(git_unavailable(&reason)),
    }
}

/// `gitValue`: the trimmed output of a call that must succeed, else `git-command`.
fn git_value(git: &dyn Git, root: &str, args: &[&str], message: &str) -> Result<String, Diagnostic> {
    let (status, stdout) = run(git, root, args)?;
    if status != 0 {
        return Err(failure(
            "repository-evidence/git-command",
            message,
            Subject::default(),
            obj(json!({ "gitArgs": args, "exitCode": status })),
            ["use the intended local Git repository and verify its origin and revision"],
        ));
    }
    Ok(stdout.trim().to_owned())
}

/// `redactRepositoryRemote`: credentials and any query or fragment never reach a diagnostic.
fn redact_remote(value: &str) -> String {
    let mut text = value.to_owned();
    // `^((?:https?|ssh):\/\/)[^/]*@` -> `$1REDACTED@`
    let lower = text.to_ascii_lowercase();
    for scheme in ["https://", "http://", "ssh://"] {
        if lower.starts_with(scheme) {
            let rest = &text[scheme.len()..];
            let authority_end = rest.find('/').unwrap_or(rest.len());
            if let Some(at) = rest[..authority_end].rfind('@') {
                text = format!("{}REDACTED@{}", &text[..scheme.len()], &rest[at + 1..]);
            }
            break;
        }
    }
    // `[?#].*$` (dot matches newlines) -> `?REDACTED`
    if let Some(cut) = text.find(['?', '#']) {
        text.truncate(cut);
        text.push_str("?REDACTED");
    }
    text
}

/// `sourceLineCount`: the lines of a file, a final line break not starting another.
fn source_line_count(content: &str) -> usize {
    if content.is_empty() {
        return 0;
    }
    let bytes = content.as_bytes();
    let (mut breaks, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                breaks += 1;
                i += 2;
            }
            b'\r' | b'\n' => {
                breaks += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    if matches!(bytes[bytes.len() - 1], b'\r' | b'\n') { breaks } else { breaks + 1 }
}

/// `verifyRepositoryEvidence`: the first failure, `None` when the document declares no evidence or
/// every citation holds at the pinned revision. `root` is `--repo-root`.
pub fn verify(doc: &Doc, root: Option<&str>, git: &dyn Git) -> Option<Diagnostic> {
    verify_all(doc, root, git).err()
}

fn verify_all(doc: &Doc, root: Option<&str>, git: &dyn Git) -> Result<(), Diagnostic> {
    let Some(Header { repo, location }) = header(doc)? else { return Ok(()) };
    let Some(input) = root else {
        return Err(root_required(doc).expect("a document with a header declares evidence"));
    };
    let real = git.resolve_root(input).map_err(|e| {
        failure(
            "repository-evidence/root-unreadable",
            &format!("Could not resolve evidence repository root \"{}\": {}", e.requested, e.reason),
            Subject::default().with_extra("repoRoot", json!(e.requested)),
            obj(json!({ "reason": e.reason })),
            ["pass one readable local repository directory"],
        )
    })?;
    let top = git_value(
        git,
        &real,
        &["rev-parse", "--show-toplevel"],
        &format!("Evidence root \"{real}\" is not a Git repository."),
    )?;
    match git.same_entry(&real, &top) {
        Entry::Same => {}
        Entry::Unknown(reason) => {
            return Err(failure(
                "repository-evidence/root-identity-indeterminate",
                "Could not determine whether the evidence root is the Git top-level directory.",
                Subject::default().with_extra("repoRoot", json!(real)),
                obj(json!({ "gitTopLevel": top, "relation": reason })),
                ["pass the readable Git top-level directory using its canonical filesystem path"],
            ));
        }
        Entry::Different => {
            return Err(failure(
                "repository-evidence/root-not-top-level",
                &format!("Evidence root must be the Git top-level directory: {top}"),
                Subject::default().with_extra("repoRoot", json!(real)),
                obj(json!({ "gitTopLevel": top })),
                [format!("pass --repo-root {top}")],
            ));
        }
    }
    let origin = git_value(git, &real, &["remote", "get-url", "origin"], "Evidence repository must have an origin remote.")?;
    if parse_remote(&origin, false).map(|r| r.identity).as_deref() != Some(location.identity.as_str()) {
        let safe = redact_remote(&origin);
        return Err(failure(
            "repository-evidence/origin-mismatch",
            &format!(
                "Evidence repository origin {} does not match {}.",
                serde_json::to_string(&safe).unwrap_or_default(),
                serde_json::to_string(&repo.url).unwrap_or_default()
            ),
            Subject::default().with_extra("repoRoot", json!(real)),
            obj(json!({ "localOrigin": safe, "authoredRepository": repo.url })),
            ["use the matching local checkout or correct the authored repository URL"],
        ));
    }

    let revision = repo.revision.to_lowercase();
    let (status, _) = run(git, &real, &["cat-file", "-e", &format!("{revision}^{{commit}}")])?;
    if status != 0 {
        return Err(failure(
            "repository-evidence/revision-unavailable",
            &format!("Evidence revision {revision} is not available in the local repository."),
            Subject::default().with_extra("repoRoot", json!(real)),
            obj(json!({ "revision": revision })),
            ["fetch the pinned commit or pin an available full commit SHA"],
        ));
    }

    let (collection, authored) = nodes(doc);
    let diagram_type = doc.diagram_type();
    let mut references = 0;
    for (node_index, (node_id, sources)) in authored.iter().enumerate() {
        for (source_index, source) in sources.unwrap_or_default().iter().enumerate() {
            if let Some(d) = source_shape(diagram_type, collection, node_index, node_id, source_index, source) {
                return Err(d);
            }
            let at = format!("/{collection}/{node_index}/sources/{source_index}");
            let where_ = format!("{at}/path");
            let subject = || node_subject(diagram_type, collection, node_id);
            let object = format!("{revision}:{}", source.path);
            let (status, kind) = run(git, &real, &["cat-file", "-t", &object])?;
            if status != 0 || kind.trim() != "blob" {
                return Err(failure(
                    "repository-evidence/file-missing",
                    &format!("{where_} does not identify a file at revision {revision}."),
                    subject().with_path(where_),
                    obj(json!({ "sourcePath": source.path, "revision": revision })),
                    ["use a file path that exists at the pinned revision"],
                ));
            }
            if let Some(line) = source.line.filter(|l| *l != 0.0) {
                let (status, content) = run(git, &real, &["show", &object])?;
                if status != 0 {
                    return Err(failure(
                        "repository-evidence/file-unreadable",
                        &format!("{where_} could not be read at revision {revision}."),
                        subject().with_path(where_),
                        obj(json!({ "sourcePath": source.path, "revision": revision })),
                        ["verify the pinned blob is readable in the local checkout"],
                    ));
                }
                let line_count = source_line_count(&content);
                let requested = source.end_line.filter(|l| *l != 0.0).unwrap_or(line);
                if requested > line_count as f64 {
                    return Err(failure(
                        "repository-evidence/line-out-of-range",
                        &format!(
                            "{at} requests line {}, but {} has {line_count} lines at revision {revision}.",
                            crate::gates::num(requested),
                            source.path
                        ),
                        subject().with_path(at),
                        obj(json!({
                            "sourcePath": source.path, "requestedLine": json_num(requested),
                            "lineCount": line_count, "revision": revision
                        })),
                        ["use a line range that exists at the pinned revision"],
                    ));
                }
            }
            references += 1;
        }
    }
    if references == 0 {
        return Err(source_required(diagram_type, collection));
    }
    Ok(())
}

fn is_control(c: char) -> bool {
    c <= '\u{1f}' || c == '\u{7f}'
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/`.
fn is_owner_repo(path: &str) -> bool {
    let word = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    };
    path.split_once('/')
        .is_some_and(|(owner, repo)| word(owner) && word(repo))
}

/// `/^(?:[\\/]|~|\.{1,2}(?:[\\/]|$)|[A-Za-z]:[\\/])/`.
fn looks_like_filesystem_path(url: &str) -> bool {
    let b = url.as_bytes();
    let slash = |i: usize| matches!(b.get(i), Some(b'/' | b'\\'));
    slash(0)
        || b.first() == Some(&b'~')
        || (b.first() == Some(&b'.')
            && (b.len() == 1
                || slash(1)
                || (b.get(1) == Some(&b'.') && (b.len() == 2 || slash(2)))))
        || (b.first().is_some_and(u8::is_ascii_alphabetic) && b.get(1) == Some(&b':') && slash(2))
}

/// What `parseRepositoryRemote` yields that the checks read.
struct Remote {
    /// `github` / `gitee` for the two public forges.
    provider: Option<&'static str>,
    /// `https:`, `http:` or `ssh:`.
    protocol: &'static str,
    /// The repository path with the forge's `.git` suffix removed.
    path: String,
    /// A forge reached over its default HTTPS or SSH port.
    standard_endpoint: bool,
    /// `JSON.stringify([hostname, endpoint, pathKind, identityPath])`: two addresses name one
    /// repository exactly when their identities are equal.
    identity: String,
}

/// `shared/repository-location.mjs:parseRepositoryRemote`. `authored` is the declared
/// `meta.repository.url` (no surrounding trim, no credentials on HTTP); not authored, it is a
/// local `origin` (trimmed, credentials tolerated).
fn parse_remote(value: &str, authored: bool) -> Option<Remote> {
    let raw = if authored { value } else { value.trim() };
    if raw.is_empty()
        || raw
            .chars()
            .any(|c| c.is_whitespace() || c == '\\' || is_control(c) || c == '?' || c == '#')
    {
        return None;
    }
    // `git@host:path`, but only when `:` comes before any `/` after the `@`.
    let scp = raw.strip_prefix("git@").and_then(|rest| {
        let split = rest.find([':', '/'])?;
        (rest.as_bytes()[split] == b':' && split > 0 && split + 1 < rest.len())
            .then(|| (&rest[..split], &rest[split + 1..]))
    });
    let scp_absolute = scp.is_some_and(|(_, path)| path.starts_with('/'));
    let expanded = match scp {
        Some((host, path)) => format!(
            "ssh://git@{host}/{}",
            path.strip_prefix('/').unwrap_or(path)
        ),
        None => raw.to_owned(),
    };
    let (scheme, after) = expanded.split_once("://")?;
    let protocol = match scheme.to_ascii_lowercase().as_str() {
        "https" => "https:",
        "http" => "http:",
        "ssh" => "ssh:",
        _ => return None,
    };
    let (authority, path) = after.split_once('/')?;
    if authority.is_empty() || path.is_empty() {
        return None;
    }
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    let mut segments: Vec<String> = Vec::new();
    for part in trimmed.split('/') {
        let part = if scp.is_some() {
            part.to_owned()
        } else {
            percent_decode(part)?
        };
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.chars().any(|c| {
                c == '/' || c == '\\' || c.is_whitespace() || is_control(c) || c == '?' || c == '#'
            })
        {
            return None;
        }
        segments.push(part);
    }

    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let (username, password) = match userinfo {
        Some(u) => match u.split_once(':') {
            Some((n, p)) => (n, p),
            None => (u, ""),
        },
        None => ("", ""),
    };
    let (host, port) = split_host_port(hostport)?;
    if protocol == "ssh:" && (username != "git" || !password.is_empty()) {
        return None;
    }
    if authored && protocol != "ssh:" && (!username.is_empty() || !password.is_empty()) {
        return None;
    }
    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let provider = match host.as_str() {
        "github.com" => Some("github"),
        "gitee.com" => Some("gitee"),
        _ => None,
    };
    let last = segments.len() - 1;
    if let Some(p) = provider {
        let name = &segments[last];
        let stripped = if name.len() >= 4 && name.is_char_boundary(name.len() - 4) {
            let (stem, ext) = name.split_at(name.len() - 4);
            let matches = if p == "github" {
                ext.eq_ignore_ascii_case(".git")
            } else {
                ext == ".git"
            };
            if matches {
                stem.to_owned()
            } else {
                name.clone()
            }
        } else {
            name.clone()
        };
        segments[last] = stripped;
    }
    if segments[last].is_empty() || segments[last] == "." || segments[last] == ".." {
        return None;
    }
    let port = port.unwrap_or(match protocol {
        "ssh:" => 22,
        "https:" => 443,
        _ => 80,
    });
    let standard_endpoint = provider.is_some()
        && ((protocol == "https:" && port == 443) || (protocol == "ssh:" && port == 22));
    let repository_path = segments.join("/");
    // Only known forges map HTTPS and SSH to one repository namespace; other hosts keep their
    // transport, port and remote-relative or absolute path semantics.
    let endpoint = if standard_endpoint { "standard".to_owned() } else { format!("{protocol}{port}") };
    let path_kind = if provider.is_some() {
        "repository"
    } else if scp.is_some() && !scp_absolute {
        "relative"
    } else {
        "absolute"
    };
    let identity_path = if provider == Some("github") { repository_path.to_lowercase() } else { repository_path.clone() };
    let identity = serde_json::to_string(&json!([host, endpoint, path_kind, identity_path])).unwrap_or_default();
    Some(Remote {
        provider,
        protocol,
        path: repository_path,
        standard_endpoint,
        identity,
    })
}

/// `host[:port]`, with a bracketed IPv6 host kept whole. `None` for a port that is not 0..=65535.
fn split_host_port(hostport: &str) -> Option<(&str, Option<u32>)> {
    let (host, port) = if hostport.starts_with('[') {
        let end = hostport.find(']')?;
        let rest = &hostport[end + 1..];
        (&hostport[..=end], rest.strip_prefix(':'))
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    let port = match port {
        None | Some("") => None,
        Some(p) if p.bytes().all(|b| b.is_ascii_digit()) => {
            Some(p.parse::<u32>().ok().filter(|p| *p <= 65535)?)
        }
        Some(_) => return None,
    };
    Some((host, port))
}

/// `decodeURIComponent`: `None` on a malformed escape or invalid UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_addresses_of_one_forge_repository_share_an_identity() {
        let id = |url: &str, authored| parse_remote(url, authored).map(|r| r.identity);
        let https = id("https://github.com/Acme/Widget", true);
        assert!(https.is_some());
        assert_eq!(https, id("git@github.com:acme/widget.git", true));
        assert_eq!(https, id("ssh://git@github.com/acme/widget", true));
        // A credential on the local origin is tolerated and ignored; on the authored URL it is refused.
        assert_eq!(https, id("https://token@github.com/acme/widget.git", false));
        assert_eq!(id("https://token@github.com/acme/widget", true), None);
        // Another host keeps its transport and path: SCP and HTTPS are different repositories there.
        assert_ne!(id("https://git.example.com/a/b", true), id("git@git.example.com:a/b", true));
        assert_ne!(id("git@git.example.com:a/b", true), id("git@git.example.com:/a/b", true));
    }

    #[test]
    fn an_origin_is_redacted_before_it_reaches_a_diagnostic() {
        assert_eq!(redact_remote("https://user:pw@host.example/a/b.git"), "https://REDACTED@host.example/a/b.git");
        assert_eq!(redact_remote("git@github.com:a/b.git"), "git@github.com:a/b.git");
        assert_eq!(redact_remote("https://host.example/a/b?token=1"), "https://host.example/a/b?REDACTED");
    }

    #[test]
    fn lines_are_counted_like_the_source_does() {
        assert_eq!(source_line_count(""), 0);
        assert_eq!(source_line_count("a"), 1);
        assert_eq!(source_line_count("a\n"), 1);
        assert_eq!(source_line_count("a\nb"), 2);
        assert_eq!(source_line_count("a\r\nb\r\n"), 2);
        assert_eq!(source_line_count("a\rb\n\n"), 3);
    }
}
