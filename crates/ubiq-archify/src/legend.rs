//! Legend: catalogue, footprint, upward wrap, placement and the scene shapes (P2.4).
//! Port of `renderers/shared/legend.mjs` (`resolveLegend`, `legendFootprint`, `measureLegend`,
//! `renderLegend`); `00` §5.8, `03` §4.6.
//!
//! One pure footprint owns both the auto-viewBox sizing (architecture, P3.1) and the final
//! placement, as in Archify. The `obstacles` collision check (`legend/content-overlap`, which only
//! the architecture and workflow renderers pass) is [`measure_with`]; the architecture catalogue is
//! [`architecture_catalog`].

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::diag::{Diagnostic, Subject};
use crate::geom::{Pt, Rect, Seg, rects_overlap, segment_intersects_rect};
use crate::i18n::Locale;
use crate::model::common::LegendMode;
use crate::scene::{
    Anchor, Bounds, Detail, Group, GroupKind, Layer, SceneBuilder, Shape, TextShape,
};
use crate::text;
use crate::tokens::Token;

/// `DEFAULT_FONT_SIZE` (`legend.mjs:6`). Rendered at `+2` (`+0.5` below 8).
pub const FONT_SIZE: f64 = 8.0;
/// `DEFAULT_ITEM_GAP` (`legend.mjs:7`).
pub const ITEM_GAP: f64 = 22.0;
/// `DEFAULT_LINE_GAP` (`legend.mjs:8`).
pub const LINE_GAP: f64 = 22.0;
/// `DEFAULT_SWATCH_GAP` (`legend.mjs:9`).
pub const SWATCH_GAP: f64 = 8.0;
/// The swatch width of an entry that sets none (`legend.mjs:63`, `?? 14`).
pub const SWATCH_WIDTH: f64 = 14.0;
/// `INTERACTIVE_BADGE_ALLOWANCE` (`legend.mjs:11`).
pub const INTERACTIVE_ALLOWANCE: f64 = 21.0;
/// The title is `12` / `650` (`legend.mjs:202`), the entries `500` (`:215`).
pub const TITLE_SIZE: f64 = 12.0;
pub const TITLE_WEIGHT: u16 = 650;
pub const ENTRY_WEIGHT: u16 = 500;
/// The title sits `20` above the first row's baseline offset by the extra rows (`legend.mjs:133`).
pub const TITLE_RISE: f64 = 20.0;
/// The collision rect of the title: `x`, `titleY - 10`, `48 x 14` (`legend.mjs:162`).
pub const TITLE_RECT_WIDTH: f64 = 48.0;
pub const TITLE_RECT_HEIGHT: f64 = 14.0;

/// One catalogue row of a type (`LEGEND_CATALOG`).
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogEntry {
    pub kind: &'static str,
    pub label: String,
    pub swatch_width: Option<f64>,
    pub swatch_gap: Option<f64>,
    /// `interactive !== false`: a present entry may carry the viewer's filter badge.
    pub interactive: bool,
}

/// A per-kind override from `meta.legend.entries` (`legendEntry`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Override {
    pub label: Option<String>,
    pub visible: Option<bool>,
}

/// `meta.legend`, type-erased. `None` where the document has no `legend` at all.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub mode: LegendMode,
    pub entries: HashMap<String, Override>,
}

impl Default for Config {
    fn default() -> Self {
        Config { mode: LegendMode::Auto, entries: HashMap::new() }
    }
}

/// A catalogue row that survived [`resolve`].
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: &'static str,
    pub label: String,
    pub swatch_width: Option<f64>,
    pub swatch_gap: Option<f64>,
    pub present: bool,
    /// `catalog.interactive !== false && present`.
    pub interactive: bool,
}

impl Entry {
    /// `entry.swatchWidth ?? 14`.
    pub fn swatch_w(&self) -> f64 {
        self.swatch_width.unwrap_or(SWATCH_WIDTH)
    }

    /// `entry.swatchGap ?? 8`.
    pub fn swatch_g(&self) -> f64 {
        self.swatch_gap.unwrap_or(SWATCH_GAP)
    }
}

/// `resolveLegend(config, catalog, presentKinds)`: `hidden` is empty; `auto` keeps the present
/// kinds, `all` every row; `visible: true` forces a row in, `visible: false` out; `label` replaces
/// the text when non-empty (`override.label || catalogEntry.label`).
pub fn resolve(config: Option<&Config>, catalog: &[CatalogEntry], present: &HashSet<&str>) -> Vec<Entry> {
    let mode = config.map_or(LegendMode::Auto, |c| c.mode);
    if mode == LegendMode::Hidden {
        return Vec::new();
    }
    let none = Override::default();
    catalog
        .iter()
        .filter_map(|c| {
            let o = config.and_then(|cfg| cfg.entries.get(c.kind)).unwrap_or(&none);
            let selected = mode == LegendMode::All || present.contains(c.kind);
            let visible = o.visible == Some(true) || (selected && o.visible != Some(false));
            visible.then(|| Entry {
                kind: c.kind,
                label: o.label.clone().filter(|l| !l.is_empty()).unwrap_or_else(|| c.label.clone()),
                swatch_width: c.swatch_width,
                swatch_gap: c.swatch_gap,
                present: present.contains(c.kind),
                interactive: c.interactive && present.contains(c.kind),
            })
        })
        .collect()
}

/// `LEGEND_CATALOG` of `render-dataflow.mjs:474-483`: the swatch of an edge row is `34` wide with a
/// `9` gap and never interactive; `database` is the `14 x 9` kind swatch. Labels are
/// `legend.dataflow.*` in the document's locale.
pub fn dataflow_catalog(l: &Locale) -> Vec<CatalogEntry> {
    let label = |kind: &str| l.t(&format!("legend.dataflow.{kind}"));
    let line = |kind| CatalogEntry {
        kind,
        label: label(kind),
        swatch_width: Some(34.0),
        swatch_gap: Some(9.0),
        interactive: false,
    };
    vec![
        line("emphasis"),
        line("security"),
        line("dashed"),
        CatalogEntry { kind: "database", label: label("database"), swatch_width: None, swatch_gap: None, interactive: true },
        line("default"),
    ]
}

/// The seven kinds as plain `{kind, label}` rows (default `14` swatch, `8` gap, interactive when
/// present), in `order`, labelled `legend.<ty>.<kind>`.
fn kind_catalog(l: &Locale, ty: &str, order: [&'static str; 7]) -> Vec<CatalogEntry> {
    order
        .into_iter()
        .map(|kind| CatalogEntry {
            kind,
            label: l.t(&format!("legend.{ty}.{kind}")),
            swatch_width: None,
            swatch_gap: None,
            interactive: true,
        })
        .collect()
}

/// `LEGEND_CATALOG` of `render-architecture.mjs:78-86`: the seven kinds in this order.
pub fn architecture_catalog(l: &Locale) -> Vec<CatalogEntry> {
    kind_catalog(l, "architecture", ["frontend", "backend", "database", "cloud", "security", "messagebus", "external"])
}

/// `LEGEND_CATALOG` of `workflow-compiler.mjs:818-826`: the seven kinds in this order (note
/// `security`, `messagebus` before `database`).
pub fn workflow_catalog(l: &Locale) -> Vec<CatalogEntry> {
    kind_catalog(l, "workflow", ["frontend", "backend", "security", "messagebus", "database", "cloud", "external"])
}

/// What an authored relationship leaves in the legend band (`relationshipLegendObstacles`): one
/// segment per leg of a route (`relationship-segment`) and the label rect (`relationship-label`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Obstacle {
    Segment(Seg),
    Rect(Rect),
}

/// `relationshipLegendObstacles`: for each relation, in order, its route (non-finite points are
/// skipped) as consecutive segments, then its label rect when it has one and every number is finite.
pub fn relationship_obstacles<'a>(relations: impl IntoIterator<Item = (&'a [Pt], Option<Rect>)>) -> Vec<Obstacle> {
    let mut out = Vec::new();
    for (points, label) in relations {
        let finite: Vec<Pt> = points.iter().copied().filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
        out.extend(finite.windows(2).map(|w| Obstacle::Segment(Seg::new(w[0], w[1]))));
        if let Some(rect) = label.filter(Rect::is_finite) {
            out.push(Obstacle::Rect(rect));
        }
    }
    out
}

/// `measuredEntryWidth`: `ceil(swatch + gap + units * fontSize * 0.62 (+ 21 if interactive))`,
/// evaluated left to right as the JS does.
fn entry_width(entry: &Entry, font_size: f64) -> f64 {
    (entry.swatch_w()
        + entry.swatch_g()
        + text::legend_text_width(&entry.label, font_size)
        + if entry.interactive { INTERACTIVE_ALLOWANCE } else { 0.0 })
    .ceil()
}

/// `legendFootprint`: the widths, the greedy rows (indices into the entries) and the extra height.
#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    pub widths: Vec<f64>,
    pub rows: Vec<Vec<usize>>,
    pub row_count: usize,
    /// The widest entry (architecture needs it for `legend min width + 80`).
    pub min_width: f64,
    pub extra_height: f64,
}

/// The two numbers a renderer may change in `legendFootprint`/`measureLegend`: the entry font size
/// and the gap between entries of a row. Workflow passes `7` and `7`; every other type keeps the
/// defaults (`8`, `22`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub font_size: f64,
    pub item_gap: f64,
}

impl Default for Style {
    fn default() -> Self {
        Style { font_size: FONT_SIZE, item_gap: ITEM_GAP }
    }
}

/// Greedy wrap into rows no wider than `width`: an entry that does not fit starts a new row.
pub fn footprint(entries: &[Entry], width: f64) -> Footprint {
    footprint_styled(entries, width, Style::default())
}

/// [`footprint`] at an explicit [`Style`].
pub fn footprint_styled(entries: &[Entry], width: f64, style: Style) -> Footprint {
    if entries.is_empty() {
        return Footprint { widths: Vec::new(), rows: Vec::new(), row_count: 0, min_width: 0.0, extra_height: 0.0 };
    }
    let widths: Vec<f64> = entries.iter().map(|e| entry_width(e, style.font_size)).collect();
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut cursor = 0.0;
    for (i, w) in widths.iter().enumerate() {
        let row = rows.last_mut().expect("rows is never empty");
        let required = (if row.is_empty() { 0.0 } else { style.item_gap }) + w;
        if !row.is_empty() && cursor + required > width {
            rows.push(vec![i]);
            cursor = *w;
        } else {
            row.push(i);
            cursor += required;
        }
    }
    Footprint {
        min_width: widths.iter().copied().fold(0.0, f64::max),
        extra_height: (rows.len() - 1) as f64 * LINE_GAP,
        row_count: rows.len(),
        widths,
        rows,
    }
}

/// What to do when the legend does not fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unfit {
    /// An implicit legend (the document has no `meta.legend`) is dropped.
    Hide,
    /// An authored legend is an error.
    Error,
}

/// The band a legend is measured into (`measureLegend`'s options).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub x: f64,
    pub baseline_y: f64,
    pub width: f64,
    pub min_title_y: f64,
    pub unfit: Unfit,
    pub diagram_type: &'static str,
}

/// An entry with its place: `x`, the `baseline` of its row and its measured `width`.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub entry: Entry,
    pub width: f64,
    pub x: f64,
    pub baseline: f64,
    pub row: usize,
}

impl Placed {
    /// `x + swatchWidth + swatchGap`, where the text starts.
    pub fn text_x(&self) -> f64 {
        self.x + self.entry.swatch_w() + self.entry.swatch_g()
    }

    /// The `legendRects` box of the entry (`y = baseline - 10`, `h = 14`).
    pub fn rect(&self) -> Bounds {
        Bounds::new(self.x, self.baseline - 10.0, self.width, 14.0)
    }
}

/// The placed legend.
#[derive(Clone, Debug, PartialEq)]
pub struct Measured {
    pub entries: Vec<Placed>,
    pub row_count: usize,
    pub title_x: f64,
    pub title_y: f64,
    pub font_size: f64,
}

impl Measured {
    /// `renderedFontSize`: `+0.5` below 8, else `+2`.
    pub fn rendered_font_size(&self) -> f64 {
        if self.font_size < 8.0 { self.font_size + 0.5 } else { self.font_size + 2.0 }
    }

    /// Whether any entry carries the viewer filter badge (`data-legend-bridge`).
    pub fn has_interactive(&self) -> bool {
        self.entries.iter().any(|p| p.entry.interactive)
    }
}

fn legend_error(layout: &Layout, code: &str, message: String, path: String, evidence: Value, fix: &str) -> Box<Diagnostic> {
    let Value::Object(evidence) = evidence else { unreachable!("evidence is built as an object") };
    Box::new(
        // Archify writes the code into the sentence (`[legend/content-overlap] architecture legend ...`).
        Diagnostic::error(code, &format!("[{code}] {message}"))
            .with_subject(Subject::of(layout.diagram_type).with_path(path))
            .with_evidence(evidence)
            .with_fixes([fix]),
    )
}

fn obj<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Map<_, _>>())
}

/// `measureLegend` for a type that passes no obstacles (dataflow, lifecycle, sequence).
pub fn measure(entries: &[Entry], layout: &Layout) -> Result<Option<Measured>, Box<Diagnostic>> {
    measure_with(entries, layout, &[])
}

/// `measureLegend`: `Ok(None)` when an implicit legend is dropped (or empty), `Err` for an authored
/// legend that does not fit (`legend/label-too-wide`, `legend/vertical-overflow`) or that lies on an
/// obstacle (`legend/content-overlap`, architecture and workflow). The first legend rect (the title,
/// then the entries in order) that any obstacle touches is the one reported; a segment is tested
/// with `segmentIntersectsRect`, a rect with `rectsOverlap`, both at gap 0.
pub fn measure_with(entries: &[Entry], layout: &Layout, obstacles: &[Obstacle]) -> Result<Option<Measured>, Box<Diagnostic>> {
    measure_styled(entries, layout, obstacles, Style::default())
}

/// [`measure_with`] at an explicit [`Style`] (workflow: font `7`, item gap `7`).
pub fn measure_styled(
    entries: &[Entry],
    layout: &Layout,
    obstacles: &[Obstacle],
    style: Style,
) -> Result<Option<Measured>, Box<Diagnostic>> {
    if entries.is_empty() {
        return Ok(None);
    }
    let fp = footprint_styled(entries, layout.width, style);
    let dt = layout.diagram_type;
    if let Some((i, w)) = fp.widths.iter().copied().enumerate().find(|(_, w)| *w > layout.width) {
        if layout.unfit == Unfit::Hide {
            return Ok(None);
        }
        let kind = entries[i].kind;
        return Err(legend_error(
            layout,
            "legend/label-too-wide",
            format!("{dt} legend label for \"{kind}\" needs {w}px but only {}px is available.", layout.width),
            format!("/meta/legend/entries/{kind}/label"),
            obj([
                ("kind", kind.into()),
                ("measuredWidthPx", crate::diag::json_num(w)),
                ("availableWidthPx", crate::diag::json_num(layout.width)),
            ]),
            "shorten the legend label or use a wider viewBox",
        ));
    }

    let title_y = layout.baseline_y - fp.extra_height - TITLE_RISE;
    let top_y = title_y - 10.0;
    if top_y < layout.min_title_y {
        if layout.unfit == Unfit::Hide {
            return Ok(None);
        }
        return Err(legend_error(
            layout,
            "legend/vertical-overflow",
            format!(
                "{dt} legend needs {} rows, which would start at y={top_y} above the available legend band at y={}.",
                fp.row_count, layout.min_title_y
            ),
            "/meta/legend".to_owned(),
            obj([
                ("rowCount", fp.row_count.into()),
                ("requiredTopY", crate::diag::json_num(top_y)),
                ("availableTopY", crate::diag::json_num(layout.min_title_y)),
            ]),
            "shorten legend labels, hide nonessential entries, or use a wider viewBox",
        ));
    }

    let mut placed = Vec::with_capacity(entries.len());
    for (row_index, row) in fp.rows.iter().enumerate() {
        let mut x = layout.x;
        let baseline = layout.baseline_y - (fp.row_count - row_index - 1) as f64 * LINE_GAP;
        for &i in row {
            placed.push(Placed { entry: entries[i].clone(), width: fp.widths[i], x, baseline, row: row_index });
            x += fp.widths[i] + style.item_gap;
        }
    }
    if !obstacles.is_empty() {
        let title = ("title", Rect::new(layout.x, top_y, TITLE_RECT_WIDTH, TITLE_RECT_HEIGHT));
        let rects = std::iter::once(title).chain(placed.iter().map(|p| (p.entry.kind, p.rect().rect())));
        let hit = rects.into_iter().find(|(_, rect)| {
            obstacles.iter().any(|o| match o {
                Obstacle::Segment(seg) => segment_intersects_rect(seg, rect, 0.0),
                Obstacle::Rect(r) => rects_overlap(r, rect, 0.0),
            })
        });
        if let Some((kind, rect)) = hit {
            if layout.unfit == Unfit::Hide {
                return Ok(None);
            }
            return Err(legend_error(
                layout,
                "legend/content-overlap",
                format!("{dt} legend entry \"{kind}\" overlaps authored relationship geometry."),
                "/meta/legend".to_owned(),
                obj([
                    ("legendKind", kind.into()),
                    (
                        "legendRect",
                        obj([
                            ("kind", kind.into()),
                            ("x", crate::diag::json_num(rect.x)),
                            ("y", crate::diag::json_num(rect.y)),
                            ("width", crate::diag::json_num(rect.width)),
                            ("height", crate::diag::json_num(rect.height)),
                        ]),
                    ),
                ]),
                "shorten or hide legend entries, use a wider viewBox, or move the authored relationship route/label out of the legend band",
            ));
        }
    }

    Ok(Some(Measured { entries: placed, row_count: fp.row_count, title_x: layout.x, title_y, font_size: style.font_size }))
}

/// `renderLegend` into the scene: the title, then per entry a `Legend` group holding the swatch the
/// caller draws and the label. `swatch` returns the entry's swatch shapes (the only per-type part).
pub fn push_scene(b: &mut SceneBuilder, m: &Measured, l: &Locale, swatch: &dyn Fn(&Placed) -> Vec<Shape>) {
    b.push(
        Layer::Legend,
        Shape::Text(TextShape {
            at: [m.title_x, m.title_y],
            text: l.legend_title(),
            size: TITLE_SIZE,
            weight: TITLE_WEIGHT,
            anchor: Anchor::Start,
            token: Token::Text,
            detail: Detail::Anchor,
        }),
    );
    for p in &m.entries {
        let mut group = Group::new(GroupKind::Legend, format!("legend-{}", p.entry.kind), p.rect());
        group.label = p.entry.label.clone();
        let id = b.group(group);
        for shape in swatch(p) {
            b.push_in(id, Layer::Legend, shape);
        }
        b.push_in(
            id,
            Layer::Legend,
            Shape::Text(TextShape {
                at: [p.text_x(), p.baseline],
                text: p.entry.label.clone(),
                size: m.rendered_font_size(),
                weight: ENTRY_WEIGHT,
                anchor: Anchor::Start,
                token: Token::TextMuted,
                detail: Detail::Anchor,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &'static str, label: &str, interactive: bool) -> Entry {
        Entry {
            kind,
            label: label.to_owned(),
            swatch_width: Some(34.0),
            swatch_gap: Some(9.0),
            present: true,
            interactive,
        }
    }

    #[test]
    fn entry_width_is_ceil_of_swatch_gap_and_text() {
        // 34 + 9 + 12 * 8 * 0.62 = 102.52
        assert_eq!(entry_width(&entry("emphasis", "primary data", false), 8.0), 103.0);
        // + 21 for the badge: 123.52
        assert_eq!(entry_width(&entry("emphasis", "primary data", true), 8.0), 124.0);
    }

    #[test]
    fn rows_wrap_greedily_and_stack_upward() {
        let es = vec![entry("a", "primary data", false), entry("b", "policy / PII", false), entry("c", "async batch", false)];
        // 103 + 22 + 103 = 228 fits 230; the third (98) starts a row.
        let fp = footprint(&es, 230.0);
        assert_eq!(fp.rows, vec![vec![0, 1], vec![2]]);
        assert_eq!(fp.extra_height, 22.0);
        let m = measure(
            &es,
            &Layout { x: 40.0, baseline_y: 100.0, width: 230.0, min_title_y: 0.0, unfit: Unfit::Error, diagram_type: "dataflow" },
        )
        .unwrap()
        .unwrap();
        assert_eq!(m.title_y, 100.0 - 22.0 - 20.0);
        assert_eq!(m.entries[0].baseline, 78.0);
        assert_eq!(m.entries[2].baseline, 100.0);
        assert_eq!(m.entries[1].x, 40.0 + 103.0 + 22.0);
    }

    #[test]
    fn an_unfit_legend_is_hidden_or_an_error() {
        let es = vec![entry("a", "primary data", false)];
        let mut layout =
            Layout { x: 40.0, baseline_y: 60.0, width: 100.0, min_title_y: 0.0, unfit: Unfit::Hide, diagram_type: "dataflow" };
        assert_eq!(measure(&es, &layout), Ok(None));
        layout.unfit = Unfit::Error;
        assert_eq!(measure(&es, &layout).unwrap_err().code, "legend/label-too-wide");
        layout.width = 400.0;
        layout.min_title_y = 50.0;
        assert_eq!(measure(&es, &layout).unwrap_err().code, "legend/vertical-overflow");
    }

    #[test]
    fn resolve_follows_mode_and_overrides() {
        let cat = dataflow_catalog(&Locale::default());
        let present: HashSet<&str> = ["default", "emphasis"].into_iter().collect();
        let kinds = |c: Option<&Config>| resolve(c, &cat, &present).iter().map(|e| e.kind).collect::<Vec<_>>();
        assert_eq!(kinds(None), ["emphasis", "default"]);
        let all = Config { mode: LegendMode::All, entries: HashMap::new() };
        assert_eq!(kinds(Some(&all)).len(), 5);
        let hidden = Config { mode: LegendMode::Hidden, entries: HashMap::new() };
        assert!(kinds(Some(&hidden)).is_empty());
        let mut entries = HashMap::new();
        entries.insert("default".to_owned(), Override { label: Some("flow".into()), visible: Some(false) });
        entries.insert("dashed".to_owned(), Override { label: None, visible: Some(true) });
        let cfg = Config { mode: LegendMode::Auto, entries };
        assert_eq!(kinds(Some(&cfg)), ["emphasis", "dashed"]);
    }
}
