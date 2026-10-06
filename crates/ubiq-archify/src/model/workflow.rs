//! `workflow.schema.json`. Version 2 is a solver and ignores array order; the model keeps it anyway.

use serde::{Deserialize, Serialize};

use super::common::{
    BrandMark, Card, ComponentType, Id, NodeIcon, Point, SchemaVersion, Side, SourceRef, Variant,
};

diagram_tag!(WorkflowTag, Workflow, "workflow");

legend_entries!(WorkflowLegendEntries {
    frontend = "frontend",
    backend = "backend",
    database = "database",
    cloud = "cloud",
    security = "security",
    messagebus = "messagebus",
    external = "external",
});

meta_struct!(WorkflowMeta, WorkflowLegendEntries;);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Workflow {
    #[serde(rename = "schema_version")]
    pub schema_version: SchemaVersion,
    #[serde(rename = "diagram_type")]
    pub diagram_type: WorkflowTag,
    pub meta: WorkflowMeta,
    pub lanes: Vec<Lane>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phases: Option<Vec<Phase>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<Group>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_path: Option<Vec<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_checks: Option<SemanticChecks>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cards: Option<Vec<Card>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaneVariant {
    Normal,
    Exception,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub id: Id,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<LaneVariant>,
}

/// A header band over columns `from_col..=to_col`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Phase {
    pub id: Id,
    pub label: String,
    pub from_col: f64,
    pub to_col: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
}

/// A frame inside one lane over columns `from_col..=to_col`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Group {
    pub id: Id,
    pub label: String,
    pub lane: Id,
    pub from_col: f64,
    pub to_col: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SemanticChecks {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_roots: Option<Vec<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_terminals: Option<Vec<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_edges: Option<Vec<Relation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_paths: Option<Vec<Relation>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub from: Id,
    pub to: Id,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Node {
    pub id: Id,
    pub lane: Id,
    pub col: f64,
    #[serde(rename = "type")]
    pub kind: ComponentType,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sublabel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<NodeIcon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<BrandMark>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_offset: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Main,
    Branch,
    Async,
    Return,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeRoute {
    Auto,
    Straight,
    Drop,
    OutsideRight,
    ReturnLeft,
    BottomChannel,
    UpChannel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Edge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub from: Id,
    pub to: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<EdgeRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<Vec<Point>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_at: Option<Point>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_dx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_dy: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_segment: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_y: Option<f64>,
    /// 0..=1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bias: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
}
