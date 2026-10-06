//! `sequence.schema.json`. Participant order is the column order; a message's vertical place is its `y`.

use serde::{Deserialize, Serialize};

use super::common::{BrandMark, Card, ComponentType, Id, NodeIcon, SchemaV1, SourceRef};

diagram_tag!(SequenceTag, Sequence, "sequence");

legend_entries!(SequenceLegendEntries {
    default = "default",
    emphasis = "emphasis",
    security = "security",
    dashed = "dashed",
    return_ = "return",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnFit {
    Fixed,
    Spread,
}

meta_struct!(SequenceMeta, SequenceLegendEntries;
    #[serde(skip_serializing_if = "Option::is_none")]
    column_fit: Option<ColumnFit>,
);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
    pub schema_version: SchemaV1,
    pub diagram_type: SequenceTag,
    pub meta: SequenceMeta,
    pub participants: Vec<Participant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<Segment>>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activations: Option<Vec<Activation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cards: Option<Vec<Card>>,
}

/// No `tag`, unlike the other node kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub id: Id,
    #[serde(rename = "type")]
    pub kind: ComponentType,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sublabel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<NodeIcon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<BrandMark>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceRef>>,
}

/// A labelled vertical span `from..to` (y values).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub from: f64,
    pub to: f64,
    pub label: String,
}

/// The message variants are the edge ones plus `return`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageVariant {
    Default,
    Emphasis,
    Security,
    Dashed,
    Return,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub from: Id,
    pub to: Id,
    pub y: f64,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<MessageVariant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A bar on a participant's lifeline over the y span `from..to`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    pub participant: Id,
    pub from: f64,
    pub to: f64,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<ComponentType>,
}
