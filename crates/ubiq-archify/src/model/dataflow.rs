//! `dataflow.schema.json`. A stage is a column; its index is the `stage` of a node.

use serde::{Deserialize, Serialize};

use super::common::{
    BrandMark, Card, ComponentType, Id, NodeIcon, Point, SchemaV1, Side, SourceRef, Variant,
};

diagram_tag!(DataflowTag, Dataflow, "dataflow");

legend_entries!(DataflowLegendEntries {
    default = "default",
    emphasis = "emphasis",
    security = "security",
    dashed = "dashed",
    database = "database",
});

meta_struct!(DataflowMeta, DataflowLegendEntries;);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dataflow {
    pub schema_version: SchemaV1,
    pub diagram_type: DataflowTag,
    pub meta: DataflowMeta,
    pub stages: Vec<Stage>,
    pub nodes: Vec<Node>,
    pub flows: Vec<Flow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cards: Option<Vec<Card>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Node {
    pub id: Id,
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
    pub stage: f64,
    pub row: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_offset: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlowRoute {
    Auto,
    Straight,
    VerticalChannel,
    BottomChannel,
    TopChannel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Flow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub from: Id,
    pub to: Id,
    pub label: String,
    /// The second label line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<FlowRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_at: Option<Point>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_dx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_dy: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_segment: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<Vec<Point>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
}
