//! SVG export of a compiled [`Scene`] and its PNG rasterisation.
//!
//! The scene's primitives are already SVG-shaped, so [`scene_to_svg`] is a plain writer: items in
//! z-order, one element each, in viewBox units. Colours are never decided here: the caller resolves
//! every [`Token`] (typically `palette.resolve(token).css()`). With the `png` feature, `svg_to_png` rasterises with
//! `resvg`, loading the system fonts so text renders.

use std::fmt::Write as _;
#[cfg(feature = "png")]
use std::sync::Arc;

#[cfg(feature = "png")]
use anyhow::{Context, Result, anyhow};

use crate::scene::{Anchor, Cmd, PathShape, PolylineShape, RectShape, Scene, Shape, Stroke, TextShape};
use crate::tokens::Token;

/// The text face; the glyphs are measured with a 0.6 em advance, so any sans fits.
const FONT_FAMILY: &str = "Inter, 'Segoe UI', Helvetica, Arial, sans-serif";

/// A standalone SVG of `scene`. `colours` resolves a token to a CSS colour (`#rrggbb` or
/// `rgba(r, g, b, a)`); `background`, if given, is painted as a full-size rect first.
pub fn scene_to_svg(scene: &Scene, colours: &dyn Fn(Token) -> String, background: Option<&str>) -> String {
    let [w, h] = scene.view_box;
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="{}">"#,
        esc(FONT_FAMILY)
    );
    if let Some(bg) = background {
        let _ = write!(s, r#"<rect width="{w}" height="{h}" fill="{}"/>"#, esc(bg));
    }
    for item in &scene.items {
        match &item.shape {
            Shape::Rect(r) => rect(&mut s, r, colours),
            Shape::Polyline(p) => polyline(&mut s, p, colours),
            Shape::Text(t) => text(&mut s, t, colours),
            Shape::Path(p) => path(&mut s, p, colours),
        }
    }
    s.push_str("</svg>\n");
    s
}

fn rect(s: &mut String, r: &RectShape, colours: &dyn Fn(Token) -> String) {
    let b = r.rect;
    let _ = write!(s, r#"<rect x="{}" y="{}" width="{}" height="{}""#, b.x, b.y, b.width, b.height);
    if r.radius > 0.0 {
        let _ = write!(s, r#" rx="{0}" ry="{0}""#, r.radius);
    }
    fill_attrs(s, r.fill.map(|f| (f.token, f.alpha)), colours);
    stroke_attrs(s, r.stroke.as_ref(), colours);
    s.push_str("/>");
}

fn polyline(s: &mut String, p: &PolylineShape, colours: &dyn Fn(Token) -> String) {
    let d = cmds_d(&p.cmds());
    if p.halo {
        let _ = write!(
            s,
            r#"<path d="{d}" fill="none" stroke="{}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"/>"#,
            esc(&colours(Token::Mask)),
            p.halo_width()
        );
    }
    let _ = write!(s, r#"<path d="{d}" fill="none""#);
    stroke_attrs(s, Some(&p.stroke), colours);
    s.push_str("/>");
    if let (Some(m), Some(a)) = (p.marker, p.arrow()) {
        let pts: Vec<String> = a.triangle().iter().map(|q| format!("{},{}", q[0], q[1])).collect();
        let _ = write!(s, r#"<polygon points="{}" fill="{}"/>"#, pts.join(" "), esc(&colours(m.fill)));
    }
}

fn text(s: &mut String, t: &TextShape, colours: &dyn Fn(Token) -> String) {
    let anchor = match t.anchor {
        Anchor::Start => "start",
        Anchor::Middle => "middle",
    };
    let _ = write!(
        s,
        r#"<text x="{}" y="{}" font-size="{}" font-weight="{}" text-anchor="{anchor}" fill="{}">{}</text>"#,
        t.at[0],
        t.at[1],
        t.size,
        t.weight,
        esc(&colours(t.token)),
        esc(&t.text)
    );
}

fn path(s: &mut String, p: &PathShape, colours: &dyn Fn(Token) -> String) {
    let _ = write!(s, r#"<path d="{}""#, cmds_d(&p.cmds));
    let t = p.transform;
    if t != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
        let _ = write!(s, r#" transform="matrix({} {} {} {} {} {})""#, t[0], t[1], t[2], t[3], t[4], t[5]);
    }
    if p.opacity < 1.0 {
        let _ = write!(s, r#" opacity="{}""#, p.opacity);
    }
    fill_attrs(s, p.fill.map(|f| (f.token, f.alpha)), colours);
    stroke_attrs(s, p.stroke.as_ref(), colours);
    if p.non_scaling {
        s.push_str(r#" vector-effect="non-scaling-stroke""#);
    }
    s.push_str("/>");
}

fn fill_attrs(s: &mut String, fill: Option<(Token, f64)>, colours: &dyn Fn(Token) -> String) {
    match fill {
        Some((tok, alpha)) => {
            let _ = write!(s, r#" fill="{}""#, esc(&colours(tok)));
            if alpha < 1.0 {
                let _ = write!(s, r#" fill-opacity="{alpha}""#);
            }
        }
        None => s.push_str(r#" fill="none""#),
    }
}

fn stroke_attrs(s: &mut String, stroke: Option<&Stroke>, colours: &dyn Fn(Token) -> String) {
    let Some(st) = stroke else { return };
    let _ = write!(s, r#" stroke="{}" stroke-width="{}""#, esc(&colours(st.token)), st.width);
    if !st.dash.is_empty() {
        let d: Vec<String> = st.dash.iter().map(f64::to_string).collect();
        let _ = write!(s, r#" stroke-dasharray="{}""#, d.join(" "));
    }
}

fn cmds_d(cmds: &[Cmd]) -> String {
    let mut d = String::new();
    for c in cmds {
        let _ = match c {
            Cmd::M([x, y]) => write!(d, "M{x} {y}"),
            Cmd::L([x, y]) => write!(d, "L{x} {y}"),
            Cmd::Q([cx, cy], [x, y]) => write!(d, "Q{cx} {cy} {x} {y}"),
            Cmd::C([a, b], [c, e], [x, y]) => write!(d, "C{a} {b} {c} {e} {x} {y}"),
            Cmd::Z => write!(d, "Z"),
        };
    }
    d
}

/// XML-escape text and attribute values.
fn esc(t: &str) -> String {
    let mut o = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

#[cfg(feature = "png")]
/// Rasterise `svg` to PNG bytes at `scale` times its intrinsic size. System fonts are loaded so
/// text renders; a transparent background stays transparent.
pub fn svg_to_png(svg: &str, scale: f32) -> Result<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let mut opt = usvg::Options::default();
    Arc::make_mut(&mut opt.fontdb).load_system_fonts();
    let tree = usvg::Tree::from_str(svg, &opt).context("parse svg")?;
    let size = tree.size();
    let (w, h) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or_else(|| anyhow!("bad pixmap size {w}x{h}"))?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().context("encode png")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{Opts, compile};
    use crate::tokens::{Mode, classic};

    fn compiled_svg() -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/web-app.architecture.golden.json");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let src = serde_json::to_string(&v["source_doc"]).unwrap();
        let c = compile(&src, &Opts { input: "web-app.architecture.json", doc_type: Some("architecture"), quality: None });
        let scene = c.layout.as_ref().expect("fixture lays out").scene();
        let pal = classic(Mode::Dark);
        scene_to_svg(scene, &|t| pal.resolve(t).css(), Some("#101014"))
    }

    #[test]
    fn svg_parses_and_carries_the_labels() {
        let svg = compiled_svg();
        #[cfg(feature = "png")]
        usvg_parse(&svg);
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/web-app.architecture.golden.json")).unwrap(),
        )
        .unwrap();
        let nodes = v["source_doc"]["components"].as_array().expect("components");
        assert!(!nodes.is_empty());
        let label = nodes[0]["label"].as_str().unwrap();
        assert!(svg.contains(&esc(label)), "label {label} missing");
        assert!(svg.starts_with("<svg") && svg.contains("viewBox"));
    }

    #[cfg(feature = "png")]
    fn usvg_parse(svg: &str) {
        let mut o = resvg::usvg::Options::default();
        Arc::make_mut(&mut o.fontdb).load_system_fonts();
        resvg::usvg::Tree::from_str(svg, &o).expect("usvg parses the export");
    }

    #[cfg(feature = "png")]
    #[test]
    fn png_has_signature_and_size() {
        let png = svg_to_png(&compiled_svg(), 1.0).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(png.len() > 1000);
        let big = svg_to_png(&compiled_svg(), 2.0).unwrap();
        assert!(big.len() > png.len() / 2);
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(esc(r#"a<b>&"c""#), "a&lt;b&gt;&amp;&quot;c&quot;");
    }
}
