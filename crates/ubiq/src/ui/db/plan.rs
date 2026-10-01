//! The "Plan" result tab: a query plan as an indented, collapsible tree, or the engine's raw text.
//!
//! The tree is [`Plan`]'s portable [`PlanNode`]s — label, detail, estimated rows and cost, and,
//! after `EXPLAIN ANALYZE`, the actual rows and milliseconds. The most expensive node is tinted
//! (`db_plan_hot`) and named: by **self** time when the plan was measured, else by **self**
//! estimated cost (a node's figure minus its children's, so the root does not win by construction).
//! Everything that decides what is shown — the hottest node, the flattened rows, the number formats
//! — is a plain function below, tested without a window; [`view`] only draws them, and a click is a
//! [`PlanAction`] applied to the tab's [`PlanState`].

use std::collections::HashSet;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use ubiq_proto::db::{Plan, PlanFormat, PlanNode};
use ubiq_proto::ids::DbSessionId;

use crate::app::AppState;
use crate::theme::{self, Family, Role};
use crate::ui::kit::{choice_pill, ghost_button, mono};

/// Which figure ranks the nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// Measured milliseconds (`EXPLAIN ANALYZE`).
    Ms,
    /// The engine's estimated cost.
    Cost,
}

/// The ranking figure of the plan: measured time if any node has it, else estimated cost.
pub fn metric_of(root: &PlanNode) -> Option<Metric> {
    fn any(node: &PlanNode, f: &impl Fn(&PlanNode) -> bool) -> bool {
        f(node) || node.children.iter().any(|c| any(c, f))
    }
    if any(root, &|n| n.actual_ms.is_some()) {
        Some(Metric::Ms)
    } else if any(root, &|n| n.est_cost.is_some()) {
        Some(Metric::Cost)
    } else {
        None
    }
}

fn figure(node: &PlanNode, metric: Metric) -> f64 {
    match metric {
        Metric::Ms => node.actual_ms,
        Metric::Cost => node.est_cost,
    }
    .filter(|v| v.is_finite())
    .unwrap_or(0.0)
}

/// What the node spent itself: its figure minus its children's, never negative.
pub fn self_figure(node: &PlanNode, metric: Metric) -> f64 {
    let children: f64 = node.children.iter().map(|c| figure(c, metric)).sum();
    (figure(node, metric) - children).max(0.0)
}

/// The path (child indexes from the root; empty = the root) of the node with the largest self
/// figure. The first one in pre-order wins a tie; `None` when nothing has a positive figure or the
/// plan is a single node (nothing to single out).
pub fn hottest(root: &PlanNode) -> Option<Vec<usize>> {
    if root.children.is_empty() {
        return None;
    }
    let metric = metric_of(root)?;
    fn walk(
        node: &PlanNode,
        metric: Metric,
        path: &mut Vec<usize>,
        best: &mut Option<(f64, Vec<usize>)>,
    ) {
        let value = self_figure(node, metric);
        if value > 0.0 && best.as_ref().is_none_or(|(b, _)| value > *b) {
            *best = Some((value, path.clone()));
        }
        for (ix, child) in node.children.iter().enumerate() {
            path.push(ix);
            walk(child, metric, path, best);
            path.pop();
        }
    }
    let mut best = None;
    walk(root, metric, &mut Vec::new(), &mut best);
    best.map(|(_, path)| path)
}

/// One visible row of the tree.
#[derive(Debug)]
pub struct FlatRow<'a> {
    pub path: Vec<usize>,
    pub depth: usize,
    pub node: &'a PlanNode,
    pub collapsed: bool,
}

/// The rows currently visible: every node whose ancestors are all expanded, in pre-order.
pub fn flatten<'a>(root: &'a PlanNode, collapsed: &HashSet<Vec<usize>>) -> Vec<FlatRow<'a>> {
    fn walk<'a>(
        node: &'a PlanNode,
        path: &mut Vec<usize>,
        collapsed: &HashSet<Vec<usize>>,
        out: &mut Vec<FlatRow<'a>>,
    ) {
        let is_collapsed = !node.children.is_empty() && collapsed.contains(path.as_slice());
        out.push(FlatRow {
            path: path.clone(),
            depth: path.len(),
            node,
            collapsed: is_collapsed,
        });
        if is_collapsed {
            return;
        }
        for (ix, child) in node.children.iter().enumerate() {
            path.push(ix);
            walk(child, path, collapsed, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(root, &mut Vec::new(), collapsed, &mut out);
    out
}

/// Paths of every node that has children (what "Collapse all" collapses).
fn branch_paths(root: &PlanNode) -> HashSet<Vec<usize>> {
    fn walk(node: &PlanNode, path: &mut Vec<usize>, out: &mut HashSet<Vec<usize>>) {
        if node.children.is_empty() {
            return;
        }
        out.insert(path.clone());
        for (ix, child) in node.children.iter().enumerate() {
            path.push(ix);
            walk(child, path, out);
            path.pop();
        }
    }
    let mut out = HashSet::new();
    walk(root, &mut Vec::new(), &mut out);
    out
}

/// A row count: `950`, `1.2k`, `3.4M`.
pub fn fmt_count(value: f64) -> String {
    let v = value.abs();
    if v >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if v >= 10_000.0 {
        format!("{:.0}k", value / 1_000.0)
    } else if v >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// Milliseconds: `0.31 ms`, `12.3 ms`, `1.25 s`.
pub fn fmt_ms(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else if ms >= 10.0 {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.2} ms")
    }
}

/// An estimated cost: `12.3`, `4.5k`.
pub fn fmt_cost(cost: f64) -> String {
    if cost >= 1_000_000.0 {
        format!("{:.1}M", cost / 1_000_000.0)
    } else if cost >= 10_000.0 {
        format!("{:.0}k", cost / 1_000.0)
    } else if cost >= 1_000.0 {
        format!("{:.1}k", cost / 1_000.0)
    } else {
        format!("{cost:.1}")
    }
}

/// The estimate shown on the right of a row: `est 1.2k rows · cost 12.3`.
pub fn estimate_text(node: &PlanNode) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(rows) = node.est_rows {
        parts.push(format!("est {} rows", fmt_count(rows)));
    }
    if let Some(cost) = node.est_cost {
        parts.push(format!("cost {}", fmt_cost(cost)));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What was measured, when it was: `actual 1000 rows · 0.31 ms`.
pub fn actual_text(node: &PlanNode) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(rows) = node.actual_rows {
        parts.push(format!("actual {} rows", fmt_count(rows)));
    }
    if let Some(ms) = node.actual_ms {
        parts.push(fmt_ms(ms));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn format_name(format: PlanFormat) -> &'static str {
    match format {
        PlanFormat::Json => "JSON",
        PlanFormat::Text => "text",
        PlanFormat::Xml => "XML",
        PlanFormat::Table => "rows",
    }
}

/// What a click on the plan does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanAction {
    /// Show the engine's raw output (`true`) or the tree.
    Raw(bool),
    ExpandAll,
    CollapseAll,
    /// Fold or unfold the node at this path.
    Toggle(Vec<usize>),
}

/// A plan and how it is being read: which nodes are folded, tree or raw.
pub struct PlanState {
    pub plan: Plan,
    pub collapsed: HashSet<Vec<usize>>,
    pub hot: Option<Vec<usize>>,
    pub metric: Option<Metric>,
    pub raw: bool,
}

impl PlanState {
    pub fn new(plan: Plan) -> Self {
        Self {
            hot: hottest(&plan.root),
            metric: metric_of(&plan.root),
            plan,
            collapsed: HashSet::new(),
            raw: false,
        }
    }

    pub fn apply(&mut self, action: PlanAction) {
        match action {
            PlanAction::Raw(raw) => self.raw = raw,
            PlanAction::ExpandAll => self.collapsed.clear(),
            PlanAction::CollapseAll => {
                self.collapsed = branch_paths(&self.plan.root);
                // The root stays open, or there would be nothing to see.
                self.collapsed.remove(&Vec::new());
            }
            PlanAction::Toggle(path) => {
                if !self.collapsed.remove(&path) {
                    self.collapsed.insert(path);
                }
            }
        }
    }
}

fn click(
    cx: &Context<AppState>,
    session: DbSessionId,
    result: usize,
    action: PlanAction,
) -> impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    let view = cx.entity();
    move |_, _, cx| {
        let action = action.clone();
        view.update(cx, |this, cx| this.db_sql_plan(session, result, action, cx));
    }
}

/// The plan tab's body: a header (Tree / Raw, fold all, the summary) over the tree or the text.
pub fn view(
    session: DbSessionId,
    result: usize,
    state: &PlanState,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let nodes = state.plan.root.count();
    let measured = state.metric == Some(Metric::Ms);
    let summary = format!(
        "{nodes} node{} · {}{}",
        if nodes == 1 { "" } else { "s" },
        format_name(state.plan.format),
        if measured { " · analyzed" } else { "" },
    );
    let header = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .h(px(theme::scaled(30.0)))
        .border_b(px(theme::hairline()))
        .border_color(theme::border())
        .child(choice_pill(
            ("db-plan-tree", result),
            "Tree",
            !state.raw,
            click(cx, session, result, PlanAction::Raw(false)),
        ))
        .child(choice_pill(
            ("db-plan-raw", result),
            "Raw",
            state.raw,
            click(cx, session, result, PlanAction::Raw(true)),
        ))
        .when(!state.raw, |this| {
            this.child(ghost_button(
                ("db-plan-expand", result),
                None,
                "Expand all",
                click(cx, session, result, PlanAction::ExpandAll),
            ))
            .child(ghost_button(
                ("db-plan-collapse", result),
                None,
                "Collapse all",
                click(cx, session, result, PlanAction::CollapseAll),
            ))
        })
        .child(div().flex_1())
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_faint())
                .child(SharedString::from(summary)),
        );

    let body = if state.raw {
        raw(session, result, &state.plan.raw)
    } else {
        tree(session, result, state, cx)
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h(px(0.))
        .child(header)
        .child(body)
        .into_any_element()
}

/// The engine's own output, a line per row so its indentation survives.
fn raw(session: DbSessionId, result: usize, text: &str) -> AnyElement {
    div()
        .id(crate::ui::eid2("db-plan-raw-scroll", session, result))
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll()
        .p_2()
        .children(text.lines().map(|line| {
            div()
                .flex_none()
                .whitespace_nowrap()
                .font_family(theme::MONO_FONT)
                .text_size(theme::font(Family::Content, Role::Dense))
                .text_color(theme::text())
                .child(SharedString::from(line.to_string()))
        }))
        .into_any_element()
}

fn tree(
    session: DbSessionId,
    result: usize,
    state: &PlanState,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let rows = flatten(&state.plan.root, &state.collapsed);
    div()
        .id(crate::ui::eid2("db-plan-scroll", session, result))
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .children(rows.into_iter().enumerate().map(|(ix, row)| {
            let has_children = !row.node.children.is_empty();
            let hot = state.hot.as_ref() == Some(&row.path);
            let glyph = match (has_children, row.collapsed) {
                (false, _) => " ",
                (true, true) => "▸",
                (true, false) => "▾",
            };
            let mut line = div()
                .id(crate::ui::eid2(
                    "db-plan-row",
                    session,
                    format!("{result}-{ix}"),
                ))
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .pl(px(theme::scaled(8.0 + 16.0 * row.depth as f32)))
                .pr_2()
                .py_0p5()
                .text_size(theme::font(Family::Content, Role::Body))
                .when(hot, |this| this.bg(theme::db_plan_hot()))
                .when(!hot, |this| this.hover(|s| s.bg(theme::hover())))
                .child(
                    div()
                        .flex_none()
                        .w(px(theme::scaled(12.0)))
                        .text_color(theme::text_faint())
                        .child(glyph),
                )
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(SharedString::from(row.node.label.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(theme::font(Family::Content, Role::Meta))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(
                            row.node.detail.clone().unwrap_or_default(),
                        )),
                )
                .when(hot, |this| {
                    this.child(mono(
                        match state.metric {
                            Some(Metric::Ms) => "slowest",
                            _ => "costliest",
                        },
                        theme::danger(),
                    ))
                })
                .when_some(estimate_text(row.node), |this, text| {
                    this.child(mono(text, theme::text_faint()))
                })
                .when_some(actual_text(row.node), |this, text| {
                    this.child(mono(text, theme::accent()))
                });
            if has_children {
                line = line.cursor_pointer().on_click(click(
                    cx,
                    session,
                    result,
                    PlanAction::Toggle(row.path.clone()),
                ));
            }
            line
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(label: &str, cost: Option<f64>, ms: Option<f64>, children: Vec<PlanNode>) -> PlanNode {
        PlanNode {
            label: label.into(),
            est_cost: cost,
            actual_ms: ms,
            children,
            ..PlanNode::default()
        }
    }

    /// root(100) -> [a(10), b(80) -> [c(70)]]
    fn costed() -> PlanNode {
        node(
            "root",
            Some(100.0),
            None,
            vec![
                node("a", Some(10.0), None, vec![]),
                node(
                    "b",
                    Some(80.0),
                    None,
                    vec![node("c", Some(70.0), None, vec![])],
                ),
            ],
        )
    }

    #[test]
    fn metric_prefers_measured_time() {
        assert_eq!(metric_of(&costed()), Some(Metric::Cost));
        let mut plan = costed();
        plan.children[0].actual_ms = Some(1.0);
        assert_eq!(metric_of(&plan), Some(Metric::Ms));
        assert_eq!(metric_of(&PlanNode::new("x")), None);
    }

    #[test]
    fn hottest_is_by_self_cost_not_total() {
        // root self = 100-90 = 10, a = 10, b self = 80-70 = 10, c = 70 -> c.
        assert_eq!(hottest(&costed()), Some(vec![1, 0]));
    }

    #[test]
    fn hottest_uses_measured_ms_when_present() {
        let plan = node(
            "root",
            Some(1.0),
            Some(50.0),
            vec![
                node("a", Some(1000.0), Some(5.0), vec![]),
                node("b", Some(1.0), Some(40.0), vec![]),
            ],
        );
        assert_eq!(hottest(&plan), Some(vec![1]));
    }

    #[test]
    fn hottest_none_for_single_node_or_no_figures() {
        assert_eq!(hottest(&node("x", Some(5.0), None, vec![])), None);
        let flat = node("r", None, None, vec![PlanNode::new("a")]);
        assert_eq!(hottest(&flat), None);
    }

    #[test]
    fn hottest_first_wins_a_tie() {
        let plan = node(
            "r",
            Some(20.0),
            None,
            vec![
                node("a", Some(10.0), None, vec![]),
                node("b", Some(10.0), None, vec![]),
            ],
        );
        assert_eq!(hottest(&plan), Some(vec![0]));
    }

    #[test]
    fn flatten_follows_collapse() {
        let plan = costed();
        let open = flatten(&plan, &HashSet::new());
        let labels: Vec<_> = open.iter().map(|r| r.node.label.as_str()).collect();
        assert_eq!(labels, ["root", "a", "b", "c"]);
        assert_eq!(open[3].depth, 2);
        assert_eq!(open[3].path, vec![1, 0]);

        let closed = flatten(&plan, &HashSet::from([vec![1]]));
        assert_eq!(closed.len(), 3);
        assert!(closed[2].collapsed);

        // A leaf in the set is not "collapsed".
        let leaf = flatten(&plan, &HashSet::from([vec![0]]));
        assert_eq!(leaf.len(), 4);
        assert!(!leaf[1].collapsed);

        let root_closed = flatten(&plan, &HashSet::from([vec![]]));
        assert_eq!(root_closed.len(), 1);
    }

    #[test]
    fn actions_fold_the_tree() {
        let mut state = PlanState::new(Plan {
            root: costed(),
            raw: String::new(),
            format: PlanFormat::Text,
        });
        assert_eq!(state.hot, Some(vec![1, 0]));
        state.apply(PlanAction::CollapseAll);
        // The root stays open: only `b` has children besides it.
        assert_eq!(state.collapsed, HashSet::from([vec![1]]));
        state.apply(PlanAction::Toggle(vec![1]));
        assert!(state.collapsed.is_empty());
        state.apply(PlanAction::Toggle(vec![1]));
        state.apply(PlanAction::ExpandAll);
        assert!(state.collapsed.is_empty());
        state.apply(PlanAction::Raw(true));
        assert!(state.raw);
    }

    #[test]
    fn number_formats() {
        assert_eq!(fmt_count(950.0), "950");
        assert_eq!(fmt_count(1234.0), "1.2k");
        assert_eq!(fmt_count(12_345.0), "12k");
        assert_eq!(fmt_count(3_400_000.0), "3.4M");
        assert_eq!(fmt_count(2.5), "2.5");
        assert_eq!(fmt_ms(0.314), "0.31 ms");
        assert_eq!(fmt_ms(12.34), "12.3 ms");
        assert_eq!(fmt_ms(1250.0), "1.25 s");
        assert_eq!(fmt_cost(12.34), "12.3");
        assert_eq!(fmt_cost(4500.0), "4.5k");
    }

    #[test]
    fn figure_text() {
        let mut n = PlanNode::new("Seq Scan");
        assert_eq!(estimate_text(&n), None);
        assert_eq!(actual_text(&n), None);
        n.est_rows = Some(1200.0);
        n.est_cost = Some(12.3);
        n.actual_rows = Some(1000.0);
        n.actual_ms = Some(0.31);
        assert_eq!(estimate_text(&n).unwrap(), "est 1.2k rows · cost 12.3");
        assert_eq!(actual_text(&n).unwrap(), "actual 1.0k rows · 0.31 ms");
    }
}
