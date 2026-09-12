//! A surface that is being used rather than only looked at.
//!
//! **This is the one module under `state/a2ui/` that names gpui**, and the reason is the rule the
//! UI skill states for when a component-library type may live in `state/`: the widget's state has
//! to *be* the model. Here it literally is — the text in a field is the value at the pointer the
//! field binds to, with no second copy anywhere. A `String` beside the `Entity` would be two
//! answers to one question.
//!
//! **A field's subscription lives in the map with it.** A gpui `Subscription` unsubscribes when it
//! drops, so clearing [`Live::fields`] is the whole teardown of an old surface — there is no list
//! to remember to prune, and a field from a payload that has been edited away cannot still be
//! writing into the model.
//!
//! **Only text-bearing inputs get an entity.** A checkbox, a choice and a slider are clicks: they
//! write through the renderer's callback and hold nothing, so a surface with no text field
//! allocates nothing at all.

use std::collections::HashMap;

use gpui::{Entity, Subscription};
use gpui_component::input::InputState;
use serde_json::Value;

use super::{Kind, Parsed, Scope, Surface, scoped, value};

/// How many fired actions the page keeps. A log is a record of the last few clicks, not a history.
pub const LOG_MAX: usize = 50;

/// One text input in a drawn surface: its buffer, where it writes, and the subscription that does
/// the writing.
pub struct Field {
    pub input: Entity<InputState>,
    /// The **absolute** pointer this field writes to, already resolved through its template scope.
    pub path: String,
    _sub: Subscription,
}

impl Field {
    pub fn new(input: Entity<InputState>, path: String, subscription: Subscription) -> Self {
        Self {
            input,
            path,
            _sub: subscription,
        }
    }
}

/// One input the renderer will draw that needs a buffer behind it.
///
/// Produced without a window, which is what lets the template arithmetic — the part that turns a
/// relative `qty` into `/rows/2/qty` — be tested without one.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    /// `componentId@itemPrefix`, unique across the repeats of one template component.
    pub key: String,
    pub component: String,
    pub path: String,
    pub seed: String,
}

/// Every input in the surface that needs a buffer, in the order the renderer would reach them.
///
/// A field inside a tab that is not forward still gets one: which tab is showing is navigation,
/// and what has been typed into the others must survive a reader looking away.
pub fn input_slots(parsed: &Parsed) -> Vec<Slot> {
    let mut slots = Vec::new();
    super::walk(parsed, &mut |component, scope| {
        let value = match &component.kind {
            Kind::TextField { value, .. } | Kind::DateTimeInput { value, .. } => value.as_ref(),
            _ => None,
        };
        // A literal value is a field with nowhere to write, and the renderer draws it as the
        // unwritable thing it is rather than pretending a keystroke went somewhere.
        let Some(path) = value.and_then(|value| value.write_target(scope)) else {
            return;
        };
        let seed = value
            .map(|value| value.as_display_string(&parsed.model, scope))
            .unwrap_or_default();
        slots.push(Slot {
            key: scoped(&component.id, scope),
            component: component.id.clone(),
            path,
            seed,
        });
    });
    slots
}

/// The surface the page is showing, its data model, where the reader has navigated to, and what
/// has been sent.
///
/// `surface` is the last payload that **parsed**, which is not always the payload in the editor: a
/// half-typed one sets `error` and leaves everything else alone, because dropping the surface on
/// every intermediate keystroke would drop every field buffer with it and throw away what the
/// reader had typed into the drawing.
#[derive(Default)]
pub struct Live {
    pub surface: Option<Surface>,
    pub error: Option<String>,
    pub surface_id: String,
    pub send_data_model: bool,
    pub model: Value,
    /// The tab forward in each `Tabs`, by component id. Absent means the first.
    pub tabs: HashMap<String, usize>,
    /// The `Modal` whose content is disclosed, if any.
    pub modal: Option<String>,
    /// One entry per text input, by [`Slot::key`].
    pub fields: HashMap<String, Field>,
    /// What blocked the last action, by the scoped key of the input that refused it.
    pub invalid: HashMap<String, String>,
    /// Fired envelopes and local calls, newest first.
    pub log: Vec<String>,
}

impl Live {
    /// Write one value into the data model. Every keystroke and every click ends here.
    pub fn set(&mut self, pointer: &str, value: Value) {
        super::value::set(&mut self.model, pointer, value);
    }

    /// What is at a pointer now — what a control reads to draw itself.
    pub fn get(&self, pointer: &str) -> Option<&Value> {
        value::get(&self.model, pointer)
    }

    /// Record something that left the surface, and forget the oldest when the log is full.
    pub fn record(&mut self, entry: String) {
        self.log.insert(0, entry);
        self.log.truncate(LOG_MAX);
    }

    /// The surface and its model together, for the pure functions that want a whole payload.
    ///
    /// A clone of the map, which is what keeps [`super::walk`] and [`super::action`] free of any
    /// notion of a live surface — they take what a parse produced, and this is how a live one says
    /// what it currently is.
    pub fn parsed(&self) -> Option<Parsed> {
        Some(Parsed {
            surface: self.surface.clone()?,
            surface_id: self.surface_id.clone(),
            model: self.model.clone(),
            send_data_model: self.send_data_model,
        })
    }

    /// The scope a component is drawn in, given the key it was drawn under.
    ///
    /// The key carries the template item after its `@`, which is the whole of a scope bar the
    /// index — and the index is recovered from the item's last segment, because a template item is
    /// always an array element.
    pub fn scope_of(key: &str) -> (String, Option<String>, Option<usize>) {
        let (component, item) = key.split_once('@').unwrap_or((key, ""));
        let index = item
            .rsplit('/')
            .next()
            .and_then(|last| last.parse::<usize>().ok());
        (
            component.to_string(),
            (!item.is_empty()).then(|| item.to_string()),
            index,
        )
    }
}

/// A scope built back out of what [`Live::scope_of`] recovered.
pub fn scope<'a>(item: Option<&'a String>, index: Option<usize>) -> Scope<'a> {
    Scope {
        item: item.map(String::as_str),
        index,
    }
}
