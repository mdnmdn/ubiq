//! P9.1: rendering fidelity against Archify itself (D5), over `fixtures/fidelity_cases.json`
//! (`gen-fidelity-cases.mjs`):
//!
//! - **brand badges**: every one of the 107 marks through all five types, by id, title, alias, domain
//!   URL and padded or upper-cased text: the badge position, the vector's `d`, scale and colour, the
//!   node's `data-node-brand*` metadata, and no badge where Archify draws none;
//! - **locales**: the legend title, labels and widths and the default node context of `zh-CN`, a
//!   custom catalogue (every `examples/locales/*.json`), a partial one and an override of `en`
//!   and `zh-CN`, in all five types;
//! - **V4**: the `i18n/*` warnings, message and whole diagnostic (as Archify recorded them), and
//!   the stderr line Archify prints;
//! - **V8**: the `brand/*` errors of `validate`, the receipt's `error` and every diagnostic.
//!
//! The goldens of every type compare the same legend and context parts in their own tests, and
//! `every_golden_names_the_preset_the_scene_resolves` ties `meta.visual_preset` to `data-preset`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use ubiq_archify::brand;
use ubiq_archify::compile::{Opts, compile};
use ubiq_archify::model::{self, Doc};
use ubiq_archify::scene::{Layer, Scene, Shape};
use ubiq_archify::tokens::{Preset, Token};
use serde_json::{Value, json};

struct Fixture {
    locales: BTreeMap<String, Value>,
    cases: Vec<Value>,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fidelity_cases.json");
    let v: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let locales = v["locales"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    Fixture { locales, cases: v["cases"].as_array().unwrap().clone() }
}

/// The case's document with `"@tag"` translations expanded.
fn doc_of(f: &Fixture, case: &Value) -> Value {
    let mut doc = case["doc"].clone();
    if let Some(tag) = doc["meta"]["translations"].as_str().and_then(|s| s.strip_prefix('@')) {
        doc["meta"]["translations"] = f.locales[tag].clone();
    }
    doc
}

fn compiled(doc: &Value, ty: &str) -> ubiq_archify::compile::Compiled {
    compile(&doc.to_string(), &Opts { input: "<input>", doc_type: Some(ty), quality: None })
}

fn scene_of(c: &ubiq_archify::compile::Compiled) -> &Scene {
    c.layout.as_ref().expect("a layout").scene()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

/// Compares one rendered case; returns the number of values checked.
fn compare_render(f: &Fixture, case: &Value, report: &mut Vec<String>) -> usize {
    let name = case["name"].as_str().unwrap();
    let ty = case["type"].as_str().unwrap();
    let doc = doc_of(f, case);
    let c = compiled(&doc, ty);
    let mut checked = 0;
    let mut say = |what: String| report.push(format!("{name}: {what}"));
    if !c.ok() {
        say(format!("did not compile: {:?}", c.diagnostics().iter().map(|d| &d.message).collect::<Vec<_>>()));
        return 0;
    }
    let scene = scene_of(&c);

    // V4: the stderr lines, in order, one per warning.
    let ours: Vec<String> = c.warnings.iter().filter(|d| d.code.starts_with("i18n/")).map(|d| format!("archify: {}", d.message)).collect();
    let want: Vec<String> = case["stderr"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_owned()).collect();
    checked += 1;
    if ours != want {
        say(format!("warnings {ours:?} want {want:?}"));
    }

    // nodes: context and badge
    for n in case["nodes"].as_array().unwrap() {
        let id = n["id"].as_str().unwrap();
        let Some((gi, group)) = scene.groups.iter().enumerate().find(|(_, g)| g.id == format!("node-{id}")) else {
            say(format!("node {id}: no group"));
            continue;
        };
        checked += 1;
        if group.context.as_deref() != n["context"].as_str() {
            say(format!("node {id} context {:?} want {:?}", group.context, n["context"]));
        }
        let items: Vec<&Shape> = scene.items.iter().filter(|i| i.group == Some(ubiq_archify::scene::GroupId(gi as u32))).map(|i| &i.shape).collect();
        let plate = items.iter().find_map(|s| match s {
            Shape::Rect(r) if r.fill.is_some_and(|f| f.token == Token::BrandBadge) => Some(r),
            _ => None,
        });
        let ink = items.iter().find_map(|s| match s {
            Shape::Path(p) => match p.fill {
                Some(f) => match f.token {
                    Token::BrandInk(hex) => Some((p, hex)),
                    _ => None,
                },
                None => None,
            },
            _ => None,
        });
        match (n.get("brand").filter(|b| !b.is_null()), plate) {
            (None, None) => {}
            (Some(b), Some(plate)) => {
                checked += 6;
                let (bx, by) = (num(&b["x"]), num(&b["y"]));
                if (plate.rect.x, plate.rect.y, plate.rect.width, plate.rect.height, plate.radius) != (bx, by, 16.0, 16.0, 4.0) {
                    say(format!("node {id} plate {:?} want ({bx}, {by}, 16, 16, 4)", plate.rect));
                }
                let Some((p, hex)) = ink else {
                    say(format!("node {id}: no vector"));
                    continue;
                };
                let (s, inset) = (num(&b["scale"]), num(&b["inset"]));
                if p.transform != [s, 0.0, 0.0, s, bx + inset, by + inset] {
                    say(format!("node {id} vector transform {:?} want scale {s} at ({}, {})", p.transform, bx + inset, by + inset));
                }
                if format!("{hex:06X}") != b["hex"].as_str().unwrap().to_uppercase() {
                    say(format!("node {id} colour {hex:06x} want {}", b["hex"]));
                }
                let authored = authored_brand(&doc, ty, id);
                let Some(authored) = authored else {
                    say(format!("node {id}: no authored brand"));
                    continue;
                };
                match brand::meta(&authored) {
                    Some(m) => {
                        let (mark, title, status) = (b["mark"].as_str().unwrap(), b["title"].as_str().unwrap(), b["status"].as_str().unwrap());
                        if (m.id.as_str(), m.brand.as_str(), m.status) != (mark, title, status) || Some(m.source.as_str()) != b["source"].as_str() {
                            say(format!("node {id} metadata {m:?} want {mark} / {title} / {status} / {}", b["source"]));
                        }
                        if let Some(brand::Resolved::Preset(mark)) = brand::resolve(&authored)
                            && Some(mark.path.as_str()) != b["d"].as_str()
                        {
                            say(format!("node {id}: the embedded path of {} differs", mark.id));
                        }
                    }
                    None => say(format!("node {id}: brand does not resolve")),
                }
            }
            (want, got) => say(format!("node {id}: badge drawn {} where Archify draws {}", got.is_some(), want.is_some())),
        }
    }

    // legend
    let title = scene.items.iter().find_map(|i| match &i.shape {
        Shape::Text(t) if i.layer == Layer::Legend && i.group.is_none() => Some(t.text.as_str()),
        _ => None,
    });
    match case["legend"].as_object() {
        None => {
            checked += 1;
            if title.is_some() {
                say("legend drawn where Archify draws none".into());
            }
        }
        Some(l) => {
            checked += 1;
            if title != l["title"].as_str() {
                say(format!("legend title {title:?} want {:?}", l["title"]));
            }
            for item in l["items"].as_array().unwrap() {
                let kind = item["kind"].as_str().unwrap();
                let Some(g) = scene.groups.iter().find(|g| g.id == format!("legend-{kind}")) else {
                    say(format!("legend {kind}: no group"));
                    continue;
                };
                checked += 3;
                if Some(g.label.as_str()) != item["label"].as_str() {
                    say(format!("legend {kind} label {:?} want {:?}", g.label, item["label"]));
                }
                if (g.bounds.x, g.bounds.width) != (num(&item["x"]), num(&item["width"])) {
                    say(format!("legend {kind} x/width ({}, {}) want ({}, {})", g.bounds.x, g.bounds.width, item["x"], item["width"]));
                }
            }
        }
    }
    checked
}

/// The authored `brand` of node `id`.
fn authored_brand(doc: &Value, ty: &str, id: &str) -> Option<ubiq_archify::model::common::BrandMark> {
    let coll = match ty {
        "architecture" => "components",
        "sequence" => "participants",
        "lifecycle" => "states",
        _ => "nodes",
    };
    let node = doc[coll].as_array()?.iter().find(|n| n["id"] == id)?;
    serde_json::from_value(node["brand"].clone()).ok()
}

#[test]
fn brands_and_locales_match_archify() {
    let f = fixture();
    let mut report = Vec::new();
    let (mut checked, mut runs, mut badges) = (0, 0, 0);
    for case in f.cases.iter().filter(|c| c["kind"] == "render") {
        assert_eq!(case["ok"], true, "{}: the oracle failed to render", case["name"]);
        runs += 1;
        badges += case["nodes"].as_array().unwrap().iter().filter(|n| n.get("brand").is_some_and(|b| !b.is_null())).count();
        checked += compare_render(&f, case, &mut report);
    }
    assert!(report.is_empty(), "{checked} values checked over {runs} runs\n{}", report.join("\n"));
    assert!(runs >= 100 && badges >= 500, "{runs} runs, {badges} badges");
    println!("{runs} documents, {badges} badges, {checked} values compared");
}

#[test]
fn every_built_in_mark_is_exercised() {
    let f = fixture();
    let mut seen = std::collections::BTreeSet::new();
    for case in f.cases.iter().filter(|c| c["kind"] == "render") {
        for n in case["nodes"].as_array().unwrap() {
            if let Some(b) = n.get("brand").filter(|b| !b.is_null()) {
                seen.insert(b["mark"].as_str().unwrap().to_owned());
            }
        }
    }
    let all: std::collections::BTreeSet<String> = brand::marks().iter().map(|m| m.id.clone()).collect();
    assert_eq!(seen, all);
}

/// V4 and V8 together: the failing run Archify recorded (`diagnostics` = the `i18n/*` warnings, then
/// the `brand/unknown` error) against our warnings and receipt.
#[test]
fn v4_warnings_and_the_v8_error_match_the_recorded_diagnostics() {
    let f = fixture();
    let (mut runs, mut warned) = (0, 0);
    let mut report = Vec::new();
    for case in f.cases.iter().filter(|c| c["kind"] == "diag") {
        let name = case["name"].as_str().unwrap();
        let ty = case["type"].as_str().unwrap();
        let mut doc = doc_of(&f, case);
        let coll = match ty {
            "architecture" => "components",
            "sequence" => "participants",
            "lifecycle" => "states",
            _ => "nodes",
        };
        doc[coll][0]["brand"] = json!("zzz-not-a-brand");
        let c = compiled(&doc, ty);
        runs += 1;
        let want: Vec<&Value> = case["diagnostics"].as_array().unwrap().iter().collect();
        let (want_warn, want_err): (Vec<&Value>, Vec<&Value>) = want.into_iter().partition(|d| d["severity"] == "warning");
        let got_warn: Vec<Value> = c.warnings.iter().map(|d| serde_json::to_value(d).unwrap()).collect();
        let got_err: Vec<Value> = c.receipt.diagnostics.iter().map(|d| serde_json::to_value(d).unwrap()).collect();
        warned += want_warn.len();
        if got_warn.iter().collect::<Vec<_>>() != want_warn {
            report.push(format!("{name}: warnings\n  got  {got_warn:?}\n  want {want_warn:?}"));
        }
        if got_err.iter().collect::<Vec<_>>() != want_err {
            report.push(format!("{name}: errors\n  got  {got_err:?}\n  want {want_err:?}"));
        }
        if Some(case["error"].as_str().unwrap()) != c.receipt.error.as_deref() {
            report.push(format!("{name}: error {:?} want {:?}", c.receipt.error, case["error"]));
        }
        if c.stages_run.last() != Some(&"V8") {
            report.push(format!("{name}: stopped after {:?}", c.stages_run));
        }
    }
    assert!(report.is_empty(), "{runs} runs\n{}", report.join("\n"));
    assert!(runs >= 50 && warned >= 40, "{runs} runs, {warned} warnings");
}

#[test]
fn brand_failures_of_validate_match_the_receipts() {
    let f = fixture();
    let mut report = Vec::new();
    let mut runs = 0;
    for case in f.cases.iter().filter(|c| c["kind"] == "validate") {
        let name = case["name"].as_str().unwrap();
        let ty = case["type"].as_str().unwrap();
        let c = compiled(&case["doc"], ty);
        runs += 1;
        let want = &case["receipt"];
        let got = serde_json::to_value(&c.receipt).unwrap();
        for key in ["ok", "stage", "error", "diagnostics"] {
            if got[key] != want[key] {
                report.push(format!("{name}: {key}\n  got  {}\n  want {}", got[key], want[key]));
            }
        }
        if c.receipt.exit_code() != case["exit_code"].as_i64().unwrap() as i32 {
            report.push(format!("{name}: exit {} want {}", c.receipt.exit_code(), case["exit_code"]));
        }
    }
    assert!(report.is_empty(), "{runs} runs\n{}", report.join("\n"));
    assert!(runs >= 20, "{runs}");
}

/// `data-preset` of every golden that carries one is the preset the document asks for.
#[test]
fn every_golden_names_the_preset_the_scene_resolves() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let (mut seen, mut non_classic) = (0, 0);
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.to_string_lossy().ends_with(".golden.json") {
            continue;
        }
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let Some(want) = v["render"]["svg_attrs"]["data-preset"].as_str() else { continue };
        let Some(ty) = v["type"].as_str() else { continue };
        let Ok(doc) = model::parse(ty, &v["source_doc"]) else { continue };
        let got = Preset::of(doc.visual_preset());
        assert_eq!(got.as_str(), want, "{}", path.display());
        seen += 1;
        non_classic += usize::from(!got.is_classic());
    }
    assert!(seen >= 30 && non_classic >= 8, "{seen} goldens, {non_classic} non-classic");
}

/// A document opens in the preset it names; the interface restyles the scene for it.
#[test]
fn restyle_applies_the_frame_style_and_the_grid() {
    use ubiq_archify::tokens::{frame_style, restyle};
    let doc = json!({"schema_version": 1, "diagram_type": "architecture",
        "meta": {"title": "t", "output": "a.html", "visual_preset": "blueprint"},
        "components": [{"id": "a", "type": "backend", "label": "A", "pos": [60, 60], "size": [130, 70]}],
        "boundaries": [{"kind": "region", "label": "R", "wraps": ["a"]}],
        "connections": []});
    let c = compiled(&doc, "architecture");
    assert!(c.ok(), "{:?}", c.diagnostics());
    let mut scene = scene_of(&c).clone();
    let before = scene.items.len();
    let preset = Preset::of(Doc::from_value(&doc).unwrap().visual_preset());
    assert_eq!(preset, Preset::Blueprint);
    restyle(&mut scene, preset);
    assert_eq!(scene.preset, Preset::Blueprint);
    assert_eq!(scene.items.len(), before + 1, "the grid is one path after the ground");
    let grid = &scene.items[1];
    assert_eq!(grid.layer, Layer::Background);
    assert!(matches!(&grid.shape, Shape::Path(p) if p.stroke.as_ref().is_some_and(|s| s.dash == [1.0, 3.0] && s.token == Token::Grid) && p.opacity == 0.78));
    let region = scene.items.iter().find_map(|i| match &i.shape {
        Shape::Rect(r) if i.layer == Layer::Frames => r.stroke.as_ref().map(|s| s.dash.clone()),
        _ => None,
    });
    assert_eq!(region.as_deref(), frame_style(Preset::Blueprint).region_dash);

    // Classic hides the grid and changes nothing.
    let mut classic = scene_of(&c).clone();
    restyle(&mut classic, Preset::Classic);
    assert_eq!(classic.items, scene_of(&c).items);
}
