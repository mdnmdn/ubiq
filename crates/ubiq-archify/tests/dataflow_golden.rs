//! P2.4: the dataflow layout against Archify's own render (D5). For every dataflow golden that has a
//! render (the two examples and the two `v1-baseline` fixtures), `layout::dataflow::build` must give,
//! with no tolerance (`==` on `f64`, the numbers Archify prints are shortest round-trip doubles):
//! the viewBox, the stage frames and headers, the node rects and text positions, the edge
//! composition points (and their `d`), the label rects and text, and the legend. Every diff is
//! collected and reported before the test fails.

use std::fs;
use std::path::PathBuf;

use ubiq_archify::geom::polyline_path;
use ubiq_archify::layout::dataflow::{DataflowGeometry, build};
use ubiq_archify::model::{self, Doc};
use ubiq_archify::scene::{Group, Scene, Shape, TextShape};
use serde_json::Value;

struct Case {
    name: String,
    doc: Value,
    render: Value,
}

fn cases() -> Vec<Case> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out: Vec<Case> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if !name.ends_with(".dataflow.golden.json") {
                return None;
            }
            let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let render = v.get("render").filter(|r| r.get("nodes").is_some())?.clone();
            Some(Case { name, doc: v["source_doc"].clone(), render })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
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

fn texts_of<'a>(scene: &'a Scene, group: &Group) -> Vec<&'a TextShape> {
    let id = scene.groups.iter().position(|g| g.id == group.id).map(|i| ubiq_archify::scene::GroupId(i as u32)).unwrap();
    scene
        .items_of(id)
        .filter_map(|i| match &i.shape {
            Shape::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn group<'a>(scene: &'a Scene, id: &str) -> Option<&'a Group> {
    scene.groups.iter().find(|g| g.id == id)
}

fn numbers_in(s: &str) -> Vec<f64> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap())
        .collect()
}

fn compare(d: &mut Diffs, case: &Case, scene: &Scene, g: &DataflowGeometry) {
    let r = &case.render;

    // viewBox
    let vb = r["viewBox"].as_array().unwrap();
    d.nums("viewBox", &g.view_box, &[num(&vb[2]), num(&vb[3])]);
    d.nums("scene viewBox", &scene.view_box, &[num(&vb[2]), num(&vb[3])]);

    // stage frames and headers
    let frames = r["frames"].as_array().unwrap();
    if d.count("frames", g.frames.len(), frames.len()) {
        for (i, f) in frames.iter().enumerate() {
            let got = g.frames[i];
            d.nums(&format!("frame {i}"), &[got.x, got.y, got.width, got.height], &rect_of(&f["rect"]));
        }
    }
    let headers: Vec<&Value> = r["decor"]["Data Stages"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    if d.count("headers", headers.len(), frames.len()) {
        for (i, h) in headers.iter().enumerate() {
            let fg = group(scene, &format!("frame-stage-{i}")).expect("frame group");
            let ts = texts_of(scene, fg);
            if d.count(&format!("header {i} lines"), ts.len(), 1) {
                d.shape_text(&format!("header {i}"), ts[0], h);
            }
        }
    }

    // nodes
    let nodes = r["nodes"].as_array().unwrap();
    if d.count("nodes", g.nodes.len(), nodes.len()) {
        for (i, n) in nodes.iter().enumerate() {
            let id = n["id"].as_str().unwrap();
            let got = &g.nodes[i];
            d.text(&format!("node {i} id"), &got.id, id);
            d.nums(
                &format!("node {id} rect"),
                &[got.rect.x, got.rect.y, got.rect.width, got.rect.height],
                &rect_of(&n["rect"]),
            );
            let ng = group(scene, &format!("node-{id}")).expect("node group");
            d.text(&format!("node {id} context"), ng.context.as_deref().unwrap_or(""), n["context"].as_str().unwrap());
            d.text(&format!("node {id} kind"), ng.node_kind.map_or("", |k| k.as_str()), n["kind"].as_str().unwrap());
            let ts = texts_of(scene, ng);
            let want = n["texts"].as_array().unwrap();
            if d.count(&format!("node {id} texts"), ts.len(), want.len()) {
                for (t, w) in ts.iter().zip(want) {
                    d.shape_text(&format!("node {id}"), t, w);
                }
            }
            if let Some(tr) = n["sigil"]["transform"].as_str() {
                let want = numbers_in(tr);
                let gid = ubiq_archify::scene::GroupId(scene.groups.iter().position(|x| x.id == ng.id).unwrap() as u32);
                let path = scene.items_of(gid).find_map(|i| match &i.shape {
                    Shape::Path(p) => Some(p),
                    _ => None,
                });
                match path {
                    Some(p) => d.nums(&format!("node {id} sigil"), &[p.transform[4], p.transform[5], p.transform[0]], &want),
                    None => d.lines.push(format!("node {id}: no sigil path, want {tr}")),
                }
            }
        }
    }

    // edges and labels
    let edges = r["edges"].as_array().unwrap();
    let labels = r["labels"].as_array().unwrap();
    if d.count("edges", g.flows.len(), edges.len()) && d.count("labels", g.flows.len(), labels.len()) {
        for (i, e) in edges.iter().enumerate() {
            let got = &g.flows[i];
            let want: Vec<f64> = e["points"].as_array().unwrap().iter().flat_map(|p| [num(&p[0]), num(&p[1])]).collect();
            let have: Vec<f64> = got.points.iter().flat_map(|p| [p[0], p[1]]).collect();
            d.nums(&format!("edge {i} points"), &have, &want);
            d.text(&format!("edge {i} d"), &polyline_path(&got.points), e["d"].as_str().unwrap());
            if let Some(Shape::Polyline(p)) = scene
                .items_of(ubiq_archify::scene::GroupId(
                    scene.groups.iter().position(|x| x.id == format!("edge-{i}")).unwrap() as u32,
                ))
                .map(|i| &i.shape)
                .next()
            {
                d.num(&format!("edge {i} stroke width"), p.stroke.width, num(&e["stroke_width"]));
                d.nums(
                    &format!("edge {i} scene points"),
                    &p.points.iter().flat_map(|q| [q[0], q[1]]).collect::<Vec<_>>(),
                    &want,
                );
            } else {
                d.lines.push(format!("edge {i}: no polyline"));
            }

            let l = &labels[i];
            d.text(&format!("label {i} text"), case.doc["flows"][i]["label"].as_str().unwrap_or(""), l["text"].as_str().unwrap());
            let rc = got.label.rect;
            d.nums(&format!("label {i} rect"), &[rc.x, rc.y, rc.width, rc.height], &rect_of(&l["rect"]));
            let lg = group(scene, &format!("label-{i}")).expect("label group");
            let ts = texts_of(scene, lg);
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
            let title = model::parse("dataflow", &case.doc).unwrap().locale().legend_title();
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
                    let lg = group(scene, &format!("legend-{kind}")).expect("legend group");
                    let gid = ubiq_archify::scene::GroupId(scene.groups.iter().position(|x| x.id == lg.id).unwrap() as u32);
                    let shapes = w["shapes"].as_array().unwrap();
                    let items: Vec<&Shape> = scene.items_of(gid).map(|i| &i.shape).collect();
                    if d.count(&format!("legend {kind} shapes"), items.len(), shapes.len()) {
                        for (s, ws) in items.iter().zip(shapes) {
                            match (s, ws["tag"].as_str().unwrap()) {
                                (Shape::Polyline(pl), "path") => {
                                    d.text(&format!("legend {kind} swatch"), &polyline_path(&pl.points), ws["d"].as_str().unwrap())
                                }
                                (Shape::Rect(rc), "rect") => d.nums(
                                    &format!("legend {kind} swatch"),
                                    &[rc.rect.x, rc.rect.y, rc.rect.width, rc.rect.height, rc.radius],
                                    &[num(&ws["x"]), num(&ws["y"]), num(&ws["width"]), num(&ws["height"]), num(&ws["rx"])],
                                ),
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

#[test]
fn dataflow_layout_matches_archify() {
    let cases = cases();
    assert!(cases.len() >= 4, "expected the two examples and two v1-baseline fixtures, found {}", cases.len());
    let mut report = Vec::new();
    let mut checked = 0;
    for case in &cases {
        let Doc::Dataflow(doc) = model::parse("dataflow", &case.doc).unwrap_or_else(|e| panic!("{}: {e}", case.name)) else {
            panic!("{}: not a dataflow", case.name)
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
    assert!(checked > 800, "only {checked} values were compared");
    assert!(report.is_empty(), "{} diffs against Archify:\n{}", report.len(), report.join("\n"));
}

#[test]
fn every_dataflow_negative_builds_or_reports_without_panicking() {
    // The negatives have no render to compare; `build` must give a scene or diagnostics, never panic.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut seen = 0;
    for e in fs::read_dir(&dir).unwrap() {
        let path = e.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !(name.starts_with("neg.") && name.ends_with(".golden.json")) {
            continue;
        }
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if v["type"] != "dataflow" {
            continue;
        }
        if let Ok(Doc::Dataflow(doc)) = model::parse("dataflow", &v["source_doc"]) {
            let _ = build(&doc);
            seen += 1;
        }
    }
    assert!(seen >= 5, "only {seen} dataflow negatives were built");
}
