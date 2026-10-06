//! The golden harness, validation part (P1.6, D5, D20).
//!
//! Every `tests/fixtures/*.golden.json` holds three runs of Archify's `validate --json`: the
//! document's own quality profile (`authored`), `--quality standard` and `--quality showcase`. Each
//! is replayed through [`compile`] and the diagnostic sets are compared as `(code, subject.path,
//! severity, first line of the message)`, after the receipt's dedup by message. Also compared: `ok`,
//! `stage`, the exit code, and the reported composition profile of a pass.
//!
//! # What is compared, and what is skipped
//!
//! Only the rules this crate implements are compared ([`classify`]): the input, output, schema,
//! relationship, engineering and repository-evidence families, `internal/unclassified`, the graph
//! rules of V9 (the `layout/constraint` sentences and `workflow/*` codes that need no geometry).
//! Everything else Archify says is geometry (`clean-flow`, `composition`, `legend`, the
//! `layout/*-out-of-bounds` family, the workflow solver) and is *skipped with a reason* by the
//! [`classify`] table, never silently: the summary counts skipped diagnostics per reason, and a run
//! that had to skip anything is `partial` (its `ok` is not compared, because Archify's `ok:false`
//! may be the skipped rule's). Our side is never filtered: a diagnostic we emit that Archify did not
//! say is a mismatch. Lift a skip by implementing the rule (P3-P6), not by editing the table.
//!
//! Lifecycle is complete too (v2 P4.2, v1 P4.3): nothing it says is skipped.
//!
//! Architecture (P3.6), dataflow (P2.5) and sequence (P4.1) are complete and skip nothing: every rule family is compared, whole
//! messages (repair suggestions included) and, for a V9 failure, the receipt's `error` text, which
//! carries the problem order. A passing run also compares `composition.status`. The fixtures were
//! written with a JSON compactor that turns `[1, 2]` into `[1,2]` even inside strings, so our side
//! goes through the same transformation before comparing.
//!
//! A workflow v1 document the frame padding changes (D40) is compared, in the layout-json and
//! artifact-checker tests, with our own recorded output (`tests/deviation/mod.rs`), not Archify's.
//!
//! Cases with no document on disk (`source_missing`) are `input/read`, which is the caller's I/O
//! (`tests/cli.rs`). The architecture `layout_json` part is `architecture_layout_json_matches_the_goldens`
//! below; the `render` part is covered by the per-type router tests.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use ubiq_archify::compile::{Opts, compile};
use ubiq_archify::diag::Severity;
use serde_json::Value;

mod deviation;
use deviation::{Deviations, deviates};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Whether a diagnostic of Archify's receipt is a rule this crate implements. `Err` carries the
/// reason it is not (the skip table).
fn classify(_message: &str) -> Result<(), &'static str> {
    // The post-render artifact checker (stage `check`, `gates::artifact`) is ported (P9.2): its
    // diagnostics are compared like any other, nothing is skipped for it.
    // Dataflow is complete (P2.5): its geometry, `clean-flow`, `composition` and `legend` rules are
    // all implemented, so nothing it says is skipped.
    // Sequence is complete too (P4.1): `gates::sequence` is `validateSequence`, graph rules included.
    // So is lifecycle, v1 and v2 (P4.2, P4.3, `gates::lifecycle`).
    // Architecture is complete too (P3.6, `gates::architecture`): `validateArchitecture`, the legend
    // errors and the V6 profile rules, so nothing it says is skipped.
    // Workflow is complete too (P6.4): `gates::workflow_v1` and `gates::workflow_v2` are
    // `validateWorkflow`, graph rules included, and the router's failures are compared whole.
    // Every type is complete: nothing is skipped by rule, only the artifact checker above.
    Ok(())
}

type Key = (String, Option<String>, Severity, String);

/// `full`: compare every line of the message (the repair suggestions too), as for dataflow; else
/// the first line only.
fn key(code: &str, path: Option<&str>, severity: Severity, message: &str, full: bool) -> Key {
    // The text of these two is V8's (a parser error, a stack message): code and path only.
    let message = if code == "internal/unclassified" || code == "input/json-parse" {
        ""
    } else if full {
        message
    } else {
        message.lines().next().unwrap_or_default()
    };
    (
        code.to_owned(),
        path.map(str::to_owned),
        severity,
        compact_numeric_arrays(message),
    )
}

/// The fixture writer (`gen-fixtures.mjs`, `stringify`) collapses `[1, 2]` to `[1,2]` in
/// *every* numeric array of the JSON text, strings included. Apply the same to our side so a
/// message like `... segment 0 [315, 186] -> [315, 148] ...` compares equal to its golden.
fn compact_numeric_arrays(text: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        let num = r"-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?";
        Regex::new(&format!(r"\[\s*({num}(?:\s*,\s*{num})*)\s*\]")).unwrap()
    });
    let comma = Regex::new(r"\s*,\s*").unwrap();
    re.replace_all(text, |caps: &regex::Captures<'_>| format!("[{}]", comma.replace_all(&caps[1], ",")))
        .into_owned()
}

#[derive(Default)]
struct Tally {
    runs: usize,
    exact: usize,
    partial: usize,
    skipped: usize,
    mismatched: usize,
}

#[test]
fn validation_matches_the_goldens() {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".golden.json"))
        .collect();
    paths.sort();

    let mut by_type: BTreeMap<String, Tally> = BTreeMap::new();
    let mut skipped_diagnostics: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut skipped_cases: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut failures = Vec::new();

    for path in &paths {
        let case: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let name = case["name"].as_str().unwrap();
        let doc_type = case["type"].as_str().unwrap();
        let tally = by_type.entry(doc_type.to_owned()).or_default();
        // All five types are fully implemented: compare whole messages, not just their first line.
        let full = true;

        let text = match (case.get("source_text"), case.get("source_doc")) {
            (Some(Value::String(text)), _) => text.clone(),
            (_, Some(doc)) => doc.to_string(),
            _ => {
                for _ in 0..3 {
                    tally.runs += 1;
                    tally.skipped += 1;
                    *skipped_cases
                        .entry("no document on disk: input/read is the caller's I/O (tests/cli.rs)")
                        .or_default() += 1;
                }
                continue;
            }
        };

        for (run, quality) in [
            ("authored", None),
            ("standard", Some("standard")),
            ("showcase", Some("showcase")),
        ] {
            tally.runs += 1;
            let golden = &case["validate_receipt"][run];
            let Some(receipt) = golden.get("receipt") else {
                tally.skipped += 1;
                *skipped_cases
                    .entry("Archify printed no JSON receipt")
                    .or_default() += 1;
                continue;
            };

            let mut skipped_here = 0;
            let mut expected = BTreeSet::new();
            for d in receipt["diagnostics"].as_array().into_iter().flatten() {
                let code = d["code"].as_str().unwrap_or_default();
                let message = d["message"].as_str().unwrap_or_default();
                match classify(message) {
                    Ok(()) => {
                        let severity = if d["severity"] == "warning" {
                            Severity::Warning
                        } else {
                            Severity::Error
                        };
                        expected.insert(key(
                            code,
                            d["subject"]["path"].as_str(),
                            severity,
                            message,
                            full,
                        ));
                    }
                    Err(reason) => {
                        skipped_here += 1;
                        *skipped_diagnostics.entry(reason).or_default() += 1;
                    }
                }
            }

            let compiled = compile(
                &text,
                &Opts {
                    input: "<input>",
                    doc_type: Some(doc_type),
                    quality,
                },
            );
            let actual: BTreeSet<Key> = compiled
                .diagnostics()
                .iter()
                .map(|d| key(&d.code, d.subject.path.as_deref(), d.severity, &d.message, full))
                .collect();

            let mut problems = Vec::new();
            if actual != expected {
                problems.push(format!(
                    "diagnostics\n    only in Archify: {:?}\n    only in ours:    {:?}",
                    expected.difference(&actual).collect::<Vec<_>>(),
                    actual.difference(&expected).collect::<Vec<_>>()
                ));
            }
            if skipped_here == 0 {
                if compiled.ok() != (receipt["ok"] == true) {
                    problems.push(format!(
                        "ok: Archify {}, ours {}",
                        receipt["ok"],
                        compiled.ok()
                    ));
                } else if compiled.ok() {
                    let theirs = receipt["composition"]["profile"]
                        .as_str()
                        .unwrap_or_default();
                    if theirs != compiled.profile {
                        problems.push(format!(
                            "composition.profile: Archify {theirs}, ours {}",
                            compiled.profile
                        ));
                    }
                    // Dataflow composes its own block (P2.5): the status must agree. The summary
                    // and metrics are not compared: `desktop-readability` and `viewport-height`
                    // are not measured (D15, the reader's viewport).
                    let ours = compiled.receipt.composition.as_ref().map(|c| &c["status"]);
                    if full && ours != Some(&receipt["composition"]["status"]) {
                        problems.push(format!(
                            "composition.status: Archify {}, ours {ours:?}",
                            receipt["composition"]["status"]
                        ));
                    }
                } else {
                    let stage = serde_json::to_value(compiled.receipt.stage).unwrap();
                    if stage != receipt["stage"] {
                        problems.push(format!("stage: Archify {}, ours {stage}", receipt["stage"]));
                    }
                    // The problem order is the receipt's `error` text.
                    let theirs = receipt["error"].as_str().unwrap_or_default();
                    let ours = compiled.receipt.error.as_deref().unwrap_or_default();
                    if full
                        && (theirs.starts_with("Architecture layout validation failed")
                            || theirs.starts_with("Data-flow layout validation failed")
                            || theirs.starts_with("Sequence layout validation failed")
                            || theirs.starts_with("Lifecycle layout validation failed")
                            || theirs.starts_with("Workflow layout validation failed"))
                        && compact_numeric_arrays(ours) != compact_numeric_arrays(theirs)
                    {
                        problems.push(format!("error text\n    Archify: {theirs}\n    ours:    {ours}"));
                    }
                    if compiled.receipt.exit_code() as i64 != golden["exit_code"].as_i64().unwrap()
                    {
                        problems.push(format!(
                            "exit code: Archify {}, ours {}",
                            golden["exit_code"],
                            compiled.receipt.exit_code()
                        ));
                    }
                }
            } else if !compiled.ok() && expected.is_empty() {
                problems.push("we fail where Archify said only skipped rules".to_owned());
            }

            if problems.is_empty() {
                if skipped_here == 0 {
                    tally.exact += 1;
                } else {
                    tally.partial += 1;
                }
            } else {
                tally.mismatched += 1;
                failures.push(format!("{name} [{run}]\n  {}", problems.join("\n  ")));
            }
        }
    }

    println!("\ngolden validation (compared / skipped by type, per quality run)");
    println!(
        "{:<13}{:>6}{:>8}{:>9}{:>9}{:>11}",
        "type", "runs", "exact", "partial", "skipped", "mismatch"
    );
    let mut total = Tally::default();
    for (doc_type, t) in &by_type {
        println!(
            "{doc_type:<13}{:>6}{:>8}{:>9}{:>9}{:>11}",
            t.runs, t.exact, t.partial, t.skipped, t.mismatched
        );
        total.runs += t.runs;
        total.exact += t.exact;
        total.partial += t.partial;
        total.skipped += t.skipped;
        total.mismatched += t.mismatched;
    }
    println!(
        "{:<13}{:>6}{:>8}{:>9}{:>9}{:>11}",
        "total", total.runs, total.exact, total.partial, total.skipped, total.mismatched
    );
    println!("exact: every Archify diagnostic compared; partial: also had skipped rules (below)");
    println!("skipped diagnostics (Archify said them; not implemented yet):");
    for (reason, count) in &skipped_diagnostics {
        println!("  {count:>4}  {reason}");
    }
    println!("skipped runs:");
    for (reason, count) in &skipped_cases {
        println!("  {count:>4}  {reason}");
    }

    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Guard against the harness quietly comparing nothing.
    assert!(
        total.exact + total.partial >= 500,
        "only {} runs compared",
        total.exact + total.partial
    );
}

/// serde_json's default float parser is up to 1 ulp off; every number token becomes `"n:<token>"`,
/// so two documents compare by what was printed (`tests/architecture_routes.rs`).
fn exact_numbers(text: &str) -> String {
    let token = regex::Regex::new(r#""(?:[^"\\]|\\.)*"|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)"#).unwrap();
    token
        .replace_all(text, |caps: &regex::Captures<'_>| match caps.get(1) {
            Some(n) => format!("\"n:{}\"", n.as_str()),
            None => caps[0].to_owned(),
        })
        .into_owned()
}

/// `subject.rule` is this crate's own field (D7), not Archify's.
fn strip_rule(v: &mut Value) {
    match v {
        Value::Object(m) => {
            if let Some(Value::Object(subject)) = m.get_mut("subject") {
                subject.remove("rule");
            }
            m.values_mut().for_each(strip_rule);
        }
        Value::Array(a) => a.iter_mut().for_each(strip_rule),
        _ => {}
    }
}

/// The paths where `ours` and `golden` differ, at most `limit`.
fn diff(path: &str, ours: &Value, golden: &Value, out: &mut Vec<String>, limit: usize) {
    if out.len() >= limit || ours == golden {
        return;
    }
    match (ours, golden) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys().filter(|k| !a.contains_key(*k))) {
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => diff(&format!("{path}/{key}"), x, y, out, limit),
                    (Some(x), None) => out.push(format!("{path}/{key}: only ours: {x}")),
                    (None, Some(y)) => out.push(format!("{path}/{key}: only Archify: {y}")),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                diff(&format!("{path}/{i}"), x, y, out, limit);
            }
        }
        _ => out.push(format!("{path}: ours {ours} / Archify {golden}")),
    }
}

/// P3.7: `archify_layout` / `validate --layout-json` against the `layout_json` of every architecture
/// golden, under the document's own quality and both overrides. A receipt is compared whole (every
/// number as printed, rejected layouts' `error` and `diagnostics` too); a failure before the layout
/// is the ordinary failure receipt, compared as the text Archify printed on stderr.
#[test]
fn architecture_layout_json_matches_the_goldens() {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".golden.json"))
        .collect();
    paths.sort();
    let (mut runs, mut exact, mut texts) = (0, 0, 0);
    let mut failures = Vec::new();
    for path in &paths {
        let raw = fs::read_to_string(path).unwrap();
        let case: Value = serde_json::from_str(&raw).unwrap();
        if case["type"] != "architecture" || case.get("layout_json").is_none_or(Value::is_null) {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let text = match (case.get("source_text"), case.get("source_doc")) {
            (Some(Value::String(text)), _) => text.clone(),
            (_, Some(doc)) => doc.to_string(),
            _ => continue,
        };
        let exact_case: Value = serde_json::from_str(&exact_numbers(&raw)).unwrap();
        // The generator records the authored layout receipt only for negatives.
        let negative = case["group"] == "negative";
        for (run, quality) in [("authored", None), ("standard", Some("standard")), ("showcase", Some("showcase"))] {
            if negative && quality.is_some() {
                continue;
            }
            runs += 1;
            let golden = case["layout_json_by_quality"].get(run).filter(|v| !v.is_null()).unwrap_or(&case["layout_json"]);
            let golden_exact = exact_case["layout_json_by_quality"]
                .get(run)
                .filter(|v| !v.is_null())
                .unwrap_or(&exact_case["layout_json"]);
            let reply = ubiq_archify::layout_json::architecture(&text, &Opts { input: "<input>", doc_type: Some("architecture"), quality });
            let mut problems = Vec::new();
            if reply.exit_code() as i64 != golden["exit_code"].as_i64().unwrap() {
                problems.push(format!("exit code: Archify {}, ours {}", golden["exit_code"], reply.exit_code()));
            }
            match (&reply, golden.get("receipt")) {
                (ubiq_archify::layout_json::LayoutReply::Layout { json, .. }, Some(_)) => {
                    let mut ours: Value =
                        serde_json::from_str(&exact_numbers(&compact_numeric_arrays(&serde_json::to_string(json).unwrap()))).unwrap();
                    strip_rule(&mut ours);
                    diff("", &ours, &golden_exact["receipt"], &mut problems, 6);
                }
                (ubiq_archify::layout_json::LayoutReply::Failed(receipt), None) => {
                    texts += 1;
                    let theirs = golden["stderr"].as_str().unwrap_or_default();
                    let ours = format!("{}\n", receipt.text());
                    // The sentence of these two is V8's (a parser error, a stack message): compare the code.
                    let key = |text: &str| {
                        ["[internal/unclassified]", "[input/json-parse]"]
                            .into_iter()
                            .find(|code| text.contains(code))
                            .map_or_else(|| compact_numeric_arrays(text), str::to_owned)
                    };
                    if key(&ours) != key(theirs) {
                        problems.push(format!("stderr\n    Archify: {theirs:?}\n    ours:    {ours:?}"));
                    }
                }
                (ubiq_archify::layout_json::LayoutReply::Layout { .. }, None) => problems.push("we print a layout where Archify printed text".to_owned()),
                (ubiq_archify::layout_json::LayoutReply::Failed(r), Some(_)) => problems.push(format!("we fail before the layout ({:?}) where Archify printed a layout", r.error)),
            }
            if problems.is_empty() {
                exact += 1;
            } else {
                failures.push(format!("{name} [{run}]\n  {}", problems.join("\n  ")));
            }
        }
    }
    println!("\narchitecture layout_json: {runs} runs, {exact} exact ({texts} failures compared as text), {} mismatched", failures.len());
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
    // The examples and fixtures run under three qualities, every negative once (113 today).
    assert!(runs >= 110, "only {runs} runs compared");
}

/// P6.4: `validate workflow --layout-json` (`layout_json::workflow`) against the `layout_json` of
/// every workflow golden, under the document's own quality and both overrides where the generator
/// recorded them. The compiler receipt is compared whole (every number as printed, a rejected
/// layout's diagnostics too); a failure before the compiler is the ordinary failure text.
#[test]
fn workflow_layout_json_matches_the_goldens() {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".golden.json"))
        .collect();
    paths.sort();
    let (mut runs, mut exact, mut texts) = (0, 0, 0);
    let mut failures = Vec::new();
    let mut deviations = Deviations::open("workflow_layout_json");
    for path in &paths {
        let raw = fs::read_to_string(path).unwrap();
        let case: Value = serde_json::from_str(&raw).unwrap();
        if case["type"] != "workflow" || case.get("layout_json").is_none_or(Value::is_null) {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let text = match (case.get("source_text"), case.get("source_doc")) {
            (Some(Value::String(text)), _) => text.clone(),
            (_, Some(doc)) => doc.to_string(),
            _ => continue,
        };
        let exact_case: Value = serde_json::from_str(&exact_numbers(&raw)).unwrap();
        let negative = case["group"] == "negative";
        let deviating = deviates(Some("workflow"), &text);
        for (run, quality) in [("authored", None), ("standard", Some("standard")), ("showcase", Some("showcase"))] {
            if negative && quality.is_some() {
                continue;
            }
            runs += 1;
            let golden = case["layout_json_by_quality"].get(run).filter(|v| !v.is_null()).unwrap_or(&case["layout_json"]);
            let golden_exact = exact_case["layout_json_by_quality"]
                .get(run)
                .filter(|v| !v.is_null())
                .unwrap_or(&exact_case["layout_json"]);
            let reply = ubiq_archify::layout_json::workflow(&text, &Opts { input: "<input>", doc_type: Some("workflow"), quality });
            if deviating {
                let ours = match &reply {
                    ubiq_archify::layout_json::LayoutReply::Layout { json, .. } => as_written(&serde_json::to_value(json).unwrap()),
                    ubiq_archify::layout_json::LayoutReply::Failed(receipt) => Value::String(receipt.text()),
                };
                failures.extend(deviations.check(&format!("{name} [{run}]"), serde_json::json!({ "exit_code": reply.exit_code(), "out": ours })));
                continue;
            }
            let mut problems = Vec::new();
            if reply.exit_code() as i64 != golden["exit_code"].as_i64().unwrap() {
                problems.push(format!("exit code: Archify {}, ours {}", golden["exit_code"], reply.exit_code()));
            }
            match (&reply, golden.get("receipt")) {
                (ubiq_archify::layout_json::LayoutReply::Layout { json, .. }, Some(_)) => {
                    let mut ours: Value =
                        serde_json::from_str(&exact_numbers(&compact_numeric_arrays(&serde_json::to_string(json).unwrap()))).unwrap();
                    strip_rule(&mut ours);
                    // The `viewbox-capacity` fix prints `[768, 404]` in the Archify compiler and `[768,404]` in the fixture.
                    let mut want = golden_exact["receipt"].clone();
                    strip_rule(&mut want);
                    diff("", &ours, &want, &mut problems, 6);
                }
                (ubiq_archify::layout_json::LayoutReply::Failed(receipt), None) => {
                    texts += 1;
                    let theirs = golden["stderr"].as_str().unwrap_or_default();
                    let ours = format!("{}\n", receipt.text());
                    let key = |text: &str| {
                        ["[internal/unclassified]", "[input/json-parse]"]
                            .into_iter()
                            .find(|code| text.contains(code))
                            .map_or_else(|| compact_numeric_arrays(text), str::to_owned)
                    };
                    if key(&ours) != key(theirs) {
                        problems.push(format!("stderr\n    Archify: {theirs:?}\n    ours:    {ours:?}"));
                    }
                }
                (ubiq_archify::layout_json::LayoutReply::Layout { .. }, None) => problems.push("we print a layout where Archify printed text".to_owned()),
                (ubiq_archify::layout_json::LayoutReply::Failed(r), Some(_)) => problems.push(format!("we fail before the compiler ({:?}) where Archify printed a receipt", r.error)),
            }
            if problems.is_empty() {
                exact += 1;
            } else {
                failures.push(format!("{name} [{run}]\n  {}", problems.join("\n  ")));
            }
        }
    }
    println!(
        "\nworkflow layout_json: {runs} runs, {exact} exact ({texts} failures compared as text), {} against the recorded deviation (D40), {} mismatched",
        deviations.len(),
        failures.len()
    );
    failures.extend(deviations.finish());
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
    assert!(runs >= 40, "only {runs} runs compared");
}

/// Our JSON the way the fixture writer printed Archify's: numeric arrays compacted, numbers
/// compared by what was printed ([`exact_numbers`]).
fn as_written(value: &Value) -> Value {
    serde_json::from_str(&exact_numbers(&compact_numeric_arrays(&serde_json::to_string(value).unwrap()))).unwrap()
}

/// P9.2: the artifact checker. A passing run's `checks` and whole `composition` block (metrics,
/// route review, leading space, readability evidence, issues) and a failing run's `checker` receipt
/// and diagnostics (evidence and supported fixes too) equal Archify's, for every run of every
/// fixture that reached the checker.
#[test]
fn the_artifact_checker_matches_the_goldens() {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".golden.json"))
        .collect();
    paths.sort();
    let (mut passing, mut failing, mut skipped) = (0, 0, 0);
    let mut failures = Vec::new();
    let mut deviations = Deviations::open("artifact_checker");
    for path in &paths {
        let raw = fs::read_to_string(path).unwrap();
        let case: Value = serde_json::from_str(&exact_numbers(&raw)).unwrap();
        let name = case["name"].as_str().unwrap();
        let text = match (case.get("source_text"), case.get("source_doc")) {
            (Some(Value::String(text)), _) => text.clone(),
            (_, Some(doc)) => {
                let plain: Value = serde_json::from_str(&raw).unwrap();
                plain["source_doc"].to_string().clone() + &" ".repeat(doc.as_object().map_or(0, |_| 0))
            }
            _ => continue,
        };
        for (run, quality) in [("authored", None), ("standard", Some("standard")), ("showcase", Some("showcase"))] {
            let Some(want) = case["validate_receipt"][run].get("receipt") else { continue };
            let reached = want["ok"] == true || want["stage"] == "check";
            if !reached {
                skipped += 1;
                continue;
            }
            let compiled = compile(&text, &Opts { input: "<input>", doc_type: case["type"].as_str(), quality });
            let ours = as_written(&serde_json::to_value(&compiled.receipt).unwrap());
            if deviates(case["type"].as_str(), &text) {
                let mut kept = serde_json::Map::new();
                for key in ["ok", "stage", "checks", "composition", "diagnostics", "checker"] {
                    kept.insert(key.to_owned(), ours[key].clone());
                }
                failures.extend(deviations.check(&format!("{name} [{run}]"), Value::Object(kept)));
                continue;
            }
            let mut problems = Vec::new();
            if want["ok"] == true {
                passing += 1;
                diff("/checks", &ours["checks"], &want["checks"], &mut problems, 8);
                diff("/composition", &ours["composition"], &want["composition"], &mut problems, 8);
            } else {
                failing += 1;
                let mut want_diagnostics = want["diagnostics"].clone();
                strip_rule(&mut want_diagnostics);
                let mut our_diagnostics = ours["diagnostics"].clone();
                strip_rule(&mut our_diagnostics);
                diff("/diagnostics", &our_diagnostics, &want_diagnostics, &mut problems, 8);
                let (mut our_checker, mut want_checker) = (ours["checker"].clone(), want["checker"].clone());
                for checker in [&mut our_checker, &mut want_checker] {
                    if let Some(m) = checker.as_object_mut() {
                        m.remove("file");
                        m.remove("artifact");
                    }
                }
                diff("/checker", &our_checker, &want_checker, &mut problems, 8);
            }
            if !problems.is_empty() {
                failures.push(format!("{name} [{run}]\n  {}", problems.join("\n  ")));
            }
        }
    }
    println!(
        "\nartifact checker: {passing} passing and {failing} failing runs compared, {skipped} rejected before the checker, {} against the recorded deviation (D40)",
        deviations.len()
    );
    failures.extend(deviations.finish());
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.iter().take(30).cloned().collect::<Vec<_>>().join("\n"));
    assert!(passing >= 90 && failing >= 2, "{passing} passing, {failing} failing");
}
