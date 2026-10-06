//! Architecture placement (P3.1): grid and free placement, boundary frames, the boundary-title
//! rail, the auto viewBox and the legend footprint. Port of `renderers/architecture/{grid,
//! render-architecture}.mjs` and the architecture side of `shared/legend.mjs`; `00` §5.5, §5.8,
//! `03` §2.2.
//!
//! Archify's renderer is a module-scope script whose canvas depends on its own routes: the auto
//! viewBox covers every route point and label rect, and the title convergence measures that width.
//! So placement is two calls, with the router between them:
//!
//! 1. [`prepare`]: components, the member-derived boundary frames ([`Prepared::raw_boundaries`], the
//!    only frames the router sees, `frames:` of `createRouter`) and the resolved legend entries.
//! 2. route and measure connection labels, then [`Prepared::finish`] with the extra rects
//!    (`connectionGeometry`: every label rect, and every route point as a `0 x 0` rect).
//!
//! [`place`] is both calls for a caller that has no routes (an authored viewBox that the routes
//! cannot move, or a document without connections).
//!
//! The scene half (`begin_scene`, [`push_components`], [`measure_legend`] and [`push_legend`]) is
//! split the same way: the router stage calls them around its edges and labels. Brand badges are
//! drawn after the sigil (`brand::push_badge`); the presets are `tokens::restyle`'s.
//!
//! The validation of `validateArchitecture` (bounds, overlap, titles, ...) is `gates::architecture`; only what makes a
//! placement impossible is reported here ([`grid_problems`] is `validateGridPlacement`).

use std::collections::{HashMap, HashSet};

use crate::diag::Diagnostic;
use crate::geom::{Pt, Rect, rects_overlap};
use crate::legend::{self, Entry, Footprint, Layout as LegendLayout, Measured, Obstacle, Placed, Unfit};
use crate::i18n::Locale;
use crate::model::architecture::{Architecture, ArchitectureMeta, BoundaryKind, Component};
use crate::model::common::{ComponentType, LegendMode, NodeIcon};
use crate::scene::{
    Anchor, Bounds, Detail, Fill, Group, GroupId, GroupKind, Layer, RectShape, SceneBuilder, Shape, Stroke, TextShape,
};
use crate::sigils;
use crate::text::{self, DiagramType, boundary_label_width};
use crate::tokens::{Kind, Token};

/// `DEFAULT_GRID` (`grid.mjs:3-11`): origin `[40, 80]`, 4 columns, gaps `30` / `40`, cell `130 x 64`.
pub const GRID_ORIGIN: Pt = [40.0, 80.0];
pub const GRID_COLS: f64 = 4.0;
pub const GRID_GAP_X: f64 = 30.0;
pub const GRID_GAP_Y: f64 = 40.0;
pub const GRID_CELL_W: f64 = 130.0;
pub const GRID_CELL_H: f64 = 64.0;

/// `layout.defaultW` / `defaultH` (`render-architecture.mjs:61-62`): the size of a component with no `size`.
pub const DEFAULT_W: f64 = 120.0;
pub const DEFAULT_H: f64 = 60.0;
/// `layout.margin` (`:63`): the canvas margin right and below, and the legend side margin.
pub const MARGIN: f64 = 40.0;
/// `layout.boundaryPad` (`:66`) and `boundaryExtraBottom` (`:67`).
pub const BOUNDARY_PAD: f64 = 30.0;
pub const BOUNDARY_EXTRA_BOTTOM: f64 = 20.0;
/// `layout.boundaryLabelBaseline` (`:68`) and `boundaryLabelClearance` (`:69`): `topPad >= 18 + 4`.
pub const BOUNDARY_LABEL_BASELINE: f64 = 18.0;
pub const BOUNDARY_LABEL_CLEARANCE: f64 = 4.0;
/// `layout.boundaryLabelFontPreferred` / `Minimum` (`:70-71`).
pub const BOUNDARY_FONT_PREFERRED: f64 = 9.0;
pub const BOUNDARY_FONT_MINIMUM: f64 = 6.0;
/// `layout.boundaryLabelMaskHeight` (`:72`), `RailGap` (`:73`) and `FrameInset` (`:74`).
pub const BOUNDARY_MASK_HEIGHT: f64 = 16.0;
pub const BOUNDARY_RAIL_GAP: f64 = 2.0;
pub const BOUNDARY_FRAME_INSET: f64 = 4.0;
/// `layout.legendH` (`:75`): the band the auto canvas reserves below the content.
pub const LEGEND_H: f64 = 28.0;
/// `legendY() = viewBox[1] - 16` (`:404`).
pub const LEGEND_BASELINE_UP: f64 = 16.0;
/// The title may not start above `contentBottom + 8` (`:925`).
pub const LEGEND_CONTENT_GAP: f64 = 8.0;
/// `maximumIterations` of `resolveBoundaryTitles` (`:355`).
pub const MAX_TITLE_ITERATIONS: usize = 32;
/// `DESKTOP_READER_DIAGRAM_WIDTH` and `MIN_PROJECTED_NODE_TEXT_PX` (`desktop-readability.mjs:4,7`).
pub const READER_DIAGRAM_WIDTH: f64 = 930.0;
pub const MIN_PROJECTED_TEXT_PX: f64 = 6.0;
/// The `+ 1e-6` the budget pass adds to the minimum font (`:361`; only the budget pass, not the final check).
pub const MINIMUM_FONT_EPSILON: f64 = 1e-6;
/// The width Archify clamps `viewBox` and the legend to when asking for a footprint (`Math.max(1, ...)`).
const MIN_FOOTPRINT_WIDTH: f64 = 1.0;

/// Frame radius: `12` for a region, `8` for a security group (`:843`). Dashes `8,4` and `4,4`
/// (`template.html:4695-4696`), 1 wide.
pub const REGION_RADIUS: f64 = 12.0;
pub const SECURITY_GROUP_RADIUS: f64 = 8.0;
pub const REGION_DASH: [f64; 2] = [8.0, 4.0];
pub const SECURITY_GROUP_DASH: [f64; 2] = [4.0, 4.0];
pub const FRAME_STROKE: f64 = 1.0;
/// The title mask radius 3 (`:850`), text weight 600 (`:851`).
pub const TITLE_RADIUS: f64 = 3.0;
pub const TITLE_WEIGHT: u16 = 600;
/// Node box: radius 6, stroke 1.5 (`:898-900`); the sigil sits at `+6, +6`, 11 wide.
pub const NODE_RADIUS: f64 = 6.0;
pub const NODE_STROKE: f64 = 1.5;
pub const SIGIL_AT: f64 = 6.0;
/// Text rows (`:886-891`): label `cy - 2` with a sublabel else `cy + 4`; sublabel `cy + 14`; tag `y + h - 8`.
pub const LABEL_UP_WITH_SUBLABEL: f64 = 2.0;
pub const LABEL_DOWN: f64 = 4.0;
pub const SUBLABEL_DOWN: f64 = 14.0;
pub const TAG_UP: f64 = 8.0;
/// Legend swatch: a `16 x 10` rect, radius 2.5, stroke 1, its top `9` above the baseline (`:930`).
pub const LEGEND_SWATCH: (f64, f64, f64) = (16.0, 10.0, 2.5);

/// The resolved grid (`gridLayout`): `DEFAULT_GRID` overridden by the authored fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub origin: Pt,
    pub cols: f64,
    pub gap_x: f64,
    pub gap_y: f64,
    pub cell_w: f64,
    pub cell_h: f64,
}

/// `gridLayout(arch)`: `None` unless `layout.mode` is `grid`.
pub fn grid_of(doc: &Architecture) -> Option<Grid> {
    let l = doc.layout.as_ref()?;
    Some(Grid {
        origin: l.origin.unwrap_or(GRID_ORIGIN),
        cols: l.cols.unwrap_or(GRID_COLS),
        gap_x: l.gap_x.unwrap_or(GRID_GAP_X),
        gap_y: l.gap_y.unwrap_or(GRID_GAP_Y),
        cell_w: l.cell_w.unwrap_or(GRID_CELL_W),
        cell_h: l.cell_h.unwrap_or(GRID_CELL_H),
    })
}

fn is_integer(v: Option<f64>) -> Option<f64> {
    v.filter(|n| n.is_finite() && n.fract() == 0.0)
}

/// `resolveComponentPos`: `pos` wins; else the grid cell of an integer `row` and `col`; else `[NaN, NaN]`
/// (a gate reports it).
pub fn resolve_pos(c: &Component, grid: Option<&Grid>) -> Pt {
    if let Some(pos) = c.pos {
        return pos;
    }
    let (Some(g), Some(row), Some(col)) = (grid, is_integer(c.row), is_integer(c.col)) else { return [f64::NAN, f64::NAN] };
    [g.origin[0] + col * (g.cell_w + g.gap_x), g.origin[1] + row * (g.cell_h + g.gap_y)]
}

/// `validateGridPlacement`: the placement problems of a grid document (the message texts of
/// `grid.mjs:35-61`), in order. Empty without a grid; the free-placement message lives with the gate.
pub fn grid_problems(doc: &Architecture) -> Vec<String> {
    let Some(grid) = grid_of(doc) else { return Vec::new() };
    let mut problems = Vec::new();
    let mut seen: HashMap<(i64, i64), &str> = HashMap::new();
    for c in &doc.components {
        if c.pos.is_some() {
            continue;
        }
        let (Some(row), Some(col)) = (is_integer(c.row), is_integer(c.col)) else {
            problems.push(format!("Component \"{}\" needs pos [x,y] or grid row/col when layout.mode is \"grid\".", c.id));
            continue;
        };
        if row < 0.0 || col < 0.0 {
            problems.push(format!("Component \"{}\" row/col must be non-negative integers.", c.id));
            continue;
        }
        if col >= grid.cols {
            problems.push(format!(
                "Component \"{}\" col {col} exceeds layout.cols {} (valid: 0..{}).",
                c.id,
                grid.cols,
                grid.cols - 1.0
            ));
        }
        match seen.get(&(row as i64, col as i64)) {
            Some(first) => problems.push(format!("Components \"{first}\" and \"{}\" share grid cell row {row} col {col}.", c.id)),
            None => {
                seen.insert((row as i64, col as i64), &c.id);
            }
        }
    }
    problems
}

/// A measured component: the rect the router and the gates see. `index` is its place in
/// `components` (for a duplicate id, the last one wins and keeps the first one's slot, as a JS `Map`).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedComponent {
    pub id: String,
    pub kind: ComponentType,
    pub rect: Rect,
    pub index: usize,
}

/// The member-derived frame of a boundary (`boundaryRect`), before any title work.
#[derive(Clone, Debug, PartialEq)]
pub struct RawBoundary {
    /// The boundary's index in `boundaries` (a boundary with no known member is dropped).
    pub source: usize,
    pub kind: BoundaryKind,
    pub label: String,
    pub wraps: Vec<String>,
    pub rect: Rect,
    /// `minY` of the members: the title rail sits `4 + height` above it.
    pub member_top: f64,
}

impl RawBoundary {
    /// `radius: boundary.kind === 'security-group' ? 8 : 12`.
    pub fn radius(&self) -> f64 {
        match self.kind {
            BoundaryKind::SecurityGroup => SECURITY_GROUP_RADIUS,
            BoundaryKind::Region => REGION_RADIUS,
        }
    }
}

/// `measureBoundaryTitle`: the mask rect and the fitted font.
#[derive(Clone, Debug, PartialEq)]
pub struct Title {
    pub rect: Rect,
    pub font_size: f64,
    pub minimum_font_size: f64,
    /// The text baseline sits `fontSize + 4` below the mask top; its `x` is `rect.x + 4`.
    pub baseline_offset: f64,
    pub available_width: f64,
    pub minimum_width: f64,
}

impl Title {
    pub fn text_at(&self) -> Pt {
        [self.rect.x + BOUNDARY_FRAME_INSET, self.rect.y + self.baseline_offset]
    }
}

/// A boundary after the title work: the final frame (extended up to hold the title, and widened,
/// under a quality profile) and its title.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedBoundary {
    pub raw: RawBoundary,
    pub rect: Rect,
    pub title: Title,
}

/// The stage-1 result: everything that does not depend on the routes.
#[derive(Clone, Debug)]
pub struct Prepared<'a> {
    doc: &'a Architecture,
    pub components: Vec<PlacedComponent>,
    pub raw_boundaries: Vec<RawBoundary>,
    pub legend_entries: Vec<Entry>,
    /// `Boolean(meta.quality_profile)`: the title-composition contract (the `--quality` override of
    /// the CLI does not reach the renderer, so it does not switch this).
    pub enforces_titles: bool,
}

/// The final placement the router consumes (and the scene emitters read).
#[derive(Clone, Debug, PartialEq)]
pub struct ArchitecturePlacement {
    pub components: Vec<PlacedComponent>,
    /// The router's `frames` (member-derived, never the title-extended ones).
    pub raw_boundaries: Vec<RawBoundary>,
    pub boundaries: Vec<PlacedBoundary>,
    pub view_box: [f64; 2],
    /// `meta.viewBox` was set: it is honoured, never grown.
    pub view_box_authored: bool,
    pub legend_entries: Vec<Entry>,
    /// The footprint of the entries at the final width (`viewBox[0] - 80`).
    pub legend_footprint: Footprint,
    /// `meta.legend` is set: an unfit legend is an error, not dropped.
    pub legend_authored: bool,
    /// `[composition/desktop-readability] ...` when the title width did not converge.
    pub readability_problem: Option<String>,
    pub enforces_titles: bool,
    /// `node.context.architecture` in the document's locale: the context of a component no
    /// boundary wraps.
    pub default_context: String,
}

fn legend_config(meta: &ArchitectureMeta) -> Option<legend::Config> {
    let l = meta.legend.as_ref()?;
    let mut entries = HashMap::new();
    if let Some(e) = &l.entries {
        for (kind, entry) in [
            ("frontend", &e.frontend),
            ("backend", &e.backend),
            ("database", &e.database),
            ("cloud", &e.cloud),
            ("security", &e.security),
            ("messagebus", &e.messagebus),
            ("external", &e.external),
        ] {
            if let Some(o) = entry {
                entries.insert(kind.to_owned(), legend::Override { label: o.label.clone(), visible: o.visible });
            }
        }
    }
    Some(legend::Config { mode: l.mode.unwrap_or(LegendMode::Auto), entries })
}

/// `Math.max` / `Math.min`: a NaN poisons the result (Rust's `f64::max` would skip it).
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

/// `measureComponent`: `size` or 120 x 60 at the resolved position.
fn measure_component(c: &Component, grid: Option<&Grid>) -> Rect {
    let [x, y] = resolve_pos(c, grid);
    let [w, h] = c.size.unwrap_or([DEFAULT_W, DEFAULT_H]);
    Rect::new(x, y, w, h)
}

/// `boundaryRect`: the bbox of the known members plus `pad` (`30`) left, right and top (the top at
/// least `18 + 4`), and `pad` + `20` below. `None` when no member exists.
fn boundary_rect(source: usize, b: &crate::model::architecture::Boundary, components: &[PlacedComponent]) -> Option<RawBoundary> {
    let members: Vec<&Rect> = b.wraps.iter().filter_map(|id| components.iter().find(|c| &c.id == id)).map(|c| &c.rect).collect();
    if members.is_empty() {
        return None;
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for m in members {
        min_x = js_min(min_x, m.x);
        min_y = js_min(min_y, m.y);
        max_x = js_max(max_x, m.x + m.width);
        max_y = js_max(max_y, m.y + m.height);
    }
    let pad = b.pad.unwrap_or(BOUNDARY_PAD);
    let top_pad = js_max(pad, BOUNDARY_LABEL_BASELINE + BOUNDARY_LABEL_CLEARANCE);
    Some(RawBoundary {
        source,
        kind: b.kind,
        label: b.label.clone(),
        wraps: b.wraps.clone(),
        rect: Rect::new(min_x - pad, min_y - top_pad, max_x - min_x + pad * 2.0, max_y - min_y + top_pad + BOUNDARY_EXTRA_BOTTOM),
        member_top: min_y,
    })
}

/// Stage 1: place the components, derive the raw boundary frames and resolve the legend.
pub fn prepare(doc: &Architecture) -> Prepared<'_> {
    let grid = grid_of(doc);
    let mut components: Vec<PlacedComponent> = Vec::with_capacity(doc.components.len());
    for (index, c) in doc.components.iter().enumerate() {
        let placed = PlacedComponent { id: c.id.clone(), kind: c.kind, rect: measure_component(c, grid.as_ref()), index };
        match components.iter_mut().find(|p| p.id == c.id) {
            Some(slot) => {
                slot.kind = placed.kind;
                slot.rect = placed.rect;
                slot.index = index;
            }
            None => components.push(placed),
        }
    }
    let raw_boundaries: Vec<RawBoundary> = doc
        .boundaries
        .iter()
        .flatten()
        .enumerate()
        .filter_map(|(source, b)| boundary_rect(source, b, &components))
        .collect();
    let present: HashSet<&str> = components.iter().map(|c| kind_of(c.kind).as_str()).collect();
    let config = legend_config(&doc.meta);
    let legend_entries = legend::resolve(config.as_ref(), &legend::architecture_catalog(&doc.meta.locale()), &present);
    Prepared { doc, components, raw_boundaries, legend_entries, enforces_titles: doc.meta.quality_profile.is_some() }
}

/// `minimumReadableSourceTextPx(viewBoxWidth)`: `6 / min(1, 930 / width)`; NaN for a width that is
/// not positive and finite.
pub fn minimum_readable_source_text_px(view_box_width: f64) -> f64 {
    if !(view_box_width.is_finite() && view_box_width > 0.0) {
        return f64::NAN;
    }
    MIN_PROJECTED_TEXT_PX / (READER_DIAGRAM_WIDTH / view_box_width).min(1.0)
}

/// `autoViewBoxFor`: `w = ceil(maxRight + 40)` over components, boundaries and `extra`; the legend
/// may force it wider (`minWidth + 80`); `h = ceil(maxBottom + 40 + 28 + legend extra height)`.
pub fn auto_view_box_for(components: &[PlacedComponent], boundaries: &[Rect], extra: &[Rect], entries: &[Entry]) -> [f64; 2] {
    let (mut max_x, mut max_y) = (0.0, 0.0);
    for r in components.iter().map(|c| &c.rect).chain(boundaries).chain(extra) {
        max_x = js_max(max_x, r.x + r.width);
        max_y = js_max(max_y, r.y + r.height);
    }
    let mut width = f64::ceil(max_x + MARGIN);
    let mut footprint = legend::footprint(entries, js_max(MIN_FOOTPRINT_WIDTH, width - MARGIN * 2.0));
    if footprint.min_width > width - MARGIN * 2.0 {
        width = f64::ceil(footprint.min_width + MARGIN * 2.0);
        footprint = legend::footprint(entries, width - MARGIN * 2.0);
    }
    [width, f64::ceil(max_y + MARGIN + LEGEND_H + footprint.extra_height)]
}

fn horizontal_overlap(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.width && a.x + a.width > b.x
}

/// `measureBoundaryTitle`: the font fits the frame (`(avail - 10) / (0.6 u)`) between the minimum and
/// `max(9, minimum)`; the rail sits `4` above the member top; the mask is `max(16, ceil(fs + 7))` high
/// and `min(avail, titleWidth)` wide.
fn measure_title(frame: &Rect, label: &str, member_top: f64, minimum_font: f64) -> Title {
    let available = js_max(0.0, frame.width - BOUNDARY_FRAME_INSET * 2.0);
    let units = text::units(label) as f64;
    let fitted = if units > 0.0 { (available - 10.0) / (units * text::WIDTH_FACTOR) } else { BOUNDARY_FONT_PREFERRED };
    let preferred = js_max(BOUNDARY_FONT_PREFERRED, minimum_font);
    let font_size = js_max(minimum_font, js_min(preferred, fitted));
    let desired = boundary_label_width(label, font_size);
    let height = js_max(BOUNDARY_MASK_HEIGHT, f64::ceil(font_size + 7.0));
    Title {
        rect: Rect::new(
            frame.x + BOUNDARY_FRAME_INSET,
            member_top - BOUNDARY_LABEL_CLEARANCE - height,
            js_min(available, desired),
            height,
        ),
        font_size,
        minimum_font_size: minimum_font,
        baseline_offset: font_size + 4.0,
        available_width: available,
        minimum_width: boundary_label_width(label, minimum_font),
    }
}

impl<'a> Prepared<'a> {
    /// `layoutBoundaryTitles(rawBoundaries, minimumFontSize)`: boundaries in ascending frame area
    /// (ties by order) are widened to hold their title at the minimum font (profile only), measured,
    /// and the title is lifted above any placed title or component it overlaps; then, under a profile,
    /// the frame is extended up to `title.y - 4`.
    pub fn layout_titles(&self, minimum_font: f64) -> Vec<PlacedBoundary> {
        let raws = &self.raw_boundaries;
        let mut order: Vec<usize> = (0..raws.len()).collect();
        order.sort_by(|&a, &b| {
            let area = |i: usize| raws[i].rect.width * raws[i].rect.height;
            area(a).partial_cmp(&area(b)).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b))
        });
        let mut placed: Vec<Rect> = Vec::with_capacity(raws.len());
        let mut measured: Vec<Option<(Rect, Title)>> = vec![None; raws.len()];
        for index in order {
            let raw = &raws[index];
            let mut frame = raw.rect;
            if self.enforces_titles {
                let required = boundary_label_width(&raw.label, minimum_font) + BOUNDARY_FRAME_INSET * 2.0;
                let extra = js_max(0.0, required - frame.width);
                if extra != 0.0 {
                    frame.x -= extra / 2.0;
                    frame.width += extra;
                }
            }
            let mut title = measure_title(&frame, &raw.label, raw.member_top, minimum_font);
            let mut guard = 0;
            while guard < raws.len() + self.components.len() + 1 {
                guard += 1;
                let blockers: Vec<&Rect> = placed
                    .iter()
                    .chain(self.components.iter().map(|c| &c.rect))
                    .filter(|c| horizontal_overlap(&title.rect, c) && rects_overlap(&title.rect, c, 0.0))
                    .collect();
                if blockers.is_empty() {
                    break;
                }
                title.rect.y = blockers.iter().fold(f64::INFINITY, |min, b| js_min(min, b.y - BOUNDARY_RAIL_GAP - title.rect.height));
            }
            placed.push(title.rect);
            measured[index] = Some((frame, title));
        }
        raws.iter()
            .zip(measured)
            .map(|(raw, m)| {
                let (frame, title) = m.expect("every boundary was measured");
                let bottom = frame.y + frame.height;
                let y = if self.enforces_titles { js_min(frame.y, title.rect.y - BOUNDARY_FRAME_INSET) } else { frame.y };
                PlacedBoundary { raw: raw.clone(), rect: Rect::new(frame.x, y, frame.width, bottom - y), title }
            })
            .collect()
    }

    fn view_box_width(&self, candidate: &[Rect], extra: &[Rect]) -> f64 {
        match self.doc.meta.view_box {
            Some(vb) if vb[0].is_finite() => vb[0],
            _ => auto_view_box_for(&self.components, candidate, extra, &self.legend_entries)[0],
        }
    }

    /// `resolveBoundaryTitles`: without a profile (or boundaries) one pass at the minimum font 6.
    /// Under a profile the title <-> canvas width <-> minimum font loop runs at most 32 times: lay
    /// out at the font the *candidate* canvas needs (+1e-6), measure the canvas that layout gives,
    /// stop when the font used is at least the font that canvas needs, else retry on that layout.
    /// A loop that does not settle returns the last layout and the readability problem.
    pub fn resolve_titles(&self, extra: &[Rect]) -> (Vec<PlacedBoundary>, Option<String>) {
        if !self.enforces_titles || self.raw_boundaries.is_empty() {
            return (self.layout_titles(BOUNDARY_FONT_MINIMUM), None);
        }
        let mut candidate: Vec<Rect> = self.raw_boundaries.iter().map(|b| b.rect).collect();
        let mut last: Option<Vec<PlacedBoundary>> = None;
        for _ in 0..MAX_TITLE_ITERATIONS {
            let budget = self.view_box_width(&candidate, extra);
            let minimum = js_max(BOUNDARY_FONT_MINIMUM, minimum_readable_source_text_px(budget) + MINIMUM_FONT_EPSILON);
            let next = self.layout_titles(minimum);
            let next_rects: Vec<Rect> = next.iter().map(|b| b.rect).collect();
            let final_width = self.view_box_width(&next_rects, extra);
            let final_minimum = js_max(BOUNDARY_FONT_MINIMUM, minimum_readable_source_text_px(final_width));
            if minimum >= final_minimum {
                return (next, None);
            }
            candidate = next_rects;
            last = Some(next);
        }
        let width = self.view_box_width(&candidate, extra);
        let problem = format!(
            "[composition/desktop-readability] Boundary title layout did not converge after {MAX_TITLE_ITERATIONS} iterations for the final {width}px viewBox \u{2014} shorten boundary labels, provide a wider authored viewBox, or move wrapped components closer to the left edge."
        );
        // `candidateBoundaries` is the last layout (the loop ran at least once).
        (last.unwrap_or_default(), Some(problem))
    }

    /// Stage 2. `extra` is `connectionGeometry`: every connection label rect and every route point
    /// as a zero-size rect, `[]` for a document with no connections.
    pub fn finish(&self, extra: &[Rect]) -> ArchitecturePlacement {
        let (boundaries, readability_problem) = self.resolve_titles(extra);
        let frames: Vec<Rect> = boundaries.iter().map(|b| b.rect).collect();
        let (view_box, view_box_authored) = match self.doc.meta.view_box {
            Some(vb) => (vb, true),
            None => (auto_view_box_for(&self.components, &frames, extra, &self.legend_entries), false),
        };
        ArchitecturePlacement {
            components: self.components.clone(),
            raw_boundaries: self.raw_boundaries.clone(),
            boundaries,
            view_box,
            view_box_authored,
            legend_entries: self.legend_entries.clone(),
            legend_footprint: legend::footprint(&self.legend_entries, view_box[0] - MARGIN * 2.0),
            legend_authored: self.doc.meta.legend.is_some(),
            readability_problem,
            enforces_titles: self.enforces_titles,
            default_context: self.doc.meta.locale().t("node.context.architecture"),
        }
    }
}

/// Both stages with no routes: for a caller whose canvas the routes cannot move.
pub fn place(doc: &Architecture) -> ArchitecturePlacement {
    prepare(doc).finish(&[])
}

impl ArchitecturePlacement {
    pub fn component(&self, id: &str) -> Option<&PlacedComponent> {
        self.components.iter().find(|c| c.id == id)
    }

    /// `legendY()`: the baseline of the legend's last row.
    pub fn legend_baseline(&self) -> f64 {
        self.view_box[1] - LEGEND_BASELINE_UP
    }

    /// `contentBottom` of `renderLegend`: the lowest component or final frame.
    pub fn content_bottom(&self) -> f64 {
        self.components
            .iter()
            .map(|c| c.rect.bottom())
            .chain(self.boundaries.iter().map(|b| b.rect.bottom()))
            .fold(0.0, js_max)
    }

    /// `placementBottom` for the showcase label relocation (P3.5): the legend band stays free.
    pub fn label_placement_bottom(&self) -> f64 {
        if self.legend_entries.is_empty() {
            self.view_box[1]
        } else {
            let footprint = legend::footprint(&self.legend_entries, self.view_box[0] - MARGIN * 2.0);
            self.legend_baseline() - 32.0 - footprint.extra_height
        }
    }

    /// The final frames and the components as the obstacles the gates and label placement test
    /// (`contentRects` of the detour gate).
    pub fn content_rects(&self) -> Vec<Rect> {
        self.components.iter().map(|c| c.rect).chain(self.boundaries.iter().map(|b| b.rect)).collect()
    }

    /// `componentContext`: the labels of the final boundaries that wrap the component, largest
    /// frame first, joined by ` › `; else `Architecture component`.
    pub fn context_of(&self, id: &str) -> String {
        let mut scopes: Vec<&PlacedBoundary> = self.boundaries.iter().filter(|b| b.raw.wraps.iter().any(|w| w == id)).collect();
        scopes.sort_by(|a, b| {
            (b.rect.width * b.rect.height).partial_cmp(&(a.rect.width * a.rect.height)).unwrap_or(std::cmp::Ordering::Equal)
        });
        if scopes.is_empty() {
            self.default_context.clone()
        } else {
            scopes.iter().map(|b| b.raw.label.as_str()).collect::<Vec<_>>().join(" \u{203A} ")
        }
    }

    /// `renderLegend`'s `measureLegend` call: the legend placed in the band under the content,
    /// against the routes and labels the router produced. `Ok(None)` when no legend is drawn
    /// (hidden, empty, or an implicit one that does not fit or lies on a route).
    pub fn measure_legend(&self, obstacles: &[Obstacle]) -> Result<Option<Measured>, Box<Diagnostic>> {
        legend::measure_with(
            &self.legend_entries,
            &LegendLayout {
                x: MARGIN,
                baseline_y: self.legend_baseline(),
                width: self.view_box[0] - MARGIN * 2.0,
                min_title_y: self.content_bottom() + LEGEND_CONTENT_GAP,
                unfit: if self.legend_authored { Unfit::Error } else { Unfit::Hide },
                diagram_type: "architecture",
            },
            obstacles,
        )
    }
}

fn kind_of(t: ComponentType) -> Kind {
    match t {
        ComponentType::Frontend => Kind::Frontend,
        ComponentType::Backend => Kind::Backend,
        ComponentType::Database => Kind::Database,
        ComponentType::Cloud => Kind::Cloud,
        ComponentType::Security => Kind::Security,
        ComponentType::Messagebus => Kind::Messagebus,
        ComponentType::External => Kind::External,
    }
}

fn icon_name(icon: NodeIcon) -> String {
    serde_json::to_value(icon).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

fn text_shape(at: Pt, text: &str, size: f64, weight: u16, anchor: Anchor, token: Token, detail: Detail) -> Shape {
    Shape::Text(TextShape { at, text: text.to_owned(), size, weight, anchor, token, detail })
}

/// The scene so far: the canvas ground, the boundary frames (behind everything) and their titles
/// (the titles draw in front of the routes, the scene's layers order them).
pub fn begin_scene(p: &ArchitecturePlacement) -> SceneBuilder {
    let mut b = SceneBuilder::new(p.view_box[0], p.view_box[1]);
    b.push(
        Layer::Background,
        Shape::Rect(RectShape {
            rect: Bounds::new(0.0, 0.0, p.view_box[0], p.view_box[1]),
            radius: 0.0,
            fill: Some(Fill::new(Token::Bg)),
            stroke: None,
        }),
    );
    for (index, boundary) in p.boundaries.iter().enumerate() {
        push_boundary(&mut b, index, boundary);
    }
    b
}

fn push_boundary(b: &mut SceneBuilder, index: usize, boundary: &PlacedBoundary) {
    let (kind_name, accent, fill, radius, dash) = match boundary.raw.kind {
        BoundaryKind::Region => ("region", Kind::Cloud, Some(Fill::new(Token::RegionFill)), REGION_RADIUS, REGION_DASH),
        BoundaryKind::SecurityGroup => ("security-group", Kind::Security, None, SECURITY_GROUP_RADIUS, SECURITY_GROUP_DASH),
    };
    let mut group = Group::new(GroupKind::Frame, format!("frame-{kind_name}-{index}"), boundary.rect.into());
    group.label = boundary.raw.label.clone();
    let id = b.group(group);
    b.push_in(
        id,
        Layer::Frames,
        Shape::Rect(RectShape {
            rect: boundary.rect.into(),
            radius,
            fill,
            stroke: Some(Stroke::dashed(Token::KindStroke(accent), FRAME_STROKE, &dash)),
        }),
    );
    let title = &boundary.title;
    b.push_in(id, Layer::FrameTitles, Shape::Rect(RectShape::mask(title.rect.into(), TITLE_RADIUS)));
    b.push_in(
        id,
        Layer::FrameTitles,
        text_shape(title.text_at(), &boundary.raw.label, title.font_size, TITLE_WEIGHT, Anchor::Start, Token::KindStroke(accent), Detail::Anchor),
    );
}

/// `renderComponent` for every component, in document order: the mask, the kind fill, the sigil,
/// then the label, sublabel and tag. Brand marks are not drawn.
pub fn push_components(b: &mut SceneBuilder, doc: &Architecture, p: &ArchitecturePlacement) {
    for placed in &p.components {
        let c = &doc.components[placed.index];
        push_component(b, c, placed, p);
    }
}

fn push_component(b: &mut SceneBuilder, c: &Component, placed: &PlacedComponent, p: &ArchitecturePlacement) {
    let kind = kind_of(placed.kind);
    let r = placed.rect;
    let fonts = text::node_fonts(DiagramType::Architecture, 1);
    let sub = c.sublabel.as_deref().filter(|s| !s.is_empty());
    let tag = c.tag.as_deref().filter(|s| !s.is_empty());
    let cx = r.cx();
    let label_y = if sub.is_some() { r.y + r.height / 2.0 - LABEL_UP_WITH_SUBLABEL } else { r.y + r.height / 2.0 + LABEL_DOWN };
    let label_font = text::fit(&c.label, text::label_fit_width(r.width, c.brand.is_some()), fonts.label.0, fonts.label.1);

    let mut group = Group::node(&c.id, kind, &c.label, r.into());
    group.sublabel = sub.map(str::to_owned);
    group.context = Some(p.context_of(&c.id));
    group.tags = tag.map(str::to_owned).into_iter().collect();
    let id: GroupId = b.group(group);

    b.push_in(id, Layer::Nodes, Shape::Rect(RectShape::mask(r.into(), NODE_RADIUS)));
    b.push_in(
        id,
        Layer::Nodes,
        Shape::Rect(RectShape {
            rect: r.into(),
            radius: NODE_RADIUS,
            fill: Some(Fill::new(Token::KindFill(kind))),
            stroke: Some(Stroke::solid(Token::KindStroke(kind), NODE_STROKE)),
        }),
    );
    let icon = c.icon.map(icon_name);
    for path in sigils::sigil_paths(kind.as_str(), icon.as_deref(), r.x + SIGIL_AT, r.y + SIGIL_AT, sigils::SIGIL_SIZE) {
        b.push_in(id, Layer::Nodes, Shape::Path(path));
    }
    crate::brand::push_badge(b, id, c.brand.as_ref(), r.x + r.width, r.y);
    b.push_in(id, Layer::Nodes, text_shape([cx, label_y], &c.label, label_font, 600, Anchor::Middle, Token::Text, Detail::Anchor));
    if let Some(s) = sub {
        let (pref, min) = fonts.sublabel;
        let font = text::fit(s, r.width, pref, min);
        b.push_in(
            id,
            Layer::Nodes,
            text_shape([cx, r.y + r.height / 2.0 + SUBLABEL_DOWN], s, font, 400, Anchor::Middle, Token::TextMuted, Detail::Context),
        );
    }
    if let Some(t) = tag {
        let (pref, min) = fonts.tag.unwrap_or((7.0, 6.0));
        let font = text::fit(t, r.width, pref, min);
        b.push_in(
            id,
            Layer::Nodes,
            text_shape([cx, r.y + r.height - TAG_UP], t, font, 400, Anchor::Middle, Token::KindStroke(kind), Detail::Fine),
        );
    }
}

/// The swatch of an architecture legend row: a `16 x 10` kind rect, its top `9` above the baseline.
pub fn legend_swatch(p: &Placed) -> Vec<Shape> {
    let kind = Kind::parse(p.entry.kind).unwrap_or(Kind::External);
    let (w, h, radius) = LEGEND_SWATCH;
    vec![Shape::Rect(RectShape {
        rect: Bounds::new(p.x, p.baseline - 9.0, w, h),
        radius,
        fill: Some(Fill::new(Token::KindFill(kind))),
        stroke: Some(Stroke::solid(Token::KindStroke(kind), 1.0)),
    })]
}

/// `renderLegend` into the scene (a no-op for `None`).
pub fn push_legend(b: &mut SceneBuilder, legend: Option<&Measured>, locale: &Locale) {
    if let Some(m) = legend {
        legend::push_scene(b, m, locale, &legend_swatch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(value: serde_json::Value) -> Architecture {
        serde_json::from_value(value).expect("a valid architecture")
    }

    fn base() -> serde_json::Value {
        json!({
            "schema_version": 1, "diagram_type": "architecture",
            "meta": { "title": "t", "output": "t.html" },
            "layout": { "mode": "grid" },
            "components": [
                { "id": "a", "type": "frontend", "label": "A", "row": 0, "col": 0 },
                { "id": "b", "type": "backend", "label": "B", "row": 1, "col": 2 },
                { "id": "c", "type": "database", "label": "C", "pos": [500, 400], "size": [100, 50] }
            ],
            "boundaries": [{ "kind": "region", "label": "R", "wraps": ["a", "b"] }]
        })
    }

    #[test]
    fn grid_cells_default_sizes_and_pos_wins() {
        let p = place(&doc(base()));
        assert_eq!(p.components[0].rect, Rect::new(40.0, 80.0, 120.0, 60.0));
        // col 2: 40 + 2 * (130 + 30); row 1: 80 + 1 * (64 + 40).
        assert_eq!(p.components[1].rect, Rect::new(360.0, 184.0, 120.0, 60.0));
        assert_eq!(p.components[2].rect, Rect::new(500.0, 400.0, 100.0, 50.0));
    }

    #[test]
    fn grid_overrides_and_problems() {
        let mut v = base();
        v["layout"] = json!({ "mode": "grid", "origin": [10, 20], "cols": 2, "gapX": 0, "gapY": 0, "cellW": 100, "cellH": 50 });
        v["components"][2] = json!({ "id": "c", "type": "database", "label": "C", "row": 0, "col": 0 });
        let d = doc(v);
        assert_eq!(place(&d).components[1].rect, Rect::new(210.0, 70.0, 120.0, 60.0));
        assert_eq!(
            grid_problems(&d),
            [
                "Component \"b\" col 2 exceeds layout.cols 2 (valid: 0..1).",
                "Components \"a\" and \"c\" share grid cell row 0 col 0."
            ]
        );
    }

    #[test]
    fn boundary_frame_pads_and_the_title_rail() {
        let p = place(&doc(base()));
        let b = &p.boundaries[0];
        // members: x 40..480, y 80..244; pad 30, top 30, bottom 30 + 20.
        assert_eq!(b.raw.rect, Rect::new(10.0, 50.0, 500.0, 214.0));
        assert_eq!(b.raw.member_top, 80.0);
        // No profile: the legacy frame, the title rail is still measured (4 + 16 above the members).
        assert_eq!(b.rect, b.raw.rect);
        assert_eq!(b.title.rect, Rect::new(14.0, 60.0, 30.0, 16.0));
        assert_eq!(b.title.font_size, 9.0);
    }

    #[test]
    fn a_profile_extends_the_frame_up_to_the_lifted_title() {
        let mut v = base();
        v["meta"]["quality_profile"] = json!("standard");
        // A component right above the title blocks it: it is lifted to `blocker.y - 2 - h`.
        v["components"][2] = json!({ "id": "c", "type": "database", "label": "C", "pos": [20, 30], "size": [100, 40] });
        let p = place(&doc(v));
        let b = &p.boundaries[0];
        assert_eq!(b.title.rect.y, 30.0 - 2.0 - 16.0);
        assert_eq!(b.rect.y, b.title.rect.y - 4.0);
        assert_eq!(b.rect.bottom(), b.raw.rect.bottom());
    }

    #[test]
    fn auto_view_box_covers_extra_rects_and_the_legend() {
        let d = doc(base());
        let prepared = prepare(&d);
        let plain = prepared.finish(&[]);
        // Right: 600 + 40; bottom: 450 + 40 + 28 + 0 (one legend row).
        assert_eq!(plain.view_box, [640.0, 518.0]);
        let routed = prepared.finish(&[Rect::new(900.0, 700.0, 0.0, 0.0)]);
        assert_eq!(routed.view_box, [940.0, 768.0]);
        // An authored canvas is honoured and never grown.
        let mut v = base();
        v["meta"]["viewBox"] = json!([700, 500]);
        assert_eq!(prepare(&doc(v)).finish(&[Rect::new(900.0, 700.0, 0.0, 0.0)]).view_box, [700.0, 500.0]);
    }

    #[test]
    fn the_legend_wraps_into_the_canvas_height() {
        let mut v = base();
        v["meta"]["legend"] = json!({ "mode": "all" });
        let p = place(&doc(v));
        assert_eq!(p.legend_entries.len(), 7);
        assert_eq!(p.legend_footprint.row_count, p.legend_footprint.rows.len());
        // Height = ceil(450 + 40 + 28 + extra).
        assert_eq!(p.view_box[1], 518.0 + p.legend_footprint.extra_height);
    }

    #[test]
    fn minimum_readable_font_follows_the_canvas_width() {
        assert_eq!(minimum_readable_source_text_px(930.0), 6.0);
        assert_eq!(minimum_readable_source_text_px(600.0), 6.0);
        assert_eq!(minimum_readable_source_text_px(1860.0), 12.0);
        assert!(minimum_readable_source_text_px(0.0).is_nan());
    }

    #[test]
    fn a_wide_canvas_raises_the_title_font_floor_and_widens_the_frame() {
        let mut v = base();
        v["meta"]["quality_profile"] = json!("showcase");
        v["meta"]["viewBox"] = json!([1860, 600]);
        v["boundaries"][0]["label"] = json!("x".repeat(80));
        let p = place(&doc(v));
        let b = &p.boundaries[0];
        assert!(b.title.minimum_font_size >= 12.0);
        assert!(b.title.font_size >= 12.0);
        // The frame widens symmetrically to hold the title at the floor.
        assert!(b.rect.width > b.raw.rect.width);
        assert_eq!(b.rect.cx(), b.raw.rect.cx());
        assert!(p.readability_problem.is_none());
    }

    #[test]
    fn components_in_scene_order_with_context() {
        let d = doc(base());
        let p = place(&d);
        let mut b = begin_scene(&p);
        push_components(&mut b, &d, &p);
        let m = p.measure_legend(&[]).unwrap();
        push_legend(&mut b, m.as_ref(), &Locale::default());
        let scene = b.build();
        let layers: Vec<Layer> = scene.items.iter().map(|i| i.layer).collect();
        assert!(layers.windows(2).all(|w| w[0] <= w[1]));
        let a = scene.group(scene.node("a").unwrap()).unwrap();
        assert_eq!(a.tooltip(), "A \u{b7} R");
        let c = scene.group(scene.node("c").unwrap()).unwrap();
        assert_eq!(c.tooltip(), "C \u{b7} Architecture component");
    }

    #[test]
    fn legend_obstacles_hide_an_implicit_legend_and_fail_an_authored_one() {
        let d = doc(base());
        let p = place(&d);
        let band = p.view_box[1] - 16.0;
        let route: [Pt; 2] = [[0.0, band - 4.0], [600.0, band - 4.0]];
        let obstacles = legend::relationship_obstacles([(route.as_slice(), None)]);
        assert_eq!(p.measure_legend(&obstacles).unwrap(), None);
        let mut v = base();
        v["meta"]["legend"] = json!({ "mode": "auto" });
        let err = place(&doc(v)).measure_legend(&obstacles).unwrap_err();
        assert_eq!(err.code, "legend/content-overlap");
    }
}
