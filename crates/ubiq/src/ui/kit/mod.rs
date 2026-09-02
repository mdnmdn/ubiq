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

pub mod canvas;
pub mod controls;
pub mod files;
pub mod menu;
pub mod overlay;
pub mod panel;

pub use controls::{
    badge, card, check_box, choice_pill, disclosure, elided, elided_with, field, ghost_button,
    icon_button, meter, mono, pill, primary_button, progress_ring, section_label, slab, state_chip,
    status_dot, stepper, toggle_pill,
};
pub use files::{INDENT, ROW_HEIGHT, file_row, filter_bar, kind_icon, twisty, view_switch};
pub use menu::{ContextItem, Picker, PickerStyle, context_menu, context_panel};
pub use overlay::{modal, modal_note};
pub use panel::{Tab, panel, panel_header, tab_strip};
