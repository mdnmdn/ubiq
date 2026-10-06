//! P3.6/P3.7: the architecture gates (`gates::architecture`, wired into `compile`) against Archify's
//! own `validate architecture --json` (`gen-gate-cases.mjs`, `tests/gate_cases.json`):
//! the 250 router-corpus documents, 140 stress documents (self-loops, small hubs, authored `via`
//! corridors, tiny canvases, boundary titles) and 60 mutations of the deployment-ownership example
//! (boundary nesting), each under the document's profile, `--quality standard` and
//! `--quality showcase`. `composition/excessive-route-detour` never fires in these (only in the
//! `neg.comp-excessive-route-detour` golden).
//!
//! Compared per run, with no tolerance: `ok`, `stage`, every diagnostic's code, severity and whole
//! message **in order**, the receipt's `error` text, and a pass's composition `status` and `profile`.
//! A run Archify rejected at stage `check` (the artifact checker on the rendered HTML:
//! `artifact/*`, `composition/desktop-readability`) is compared like any other: `gates::artifact`
//! re-derives that checker from the scene (P9.2), so nothing is skipped.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use ubiq_archify::compile::{Opts, compile};
use ubiq_archify::diag::Severity;
use serde_json::Value;

#[derive(Default)]
struct Tally {
    runs: usize,
    exact: usize,
    skipped: usize,
    mismatched: usize,
}

#[test]
fn architecture_gates_match_archify_on_the_router_corpora() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/gate_cases.json");
    let cases: Vec<Value> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(cases.len() >= 440, "{} cases", cases.len());

    let mut by_run: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut codes: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let text = case["doc"].to_string();
        for (run, quality) in [("authored", None), ("standard", Some("standard")), ("showcase", Some("showcase"))] {
            let tally = by_run.entry(run).or_default();
            tally.runs += 1;
            let want = &case["runs"][run];
            let compiled = compile(&text, &Opts { input: "<input>", doc_type: Some("architecture"), quality });
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
                    "diagnostics\n    only in Archify: {:?}\n    only in ours:    {:?}\n    (order {})",
                    theirs.iter().filter(|d| !ours.contains(d)).collect::<Vec<_>>(),
                    ours.iter().filter(|d| !theirs.contains(d)).collect::<Vec<_>>(),
                    if theirs.len() == ours.len() && theirs.iter().all(|d| ours.contains(d)) { "differs" } else { "n/a" },
                ));
            }
            if !compiled.ok() {
                let theirs = want["error"].as_str().unwrap_or_default();
                let ours = compiled.receipt.error.as_deref().unwrap_or_default();
                if theirs != ours {
                    problems.push(format!("error text\n    Archify: {theirs}\n    ours:    {ours}"));
                }
            } else if want["ok"] == true {
                let status = compiled.receipt.composition.as_ref().map(|c| c["status"].clone());
                if status.as_ref() != Some(&want["composition"]["status"]) {
                    problems.push(format!("composition.status: Archify {}, ours {status:?}", want["composition"]["status"]));
                }
                if want["composition"]["profile"] != compiled.profile {
                    problems.push(format!("composition.profile: Archify {}, ours {}", want["composition"]["profile"], compiled.profile));
                }
            }
            if problems.is_empty() {
                tally.exact += 1;
                for (code, _, _) in &ours {
                    *codes.entry(code.clone()).or_default() += 1;
                }
            } else {
                tally.mismatched += 1;
                failures.push(format!("{name} [{run}]\n  {}", problems.join("\n  ")));
            }
        }
    }

    println!("\narchitecture gates vs Archify (route, placement, stress and deployment corpora)");
    println!("{:<10}{:>6}{:>8}{:>9}{:>11}", "run", "runs", "exact", "skipped", "mismatch");
    for (run, t) in &by_run {
        println!("{run:<10}{:>6}{:>8}{:>9}{:>11}", t.runs, t.exact, t.skipped, t.mismatched);
    }
    println!("diagnostics reproduced exactly, by code:");
    for (code, count) in &codes {
        println!("  {count:>5}  {code}");
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}
