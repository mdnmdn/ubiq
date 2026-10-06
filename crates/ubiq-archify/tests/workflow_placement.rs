//! P6.2: the workflow v2 rank solver against Archify (D5), with no tolerance (`==` on `f64`).
//!
//! - Every `readable-v2` golden that laid out (exit 0): the receipt's `columns` and node rects, the
//!   rendered lane / exception-lane / group frames, the group label positions and the phase headers
//!   (line, mask, text x), under the feedback Archify's router settled on (recorded by the oracle).
//! - The `node-overlap` golden: the whole failure (message, subject, evidence rects).
//! - `tests/workflow_cases.json` (`gen-workflow-cases.mjs`): random documents, each compared
//!   on Archify's own layout object (`createReadableLayout`: columns, lane width/heights/gap, group
//!   reserves, channel-label edges, width/height contributors), plus the node rects and frames when
//!   it rendered, the `node-overlap` failure when it threw one, and the layout under a synthetic
//!   router feedback for a share of the cases. The fixture goldens' layout objects are compared too.
//!
//! Every diff is collected and reported before the test fails.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use ubiq_archify::layout::workflow::readable::{LayoutFeedback, ReadableLayout, ReadablePlacement, place};
use ubiq_archify::model::workflow::Workflow;
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// Quote every non-integer number outside a string, so that `serde_json` (whose default float parser
/// is off by one ulp on some shortest round-trip decimals) hands them over as text and [`num`] parses
/// them exactly. Same as `tests/sequence_golden.rs`.
fn quote_floats(text: &str) -> String {
    let (bytes, mut out, mut i, mut in_string) = (text.as_bytes(), String::with_capacity(text.len() + 4096), 0, false);
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                out.push(c as char);
                i += 1;
            } else if c == b'"' {
                in_string = false;
            }
            let width = text[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[i..i + width]);
            i += width;
            continue;
        }
        if c == b'"' {
            in_string = true;
        }
        if c == b'-' || c.is_ascii_digit() {
            let end = (i..bytes.len())
                .find(|&j| !matches!(bytes[j], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
                .unwrap_or(bytes.len());
            let token = &text[i..end];
            if token.contains(['.', 'e', 'E']) {
                out.push_str(&format!("\"{token}\""));
            } else {
                out.push_str(token);
            }
            i = end;
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// Read a file twice: plainly (documents) and with quoted floats (expectations).
fn load(path: &PathBuf) -> (Value, Value) {
    let text = fs::read_to_string(path).unwrap();
    (serde_json::from_str(&text).unwrap(), serde_json::from_str(&quote_floats(&text)).unwrap())
}

fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) => s.parse().unwrap_or_else(|_| panic!("not a number: {s}")),
        Value::Null => f64::NAN,
        other => other.as_f64().unwrap_or_else(|| panic!("not a number: {other}")),
    }
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array().map(|a| a.iter().map(num).collect()).unwrap_or_default()
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(|s| s.as_str().unwrap_or_default().to_owned()).collect()).unwrap_or_default()
}

fn feedback_of(v: &Value) -> LayoutFeedback {
    let mut f = LayoutFeedback::default();
    if let Some(map) = v["rankGapMinimums"].as_object() {
        f.rank_gap_minimums = map.iter().map(|(k, m)| (k.clone(), num(m))).collect::<BTreeMap<_, _>>();
    }
    if let Some(map) = v["rankGapContributors"].as_object() {
        f.rank_gap_contributors = map.iter().map(|(k, c)| (k.clone(), strs(c))).collect();
    }
    if !v["laneGapMin"].is_null() {
        f.lane_gap_min = Some(num(&v["laneGapMin"]));
    }
    f.lane_gap_contributors = strs(&v["laneGapContributors"]);
    f
}

struct Diffs {
    name: String,
    lines: Vec<String>,
    checked: usize,
}

impl Diffs {
    fn new(name: &str) -> Self {
        Diffs { name: name.to_owned(), lines: Vec::new(), checked: 0 }
    }

    fn num(&mut self, what: &str, got: f64, want: f64) {
        self.checked += 1;
        if got != want && !(got.is_nan() && want.is_nan()) {
            self.lines.push(format!("{what}: got {got} want {want}"));
        }
    }

    fn nums(&mut self, what: &str, got: &[f64], want: &[f64]) {
        if got.len() != want.len() {
            self.lines.push(format!("{what}: got {got:?} want {want:?}"));
            return;
        }
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            self.num(&format!("{what}[{i}]"), *g, *w);
        }
    }

    fn text(&mut self, what: &str, got: &str, want: &str) {
        self.checked += 1;
        if got != want {
            self.lines.push(format!("{what}: got {got:?} want {want:?}"));
        }
    }

    fn texts(&mut self, what: &str, got: &[String], want: &[String]) {
        self.checked += 1;
        if got != want {
            self.lines.push(format!("{what}: got {got:?} want {want:?}"));
        }
    }

    fn count(&mut self, what: &str, got: usize, want: usize) -> bool {
        self.checked += 1;
        if got != want {
            self.lines.push(format!("{what}: got {got} want {want}"));
        }
        got == want
    }

    fn rect(&mut self, what: &str, got: &ubiq_archify::geom::Rect, want: &Value) {
        self.num(&format!("{what}.x"), got.x, num(&want["x"]));
        self.num(&format!("{what}.y"), got.y, num(&want["y"]));
        self.num(&format!("{what}.width"), got.width, num(&want["width"]));
        self.num(&format!("{what}.height"), got.height, num(&want["height"]));
    }
}

/// Archify's layout object (as the oracle prints it) against ours.
fn compare_layout(d: &mut Diffs, what: &str, got: &ReadableLayout, want: &Value) {
    d.nums(&format!("{what}.colXs"), &got.col_xs, &nums(&want["colXs"]));
    d.num(&format!("{what}.laneW"), got.lane_w, num(&want["laneW"]));
    d.num(&format!("{what}.laneH"), got.lane_h, num(&want["laneH"]));
    d.nums(&format!("{what}.laneHeights"), &got.lane_heights, &nums(&want["laneHeights"]));
    d.num(&format!("{what}.laneGap"), got.lane_gap, num(&want["laneGap"]));
    d.nums(&format!("{what}.groupHeaderHeights"), &got.group_header_heights, &nums(&want["groupHeaderHeights"]));
    d.nums(&format!("{what}.groupFooterHeights"), &got.group_footer_heights, &nums(&want["groupFooterHeights"]));
    d.num(&format!("{what}.defaultViewBoxWidth"), got.default_view_box_width, num(&want["defaultViewBoxWidth"]));
    let channels: Vec<f64> =
        got.channel_label_edges.iter().enumerate().filter(|(_, c)| **c).map(|(i, _)| i as f64).collect();
    d.nums(&format!("{what}.channelLabelEdges"), &channels, &nums(&want["channelLabelEdges"]));
    d.texts(&format!("{what}.widthContributors"), &got.width_contributors, &strs(&want["widthContributors"]));
    d.texts(&format!("{what}.heightContributors"), &got.height_contributors, &strs(&want["heightContributors"]));
}

/// The receipt's `nodes` (canonical order).
fn compare_nodes(d: &mut Diffs, p: &ReadablePlacement, want: &Value) {
    let want = want.as_array().cloned().unwrap_or_default();
    if !d.count("nodes", p.nodes.len(), want.len()) {
        return;
    }
    for (got, want) in p.nodes.iter().zip(&want) {
        let what = format!("node {}", got.id);
        d.text(&format!("{what}.id"), &got.id, want["id"].as_str().unwrap_or_default());
        d.text(&format!("{what}.lane"), &got.lane, want["lane"].as_str().unwrap_or_default());
        d.num(&format!("{what}.col"), got.col, num(&want["col"]));
        d.rect(&what, &got.rect, want);
    }
}

/// The rendered frames (lanes, exception lanes, groups), phase headers and group labels.
fn compare_render(d: &mut Diffs, p: &ReadablePlacement, frames: &Value, phases: &Value, groups: &Value) {
    let mut expected: Vec<(String, String, &ubiq_archify::geom::Rect)> = Vec::new();
    for lane in &p.lanes {
        expected.push(("lane".into(), format!("lane-{}", lane.index), &lane.rect));
        if let Some(exception) = &lane.exception {
            expected.push(("exception-lane".into(), format!("lane-{}-exception", lane.index), exception));
        }
    }
    for group in &p.groups {
        expected.push(("group".into(), format!("group-{}", group.index), &group.rect));
    }
    let frames = frames.as_array().cloned().unwrap_or_default();
    if d.count("frames", expected.len(), frames.len()) {
        for ((kind, id, rect), want) in expected.iter().zip(&frames) {
            d.text(&format!("{id}.kind"), kind, want["kind"].as_str().unwrap_or_default());
            d.text(&format!("{id}.id"), id, want["id"].as_str().unwrap_or_default());
            d.rect(id, rect, &want["rect"]);
        }
    }
    // Phase headers: line, mask rect, text, per phase.
    let phases = phases.as_array().cloned().unwrap_or_default();
    if d.count("phase shapes", p.phases.len() * 3, phases.len()) {
        for (phase, shapes) in p.phases.iter().zip(phases.chunks(3)) {
            let what = format!("phase {}", phase.id);
            let s = phase.span;
            d.num(&format!("{what}.line.x1"), s.x, num(&shapes[0]["x1"]));
            d.num(&format!("{what}.line.x2"), s.x + s.width, num(&shapes[0]["x2"]));
            d.num(&format!("{what}.line.y"), 35.0, num(&shapes[0]["y1"]));
            d.num(&format!("{what}.mask.x"), s.x, num(&shapes[1]["x"]));
            d.num(&format!("{what}.mask.y"), 27.0, num(&shapes[1]["y"]));
            d.num(&format!("{what}.mask.width"), s.width, num(&shapes[1]["width"]));
            d.num(&format!("{what}.mask.height"), 16.0, num(&shapes[1]["height"]));
            d.num(&format!("{what}.text.x"), s.cx, num(&shapes[2]["x"]));
            d.num(&format!("{what}.text.y"), 39.0, num(&shapes[2]["y"]));
            d.text(&format!("{what}.text"), &phase.label, shapes[2]["text"].as_str().unwrap_or_default());
        }
    }
    let groups = groups.as_array().cloned().unwrap_or_default();
    if d.count("group labels", p.groups.len(), groups.len()) {
        for (group, want) in p.groups.iter().zip(&groups) {
            let what = format!("group {} label", group.id);
            d.num(&format!("{what}.x"), group.label_x, num(&want["x"]));
            d.num(&format!("{what}.y"), group.label_y, num(&want["y"]));
            d.text(&format!("{what}.text"), &group.label, want["text"].as_str().unwrap_or_default());
        }
    }
}

/// A thrown `node-overlap` failure: error text, and per diagnostic code, message, subject and
/// evidence (fixes need the whole compile, P6.4).
fn compare_overlap(d: &mut Diffs, p: &ReadablePlacement, error: &str, diagnostics: &Value) {
    let Some(failure) = p.geometry_failure(|_, _| false) else {
        d.lines.push("geometry_failure: got none, want node-overlap".into());
        return;
    };
    d.text("error", &failure.error, error);
    let want = diagnostics.as_array().cloned().unwrap_or_default();
    if !d.count("overlap diagnostics", failure.diagnostics.len(), want.len()) {
        return;
    }
    for (i, (got, want)) in failure.diagnostics.iter().zip(&want).enumerate() {
        let what = format!("overlap[{i}]");
        d.text(&format!("{what}.code"), &got.code, want["code"].as_str().unwrap_or_default());
        d.text(&format!("{what}.message"), &got.message, want["message"].as_str().unwrap_or_default());
        d.text(
            &format!("{what}.subject.node"),
            got.subject.extra.get("node").and_then(Value::as_str).unwrap_or_default(),
            want["subject"]["node"].as_str().unwrap_or_default(),
        );
        d.text(
            &format!("{what}.subject.path"),
            got.subject.path.as_deref().unwrap_or_default(),
            want["subject"]["path"].as_str().unwrap_or_default(),
        );
        let (ge, we) = (&got.evidence, &want["evidence"]);
        d.text(&format!("{what}.lane"), ge["lane"].as_str().unwrap_or_default(), we["lane"].as_str().unwrap_or_default());
        for k in 0..2 {
            let (gn, wn) = (&ge["nodes"][k], &we["nodes"][k]);
            d.text(&format!("{what}.nodes[{k}].id"), gn["id"].as_str().unwrap_or_default(), wn["id"].as_str().unwrap_or_default());
            for f in ["x", "y", "width", "height"] {
                d.num(&format!("{what}.nodes[{k}].rect.{f}"), num(&gn["rect"][f]), num(&wn["rect"][f]));
            }
        }
    }
}

fn report(all: Vec<Diffs>, label: &str) {
    let checked: usize = all.iter().map(|d| d.checked).sum();
    let failing: Vec<&Diffs> = all.iter().filter(|d| !d.lines.is_empty()).collect();
    println!("{label}: {} documents, {checked} values, {} with diffs", all.len(), failing.len());
    if !failing.is_empty() {
        let mut text = String::new();
        for d in &failing {
            text.push_str(&format!("\n== {} ({} diffs)\n", d.name, d.lines.len()));
            for line in d.lines.iter().take(12) {
                text.push_str(&format!("  {line}\n"));
            }
        }
        panic!("{label}: {} of {} documents differ:{text}", failing.len(), all.len());
    }
}

fn cases() -> (Value, Value) {
    load(&root().join("workflow_cases.json"))
}

#[test]
fn goldens_match_the_readable_receipts() {
    let (_, oracle) = cases();
    let mut all = Vec::new();
    let mut names: Vec<PathBuf> = fs::read_dir(root().join("fixtures"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".golden.json"))
        .collect();
    names.sort();
    for path in names {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (plain, exact) = load(&path);
        if plain["type"] != "workflow" || exact["layout_json"]["receipt"]["contract"] != "readable-v2" {
            continue;
        }
        let Ok(workflow) = serde_json::from_value::<Workflow>(plain["source_doc"].clone()) else { continue };
        let entry = &oracle["fixtures"][&name];
        let p = place(&workflow, &feedback_of(&entry["feedback"]));
        let mut d = Diffs::new(&name);
        if !entry.is_null() {
            compare_layout(&mut d, "layout", &p.layout, &entry["layout"]);
        }
        let receipt = &exact["layout_json"]["receipt"];
        if receipt["nodes"].is_array() {
            d.nums("columns", &p.layout.col_xs, &nums(&receipt["columns"]));
            compare_nodes(&mut d, &p, &receipt["nodes"]);
            let render = &exact["render"];
            compare_render(&mut d, &p, &render["frames"], &render["decor"]["Phase headers"], &render["decor"]["Workflow groups"]);
        }
        let diagnostics = &receipt["diagnostics"];
        if diagnostics.as_array().is_some_and(|a| a.iter().any(|x| x["code"] == "workflow/node-overlap")) {
            let error: Vec<&str> = diagnostics.as_array().unwrap().iter().filter_map(|x| x["message"].as_str()).collect();
            let error = if error.len() == 1 { error[0].to_owned() } else { format!("Workflow node overlap:\n- {}", error.join("\n- ")) };
            compare_overlap(&mut d, &p, &error, diagnostics);
        }
        all.push(d);
    }
    assert!(all.len() >= 8, "expected the readable-v2 goldens, found {}", all.len());
    report(all, "workflow v2 goldens");
}

#[test]
fn random_corpus_matches_archify() {
    let (plain, exact) = cases();
    let docs = plain["cases"].as_array().unwrap();
    let wants = exact["cases"].as_array().unwrap();
    let mut all = Vec::new();
    for (doc_case, want) in docs.iter().zip(wants) {
        let name = want["name"].as_str().unwrap_or_default();
        let mut d = Diffs::new(name);
        let workflow: Workflow = match serde_json::from_value(doc_case["doc"].clone()) {
            Ok(w) => w,
            Err(e) => {
                d.lines.push(format!("doc does not parse: {e}"));
                all.push(d);
                continue;
            }
        };
        let p = place(&workflow, &feedback_of(&want["feedback"]));
        compare_layout(&mut d, "layout", &p.layout, &want["layout"]);
        if want["nodes"].is_array() {
            compare_nodes(&mut d, &p, &want["nodes"]);
        }
        if want["render"].is_object() {
            let r = &want["render"];
            compare_render(&mut d, &p, &r["frames"], &r["phases"], &r["groups"]);
        }
        if want["overlap"].is_object() {
            compare_overlap(&mut d, &p, want["overlap"]["error"].as_str().unwrap_or_default(), &want["overlap"]["diagnostics"]);
        } else if want["ok"] == true {
            d.checked += 1;
            if let Some(f) = p.geometry_failure(|_, _| false) {
                d.lines.push(format!("geometry_failure on a document Archify laid out: {}", f.error));
            }
        }
        if want["synthetic"].is_object() {
            let s = place(&workflow, &feedback_of(&want["synthetic"]["feedback"]));
            compare_layout(&mut d, "synthetic", &s.layout, &want["synthetic"]["layout"]);
        }
        all.push(d);
    }
    assert!(all.len() >= 100, "corpus too small: {}", all.len());
    report(all, "workflow v2 corpus");
}
