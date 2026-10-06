//! Repository-evidence verification (`ubiq_archify::evidence::verify`, the host's `git` behind it)
//! against Archify's own `validate --json --repo-root`.
//!
//! `tests/archify_evidence_cases.json` (the frozen output of the Archify Node oracle) holds a small deterministic Git
//! repository (fixed author, committer and dates, no user configuration: its commit SHA is the same
//! wherever it is built) and 44 documents that cite it well and badly: a file, a line past the end,
//! a missing blob, an unknown revision, an origin that does not match, a root that is not the
//! top-level, a path that escapes, the five types' node collections. The test builds the same
//! repositories in a temp directory, replays every document through `compile_with` with the git
//! verifier and compares `ok`, the stage and every diagnostic whole (code, message, subject,
//! evidence, supported fixes), the temp path written as `<root-base>`.
//!
//! Also here: what `compile` does with no verifier, which is what the interface does.

use std::path::{Path, PathBuf};
use std::process::Command;

use ubiq_archify::compile::{Opts, compile, compile_with};
use ubiq_host::archify::verifier;
use serde_json::Value;

const GIT_ENV: [(&str, &str); 8] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "Archify Oracle"),
    ("GIT_AUTHOR_EMAIL", "oracle@example.com"),
    ("GIT_AUTHOR_DATE", "2020-01-01T00:00:00 +0000"),
    ("GIT_COMMITTER_NAME", "Archify Oracle"),
    ("GIT_COMMITTER_EMAIL", "oracle@example.com"),
    ("GIT_COMMITTER_DATE", "2020-01-01T00:00:00 +0000"),
];

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(dir).args(args).envs(GIT_ENV).output().expect("git is installed");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A repository with the oracle's files, committed, `origin` set (or not).
fn build(dir: &Path, files: &Value, origin: Option<&str>) -> String {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q"]);
    for (file, content) in files.as_object().unwrap() {
        let target = dir.join(file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content.as_str().unwrap()).unwrap();
    }
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "oracle"]);
    if let Some(url) = origin {
        git(dir, &["remote", "add", "origin", url]);
    }
    git(dir, &["rev-parse", "HEAD"])
}

fn without_rule(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(subject)) = map.get_mut("subject") {
                subject.remove("rule");
            }
            map.values_mut().for_each(without_rule);
        }
        Value::Array(items) => items.iter_mut().for_each(without_rule),
        _ => {}
    }
}

fn replacing(value: &Value, bases: &[String]) -> Value {
    let mut text = value.to_string();
    for base in bases {
        text = text.replace(&serde_json::to_string(base).unwrap().trim_matches('"').to_owned(), "<root-base>");
    }
    serde_json::from_str(&text).unwrap()
}

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archify-evidence-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn verification_matches_archify_with_a_real_repository() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/archify_evidence_cases.json");
    let oracle: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let work = work_dir("oracle");
    // The resolved path first: on macOS it extends the plain one (`/private/var/...`).
    let bases = vec![std::fs::canonicalize(&work).unwrap().to_string_lossy().into_owned(), work.to_string_lossy().into_owned()];

    let files = &oracle["files"];
    let origin = oracle["origin"].as_str().unwrap();
    let sha = build(&work.join("main"), files, Some(origin));
    assert_eq!(sha, oracle["sha"], "the oracle repository must build to the same commit everywhere");
    build(&work.join("ssh-origin"), files, Some("git@github.com:acme/widget.git"));
    build(&work.join("no-origin"), files, None);
    build(&work.join("other-origin"), files, Some("https://github.com/someone/else.git"));
    build(&work.join("cred-origin"), files, Some("https://user:secret@github.com/someone/else.git"));
    build(&work.join("query-origin"), files, Some("https://github.com/someone/else.git?token=abc"));
    build(&work.join("dirty"), files, Some(origin));
    std::fs::write(work.join("dirty/src/a.rs"), "fn a() {\n    b();\n}\n// one\n// two\n// three\n").unwrap();
    std::fs::create_dir_all(work.join("plain")).unwrap();

    let (mut runs, mut passed, mut mismatched) = (0, 0, Vec::new());
    for case in oracle["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let doc_type = case["doc"]["diagram_type"].as_str().unwrap();
        let text = case["doc"].to_string().replace("@SHA_UPPER@", &sha.to_uppercase()).replace("@SHA@", &sha);
        let opts = Opts { input: "<input>", doc_type: Some(doc_type), quality: None };
        let compiled = match case["root"].as_str() {
            None => compile(&text, &opts),
            Some(root) => {
                let root = match root {
                    "sub" => work.join("main/src"),
                    "missing" => work.join("does-not-exist"),
                    other => work.join(other),
                };
                let verify = verifier(root.to_string_lossy().into_owned());
                compile_with(&text, &opts, Some(&verify))
            }
        };
        runs += 1;
        let mut ours = replacing(&serde_json::to_value(&compiled.receipt.diagnostics).unwrap(), &bases);
        without_rule(&mut ours);
        let mut problems = Vec::new();
        if compiled.ok() != (case["ok"] == true) {
            problems.push(format!("ok: Archify {}, ours {}", case["ok"], compiled.ok()));
        }
        if ours != case["diagnostics"] {
            problems.push(format!("diagnostics\n    Archify: {}\n    ours:    {ours}", case["diagnostics"]));
        }
        if problems.is_empty() {
            passed += 1;
        } else {
            mismatched.push(format!("{name}\n  {}", problems.join("\n  ")));
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    println!("repository evidence: {runs} documents, {passed} exact");
    assert!(mismatched.is_empty(), "{} mismatches:\n{}", mismatched.len(), mismatched.join("\n"));
    assert!(runs >= 40);
}

/// What the interface does: no verifier. A document that declares source evidence is refused with
/// `root-required`, after the declaration's own checks and before any source is looked at.
#[test]
fn without_a_verifier_evidence_is_refused_as_archify_refuses_it_without_a_root() {
    let doc = |repository: Value, sources: Value| {
        serde_json::json!({
            "schema_version": 1, "diagram_type": "dataflow",
            "meta": {"title": "t", "output": "a.html", "repository": repository},
            "stages": [{"label": "In"}, {"label": "Out"}],
            "nodes": [{"id": "a", "type": "backend", "label": "A", "stage": 0, "row": 0, "sources": sources},
                      {"id": "b", "type": "backend", "label": "B", "stage": 1, "row": 0}],
            "flows": [{"from": "a", "to": "b", "label": "go"}]
        })
        .to_string()
    };
    let opts = Opts { input: "<input>", doc_type: Some("dataflow"), quality: None };
    let sha = "a".repeat(40);
    let good = serde_json::json!({"url": "https://github.com/acme/widget", "revision": sha});
    // The path escapes the repository, but Archify says `root-required` first: nothing is read yet.
    let c = compile(&doc(good.clone(), serde_json::json!([{"path": "../x.rs"}])), &opts);
    assert_eq!(c.diagnostics()[0].code, "repository-evidence/root-required");
    // The declaration is checked before the root is asked for.
    let bad_url = serde_json::json!({"url": "/home/me/widget", "revision": sha});
    let c = compile(&doc(bad_url, serde_json::json!([{"path": "a.rs"}])), &opts);
    assert_eq!(c.diagnostics()[0].code, "repository-evidence/url-invalid");
    // A document with no evidence never asks for a root.
    let plain = serde_json::json!({
        "schema_version": 1, "diagram_type": "dataflow", "meta": {"title": "t", "output": "a.html"},
        "stages": [{"label": "In"}, {"label": "Out"}],
        "nodes": [{"id": "a", "type": "backend", "label": "A", "stage": 0, "row": 0},
                  {"id": "b", "type": "backend", "label": "B", "stage": 1, "row": 0}],
        "flows": [{"from": "a", "to": "b", "label": "go"}]
    });
    assert!(compile(&plain.to_string(), &opts).ok());
}
