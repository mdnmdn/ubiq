//! Reusable primitives, with no knowledge of the workbench.
//!
//! Only what `gpui-component` does not already give us belongs here — its `Icon`, `Kbd`, `Editor`,
//! `Textarea`, `Scrollbar` and resizable group are used directly rather than wrapped.

use std::rc::Rc;

use gpui::{App, Window};

/// A handler the kit can call without knowing which view owns it. Call sites build one with
/// `ui::handler`.
pub type Action = Rc<dyn Fn(&mut Window, &mut App)>;

/// The same, for the callbacks that carry the row or tab that was clicked.
pub type IndexedAction = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub mod controls;
pub mod menu;
pub mod panel;

pub use controls::{
    badge, ghost_button, icon_button, mono, pill, progress_ring, section_label, status_dot,
};
pub use menu::{Picker, PickerStyle};
pub use panel::{Tab, panel, panel_header, tab_strip};
