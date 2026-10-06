//! Workflow v2 rank solver (P6.2): `canonicalReadableWorkflow`, `createReadableLayout`
//! (`workflow-compiler.mjs:200-625`), the lane geometry and node measurement the compiler derives
//! from it (`createWorkflowLaneGeometry`, `measureWorkflowNodes`), the frames it paints (lanes,
//! exception lanes, groups, phase headers), the scene label obstacles, and the geometry half of
//! `validateReadableInputsBeforeRouting` (`non-finite-node-geometry`, `node-overlap`). `00`
//! section 5.5 (Workflow v2); `03` section 2.6.
//!
//! [`place`] is the entry point: it canonicalises, solves the columns under a [`LayoutFeedback`],
//! measures every node and builds the frames. The router (P6.3) reads [`ReadablePlacement`] and its
//! lane helpers, and drives the feedback loop (`compileWorkflowWithFeedback`) with
//! [`LayoutFeedback::next`] and [`feedback_failure`]. The legend footprint, the viewBox and
//! `viewbox-capacity` are the router's (they need routed edges); `column-capacity` is v1 only
//! ([`super::legacy`]).
//!
//! # JS parity
//!
//! - **Sorting.** `Array.prototype.sort` is stable, so is `slice::sort_by`. The constraint order
//!   (`to`, `from`, `minimum`) only decides provenance order; the column values do not depend on it.
//!   String keys compare by UTF-16 code unit ([`js_cmp`]), as JS `<` does.
//! - **Sets.** Every JS `Set` of contributors is an insertion-ordered [`Prov`]. The returned
//!   contributor lists are sorted and deduplicated, so only membership reaches the output, but the
//!   `clear()`-on-a-larger-value logic decides membership and is ported as is.
//! - **Maps.** `new Map(entries)` keeps the first position of a key with its last value: lane
//!   lookups by id (`laneIndex`, `laneOrder`) and the node map follow that ([`lane_index`] is
//!   last-wins; [`ReadablePlacement::nodes`] keeps the first position, last value).
//! - **Truthiness.** `node.tag`, `edge.label`, `phase.id`, `group.id`, `lane.id`: an empty string
//!   is false. `Number(x) || 0` turns `-0`/`NaN` into `0`. `node.width || 92` (measurement) and
//!   `Number.isFinite(node.width) ? node.width : 92` (solver) differ on `0`; both are kept.
//! - **Floats.** Every sum keeps Archify's operand order: column relaxation is
//!   `colXs[from] + minimum`, shifts are applied column by column in shift order (left, lane
//!   header, measured content), lane tops sum heights from `0` before adding `laneY`, node `y` is
//!   `laneTop + 30 + groupHeader + (contentH - h) / 2 + yOffset`. `Math.max` propagates `NaN`
//!   ([`js_max`]); an out-of-range column reads `NaN` like `colXs[undefined]`.
//! - **Feedback keys.** `Object.entries(rankGapMinimums).sort()` sorts `"from:to,minimum"`
//!   strings; columns are single digits, so that is the key order of the `BTreeMap`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value, json};

use crate::diag::{Diagnostic, Subject, json_num};
use crate::geom::{Rect, is_finite_point, rects_overlap};
use crate::model::common::SchemaVersion;
use crate::model::workflow::{Edge, EdgeRoute, Group, LaneVariant, Node, Phase, Workflow};
use crate::text::units;

/// `columnCount`: logical columns `0..=5`.
pub const COLUMN_COUNT: usize = 6;
/// `baselinePitch`, `columnStart`: column `c` starts at `94 + 120c`.
pub const BASELINE_PITCH: f64 = 120.0;
pub const COLUMN_START: f64 = 94.0;
/// `channelDetourBudgetPx` (`4 * 28`).
pub const CHANNEL_DETOUR_BUDGET_PX: f64 = 112.0;
/// `MAX_READABLE_LAYOUT_FEEDBACK_ROUNDS`.
pub const MAX_FEEDBACK_ROUNDS: usize = 3;
/// The layout object's fixed fields (`laneX`, `laneY`, `laneTitleH`, `nodeW`, `nodeH`).
pub const LANE_X: f64 = 40.0;
pub const LANE_Y: f64 = 52.0;
pub const LANE_TITLE_H: f64 = 30.0;
pub const NODE_W: f64 = 92.0;
pub const NODE_H: f64 = 52.0;
/// `GROUP_FRAME_*`, `GROUP_LABEL_*`, `GROUP_NODE_INSET`.
pub const GROUP_FRAME_TOP_INSET: f64 = 8.0;
pub const GROUP_FRAME_BOTTOM_INSET: f64 = 4.0;
pub const GROUP_LABEL_BASELINE_OFFSET: f64 = -2.0;
pub const GROUP_LABEL_MASK_ASCENT: f64 = 10.0;
pub const GROUP_LABEL_MASK_H: f64 = 14.0;
pub const GROUP_NODE_INSET: f64 = 4.0;
/// Phase header: the line at y 35, the mask rect `y 27 h 16`, the text baseline y 39.
pub const PHASE_LINE_Y: f64 = 35.0;
pub const PHASE_MASK_Y: f64 = 27.0;
pub const PHASE_MASK_H: f64 = 16.0;
pub const PHASE_TEXT_Y: f64 = 39.0;

const EPS: f64 = 0.0001;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

// ---- JS helpers ---------------------------------------------------------------------------------

/// JS `<` on strings: UTF-16 code unit order (`stableCompare`).
pub fn js_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `String(x)` for a JS number: no `.0` on whole values, exponent form outside `[1e-6, 1e21)`.
pub fn js_num(x: f64) -> String {
    if x == 0.0 {
        return "0".to_owned();
    }
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    let a = x.abs();
    if (1e-6..1e21).contains(&a) {
        return format!("{x}");
    }
    let e = format!("{x:e}");
    match e.split_once('e') {
        Some((m, exp)) if !exp.starts_with('-') => format!("{m}e+{exp}"),
        _ => e,
    }
}

/// `Math.max(a, b)`: `NaN` wins.
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

/// `Math.min(a, b)`: `NaN` wins.
pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

fn is_int(x: f64) -> bool {
    x.is_finite() && x.fract() == 0.0
}

/// A string field read for truthiness: absent and `""` are false.
fn truthy(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|s| !s.is_empty())
}

/// `x || fallback` for an id-or-label pair (`phase.id || phase.label`).
fn or_str<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a.is_empty() { b } else { a }
}

/// `Number(x) || 0`.
fn num_or_zero(x: Option<f64>) -> f64 {
    match x {
        Some(v) if v != 0.0 && !v.is_nan() => v,
        _ => 0.0,
    }
}

/// `colXs[col]`: `NaN` (`undefined` in arithmetic) unless `col` is an integer in range.
fn col_x(col_xs: &[f64], col: f64) -> f64 {
    if is_int(col) && col >= 0.0 && (col as usize) < col_xs.len() {
        col_xs[col as usize]
    } else {
        f64::NAN
    }
}

fn col_index(col_xs: &[f64], col: f64) -> Option<usize> {
    (is_int(col) && col >= 0.0 && (col as usize) < col_xs.len()).then_some(col as usize)
}

/// An insertion-ordered set of contributor strings (a JS `Set`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prov(pub Vec<String>);

impl Prov {
    fn add(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.0.contains(&s) {
            self.0.push(s);
        }
    }

    fn extend<'a>(&mut self, items: impl IntoIterator<Item = &'a String>) {
        for s in items {
            self.add(s.clone());
        }
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    /// `[...set].sort(stableCompare)`.
    fn sorted(&self) -> Vec<String> {
        let mut out = self.0.clone();
        out.sort_by(|a, b| js_cmp(a, b));
        out
    }
}

// ---- node and label measures ---------------------------------------------------------------------

/// `authoredNodeWidth`.
pub fn authored_node_width(node: &Node) -> f64 {
    node.width.filter(|w| w.is_finite()).unwrap_or(NODE_W)
}

/// `authoredNodeHeight`.
pub fn authored_node_height(node: &Node) -> f64 {
    node.height
        .filter(|h| h.is_finite())
        .unwrap_or(if truthy(&node.tag) { 68.0 } else { NODE_H })
}

/// `nodeWidthContributor`.
fn node_width_contributor(node: &Node) -> String {
    format!("node {} width {}px", node.id, js_num(authored_node_width(node)))
}

/// `workflowLabelWidth`: the edge label mask width.
pub fn workflow_label_width(label: &str) -> f64 {
    js_max(30.0, units(label) as f64 * 4.8 + 10.0)
}

/// `verticalIntervalsOverlap`.
fn vertical_intervals_overlap(a: &Node, b: &Node, clearance: f64) -> bool {
    let ac = num_or_zero(a.y_offset);
    let bc = num_or_zero(b.y_offset);
    (ac - bc).abs() < authored_node_height(a) / 2.0 + authored_node_height(b) / 2.0 + clearance
}

/// `workflowEdgeName`.
pub fn workflow_edge_name(edge: &Edge) -> String {
    match edge.id.as_deref() {
        Some(id) if !id.is_empty() => id.to_owned(),
        _ => format!("{}->{}", edge.from, edge.to),
    }
}

fn route_name(route: Option<EdgeRoute>) -> &'static str {
    match route {
        None => "",
        Some(EdgeRoute::Auto) => "auto",
        Some(EdgeRoute::Straight) => "straight",
        Some(EdgeRoute::Drop) => "drop",
        Some(EdgeRoute::OutsideRight) => "outside-right",
        Some(EdgeRoute::ReturnLeft) => "return-left",
        Some(EdgeRoute::BottomChannel) => "bottom-channel",
        Some(EdgeRoute::UpChannel) => "up-channel",
    }
}

/// `edge.route || 'auto'` is `auto` or `straight`.
fn auto_or_straight(edge: &Edge) -> bool {
    matches!(edge.route, None | Some(EdgeRoute::Auto) | Some(EdgeRoute::Straight))
}

fn lane_prefix(variant: Option<LaneVariant>, position: usize) -> String {
    if variant == Some(LaneVariant::Exception) {
        "EX".to_owned()
    } else {
        format!("{:02}", position + 1)
    }
}

// ---- canonical form -----------------------------------------------------------------------------

/// `stableValueKey`: canonical JSON with sorted keys and JS number text.
pub fn stable_value_key(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(stable_value_key).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| js_cmp(a, b));
            let parts: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", Value::String((*k).clone()), stable_value_key(&map[*k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Number(n) => js_num(n.as_f64().unwrap_or(f64::NAN)),
        other => other.to_string(),
    }
}

fn edge_key(edge: &Edge) -> String {
    serde_json::to_value(edge).map(|v| stable_value_key(&v)).unwrap_or_default()
}

/// The canonical document plus, per canonical node/edge, its authored index (`sourceIndexes`).
#[derive(Debug, Clone)]
pub struct Canonical {
    pub workflow: Workflow,
    pub node_source: Vec<usize>,
    pub edge_source: Vec<usize>,
}

/// `new Map(lanes.map((lane, index) => [lane.id, index]))`: the last index of a duplicated id.
pub fn lane_index(workflow: &Workflow, id: &str) -> Option<usize> {
    workflow.lanes.iter().rposition(|l| l.id == id)
}

/// `canonicalReadableWorkflow`: v2 ignores array order, so nodes sort by (lane order, col, id),
/// edges by (id, from, to, label, route, whole value), phases by (from, to, id), groups by (lane
/// order, from, to, id). A v1 document is returned as is.
pub fn canonicalize(w: &Workflow) -> Canonical {
    let mut workflow = w.clone();
    let identity = |n: usize| (0..n).collect::<Vec<_>>();
    if w.schema_version != SchemaVersion::V2 {
        return Canonical { node_source: identity(w.nodes.len()), edge_source: identity(w.edges.len()), workflow };
    }
    let lane_order = |lane: &str| lane_index(w, lane).map_or(MAX_SAFE_INTEGER, |i| i as i64);
    // `a - b || ...`: a NaN difference is falsy and falls through, as `partial_cmp` → `Equal` does.
    let num_cmp = |a: f64, b: f64| (a - b).partial_cmp(&0.0).unwrap_or(Ordering::Equal);
    let s = |o: &Option<String>| o.clone().unwrap_or_default();

    let mut nodes: Vec<usize> = identity(w.nodes.len());
    nodes.sort_by(|&a, &b| {
        let (l, r) = (&w.nodes[a], &w.nodes[b]);
        lane_order(&l.lane)
            .cmp(&lane_order(&r.lane))
            .then_with(|| num_cmp(l.col, r.col))
            .then_with(|| js_cmp(&l.id, &r.id))
    });
    let mut edges: Vec<usize> = identity(w.edges.len());
    edges.sort_by(|&a, &b| {
        let (l, r) = (&w.edges[a], &w.edges[b]);
        js_cmp(&s(&l.id), &s(&r.id))
            .then_with(|| js_cmp(&l.from, &r.from))
            .then_with(|| js_cmp(&l.to, &r.to))
            .then_with(|| js_cmp(&s(&l.label), &s(&r.label)))
            .then_with(|| js_cmp(route_name(l.route), route_name(r.route)))
            .then_with(|| js_cmp(&edge_key(l), &edge_key(r)))
    });
    workflow.nodes = nodes.iter().map(|&i| w.nodes[i].clone()).collect();
    workflow.edges = edges.iter().map(|&i| w.edges[i].clone()).collect();
    if let Some(phases) = workflow.phases.as_mut() {
        phases.sort_by(|l: &Phase, r: &Phase| {
            num_cmp(l.from_col, r.from_col)
                .then_with(|| num_cmp(l.to_col, r.to_col))
                .then_with(|| js_cmp(&l.id, &r.id))
        });
    }
    if let Some(groups) = workflow.groups.as_mut() {
        groups.sort_by(|l: &Group, r: &Group| {
            lane_order(&l.lane)
                .cmp(&lane_order(&r.lane))
                .then_with(|| num_cmp(l.from_col, r.from_col))
                .then_with(|| num_cmp(l.to_col, r.to_col))
                .then_with(|| js_cmp(&l.id, &r.id))
        });
    }
    Canonical { workflow, node_source: nodes, edge_source: edges }
}

// ---- router feedback ----------------------------------------------------------------------------

/// `layoutFeedback`: what the router asked of the solver in earlier rounds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutFeedback {
    /// `rankGapMinimums`, keyed `"from:to"`.
    pub rank_gap_minimums: BTreeMap<String, f64>,
    /// `rankGapContributors`, same keys.
    pub rank_gap_contributors: BTreeMap<String, Vec<String>>,
    /// `laneGapMin`.
    pub lane_gap_min: Option<f64>,
    /// `laneGapContributors`.
    pub lane_gap_contributors: Vec<String>,
}

/// What a `WorkflowLayoutFeedback` request asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum FeedbackKind {
    /// `rank-gap-minimum`: columns `from_col..to_col` at least `minimum` apart.
    RankGapMinimum { from_col: f64, to_col: f64, minimum: f64 },
    /// `lane-gap-minimum`: lanes at least `minimum` apart.
    LaneGapMinimum { minimum: f64 },
}

/// A `WorkflowLayoutFeedback` request, thrown by the router when no automatic candidate fits.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackRequest {
    pub kind: FeedbackKind,
    pub edge: Option<String>,
    pub from: String,
    pub to: String,
    pub attempted_candidate_families: Vec<String>,
    pub candidate_count: usize,
}

impl FeedbackRequest {
    /// `request.edge || `${request.from}->${request.to}``.
    fn name(&self) -> String {
        match self.edge.as_deref() {
            Some(e) if !e.is_empty() => e.to_owned(),
            _ => format!("{}->{}", self.from, self.to),
        }
    }
}

impl LayoutFeedback {
    /// One step of `compileWorkflowWithFeedback`: the feedback for the next attempt, or `None` when
    /// the request adds nothing (the caller then fails with [`feedback_failure`], as it does after
    /// [`MAX_FEEDBACK_ROUNDS`] retries).
    pub fn next(&self, request: &FeedbackRequest) -> Option<LayoutFeedback> {
        match request.kind {
            FeedbackKind::RankGapMinimum { from_col, to_col, minimum } => {
                if !(is_int(from_col) && is_int(to_col) && minimum.is_finite()) {
                    return None;
                }
                let key = format!("{}:{}", js_num(from_col), js_num(to_col));
                let current = self.rank_gap_minimums.get(&key).copied().unwrap_or(f64::NEG_INFINITY);
                if minimum <= current + EPS {
                    return None;
                }
                let mut next = self.clone();
                next.rank_gap_minimums.insert(key.clone(), minimum);
                next.rank_gap_contributors.insert(
                    key,
                    vec![
                        format!("rank {}→{} route clearance", js_num(from_col), js_num(to_col)),
                        format!("edge {} route", request.name()),
                    ],
                );
                Some(next)
            }
            FeedbackKind::LaneGapMinimum { minimum } => {
                if !(minimum.is_finite() && minimum > self.lane_gap_min.unwrap_or(f64::NEG_INFINITY) + EPS) {
                    return None;
                }
                let mut next = self.clone();
                next.lane_gap_min = Some(minimum);
                next.lane_gap_contributors = vec![format!("edge {} lane-gap route clearance", request.name())];
                Some(next)
            }
        }
    }
}

/// `feedbackFailure`: the error text and its `workflow/solver-budget-exhausted` diagnostic.
pub fn feedback_failure(request: &FeedbackRequest) -> (String, Diagnostic) {
    let message = format!(
        "Workflow edge \"{}\" exhausted bounded readable-v2 layout feedback without a feasible automatic route.",
        request.name()
    );
    let mut evidence = Map::new();
    evidence.insert("attemptedCandidateFamilies".into(), json!(request.attempted_candidate_families));
    evidence.insert("candidateCount".into(), json!(request.candidate_count));
    let diagnostic = Diagnostic::error("workflow/solver-budget-exhausted", &message)
        .with_subject(
            Subject::of("workflow")
                .with_rule("workflow/solver-budget-exhausted")
                .with_extra("edge", json!(request.edge))
                .with_extra("from", json!(request.from))
                .with_extra("to", json!(request.to)),
        )
        .with_evidence(evidence);
    (message, diagnostic)
}

// ---- the solver ---------------------------------------------------------------------------------

/// The layout object `createReadableLayout` returns (its fixed fields are the `LANE_*`/`NODE_*`
/// constants).
#[derive(Debug, Clone, PartialEq)]
pub struct ReadableLayout {
    /// `colXs`: the centre x of each column.
    pub col_xs: [f64; COLUMN_COUNT],
    pub lane_w: f64,
    /// `laneH`: the shared lane height (before group reserves).
    pub lane_h: f64,
    /// `laneHeights`: per lane, with group header/footer reserves.
    pub lane_heights: Vec<f64>,
    pub lane_gap: f64,
    pub group_header_heights: Vec<f64>,
    pub group_footer_heights: Vec<f64>,
    /// `defaultViewBoxWidth` (`40 + laneW + 16`).
    pub default_view_box_width: f64,
    /// `channelLabelEdgeKeys`, per canonical edge: the label goes through an automatic channel
    /// instead of widening the rank gap.
    pub channel_label_edges: Vec<bool>,
    /// Sorted, deduplicated (`viewbox-capacity` evidence).
    pub width_contributors: Vec<String>,
    pub height_contributors: Vec<String>,
}

#[derive(Debug, Clone)]
struct Constraint {
    from: f64,
    to: f64,
    minimum: f64,
    contributors: Vec<String>,
}

impl Constraint {
    fn new(from: f64, to: f64, minimum: f64, contributors: Vec<String>) -> Self {
        Constraint { from, to, minimum, contributors }
    }
}

/// `hasAbsoluteWorkflowPins`.
pub fn has_absolute_workflow_pins(w: &Workflow) -> bool {
    w.edges.iter().any(|e| e.via.is_some() || e.label_at.is_some() || e.channel_x.is_some() || e.channel_y.is_some())
}

/// `hasVerticalStack`: two nodes of one lane and column with different `yOffset`s.
pub fn has_vertical_stack(w: &Workflow) -> bool {
    let mut offsets: HashMap<(&str, i64), Vec<f64>> = HashMap::new();
    for node in &w.nodes {
        if !is_int(node.col) {
            continue;
        }
        let set = offsets.entry((node.lane.as_str(), node.col as i64)).or_default();
        let offset = num_or_zero(node.y_offset);
        if !set.contains(&offset) {
            set.push(offset);
        }
        if set.len() > 1 {
            return true;
        }
    }
    false
}

/// `usesIndependentLaneMeasurement`.
pub fn uses_independent_lane_measurement(w: &Workflow) -> bool {
    has_vertical_stack(w) && w.meta.view_box.is_none() && !has_absolute_workflow_pins(w)
}

/// `{x, width, cx}` of a phase/group span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub x: f64,
    pub width: f64,
    pub cx: f64,
}

/// `readableGroupBounds`: `[col[from] - 50, col[to] + 50]`, widened to the label
/// (`units · 5.6 + 20`) and to every member node `± 4`. An invalid range is `{0, 0, 0}`.
pub fn readable_group_bounds(w: &Workflow, group: &Group, col_xs: &[f64]) -> Span {
    let n = col_xs.len() as f64;
    if !is_int(group.from_col) || !is_int(group.to_col) || group.from_col < 0.0 || group.from_col > group.to_col || group.to_col >= n
    {
        return Span { x: 0.0, width: 0.0, cx: 0.0 };
    }
    let start = col_xs[group.from_col as usize] - 50.0;
    let end = col_xs[group.to_col as usize] + 50.0;
    let natural = end - start;
    let minimum = units(&group.label) as f64 * 5.6 + 20.0;
    let mut width = js_max(natural, minimum);
    let mut left = if group.from_col == group.to_col && width > natural { start } else { (start + end - width) / 2.0 };
    let mut right = left + width;
    for node in &w.nodes {
        if node.lane != group.lane
            || !is_int(node.col)
            || node.col < group.from_col
            || node.col > group.to_col
            || node.col < 0.0
            || node.col >= n
        {
            continue;
        }
        let half = authored_node_width(node) / 2.0;
        left = js_min(left, col_xs[node.col as usize] - half - GROUP_NODE_INSET);
        right = js_max(right, col_xs[node.col as usize] + half + GROUP_NODE_INSET);
    }
    width = right - left;
    Span { x: left, width, cx: left + width / 2.0 }
}

fn valid_range(from: f64, to: f64) -> bool {
    is_int(from) && is_int(to) && from >= 0.0 && from <= to && to < COLUMN_COUNT as f64
}

/// `createReadableLayout` (`workflow-compiler.mjs:200-625`) over a *canonical* document.
pub fn create_layout(w: &Workflow, feedback: &LayoutFeedback) -> ReadableLayout {
    let nodes = &w.nodes;
    let mut nodes_by_id: HashMap<&str, &Node> = HashMap::new();
    for node in nodes {
        nodes_by_id.insert(node.id.as_str(), node); // last wins, as `new Map(entries)`
    }
    let mut constraints: Vec<Constraint> = Vec::new();
    let mut feedback_constraints: Vec<Constraint> = Vec::new();
    let mut channel_label_edges = vec![false; w.edges.len()];
    let mut width_contributors = Prov::default();
    let mut height_contributors = Prov::default();

    for col in 0..COLUMN_COUNT - 1 {
        constraints.push(Constraint::new(col as f64, (col + 1) as f64, BASELINE_PITCH, Vec::new()));
    }
    for (key, &minimum) in &feedback.rank_gap_minimums {
        let mut parts = key.split(':').map(|p| p.parse::<f64>().unwrap_or(f64::NAN));
        let from = parts.next().unwrap_or(f64::NAN);
        let to = parts.next().unwrap_or(f64::NAN);
        let contributors = feedback
            .rank_gap_contributors
            .get(key)
            .cloned()
            .unwrap_or_else(|| vec![format!("rank {}→{} route clearance", js_num(from), js_num(to))]);
        constraints.push(Constraint::new(from, to, minimum, contributors));
    }

    // Same-lane nodes that overlap vertically keep their widths apart.
    for (li, left) in nodes.iter().enumerate() {
        for right in &nodes[li + 1..] {
            if left.lane != right.lane || left.col == right.col {
                continue;
            }
            if !vertical_intervals_overlap(left, right, 8.0) {
                continue;
            }
            let (a, b) = if left.col < right.col { (left, right) } else { (right, left) };
            constraints.push(Constraint::new(
                a.col,
                b.col,
                authored_node_width(a) / 2.0 + 8.0 + authored_node_width(b) / 2.0,
                vec![
                    format!("rank {}→{} node width clearance", js_num(a.col), js_num(b.col)),
                    node_width_contributor(a),
                    node_width_contributor(b),
                ],
            ));
        }
    }

    // Direct same-lane edges: 28 px of clearance, and room for the label mask (a feedback
    // constraint, applied in the second round) unless the label goes through a channel.
    for (ei, edge) in w.edges.iter().enumerate() {
        let (Some(&from), Some(&to)) = (nodes_by_id.get(edge.from.as_str()), nodes_by_id.get(edge.to.as_str())) else {
            continue;
        };
        if from.lane != to.lane || from.col == to.col {
            continue;
        }
        if edge.via.is_some() || edge.channel_x.is_some() || edge.channel_y.is_some() || !auto_or_straight(edge) {
            continue;
        }
        if num_or_zero(from.y_offset) != num_or_zero(to.y_offset) {
            continue;
        }
        let (earlier, later) = if from.col < to.col { (from, to) } else { (to, from) };
        let labeled = truthy(&edge.label) && edge.label_at.is_none();
        let labeled_clearance =
            if labeled { js_max(28.0, workflow_label_width(edge.label.as_deref().unwrap_or("")) + 8.0) } else { 28.0 };
        let expansion = js_max(0.0, labeled_clearance - 28.0);
        let can_channel = labeled
            && matches!(edge.route, None | Some(EdgeRoute::Auto))
            && edge.from_side.is_none()
            && edge.to_side.is_none()
            && edge.channel_x.is_none()
            && edge.channel_y.is_none();
        let prefer_channel = can_channel && expansion > CHANNEL_DETOUR_BUDGET_PX;
        if prefer_channel {
            channel_label_edges[ei] = true;
        }
        let (ec, lc) = (js_num(earlier.col), js_num(later.col));
        constraints.push(Constraint::new(
            earlier.col,
            later.col,
            authored_node_width(earlier) / 2.0 + 28.0 + authored_node_width(later) / 2.0,
            vec![
                format!("rank {ec}→{lc} direct clearance"),
                format!("rank {ec}→{lc} node width clearance"),
                node_width_contributor(earlier),
                node_width_contributor(later),
            ],
        ));
        if !prefer_channel && labeled_clearance > 28.0 {
            feedback_constraints.push(Constraint::new(
                earlier.col,
                later.col,
                authored_node_width(earlier) / 2.0 + labeled_clearance + authored_node_width(later) / 2.0,
                vec![
                    format!("rank {ec}→{lc} direct clearance"),
                    format!("edge {} label mask", workflow_edge_name(edge)),
                    node_width_contributor(earlier),
                    node_width_contributor(later),
                ],
            ));
        }
    }

    // Phase and group label spans.
    for phase in w.phases.iter().flatten() {
        if !valid_range(phase.from_col, phase.to_col) {
            continue;
        }
        let minimum_width = units(&phase.label) as f64 * 5.6 + 8.0;
        let who = vec![format!("phase {} label span", or_str(&phase.id, &phase.label))];
        if phase.from_col == phase.to_col {
            if phase.to_col < (COLUMN_COUNT - 1) as f64 {
                let minimum = BASELINE_PITCH + js_max(0.0, minimum_width - 92.0);
                constraints.push(Constraint::new(phase.to_col, phase.to_col + 1.0, minimum, who));
            }
            continue;
        }
        constraints.push(Constraint::new(phase.from_col, phase.to_col, js_max(0.0, minimum_width - 92.0), who));
    }
    for group in w.groups.iter().flatten() {
        if !valid_range(group.from_col, group.to_col) {
            continue;
        }
        let minimum_width = units(&group.label) as f64 * 5.6 + 20.0;
        let who = vec![format!("group {} label span", or_str(&group.id, &group.label))];
        if group.from_col == group.to_col {
            if group.to_col < (COLUMN_COUNT - 1) as f64 {
                let minimum = BASELINE_PITCH + js_max(0.0, minimum_width - 100.0);
                constraints.push(Constraint::new(group.to_col, group.to_col + 1.0, minimum, who));
            }
            continue;
        }
        constraints.push(Constraint::new(group.from_col, group.to_col, js_max(0.0, minimum_width - 100.0), who));
    }

    // Relax left to right; one feedback round adds the label-mask constraints.
    let baseline = |col: usize| COLUMN_START + col as f64 * BASELINE_PITCH;
    let mut active = constraints;
    let mut col_xs = [0.0; COLUMN_COUNT];
    let mut prov: Vec<Prov> = Vec::new();
    for iteration in 0..3 {
        col_xs = std::array::from_fn(baseline);
        prov = vec![Prov::default(); COLUMN_COUNT];
        let mut ordered: Vec<&Constraint> = active
            .iter()
            .filter(|c| {
                is_int(c.from)
                    && is_int(c.to)
                    && c.from >= 0.0
                    && c.from < c.to
                    && c.to < COLUMN_COUNT as f64
                    && c.minimum.is_finite()
            })
            .collect();
        // Stable, as `Array.prototype.sort`; the differences are finite here.
        ordered.sort_by(|a, b| {
            (a.to - b.to)
                .partial_cmp(&0.0)
                .unwrap_or(Ordering::Equal)
                .then_with(|| (a.from - b.from).partial_cmp(&0.0).unwrap_or(Ordering::Equal))
                .then_with(|| (a.minimum - b.minimum).partial_cmp(&0.0).unwrap_or(Ordering::Equal))
        });
        for to in 1..COLUMN_COUNT {
            for c in ordered.iter().filter(|c| c.to as usize == to) {
                let from = c.from as usize;
                let candidate = col_xs[from] + c.minimum;
                let mut candidate_prov = prov[from].clone();
                candidate_prov.extend(&c.contributors);
                if candidate > col_xs[to] + EPS {
                    col_xs[to] = candidate;
                    prov[to] = candidate_prov;
                } else if (candidate - col_xs[to]).abs() <= EPS && candidate > baseline(to) + EPS {
                    prov[to].extend(&candidate_prov.0);
                }
            }
        }
        if iteration > 0 || feedback_constraints.is_empty() {
            break;
        }
        active.extend(feedback_constraints.iter().cloned());
    }

    // Left shift: the widest first-rank node clears the lane edge by 8.
    let first_rank: Vec<&Node> = nodes.iter().filter(|n| n.col == 0.0).collect();
    let first_extent = first_rank.iter().fold(46.0, |m, n| js_max(m, authored_node_width(n) / 2.0));
    let left_shift = js_max(0.0, LANE_X + 8.0 + first_extent - col_xs[0]);
    if left_shift != 0.0 && !left_shift.is_nan() {
        for x in col_xs.iter_mut() {
            *x += left_shift;
        }
        for node in &first_rank {
            if (authored_node_width(node) / 2.0 - first_extent).abs() > EPS {
                continue;
            }
            for p in prov.iter_mut() {
                p.add(node_width_contributor(node));
            }
        }
    }

    // Lane header shift: an unpinned top-side endpoint must clear its lane's header text.
    let mut top_endpoints: Vec<&str> = Vec::new();
    for edge in &w.edges {
        if edge.via.is_some() || edge.channel_x.is_some() {
            continue;
        }
        for (side, id) in [(edge.from_side, &edge.from), (edge.to_side, &edge.to)] {
            if side == Some(crate::model::common::Side::Top) && !top_endpoints.contains(&id.as_str()) {
                top_endpoints.push(id);
            }
        }
    }
    let mut header_shift = 0.0;
    let mut header_who = Prov::default();
    for id in top_endpoints {
        let Some(node) = nodes_by_id.get(id) else { continue };
        if col_index(&col_xs, node.col).is_none() {
            continue;
        }
        let Some(position) = lane_index(w, &node.lane) else { continue };
        let lane = &w.lanes[position];
        let header_right = LANE_X
            + 14.0
            + units(&format!("{} / {}", lane_prefix(lane.variant, position), lane.label)) as f64 * 6.2;
        let required = header_right + 2.0 - col_xs[node.col as usize];
        if required > header_shift + EPS {
            header_shift = required;
            header_who.clear();
            header_who.add(format!("lane {} label width", lane.id));
        } else if required > 0.0 && (required - header_shift).abs() <= EPS {
            header_who.add(format!("lane {} label width", lane.id));
        }
    }
    if header_shift > 0.0 {
        for x in col_xs.iter_mut() {
            *x += header_shift;
        }
        for p in prov.iter_mut() {
            p.extend(&header_who.0);
        }
    }

    // Measured content: channel labels, phase headers and group frames stay right of the canvas
    // edge (16, 16, 44).
    let mut content_shift = w.edges.iter().enumerate().fold(0.0, |m, (ei, edge)| {
        if !channel_label_edges[ei] {
            return m;
        }
        let (Some(from), Some(to)) = (nodes_by_id.get(edge.from.as_str()), nodes_by_id.get(edge.to.as_str())) else {
            return m;
        };
        let center = (col_x(&col_xs, from.col) + col_x(&col_xs, to.col)) / 2.0;
        let left = center - workflow_label_width(edge.label.as_deref().unwrap_or("")) / 2.0;
        js_max(m, 16.0 - left)
    });
    for phase in w.phases.iter().flatten() {
        if !is_int(phase.from_col) || !is_int(phase.to_col) {
            continue;
        }
        let (fx, tx) = (col_x(&col_xs, phase.from_col), col_x(&col_xs, phase.to_col));
        let width = js_max(tx - fx + 92.0, units(&phase.label) as f64 * 5.6 + 8.0);
        let left = if phase.from_col == phase.to_col { fx - 46.0 } else { (fx + tx - width) / 2.0 };
        content_shift = js_max(content_shift, 16.0 - left);
    }
    for group in w.groups.iter().flatten() {
        if !is_int(group.from_col) || !is_int(group.to_col) {
            continue;
        }
        let bounds = readable_group_bounds(w, group, &col_xs);
        content_shift = js_max(content_shift, 44.0 - bounds.x);
    }
    if content_shift > 0.0 {
        for x in col_xs.iter_mut() {
            *x += content_shift;
        }
    }

    // Lane width: the rightmost node, group frame or lane header.
    let mut rightmost = col_xs[COLUMN_COUNT - 1] + 50.0;
    let mut rightmost_who = prov[COLUMN_COUNT - 1].clone();
    for node in nodes {
        let Some(col) = col_index(&col_xs, node.col) else { continue };
        let node_right = col_xs[col] + authored_node_width(node) / 2.0;
        let mut who = prov[col].clone();
        who.add(node_width_contributor(node));
        if node_right > rightmost + EPS {
            rightmost = node_right;
            rightmost_who = who;
        } else if (node_right - rightmost).abs() <= EPS {
            rightmost_who.extend(&who.0);
        }
    }
    let empty = Prov::default();
    let prov_at = |col: f64| col_index(&col_xs, col).map_or(&empty, |c| &prov[c]);
    for group in w.groups.iter().flatten() {
        if !is_int(group.from_col) || !is_int(group.to_col) {
            continue;
        }
        let bounds = readable_group_bounds(w, group, &col_xs);
        let group_right = bounds.x + bounds.width;
        let mut who = prov_at(group.from_col).clone();
        who.extend(&prov_at(group.to_col).0);
        who.add(format!("group {} label span", or_str(&group.id, &group.label)));
        for node in nodes {
            if node.lane == group.lane && node.col >= group.from_col && node.col <= group.to_col {
                who.add(node_width_contributor(node));
            }
        }
        if group_right > rightmost + EPS {
            rightmost = group_right;
            rightmost_who = who;
        } else if (group_right - rightmost).abs() <= EPS {
            rightmost_who.extend(&who.0);
        }
    }
    // `${index + 1}` here even for an exception lane (its header reads `EX / …`).
    let mut widest: (f64, Option<usize>) = (0.0, None);
    for (index, lane) in w.lanes.iter().enumerate() {
        let width = units(&format!("{:02} / {}", index + 1, lane.label)) as f64 * 6.2 + 30.0;
        if width > widest.0 {
            widest = (width, Some(index));
        }
    }
    let lane_label_width = widest.0;
    let rightmost_lane_width = (rightmost - LANE_X + 8.0).ceil();
    let lane_w = js_max(js_max(640.0, rightmost_lane_width), lane_label_width.ceil());
    if lane_w > 640.0 {
        if rightmost_lane_width == lane_w {
            width_contributors.extend(&rightmost_who.0);
        }
        if lane_label_width.ceil() == lane_w
            && let Some(index) = widest.1
        {
            let lane = &w.lanes[index];
            width_contributors.add(format!("lane {} label width", or_str(&lane.id, &lane.label)));
        }
    }

    // Lane heights: the tallest node (with its offset), per lane under a vertical stack.
    let vertical_extent = |lane: Option<&str>| {
        let mut maximum = 0.0;
        let mut who = Prov::default();
        for node in nodes {
            if lane.is_some_and(|l| node.lane != l) {
                continue;
            }
            let y_offset = num_or_zero(node.y_offset);
            let height = authored_node_height(node);
            let extent = height / 2.0 + y_offset.abs();
            let suffix =
                if y_offset != 0.0 { format!(" with yOffset {}px", js_num(y_offset)) } else { String::new() };
            let contributor = format!("node {} height {}px{suffix}", node.id, js_num(height));
            if extent > maximum + EPS {
                maximum = extent;
                who.clear();
                who.add(contributor);
            } else if (extent - maximum).abs() <= EPS {
                who.add(contributor);
            }
        }
        (maximum, who)
    };
    let lane_height_of = |extent: f64| 30.0 + js_max(74.0, (extent * 2.0 + 8.0).ceil());
    let (shared_extent, shared_who) = vertical_extent(None);
    let lane_h = lane_height_of(shared_extent);
    let independent = uses_independent_lane_measurement(w);
    let lane_base_heights: Vec<f64> = w
        .lanes
        .iter()
        .map(|lane| {
            if !independent {
                return lane_h;
            }
            let (extent, who) = vertical_extent(Some(&lane.id));
            let height = lane_height_of(extent);
            if height > 104.0 {
                height_contributors.extend(&who.0);
            }
            height
        })
        .collect();
    if !independent && lane_h > 104.0 {
        height_contributors.extend(&shared_who.0);
    }

    // Group reserves: room above a node for its group label, below it for the frame.
    let mut reserves: Vec<(f64, f64)> = Vec::with_capacity(w.lanes.len());
    for (lane_position, lane) in w.lanes.iter().enumerate() {
        let base_content = lane_base_heights[lane_position] - 30.0;
        let (mut header, mut footer) = (0.0_f64, 0.0_f64);
        for group in w.groups.iter().flatten().filter(|g| g.lane == lane.id) {
            let bounds = readable_group_bounds(w, group, &col_xs);
            let label_left = bounds.x + 10.0;
            let label_right = label_left + units(&group.label) as f64 * 5.6;
            for node in nodes {
                if node.lane != group.lane
                    || !is_int(node.col)
                    || node.col < group.from_col
                    || node.col > group.to_col
                    || node.col < 0.0
                    || node.col >= COLUMN_COUNT as f64
                {
                    continue;
                }
                let half = authored_node_width(node) / 2.0;
                let node_left = col_xs[node.col as usize] - half;
                let node_right = col_xs[node.col as usize] + half;
                let overlaps_label = node_right > label_left && node_left < label_right;
                let top_offset = (base_content - authored_node_height(node)) / 2.0 + num_or_zero(node.y_offset);
                let minimum_top = if overlaps_label { 11.0 } else { 9.0 };
                header = js_max(header, (minimum_top - top_offset).ceil());
                let bottom_margin = base_content - GROUP_FRAME_BOTTOM_INSET - top_offset - authored_node_height(node);
                footer = js_max(footer, (1.0 - bottom_margin).ceil());
            }
        }
        reserves.push((js_max(0.0, header), js_max(0.0, footer)));
    }
    let group_header_heights: Vec<f64> = reserves.iter().map(|r| r.0).collect();
    let group_footer_heights: Vec<f64> = reserves.iter().map(|r| r.1).collect();
    let lane_heights: Vec<f64> =
        reserves.iter().enumerate().map(|(i, (header, footer))| lane_base_heights[i] + header + footer).collect();
    let lane_gap = js_max(20.0, num_or_zero(feedback.lane_gap_min).ceil());
    for (index, &reserve) in group_header_heights.iter().enumerate() {
        if reserve == 0.0 || reserve.is_nan() {
            continue;
        }
        let lane = &w.lanes[index];
        height_contributors
            .add(format!("lane {} group label clearance {}px", or_str(&lane.id, &lane.label), js_num(reserve)));
    }
    for (index, &reserve) in group_footer_heights.iter().enumerate() {
        if reserve == 0.0 || reserve.is_nan() {
            continue;
        }
        let lane = &w.lanes[index];
        height_contributors
            .add(format!("lane {} group frame containment {}px", or_str(&lane.id, &lane.label), js_num(reserve)));
    }
    if lane_gap > 20.0 {
        height_contributors.extend(&feedback.lane_gap_contributors);
    }

    ReadableLayout {
        col_xs,
        lane_w,
        lane_h,
        lane_heights,
        lane_gap,
        group_header_heights,
        group_footer_heights,
        default_view_box_width: LANE_X + lane_w + 16.0,
        channel_label_edges,
        width_contributors: width_contributors.sorted(),
        height_contributors: height_contributors.sorted(),
    }
}

// ---- placement ------------------------------------------------------------------------------------

/// A measured node (`measureNode`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedNode {
    /// Position in the canonical `workflow.nodes`.
    pub index: usize,
    /// Position in the authored document (`sourceIndexes.nodes`).
    pub source_index: usize,
    pub id: String,
    pub lane: String,
    pub col: f64,
    /// `x, y, width, height`; `cx = colXs[col]`, `cy = y + height / 2`.
    pub rect: Rect,
    pub cx: f64,
    pub cy: f64,
}

/// A lane frame (`data-composition-frame-kind="lane"`, rx 10) and its exception inset (rx 8).
#[derive(Debug, Clone, PartialEq)]
pub struct LaneFrame {
    pub index: usize,
    pub id: String,
    pub label: String,
    pub rect: Rect,
    /// `lane-<i>-exception`: inset 6, class `c-security-group`.
    pub exception: Option<Rect>,
    /// `NN / label` or `EX / label`, at `(laneX + 14, y + 22)`, font 10.
    pub header: String,
}

/// A group frame (rx 9) and its label baseline (font 7).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupFrame {
    /// Position in the canonical `workflow.groups` (`group-<i>`).
    pub index: usize,
    pub id: String,
    pub label: String,
    pub lane: String,
    pub rect: Rect,
    pub label_x: f64,
    pub label_y: f64,
}

/// A phase header: the line `y 35` over `[x, x + width]`, the mask rect `(x, 27, width, 16)` and
/// the label centred at `(cx, 39)`, font 8.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaseHeader {
    /// Position in the canonical `workflow.phases`.
    pub index: usize,
    pub id: String,
    pub label: String,
    pub span: Span,
}

/// `workflowSceneLabelObstacles` kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelObstacleKind {
    LaneHeader,
    PhaseHeader,
    GroupLabel,
}

/// A text box the router keeps edges and labels away from.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelObstacle {
    pub kind: LabelObstacleKind,
    pub id: Option<String>,
    pub rect: Rect,
}

/// Everything the v2 router (P6.3) starts from: the canonical document, the solved layout, the
/// measured nodes (canonical order, the receipt's `nodes` order) and the frames.
#[derive(Debug, Clone)]
pub struct ReadablePlacement {
    pub canonical: Canonical,
    pub layout: ReadableLayout,
    pub nodes: Vec<PlacedNode>,
    pub lanes: Vec<LaneFrame>,
    pub groups: Vec<GroupFrame>,
    pub phases: Vec<PhaseHeader>,
}

/// Canonicalise, solve under `feedback`, measure and frame a v2 document.
pub fn place(authored: &Workflow, feedback: &LayoutFeedback) -> ReadablePlacement {
    let canonical = canonicalize(authored);
    let layout = create_layout(&canonical.workflow, feedback);
    let mut placement =
        ReadablePlacement { canonical, layout, nodes: Vec::new(), lanes: Vec::new(), groups: Vec::new(), phases: Vec::new() };
    placement.nodes = placement.measure_nodes();
    placement.lanes = placement.lane_frames();
    placement.groups = placement.group_frames();
    placement.phases = placement.phase_headers();
    placement
}

impl ReadablePlacement {
    pub fn workflow(&self) -> &Workflow {
        &self.canonical.workflow
    }

    /// `laneIndex.get(id)` (last index of a duplicated id).
    pub fn lane_index(&self, id: &str) -> Option<usize> {
        lane_index(self.workflow(), id)
    }

    /// `laneHeight(index)`: `laneHeights[index] ?? laneH`.
    pub fn lane_height_at(&self, index: usize) -> f64 {
        self.layout.lane_heights.get(index).copied().unwrap_or(self.layout.lane_h)
    }

    /// `laneHeight(id)`.
    pub fn lane_height(&self, id: &str) -> f64 {
        self.lane_index(id).map_or(self.layout.lane_h, |i| self.lane_height_at(i))
    }

    /// `laneGroupHeaderH(id)`.
    pub fn group_header_h(&self, id: &str) -> f64 {
        self.lane_index(id).and_then(|i| self.layout.group_header_heights.get(i).copied()).unwrap_or(0.0)
    }

    /// `laneGroupFooterH(id)`.
    pub fn group_footer_h(&self, id: &str) -> f64 {
        self.lane_index(id).and_then(|i| self.layout.group_footer_heights.get(i).copied()).unwrap_or(0.0)
    }

    /// `laneTop(id)`: `laneY + Σ preceding heights + index · laneGap`; `NaN` for an unknown lane.
    pub fn lane_top(&self, id: &str) -> f64 {
        let Some(index) = self.lane_index(id) else { return f64::NAN };
        let preceding = (0..index).fold(0.0, |total, position| total + self.lane_height_at(position));
        LANE_Y + preceding + index as f64 * self.layout.lane_gap
    }

    /// `lastLaneBottom()`.
    pub fn last_lane_bottom(&self) -> f64 {
        let lanes = self.workflow().lanes.len();
        LANE_Y + (0..lanes).fold(0.0, |total, i| total + self.lane_height_at(i)) + (lanes as f64 - 1.0) * self.layout.lane_gap
    }

    /// `autoHeight` of `createWorkflowLaneGeometry`, given the legend's extra height.
    pub fn auto_height(&self, legend_extra_height: f64) -> f64 {
        let lanes = self.workflow().lanes.len().max(1);
        LANE_Y
            + self.layout.lane_heights.iter().fold(0.0, |total, h| total + h)
            + (lanes as f64 - 1.0) * self.layout.lane_gap
            + 124.0
            + legend_extra_height
    }

    /// `legendY()`: `lastLaneBottom + 44 + legendExtraHeight`.
    pub fn legend_y(&self, legend_extra_height: f64) -> f64 {
        self.last_lane_bottom() + 44.0 + legend_extra_height
    }

    /// `gapYBetween(fromLane, toLane, bias)`.
    pub fn gap_y_between(&self, from_lane: &str, to_lane: &str, bias: f64) -> f64 {
        let a = self.lane_top(from_lane) + self.lane_height(from_lane);
        let b = self.lane_top(to_lane);
        a + (b - a) * bias
    }

    /// `spanForCols(fromCol, toCol, pad, minimumWidth)`.
    pub fn span_for_cols(&self, from_col: f64, to_col: f64, pad: f64, minimum_width: f64) -> Span {
        let start = col_x(&self.layout.col_xs, from_col) - pad;
        let end = col_x(&self.layout.col_xs, to_col) + pad;
        let width = js_max(end - start, minimum_width);
        if from_col == to_col && width > end - start {
            return Span { x: start, width, cx: start + width / 2.0 };
        }
        let cx = (start + end) / 2.0;
        Span { x: cx - width / 2.0, width, cx }
    }

    /// `phaseSpan(phase)` (v2: at least `units · 5.6 + 8` wide).
    pub fn phase_span(&self, phase: &Phase) -> Span {
        self.span_for_cols(phase.from_col, phase.to_col, 46.0, units(&phase.label) as f64 * 5.6 + 8.0)
    }

    /// `groupSpan(group)` (v2: [`readable_group_bounds`]).
    pub fn group_span(&self, group: &Group) -> Span {
        readable_group_bounds(self.workflow(), group, &self.layout.col_xs)
    }

    /// The measured node with this id (the map keeps the last duplicate's value).
    pub fn node(&self, id: &str) -> Option<&PlacedNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// `measureWorkflowNodes`: `new Map(nodes.map(n => [n.id, measureNode(n)]))`, so a duplicated
    /// id keeps its first position and its last measurement.
    fn measure_nodes(&self) -> Vec<PlacedNode> {
        let mut out: Vec<PlacedNode> = Vec::new();
        for (index, node) in self.workflow().nodes.iter().enumerate() {
            let width = node.width.filter(|w| *w != 0.0 && !w.is_nan()).unwrap_or(NODE_W);
            let height = node
                .height
                .filter(|h| *h != 0.0 && !h.is_nan())
                .unwrap_or(if truthy(&node.tag) { 68.0 } else { NODE_H });
            let cx = col_x(&self.layout.col_xs, node.col);
            let header = self.group_header_h(&node.lane);
            let content_h = self.lane_height(&node.lane) - LANE_TITLE_H - header - self.group_footer_h(&node.lane);
            let y = self.lane_top(&node.lane) + LANE_TITLE_H + header + (content_h - height) / 2.0 + num_or_zero(node.y_offset);
            let placed = PlacedNode {
                index,
                source_index: self.canonical.node_source[index],
                id: node.id.clone(),
                lane: node.lane.clone(),
                col: node.col,
                rect: Rect::new(cx - width / 2.0, y, width, height),
                cx,
                cy: y + height / 2.0,
            };
            match out.iter_mut().find(|n| n.id == node.id) {
                Some(slot) => *slot = PlacedNode { index: slot.index, ..placed },
                None => out.push(placed),
            }
        }
        out
    }

    /// `renderLane` / `workflowCompositionFrames` (lanes). The top looks the id up (last wins), the
    /// height reads the position, as Archify does.
    fn lane_frames(&self) -> Vec<LaneFrame> {
        self.workflow()
            .lanes
            .iter()
            .enumerate()
            .map(|(index, lane)| {
                let y = self.lane_top(&lane.id);
                let height = self.lane_height_at(index);
                let exception = (lane.variant == Some(LaneVariant::Exception))
                    .then(|| Rect::new(LANE_X + 6.0, y + 6.0, self.layout.lane_w - 12.0, height - 12.0));
                LaneFrame {
                    index,
                    id: lane.id.clone(),
                    label: lane.label.clone(),
                    rect: Rect::new(LANE_X, y, self.layout.lane_w, height),
                    exception,
                    header: format!("{} / {}", lane_prefix(lane.variant, index), lane.label),
                }
            })
            .collect()
    }

    /// `renderGroup` (v2): top inset 8, bottom inset 4, label baseline 2 above the frame top.
    fn group_frames(&self) -> Vec<GroupFrame> {
        self.workflow()
            .groups
            .iter()
            .flatten()
            .enumerate()
            .map(|(index, group)| {
                let span = self.group_span(group);
                let y = self.lane_top(&group.lane) + LANE_TITLE_H + GROUP_FRAME_TOP_INSET;
                let height =
                    self.lane_height(&group.lane) - LANE_TITLE_H - GROUP_FRAME_TOP_INSET - GROUP_FRAME_BOTTOM_INSET;
                GroupFrame {
                    index,
                    id: group.id.clone(),
                    label: group.label.clone(),
                    lane: group.lane.clone(),
                    rect: Rect::new(span.x, y, span.width, height),
                    label_x: span.x + 10.0,
                    label_y: y + GROUP_LABEL_BASELINE_OFFSET,
                }
            })
            .collect()
    }

    /// `renderPhase`: every phase, valid or not (an invalid one reads `NaN`).
    fn phase_headers(&self) -> Vec<PhaseHeader> {
        self.workflow()
            .phases
            .iter()
            .flatten()
            .enumerate()
            .map(|(index, phase)| PhaseHeader {
                index,
                id: phase.id.clone(),
                label: phase.label.clone(),
                span: self.phase_span(phase),
            })
            .collect()
    }

    /// `workflowSceneLabelObstacles`: lane headers, valid phase headers, group labels of valid
    /// groups in a known lane.
    pub fn label_obstacles(&self) -> Vec<LabelObstacle> {
        let w = self.workflow();
        let mut out = Vec::new();
        for lane in &self.lanes {
            out.push(LabelObstacle {
                kind: LabelObstacleKind::LaneHeader,
                id: Some(lane.id.clone()),
                rect: Rect::new(LANE_X + 14.0, self.lane_top(&lane.id) + 12.0, units(&lane.header) as f64 * 6.2, 14.0),
            });
        }
        for phase in w.phases.iter().flatten() {
            if !valid_range(phase.from_col, phase.to_col) {
                continue;
            }
            let span = self.phase_span(phase);
            out.push(LabelObstacle {
                kind: LabelObstacleKind::PhaseHeader,
                id: Some(phase.id.clone()),
                rect: Rect::new(span.x, PHASE_MASK_Y, span.width, PHASE_MASK_H),
            });
        }
        for group in w.groups.iter().flatten() {
            if self.lane_index(&group.lane).is_none() || !valid_range(group.from_col, group.to_col) {
                continue;
            }
            let span = self.group_span(group);
            let frame_y = self.lane_top(&group.lane) + LANE_TITLE_H + GROUP_FRAME_TOP_INSET;
            let baseline = frame_y + GROUP_LABEL_BASELINE_OFFSET;
            out.push(LabelObstacle {
                kind: LabelObstacleKind::GroupLabel,
                id: Some(group.id.clone()),
                rect: Rect::new(
                    span.x + 10.0,
                    baseline - GROUP_LABEL_MASK_ASCENT,
                    units(&group.label) as f64 * 5.6,
                    GROUP_LABEL_MASK_H,
                ),
            });
        }
        out
    }
}

// ---- pre-routing geometry checks -----------------------------------------------------------------

/// A thrown diagnostic error: the error text and its diagnostics.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Failure {
    pub error: String,
    /// `compilerFailure`'s diagnostics, the `--layout-json` receipt's.
    pub diagnostics: Vec<Diagnostic>,
    /// What `validate --json` reports when it is not `diagnostics`: the gates' recorded diagnostics
    /// come first, then the other problems.
    pub reported: Option<Vec<Diagnostic>>,
}

impl Failure {
    pub fn new(error: String, diagnostics: Vec<Diagnostic>) -> Self {
        Failure { error, diagnostics, reported: None }
    }

    /// The diagnostics `validate --json` prints.
    pub fn reported(&self) -> &[Diagnostic] {
        self.reported.as_deref().unwrap_or(&self.diagnostics)
    }
}

fn rect_json(r: &Rect) -> Value {
    json!({ "x": json_num(r.x), "y": json_num(r.y), "width": json_num(r.width), "height": json_num(r.height) })
}

impl ReadablePlacement {
    /// The geometry half of `validateReadableInputsBeforeRouting`, run after the graph half
    /// (`crate::graph`: duplicate ids, unknown endpoint/lane, invalid column) found nothing:
    /// `workflow/non-finite-node-geometry` for the first node (authored order) with a non-finite
    /// rect, else every same-lane pair less than 8 px apart as `workflow/node-overlap`, all at once.
    ///
    /// `accepts_col_fix(canonical_node_index, col)` verifies a fix (`acceptsFix`: the whole compile
    /// of the mutated document passes). It is the caller's, since only the full pipeline (P6.4)
    /// can answer; pass `|_, _| false` for no fixes.
    pub fn geometry_failure(&self, mut accepts_col_fix: impl FnMut(usize, usize) -> bool) -> Option<Failure> {
        let w = self.workflow();
        // `authoredNodes`: the canonical nodes back in authored order.
        let mut authored: Vec<usize> = (0..w.nodes.len()).collect();
        authored.sort_by_key(|&i| self.canonical.node_source[i]);
        // `nodeSourceIndexes`: id → authored index, last wins.
        let mut source_of: HashMap<&str, usize> = HashMap::new();
        for &i in &authored {
            source_of.insert(w.nodes[i].id.as_str(), self.canonical.node_source[i]);
        }

        let mut by_lane: Vec<(&str, Vec<&PlacedNode>)> = Vec::new();
        for &i in &authored {
            let source = &w.nodes[i];
            let Some(node) = self.node(&source.id) else { continue };
            let r = &node.rect;
            if !is_finite_point(&[r.x, r.y, node.cx, node.cy]) {
                let message = format!("Workflow node \"{}\" produced non-finite coordinates.", node.id);
                let evidence = json!({
                    "measuredRect": rect_json(r),
                    "authored": {
                        "col": json_num(source.col),
                        "width": source.width.map(json_num),
                        "height": source.height.map(json_num),
                        "yOffset": source.y_offset.map(json_num),
                    },
                });
                let diagnostic = Diagnostic::error("workflow/non-finite-node-geometry", &message)
                    .with_subject(
                        Subject::of("workflow")
                            .with_rule("workflow/non-finite-node-geometry")
                            .with_extra("node", json!(node.id))
                            .with_path(format!("/nodes/{}", self.canonical.node_source[i])),
                    )
                    .with_evidence(evidence.as_object().cloned().unwrap_or_default());
                return Some(Failure::new(message, vec![diagnostic]));
            }
            match by_lane.iter_mut().find(|(lane, _)| *lane == node.lane) {
                Some((_, list)) => list.push(node),
                None => by_lane.push((node.lane.as_str(), vec![node])),
            }
        }

        let mut overlaps: Vec<Diagnostic> = Vec::new();
        for (lane, list) in &by_lane {
            for (li, left) in list.iter().enumerate() {
                for right in &list[li + 1..] {
                    if !rects_overlap(&left.rect, &right.rect, 8.0) {
                        continue;
                    }
                    let right_index = source_of.get(right.id.as_str()).copied().unwrap_or(right.source_index);
                    let canonical_index = w.nodes.iter().position(|n| n.id == right.id).unwrap_or(right.index);
                    let fixes: Vec<String> = (0..COLUMN_COUNT)
                        .filter(|&col| col as f64 != right.col && accepts_col_fix(canonical_index, col))
                        .map(|col| format!("set /nodes/{right_index}/col to verified free column {col}"))
                        .collect();
                    let message = format!(
                        "Workflow nodes \"{}\" and \"{}\" are less than 8px apart in lane \"{lane}\".",
                        left.id, right.id
                    );
                    let evidence = json!({
                        "lane": lane,
                        "minimumClearancePx": 8,
                        "nodes": [
                            { "id": left.id, "rect": rect_json(&left.rect) },
                            { "id": right.id, "rect": rect_json(&right.rect) },
                        ],
                    });
                    overlaps.push(
                        Diagnostic::error("workflow/node-overlap", &message)
                            .with_subject(
                                Subject::of("workflow")
                                    .with_rule("workflow/node-overlap")
                                    .with_extra("node", json!(right.id))
                                    .with_path(format!("/nodes/{right_index}")),
                            )
                            .with_evidence(evidence.as_object().cloned().unwrap_or_default())
                            .with_fixes(fixes),
                    );
                }
            }
        }
        match overlaps.len() {
            0 => None,
            1 => Some(Failure::new(overlaps[0].message.clone(), overlaps)),
            _ => {
                let lines: Vec<&str> = overlaps.iter().map(|d| d.message.as_str()).collect();
                Some(Failure::new(format!("Workflow node overlap:\n- {}", lines.join("\n- ")), overlaps))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: FeedbackKind) -> FeedbackRequest {
        FeedbackRequest {
            kind,
            edge: None,
            from: "a".into(),
            to: "b".into(),
            attempted_candidate_families: vec!["facing-straight".into()],
            candidate_count: 1,
        }
    }

    #[test]
    fn js_numbers_print_like_string_of_a_number() {
        assert_eq!(js_num(0.0), "0");
        assert_eq!(js_num(-0.0), "0");
        assert_eq!(js_num(92.0), "92");
        assert_eq!(js_num(252.00000000000006), "252.00000000000006");
        assert_eq!(js_num(1e21), "1e+21");
        assert_eq!(js_num(1.5e-7), "1.5e-7");
    }

    #[test]
    fn feedback_grows_only_on_a_larger_request() {
        let rank = request(FeedbackKind::RankGapMinimum { from_col: 1.0, to_col: 2.0, minimum: 152.5 });
        let first = LayoutFeedback::default().next(&rank).unwrap();
        assert_eq!(first.rank_gap_minimums["1:2"], 152.5);
        assert_eq!(first.rank_gap_contributors["1:2"], ["rank 1→2 route clearance", "edge a->b route"]);
        assert!(first.next(&rank).is_none(), "the same minimum adds nothing");
        let lane = request(FeedbackKind::LaneGapMinimum { minimum: 32.0 });
        let second = first.next(&lane).unwrap();
        assert_eq!(second.lane_gap_min, Some(32.0));
        assert_eq!(second.lane_gap_contributors, ["edge a->b lane-gap route clearance"]);
        assert!(second.next(&lane).is_none());
        let (error, diagnostic) = feedback_failure(&lane);
        assert_eq!(
            error,
            "Workflow edge \"a->b\" exhausted bounded readable-v2 layout feedback without a feasible automatic route."
        );
        assert_eq!(diagnostic.code, "workflow/solver-budget-exhausted");
    }

    #[test]
    fn contributors_sort_by_utf16_code_unit() {
        // U+FF5E (BMP, above the surrogates) sorts after U+1F600 in UTF-16, before it in UTF-8.
        assert_eq!(js_cmp("\u{1F600}", "\u{FF5E}"), Ordering::Less);
    }
}
