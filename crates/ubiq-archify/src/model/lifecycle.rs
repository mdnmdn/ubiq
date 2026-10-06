//! `lifecycle.schema.json`. Version 1 has fixed bands and an implied main rail; version 2 authors
//! every transition.

use serde::{Deserialize, Serialize};

use super::common::{
    BrandMark, Card, Id, NodeIcon, Point, SchemaVersion, Side, SourceRef, Variant,
};

diagram_tag!(LifecycleTag, Lifecycle, "lifecycle");

legend_entries!(LifecycleLegendEntries {
    start = "start",
    active = "active",
    waiting = "waiting",
    decision = "decision",
    success = "success",
    failure = "failure",
    neutral = "neutral",
    external = "external",
});

meta_struct!(LifecycleMeta, LifecycleLegendEntries;);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lifecycle {
    pub schema_version: SchemaVersion,
    pub diagram_type: LifecycleTag,
    pub meta: LifecycleMeta,
    pub lanes: Vec<Lane>,
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cards: Option<Vec<Card>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub id: Id,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StateType {
    Start,
    Active,
    Waiting,
    Decision,
    Success,
    Failure,
    Neutral,
    External,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct State {
    pub id: Id,
    #[serde(rename = "type")]
    pub kind: StateType,
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
    /// A display tag, not an ordering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    pub lane: Id,
    pub col: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_offset: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransitionRoute {
    Auto,
    Straight,
    Drop,
    BottomChannel,
    TopChannel,
    RightChannel,
    LeftChannel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Transition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub from: Id,
    pub to: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<TransitionRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
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
