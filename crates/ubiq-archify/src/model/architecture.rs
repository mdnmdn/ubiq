//! `architecture.schema.json`.

use serde::{Deserialize, Serialize};

use super::common::{
    BrandMark, Card, ComponentType, Id, NodeIcon, Point, SchemaV1, Side, SourceRef, Variant,
};

diagram_tag!(ArchitectureTag, Architecture, "architecture");

legend_entries!(ArchitectureLegendEntries {
    frontend = "frontend",
    backend = "backend",
    database = "database",
    cloud = "cloud",
    security = "security",
    messagebus = "messagebus",
    external = "external",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineeringProfile {
    #[serde(rename = "deployment-ownership")]
    DeploymentOwnership,
}

meta_struct!(ArchitectureMeta, ArchitectureLegendEntries;
    #[serde(skip_serializing_if = "Option::is_none")]
    engineering_profile: Option<EngineeringProfile>,
);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Architecture {
    pub schema_version: SchemaV1,
    pub diagram_type: ArchitectureTag,
    pub meta: ArchitectureMeta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    pub components: Vec<Component>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boundaries: Option<Vec<Boundary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections: Option<Vec<Connection>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cards: Option<Vec<Card>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutMode {
    Grid,
}

/// The grid defaults; every field falls back to them when absent (00 section 2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Layout {
    pub mode: LayoutMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Point>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_h: Option<f64>,
}

/// Placed by `pos`, or by `row` and `col` under a `layout`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos: Option<Point>,
    /// `[width, height]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Point>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BoundaryKind {
    Region,
    SecurityGroup,
}

/// Has no id: its identity is `kind` plus `label`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    pub kind: BoundaryKind,
    pub label: String,
    pub wraps: Vec<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionRoute {
    Auto,
    Straight,
    OrthogonalH,
    OrthogonalV,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Connection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub from: Id,
    pub to: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<ConnectionRoute>,
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
    pub width: Option<f64>,
}
