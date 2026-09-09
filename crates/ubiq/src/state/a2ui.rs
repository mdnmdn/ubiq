//! A2UI surfaces: the component tree an agent sends instead of prose, and the fixtures the sink
//! draws it from.
//!
//! **This is a renderer's half of a protocol Ubiq does not own.** A2UI is documented in
//! `_docs/references/a2ui-protocol.md` and its component set in `_docs/references/a2ui-catalog.md`;
//! what lives here is only enough of the `createSurface` envelope to draw one. The data model, the
//! JSON Pointer bindings behind it, the validation checks and every message that changes a surface
//! after it is created are the agent's side of the protocol and are not parsed — a bound property
//! keeps its pointer so the interface can show what a value *would* come from.
//!
//! **A surface is a flat adjacency list, not a tree.** Containers name their children by id and the
//! tree is implicit, which is what lets an agent stream one component at a time. So the parse
//! yields a map and a root id, and the shape is only discovered by walking it — a walk that has to
//! assume nothing about the graph, because a payload from an agent may name a child that never
//! arrives or point two components at each other.
//!
//! **Nothing here draws.** The map is data; `crate::ui::a2ui` is what turns it into elements.

use std::collections::HashMap;

use serde::Deserialize;

/// The maximum depth a walk of a surface descends before it stops.
///
/// A flat adjacency list can hold a cycle, and a renderer that recurses into one never returns.
/// This bounds the work at a depth no honest layout reaches, which costs one counter rather than
/// the visited set a general cycle check would need.
pub const MAX_DEPTH: u32 = 64;

/// One parsed surface: every component by id, and the id the walk starts from.
#[derive(Debug, Clone)]
pub struct Surface {
    pub root: String,
    pub by_id: HashMap<String, Component>,
}

impl Surface {
    /// The component with this id, or nothing when the payload names one it does not carry.
    pub fn get(&self, id: &str) -> Option<&Component> {
        self.by_id.get(id)
    }
}

/// One component: the id every other component refers to it by, and what it is.
#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    pub id: String,
    #[serde(flatten)]
    pub kind: Kind,
}

/// A property that is either written into the payload or bound to a path in the data model.
///
/// The data model is not parsed, so a binding is drawn as the pointer it names rather than as the
/// value behind it. `Other` is what keeps an unexpected form — a function call, a shape a later
/// version of the protocol adds — from failing the whole parse.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Prop<T> {
    Literal(T),
    Binding { path: String },
    Other(serde_json::Value),
}

impl<T> Prop<T> {
    /// The written value, when the property carries one.
    pub fn literal(&self) -> Option<&T> {
        match self {
            Prop::Literal(value) => Some(value),
            _ => None,
        }
    }

    /// The data-model pointer, when the property is bound to one.
    pub fn binding(&self) -> Option<&str> {
        match self {
            Prop::Binding { path } => Some(path.as_str()),
            _ => None,
        }
    }
}

impl Prop<String> {
    /// What to draw: the written string, or the pointer a bound property names.
    pub fn text(&self) -> &str {
        match self {
            Prop::Literal(value) => value.as_str(),
            Prop::Binding { path } => path.as_str(),
            Prop::Other(_) => "",
        }
    }
}

/// One option of a [`Kind::ChoicePicker`].
#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub value: String,
}

/// One tab of a [`Kind::Tabs`]: what its button says, and the component it shows.
#[derive(Debug, Clone, Deserialize)]
pub struct TabEntry {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub child: String,
}

/// How a container names its children: written out, or a template instantiated per array item.
///
/// The template form is the protocol's repeat: one component id drawn once for every element the
/// pointer finds. The data model is not parsed, so the template is reported rather than repeated.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Children {
    Explicit(Vec<String>),
    Template {
        path: String,
        #[serde(rename = "componentId")]
        component_id: String,
    },
    Other(serde_json::Value),
}

impl Children {
    /// The ids to draw. A template names none, because instantiating it needs the data model.
    pub fn ids(&self) -> &[String] {
        match self {
            Children::Explicit(ids) => ids,
            _ => &[],
        }
    }

    /// The pointer and the component a template repeats, when the children are one.
    pub fn template(&self) -> Option<(&str, &str)> {
        match self {
            Children::Template { path, component_id } => Some((path, component_id)),
            _ => None,
        }
    }
}

impl Default for Children {
    fn default() -> Self {
        Children::Explicit(Vec::new())
    }
}

/// The eighteen components of the basic catalog, and the fallback for anything else.
///
/// The tag is the payload's own `component` field, so the match a renderer writes is the catalog
/// itself and the compiler is what checks it covers every one. `Unknown` is what the protocol asks
/// a renderer to do with a component it does not implement: draw a placeholder rather than fail.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "component")]
pub enum Kind {
    Text {
        #[serde(default)]
        text: Option<Prop<String>>,
        #[serde(default)]
        variant: Option<String>,
    },
    Image {
        #[serde(default)]
        url: Option<Prop<String>>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        fit: Option<String>,
        #[serde(default)]
        variant: Option<String>,
    },
    Icon {
        #[serde(default)]
        name: Option<Prop<String>>,
    },
    Video {
        #[serde(default)]
        url: Option<Prop<String>>,
        #[serde(default, rename = "posterUrl")]
        poster_url: Option<String>,
    },
    AudioPlayer {
        #[serde(default)]
        url: Option<Prop<String>>,
        #[serde(default)]
        description: Option<String>,
    },
    Row {
        #[serde(default)]
        children: Children,
        #[serde(default)]
        justify: Option<String>,
        #[serde(default)]
        align: Option<String>,
        #[serde(default)]
        weight: Option<f32>,
    },
    Column {
        #[serde(default)]
        children: Children,
        #[serde(default)]
        justify: Option<String>,
        #[serde(default)]
        align: Option<String>,
        #[serde(default)]
        weight: Option<f32>,
    },
    List {
        #[serde(default)]
        children: Children,
        #[serde(default)]
        direction: Option<String>,
        #[serde(default)]
        align: Option<String>,
    },
    Card {
        #[serde(default)]
        child: Option<String>,
    },
    Tabs {
        #[serde(default)]
        tabs: Vec<TabEntry>,
    },
    Modal {
        #[serde(default)]
        trigger: Option<String>,
        #[serde(default)]
        content: Option<String>,
    },
    Divider {
        #[serde(default)]
        axis: Option<String>,
    },
    Button {
        #[serde(default)]
        child: Option<String>,
        #[serde(default)]
        variant: Option<String>,
    },
    TextField {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        value: Option<Prop<String>>,
        #[serde(default)]
        placeholder: Option<String>,
        #[serde(default)]
        variant: Option<String>,
    },
    CheckBox {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        value: Option<Prop<bool>>,
    },
    ChoicePicker {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        options: Vec<Choice>,
        #[serde(default)]
        value: Option<Prop<Vec<String>>>,
        #[serde(default)]
        variant: Option<String>,
        #[serde(default, rename = "displayStyle")]
        display_style: Option<String>,
    },
    Slider {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        value: Option<Prop<f32>>,
        #[serde(default)]
        min: Option<f32>,
        #[serde(default)]
        max: Option<f32>,
    },
    DateTimeInput {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        value: Option<Prop<String>>,
        #[serde(default, rename = "enableDate")]
        enable_date: bool,
        #[serde(default, rename = "enableTime")]
        enable_time: bool,
    },
    #[serde(other)]
    Unknown,
}

impl Kind {
    /// What the catalog calls this component, for a placeholder that has to name it.
    pub fn tag(&self) -> &'static str {
        match self {
            Kind::Text { .. } => "Text",
            Kind::Image { .. } => "Image",
            Kind::Icon { .. } => "Icon",
            Kind::Video { .. } => "Video",
            Kind::AudioPlayer { .. } => "AudioPlayer",
            Kind::Row { .. } => "Row",
            Kind::Column { .. } => "Column",
            Kind::List { .. } => "List",
            Kind::Card { .. } => "Card",
            Kind::Tabs { .. } => "Tabs",
            Kind::Modal { .. } => "Modal",
            Kind::Divider { .. } => "Divider",
            Kind::Button { .. } => "Button",
            Kind::TextField { .. } => "TextField",
            Kind::CheckBox { .. } => "CheckBox",
            Kind::ChoicePicker { .. } => "ChoicePicker",
            Kind::Slider { .. } => "Slider",
            Kind::DateTimeInput { .. } => "DateTimeInput",
            Kind::Unknown => "unknown",
        }
    }
}

/// The `createSurface` envelope, as much of it as drawing one needs.
#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "createSurface")]
    create_surface: CreateSurface,
}

#[derive(Deserialize)]
struct CreateSurface {
    #[serde(default)]
    components: Vec<Component>,
}

/// The id every surface starts its walk from, named by the protocol rather than chosen here.
const ROOT: &str = "root";

/// Read a `createSurface` payload into a surface, or say what is wrong with it.
///
/// The error is a sentence for a person reading it beside the payload, not a type to match on:
/// this parses what an agent sent, and every way it fails ends in the same place — a line above
/// the preview saying why nothing is drawn.
pub fn parse(source: &str) -> Result<Surface, String> {
    let envelope: Envelope = serde_json::from_str(source).map_err(|error| error.to_string())?;
    let components = envelope.create_surface.components;
    if components.is_empty() {
        return Err("`createSurface` carries no components".to_string());
    }
    let by_id: HashMap<String, Component> = components
        .into_iter()
        .map(|component| (component.id.clone(), component))
        .collect();
    if !by_id.contains_key(ROOT) {
        return Err(format!("no component has the id `{ROOT}`"));
    }
    Ok(Surface {
        root: ROOT.to_string(),
        by_id,
    })
}

/// One example payload: what the picker calls it, where it came from, and the JSON itself.
///
/// The provenance is on the page because it is load-bearing. Four of these are upstream
/// conformance fixtures and one is written here, and a reader deciding whether the renderer
/// handles real agent output needs to know which is which.
pub struct Example {
    pub name: &'static str,
    pub origin: &'static str,
    pub source: &'static str,
}

/// The examples the picker offers, in the order it lists them.
///
/// Between them they use all eighteen components of the basic catalog. The upstream four come from
/// `specification/v1_0/test/cases/` in `a2ui-project/a2ui`; the fifth is written here because no
/// payload anywhere upstream uses `Image`, `Video`, `AudioPlayer`, `List` or `Modal`.
pub const EXAMPLES: &[Example] = &[
    Example {
        name: "Contact form",
        origin: "adapted from the upstream contact_form_example trace",
        source: CONTACT_FORM,
    },
    Example {
        name: "Checkable inputs",
        origin: "adapted from the upstream checkable_components case",
        source: CHECKABLE_INPUTS,
    },
    Example {
        name: "Tabs, icons and buttons",
        origin: "adapted from the upstream tabs, icon and button cases",
        source: TABS_ICONS_BUTTONS,
    },
    Example {
        name: "Icons and text",
        origin: "adapted from the upstream icon and text_variants cases",
        source: ICONS_AND_TEXT,
    },
    Example {
        name: "Every component",
        origin: "written for this page — upstream has no payload using all eighteen",
        source: KITCHEN_SINK,
    },
];

const CONTACT_FORM: &str = include_str!("a2ui/contact-form.json");
const CHECKABLE_INPUTS: &str = include_str!("a2ui/checkable-inputs.json");
const TABS_ICONS_BUTTONS: &str = include_str!("a2ui/tabs-icons-buttons.json");
const ICONS_AND_TEXT: &str = include_str!("a2ui/icons-and-text.json");
const KITCHEN_SINK: &str = include_str!("a2ui/kitchen-sink.json");
