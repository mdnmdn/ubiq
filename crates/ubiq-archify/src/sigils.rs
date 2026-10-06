//! 16x16 sigil glyphs as `Path` primitives (P2.3). `00` §5.3, `03` §4.5, `renderers/shared/utils.mjs:32-91`.
//!
//! A sigil is a stroke-only glyph in a 16x16 box, drawn at `(x + 6, y + 6)`, size 11
//! (`scale = size / 16`), `stroke: currentColor` (the kind's stroke token), width 1.35 **screen px**
//! (`vector-effect: non-scaling-stroke`), round caps and joins, opacity .76; the `sigil-fill` parts
//! are filled and unstroked. `icon: "none"` hides it; an unknown `icon` falls back to `neutral`.
//!
//! The source SVG elements (`rect`, `circle`, `ellipse`, `path`) are turned into path commands with
//! `kurbo` (arcs become cubics); this module owns no SVG parser.
//!
//! Brand marks (107 vector marks, the badge, the globe) are in `brand.rs`; the rail a badge takes
//! beside the sigil is [`SOURCE_BADGE_FOOTPRINT`]'s neighbour in `text.rs` (`BRAND_RAIL`).

use kurbo::{BezPath, Circle, Ellipse, PathEl, Rect as KRect, RoundedRect, Shape as _};

use crate::scene::{Cmd, Fill, PathShape, Stroke};
use crate::tokens::{Kind, Token};

pub const SIGIL_INSET: f64 = 6.0;
pub const SIGIL_SIZE: f64 = 11.0;
/// Left rail reserved by a sigil: inset + size.
pub const SIGIL_FOOTPRINT: f64 = SIGIL_INSET + SIGIL_SIZE;
/// Right rail reserved by the runtime "sources" beacon, just left of the brand mark.
pub const SOURCE_BADGE_FOOTPRINT: f64 = 38.0;
/// Stroke width in screen px (non-scaling).
pub const SIGIL_STROKE: f64 = 1.35;
pub const SIGIL_OPACITY: f64 = 0.76;
/// Tolerance for turning circles and rounded rects into cubics, in glyph units.
const TOLERANCE: f64 = 0.01;

/// The 19 glyph keys.
pub const NAMES: [&str; 19] = [
    "calendar",
    "clock",
    "person",
    "briefcase",
    "flag",
    "moon",
    "frontend",
    "backend",
    "database",
    "cloud",
    "security",
    "messagebus",
    "external",
    "start",
    "active",
    "waiting",
    "success",
    "failure",
    "neutral",
];

/// One SVG element of a glyph, in 16x16 space.
enum Part {
    Rect(f64, f64, f64, f64, f64),
    Circle(f64, f64, f64),
    Ellipse(f64, f64, f64, f64),
    Path(&'static str),
}

use Part::{Circle as Ci, Ellipse as El, Path as P, Rect as Re};

/// `(part, filled)` per glyph, copied from `SIGIL_SHAPE`.
fn parts(name: &str) -> Option<&'static [(Part, bool)]> {
    Some(match name {
        "calendar" => &[(Re(2.0, 3.5, 12.0, 10.5, 2.0), false), (P("M5 2v3M11 2v3M2 7h12M5 10h2M9 10h2"), false)],
        "clock" => &[(Ci(8.0, 8.0, 6.0), false), (P("M8 4v4l3 2"), false)],
        "person" => &[(Ci(8.0, 4.5, 2.5), false), (P("M3 14v-2a5 5 0 0 1 10 0v2"), false)],
        "briefcase" => &[(Re(2.0, 5.0, 12.0, 9.0, 2.0), false), (P("M5 5V2h6v3M2 9h12M7 9v2h2V9"), false)],
        "flag" => &[(P("M3 14V2h10l-2 3 2 3H3"), false)],
        "moon" => &[(P("M13.5 10A6 6 0 0 1 6 2.5 6 6 0 1 0 13.5 10Z"), false)],
        "frontend" => &[
            (Re(2.0, 3.0, 12.0, 10.0, 2.0), false),
            (P("M2 6.5h12"), false),
            (Ci(4.1, 4.8, 0.7), true),
            (Ci(6.3, 4.8, 0.7), true),
        ],
        "backend" => &[(P("M6 3 3 8l3 5M10 3l3 5-3 5"), false)],
        "database" => &[
            (El(8.0, 4.0, 5.0, 2.0), false),
            (P("M3 4v8c0 1.1 2.2 2 5 2s5-.9 5-2V4M3 8c0 1.1 2.2 2 5 2s5-.9 5-2"), false),
        ],
        "cloud" => &[(P("M4.3 12.5h7.3a2.4 2.4 0 0 0 .2-4.8 4 4 0 0 0-7.5-1.3A3.1 3.1 0 0 0 4.3 12.5Z"), false)],
        "security" => &[
            (P("M8 2.2 13 4v3.5c0 3.1-1.8 5.4-5 6.5-3.2-1.1-5-3.4-5-6.5V4Z"), false),
            (P("m5.8 8 1.5 1.5 3-3"), false),
        ],
        "messagebus" => &[
            (P("M2.5 4.5h11M2.5 8h11M2.5 11.5h11"), false),
            (Ci(5.0, 4.5, 1.0), true),
            (Ci(10.5, 8.0, 1.0), true),
            (Ci(7.0, 11.5, 1.0), true),
        ],
        "external" => &[(Re(2.5, 5.0, 8.5, 8.0, 1.5), false), (P("M8 2.5h5.5V8M13.5 2.5 7.5 8.5"), false)],
        "start" => &[(Ci(8.0, 8.0, 5.0), false), (P("m7 5.4 3.6 2.6L7 10.6Z"), true)],
        "active" => &[(P("M2 8h3l1.5-3.5L9 12l1.6-4H14"), false)],
        "waiting" => &[(
            P("M4 2.5h8M4 13.5h8M5 3c0 2.8 2 3.2 3 5-1 1.8-3 2.2-3 5M11 3c0 2.8-2 3.2-3 5 1 1.8 3 2.2 3 5"),
            false,
        )],
        "success" => &[(Ci(8.0, 8.0, 5.3), false), (P("m5.2 8 1.8 1.8 3.8-4"), false)],
        "failure" => &[(Ci(8.0, 8.0, 5.3), false), (P("m5.7 5.7 4.6 4.6m0-4.6-4.6 4.6"), false)],
        "neutral" => &[(Re(3.0, 3.0, 10.0, 10.0, 2.0), false), (Ci(8.0, 8.0, 1.2), true)],
        _ => return None,
    })
}

fn bez(part: &Part) -> BezPath {
    match *part {
        Part::Rect(x, y, w, h, rx) if rx > 0.0 => {
            RoundedRect::new(x, y, x + w, y + h, rx).to_path(TOLERANCE)
        }
        Part::Rect(x, y, w, h, _) => KRect::new(x, y, x + w, y + h).to_path(TOLERANCE),
        Part::Circle(cx, cy, r) => Circle::new((cx, cy), r).to_path(TOLERANCE),
        Part::Ellipse(cx, cy, rx, ry) => Ellipse::new((cx, cy), (rx, ry), 0.0).to_path(TOLERANCE),
        // The strings above are constants; a typo is a bug caught by `every_glyph_parses`.
        Part::Path(d) => BezPath::from_svg(d).unwrap_or_default(),
    }
}

fn cmds(path: &BezPath) -> Vec<Cmd> {
    let pt = |p: kurbo::Point| [p.x, p.y];
    path.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(p) => Cmd::M(pt(p)),
            PathEl::LineTo(p) => Cmd::L(pt(p)),
            PathEl::QuadTo(c, e) => Cmd::Q(pt(c), pt(e)),
            PathEl::CurveTo(c1, c2, e) => Cmd::C(pt(c1), pt(c2), pt(e)),
            PathEl::ClosePath => Cmd::Z,
        })
        .collect()
}

/// The glyph a node shows: `icon` if set (`"none"` hides it, an unknown key falls back to
/// `neutral`), else its `kind`/`type` key.
pub fn resolve(kind: &str, icon: Option<&str>) -> Option<&'static str> {
    let selected = icon.unwrap_or(kind);
    if selected == "none" && icon.is_some() {
        return None;
    }
    NAMES.iter().copied().find(|n| *n == selected).or(Some("neutral"))
}

/// `SIGIL_TONE`: the colour kind of a node `kind`/`type`. Unknown is `external`.
pub fn tone(kind: &str) -> Kind {
    Kind::from_state_type(kind)
}

/// One element of a glyph in its own 16x16 space: commands and whether it is filled.
pub fn glyph(name: &str) -> Option<Vec<(Vec<Cmd>, bool)>> {
    Some(parts(name)?.iter().map(|(p, filled)| (cmds(&bez(p)), *filled)).collect())
}

/// The `Path` primitives of a node's sigil at `(x, y)` (the node's top-left plus the inset the
/// caller chose, normally [`SIGIL_INSET`]), `size` normally [`SIGIL_SIZE`]. Empty for `icon: "none"`.
pub fn sigil_paths(kind: &str, icon: Option<&str>, x: f64, y: f64, size: f64) -> Vec<PathShape> {
    let Some(name) = resolve(kind, icon) else { return Vec::new() };
    let token = Token::KindStroke(tone(kind));
    let s = size / 16.0;
    parts(name)
        .unwrap_or(&[])
        .iter()
        .map(|(p, filled)| PathShape {
            cmds: cmds(&bez(p)),
            transform: [s, 0.0, 0.0, s, x, y],
            stroke: (!filled).then(|| Stroke::solid(token, SIGIL_STROKE)),
            fill: filled.then(|| Fill::new(token)),
            opacity: SIGIL_OPACITY,
            non_scaling: true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_parses_and_stays_inside_the_box() {
        for name in NAMES {
            let ps = parts(name).unwrap_or_else(|| panic!("{name} has no parts"));
            assert!(!ps.is_empty());
            for (p, _) in ps {
                let path = bez(p);
                assert!(!path.elements().is_empty(), "{name}: empty path");
                let b = path.bounding_box();
                assert!(b.x0 >= -0.01 && b.y0 >= -0.01 && b.x1 <= 16.01 && b.y1 <= 16.01, "{name}: {b:?}");
                assert!(b.x0.is_finite() && b.y1.is_finite());
            }
        }
        assert!(parts("nope").is_none());
    }

    #[test]
    fn resolve_follows_render_semantic_sigil() {
        assert_eq!(resolve("backend", None), Some("backend"));
        assert_eq!(resolve("backend", Some("clock")), Some("clock"));
        assert_eq!(resolve("backend", Some("none")), None);
        assert_eq!(resolve("backend", Some("sparkle")), Some("neutral"));
        assert_eq!(resolve("decision", None), Some("neutral"), "decision has a tone but no glyph");
    }

    #[test]
    fn tone_table() {
        for (k, t) in [
            ("frontend", Kind::Frontend),
            ("start", Kind::Frontend),
            ("active", Kind::Frontend),
            ("backend", Kind::Backend),
            ("success", Kind::Backend),
            ("database", Kind::Database),
            ("decision", Kind::Database),
            ("cloud", Kind::Cloud),
            ("waiting", Kind::Cloud),
            ("security", Kind::Security),
            ("failure", Kind::Security),
            ("messagebus", Kind::Messagebus),
            ("external", Kind::External),
            ("neutral", Kind::External),
            ("whatever", Kind::External),
        ] {
            assert_eq!(tone(k), t, "{k}");
        }
    }

    #[test]
    fn frontend_has_two_filled_dots() {
        let g = glyph("frontend").unwrap();
        assert_eq!(g.len(), 4);
        assert_eq!(g.iter().filter(|(_, f)| *f).count(), 2);
        assert!(glyph("start").unwrap().iter().any(|(c, f)| *f && c.last() == Some(&Cmd::Z)));
    }

    #[test]
    fn sigil_paths_carry_the_spec_style() {
        let ps = sigil_paths("database", None, 50.0, 134.0, SIGIL_SIZE);
        assert_eq!(ps.len(), 2);
        let scale = 11.0 / 16.0;
        for p in &ps {
            assert_eq!(p.transform, [scale, 0.0, 0.0, scale, 50.0, 134.0]);
            assert_eq!(p.opacity, 0.76);
            assert!(p.non_scaling);
            let s = p.stroke.as_ref().unwrap();
            assert_eq!((s.token, s.width), (Token::KindStroke(Kind::Database), 1.35));
            assert!(p.fill.is_none());
        }
        let msg = sigil_paths("messagebus", None, 0.0, 0.0, 11.0);
        assert_eq!(msg.iter().filter(|p| p.fill.is_some() && p.stroke.is_none()).count(), 3);
        assert!(sigil_paths("backend", Some("none"), 0.0, 0.0, 11.0).is_empty());
        // The footprint constants match text-fit.
        assert_eq!(SIGIL_FOOTPRINT, 17.0);
    }

    #[test]
    fn a_path_glyph_keeps_its_commands() {
        // `flag` is `M3 14V2h10l-2 3 2 3H3`: one move then seven lines.
        let g = glyph("flag").unwrap();
        let c = &g[0].0;
        assert_eq!(c[0], Cmd::M([3.0, 14.0]));
        assert_eq!(c[1], Cmd::L([3.0, 2.0]));
        assert_eq!(c[2], Cmd::L([13.0, 2.0]));
        assert_eq!(c[3], Cmd::L([11.0, 5.0]));
        assert_eq!(*c.last().unwrap(), Cmd::L([3.0, 8.0]));
    }
}
