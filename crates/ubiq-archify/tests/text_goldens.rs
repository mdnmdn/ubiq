//! P2.1: the text model against Archify's own renders. For every golden that has a render, the font
//! size of each node's label, sublabel and tag is what `text::fit` computes, the label position
//! is what `node_label_layout` computes, and every edge-label box has the width of
//! `edge_label_width`. The validation model is checked on the `~Npx` of Archify's own
//! `geom/label-too-wide` messages.

use std::fs;
use std::path::PathBuf;

use ubiq_archify::diag::js_round;
use ubiq_archify::text::{
    DiagramType, LabelBox, Profile, Row, Side, edge_label_width, fit, label_fit_width,
    label_too_wide, node_fonts, node_label_layout, units, validation_label,
};
use serde_json::Value;

fn goldens() -> Vec<(String, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out: Vec<(String, Value)> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            name.ends_with(".golden.json").then(|| {
                let v = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
                (name, v)
            })
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Every successful render of a case: the main one and the per-quality ones.
fn renders(case: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    if let Some(r) = case.get("render") {
        out.push(r);
    }
    if let Some(map) = case.get("render_by_quality").and_then(Value::as_object) {
        out.extend(map.values());
    }
    out.retain(|r| r.get("nodes").is_some());
    out
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn str_of(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn ty_of(case: &Value) -> DiagramType {
    DiagramType::parse(str_of(&case["type"])).unwrap()
}

#[test]
fn node_font_sizes_follow_fit() {
    let mut checked = 0;
    for (name, case) in goldens() {
        let ty = ty_of(&case);
        let version = case["schema_version"].as_u64().unwrap_or(1) as u32;
        let fonts = node_fonts(ty, version);
        for render in renders(&case) {
            for node in render["nodes"].as_array().unwrap() {
                let width = num(&node["rect"]["width"]);
                let brand = node["data"].get("data-node-brand").is_some();
                let label = str_of(&node["label"]);
                let texts = node["texts"].as_array().unwrap();
                let at = |role: &str| texts.iter().find(|t| t["role"] == role);
                let label_text = at("label").unwrap_or_else(|| panic!("{name}: no label text"));
                let want = fit(label, label_fit_width(width, brand), fonts.label.0, fonts.label.1);
                assert_eq!(num(&label_text["font_size"]), want, "{name} {label:?} label");
                checked += 1;
                if let Some(sub) = node.get("sublabel").and_then(Value::as_str) {
                    let t = texts
                        .iter()
                        .find(|t| t["detail"] == "context" && str_of(&t["text"]) == sub)
                        .unwrap_or_else(|| panic!("{name}: no sublabel text {sub:?}"));
                    let want = fit(sub, width, fonts.sublabel.0, fonts.sublabel.1);
                    assert_eq!(num(&t["font_size"]), want, "{name} {sub:?} sublabel");
                    checked += 1;
                }
                if let (Some(tag), Some(sizes)) = (node.get("tag").and_then(Value::as_str), fonts.tag) {
                    let t = texts
                        .iter()
                        .find(|t| t["detail"] == "fine" && str_of(&t["text"]) == tag)
                        .unwrap_or_else(|| panic!("{name}: no tag text {tag:?}"));
                    let want = fit(tag, width, sizes.0, sizes.1);
                    assert_eq!(num(&t["font_size"]), want, "{name} {tag:?} tag");
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 600, "only {checked} font sizes were compared");
}

#[test]
fn label_layout_follows_node_label_layout() {
    let mut checked = 0;
    for (name, case) in goldens() {
        let ty = ty_of(&case);
        if !matches!(ty, DiagramType::Dataflow | DiagramType::Lifecycle | DiagramType::Workflow) {
            continue;
        }
        let version = case["schema_version"].as_u64().unwrap_or(1) as u32;
        let fonts = node_fonts(ty, version);
        // Baselines of the three rows relative to the node's top: (label, sublabel, tag from bottom).
        let (y0, y1, tag_up) = match (ty, version) {
            (DiagramType::Workflow, _) => (21.0, 38.0, 12.0),
            (DiagramType::Lifecycle, 2) => (23.0, 40.0, 12.0),
            _ => (21.0, 37.0, 11.0),
        };
        for render in renders(&case) {
            for node in render["nodes"].as_array().unwrap() {
                let rect = &node["rect"];
                let (x, y, width, height) =
                    (num(&rect["x"]), num(&rect["y"]), num(&rect["width"]), num(&rect["height"]));
                let brand = node["data"].get("data-node-brand").is_some();
                let label = str_of(&node["label"]);
                let sub = node.get("sublabel").and_then(Value::as_str).filter(|s| !s.is_empty());
                let tag = node.get("tag").and_then(Value::as_str).filter(|s| !s.is_empty());
                let font = |text: &str, sizes: (f64, f64), w: f64| fit(text, w, sizes.0, sizes.1);
                let mut rows = vec![Row {
                    text: label,
                    font: font(label, fonts.label, label_fit_width(width, brand)),
                    y: y0,
                }];
                if let Some(s) = sub {
                    rows.push(Row { text: s, font: font(s, fonts.sublabel, width), y: y1 });
                }
                if let Some(t) = tag {
                    rows.push(Row { text: t, font: font(t, fonts.tag.unwrap(), width), y: height - tag_up });
                }
                let texts = node["texts"].as_array().unwrap();
                let step = texts
                    .iter()
                    .find(|t| t["font_weight"] == "700")
                    .map(|t| str_of(&t["text"]))
                    .unwrap_or("");
                let got = node_label_layout(
                    &LabelBox { width, height, side: Side::Left, brand, source: false, step },
                    &rows,
                );
                let find = |text: &str, detail: Option<&str>| {
                    texts
                        .iter()
                        .find(|t| str_of(&t["text"]) == text && detail.is_none_or(|d| t["detail"] == d))
                        .unwrap_or_else(|| panic!("{name}: no text {text:?}"))
                };
                let label_text = texts.iter().find(|t| t["role"] == "label").unwrap();
                assert!(close(num(&label_text["x"]) - x, got.x), "{name} {label:?} x {} vs {}", num(&label_text["x"]) - x, got.x);
                assert!(close(num(&label_text["y"]) - y, got.ys[0]), "{name} {label:?} y");
                let mut next = 1;
                if let Some(s) = sub {
                    let t = find(s, Some("context"));
                    assert!(close(num(&t["y"]) - y, got.ys[next]), "{name} {s:?} y");
                    next += 1;
                }
                if let Some(t) = tag {
                    let tt = find(t, Some("fine"));
                    assert!(close(num(&tt["y"]) - y, got.ys[next]), "{name} {t:?} y");
                }
                let sigil = node["sigil"]["transform"].as_str().unwrap_or("");
                if !sigil.is_empty() {
                    // `translate(X Y) scale(S)`: Y is the node's top plus the (maybe moved) sigil y.
                    let nums: Vec<f64> = sigil
                        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                        .filter(|p| !p.is_empty())
                        .map(|p| p.parse().unwrap())
                        .collect();
                    assert!(close(nums[1] - y, got.sigil_y), "{name} {label:?} sigil y {sigil}");
                    assert!(close(nums[2], got.sigil_size / 16.0), "{name} {label:?} sigil size {sigil}");
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 100, "only {checked} nodes were laid out");
}

#[test]
fn edge_label_boxes_follow_edge_label_width() {
    let mut checked = 0;
    for (name, case) in goldens() {
        let ty = ty_of(&case);
        for render in renders(&case) {
            let profile = match render["svg_attrs"]["data-quality-profile"].as_str() {
                Some("showcase") => Profile::Showcase,
                _ => Profile::Standard,
            };
            let Some(labels) = render["labels"].as_array() else { continue };
            for label in labels {
                let Some(rect) = label.get("rect") else { continue };
                let texts = label["texts"].as_array().unwrap();
                let lines: Vec<&str> = texts.iter().map(|t| str_of(&t["text"])).collect();
                let got = edge_label_width(ty, &lines, profile);
                let want = num(&rect["width"]);
                assert!(close(got, want), "{name} {lines:?} ({profile:?}): {got} vs {want}");
                checked += 1;
            }
        }
    }
    assert!(checked > 100, "only {checked} label boxes were compared");
}

/// Archify's own `geom/label-too-wide` messages say `~Npx` and the box width; both models agree.
#[test]
fn validation_model_reproduces_the_label_too_wide_messages() {
    let mut checked = 0;
    for (name, case) in goldens() {
        if !name.starts_with("neg.") || case["rule"] != "geom/label-too-wide" {
            continue;
        }
        let ty = ty_of(&case);
        let diags = case["validate_receipt"]["authored"]["receipt"]["diagnostics"].as_array().unwrap();
        for d in diags {
            let msg = str_of(&d["message"]);
            let Some(rest) = msg.strip_prefix("Label \"") else { continue };
            let Some((label, rest)) = rest.split_once("\" (~") else { continue };
            if !rest.contains("is wider than") {
                continue;
            }
            let (px, rest) = rest.split_once("px)").unwrap();
            let want: f64 = px.parse().unwrap();
            // The box width is the first number after the clause: `node "src" (112px)`, `the 86px participant box`.
            let width: f64 = rest
                .split(|c: char| !c.is_ascii_digit())
                .find(|s| !s.is_empty())
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| panic!("{name}: no width in {msg}"));
            let (k, _) = validation_label(ty);
            assert_eq!(js_round(k * units(label) as f64), want, "{name}: {msg}");
            assert!(label_too_wide(ty, label, width), "{name}: {label:?} in {width}");
            checked += 1;
        }
    }
    // dataflow, architecture x2, lifecycle x2, sequence, workflow v2.
    assert_eq!(checked, 7);
}

/// Every node of a document Archify accepted passes the validation model.
#[test]
fn accepted_documents_have_no_too_wide_label() {
    let mut checked = 0;
    for (name, case) in goldens() {
        if case["validate_receipt"]["authored"]["exit_code"] != 0 {
            continue;
        }
        let ty = ty_of(&case);
        for render in renders(&case) {
            for node in render["nodes"].as_array().unwrap() {
                let (label, width) = (str_of(&node["label"]), num(&node["rect"]["width"]));
                assert!(!label_too_wide(ty, label, width), "{name}: {label:?} in {width}");
                checked += 1;
            }
        }
    }
    assert!(checked > 100, "only {checked} nodes were checked");
}
