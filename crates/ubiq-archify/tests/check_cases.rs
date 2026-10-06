//! P9.2: the artifact checker (`gates::artifact`) against Archify's own, on generated corpora
//! (`gen-check-cases.mjs`, `tests/check_cases.json`): workflow v1 and v2, lifecycle v1 and v2
//! and architecture documents, each under the document's profile, `--quality standard` and
//! `--quality showcase`.
//!
//! Compared per run, with no tolerance: `ok`, `stage`, every diagnostic's code, severity and whole
//! message in order, and the SHA-256 of the checker's `checks` and `composition` blocks as canonical
//! JSON (sorted keys): the whole of what the checker measured, for the runs it rejected (stage
//! `check`) and for every run that passed. To see *what* differs in a run, regenerate with
//! `--full <dir>` and diff that receipt against `compile`'s.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use ubiq_archify::compile::{Opts, compile};
use ubiq_archify::diag::Severity;
use serde_json::Value;
use sha2::{Digest, Sha256};

mod deviation;
use deviation::{Deviations, deviates};

/// `JSON.stringify` with sorted keys.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}:", serde_json::to_string(key).unwrap());
                canonical(&map[key], out);
            }
            out.push('}');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap()),
    }
}

fn digest(value: &Value) -> String {
    let mut text = String::new();
    canonical(value, &mut text);
    Sha256::digest(text.as_bytes()).iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[derive(Default)]
struct Tally {
    runs: usize,
    exact: usize,
    mismatched: usize,
}

#[test]
fn the_artifact_checker_matches_archify_on_the_generated_corpora() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/check_cases.json");
    let cases: Vec<Value> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(cases.len() >= 200, "{} documents", cases.len());

    let mut by_kind: BTreeMap<String, Tally> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut deviations = Deviations::open("check_cases");
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let doc_type = case["type"].as_str().unwrap();
        let text = case["doc"].to_string();
        let deviating = deviates(Some(doc_type), &text);
        for (run, quality) in [("authored", None), ("standard", Some("standard")), ("showcase", Some("showcase"))] {
            let want = &case["runs"][run];
            if want["crashed"] == true {
                continue;
            }
            if deviating {
                let compiled = compile(&text, &Opts { input: "<input>", doc_type: Some(doc_type), quality });
                let (composition, checks) = if compiled.ok() {
                    (compiled.receipt.composition.clone(), Some(Value::Array(compiled.receipt.checks.clone())))
                } else {
                    let checker = compiled.receipt.checker.clone().unwrap_or(Value::Null);
                    (Some(checker["composition"].clone()), Some(checker["checks"].clone()))
                };
                let diagnostics: Vec<String> = compiled.diagnostics().iter().map(|d| d.message.clone()).collect();
                let ours = serde_json::json!({
                    "ok": compiled.ok(),
                    "diagnostics": diagnostics,
                    "composition": composition.as_ref().map(digest),
                    "checks": checks.as_ref().map(digest),
                });
                failures.extend(deviations.check(&format!("{name} [{run}]"), ours));
                continue;
            }
            let kind = format!("{doc_type} {}", if want["ok"] == true { "pass" } else { want["stage"].as_str().unwrap_or("?") });
            let tally = by_kind.entry(kind).or_default();
            tally.runs += 1;
            let compiled = compile(&text, &Opts { input: "<input>", doc_type: Some(doc_type), quality });
            let mut problems = Vec::new();
            if compiled.ok() != (want["ok"] == true) {
                problems.push(format!("ok: Archify {}, ours {}", want["ok"], compiled.ok()));
            }
            let theirs: Vec<(String, Severity, String)> = want["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| {
                    let severity = if d["severity"] == "warning" { Severity::Warning } else { Severity::Error };
                    (d["code"].as_str().unwrap().to_owned(), severity, d["message"].as_str().unwrap().to_owned())
                })
                .collect();
            let ours: Vec<(String, Severity, String)> =
                compiled.diagnostics().iter().map(|d| (d.code.clone(), d.severity, d.message.clone())).collect();
            if ours != theirs {
                problems.push(format!(
                    "diagnostics\n    only in Archify: {:?}\n    only in ours:    {:?}",
                    theirs.iter().filter(|d| !ours.contains(d)).collect::<Vec<_>>(),
                    ours.iter().filter(|d| !theirs.contains(d)).collect::<Vec<_>>(),
                ));
            }
            if want.get("composition").is_some() {
                let (composition, checks) = if compiled.ok() {
                    (compiled.receipt.composition.clone(), Some(Value::Array(compiled.receipt.checks.clone())))
                } else {
                    let checker = compiled.receipt.checker.clone().unwrap_or(Value::Null);
                    (Some(checker["composition"].clone()), Some(checker["checks"].clone()))
                };
                if composition.as_ref().map(digest).as_deref() != want["composition"].as_str() {
                    problems.push("composition block differs (regenerate with --full to diff)".to_owned());
                }
                if checks.as_ref().map(digest).as_deref() != want["checks"].as_str() {
                    problems.push("checks differ (regenerate with --full to diff)".to_owned());
                }
            }
            if problems.is_empty() {
                tally.exact += 1;
            } else {
                tally.mismatched += 1;
                failures.push(format!("{name} [{run}]\n  {}", problems.join("\n  ")));
            }
        }
    }

    println!("\nartifact checker vs Archify (generated corpora)");
    println!("{:<22}{:>6}{:>8}{:>11}", "kind", "runs", "exact", "mismatch");
    for (kind, t) in &by_kind {
        println!("{kind:<22}{:>6}{:>8}{:>11}", t.runs, t.exact, t.mismatched);
    }
    println!("{} runs against the recorded deviation (D40)", deviations.len());
    failures.extend(deviations.finish());
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    assert!(by_kind.get("lifecycle check").is_some_and(|t| t.runs >= 10), "the corpora lost their stage `check` runs");
}
