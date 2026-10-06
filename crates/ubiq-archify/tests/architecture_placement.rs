//! P3.1: architecture placement against Archify's own `--layout-json` and render (D5). For every
//! architecture example and fixture, `layout::architecture` must give, with no tolerance (`==` on
//! `f64`; the render's numbers are shortest round-trip doubles, the layout receipt's are
//! `Math.round`ed): component rects, boundary frames, boundary title masks and fonts, the viewBox
//! and the legend rows; plus the node scene (texts, sigil, context).
//!
//! The viewBox of an auto canvas covers every route point and connection label rect, and the title
//! convergence measures that width, so it depends on routes that P3.3 produces. The test feeds the
//! oracle's own routes (`layout_json.connections[].points`) and the default label anchors
//! (`labelPoint` on those routes, which is what `connectionGeometry` holds before the showcase
//! relocation) through [`Prepared::finish`]; the legend is measured against the oracle's routes and
//! final label rects. A second, route-free pass (`place`) lists the documents whose canvas the
//! routes do not move.

use std::fs;
use std::path::PathBuf;

use ubiq_archify::diag::js_round;
use ubiq_archify::geom::{Pt, Rect};
use ubiq_archify::labels::{Hints, label_point};
use ubiq_archify::layout::architecture::{ArchitecturePlacement, begin_scene, grid_problems, prepare, push_components, push_legend};
use ubiq_archify::legend::{Obstacle, relationship_obstacles};
use ubiq_archify::model::architecture::Architecture;
use ubiq_archify::scene::{Group, GroupId, Scene, Shape, TextShape};
use ubiq_archify::text::{self, DiagramType, Profile};
use serde_json::Value;

struct Case {
    name: String,
    arch: Architecture,
    layout: Value,
    render: Value,
}

fn cases() -> Vec<Case> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out: Vec<Case> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if !name.ends_with(".architecture.golden.json") || name.starts_with("neg.") {
                return None;
            }
            let text = fs::read_to_string(&path).unwrap();
            let v: Value = serde_json::from_str(&exact_numbers(&text)).unwrap();
            let layout = v.pointer("/layout_json/receipt").filter(|r| r.get("components").is_some())?.clone();
            let render = v.get("render").filter(|r| r.get("nodes").is_some())?.clone();
            let plain: Value = serde_json::from_str(&text).unwrap();
            let arch: Architecture = serde_json::from_value(plain["source_doc"].clone()).unwrap();
            Some(Case { name, arch, layout, render })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// serde_json's default float parser is not correctly rounded (it is up to 1 ulp off, e.g. it reads
/// `123.39999999999999` as `123.4`), and the oracle must be exact. Every number token outside a
/// string becomes the string `"n:<token>"`, which [`num`] parses with `str::parse`.
fn exact_numbers(text: &str) -> String {
    let token = regex::Regex::new(r#""(?:[^"\\]|\\.)*"|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)"#).unwrap();
    token
        .replace_all(text, |caps: &regex::Captures| match caps.get(1) {
            Some(n) => format!("\"n:{}\"", n.as_str()),
            None => caps[0].to_owned(),
        })
        .into_owned()
}

fn num(v: &Value) -> f64 {
    match v.as_str().and_then(|s| s.strip_prefix("n:")) {
        Some(n) => n.parse().unwrap_or_else(|_| panic!("not a number: {v}")),
        None => v.as_f64().unwrap_or_else(|| panic!("not a number: {v}")),
    }
}

fn rect_of(v: &Value) -> [f64; 4] {
    [num(&v["x"]), num(&v["y"]), num(&v["width"]), num(&v["height"])]
}

fn arr(r: &Rect) -> [f64; 4] {
    [r.x, r.y, r.width, r.height]
}

struct Diffs {
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
}

/// Routes, default label rects and final label rects of the oracle, aligned with `arch.connections`
/// (the receipt lists only connections whose endpoints exist, in order).
struct Routes {
    points: Vec<Vec<Pt>>,
    /// `connectionLabelBoxAt(conn, labelPoint(conn, points))`, the box before any relocation.
    default_labels: Vec<Option<Rect>>,
    /// The rect the render draws (after showcase relocation).
    final_labels: Vec<Option<Rect>>,
}

fn label_width(label: &str) -> f64 {
    text::edge_label_width(DiagramType::Architecture, &[label], Profile::Standard)
}

fn routes_of(c: &Case) -> Routes {
    let known: Vec<&str> = c.arch.components.iter().map(|k| k.id.as_str()).collect();
    let conns = c.arch.connections.as_deref().unwrap_or(&[]);
    let kept: Vec<_> = conns.iter().filter(|k| known.contains(&k.from.as_str()) && known.contains(&k.to.as_str())).collect();
    let receipt = c.layout["connections"].as_array().unwrap();
    assert_eq!(kept.len(), receipt.len(), "{}: connections", c.name);
    let edges = c.render["edges"].as_array().unwrap();
    let labels = c.render["labels"].as_array().unwrap();
    let mut out = Routes { points: Vec::new(), default_labels: Vec::new(), final_labels: Vec::new() };
    for (i, (conn, r)) in kept.iter().zip(receipt).enumerate() {
        let points: Vec<Pt> = r["points"].as_array().unwrap().iter().map(|p| [num(&p[0]), num(&p[1])]).collect();
        let hints = Hints { at: conn.label_at, dx: conn.label_dx, dy: conn.label_dy, segment: conn.label_segment };
        let label = conn.label.as_deref().filter(|l| !l.is_empty());
        out.default_labels.push(label.map(|l| {
            let [lx, ly] = label_point(&hints, &points);
            let w = label_width(l);
            Rect::new(lx - w / 2.0, ly - 10.0, w, 14.0)
        }));
        let key = edges.get(i).map_or(i as u64, |e| num(&e["key"]) as u64);
        out.final_labels.push(label.and_then(|_| {
            labels.iter().find(|l| num(&l["edge_key"]) as u64 == key).map(|l| {
                let [x, y, w, h] = rect_of(&l["rect"]);
                Rect::new(x, y, w, h)
            })
        }));
        out.points.push(points);
    }
    out
}

fn extra_rects(routes: &Routes) -> Vec<Rect> {
    let mut extra: Vec<Rect> = routes.default_labels.iter().flatten().copied().collect();
    extra.extend(routes.points.iter().flatten().map(|p| Rect::new(p[0], p[1], 0.0, 0.0)));
    extra
}

fn obstacles(routes: &Routes) -> Vec<Obstacle> {
    relationship_obstacles(routes.points.iter().map(|p| p.as_slice()).zip(routes.final_labels.iter().copied()))
}

fn group_texts<'a>(scene: &'a Scene, id: &str) -> Vec<&'a TextShape> {
    let Some(i) = scene.groups.iter().position(|g: &Group| g.id == id) else { return Vec::new() };
    scene
        .items_of(GroupId(i as u32))
        .filter_map(|it| match &it.shape {
            Shape::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn compare(d: &mut Diffs, c: &Case, p: &ArchitecturePlacement, routes: &Routes) {
    let r = &c.render;

    // viewBox, render and receipt
    let vb = r["viewBox"].as_array().unwrap();
    d.nums("render viewBox", &p.view_box, &[num(&vb[2]), num(&vb[3])]);
    let lvb = c.layout["viewBox"].as_array().unwrap();
    d.nums("receipt viewBox", &p.view_box, &[num(&lvb[0]), num(&lvb[1])]);
    d.text("readability", p.readability_problem.as_deref().unwrap_or(""), "");

    // components: the render's un-rounded rects, the receipt's rounded x/y
    let nodes = r["nodes"].as_array().unwrap();
    if d.count("components", p.components.len(), nodes.len()) {
        for n in nodes {
            let id = n["id"].as_str().unwrap();
            let Some(got) = p.component(id) else {
                d.lines.push(format!("component {id}: missing"));
                continue;
            };
            d.nums(&format!("component {id}"), &arr(&got.rect), &rect_of(&n["rect"]));
        }
    }
    let receipt = c.layout["components"].as_array().unwrap();
    if d.count("receipt components", p.components.len(), receipt.len()) {
        for (got, want) in p.components.iter().zip(receipt) {
            d.text("receipt component id", &got.id, want["id"].as_str().unwrap());
            d.nums(
                &format!("receipt component {}", got.id),
                &[js_round(got.rect.x), js_round(got.rect.y), got.rect.width, got.rect.height],
                &[num(&want["x"]), num(&want["y"]), num(&want["width"]), num(&want["height"])],
            );
        }
    }

    // boundaries: frames, receipt, title masks and fonts
    let frames = r["frames"].as_array().unwrap();
    let boundaries = c.layout["boundaries"].as_array().unwrap();
    let titles: Vec<&Value> = r["decor"]["Boundary labels"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    if d.count("frames", p.boundaries.len(), frames.len())
        && d.count("receipt boundaries", p.boundaries.len(), boundaries.len())
        && d.count("title shapes", titles.len(), 2 * frames.len())
    {
        for (i, b) in p.boundaries.iter().enumerate() {
            d.text(&format!("frame {i} label"), &b.raw.label, frames[i]["label"].as_str().unwrap());
            d.nums(&format!("frame {i}"), &arr(&b.rect), &rect_of(&frames[i]["rect"]));
            d.nums(
                &format!("receipt boundary {i}"),
                &[js_round(b.rect.x), js_round(b.rect.y), js_round(b.rect.width), js_round(b.rect.height)],
                &[num(&boundaries[i]["x"]), num(&boundaries[i]["y"]), num(&boundaries[i]["width"]), num(&boundaries[i]["height"])],
            );
            let (mask, label) = (titles[2 * i], titles[2 * i + 1]);
            d.nums(&format!("title {i} mask"), &arr(&b.title.rect), &rect_of(mask));
            let at = b.title.text_at();
            d.nums(&format!("title {i} text"), &[at[0], at[1], b.title.font_size], &[num(&label["x"]), num(&label["y"]), num(&label["font_size"])]);
        }
    }

    // legend rows
    let legend = p.measure_legend(&obstacles(routes));
    let want = r.get("legend").filter(|l| !l.is_null());
    match (&legend, want) {
        (Ok(Some(m)), Some(w)) => {
            let items = w["items"].as_array().unwrap();
            if d.count("legend entries", m.entries.len(), items.len()) {
                for (got, item) in m.entries.iter().zip(items) {
                    d.text("legend kind", got.entry.kind, item["semantic_kind"].as_str().unwrap());
                    // The label in the document's locale (`ja` and `ko` catalogues included).
                    if let Some(label) = item["label"].as_str() {
                        d.text(&format!("legend {} label", got.entry.kind), &got.entry.label, label);
                    }
                    d.nums(
                        &format!("legend {}", got.entry.kind),
                        &[got.x, got.baseline, got.width],
                        &[num(&item["x"]), num(&item["baseline"]), num(&item["width"])],
                    );
                }
            }
        }
        (Ok(None), None) => {}
        (got, want) => d.lines.push(format!("legend: got {:?} want {:?}", got.as_ref().map(|m| m.is_some()), want.is_some())),
    }
}

fn compare_scene(d: &mut Diffs, c: &Case, p: &ArchitecturePlacement) {
    let mut b = begin_scene(p);
    push_components(&mut b, &c.arch, p);
    push_legend(&mut b, p.measure_legend(&[]).ok().flatten().as_ref(), &c.arch.meta.locale());
    let scene = b.build();
    d.nums("scene viewBox", &scene.view_box, &p.view_box);
    for n in c.render["nodes"].as_array().unwrap() {
        let id = n["id"].as_str().unwrap();
        let gid = format!("node-{id}");
        let Some(g) = scene.groups.iter().find(|g| g.id == gid) else {
            d.lines.push(format!("node {id}: no group"));
            continue;
        };
        d.text(&format!("node {id} context"), g.context.as_deref().unwrap_or(""), n["context"].as_str().unwrap());
        // Brand: the `data-node-brand*` metadata and the badge (plate, vector, frame) after the sigil.
        let component = c.arch.components.iter().find(|k| k.id == id);
        let meta = component.and_then(|k| k.brand.as_ref()).and_then(ubiq_archify::brand::meta);
        d.text(&format!("node {id} brand"), meta.as_ref().map_or("", |m| m.brand.as_str()), n["data"]["data-node-brand"].as_str().unwrap_or(""));
        d.text(&format!("node {id} brand id"), meta.as_ref().map_or("", |m| m.id.as_str()), n["data"]["data-node-brand-id"].as_str().unwrap_or(""));
        d.text(&format!("node {id} brand status"), meta.as_ref().map_or("", |m| m.status), n["data"]["data-node-brand-status"].as_str().unwrap_or(""));
        d.text(&format!("node {id} brand source"), meta.as_ref().map_or("", |m| m.source.as_str()), n["data"]["data-node-brand-source"].as_str().unwrap_or(""));
        let badge = scene
            .items_of(GroupId(scene.groups.iter().position(|g| g.id == gid).unwrap() as u32))
            .filter(|i| matches!(&i.shape, Shape::Rect(r) if r.fill.is_some_and(|f| f.token == ubiq_archify::tokens::Token::BrandBadge)))
            .count();
        d.count(&format!("node {id} badges"), badge, usize::from(meta.is_some()));
        let want = n["texts"].as_array().unwrap();
        let got = group_texts(&scene, &gid);
        if d.count(&format!("node {id} texts"), got.len(), want.len()) {
            for (t, w) in got.iter().zip(want) {
                d.text(&format!("node {id} text"), &t.text, w["text"].as_str().unwrap());
                d.nums(&format!("node {id} {}", t.text), &[t.at[0], t.at[1], t.size], &[num(&w["x"]), num(&w["y"]), num(&w["font_size"])]);
            }
        }
        // Sigil: `translate(x y) scale(s)`.
        if let Some(sigil) = n.get("sigil").filter(|s| !s.is_null()) {
            let nums: Vec<f64> = sigil["transform"]
                .as_str()
                .unwrap()
                .split(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-'))
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().unwrap())
                .collect();
            let first = scene.items_of(GroupId(scene.groups.iter().position(|g| g.id == gid).unwrap() as u32)).find_map(|i| match &i.shape {
                Shape::Path(p) => Some(p.transform),
                _ => None,
            });
            match first {
                Some(t) => d.nums(&format!("node {id} sigil"), &[t[4], t[5], t[0]], &nums),
                None => d.lines.push(format!("node {id}: no sigil")),
            }
        }
    }
}

#[test]
fn architecture_placement_matches_archify() {
    let all = cases();
    assert_eq!(all.len(), 9, "the 5 examples (web-app, production-deployment, brand-aware-delivery, checkout-platform x2) and 4 fixtures");
    let mut report = Vec::new();
    let mut checked = 0;
    for c in &all {
        let routes = routes_of(c);
        let p = prepare(&c.arch).finish(&extra_rects(&routes));
        let mut d = Diffs { lines: Vec::new(), checked: 0 };
        compare(&mut d, c, &p, &routes);
        compare_scene(&mut d, c, &p);
        checked += d.checked;
        if !d.lines.is_empty() {
            report.push(format!("{} ({} diffs of {}):\n  {}", c.name, d.lines.len(), d.checked, d.lines.join("\n  ")));
        }
    }
    assert!(report.is_empty(), "{checked} values checked\n{}", report.join("\n"));
}

/// The canvases that route points and labels do not move: `place` with no routes gives Archify's.
/// The others (listed in `NEEDS_ROUTES`) are a P3.3 consequence, not a placement diff.
#[test]
fn route_free_canvas_where_routes_do_not_move_it() {
    let mut moved = Vec::new();
    for c in cases() {
        let p = prepare(&c.arch).finish(&[]);
        let vb = c.render["viewBox"].as_array().unwrap();
        let want = [num(&vb[2]), num(&vb[3])];
        if p.view_box != want {
            moved.push(format!("{} (got {:?} want {:?})", c.name, p.view_box, want));
        }
    }
    let names: Vec<&str> = moved.iter().map(|m| m.split(' ').next().unwrap()).collect();
    assert_eq!(names, NEEDS_ROUTES, "{moved:#?}");
}

/// Filled in from the first run; see the worklog.
const NEEDS_ROUTES: &[&str] = &[];

/// 90 random documents run through Archify (`gen-placement-cases.mjs`): grid and free
/// placement, nested and overlapping boundaries, profiles, authored canvases, wrapping legends. The
/// receipt prints rounded components and frames and the final viewBox, so those are compared (frames
/// with `Math.round`), with the receipt's own routes fed in as `connectionGeometry`.
#[test]
fn random_documents_match_the_receipts() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/placement_cases.json");
    let text = fs::read_to_string(path).unwrap();
    let v: Value = serde_json::from_str(&exact_numbers(&text)).unwrap();
    let plain: Value = serde_json::from_str(&text).unwrap();
    let cases = v.as_array().unwrap();
    assert!(cases.len() >= 80, "{} cases", cases.len());
    let mut report = Vec::new();
    let mut titled = 0;
    for (case, raw) in cases.iter().zip(plain.as_array().unwrap()) {
        let name = case["name"].as_str().unwrap();
        let arch: Architecture = serde_json::from_value(raw["doc"].clone()).unwrap();
        let r = &case["receipt"];
        let known: Vec<&str> = arch.components.iter().map(|k| k.id.as_str()).collect();
        let conns = arch.connections.as_deref().unwrap_or(&[]);
        let kept: Vec<_> = conns.iter().filter(|k| known.contains(&k.from.as_str()) && known.contains(&k.to.as_str())).collect();
        let receipt_conns = r["connections"].as_array().unwrap();
        assert_eq!(kept.len(), receipt_conns.len(), "{name}");
        let mut extra: Vec<Rect> = Vec::new();
        for (conn, rc) in kept.iter().zip(receipt_conns) {
            let points: Vec<Pt> = rc["points"].as_array().unwrap().iter().map(|p| [num(&p[0]), num(&p[1])]).collect();
            if let Some(l) = conn.label.as_deref().filter(|l| !l.is_empty()) {
                let hints = Hints { at: conn.label_at, dx: conn.label_dx, dy: conn.label_dy, segment: conn.label_segment };
                let [lx, ly] = label_point(&hints, &points);
                let w = label_width(l);
                extra.push(Rect::new(lx - w / 2.0, ly - 10.0, w, 14.0));
            }
            extra.extend(points.iter().map(|p| Rect::new(p[0], p[1], 0.0, 0.0)));
        }
        let p = prepare(&arch).finish(&extra);
        let mut d = Diffs { lines: Vec::new(), checked: 0 };
        let vb = r["viewBox"].as_array().unwrap();
        d.nums("viewBox", &p.view_box, &[num(&vb[0]), num(&vb[1])]);
        let comps = r["components"].as_array().unwrap();
        if d.count("components", p.components.len(), comps.len()) {
            for (got, want) in p.components.iter().zip(comps) {
                d.nums(
                    &format!("component {}", got.id),
                    &[js_round(got.rect.x), js_round(got.rect.y), got.rect.width, got.rect.height],
                    &[num(&want["x"]), num(&want["y"]), num(&want["width"]), num(&want["height"])],
                );
            }
        }
        let bounds = r["boundaries"].as_array().unwrap();
        if d.count("boundaries", p.boundaries.len(), bounds.len()) {
            for (i, (got, want)) in p.boundaries.iter().zip(bounds).enumerate() {
                d.nums(
                    &format!("boundary {i}"),
                    &[js_round(got.rect.x), js_round(got.rect.y), js_round(got.rect.width), js_round(got.rect.height)],
                    &[num(&want["x"]), num(&want["y"]), num(&want["width"]), num(&want["height"])],
                );
                if got.rect != got.raw.rect {
                    titled += 1;
                }
            }
        }
        if p.readability_problem.is_some() {
            d.lines.push("readability problem".to_owned());
        }
        if !d.lines.is_empty() {
            report.push(format!("{name} ({} diffs):\n  {}", d.lines.len(), d.lines.join("\n  ")));
        }
    }
    assert!(report.is_empty(), "{}", report.join("\n"));
    assert!(titled >= 10, "only {titled} boundaries had a title-extended or widened frame");
}

#[test]
fn grid_documents_in_the_corpus_have_no_placement_problems() {
    for c in cases() {
        assert!(grid_problems(&c.arch).is_empty(), "{}", c.name);
    }
}
