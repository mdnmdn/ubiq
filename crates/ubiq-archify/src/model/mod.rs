//! Typed documents for the five diagram types (P1.3).
//!
//! The model assumes the schema stage has passed: it is what the later stages read, not a
//! validator. It still never panics, and a document the schema would have rejected comes back as
//! an `Err` string. Rules that hold throughout:
//!
//! - `deny_unknown_fields` at every object level, as the schemas' `additionalProperties: false`.
//! - Every number is an `f64`, integer-typed fields included (the schema accepts `1.0` as an
//!   integer, and serde's integer types do not). Serialising writes `1.0` where Archify wrote `1`;
//!   compare as numbers, not as text.
//! - Arrays are `Vec`s in document order; an optional collection is `Option<Vec<_>>`, so an absent
//!   field and an empty array stay distinct.
//! - `meta.views` is retired: read, dropped, never written back.

#[macro_use]
pub mod common;
pub mod architecture;
pub mod dataflow;
pub mod lifecycle;
pub mod sequence;
pub mod workflow;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub use architecture::Architecture;
pub use dataflow::Dataflow;
pub use lifecycle::Lifecycle;
pub use sequence::Sequence;
pub use workflow::Workflow;

/// A document of any of the five types.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Doc {
    Architecture(Box<Architecture>),
    Workflow(Box<Workflow>),
    Sequence(Box<Sequence>),
    Dataflow(Box<Dataflow>),
    Lifecycle(Box<Lifecycle>),
}

impl Doc {
    /// The `diagram_type` const of this document.
    pub fn diagram_type(&self) -> &'static str {
        match self {
            Self::Architecture(_) => "architecture",
            Self::Workflow(_) => "workflow",
            Self::Sequence(_) => "sequence",
            Self::Dataflow(_) => "dataflow",
            Self::Lifecycle(_) => "lifecycle",
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Architecture(d) => &d.meta.title,
            Self::Workflow(d) => &d.meta.title,
            Self::Sequence(d) => &d.meta.title,
            Self::Dataflow(d) => &d.meta.title,
            Self::Lifecycle(d) => &d.meta.title,
        }
    }

    /// The catalogue `meta.locale` and `meta.translations` resolve to (`i18n::Locale::of`).
    pub fn locale(&self) -> crate::i18n::Locale {
        match self {
            Self::Architecture(d) => d.meta.locale(),
            Self::Workflow(d) => d.meta.locale(),
            Self::Sequence(d) => d.meta.locale(),
            Self::Dataflow(d) => d.meta.locale(),
            Self::Lifecycle(d) => d.meta.locale(),
        }
    }

    /// The authored `meta.visual_preset`, if any.
    pub fn visual_preset(&self) -> Option<common::VisualPreset> {
        match self {
            Self::Architecture(d) => d.meta.visual_preset,
            Self::Workflow(d) => d.meta.visual_preset,
            Self::Sequence(d) => d.meta.visual_preset,
            Self::Dataflow(d) => d.meta.visual_preset,
            Self::Lifecycle(d) => d.meta.visual_preset,
        }
    }

    /// Reads `value` as `value.diagram_type`. An absent or unknown type is an `Err`.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        match value.get("diagram_type").and_then(Value::as_str) {
            Some(t) => parse(t, value),
            None => Err("missing diagram_type".to_owned()),
        }
    }
}

impl<'de> Deserialize<'de> for Doc {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

/// Deserialises `value` as the document type `doc_type` (`architecture`, `workflow`, `sequence`,
/// `dataflow` or `lifecycle`). The type comes from the caller, never from the file name; a
/// `diagram_type` that disagrees is an `Err`.
pub fn parse(doc_type: &str, value: &Value) -> Result<Doc, String> {
    fn de<'a, T: Deserialize<'a>>(value: &'a Value) -> Result<T, String> {
        T::deserialize(value).map_err(|e| e.to_string())
    }
    Ok(match doc_type {
        "architecture" => Doc::Architecture(Box::new(de(value)?)),
        "workflow" => Doc::Workflow(Box::new(de(value)?)),
        "sequence" => Doc::Sequence(Box::new(de(value)?)),
        "dataflow" => Doc::Dataflow(Box::new(de(value)?)),
        "lifecycle" => Doc::Lifecycle(Box::new(de(value)?)),
        other => return Err(format!("unknown diagram type {other:?}")),
    })
}
