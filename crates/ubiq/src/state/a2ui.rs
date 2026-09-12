//! A2UI surfaces: the component tree an agent sends instead of prose, the data model behind it, and
//! the fixtures the sink draws them from.
//!
//! **This is a renderer's half of a protocol Ubiq does not own.** A2UI is documented in
//! `_docs/references/a2ui-protocol.md` and its component set in `_docs/references/a2ui-catalog.md`.
//! What lives here is the `createSurface` envelope — the components, the data model they bind
//! against, and the actions a click sends back — and nothing that changes a surface after it is
//! created: `updateComponents`, `updateDataModel` and `deleteSurface` are the agent's side of a
//! conversation no producer is having yet.
//!
//! **A surface is a flat adjacency list, not a tree.** Containers name their children by id and the
//! tree is implicit, which is what lets an agent stream one component at a time. So the parse
//! yields a map and a root id, and the shape is only discovered by walking it — a walk that has to
//! assume nothing about the graph, because a payload from an agent may name a child that never
//! arrives or point two components at each other.
//!
//! **The catalog is a registry, not this enum.** [`Kind`] carries the eighteen components of the
//! basic catalog, and everything else parses into [`Kind::Extension`] with its properties intact,
//! for `crate::ui::a2ui::registry` to draw if Ubiq's own catalog claims the name. A component
//! nobody implements draws a placeholder, which is what the protocol asks for.
//!
//! **Nothing here draws.** The map is data; `crate::ui::a2ui` is what turns it into elements. The
//! one module below that names gpui is [`live`], because a text field's buffer *is* the value at
//! the pointer it binds to, with no second copy.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Map, Value};

pub mod action;
pub mod eval;
pub mod live;
pub mod path;
pub mod svg;
pub mod value;

pub use action::{Action, Event};
pub use eval::{Call, Dynamic, Scope};

/// The maximum depth a walk of a surface descends before it stops.
///
/// A flat adjacency list can hold a cycle, and a renderer that recurses into one never returns.
/// This bounds the work at a depth no honest layout reaches, which costs one counter rather than
/// the visited set a general cycle check would need.
pub const MAX_DEPTH: u32 = 64;

/// The id every surface starts its walk from, named by the protocol rather than chosen here.
const ROOT: &str = "root";

/// Everything one `createSurface` envelope carried.
///
/// The surface is the components; the rest is what an action has to quote back. `send_data_model`
/// is the agent's request to be told the whole model with every message, and is the only reason
/// the renderer's copy ever leaves the window.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub surface: Surface,
    pub surface_id: String,
    pub model: Value,
    pub send_data_model: bool,
}

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

/// One component: the id every other component refers to it by, what it is, and the two properties
/// every component in the catalog may carry whatever its type.
///
/// `weight` is the catalog's flex-grow, settable on any direct child of a `Row` or a `Column` — so
/// it belongs here rather than on the container variants, which is where it used to sit and where
/// it could only ever describe the container itself.
#[derive(Debug, Clone)]
pub struct Component {
    pub id: String,
    pub kind: Kind,
    pub weight: Option<f32>,
    pub accessibility: Option<Accessibility>,
}

/// What a component says about itself to something that is not looking at it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Accessibility {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub hidden: bool,
}

/// One option of a [`Kind::ChoicePicker`].
#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub label: Option<Dynamic>,
    #[serde(default)]
    pub value: Option<Dynamic>,
}

/// One tab of a [`Kind::Tabs`]: what its button says, and the component it shows.
#[derive(Debug, Clone, Deserialize)]
pub struct TabEntry {
    #[serde(default)]
    pub title: Option<Dynamic>,
    #[serde(default)]
    pub child: String,
}

/// One validation rule on an input: a condition to evaluate, and what to say when it fails.
///
/// The condition evaluates to a validation result — `{"valid": …, "message": …}` — or to a bare
/// boolean, which agents author constantly and which [`eval::validation`] reads the same way.
#[derive(Debug, Clone, Deserialize)]
pub struct CheckRule {
    pub condition: Dynamic,
    #[serde(default)]
    pub message: Option<String>,
}

/// How a container names its children: written out, or a template instantiated per array item.
///
/// The template form is the protocol's repeat: one component id drawn once for every element the
/// pointer finds, with that element as the binding scope. Expanding it needs the data model, which
/// is why it is the renderer that walks it rather than the parse.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Children {
    Explicit(Vec<String>),
    Template {
        path: String,
        #[serde(rename = "componentId")]
        component_id: String,
    },
    Other(Value),
}

impl Children {
    /// The ids written out, when the children are a static list.
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

/// The eighteen components of the basic catalog, and the open arm everything else parses into.
///
/// The tag is the payload's own `component` field, so the match a renderer writes is the catalog
/// itself and the compiler is what checks it covers every one. [`Kind::Extension`] is skipped by
/// serde and produced by [`parse`] instead: a name outside [`BASIC`] keeps its properties and is
/// offered to the extension registry rather than thrown away.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "component")]
pub enum Kind {
    Text {
        #[serde(default)]
        text: Option<Dynamic>,
        #[serde(default)]
        variant: Option<String>,
    },
    Image {
        #[serde(default)]
        url: Option<Dynamic>,
        #[serde(default)]
        description: Option<Dynamic>,
        #[serde(default)]
        fit: Option<String>,
        #[serde(default)]
        variant: Option<String>,
    },
    /// `name` is a catalog name, or an object carrying `svgPath`, or a binding to either — so it
    /// stays a [`Dynamic`] and the renderer looks at what it evaluated to.
    Icon {
        #[serde(default)]
        name: Option<Dynamic>,
    },
    Video {
        #[serde(default)]
        url: Option<Dynamic>,
        #[serde(default, rename = "posterUrl")]
        poster_url: Option<Dynamic>,
    },
    AudioPlayer {
        #[serde(default)]
        url: Option<Dynamic>,
        #[serde(default)]
        description: Option<Dynamic>,
    },
    Row {
        #[serde(default)]
        children: Children,
        #[serde(default)]
        justify: Option<String>,
        #[serde(default)]
        align: Option<String>,
    },
    Column {
        #[serde(default)]
        children: Children,
        #[serde(default)]
        justify: Option<String>,
        #[serde(default)]
        align: Option<String>,
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
        #[serde(default)]
        action: Option<Action>,
        #[serde(default)]
        checks: Vec<CheckRule>,
    },
    TextField {
        #[serde(default)]
        label: Option<Dynamic>,
        #[serde(default)]
        value: Option<Dynamic>,
        #[serde(default)]
        placeholder: Option<Dynamic>,
        #[serde(default)]
        variant: Option<String>,
        #[serde(default)]
        checks: Vec<CheckRule>,
    },
    CheckBox {
        #[serde(default)]
        label: Option<Dynamic>,
        #[serde(default)]
        value: Option<Dynamic>,
        #[serde(default)]
        checks: Vec<CheckRule>,
    },
    ChoicePicker {
        #[serde(default)]
        label: Option<Dynamic>,
        #[serde(default)]
        options: Vec<Choice>,
        #[serde(default)]
        value: Option<Dynamic>,
        #[serde(default)]
        variant: Option<String>,
        #[serde(default, rename = "displayStyle")]
        display_style: Option<String>,
        #[serde(default)]
        filterable: bool,
        #[serde(default)]
        checks: Vec<CheckRule>,
    },
    Slider {
        #[serde(default)]
        label: Option<Dynamic>,
        #[serde(default)]
        value: Option<Dynamic>,
        #[serde(default)]
        min: Option<f32>,
        #[serde(default)]
        max: Option<f32>,
        #[serde(default)]
        steps: Option<u32>,
        #[serde(default)]
        checks: Vec<CheckRule>,
    },
    DateTimeInput {
        #[serde(default)]
        label: Option<Dynamic>,
        #[serde(default)]
        value: Option<Dynamic>,
        #[serde(default, rename = "enableDate")]
        enable_date: bool,
        #[serde(default, rename = "enableTime")]
        enable_time: bool,
        #[serde(default)]
        checks: Vec<CheckRule>,
    },
    /// A component from a catalog other than the basic one, with its properties kept whole.
    ///
    /// Skipped by serde and built by [`parse`]: an internally-tagged enum would otherwise mint a
    /// component type named `Extension`, which no catalog defines.
    #[serde(skip)]
    Extension {
        component: String,
        props: Map<String, Value>,
    },
}

/// The basic catalog, by name. [`parse`] and `Kind::tag` both read this, and so does the test that
/// holds the examples to it.
pub const BASIC: [&str; 18] = [
    "Text",
    "Image",
    "Icon",
    "Video",
    "AudioPlayer",
    "Row",
    "Column",
    "List",
    "Card",
    "Tabs",
    "Modal",
    "Divider",
    "Button",
    "TextField",
    "CheckBox",
    "ChoicePicker",
    "Slider",
    "DateTimeInput",
];

impl Kind {
    /// What the catalog calls this component — the basic name, or an extension's own.
    pub fn tag(&self) -> &str {
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
            Kind::Extension { component, .. } => component,
        }
    }

    /// What this component does when clicked, which only a `Button` in the basic catalog does.
    pub fn button_action(&self) -> Option<&Action> {
        match self {
            Kind::Button { action, .. } => action.as_ref(),
            _ => None,
        }
    }

    /// The validation rules on this component, which are only ever on an input or a button.
    pub fn checks(&self) -> &[CheckRule] {
        match self {
            Kind::Button { checks, .. }
            | Kind::TextField { checks, .. }
            | Kind::CheckBox { checks, .. }
            | Kind::ChoicePicker { checks, .. }
            | Kind::Slider { checks, .. }
            | Kind::DateTimeInput { checks, .. } => checks,
            _ => &[],
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
    #[serde(default, rename = "surfaceId")]
    surface_id: String,
    /// Read as raw values, because whether a component belongs to the basic catalog is a question
    /// about its `component` field that serde cannot ask on this enum's behalf.
    #[serde(default)]
    components: Vec<Value>,
    #[serde(default, rename = "dataModel")]
    data_model: Value,
    #[serde(default, rename = "sendDataModel")]
    send_data_model: bool,
}

/// Read a `createSurface` payload into a surface and its data model, or say what is wrong with it.
///
/// The error is a sentence for a person reading it beside the payload, not a type to match on:
/// this parses what an agent sent, and every way it fails ends in the same place — a line above
/// the preview saying why nothing is drawn.
pub fn parse(source: &str) -> Result<Parsed, String> {
    let envelope: Envelope = serde_json::from_str(source).map_err(|error| error.to_string())?;
    let create = envelope.create_surface;
    if create.components.is_empty() {
        return Err("`createSurface` carries no components".to_string());
    }

    let mut by_id = HashMap::with_capacity(create.components.len());
    for raw in create.components {
        let component = component(raw)?;
        by_id.insert(component.id.clone(), component);
    }
    if !by_id.contains_key(ROOT) {
        return Err(format!("no component has the id `{ROOT}`"));
    }

    Ok(Parsed {
        surface: Surface {
            root: ROOT.to_string(),
            by_id,
        },
        surface_id: create.surface_id,
        model: match create.data_model {
            Value::Null => Value::Object(Map::new()),
            model => model,
        },
        send_data_model: create.send_data_model,
    })
}

/// One component, split into the properties every component has and the ones its type has.
///
/// The split by name is here rather than in a serde attribute because it is a question about the
/// payload rather than about the type: a name in [`BASIC`] is the enum's, and any other name keeps
/// its properties whole for the extension registry.
fn component(raw: Value) -> Result<Component, String> {
    let Value::Object(mut props) = raw else {
        return Err("a component is not an object".to_string());
    };

    let id = props
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "a component carries no `id`".to_string())?
        .to_string();
    let name = props
        .get("component")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("component `{id}` names no type"))?
        .to_string();

    let weight = props
        .get("weight")
        .and_then(Value::as_f64)
        .map(|w| w as f32);
    let accessibility = props
        .remove("accessibility")
        .and_then(|value| serde_json::from_value(value).ok());

    let kind = if BASIC.contains(&name.as_str()) {
        serde_json::from_value(Value::Object(props))
            .map_err(|error| format!("component `{id}` ({name}): {error}"))?
    } else {
        props.remove("id");
        props.remove("component");
        props.remove("weight");
        Kind::Extension {
            component: name,
            props,
        }
    };

    Ok(Component {
        id,
        kind,
        weight,
        accessibility,
    })
}

/// The key a component is known by while a surface is drawn: its id, and the template item it is
/// being drawn for.
///
/// One template component becomes many drawn components, and everything keyed by component alone —
/// a field's buffer, a failing check, an element id — would collide between the repeats. The
/// prefix is empty outside a template, so an ordinary component's key is `id@`.
pub fn scoped(id: &str, scope: Scope<'_>) -> String {
    format!("{id}@{}", scope.key())
}

/// Visit every component the renderer would draw, in the scope it would draw it in.
///
/// **The walk is shared because it has to agree with itself.** The renderer, the enumeration of
/// which inputs need a buffer and the evaluation of which checks block an action all have to visit
/// the same set — a field that is drawn but has no buffer, or a check that blocks but belongs to a
/// template row nobody drew, is the class of bug this exists to prevent. It expands templates
/// against `model` exactly as the renderer does, and it is bounded by [`MAX_DEPTH`] for the same
/// reason: a payload may point two components at each other.
pub fn walk(parsed: &Parsed, visit: &mut dyn FnMut(&Component, Scope<'_>)) {
    descend(parsed, &parsed.surface.root, 0, Scope::default(), visit);
}

fn descend(
    parsed: &Parsed,
    id: &str,
    depth: u32,
    scope: Scope<'_>,
    visit: &mut dyn FnMut(&Component, Scope<'_>),
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(component) = parsed.surface.get(id) else {
        return;
    };
    if component
        .accessibility
        .as_ref()
        .is_some_and(|access| access.hidden)
    {
        return;
    }
    visit(component, scope);

    let mut children = |children: &Children| match children.template() {
        Some((path, template)) => {
            let base = scope.resolve(path);
            let count = value::get(&parsed.model, &base)
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            for index in 0..count {
                let item = format!("{base}/{index}");
                descend(
                    parsed,
                    template,
                    depth + 1,
                    Scope {
                        item: Some(&item),
                        index: Some(index),
                    },
                    visit,
                );
            }
        }
        None => {
            for child in children.ids() {
                descend(parsed, child, depth + 1, scope, visit);
            }
        }
    };

    match &component.kind {
        Kind::Row { children: kids, .. }
        | Kind::Column { children: kids, .. }
        | Kind::List { children: kids, .. } => children(kids),
        Kind::Card { child } | Kind::Button { child, .. } => {
            if let Some(child) = child {
                descend(parsed, child, depth + 1, scope, visit);
            }
        }
        Kind::Tabs { tabs } => {
            for tab in tabs {
                descend(parsed, &tab.child, depth + 1, scope, visit);
            }
        }
        Kind::Modal { trigger, content } => {
            for child in [trigger, content].into_iter().flatten() {
                descend(parsed, child, depth + 1, scope, visit);
            }
        }
        _ => {}
    }
}

/// One example payload: what the picker calls it, where it came from, and the JSON itself.
///
/// The provenance is on the page because it is load-bearing. Some of these are upstream
/// conformance fixtures and some are written here, and a reader deciding whether the renderer
/// handles real agent output needs to know which is which.
pub struct Example {
    pub name: &'static str,
    pub origin: &'static str,
    pub source: &'static str,
}

/// The examples the picker offers, in the order it lists them.
///
/// Between them they use all eighteen components of the basic catalog and every component Ubiq's
/// own catalog adds. The upstream ones come from `specification/v1_0/test/cases/` in
/// `a2ui-project/a2ui`; the rest are written here, because no payload anywhere upstream exercises
/// `Image`, `Video`, `AudioPlayer`, `List` or `Modal`, and none of them could exercise a catalog
/// that did not exist until Ubiq wrote it.
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
        name: "Bindings and actions",
        origin: "written for this page — the data model, templates, functions and both action shapes",
        source: BINDINGS,
    },
    Example {
        name: "Ubiq catalog",
        origin: "written for this page — the first payload to name Ubiq's own catalog",
        source: UBIQ_CATALOG_DEMO,
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
const BINDINGS: &str = include_str!("a2ui/bindings.json");
const UBIQ_CATALOG_DEMO: &str = include_str!("a2ui/ubiq-catalog-demo.json");
const KITCHEN_SINK: &str = include_str!("a2ui/kitchen-sink.json");

/// Ubiq's own A2UI catalog: the eighteen by reference, plus what this renderer adds.
///
/// A catalog is the protocol's extension point and the thing an agent is handed — it is what makes
/// "the agent may use exactly the components that exist" true rather than aspirational. It is
/// embedded rather than read off disk because the day there is a producer, this is what an MCP
/// resource hands over. `crates/ubiq/tests/a2ui.rs` holds it and the drawing registry to each
/// other.
pub const UBIQ_CATALOG: &str = include_str!("a2ui/ubiq-catalog.json");
