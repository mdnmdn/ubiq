//! Lifecycle layout (P4.2: schema v2, P4.3: schema v1). Port of
//! `renderers/lifecycle/render-lifecycle.mjs`; `00` section 5.5 (Lifecycle v1, v2), section 5.6.5;
//! `03` sections 2.4, 3.4.
//!
//! v2 places one row per populated lane (`main` first, `terminal` last) on a shared 0..4 column
//! grid: column widths, label-driven gaps compressed to the desktop budget, a probe routing pass
//! through [`LifecycleGrid`] to size every row gap for its tracks, the final routing pass, the
//! showcase label sweep (`placeAutomaticLabels` with `gridSweep`), the legend footprint and the
//! auto canvas. [`Plan`] holds all of it, so the gates ([`crate::gates::lifecycle`]) and [`build`]
//! read the same numbers, as Archify's module scope does.
//!
//! v1 has three fixed bands (`main` phase rail y 126, every other lane the event band y 278,
//! `terminal` y 450), the implied emphasis rail behind `main`, the `980 x 660` default canvas and
//! `NN / title` band headers. Its automatic transitions go to the architecture router
//! ([`crate::route::lifecycle_v1`]), as do those of a v2 document whose authored
//! `fromSide`/`toSide` contradicts the grid plan (the whole scene then leaves the grid router;
//! the row gaps are not opened for tracks).
//!
//! Not ported, and said so instead of guessed:
//!
//! - The scene cannot say `writing-mode: vertical-rl` (the scene types are frozen): a row title is
//!   a centred text at `(18, cy)`; its collision rect is the vertical one Archify uses.
//!
//! # JS parity
//!
//! The `states` Map keeps the *first* position of an id with its *last* value (`new Map(entries)`);
//! [`Plan::states`] does the same. Every `Math.max(...)` / `Math.min(...)` that can see `NaN` goes
//! through [`js_max`] / [`js_min`], which propagate it. Sort and Map-order notes are in
//! [`crate::route::lifecycle_grid`].

use std::collections::{HashMap, HashSet};

use crate::diag::Diagnostic;
use crate::geom::{
    Pt, Rect, Seg, Side, anchor, chosen_side, legacy_default_from_side, legacy_default_to_side, normalize,
    rects_overlap, segment_rect_clearance_within,
};
use crate::labels::{self, Hints, Placement, Plate, label_point};
use crate::legend::{self, CatalogEntry, Entry, Layout as LegendLayout, Measured, Placed, Unfit};
use crate::model::common::{LegendMode, NodeIcon, QualityProfile, SchemaVersion, Variant};
use crate::model::lifecycle::{Lifecycle, State, StateType, Transition, TransitionRoute};
use crate::route::dataflow::side as model_side;
use crate::route::lifecycle_grid::{GridState, GridTransition, LifecycleGrid};
use crate::route::lifecycle_v1;
use crate::scene::{
    Anchor, Bounds, Cmd, Detail, EdgeRef, Fill, Group, GroupKind, Layer, PathShape, PolylineShape, RectShape,
    Scene, SceneBuilder, Shape, Stroke, TextShape,
};
use crate::sigils;
use crate::text::{self, DiagramType, LabelBox, Profile, Row};
use crate::tokens::{EdgeVariant, Kind, Token};

/// `layoutV2.stateW` / `stateH`: the default state box.
pub const STATE_W: f64 = 140.0;
pub const STATE_H: f64 = 64.0;
/// `layoutV2.marginX`: left margin, and the right one the canvas keeps.
pub const MARGIN_X: f64 = 60.0;
/// `layoutV2.minGap` / `floorGap`: the default column gap and the compression floor.
pub const MIN_GAP: f64 = 64.0;
pub const FLOOR_GAP: f64 = 44.0;
/// `layoutV2.firstRowTop` / `rowPitch`.
pub const FIRST_ROW_TOP: f64 = 56.0;
pub const ROW_PITCH: f64 = 184.0;
/// `DESKTOP_READER_DIAGRAM_WIDTH` (960 - 30) and `MIN_PROJECTED_NODE_TEXT_PX`.
pub const READER_WIDTH: f64 = 930.0;
pub const MIN_PROJECTED_TEXT_PX: f64 = 6.0;
/// v2 `stateTextFit.step`: the step text size, and the cap of the smallest-text budget.
pub const STEP_FONT: f64 = 8.0;
/// v2 text baselines from the state's top: label, sublabel; the tag sits `12` above the bottom.
pub const LABEL_Y: f64 = 23.0;
pub const SUBLABEL_Y: f64 = 40.0;
pub const TAG_UP: f64 = 12.0;
pub const STATE_RADIUS: f64 = 7.0;
pub const STATE_STROKE: f64 = 1.5;
/// Transition widths: default `1.1`, v2 emphasis `1.6`; corner radius `10`.
pub const TRANSITION_WIDTH: f64 = 1.1;
pub const EMPHASIS_WIDTH: f64 = 1.6;
pub const CORNER_RADIUS: f64 = 10.0;
pub const LABEL_FONT: f64 = 8.0;
pub const NOTE_FONT: f64 = 7.0;
pub const LABEL_RADIUS: f64 = 4.0;
/// Row titles: x `18`, `6.2` per unit, `14` wide collision rect.
pub const BAND_X: f64 = 18.0;
pub const BAND_FONT: f64 = 10.0;
/// The bottom reserve below the states (`+ 96`) and the legend's offsets.
pub const BOTTOM_RESERVE: f64 = 96.0;
pub const LEGEND_X: f64 = 40.0;
pub const LEGEND_BASELINE_UP: f64 = 36.0;
pub const LEGEND_SIDE_PAD: f64 = 40.0;
pub const EDGE_HIT_HALF_WIDTH: f64 = 6.0;

/// v1 `layout`: the three fixed bands (y, state size, column centres).
pub const PHASE_Y: f64 = 126.0;
pub const EVENT_Y: f64 = 278.0;
pub const OUTCOME_Y: f64 = 450.0;
pub const PHASE_SIZE: (f64, f64) = (118.0, 62.0);
pub const EVENT_SIZE: (f64, f64) = (126.0, 58.0);
pub const OUTCOME_SIZE: (f64, f64) = (118.0, 58.0);
pub const PHASE_XS: [f64; 5] = [94.0, 248.0, 402.0, 556.0, 710.0];
pub const EVENT_XS: [f64; 3] = [402.0, 556.0, 710.0];
/// The v1 default canvas, the implied rail's start and its end offset from the last column centre.
pub const V1_VIEW_BOX: [f64; 2] = [980.0, 660.0];
pub const RAIL_START_X: f64 = 154.0;
pub const RAIL_END_PAD: f64 = 38.0;
pub const RAIL_WIDTH: f64 = 2.2;
/// v1 emphasis width, v1 text baselines (label, sublabel; the tag sits `11` above the bottom), the
/// step font, and the bottom the states may not pass (`viewBox[1] - 122`).
pub const V1_EMPHASIS_WIDTH: f64 = 2.0;
pub const V1_LABEL_Y: f64 = 21.0;
pub const V1_SUBLABEL_Y: f64 = 37.0;
pub const V1_TAG_UP: f64 = 11.0;
pub const V1_STEP_FONT: f64 = 7.0;
pub const V1_AREA_RESERVE: f64 = 122.0;
/// The corner radius of a transition the architecture planner routed (`roundedPath(points, 8)`,
/// whatever its `cornerRadius`).
pub const PLANNER_RADIUS: f64 = 8.0;
/// v1 band headers: left edge, header baselines, the dotted rule `12` below the baseline.
pub const V1_BAND_X: f64 = 72.0;
pub const V1_BAND_BASELINES: [f64; 3] = [100.0, 252.0, 424.0];

/// `Math.max(a, ...)` with JS `NaN` propagation (`fold(f64::max)` would skip it).
pub fn js_max(init: f64, values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(init, |m, v| if m.is_nan() || v.is_nan() { f64::NAN } else { m.max(v) })
}

/// `Math.min(a, ...)` with JS `NaN` propagation.
pub fn js_min(init: f64, values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(init, |m, v| if m.is_nan() || v.is_nan() { f64::NAN } else { m.min(v) })
}

/// A JS-truthy optional string.
fn truthy(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|s| !s.is_empty())
}

/// `transition.label || transition.note`.
pub fn label_or_note(t: &Transition) -> Option<&str> {
    t.label.as_deref().filter(|s| !s.is_empty()).or_else(|| t.note.as_deref().filter(|s| !s.is_empty()))
}

/// `transitionLabelWidth`: `max(32, 4.9 · max(units(label), units(note)) + 12)`.
pub fn transition_label_width(t: &Transition) -> f64 {
    let lines = [t.label.as_deref().unwrap_or(""), t.note.as_deref().unwrap_or("")];
    text::edge_label_width(DiagramType::Lifecycle, &lines, Profile::Standard)
}

/// `width || fallback`: an absent or zero size uses the default.
fn or_default(v: Option<f64>, fallback: f64) -> f64 {
    v.filter(|w| *w != 0.0 && !w.is_nan()).unwrap_or(fallback)
}

/// The column as an index, when it is an integer in `0..=4`.
fn col_index(col: f64) -> Option<usize> {
    (col.fract() == 0.0 && (0.0..=4.0).contains(&col)).then_some(col as usize)
}

/// `stateFontSizes(state, width)`: label (v2 11/9, v1 10/8, on the brand fit width), sublabel and tag
/// (v2 8/7, v1 7/6). An absent text fits as one unit, as `fittedNodeFontSize(undefined)` does.
pub fn state_font_sizes(state: &State, width: f64, v2: bool) -> (f64, f64, f64) {
    let fonts = text::node_fonts(DiagramType::Lifecycle, if v2 { 2 } else { 1 });
    let tag = fonts.tag.unwrap_or((8.0, 7.0));
    (
        text::fit(&state.label, text::label_fit_width(width, state.brand.is_some()), fonts.label.0, fonts.label.1),
        text::fit(state.sublabel.as_deref().unwrap_or(""), width, fonts.sublabel.0, fonts.sublabel.1),
        text::fit(state.tag.as_deref().unwrap_or(""), width, tag.0, tag.1),
    )
}

/// `v2ColumnCenters`: widths (max of 140 and the authored widths in the column), gaps (64, widened
/// to `ceil(labelW + 24)` for a labelled same-lane neighbour transition), compressed toward 44 when
/// the canvas exceeds `floor(930 · smallestText / 6)`, then the five centres from `x = 60`.
pub fn column_centers(doc: &Lifecycle) -> [f64; 5] {
    let mut widths = [STATE_W; 5];
    for (col, width) in widths.iter_mut().enumerate() {
        *width = js_max(STATE_W, doc.states.iter().filter(|s| s.col == col as f64).map(|s| s.width.unwrap_or(0.0)));
    }
    let last_col = doc.states.iter().filter_map(|s| col_index(s.col)).max().unwrap_or(0);
    let mut gaps = [MIN_GAP; 4];
    // `byId`: the last state of an id wins.
    let by_id: HashMap<&str, &State> = doc.states.iter().map(|s| (s.id.as_str(), s)).collect();
    for t in &doc.transitions {
        let (Some(from), Some(to)) = (by_id.get(t.from.as_str()), by_id.get(t.to.as_str())) else { continue };
        if label_or_note(t).is_none()
            || from.lane != to.lane
            || from.col.fract() != 0.0
            || to.col.fract() != 0.0
            || (from.col - to.col).abs() != 1.0
        {
            continue;
        }
        let gap = from.col.min(to.col);
        if (0.0..4.0).contains(&gap) {
            let g = gap as usize;
            gaps[g] = gaps[g].max((transition_label_width(t) + 24.0).ceil());
        }
    }
    let smallest = js_min(
        STEP_FONT,
        doc.states.iter().flat_map(|s| {
            let (a, b, c) = state_font_sizes(s, or_default(s.width, STATE_W), true);
            [a, b, c]
        }),
    );
    let budget = (READER_WIDTH * smallest / MIN_PROJECTED_TEXT_PX).floor();
    let fixed = MARGIN_X * 2.0 + widths[..=last_col].iter().sum::<f64>();
    let used = last_col;
    let excess = fixed + gaps[..used].iter().sum::<f64>() - budget;
    let slack: f64 = gaps[..used].iter().map(|g| g - FLOOR_GAP).sum();
    if excess > 0.0 && slack > 0.0 {
        let ratio = (excess / slack).min(1.0);
        for gap in gaps.iter_mut().take(used) {
            *gap = (*gap - (*gap - FLOOR_GAP) * ratio).floor();
        }
    }
    let mut centers = [0.0; 5];
    let mut x = MARGIN_X;
    for col in 0..5 {
        centers[col] = x + widths[col] / 2.0;
        x += widths[col] + gaps.get(col).copied().unwrap_or(0.0);
    }
    centers
}

/// `laneRowOrder`: `main`, the other populated lanes in authored order, `terminal`.
pub fn lane_row_order(doc: &Lifecycle) -> Vec<String> {
    let populated: HashSet<&str> = doc.states.iter().map(|s| s.lane.as_str()).collect();
    let mut ordered = Vec::new();
    if populated.contains("main") {
        ordered.push("main".to_owned());
    }
    for lane in &doc.lanes {
        if lane.id != "main" && lane.id != "terminal" && populated.contains(lane.id.as_str()) {
            ordered.push(lane.id.clone());
        }
    }
    if populated.contains("terminal") {
        ordered.push("terminal".to_owned());
    }
    ordered
}

/// Whether a transition goes to the grid router (`plannerRouted`): no `via` (an empty one counts as
/// authored), no non-`auto` route, no channel.
pub fn planner_routed(t: &Transition) -> bool {
    t.via.is_none()
        && matches!(t.route, None | Some(TransitionRoute::Auto))
        && t.channel_x.is_none()
        && t.channel_y.is_none()
}

/// `effectiveVariant`: the authored variant, else (v2 only; v1 has the implied rail) emphasis for a
/// forward `main` to `main` step.
fn effective_variant(t: &Transition, from: Option<&PlacedState>, to: Option<&PlacedState>, v2: bool) -> EdgeVariant {
    match t.variant {
        Some(Variant::Default) => EdgeVariant::Default,
        Some(Variant::Emphasis) => EdgeVariant::Emphasis,
        Some(Variant::Security) => EdgeVariant::Security,
        Some(Variant::Dashed) => EdgeVariant::Dashed,
        None => match (from, to) {
            (Some(f), Some(t)) if v2 && f.lane == "main" && t.lane == "main" && t.col > f.col => EdgeVariant::Emphasis,
            _ => EdgeVariant::Default,
        },
    }
}

/// The v1 band a lane sits in (`bandFor`): `main` the phase rail, `terminal` the outcomes, every
/// other lane the shared event band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandKind {
    Phase,
    Event,
    Outcome,
}

impl BandKind {
    pub fn of(lane: &str) -> Self {
        match lane {
            "main" => Self::Phase,
            "terminal" => Self::Outcome,
            _ => Self::Event,
        }
    }

    /// `band` of `bandGeometry` / the schema message ("the phase band has integer columns 0..4").
    pub fn name(self) -> &'static str {
        match self {
            Self::Phase => "phase",
            Self::Event => "event",
            Self::Outcome => "outcome",
        }
    }

    /// The column centres of the band.
    pub fn xs(self) -> &'static [f64] {
        match self {
            Self::Phase => &PHASE_XS,
            Self::Event | Self::Outcome => &EVENT_XS,
        }
    }

    fn y_and_size(self) -> (f64, (f64, f64)) {
        match self {
            Self::Phase => (PHASE_Y, PHASE_SIZE),
            Self::Event => (EVENT_Y, EVENT_SIZE),
            Self::Outcome => (OUTCOME_Y, OUTCOME_SIZE),
        }
    }
}

fn state_type_name(t: StateType) -> &'static str {
    match t {
        StateType::Start => "start",
        StateType::Active => "active",
        StateType::Waiting => "waiting",
        StateType::Decision => "decision",
        StateType::Success => "success",
        StateType::Failure => "failure",
        StateType::Neutral => "neutral",
        StateType::External => "external",
    }
}

/// A measured state, in `states` Map order.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedState {
    pub id: String,
    /// The authored index of the value the Map kept (the last state of the id).
    pub index: usize,
    pub lane: String,
    pub col: f64,
    pub rect: Rect,
    /// `cx` as measured (the column centre, `NaN` for an invalid column).
    pub cx: f64,
    /// Its row among the rendered rows.
    pub row: Option<usize>,
    pub kind: StateType,
}

/// A transition's route as `pathFor` returns it.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutedTransition {
    pub points: Vec<Pt>,
    pub from_side: Side,
    pub to_side: Side,
    /// Routed by a planner, the grid router or the architecture router (halo, crossings resolved).
    pub planned: bool,
    /// The corner radius the path is drawn with.
    pub radius: f64,
}

/// An edge label: the text anchor and the mask rect (`transitionLabelBox`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelGeom {
    pub at: Pt,
    pub rect: Rect,
}

/// A band title (`bandGeometry`): its text, anchor and collision rect. v2: a row title, vertical,
/// at `(18, cy)`. v1: `NN / title` at `(72, baseline)` with a dotted rule below.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub label: String,
    pub at: Pt,
    pub rect: Rect,
    /// `index`: the row (v2) or the band's slot among the three (v1).
    pub index: usize,
    /// v1 only: the band it heads.
    pub band: Option<BandKind>,
}

/// Everything Archify's module scope computes before `validateLifecycle`.
#[derive(Clone, Debug)]
pub struct Plan<'a> {
    pub doc: &'a Lifecycle,
    /// `schema_version === 2`.
    pub v2: bool,
    /// The grid router took the scene (v2 with no contradicting pin). False: v1, or a v2 document
    /// whose authored sides the grid plan contradicts, which the architecture router lays out.
    pub grid: bool,
    /// The v2 column centres (unused by v1).
    pub centers: [f64; 5],
    /// The rendered rows (lane ids) and their final tops (v2 only).
    pub rows: Vec<String>,
    pub row_top: HashMap<String, f64>,
    /// The `states` Map, in Map order.
    pub states: Vec<PlacedState>,
    pub by_id: HashMap<String, usize>,
    /// Per authored transition: its route, when both endpoints exist.
    pub routes: Vec<Option<RoutedTransition>>,
    /// Per authored transition: its label, when it has text and both endpoints exist.
    pub labels: Vec<Option<LabelGeom>>,
    /// Per state (Map order): no authored outgoing transition.
    pub finals: Vec<bool>,
    /// The legend entries, `final` included.
    pub entries: Vec<Entry>,
    pub view_box: [f64; 2],
    /// `legendFootprint(...).extraHeight` at the final canvas width.
    pub legend_extra: f64,
    /// `lifecycleAreaBottom()`.
    pub area_bottom: f64,
    pub bands: Vec<Band>,
}

/// The `states` Map: first position of an id, last value. `place(state)` is `measureState`: the
/// default size, the centre and the top before `yOffset`.
fn measure_with(doc: &Lifecycle, rows: &[String], place: impl Fn(&State) -> ((f64, f64), f64, f64)) -> Vec<PlacedState> {
    let mut out: Vec<PlacedState> = Vec::new();
    for (index, s) in doc.states.iter().enumerate() {
        let ((default_w, default_h), cx, top) = place(s);
        let width = or_default(s.width, default_w);
        let height = or_default(s.height, default_h);
        let y = top + s.y_offset.filter(|v| !v.is_nan()).unwrap_or(0.0);
        let placed = PlacedState {
            id: s.id.clone(),
            index,
            lane: s.lane.clone(),
            col: s.col,
            rect: Rect::new(cx - width / 2.0, y, width, height),
            cx,
            row: rows.iter().position(|r| *r == s.lane),
            kind: s.kind,
        };
        match out.iter_mut().find(|p| p.id == s.id) {
            Some(slot) => *slot = placed,
            None => out.push(placed),
        }
    }
    out
}

/// v2 `measureState`: the 140 x 64 box on the column centre and the lane's row top.
fn measure_states(doc: &Lifecycle, centers: &[f64; 5], rows: &[String], row_top: &HashMap<String, f64>) -> Vec<PlacedState> {
    measure_with(doc, rows, |s| {
        let cx = col_index(s.col).map_or(f64::NAN, |c| centers[c]);
        ((STATE_W, STATE_H), cx, row_top.get(&s.lane).copied().unwrap_or(f64::NAN))
    })
}

/// v1 `measureState`: the band's box and top; a column outside the band (or not an integer) reads
/// `xs[col]` as `undefined` and falls back to the last column.
fn measure_states_v1(doc: &Lifecycle) -> Vec<PlacedState> {
    measure_with(doc, &[], |s| {
        let band = BandKind::of(&s.lane);
        let (y, size) = band.y_and_size();
        let xs = band.xs();
        let cx = col_index(s.col).and_then(|c| xs.get(c).copied()).unwrap_or(xs[xs.len() - 1]);
        (size, cx, y)
    })
}

fn grid_input(states: &[PlacedState], by_id: &HashMap<String, usize>, planned: &[&Transition]) -> (Vec<GridState>, Vec<GridTransition>) {
    let grid_states = states
        .iter()
        .map(|s| GridState { id: s.id.clone(), rect: s.rect, cx: s.cx, row: s.row, start: s.kind == StateType::Start })
        .collect();
    let transitions = planned
        .iter()
        .map(|t| GridTransition {
            from: by_id.get(&t.from).copied(),
            to: by_id.get(&t.to).copied(),
            labelled: label_or_note(t).is_some(),
        })
        .collect();
    (grid_states, transitions)
}

/// `legendCatalog()`: the lifecycle `LEGEND_CATALOG` (`legend.lifecycle.*` in the document's
/// locale) resolved against the present state types, plus the structural `final` row when any
/// state is final.
fn legend_entries(doc: &Lifecycle, states: &[PlacedState], any_final: bool) -> Vec<Entry> {
    const CATALOG: [&str; 8] = ["start", "active", "waiting", "decision", "success", "failure", "neutral", "external"];
    let locale = doc.meta.locale();
    let catalog: Vec<CatalogEntry> = CATALOG
        .iter()
        .map(|kind| CatalogEntry {
            kind,
            label: locale.t(&format!("legend.lifecycle.{kind}")),
            swatch_width: (*kind == "start").then_some(26.0),
            swatch_gap: None,
            interactive: true,
        })
        .collect();
    let present: HashSet<&str> = states.iter().map(|s| state_type_name(s.kind)).collect();
    let config = doc.meta.legend.as_ref().map(|l| {
        let mut entries = HashMap::new();
        if let Some(e) = &l.entries {
            for (kind, entry) in [
                ("start", &e.start),
                ("active", &e.active),
                ("waiting", &e.waiting),
                ("decision", &e.decision),
                ("success", &e.success),
                ("failure", &e.failure),
                ("neutral", &e.neutral),
                ("external", &e.external),
            ] {
                if let Some(o) = entry {
                    entries.insert(kind.to_owned(), legend::Override { label: o.label.clone(), visible: o.visible });
                }
            }
        }
        legend::Config { mode: l.mode.unwrap_or(LegendMode::Auto), entries }
    });
    let mut entries = legend::resolve(config.as_ref(), &catalog, &present);
    if !entries.is_empty() && any_final {
        entries.push(Entry {
            kind: "final",
            label: locale.t("legend.lifecycle.final"),
            swatch_width: Some(16.0),
            swatch_gap: None,
            present: true,
            interactive: false,
        });
    }
    entries
}

/// v1 `isFinal`: no authored outgoing transition, and, on the `main` rail (which the implied rail
/// continues), only at the furthest occupied column.
fn v1_final(state: &PlacedState, states: &[PlacedState]) -> bool {
    BandKind::of(&state.lane) != BandKind::Phase
        || state.col == js_max(f64::NEG_INFINITY, states.iter().filter(|s| BandKind::of(&s.lane) == BandKind::Phase).map(|s| s.col))
}

/// v1 `bandGeometry`: the three `NN / title` headers (the `main` lane's label, every other lane's
/// joined by ` + `, the `terminal` lane's), at a fixed baseline each, for the bands a state sits in.
fn v1_bands(doc: &Lifecycle, states: &[PlacedState]) -> Vec<Band> {
    let lane = |id: &str| doc.lanes.iter().find(|l| l.id == id).map(|l| l.label.as_str()).filter(|l| !l.is_empty());
    let event: Vec<&str> =
        doc.lanes.iter().filter(|l| l.id != "main" && l.id != "terminal").map(|l| l.label.as_str()).collect();
    let titles = [
        lane("main").unwrap_or("Lifecycle phases").to_owned(),
        if event.is_empty() { "Interruptions + recovery".to_owned() } else { event.join(" + ") },
        lane("terminal").unwrap_or("Outcomes").to_owned(),
    ];
    [BandKind::Phase, BandKind::Event, BandKind::Outcome]
        .into_iter()
        .enumerate()
        .filter(|(_, band)| states.iter().any(|s| BandKind::of(&s.lane) == *band))
        .map(|(index, band)| {
            let baseline = V1_BAND_BASELINES[index];
            let label = format!("{:02} / {}", index + 1, titles[index]);
            let width = text::units(&label) as f64 * 6.2;
            Band {
                at: [V1_BAND_X, baseline],
                rect: Rect::new(V1_BAND_X, baseline - 11.0, width, 14.0),
                label,
                index,
                band: Some(band),
            }
        })
        .collect()
}

impl<'a> Plan<'a> {
    /// The module scope of `render-lifecycle.mjs` up to `validateLifecycle`. Never fails: the
    /// `Result` stays so a caller can still skip a document it cannot plan.
    pub fn new(doc: &'a Lifecycle) -> Result<Plan<'a>, Box<Diagnostic>> {
        let v2 = doc.schema_version == SchemaVersion::V2;
        let planned: Vec<&Transition> = doc.transitions.iter().filter(|t| planner_routed(t)).collect();
        let (centers, rows) = if v2 { (column_centers(doc), lane_row_order(doc)) } else { ([0.0; 5], Vec::new()) };
        let mut row_top: HashMap<String, f64> =
            rows.iter().enumerate().map(|(i, lane)| (lane.clone(), FIRST_ROW_TOP + i as f64 * ROW_PITCH)).collect();
        let mut states = if v2 { measure_states(doc, &centers, &rows, &row_top) } else { measure_states_v1(doc) };
        let by_id: HashMap<String, usize> = states.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();

        let mut grid = None;
        if v2 {
            // Route once to learn how many tracks each row gap carries, then open every gap to fit them.
            let (grid_states, grid_transitions) = grid_input(&states, &by_id, &planned);
            let probe = LifecycleGrid::new(&grid_states, &grid_transitions, &centers);
            // A conflicting pin sends the whole scene to the architecture planner, which then opens
            // no gap for tracks.
            let use_grid = planned.iter().enumerate().all(|(i, t)| {
                let (from, to) = probe.connection_sides(i);
                t.from_side.is_none_or(|s| model_side(s) == from) && t.to_side.is_none_or(|s| model_side(s) == to)
            });
            let mut top = FIRST_ROW_TOP;
            for (index, lane) in rows.iter().enumerate() {
                let old = row_top[lane];
                let row_height = js_max(
                    STATE_H,
                    states.iter().filter(|s| &s.lane == lane).map(|s| s.rect.y + s.rect.height - old),
                );
                row_top.insert(lane.clone(), top);
                top += row_height + (ROW_PITCH - STATE_H).max(if use_grid { probe.gap_height(index) } else { 0.0 });
            }
            states = measure_states(doc, &centers, &rows, &row_top);
            if use_grid {
                let (grid_states, grid_transitions) = grid_input(&states, &by_id, &planned);
                grid = Some(LifecycleGrid::new(&grid_states, &grid_transitions, &centers));
            }
        }

        let outgoing: HashSet<&str> = doc.transitions.iter().map(|t| t.from.as_str()).collect();
        let finals: Vec<bool> =
            states.iter().map(|s| !outgoing.contains(s.id.as_str()) && (v2 || v1_final(s, &states))).collect();
        let entries = legend_entries(doc, &states, finals.iter().any(|f| *f));

        let view_box = match (doc.meta.view_box, v2) {
            (Some(vb), _) => vb,
            (None, false) => V1_VIEW_BOX,
            (None, true) => {
                let finite = |v: &f64| v.is_finite();
                let max_right = js_max(0.0, states.iter().map(|s| s.cx + s.rect.width / 2.0).filter(finite));
                let width = (max_right + MARGIN_X).ceil().max(640.0);
                let extra = legend::footprint(&entries, width - 2.0 * LEGEND_SIDE_PAD).extra_height;
                let bottom = js_max(0.0, states.iter().map(|s| s.rect.y + s.rect.height).filter(finite));
                [width, (bottom + extra + BOTTOM_RESERVE).ceil()]
            }
        };
        let legend_extra = legend::footprint(&entries, view_box[0] - 2.0 * LEGEND_SIDE_PAD).extra_height;
        let area_bottom = if v2 { view_box[1] - legend_extra - BOTTOM_RESERVE } else { view_box[1] - V1_AREA_RESERVE };

        let lane_labels: HashMap<&str, &str> = doc.lanes.iter().map(|l| (l.id.as_str(), l.label.as_str())).collect();
        let bands = if v2 {
            rows.iter()
                .enumerate()
                .map(|(index, lane)| {
                    let cy = row_top[lane] + STATE_H / 2.0;
                    let label = lane_labels.get(lane.as_str()).copied().filter(|l| !l.is_empty()).unwrap_or(lane).to_owned();
                    let length = text::units(&label) as f64 * 6.2;
                    Band { at: [BAND_X, cy], rect: Rect::new(11.0, cy - length / 2.0, 14.0, length), label, index, band: None }
                })
                .collect()
        } else {
            v1_bands(doc, &states)
        };

        let mut plan = Plan {
            doc,
            v2,
            grid: grid.is_some(),
            centers,
            rows,
            row_top,
            states,
            by_id,
            routes: Vec::new(),
            labels: Vec::new(),
            finals,
            entries,
            view_box,
            legend_extra,
            area_bottom,
            bands,
        };
        plan.routes = plan.route_all(grid.as_ref());
        plan.labels = plan.place_labels();
        Ok(plan)
    }

    pub fn state(&self, id: &str) -> Option<&PlacedState> {
        self.by_id.get(id).map(|&i| &self.states[i])
    }

    /// Whether the authored profile is showcase (the layout never sees `--quality`).
    pub fn showcase(&self) -> bool {
        self.doc.meta.quality_profile == Some(QualityProfile::Showcase)
    }

    /// `pathFor` and `transitionSides` of every transition whose endpoints exist. An automatic
    /// transition goes to the grid router when there is one (`grid`), else to the architecture
    /// router (v1, and a v2 document whose pins contradict the grid plan).
    fn route_all(&self, grid: Option<&LifecycleGrid>) -> Vec<Option<RoutedTransition>> {
        let planner = if grid.is_none() { lifecycle_v1::plan(self) } else { Vec::new() };
        let mut planned_index = 0;
        self.doc
            .transitions
            .iter()
            .map(|t| {
                let planned = planner_routed(t);
                let pi = planned_index;
                if planned {
                    planned_index += 1;
                }
                let (from, to) = (self.state(&t.from)?, self.state(&t.to)?);
                if planned {
                    return Some(match grid {
                        Some(grid) => {
                            let (from_side, to_side) = grid.connection_sides(pi);
                            let radius = t.corner_radius.unwrap_or(CORNER_RADIUS);
                            RoutedTransition { points: grid.path_for(pi), from_side, to_side, planned, radius }
                        }
                        None => {
                            let routed = planner[pi].as_ref()?;
                            RoutedTransition {
                                points: routed.points.clone(),
                                from_side: routed.sides.from,
                                to_side: routed.sides.to,
                                planned,
                                radius: PLANNER_RADIUS,
                            }
                        }
                    });
                }
                let from_side = chosen_side(t.from_side.map(model_side), legacy_from(from, to));
                let to_side = chosen_side(t.to_side.map(model_side), legacy_to(from, to));
                let start = anchor_cx(from, from_side);
                let end = anchor_cx(to, to_side);
                let via = route_via(t, from, to, start, end, from_side, to_side);
                let points = std::iter::once(start).chain(via).chain(std::iter::once(end)).collect();
                Some(RoutedTransition { points, from_side, to_side, planned, radius: t.corner_radius.unwrap_or(CORNER_RADIUS) })
            })
            .collect()
    }

    /// `transitionLabelBox` of every labelled transition: the default `labelPoint`, or for an
    /// authored showcase document the anchor `placeAutomaticLabels` chose (v2: with the grid sweep;
    /// v1: the plain placement of the architecture renderer).
    fn place_labels(&self) -> Vec<Option<LabelGeom>> {
        let mut labels: Vec<Option<LabelGeom>> = self
            .doc
            .transitions
            .iter()
            .zip(&self.routes)
            .map(|(t, route)| {
                let route = route.as_ref()?;
                label_or_note(t)?;
                let hints = Hints { at: t.label_at, dx: t.label_dx, dy: t.label_dy, segment: t.label_segment };
                Some(label_box_at(t, label_point(&hints, &route.points)))
            })
            .collect();
        if self.showcase() {
            let moved = if self.v2 { place_labels_grid(self, &labels) } else { self.place_labels_plain(&labels) };
            // `resolvedLabelPoints`: the box is re-measured from the anchor, 11 above it (one pixel
            // higher than the placement's `rectAt`).
            for (slot, at) in labels.iter_mut().zip(moved) {
                if let (Some(label), Some(at)) = (slot.as_mut(), at) {
                    let r = label.rect;
                    *label = LabelGeom { at, rect: Rect::new(at[0] - r.width / 2.0, at[1] - 11.0, r.width, r.height) };
                }
            }
        }
        labels
    }

    /// v1's `placeAutomaticLabels({ labels, routes, components, titles, viewBox, placementBottom })`:
    /// no grid sweep, the fallback ring, not held near the route. The final anchor of every
    /// labelled transition.
    fn place_labels_plain(&self, labels: &[Option<LabelGeom>]) -> Vec<Option<Pt>> {
        let plates: Vec<(usize, Plate)> = labels
            .iter()
            .enumerate()
            .filter_map(|(rel, l)| {
                let l = l.as_ref()?;
                let t = &self.doc.transitions[rel];
                let pinned = t.label_at.is_some() || t.label_dx.is_some() || t.label_dy.is_some() || t.label_segment.is_some();
                Some((rel, Plate { rel: rel as i64, pinned, rect: l.rect, at: l.at }))
            })
            .collect();
        let routes: Vec<(i64, &[Pt])> =
            self.routes.iter().enumerate().filter_map(|(rel, r)| r.as_ref().map(|r| (rel as i64, r.points.as_slice()))).collect();
        let components: Vec<Rect> = self.states.iter().map(|s| s.rect).collect();
        let titles: Vec<Rect> = self.bands.iter().map(|b| b.rect).collect();
        let flat: Vec<Plate> = plates.iter().map(|(_, p)| *p).collect();
        let placed = labels::place_automatic_labels(
            &flat,
            &Placement {
                routes: &routes,
                components: &components,
                titles: &titles,
                view_box: self.view_box,
                placement_bottom: self.area_bottom,
                fallback_ring: true,
                keep_fallback_near_route: false,
            },
        );
        let mut out = vec![None; labels.len()];
        for ((rel, _), plate) in plates.iter().zip(placed) {
            out[*rel] = Some(plate.at);
        }
        out
    }
}

/// One label of `placeAutomaticLabels`.
struct Slot {
    rel: usize,
    pinned: bool,
    rect: Rect,
    at: Pt,
}

/// `placeAutomaticLabels` of `renderers/architecture/labels.mjs` as lifecycle v2 calls it
/// (`keepFallbackNearRoute` and `gridSweep` on, `fallbackRing` on): the final anchor of every
/// labelled transition. [`labels::place_automatic_labels`] does not port `gridSweep`, so v2 keeps
/// this copy; v1 uses the shared one ([`Plan::place_labels_plain`]).
///
/// The spatial grid of the source only prunes the segment scan; the full scan below answers the
/// same `masksRoute` (a segment within 4 of a rect has a bbox in the cells the query covers).
fn place_labels_grid(plan: &Plan<'_>, labels: &[Option<LabelGeom>]) -> Vec<Option<Pt>> {
    let doc = plan.doc;
    let mut placed: Vec<Slot> = labels
        .iter()
        .enumerate()
        .filter_map(|(rel, l)| {
            let l = l.as_ref()?;
            let t = &doc.transitions[rel];
            let pinned = t.label_at.is_some() || t.label_dx.is_some() || t.label_dy.is_some() || t.label_segment.is_some();
            Some(Slot { rel, pinned, rect: l.rect, at: l.at })
        })
        .collect();
    let segments: Vec<(usize, Seg)> = plan
        .routes
        .iter()
        .enumerate()
        .filter_map(|(rel, r)| r.as_ref().map(|r| (rel, normalize(&r.points))))
        .flat_map(|(rel, pts)| pts.windows(2).map(|w| (rel, Seg::new(w[0], w[1]))).collect::<Vec<_>>())
        .collect();
    let obstacles: Vec<Rect> = plan.states.iter().map(|s| s.rect).chain(plan.bands.iter().map(|b| b.rect)).collect();
    let (vb, bottom) = (plan.view_box, plan.area_bottom);
    let inside = |r: &Rect| r.x >= 0.0 && r.y >= 0.0 && r.x + r.width <= vb[0] && r.y + r.height <= vb[1];
    // `segmentRectClearanceWithin` may be `null` (non-finite input); JS reads it as 0.
    let within = |s: &Seg, r: &Rect, limit: f64| segment_rect_clearance_within(s, r, limit).unwrap_or(0.0);
    let masks_route = |r: &Rect, rel: usize| segments.iter().any(|(ri, s)| *ri != rel && within(s, r, 4.0) + 0.0001 < 4.0);
    let rect_at = |w: f64, h: f64, lx: f64, ly: f64| Rect::new(lx - w / 2.0, ly - 10.0, w, h);

    for index in 0..placed.len() {
        if placed[index].pinned {
            continue;
        }
        let (rel, label_rect, label_at) = (placed[index].rel, placed[index].rect, placed[index].at);
        let (w, h) = (label_rect.width, label_rect.height);
        let clear = |placed: &[Slot], r: &Rect| {
            inside(r)
                && r.y + r.height <= bottom
                && !obstacles.iter().any(|o| rects_overlap(r, o, 2.0))
                && !placed.iter().enumerate().any(|(oi, o)| oi != index && rects_overlap(r, &o.rect, 2.0))
                && !masks_route(r, rel)
        };
        let own: Vec<Seg> = segments.iter().filter(|(ri, _)| *ri == rel).map(|(_, s)| *s).collect();

        // Grid sweep: rank every position along the label's own segments, tested with a pixel of
        // slack all round.
        let candidates = grid_candidates(rel, w, h, &own, &segments);
        let found = candidates.into_iter().map(|[lx, ly]| (rect_at(w, h, lx, ly), [lx, ly])).find(|(r, _)| {
            clear(&placed, &Rect::new(r.x - 1.0, r.y - 1.0, r.width + 2.0, r.height + 2.0))
        });
        if let Some((rect, at)) = found {
            placed[index].rect = rect;
            placed[index].at = at;
            continue;
        }

        let mut replaced = false;
        for s in &own {
            let (a, b) = (s.start, s.end);
            let fractions = [0.5, 0.25, 0.75, 0.125, 0.875];
            let mut cands: Vec<Pt> = Vec::new();
            if (a[1] - b[1]).abs() < 0.0001 && (a[0] - b[0]).abs() >= w + 16.0 {
                for f in fractions {
                    let x = a[0] + (b[0] - a[0]) * f;
                    if (x - a[0]).abs().min((x - b[0]).abs()) < 8.0 {
                        continue;
                    }
                    cands.extend([[x, a[1] - 10.0], [x, a[1] + 20.0], [x, a[1] - 18.0], [x, a[1] + 28.0]]);
                }
            } else if (a[0] - b[0]).abs() < 0.0001 && (a[1] - b[1]).abs() >= h + 16.0 {
                for f in fractions {
                    let y = a[1] + (b[1] - a[1]) * f;
                    if (y - a[1]).abs().min((y - b[1]).abs()) < 8.0 {
                        continue;
                    }
                    cands.extend([
                        [a[0] - w / 2.0 - 6.0, y + 3.0],
                        [a[0] + w / 2.0 + 6.0, y + 3.0],
                        [a[0] - w / 2.0 - 14.0, y + 3.0],
                        [a[0] + w / 2.0 + 14.0, y + 3.0],
                    ]);
                }
            }
            if let Some(at) = cands.into_iter().find(|&[lx, ly]| clear(&placed, &rect_at(w, h, lx, ly))) {
                placed[index].rect = rect_at(w, h, at[0], at[1]);
                placed[index].at = at;
                replaced = true;
                break;
            }
        }
        if replaced {
            continue;
        }

        // The deterministic ring around the anchor and the own segment centres, kept within two
        // label heights of the own route.
        let mut anchors = vec![label_at];
        anchors.extend(own.iter().map(|s| [(s.start[0] + s.end[0]) / 2.0, (s.start[1] + s.end[1]) / 2.0]));
        let step = w / 2.0 + 12.0;
        let ring = [
            [0.0, -28.0],
            [0.0, 38.0],
            [-step, -28.0],
            [step, -28.0],
            [-step, 38.0],
            [step, 38.0],
            [-(w + 20.0), -52.0],
            [w + 20.0, -52.0],
            [-(w + 20.0), 62.0],
            [w + 20.0, 62.0],
            [-(w + 20.0), -76.0],
            [w + 20.0, -76.0],
            [-(w + 20.0), 86.0],
            [w + 20.0, 86.0],
        ];
        let fallback = anchors
            .iter()
            .flat_map(|[bx, by]| ring.iter().map(move |[dx, dy]| [bx + dx, by + dy]))
            .find(|&[lx, ly]| {
                let r = rect_at(w, h, lx, ly);
                clear(&placed, &r) && own.iter().any(|s| within(s, &r, h * 2.0) <= h * 2.0)
            });
        if let Some(at) = fallback {
            placed[index].rect = rect_at(w, h, at[0], at[1]);
            placed[index].at = at;
        }
    }
    let mut out = vec![None; labels.len()];
    for slot in placed {
        out[slot.rel] = Some(slot.at);
    }
    out
}

/// `gridCandidates(label)`: beside the line first, then centred on it, then stepping outward past
/// neighbouring parallels. Ranked by tier with the push order breaking ties (a stable sort).
fn grid_candidates(rel: usize, w: f64, h: f64, own: &[Seg], segments: &[(usize, Seg)]) -> Vec<Pt> {
    const FRACTIONS: [f64; 7] = [0.5, 0.25, 0.75, 0.375, 0.625, 0.125, 0.875];
    // A close parallel of another relationship on the low side makes a label there read as the
    // neighbour's.
    let parallel_on_low_side = |a: Pt, b: Pt, axis: usize| {
        let across = 1 - axis;
        let (low, high) = (a[axis].min(b[axis]), a[axis].max(b[axis]));
        let mut nearest: Option<f64> = None;
        for (ri, other) in segments {
            if *ri == rel || (other.start[across] - other.end[across]).abs() > 0.0001 {
                continue;
            }
            let distance = other.start[across] - a[across];
            if distance.abs() < 0.0001 || distance.abs() > 36.0 {
                continue;
            }
            let overlap = high.min(other.start[axis].max(other.end[axis])) - low.max(other.start[axis].min(other.end[axis]));
            if overlap <= 0.0 {
                continue;
            }
            if nearest.is_none_or(|n| distance.abs() < n.abs()) {
                nearest = Some(distance);
            }
        }
        nearest.is_some_and(|n| n < 0.0)
    };
    let mut ranked: Vec<(f64, Pt)> = Vec::new();
    for s in own {
        let (a, b) = (s.start, s.end);
        if (a[1] - b[1]).abs() < 0.0001 && (a[0] - b[0]).abs() >= w + 16.0 {
            let (above, below) = if parallel_on_low_side(a, b, 0) { (1.0, 0.0) } else { (0.0, 1.0) };
            for f in FRACTIONS {
                let x = a[0] + (b[0] - a[0]) * f;
                if (x - a[0]).abs().min((x - b[0]).abs()) < w / 2.0 + 6.0 {
                    continue;
                }
                ranked.extend([
                    (above, [x, a[1] - 10.0]),
                    (below, [x, a[1] + 20.0]),
                    (2.0, [x, a[1] + 3.0]),
                    (3.0, [x, a[1] - 18.0]),
                    (3.0, [x, a[1] + 28.0]),
                ]);
            }
        } else if (a[0] - b[0]).abs() < 0.0001 && (a[1] - b[1]).abs() >= h + 16.0 {
            let left_first = !parallel_on_low_side(a, b, 1);
            for f in FRACTIONS {
                let y = a[1] + (b[1] - a[1]) * f;
                if (y - a[1]).abs().min((y - b[1]).abs()) < h / 2.0 + 6.0 {
                    continue;
                }
                ranked.push((2.0, [a[0], y + 3.0]));
                for (step, offset) in [6.0, 14.0, 22.0, 30.0, 38.0, 46.0, 54.0].into_iter().enumerate() {
                    let tier = match step {
                        0 => 0.0,
                        1 => 1.0,
                        s => 2.0 + s as f64,
                    };
                    let (lt, rt) = if left_first { (tier, tier + 0.5) } else { (tier + 0.5, tier) };
                    ranked.push((lt, [a[0] - w / 2.0 - offset, y + 3.0]));
                    ranked.push((rt, [a[0] + w / 2.0 + offset, y + 3.0]));
                }
            }
        }
    }
    ranked.sort_by(|l, r| l.0.total_cmp(&r.0));
    ranked.into_iter().map(|(_, p)| p).collect()
}

/// `transitionLabelBoxAt(transition, [lx, ly])`: width from the longer line, height 27 with both a
/// label and a note else 16, `11` above the baseline.
pub fn label_box_at(t: &Transition, at: Pt) -> LabelGeom {
    let width = transition_label_width(t);
    let height = if truthy(&t.label) && truthy(&t.note) { 27.0 } else { 16.0 };
    LabelGeom { at, rect: Rect::new(at[0] - width / 2.0, at[1] - 11.0, width, height) }
}

/// `anchor(rect, side)` on the measured `cx` (as JS reads `rect.cx`).
fn anchor_cx(s: &PlacedState, side: Side) -> Pt {
    match side {
        Side::Top => [s.cx, s.rect.y],
        Side::Bottom => [s.cx, s.rect.bottom()],
        other => anchor(&s.rect, other),
    }
}

/// `legacyDefaultFromSide` on the measured centres.
fn legacy_from(from: &PlacedState, to: &PlacedState) -> Side {
    let (f, t) = (Rect::from_center(from.cx, from.rect.cy(), 0.0, 0.0), Rect::from_center(to.cx, to.rect.cy(), 0.0, 0.0));
    legacy_default_from_side(&f, &t)
}

/// `legacyDefaultToSide` on the measured centres.
fn legacy_to(from: &PlacedState, to: &PlacedState) -> Side {
    let (f, t) = (Rect::from_center(from.cx, from.rect.cy(), 0.0, 0.0), Rect::from_center(to.cx, to.rect.cy(), 0.0, 0.0));
    legacy_default_to_side(&f, &t)
}

/// `routeVia` (the lifecycle presets) for a transition the grid router does not take.
fn route_via(t: &Transition, from: &PlacedState, to: &PlacedState, start: Pt, end: Pt, fs: Side, ts: Side) -> Vec<Pt> {
    if let Some(via) = &t.via {
        return via.clone();
    }
    let (f, o) = (&from.rect, &to.rect);
    match t.route.unwrap_or(TransitionRoute::Auto) {
        TransitionRoute::Straight => Vec::new(),
        TransitionRoute::Drop => {
            let y = t.channel_y.unwrap_or((start[1] + end[1]) / 2.0);
            vec![[start[0], y], [end[0], y]]
        }
        TransitionRoute::BottomChannel => {
            let y = t.channel_y.unwrap_or(f.bottom().max(o.bottom()) + 34.0);
            vec![[start[0], y], [end[0], y]]
        }
        TransitionRoute::TopChannel => {
            let y = t.channel_y.unwrap_or(f.y.min(o.y) - 28.0);
            vec![[start[0], y], [end[0], y]]
        }
        TransitionRoute::RightChannel => {
            let x = t.channel_x.unwrap_or(f.right().max(o.right()) + 36.0);
            vec![[x, start[1]], [x, end[1]]]
        }
        TransitionRoute::LeftChannel => {
            let x = t.channel_x.unwrap_or(f.x.min(o.x) - 36.0);
            vec![[x, start[1]], [x, end[1]]]
        }
        TransitionRoute::Auto => {
            if start[0] == end[0] || start[1] == end[1] {
                return Vec::new();
            }
            let vertical = |s: Side| matches!(s, Side::Top | Side::Bottom);
            let (fv, tv) = (vertical(fs), vertical(ts));
            if fv != tv {
                return vec![if fv { [start[0], end[1]] } else { [end[0], start[1]] }];
            }
            if fv {
                let y = t.channel_y.unwrap_or((start[1] + end[1]) / 2.0);
                return vec![[start[0], y], [end[0], y]];
            }
            let x = t.channel_x.unwrap_or((start[0] + end[0]) / 2.0);
            vec![[x, start[1]], [x, end[1]]]
        }
    }
}

/// A placed state as the goldens read it.
#[derive(Clone, Debug, PartialEq)]
pub struct StateGeom {
    pub id: String,
    pub rect: Rect,
    /// The UML final double border (no authored outgoing transition).
    pub final_border: bool,
    /// The initial pseudo-state marker (a `start` state).
    pub initial_marker: bool,
}

/// A routed transition with its label.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionGeom {
    /// The transition's index in `transitions` (`data-edge-key`).
    pub key: usize,
    pub from: String,
    pub to: String,
    pub points: Vec<Pt>,
    pub from_side: Side,
    pub to_side: Side,
    /// Routed by a planner (grid or architecture): drawn with the crossover halo.
    pub planned: bool,
    /// The corner radius the path is drawn with.
    pub radius: f64,
    pub stroke_width: f64,
    pub label: Option<LabelGeom>,
}

/// Everything the layout measured, un-rounded.
#[derive(Clone, Debug, PartialEq)]
pub struct LifecycleGeometry {
    pub view_box: [f64; 2],
    /// In `states` Map order (the render order).
    pub states: Vec<StateGeom>,
    pub transitions: Vec<TransitionGeom>,
    pub bands: Vec<Band>,
    pub legend: Option<Measured>,
}

fn text_shape(at: Pt, text: &str, size: f64, weight: u16, anchor: Anchor, token: Token, detail: Detail) -> Shape {
    Shape::Text(TextShape { at, text: text.to_owned(), size, weight, anchor, token, detail })
}

/// The text accent of a state type (`textClass`): neutral and external are muted.
fn accent(kind: StateType) -> Token {
    match kind {
        StateType::Neutral | StateType::External => Token::TextMuted,
        other => Token::KindStroke(Kind::from_state_type(state_type_name(other))),
    }
}

/// A filled circle as four cubic arcs.
fn circle(cx: f64, cy: f64, r: f64, token: Token) -> Shape {
    let k = r * 0.552_284_749_830_793_6;
    let cmds = vec![
        Cmd::M([cx + r, cy]),
        Cmd::C([cx + r, cy + k], [cx + k, cy + r], [cx, cy + r]),
        Cmd::C([cx - k, cy + r], [cx - r, cy + k], [cx - r, cy]),
        Cmd::C([cx - r, cy - k], [cx - k, cy - r], [cx, cy - r]),
        Cmd::C([cx + k, cy - r], [cx + r, cy - k], [cx + r, cy]),
        Cmd::Z,
    ];
    Shape::Path(PathShape {
        cmds,
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        stroke: None,
        fill: Some(Fill::new(token)),
        opacity: 1.0,
        non_scaling: false,
    })
}

/// `initialMarkerShape(dotX, tipX, y)`: the dot (r 4.5), the stem and the arrowhead ending at `tipX`.
pub fn initial_marker_shapes(dot_x: f64, tip_x: f64, y: f64) -> Vec<Shape> {
    vec![
        circle(dot_x, y, 4.5, Token::ArrowEmphasis),
        Shape::Polyline(PolylineShape {
            points: vec![[dot_x + 4.5, y], [tip_x - 6.0, y]],
            radius: 0.0,
            stroke: Stroke::solid(Token::ArrowEmphasis, 1.4),
            marker: None,
            halo: false,
        }),
        Shape::Path(PathShape {
            cmds: vec![Cmd::M([tip_x - 7.0, y - 3.5]), Cmd::L([tip_x, y]), Cmd::L([tip_x - 7.0, y + 3.5]), Cmd::Z],
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            stroke: None,
            fill: Some(Fill::new(Token::ArrowEmphasis)),
            opacity: 1.0,
            non_scaling: false,
        }),
    ]
}

fn kind_rect(rect: Bounds, radius: f64, kind: Kind, stroke: f64, filled: bool) -> Shape {
    Shape::Rect(RectShape {
        rect,
        radius,
        fill: filled.then(|| Fill::new(Token::KindFill(kind))),
        stroke: Some(Stroke::solid(Token::KindStroke(kind), stroke)),
    })
}

/// `renderSwatch`: the initial marker for `start`, the double-border rect for `final`, else the
/// `14 x 9` kind rect.
fn legend_swatch(p: &Placed) -> Vec<Shape> {
    match p.entry.kind {
        "start" => initial_marker_shapes(p.x + 4.5, p.x + 24.0, p.baseline - 3.5),
        "final" => vec![
            kind_rect(Bounds::new(p.x, p.baseline - 8.0, 14.0, 9.0), 2.0, Kind::External, 1.0, true),
            kind_rect(Bounds::new(p.x + 2.5, p.baseline - 5.5, 9.0, 4.0), 1.0, Kind::External, 0.8, false),
        ],
        kind => vec![kind_rect(Bounds::new(p.x, p.baseline - 8.0, 14.0, 9.0), 2.0, Kind::from_state_type(kind), 1.0, true)],
    }
}

fn icon_name(icon: NodeIcon) -> String {
    serde_json::to_value(icon).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// `renderState`: mask, kind rect, final border, initial marker, sigil, step, label, sublabel, tag.
fn push_state(b: &mut SceneBuilder, plan: &Plan<'_>, placed: &PlacedState, is_final: bool) {
    let state = &plan.doc.states[placed.index];
    let kind = Kind::from_state_type(state_type_name(state.kind));
    let r = placed.rect;
    let (label_font, sub_font, tag_font) = state_font_sizes(state, r.width, plan.v2);
    let (label_y, sublabel_y, tag_up, step_font) =
        if plan.v2 { (LABEL_Y, SUBLABEL_Y, TAG_UP, STEP_FONT) } else { (V1_LABEL_Y, V1_SUBLABEL_Y, V1_TAG_UP, V1_STEP_FONT) };
    let sub = state.sublabel.as_deref().filter(|s| !s.is_empty());
    let tag = state.tag.as_deref().filter(|s| !s.is_empty());
    let step = state.step.as_deref().filter(|s| !s.is_empty());
    let mut rows = vec![Row { text: &state.label, font: label_font, y: label_y }];
    if let Some(s) = sub {
        rows.push(Row { text: s, font: sub_font, y: sublabel_y });
    }
    if let Some(t) = tag {
        rows.push(Row { text: t, font: tag_font, y: r.height - tag_up });
    }
    let layout = text::node_label_layout(
        &LabelBox {
            width: r.width,
            height: r.height,
            brand: state.brand.is_some(),
            step: step.unwrap_or(""),
            ..LabelBox::default()
        },
        &rows,
    );

    let lane_label = plan.doc.lanes.iter().rev().find(|l| l.id == state.lane).map(|l| l.label.as_str());
    let mut group = Group::node(&state.id, kind, &state.label, r.into());
    group.sublabel = sub.map(str::to_owned);
    group.context = Some(match lane_label.filter(|l| !l.is_empty()) {
        Some(label) => label.to_owned(),
        None => plan.doc.meta.locale().t("node.context.lifecycle"),
    });
    group.tags = tag.map(str::to_owned).into_iter().collect();
    let id = b.group(group);

    b.push_in(id, Layer::Nodes, Shape::Rect(RectShape::mask(r.into(), STATE_RADIUS)));
    b.push_in(id, Layer::Nodes, kind_rect(r.into(), STATE_RADIUS, kind, STATE_STROKE, true));
    if is_final {
        let inner = Bounds::new(r.x + 3.0, r.y + 3.0, r.width - 6.0, r.height - 6.0);
        b.push_in(id, Layer::Nodes, kind_rect(inner, 4.0, kind, 1.0, false));
    }
    if state.kind == StateType::Start {
        for shape in initial_marker_shapes(r.x - 22.0, r.x - 1.0, r.cy()) {
            b.push_in(id, Layer::Nodes, shape);
        }
    }
    let icon = state.icon.map(icon_name);
    let type_name = state_type_name(state.kind);
    for path in sigils::sigil_paths(type_name, icon.as_deref(), r.x + sigils::SIGIL_INSET, r.y + layout.sigil_y, layout.sigil_size) {
        b.push_in(id, Layer::Nodes, Shape::Path(path));
    }
    crate::brand::push_badge(b, id, state.brand.as_ref(), r.x + r.width, r.y);
    if let Some(s) = step {
        b.push_in(id, Layer::Nodes, text_shape([r.x + 23.0, r.y + 14.0], s, step_font, 700, Anchor::Start, accent(state.kind), Detail::Fine));
    }
    b.push_in(
        id,
        Layer::Nodes,
        text_shape([r.x + layout.x, r.y + layout.ys[0]], &state.label, label_font, 600, Anchor::Middle, Token::Text, Detail::Anchor),
    );
    let mut next = 1;
    if let Some(s) = sub {
        b.push_in(id, Layer::Nodes, text_shape([placed.cx, r.y + layout.ys[next]], s, sub_font, 400, Anchor::Middle, Token::TextMuted, Detail::Context));
        next += 1;
    }
    if let Some(t) = tag {
        b.push_in(id, Layer::Nodes, text_shape([placed.cx, r.y + layout.ys[next]], t, tag_font, 400, Anchor::Middle, accent(state.kind), Detail::Fine));
    }
}

/// `renderLifecycleRail` (v1 only): the implied emphasis line behind the `main` states, from the
/// first column's right edge region (`x 154`) to `38` past the furthest occupied column's centre.
fn v1_rail(plan: &Plan<'_>) -> Option<Shape> {
    if plan.v2 {
        return None;
    }
    let cols: Vec<f64> = plan.states.iter().filter(|s| BandKind::of(&s.lane) == BandKind::Phase).map(|s| s.col).collect();
    if cols.is_empty() {
        return None;
    }
    let furthest = js_max(cols[0], cols.iter().copied());
    let end = col_index(furthest).and_then(|c| PHASE_XS.get(c)).map_or(f64::NAN, |x| x + RAIL_END_PAD);
    let y = PHASE_Y + 31.0;
    Some(Shape::Polyline(PolylineShape {
        points: vec![[RAIL_START_X, y], [end, y]],
        radius: 0.0,
        stroke: Stroke::solid(EdgeVariant::Emphasis.stroke(), RAIL_WIDTH),
        marker: Some(crate::scene::Marker { fill: EdgeVariant::Emphasis.stroke() }),
        halo: false,
    }))
}

/// Lay out a lifecycle document (v1 or v2): the scene to paint and the raw geometry. The geometry
/// gates are not run here ([`crate::gates::lifecycle`]); an authored `meta.legend` that does not fit
/// is an error, as `measureLegend` says.
pub fn build(doc: &Lifecycle) -> Result<(Scene, LifecycleGeometry), Vec<Diagnostic>> {
    let plan = Plan::new(doc).map_err(|d| vec![*d])?;
    let view_box = plan.view_box;
    let legend = legend::measure(
        &plan.entries,
        &LegendLayout {
            x: LEGEND_X,
            baseline_y: view_box[1] - LEGEND_BASELINE_UP,
            width: view_box[0] - 2.0 * LEGEND_SIDE_PAD,
            min_title_y: plan.area_bottom + 8.0,
            unfit: if doc.meta.legend.is_none() { Unfit::Hide } else { Unfit::Error },
            diagram_type: "lifecycle",
        },
    )
    .map_err(|d| vec![*d])?;

    let mut transitions = Vec::new();
    for (key, (t, route)) in doc.transitions.iter().zip(&plan.routes).enumerate() {
        let Some(route) = route else { continue };
        let variant = effective_variant(t, plan.state(&t.from), plan.state(&t.to), plan.v2);
        let emphasis = if plan.v2 { EMPHASIS_WIDTH } else { V1_EMPHASIS_WIDTH };
        let stroke_width = or_default(t.width, if variant == EdgeVariant::Emphasis { emphasis } else { TRANSITION_WIDTH });
        transitions.push(TransitionGeom {
            key,
            from: t.from.clone(),
            to: t.to.clone(),
            points: route.points.clone(),
            from_side: route.from_side,
            to_side: route.to_side,
            planned: route.planned,
            radius: route.radius,
            stroke_width,
            label: plan.labels[key],
        });
    }

    let mut b = SceneBuilder::new(view_box[0], view_box[1]);
    b.push(
        Layer::Background,
        Shape::Rect(RectShape { rect: Bounds::new(0.0, 0.0, view_box[0], view_box[1]), radius: 0.0, fill: Some(Fill::new(Token::Bg)), stroke: None }),
    );
    for band in &plan.bands {
        if plan.v2 {
            b.push(Layer::Frames, text_shape(band.at, &band.label, BAND_FONT, 600, Anchor::Middle, Token::TextDim, Detail::Anchor));
            continue;
        }
        // v1: the dotted rule `12` under the baseline, then the header.
        let rule_y = band.at[1] + 12.0;
        b.push(
            Layer::Frames,
            Shape::Polyline(PolylineShape {
                points: vec![[V1_BAND_X, rule_y], [view_box[0] - V1_BAND_X, rule_y]],
                radius: 0.0,
                stroke: Stroke::dashed(EdgeVariant::Default.stroke(), 0.8, &[3.0, 8.0]),
                marker: None,
                halo: false,
            }),
        );
        b.push(Layer::Frames, text_shape(band.at, &band.label, BAND_FONT, 600, Anchor::Start, Token::TextDim, Detail::Anchor));
    }
    if let Some(rail) = v1_rail(&plan) {
        b.push(Layer::Frames, rail);
    }
    for g in &transitions {
        let t = &doc.transitions[g.key];
        let variant = effective_variant(t, plan.state(&t.from), plan.state(&t.to), plan.v2);
        let edge = EdgeRef { key: g.key as u32, from: t.from.clone(), to: t.to.clone(), id: t.id.clone() };
        let label = label_or_note(t).unwrap_or("");
        let id = b.group(Group::edge(edge, label, &g.points, EDGE_HIT_HALF_WIDTH.max(g.stroke_width / 2.0)));
        b.push_in(
            id,
            Layer::Edges,
            Shape::Polyline(PolylineShape {
                points: g.points.clone(),
                radius: g.radius,
                stroke: Stroke { token: variant.stroke(), width: g.stroke_width, dash: variant.dash().to_vec() },
                marker: Some(crate::scene::Marker { fill: variant.stroke() }),
                halo: g.planned,
            }),
        );
    }
    for (placed, is_final) in plan.states.iter().zip(&plan.finals) {
        push_state(&mut b, &plan, placed, *is_final);
    }
    for g in &transitions {
        let Some(label) = g.label else { continue };
        let t = &doc.transitions[g.key];
        let variant = effective_variant(t, plan.state(&t.from), plan.state(&t.to), plan.v2);
        let edge = EdgeRef { key: g.key as u32, from: t.from.clone(), to: t.to.clone(), id: t.id.clone() };
        let mut group = Group::new(GroupKind::Label, format!("label-{}", g.key), label.rect.into());
        group.edge = Some(edge);
        group.label = label_or_note(t).unwrap_or("").to_owned();
        let id = b.group(group);
        let [lx, ly] = label.at;
        b.push_in(id, Layer::EdgeLabels, Shape::Rect(RectShape::mask(label.rect.into(), LABEL_RADIUS)));
        let has_label = truthy(&t.label);
        if let Some(text) = t.label.as_deref().filter(|s| !s.is_empty()) {
            b.push_in(id, Layer::EdgeLabels, text_shape([lx, ly], text, LABEL_FONT, 400, Anchor::Middle, variant.label(), Detail::Context));
        }
        if let Some(note) = t.note.as_deref().filter(|s| !s.is_empty()) {
            let y = ly + if has_label { 11.0 } else { 0.0 };
            b.push_in(id, Layer::EdgeLabels, text_shape([lx, y], note, NOTE_FONT, 400, Anchor::Middle, Token::TextDim, Detail::Fine));
        }
    }
    if let Some(m) = &legend {
        legend::push_scene(&mut b, m, &doc.meta.locale(), &legend_swatch);
    }

    let geometry = LifecycleGeometry {
        view_box,
        states: plan
            .states
            .iter()
            .zip(&plan.finals)
            .map(|(s, f)| StateGeom { id: s.id.clone(), rect: s.rect, final_border: *f, initial_marker: s.kind == StateType::Start })
            .collect(),
        transitions,
        bands: plan.bands.clone(),
        legend,
    };
    Ok((b.build(), geometry))
}
