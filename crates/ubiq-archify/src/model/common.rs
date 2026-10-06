//! Shared definitions: `common.schema.json#/$defs` plus the macros that build each type's `meta`.

use std::collections::BTreeMap;

use serde::de::{Deserializer, Error as _, IgnoredAny};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// An `id` (`^[a-zA-Z][a-zA-Z0-9_-]*$`). The pattern is the schema's business, not the model's.
pub type Id = String;

/// `[x, y]` in viewBox units.
pub type Point = [f64; 2];

/// `componentType`. Also the node `type` of workflow, dataflow and sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentType {
    Frontend,
    Backend,
    Database,
    Cloud,
    Security,
    Messagebus,
    External,
}

/// `nodeIcon`: `None` hides the corner icon, an absent field takes the type default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeIcon {
    Calendar,
    Clock,
    Person,
    Briefcase,
    Flag,
    Moon,
    Frontend,
    Backend,
    Database,
    Cloud,
    Security,
    Messagebus,
    External,
    Start,
    Active,
    Waiting,
    Success,
    Failure,
    Neutral,
    None,
}

/// `variant`: the edge style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    Default,
    Emphasis,
    Security,
    Dashed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Animation {
    Trace,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VisualPreset {
    Classic,
    SignalFlow,
    Blueprint,
    Editorial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityProfile {
    Standard,
    Showcase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegendMode {
    Auto,
    All,
    Hidden,
}

/// `legendEntry` (`minProperties: 1` is the schema's).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegendEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
}

/// `meta.legend`; `E` is the per-type struct of entries (the allowed keys differ).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Legend<E> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<LegendMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<E>,
}

/// `brandMark`: a catalogue id or short text, or a digest-pinned image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrandMark {
    Text(String),
    Pinned(PinnedBrand),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedBrand {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoProvider {
    Github,
    Gitee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkMode {
    Web,
    LocalOnly,
}

/// `repository` (the commit the `sources` refer to).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<RepoProvider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_mode: Option<LinkMode>,
    pub revision: String,
}

/// One entry of `sourceReferences` (a node's `sources`, 1..3 of them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardDot {
    Cyan,
    Emerald,
    Violet,
    Amber,
    Rose,
    Orange,
    Slate,
}

/// One entry of the top-level `cards`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    pub dot: CardDot,
    pub title: String,
    pub items: Vec<String>,
}

/// `meta.translations`: a flat key to text map. Key order is not semantic, so it is sorted.
pub type Translations = BTreeMap<String, String>;

/// A field that is accepted and thrown away (the retired `meta.views`). Never serialised, and all
/// of its values are equal, so a document with views equals the same one without.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ignored;

impl<'de> Deserialize<'de> for Ignored {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        IgnoredAny::deserialize(d).map(|_| Ignored)
    }
}

/// `schema_version` of a type that has a v2 (workflow, lifecycle). The schema's `1` is any JSON
/// number equal to 1, so `1.0` is read as well; it is written as an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaVersion {
    V1,
    V2,
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match f64::deserialize(d)? {
            1.0 => Ok(Self::V1),
            2.0 => Ok(Self::V2),
            v => Err(D::Error::custom(format!(
                "schema_version {v} is not 1 or 2"
            ))),
        }
    }
}

impl Serialize for SchemaVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(match self {
            Self::V1 => 1,
            Self::V2 => 2,
        })
    }
}

/// `schema_version` of a type that only has v1 (architecture, sequence, dataflow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaV1;

impl<'de> Deserialize<'de> for SchemaV1 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match f64::deserialize(d)? {
            1.0 => Ok(Self),
            v => Err(D::Error::custom(format!("schema_version {v} is not 1"))),
        }
    }
}

impl Serialize for SchemaV1 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(1)
    }
}

/// A single-variant enum for the `diagram_type` const of one type.
macro_rules! diagram_tag {
    ($name:ident, $variant:ident, $lit:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
        pub enum $name {
            #[serde(rename = $lit)]
            $variant,
        }
    };
}

/// A per-type `meta.legend.entries` struct: each key is an optional [`LegendEntry`].
macro_rules! legend_entries {
    ($name:ident { $($field:ident = $key:literal),* $(,)? }) => {
        #[derive(Debug, Clone, Default, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $(
                #[serde(rename = $key, skip_serializing_if = "Option::is_none")]
                pub $field: Option<$crate::model::common::LegendEntry>,
            )*
        }
    };
}

/// A per-type `meta` struct: the fields every type shares, then the type's own after `;`.
macro_rules! meta_struct {
    (
        $name:ident, $entries:ty;
        $( $(#[$attr:meta])* $field:ident : $ty:ty ),* $(,)?
    ) => {
        #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub title: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub locale: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub translations: Option<$crate::model::common::Translations>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub subtitle: Option<String>,
            pub output: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub animation: Option<$crate::model::common::Animation>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub visual_preset: Option<$crate::model::common::VisualPreset>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub quality_profile: Option<$crate::model::common::QualityProfile>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub repository: Option<$crate::model::common::Repository>,
            /// Retired: read, dropped, never written back.
            #[serde(default, skip_serializing)]
            pub views: $crate::model::common::Ignored,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub legend: Option<$crate::model::common::Legend<$entries>>,
            #[serde(rename = "viewBox", skip_serializing_if = "Option::is_none")]
            pub view_box: Option<$crate::model::common::Point>,
            $( $(#[$attr])* pub $field: $ty, )*
        }

        impl $name {
            /// The catalogue this meta's `locale` and `translations` resolve to.
            pub fn locale(&self) -> $crate::i18n::Locale {
                $crate::i18n::Locale::of(self.locale.as_deref(), self.translations.as_ref())
            }
        }
    };
}
