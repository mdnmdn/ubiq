//! The components Ubiq's own A2UI catalog adds, and how one is reached.
//!
//! **The basic catalog is an enum; everything else is this table.** A2UI's stated extension point
//! is the catalog — an application publishes the components an agent may use, and most production
//! applications are expected to publish one of their own. `crate::state::a2ui::Kind` carries the
//! eighteen of the basic catalog because they are a fixed list Ubiq did not choose; anything else
//! parses into `Kind::Extension` with its properties intact and is offered here by name.
//!
//! **Two sources of truth, held together by a test.** The drawing lives in Rust and the
//! declaration lives in `state/a2ui/ubiq-catalog.json`, because the second is what an agent would
//! be handed and the first is what an agent cannot be handed. `crates/ubiq/tests/a2ui.rs` asserts
//! they name the same components, which is the same discipline `just icons-check` keeps over the
//! icon registry. See `D114`.
//!
//! A name in neither is drawn as a placeholder, which is what the protocol asks a renderer to do
//! with a component it does not implement.

use gpui::AnyElement;
use serde_json::{Map, Value};

use crate::state::a2ui::{Dynamic, Scope};

use super::Ctx;

/// One extension component, and the way back to everything around it.
pub struct ExtCtx<'a> {
    /// The component's own id, for an element id that has to be unique.
    pub id: &'a str,
    /// Its properties, exactly as the payload wrote them. An extension validates its own.
    pub props: &'a Map<String, Value>,
    pub ctx: &'a Ctx<'a>,
    pub depth: u32,
    pub scope: Scope<'a>,
}

impl ExtCtx<'_> {
    /// Draw a child by id — an extension may be a container like any other component.
    pub fn child(&self, id: &str) -> AnyElement {
        super::node(self.ctx, id, self.depth + 1, self.scope)
    }

    /// A property as the binding runtime reads it: a literal, a pointer, or a function call.
    pub fn dynamic(&self, key: &str) -> Option<Dynamic> {
        let value = self.props.get(key)?;
        serde_json::from_value(value.clone()).ok()
    }

    /// A property, resolved to what it says.
    pub fn text(&self, key: &str) -> String {
        self.dynamic(key)
            .map(|value| value.as_display_string(self.ctx.model, self.scope))
            .unwrap_or_default()
    }
}

/// How an extension component is drawn.
pub type Draw = fn(&ExtCtx) -> AnyElement;

/// Every component Ubiq's catalog adds.
///
/// A table rather than a trait: there are two entries' worth of variation between these, and a
/// function pointer is the whole of what a drawing function is.
pub const REGISTRY: &[(&str, Draw)] = &[("Svg", super::svg::draw)];

/// The drawing function for a component name, where this catalog has one.
pub fn lookup(name: &str) -> Option<Draw> {
    REGISTRY
        .iter()
        .find(|(registered, _)| *registered == name)
        .map(|(_, draw)| *draw)
}

/// Every name this catalog claims, for the test that holds it to the catalog document.
pub fn names() -> impl Iterator<Item = &'static str> {
    REGISTRY.iter().map(|(name, _)| *name)
}
