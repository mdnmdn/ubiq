//! P4.1: the sequence layout against Archify's own render (D5). For every sequence golden that has a
//! render (the two examples and the `v1-baseline` fixture), `layout::sequence::build` must give, with
//! no tolerance (`==` on `f64`, the numbers Archify prints are shortest round-trip doubles): the
//! viewBox and column fit, the participant rects and text, the lifelines, the activations, the
//! segment frames and their (bumped) labels, the message points (and their `d` and dash), the label
//! rects and text, and the legend. Every diff is collected and reported before the test fails.
//!
//! Also here: the gates agree with the layout (a document that builds passes `gates::sequence` at
//! every quality, as the goldens say), and the negatives never panic.

use std::fs;
use std::path::PathBuf;

use ubiq_archify::compile::{Layout, Opts, compile};
use ubiq_archify::gates::{Gate, sequence as gates};
use ubiq_archify::geom::polyline_path;
use ubiq_archify::layout::sequence::{SequenceGeometry, build};
use ubiq_archify::model::sequence::ColumnFit;
use ubiq_archify::model::{self, Doc};
use ubiq_archify::scene::{Group, GroupId, Layer, Scene, Shape, TextShape};
use ubiq_archify::tokens::{Kind, Token};
use serde_json::Value;

struct Case {
    name: String,
    doc: Value,
    render: Value,
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Quote every non-integer number outside a string, so that `serde_json` (whose default float parser
/// is off by one ulp on some shortest round-trip decimals: `91.19999999999999` reads as `91.2`)
/// hands them over as text and [`num`] parses them exactly.
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
            // Multi-byte characters are copied byte by byte below through `push_str` of the slice.
            let width = text[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[i..i + width]);
            i += width;
            continue;
        }
        if c == b'"' {
            in_string = true;
        }
        if c == b'-' || c.is_ascii_digit() {
            let end = (i..bytes.len()).find(|&j| !matches!(bytes[j], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')).unwrap_or(bytes.len());
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

fn cases() -> Vec<Case> {
    let mut out: Vec<Case> = fs::read_dir(fixtures())
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if !name.ends_with(".sequence.golden.json") {
                return None;
            }
            let text = fs::read_to_string(&path).unwrap();
            let v: Value = serde_json::from_str(&text).unwrap();
            let exact: Value = serde_json::from_str(&quote_floats(&text)).unwrap();
            let render = exact.get("render").filter(|r| r.get("nodes").is_some())?.clone();
            Some(Case { name, doc: v["source_doc"].clone(), render })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// An integer, or a float that [`quote_floats`] quoted and `str::parse` reads exactly.
fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) => s.parse().unwrap_or_else(|_| panic!("not a number: {s}")),
        other => other.as_f64().unwrap_or_else(|| panic!("not a number: {other}")),
    }
}

fn rect_of(v: &Value) -> [f64; 4] {
    [num(&v["x"]), num(&v["y"]), num(&v["width"]), num(&v["height"])]
}

/// Collects every disagreement of one fixture.
struct Diffs {
    name: String,
    lines: Vec<String>,
    checked: usize,
}

impl Diffs {
    fn num(&mut self, what: &str, got: f64, want: f64) {
        self.checked += 1;
        if got != want {
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

    fn count(&mut self, what: &str, got: usize, want: usize) -> bool {
        self.checked += 1;
        if got != want {
            self.lines.push(format!("{what}: got {got} want {want}"));
        }
        got == want
    }

    fn shape_text(&mut self, what: &str, got: &TextShape, want: &Value) {
        self.text(&format!("{what} text"), &got.text, want["text"].as_str().unwrap_or(""));
        self.num(&format!("{what} x"), got.at[0], num(&want["x"]));
        self.num(&format!("{what} y"), got.at[1], num(&want["y"]));
        self.num(&format!("{what} font_size"), got.size, num(&want["font_size"]));
    }
}

fn gid(scene: &Scene, id: &str) -> Option<GroupId> {
    scene.groups.iter().position(|g| g.id == id).map(|i| GroupId(i as u32))
}

fn group<'a>(scene: &'a Scene, id: &str) -> Option<&'a Group> {
    scene.groups.iter().find(|g| g.id == id)
}

fn texts_of<'a>(scene: &'a Scene, id: &str) -> Vec<&'a TextShape> {
    let id = gid(scene, id).unwrap_or_else(|| panic!("no group {id}"));
    scene
        .items_of(id)
        .filter_map(|i| match &i.shape {
            Shape::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn numbers_in(s: &str) -> Vec<f64> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap())
        .collect()
}

fn dash_of(dash: &[f64]) -> String {
    dash.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",")
}

fn scene_title_y(scene: &Scene) -> f64 {
    scene
        .items
        .iter()
        .find_map(|i| match &i.shape {
            Shape::Text(t) if t.text == "Legend" => Some(t.at[1]),
            _ => None,
        })
        .unwrap_or(f64::NAN)
}

fn compare(d: &mut Diffs, case: &Case, scene: &Scene, g: &SequenceGeometry) {
    let r = &case.render;

    // viewBox and column fit
    let vb = r["viewBox"].as_array().unwrap();
    d.nums("viewBox", &g.view_box, &[num(&vb[2]), num(&vb[3])]);
    d.nums("scene viewBox", &scene.view_box, &[num(&vb[2]), num(&vb[3])]);
    let fit = if g.column_fit == ColumnFit::Spread { "spread" } else { "fixed" };
    d.text("column fit", fit, r["svg_attrs"]["data-sequence-column-fit"].as_str().unwrap());

    // participants
    let nodes = r["nodes"].as_array().unwrap();
    if d.count("participants", g.participants.len(), nodes.len()) {
        for (i, n) in nodes.iter().enumerate() {
            let id = n["id"].as_str().unwrap();
            let got = &g.participants[i];
            d.text(&format!("participant {i} id"), &got.id, id);
            d.nums(&format!("participant {id} rect"), &[got.rect.x, got.rect.y, got.rect.width, got.rect.height], &rect_of(&n["rect"]));
            let ng = group(scene, &format!("node-{id}")).expect("node group");
            d.text(&format!("participant {id} context"), ng.context.as_deref().unwrap_or(""), n["context"].as_str().unwrap());
            d.text(&format!("participant {id} kind"), ng.node_kind.map_or("", |k| k.as_str()), n["kind"].as_str().unwrap());
            let ts = texts_of(scene, &ng.id);
            let want = n["texts"].as_array().unwrap();
            if d.count(&format!("participant {id} texts"), ts.len(), want.len()) {
                for (t, w) in ts.iter().zip(want) {
                    d.shape_text(&format!("participant {id}"), t, w);
                }
            }
            if let Some(tr) = n["sigil"]["transform"].as_str() {
                let want = numbers_in(tr);
                let path = scene.items_of(gid(scene, &ng.id).unwrap()).find_map(|i| match &i.shape {
                    Shape::Path(p) => Some(p),
                    _ => None,
                });
                match path {
                    Some(p) => d.nums(&format!("participant {id} sigil"), &[p.transform[4], p.transform[5], p.transform[0]], &want),
                    None => d.lines.push(format!("participant {id}: no sigil path, want {tr}")),
                }
            }
        }
    }

    // lifelines: ungrouped dashed polylines in the frame layer
    let lifelines: Vec<&Shape> = scene
        .items
        .iter()
        .filter(|i| i.layer == Layer::Frames && i.group.is_none() && matches!(&i.shape, Shape::Polyline(_)))
        .map(|i| &i.shape)
        .collect();
    let want = r["decor"]["Lifelines"].as_array().unwrap();
    if d.count("lifelines", lifelines.len(), want.len()) {
        for (i, (s, w)) in lifelines.iter().zip(want).enumerate() {
            let Shape::Polyline(p) = s else { unreachable!() };
            d.text(&format!("lifeline {i} d"), &polyline_path(&p.points), w["d"].as_str().unwrap());
            d.text(&format!("lifeline {i} dash"), &dash_of(&p.stroke.dash), w["dash"].as_str().unwrap());
            d.num(&format!("lifeline {i} end"), p.points[1][1], g.lifeline_end);
        }
    }

    // activations: a mask rect, then the kind-filled rect, per bar
    let bars: Vec<&ubiq_archify::scene::RectShape> = scene
        .items
        .iter()
        .filter(|i| i.layer == Layer::Frames && i.group.is_none())
        .filter_map(|i| match &i.shape {
            Shape::Rect(r) => Some(r),
            _ => None,
        })
        .collect();
    let want = r["decor"]["Activations"].as_array().unwrap();
    if d.count("activation rects", bars.len(), want.len()) && d.count("activations", g.activations.len() * 2, want.len()) {
        for (i, (b, w)) in bars.iter().zip(want).enumerate() {
            d.nums(
                &format!("activation rect {i}"),
                &[b.rect.x, b.rect.y, b.rect.width, b.rect.height, b.radius],
                &[num(&w["x"]), num(&w["y"]), num(&w["width"]), num(&w["height"]), num(&w["rx"])],
            );
            let class = w["class"].as_str().unwrap();
            let token = b.fill.map(|f| f.token);
            let expect = match class {
                "c-mask" => Some(Token::Mask),
                other => Kind::parse(&other[2..]).map(Token::KindFill),
            };
            d.checked += 1;
            if token != expect {
                d.lines.push(format!("activation rect {i}: fill {token:?} want {class}"));
            }
        }
        let rects = &g.activations;
        for (i, rc) in rects.iter().enumerate() {
            d.nums(&format!("activation {i}"), &[rc.x, rc.y, rc.width, rc.height], &rect_of(&want[i * 2]));
        }
    }

    // segments: the frame and its label
    let frames = r["frames"].as_array().unwrap();
    let labels_want = r["decor"]["Segment Labels"].as_array().unwrap();
    if d.count("segments", g.segments.len(), frames.len()) && d.count("segment label shapes", labels_want.len(), frames.len() * 2) {
        for (i, f) in frames.iter().enumerate() {
            let got = &g.segments[i];
            d.nums(&format!("segment {i} frame"), &[got.frame.x, got.frame.y, got.frame.width, got.frame.height], &rect_of(&f["rect"]));
            let lw = &labels_want[i * 2];
            d.nums(
                &format!("segment {i} label"),
                &[got.label.x, got.label.y, got.label.width, got.label.height],
                &rect_of(lw),
            );
            let ts = texts_of(scene, &format!("frame-segment-{i}"));
            if d.count(&format!("segment {i} texts"), ts.len(), 1) {
                d.shape_text(&format!("segment {i}"), ts[0], &labels_want[i * 2 + 1]);
            }
        }
    }

    // messages and labels
    let edges = r["edges"].as_array().unwrap();
    let labels = r["labels"].as_array().unwrap();
    if d.count("edges", g.messages.len(), edges.len()) && d.count("labels", g.messages.len(), labels.len()) {
        for (i, e) in edges.iter().enumerate() {
            let got = &g.messages[i];
            let want: Vec<f64> = e["points"].as_array().unwrap().iter().flat_map(|p| [num(&p[0]), num(&p[1])]).collect();
            let have: Vec<f64> = got.points.iter().flat_map(|p| [p[0], p[1]]).collect();
            d.nums(&format!("message {i} points"), &have, &want);
            d.text(&format!("message {i} d"), &polyline_path(&got.points), e["d"].as_str().unwrap());
            let edge = scene.items_of(gid(scene, &format!("edge-{i}")).unwrap()).map(|i| &i.shape).next();
            if let Some(Shape::Polyline(p)) = edge {
                d.num(&format!("message {i} stroke width"), p.stroke.width, num(&e["stroke_width"]));
                // Only `return` writes `stroke-dasharray`; the security and dashed dashes are CSS.
                let (got, want) = (dash_of(&p.stroke.dash), e["dash"].as_str().unwrap_or(""));
                if want.is_empty() && matches!(got.as_str(), "5,5" | "4,4") {
                    d.checked += 1;
                } else {
                    d.text(&format!("message {i} dash"), &got, want);
                }
            } else {
                d.lines.push(format!("message {i}: no polyline"));
            }

            let l = &labels[i];
            d.text(&format!("label {i} text"), case.doc["messages"][i]["label"].as_str().unwrap_or(""), l["text"].as_str().unwrap());
            let rc = got.label;
            d.nums(&format!("label {i} rect"), &[rc.x, rc.y, rc.width, rc.height], &rect_of(&l["rect"]));
            let ts = texts_of(scene, &format!("label-{i}"));
            let want = l["texts"].as_array().unwrap();
            if d.count(&format!("label {i} texts"), ts.len(), want.len()) {
                for (t, w) in ts.iter().zip(want) {
                    d.shape_text(&format!("label {i}"), t, w);
                }
            }
        }
    }

    // legend
    match (&g.legend, r.get("legend").filter(|l| !l.is_null())) {
        (None, None) => {}
        (Some(m), Some(l)) => {
            let title = model::parse("sequence", &case.doc).unwrap().locale().legend_title();
            d.text("legend title", &title, l["title"].as_str().unwrap());
            d.num("legend title y", m.title_y, scene_title_y(scene));
            d.text("legend bridge", &m.has_interactive().to_string(), &l["bridge"].as_bool().unwrap().to_string());
            let items = l["items"].as_array().unwrap();
            if d.count("legend items", m.entries.len(), items.len()) {
                for (p, w) in m.entries.iter().zip(items) {
                    let kind = p.entry.kind;
                    d.text("legend kind", kind, w["semantic_kind"].as_str().unwrap());
                    d.num(&format!("legend {kind} x"), p.x, num(&w["x"]));
                    d.num(&format!("legend {kind} baseline"), p.baseline, num(&w["baseline"]));
                    d.num(&format!("legend {kind} width"), p.width, num(&w["width"]));
                    let id = gid(scene, &format!("legend-{kind}")).expect("legend group");
                    let shapes = w["shapes"].as_array().unwrap();
                    let items: Vec<&Shape> = scene.items_of(id).map(|i| &i.shape).collect();
                    if d.count(&format!("legend {kind} shapes"), items.len(), shapes.len()) {
                        for (s, ws) in items.iter().zip(shapes) {
                            match (s, ws["tag"].as_str().unwrap()) {
                                (Shape::Polyline(pl), "path") => {
                                    d.text(&format!("legend {kind} swatch"), &polyline_path(&pl.points), ws["d"].as_str().unwrap());
                                    let (got, want) = (dash_of(&pl.stroke.dash), ws["dash"].as_str().unwrap_or(""));
                                    if want.is_empty() && matches!(got.as_str(), "5,5" | "4,4") {
                                        d.checked += 1;
                                    } else {
                                        d.text(&format!("legend {kind} dash"), &got, want);
                                    }
                                }
                                (Shape::Text(t), "text") => d.shape_text(&format!("legend {kind}"), t, ws),
                                (other, tag) => d.lines.push(format!("legend {kind}: scene {other:?} vs golden {tag}")),
                            }
                        }
                    }
                }
            }
        }
        (got, want) => d.lines.push(format!("legend: got {} want {}", got.is_some(), want.is_some())),
    }
}

#[test]
fn sequence_layout_matches_archify() {
    let cases = cases();
    assert!(cases.len() >= 3, "expected the two examples and the v1-baseline fixture, found {}", cases.len());
    let mut report = Vec::new();
    let mut checked = 0;
    for case in &cases {
        let Doc::Sequence(doc) = model::parse("sequence", &case.doc).unwrap_or_else(|e| panic!("{}: {e}", case.name)) else {
            panic!("{}: not a sequence", case.name)
        };
        let mut d = Diffs { name: case.name.clone(), lines: Vec::new(), checked: 0 };
        match build(&doc) {
            Ok((scene, geometry)) => compare(&mut d, case, &scene, &geometry),
            Err(errs) => d.lines.push(format!("build failed: {errs:?}")),
        }
        checked += d.checked;
        println!("{}: {} values compared, {} diffs", d.name, d.checked, d.lines.len());
        for line in &d.lines {
            report.push(format!("{}: {line}", d.name));
        }
    }
    assert!(checked > 500, "only {checked} values were compared");
    assert!(report.is_empty(), "{} diffs against Archify:\n{}", report.len(), report.join("\n"));
}

#[test]
fn the_rendered_goldens_pass_the_gates_and_compile_to_a_sequence_layout() {
    for case in cases() {
        let Doc::Sequence(doc) = model::parse("sequence", &case.doc).unwrap() else { panic!() };
        for gate in [Gate(None), Gate(Some("standard")), Gate(Some("showcase"))] {
            let (outcome, _) = gates::validate(&doc, gate);
            assert!(outcome.ok(), "{} under {gate:?}: {:?}", case.name, outcome.problems);
        }
        let compiled = compile(&case.doc.to_string(), &Opts { input: "x.sequence.json", ..Opts::default() });
        assert!(compiled.ok(), "{}: {:?}", case.name, compiled.diagnostics());
        assert!(matches!(compiled.layout, Some(Layout::Sequence { .. })), "{}", case.name);
        assert_eq!(compiled.receipt.composition.as_ref().unwrap()["status"], "pass", "{}", case.name);
    }
}

#[test]
fn every_sequence_negative_builds_or_reports_without_panicking() {
    // The negatives have no render to compare; both halves must give an answer, never panic.
    let mut seen = 0;
    for e in fs::read_dir(fixtures()).unwrap() {
        let path = e.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !(name.starts_with("neg.") && name.ends_with(".golden.json")) {
            continue;
        }
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if v["type"] != "sequence" {
            continue;
        }
        if let Ok(Doc::Sequence(doc)) = model::parse("sequence", &v["source_doc"]) {
            let _ = build(&doc);
            for gate in [Gate(None), Gate(Some("showcase"))] {
                let _ = gates::validate(&doc, gate);
            }
            seen += 1;
        }
    }
    assert!(seen >= 5, "only {seen} sequence negatives were built");
}
