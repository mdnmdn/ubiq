//! The portable query plan a driver's `explain` returns: a tree for display, and the engine's own
//! output as it came. The types only; the parsers that fill them are in `driver`.
//!
//! Units: costs are the engine's own (not comparable across engines), times are milliseconds,
//! actual rows and times are totals over every loop of the node.

use serde::{Deserialize, Serialize};

/// A query plan: a tree for display, and the engine's output as it came.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub root: PlanNode,
    /// The engine's own output (JSON, text, XML, or the tabular rows as tab-separated lines).
    pub raw: String,
    pub format: PlanFormat,
}

/// What [`Plan::raw`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanFormat {
    /// Postgres `FORMAT JSON`.
    Json,
    /// MySQL `FORMAT=TREE` / `EXPLAIN ANALYZE`.
    Text,
    /// SQL Server showplan XML.
    Xml,
    /// Rows — SQLite `EXPLAIN QUERY PLAN`, tabular MySQL `EXPLAIN` — one tab-separated line each.
    Table,
}

/// One operator of a plan. Every number is optional: an engine fills what it reports.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanNode {
    /// The operator: `Seq Scan`, `Hash Join`, `Table scan on t1`, `SCAN person`,
    /// `Clustered Index Seek`.
    pub label: String,
    /// What it works on and how: relation, index, join type, conditions.
    pub detail: Option<String>,
    pub est_rows: Option<f64>,
    /// Estimated total cost, in the engine's unit.
    pub est_cost: Option<f64>,
    /// Rows produced, over every loop (`analyze` only).
    pub actual_rows: Option<f64>,
    /// Time spent, milliseconds, over every loop (`analyze` only).
    pub actual_ms: Option<f64>,
    pub children: Vec<PlanNode>,
}

impl PlanNode {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    /// This node and every descendant.
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(PlanNode::count).sum::<usize>()
    }
}
