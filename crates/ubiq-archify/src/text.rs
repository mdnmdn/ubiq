//! Text units and fitting (P2.1). `00` §5.1, `03` §2.1. Port of `renderers/shared/text-fit.mjs`,
//! `textUnits` of `utils.mjs`, and the per-type label-box widths of the renderers.
//!
//! Text is **estimated, never measured** (D13). Two models exist and both are kept:
//!
//! - the *render* model: width = `units · 0.6 · fontSize`, which drives [`fit`], [`min_text_width`]
//!   and [`node_label_layout`];
//! - the *validation* model: a per-type px-per-unit constant ([`validation_label`]: A 6.6, W/S 6.8,
//!   D/L 6.2) that does not equal `0.6 · preferred font` (D 10 px is 6.0, W 11 px is 6.6) and must be
//!   kept for the `geom/label-too-wide` rule.
//!
//! Edge-label boxes have a third family of constants, [`edge_label_width`].
//!
//! The East-Asian table is Archify's own (Unicode 17 `FULLWIDTH_RE`, kept local because
//! `unicode-width` classifies differently), not an import.

use crate::diag::js_round;
use crate::sigils::{SIGIL_FOOTPRINT, SIGIL_INSET, SIGIL_SIZE, SOURCE_BADGE_FOOTPRINT};

/// Px of advance per text unit per px of font size (`nodeTextFit.widthFactor`).
pub const WIDTH_FACTOR: f64 = 0.6;
/// Total px reserved inside a box so text never touches the border (`horizontalPadding`).
pub const HORIZONTAL_PADDING: f64 = 8.0;
/// Legend text advance, em per unit (`legend.mjs:10`).
pub const LEGEND_ADVANCE_EM: f64 = 0.62;
/// Width a brand badge takes off the label's fit width (`brandLabelFitWidth`).
pub const BRAND_RAIL: f64 = 48.0;

/// East Asian Wide/Fullwidth ranges, inclusive, sorted and disjoint: the character class of
/// `FULLWIDTH_RE` (`utils.mjs:213`), extracted mechanically.
pub const WIDE: [(u32, u32); 50] = [
    (0x1100, 0x115F),
    (0x231A, 0x231B),
    (0x2329, 0x232A),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2630, 0x2637),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x268A, 0x268F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0xA4CF),
    (0xA960, 0xA97C),
    (0xAC00, 0xD7A3),
    (0xF900, 0xFAFF),
    (0xFE10, 0xFE19),
    (0xFE30, 0xFE6F),
    (0xFF01, 0xFF60),
    (0xFFE0, 0xFFE6),
    (0x16FE0, 0x18DFF),
    (0x1AFF0, 0x1AFFF),
    (0x1B000, 0x1B2FF),
    (0x1F000, 0x1FAFF),
    (0x20000, 0x3FFFD),
];

const VS_FIRST: char = '\u{FE00}';
const VS_LAST: char = '\u{FE0F}';
const VS16: char = '\u{FE0F}';

/// Takes two columns of advance width.
pub fn is_wide(c: char) -> bool {
    let cp = c as u32;
    WIDE.binary_search_by(|&(lo, hi)| {
        if cp < lo {
            std::cmp::Ordering::Greater
        } else if cp > hi {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

/// `textUnits`: 1 per code point, 2 per Wide/Fullwidth, 2 for a base followed by VS16, 0 for a
/// variation selector (U+FE00..FE0F).
pub fn units(s: &str) -> u32 {
    let chars: Vec<char> = s.chars().collect();
    let mut total = 0;
    for (i, &c) in chars.iter().enumerate() {
        if (VS_FIRST..=VS_LAST).contains(&c) {
            continue;
        }
        total += if chars.get(i + 1) == Some(&VS16) || is_wide(c) { 2 } else { 1 };
    }
    total
}

/// Render-model width of `text` at `font_size`: `units · 0.6 · fontSize`.
pub fn render_width(text: &str, font_size: f64) -> f64 {
    units(text) as f64 * font_size * WIDTH_FACTOR
}

/// `fittedNodeFontSize`: the largest size at or below `preferred` that fits `text` in `width`,
/// floored to 0.1 and never below `min`.
pub fn fit(text: &str, width: f64, preferred: f64, min: f64) -> f64 {
    let u = units(text).max(1) as f64;
    let available = (width - HORIZONTAL_PADDING).max(1.0);
    let fitted = preferred.min(available / (u * WIDTH_FACTOR));
    min.max((fitted * 10.0).floor() / 10.0)
}

/// `minimumNodeTextWidth`: the width `text` still needs at its legible minimum.
pub fn min_text_width(text: &str, min: f64) -> f64 {
    render_width(text, min)
}

/// `availableNodeTextWidth`.
pub fn available_width(width: f64) -> f64 {
    width - HORIZONTAL_PADDING
}

/// Whether shrinking to `min` cannot save `text` in a box `width` wide (the validation test of the
/// sublabel/tag fit rules).
pub fn overflows_at_min(text: &str, width: f64, min: f64) -> bool {
    min_text_width(text, min) > available_width(width)
}

/// `brandLabelFitWidth`: the width a label may use, `width - 48` when the node bears a brand mark.
pub fn label_fit_width(width: f64, brand: bool) -> f64 {
    if brand { (width - BRAND_RAIL).max(1.0) } else { width }
}

/// `brandTopRailProblem`: `(available, required)` when a brand node's label cannot fit at its
/// legible minimum, else `None`.
pub fn brand_rail_problem(label: &str, width: f64, min_font: f64) -> Option<(f64, f64)> {
    let available = width - BRAND_RAIL;
    let required = render_width(label, min_font);
    (available < required).then_some((available, required))
}

/// The five diagram types, as far as text is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagramType {
    Architecture,
    Workflow,
    Sequence,
    Dataflow,
    Lifecycle,
}

impl DiagramType {
    pub fn parse(s: &str) -> Option<DiagramType> {
        Some(match s {
            "architecture" => DiagramType::Architecture,
            "workflow" => DiagramType::Workflow,
            "sequence" => DiagramType::Sequence,
            "dataflow" => DiagramType::Dataflow,
            "lifecycle" => DiagramType::Lifecycle,
            _ => return None,
        })
    }
}

/// A `(preferred, minimum)` font-size pair.
pub type Sizes = (f64, f64);

/// The node-text sizes of one renderer (`00` §5.1). `tag` is `None` for sequence participants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeFonts {
    pub label: Sizes,
    pub sublabel: Sizes,
    pub tag: Option<Sizes>,
}

/// The sizes each renderer passes to [`fit`]. `schema_version` matters only for lifecycle
/// (v2: 11/9, 8/7, 8/7; v1: 10/8, 7/6, 7/6).
pub fn node_fonts(ty: DiagramType, schema_version: u32) -> NodeFonts {
    match ty {
        DiagramType::Architecture => NodeFonts { label: (11.0, 8.0), sublabel: (9.0, 6.0), tag: Some((7.0, 6.0)) },
        DiagramType::Dataflow => NodeFonts { label: (10.0, 8.0), sublabel: (7.0, 6.0), tag: Some((7.0, 6.0)) },
        DiagramType::Lifecycle if schema_version == 2 => {
            NodeFonts { label: (11.0, 9.0), sublabel: (8.0, 7.0), tag: Some((8.0, 7.0)) }
        }
        DiagramType::Lifecycle => NodeFonts { label: (10.0, 8.0), sublabel: (7.0, 6.0), tag: Some((7.0, 6.0)) },
        DiagramType::Workflow => NodeFonts { label: (11.0, 9.0), sublabel: (8.0, 6.0), tag: Some((7.0, 6.0)) },
        DiagramType::Sequence => NodeFonts { label: (11.0, 8.0), sublabel: (7.0, 6.0), tag: None },
    }
}

/// The validation model's `(px per unit, slack)` for the `geom/label-too-wide` rule:
/// `k · units(label) > width + p`. A 6.6/8, W and S 6.8/6, D and L 6.2/6.
pub fn validation_label(ty: DiagramType) -> (f64, f64) {
    match ty {
        DiagramType::Architecture => (6.6, 8.0),
        DiagramType::Workflow | DiagramType::Sequence => (6.8, 6.0),
        DiagramType::Dataflow | DiagramType::Lifecycle => (6.2, 6.0),
    }
}

/// `geom/label-too-wide`, validation model (D13): the node (or participant) is too narrow for
/// its label even at full size.
pub fn label_too_wide(ty: DiagramType, label: &str, width: f64) -> bool {
    let (k, p) = validation_label(ty);
    k * units(label) as f64 > width + p
}

/// Message-label sizing of a sequence document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Standard,
    Showcase,
}

/// The width of an edge-label box for its longest line (`lines` is the label, plus the dataflow
/// `classification` or lifecycle `note`):
///
/// - A, W: `max(30, 4.8u + 10)`
/// - D: `max(34, 4.9u + 12)`, rounded to 0.1 (`Math.round`)
/// - L: `max(32, 4.9u + 12)`
/// - S: `max(34, k·u + 12)`, `k` 6.6 showcase / 5.2 standard.
pub fn edge_label_width(ty: DiagramType, lines: &[&str], profile: Profile) -> f64 {
    let u = lines.iter().map(|l| units(l)).max().unwrap_or(0) as f64;
    match ty {
        DiagramType::Architecture | DiagramType::Workflow => (u * 4.8 + 10.0).max(30.0),
        DiagramType::Dataflow => js_round((u * 4.9 + 12.0).max(34.0) * 10.0) / 10.0,
        DiagramType::Lifecycle => (u * 4.9 + 12.0).max(32.0),
        DiagramType::Sequence => {
            let k = if profile == Profile::Showcase { 6.6 } else { 5.2 };
            (u * k + 12.0).max(34.0)
        }
    }
}

/// Architecture boundary-title box: `max(30, 0.6·u·fs + 10)`.
pub fn boundary_label_width(label: &str, font_size: f64) -> f64 {
    (render_width(label, font_size) + 10.0).max(30.0)
}

/// Legend text advance: `units · fontSize · 0.62`.
pub fn legend_text_width(label: &str, font_size: f64) -> f64 {
    units(label) as f64 * font_size * LEGEND_ADVANCE_EM
}

/// Which side of the node the sigil sits (lifecycle steps share that rail).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Side {
    #[default]
    Left,
    Right,
}

/// One text row: its text (only row 0's is measured), font size and baseline offset from the
/// node's top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row<'a> {
    pub text: &'a str,
    pub font: f64,
    pub y: f64,
}

/// Input of [`node_label_layout`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LabelBox<'a> {
    pub width: f64,
    pub height: f64,
    pub side: Side,
    /// A brand badge sits top-right.
    pub brand: bool,
    /// The runtime sources beacon (never set by the static renderer; kept for parity).
    pub source: bool,
    /// Lifecycle step text on the sigil rail, `""` for none.
    pub step: &'a str,
}

/// Output of [`node_label_layout`]: the label centre `x` (offset from the node's left), the row
/// baselines, and the sigil `y`/`size` (shrunk only as a last resort).
#[derive(Clone, Debug, PartialEq)]
pub struct LabelLayout {
    pub x: f64,
    pub ys: Vec<f64>,
    pub sigil_y: f64,
    pub sigil_size: f64,
}

/// `nodeLabelLayout` (dataflow, lifecycle, workflow; not architecture or sequence): keep the label
/// centred; shift it clear of the sigil/step (left), brand/source (right); else drop every row
/// below the decoration rail; else (source badge only) pack compactly; else shrink the sigil.
pub fn node_label_layout(b: &LabelBox<'_>, rows: &[Row<'_>]) -> LabelLayout {
    let mut result = LabelLayout {
        x: b.width / 2.0,
        ys: rows.iter().map(|r| r.y).collect(),
        sigil_y: SIGIL_INSET,
        sigil_size: SIGIL_SIZE,
    };
    let Some(first) = rows.first() else { return result };
    let left_side = b.side == Side::Left;
    let label_width = min_text_width(first.text, first.font);
    let step_end = if b.step.is_empty() {
        0.0
    } else {
        (if left_side { 23.0 } else { 10.0 }) + min_text_width(b.step, 8.0) + 3.0
    };
    let left = (if left_side { SIGIL_FOOTPRINT + 2.0 } else { 4.0 }).max(step_end);
    let right = b.width
        - (if b.brand {
            26.0
        } else if b.side == Side::Right {
            SIGIL_FOOTPRINT + 2.0
        } else {
            4.0
        })
        - (if b.source { SOURCE_BADGE_FOOTPRINT } else { 0.0 });
    if result.x - label_width / 2.0 >= left && result.x + label_width / 2.0 <= right {
        return result;
    }
    if label_width <= right - left {
        // Round away from the icon, retaining the node centre whenever possible.
        result.x = (((right - label_width / 2.0) * 10.0).floor() / 10.0)
            .min(result.x.max(((left + label_width / 2.0) * 10.0).ceil() / 10.0));
        return result;
    }
    // Keep the font sizes and put the text below the decoration rail.
    let mut bottom = (if b.brand { 22.0 } else { SIGIL_FOOTPRINT }).max(if b.source { 19.0 } else { 0.0 });
    let ys: Vec<f64> = rows
        .iter()
        .map(|r| {
            let y = r.y.max(((bottom + 2.0 + r.font * 1.2) * 10.0).ceil() / 10.0);
            bottom = y + r.font * 0.3;
            y
        })
        .collect();
    if bottom <= b.height - 2.0 {
        result.ys = ys;
        return result;
    }
    if b.source {
        // Crowded fallback: a full em above each baseline and 0.3 em below, using the bottom padding.
        let mut compact = (if b.brand { 22.0 } else { SIGIL_FOOTPRINT }).max(19.0) + 1.0;
        let compact_ys: Vec<f64> = rows
            .iter()
            .map(|r| {
                let y = ((compact + 1.0 + r.font) * 10.0).ceil() / 10.0;
                compact = y + r.font * 0.3;
                y
            })
            .collect();
        if compact <= b.height - 0.5 {
            result.ys = compact_ys;
            return result;
        }
    }
    // A deliberately short fixed box has no spare row: keep the text and shrink only the sigil.
    result.sigil_y = 1.0;
    result.sigil_size = 1.0_f64.max(SIGIL_SIZE.min((first.y - first.font * 1.2 - 3.0).floor()));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Numbers below come from running Archify's own `textUnits` / `fittedNodeFontSize` /
    // `minimumNodeTextWidth` / `nodeLabelLayout` (the Node oracle) on the same inputs.

    #[test]
    fn units_match_archify() {
        for (s, want) in [
            ("", 0),
            ("a", 1),
            ("Checkout API", 12),
            ("日本語", 6),
            ("ｱｲｳ", 3), // halfwidth katakana stays narrow
            ("한국어", 6),
            ("✅", 2),
            ("✅ ok", 5),
            ("\u{2764}\u{FE0F}", 2),  // heart + VS16
            ("\u{2764}", 1),          // text heart
            ("\u{2764}\u{FE0E}", 1),  // VS15 keeps the base width
            ("👍\u{1F3FD}", 4),       // skin-tone modifier is its own wide code point
            ("😀", 2),
            ("⚡", 2),
            ("\u{2630}\u{2637}", 4), // Unicode 16 trigrams
            ("⭐", 2),
            ("é", 1),
            ("e\u{0301}", 2), // combining mark is a code point
            ("\u{FE0F}", 0),
            ("a\u{FE0F}", 2),
            ("日\u{FE0F}", 2),
            ("日\u{FE0E}", 2), // wide base, VS15 ignored
            ("＠", 2),
            ("\u{20000}", 2),
            ("\u{30000}", 2),
            ("\u{3FFFD}", 2),
            ("\u{3FFFE}", 1),
            ("\u{A97D}", 1),
            ("\u{A97C}", 2),
            ("\u{1F000}", 2),
            ("〒", 2),
            ("→", 1),
            ("\u{1100}", 2),
            ("\u{115F}", 2),
            ("\u{1160}", 1),
        ] {
            assert_eq!(units(s), want, "{s:?}");
        }
    }

    #[test]
    fn wide_table_is_sorted_and_disjoint() {
        for w in WIDE.windows(2) {
            assert!(w[0].1 < w[1].0, "{w:?}");
        }
        for (lo, hi) in WIDE {
            assert!(lo <= hi);
        }
    }

    #[test]
    fn fit_matches_archify() {
        for (text, width, pref, min, want) in [
            ("Checkout API", 112.0, 10.0, 8.0, 10.0),
            ("PaymentCaptured", 60.0, 10.0, 8.0, 8.0),
            ("日本語のラベルです", 112.0, 10.0, 8.0, 9.6),
            ("x", 0.0, 10.0, 8.0, 8.0),
            ("x", 9.0, 10.0, 8.0, 8.0),
            ("A very long node label that overflows", 112.0, 11.0, 8.0, 8.0),
            ("order producer", 112.0, 7.0, 6.0, 7.0),
            ("short", 30.0, 9.0, 6.0, 7.3),
            ("", 100.0, 10.0, 8.0, 10.0),
            ("abcdefghij", 60.0, 11.0, 9.0, 9.0),
        ] {
            assert_eq!(fit(text, width, pref, min), want, "{text:?} in {width}");
        }
        // The brand rail takes 48 off the label's width.
        assert_eq!(label_fit_width(112.0, true), 64.0);
        assert_eq!(label_fit_width(112.0, false), 112.0);
        assert_eq!(label_fit_width(40.0, true), 1.0);
    }

    #[test]
    fn min_width_and_overflow() {
        assert!((min_text_width("abc", 8.0) - 14.4).abs() < 1e-12);
        assert!((min_text_width("日本", 9.0) - 21.6).abs() < 1e-12);
        assert!(overflows_at_min("abcdefghijklmnop", 60.0, 6.0));
        assert!(!overflows_at_min("abcdefghij", 60.0, 6.0));
        // Brand rail: 112 - 48 = 64 px for a 12-unit label at 8 = 57.6.
        assert_eq!(brand_rail_problem("Checkout API", 112.0, 8.0), None);
        let (avail, req) = brand_rail_problem("Checkout API", 100.0, 8.0).unwrap();
        assert_eq!(avail, 52.0);
        assert!((req - 57.6).abs() < 1e-9);
    }

    fn layout(width: f64, height: f64, rows: &[Row<'_>], f: impl Fn(&mut LabelBox<'_>)) -> LabelLayout {
        let mut b = LabelBox { width, height, ..LabelBox::default() };
        f(&mut b);
        node_label_layout(&b, rows)
    }

    fn row(text: &str, font: f64, y: f64) -> Row<'_> {
        Row { text, font, y }
    }

    fn lay(x: f64, ys: &[f64], sigil_y: f64, sigil_size: f64) -> LabelLayout {
        LabelLayout { x, ys: ys.to_vec(), sigil_y, sigil_size }
    }

    #[test]
    fn node_label_layout_matches_archify() {
        let r3 = [row("Checkout API", 10.0, 21.0), row("order producer", 7.0, 37.0), row("team commerce", 7.0, 47.0)];
        assert_eq!(layout(112.0, 58.0, &r3, |_| {}), lay(56.0, &[21.0, 37.0, 47.0], 6.0, 11.0));
        let r2 = [row("Checkout API", 8.0, 21.0), row("order producer", 6.0, 37.0)];
        assert_eq!(layout(60.0, 58.0, &r2, |_| {}), lay(30.0, &[28.6, 40.2], 6.0, 11.0));
        let r = [row("Checkout", 8.0, 21.0), row("sub", 6.0, 37.0)];
        assert_eq!(layout(70.0, 58.0, &r, |_| {}), lay(38.2, &[21.0, 37.0], 6.0, 11.0));
        let r = [row("Checkout X", 8.0, 21.0), row("sub", 6.0, 37.0)];
        assert_eq!(layout(70.0, 58.0, &r, |b| b.brand = true), lay(35.0, &[33.6, 45.2], 6.0, 11.0));
        let r = [row("Checkout", 8.0, 21.0), row("sub", 6.0, 37.0)];
        let both = |b: &mut LabelBox<'_>| {
            b.brand = true;
            b.source = true;
        };
        assert_eq!(layout(70.0, 58.0, &r, both), lay(35.0, &[33.6, 45.2], 6.0, 11.0));
        let r = [row("Checkout API", 8.0, 21.0)];
        assert_eq!(layout(60.0, 30.0, &r, both), lay(30.0, &[21.0], 1.0, 8.0));
        assert_eq!(layout(60.0, 30.0, &r, |_| {}), lay(30.0, &[21.0], 1.0, 8.0));
        let r = [row("Checkout API", 8.0, 16.0)];
        assert_eq!(layout(60.0, 24.0, &r, both), lay(30.0, &[16.0], 1.0, 3.0));
        let r = [row("Wait", 9.0, 23.0), row("x", 7.0, 40.0)];
        assert_eq!(layout(90.0, 58.0, &r, |b| b.step = "01"), lay(46.4, &[23.0, 40.0], 6.0, 11.0));
        let r = [row("Wait", 9.0, 23.0)];
        let right = |b: &mut LabelBox<'_>| {
            b.side = Side::Right;
            b.step = "01";
        };
        assert_eq!(layout(90.0, 58.0, &r, right), lay(45.0, &[23.0], 6.0, 11.0));
        let r = [row("Wait", 9.0, 23.0), row("sub", 7.0, 40.0)];
        let src = |b: &mut LabelBox<'_>| {
            b.source = true;
            b.step = "1";
        };
        assert_eq!(layout(50.0, 30.0, &r, src), lay(25.0, &[23.0, 40.0], 1.0, 9.0));
        // The source badge's compact fallback (a 44-high box does not take the full rail).
        let r = [row("Checkout API", 8.0, 21.0), row("sub", 6.0, 37.0)];
        let source = |b: &mut LabelBox<'_>| b.source = true;
        assert_eq!(layout(70.0, 44.0, &r, source), lay(35.0, &[29.0, 38.4], 6.0, 11.0));
        assert_eq!(layout(70.0, 46.0, &r, source), lay(35.0, &[30.6, 42.2], 6.0, 11.0));
        assert_eq!(layout(70.0, 44.0, &r, both), lay(35.0, &[32.0, 41.4], 6.0, 11.0));
        assert_eq!(layout(70.0, 41.0, &r, both), lay(35.0, &[21.0, 37.0], 1.0, 8.0));
        assert_eq!(layout(70.0, 40.0, &r, source), lay(35.0, &[21.0, 37.0], 1.0, 8.0));
    }

    #[test]
    fn empty_rows_are_harmless() {
        let l = node_label_layout(&LabelBox { width: 50.0, height: 20.0, ..LabelBox::default() }, &[]);
        assert_eq!(l, lay(25.0, &[], 6.0, 11.0));
    }

    #[test]
    fn node_fonts_per_type() {
        use DiagramType::*;
        assert_eq!(node_fonts(Architecture, 1).label, (11.0, 8.0));
        assert_eq!(node_fonts(Architecture, 1).sublabel, (9.0, 6.0));
        assert_eq!(node_fonts(Dataflow, 1).label, (10.0, 8.0));
        assert_eq!(node_fonts(Lifecycle, 1).label, (10.0, 8.0));
        assert_eq!(node_fonts(Lifecycle, 2).label, (11.0, 9.0));
        assert_eq!(node_fonts(Lifecycle, 2).tag, Some((8.0, 7.0)));
        assert_eq!(node_fonts(Workflow, 2).label, (11.0, 9.0));
        assert_eq!(node_fonts(Workflow, 2).sublabel, (8.0, 6.0));
        assert_eq!(node_fonts(Sequence, 1).tag, None);
        assert_eq!(DiagramType::parse("dataflow"), Some(Dataflow));
        assert_eq!(DiagramType::parse("nope"), None);
    }

    #[test]
    fn the_two_width_models_differ() {
        use DiagramType::*;
        // D13: the render model at the preferred font vs the validation constant.
        assert_eq!(WIDTH_FACTOR * 10.0, 6.0);
        assert_eq!(validation_label(Dataflow).0, 6.2);
        assert!((WIDTH_FACTOR * 11.0 - 6.6).abs() < 1e-12);
        assert_eq!(validation_label(Workflow).0, 6.8);
        assert_eq!(validation_label(Architecture), (6.6, 8.0));
        assert_eq!(validation_label(Sequence), (6.8, 6.0));
        assert_eq!(validation_label(Lifecycle), (6.2, 6.0));
        // 20 units in a dataflow node: validation says 124 > width + 6 for width < 118, while the
        // render model shrinks the text to fit (and reports it fits at its minimum of 8).
        let label = "abcdefghijklmnopqrst";
        assert!(label_too_wide(Dataflow, label, 117.0));
        assert!(!label_too_wide(Dataflow, label, 118.0));
        assert!(!overflows_at_min(label, 117.0, 8.0));
        assert!(!label_too_wide(Architecture, "abc", 12.0), "19.8 > 20 is false");
        assert!(label_too_wide(Architecture, "abcd", 18.0), "26.4 > 26");
    }

    #[test]
    fn edge_label_widths() {
        use DiagramType::*;
        let std = Profile::Standard;
        // Dataflow: the two goldens of event-stream. OrderPlaced is 11 units, PaymentCaptured 15.
        assert!((edge_label_width(Dataflow, &["OrderPlaced", "schema v1"], std) - 65.9).abs() < 1e-9);
        assert!((edge_label_width(Dataflow, &["PaymentCaptured", "schema v2"], std) - 85.5).abs() < 1e-9);
        assert_eq!(edge_label_width(Dataflow, &["ab"], std), 34.0);
        assert_eq!(edge_label_width(Architecture, &["ab"], std), 30.0);
        assert!((edge_label_width(Architecture, &["abcdefghij"], std) - 58.0).abs() < 1e-9);
        assert_eq!(edge_label_width(Workflow, &["日本語"], std), 38.8);
        assert_eq!(edge_label_width(Lifecycle, &["ab"], std), 32.0);
        assert!((edge_label_width(Lifecycle, &["ab", "a longer note"], std) - 75.7).abs() < 1e-9);
        assert_eq!(edge_label_width(Sequence, &["ab"], Profile::Showcase), 34.0);
        assert!((edge_label_width(Sequence, &["abcdefghij"], Profile::Showcase) - 78.0).abs() < 1e-9);
        assert!((edge_label_width(Sequence, &["abcdefghij"], std) - 64.0).abs() < 1e-9);
        assert_eq!(edge_label_width(Sequence, &[], std), 34.0);
        assert_eq!(boundary_label_width("ab", 9.0), 30.0);
        assert!((boundary_label_width("Production region", 9.0) - 101.8).abs() < 1e-9);
        assert!((legend_text_width("primary data", 10.0) - 74.4).abs() < 1e-9);
    }
}
