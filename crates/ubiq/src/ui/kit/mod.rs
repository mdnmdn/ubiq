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

/// Which [`UbiqIcon`] draws a harness — matched on the library's own id (`AgentTypeInfo::id`,
/// e.g. `"claude-code"`) where that reached the call site, or on the display name the host minted
/// for a live conversation (`AgentTypeInfo::label`) where only that did. Answers
/// [`UbiqIcon::HarnessAny`] for anything neither form names.
///
/// The fallback is deliberate, not an oversight: the asterisk this replaces is Claude's own mark,
/// so a harness with no mark drawn for it yet — or one this match cannot even identify — must
/// land on the honest "no harness named" icon rather than borrow Claude's, which is exactly the
/// bug this helper exists to close.
pub fn harness_icon(harness: &str) -> UbiqIcon {
    match harness {
        "claude-code" | "claude-code-acp" | "Claude Code" | "Claude Code (ACP)" => {
            UbiqIcon::HarnessClaudeCode
        }
        "codex" | "codex-acp" | "Codex" | "Codex (ACP)" => UbiqIcon::HarnessCodex,
        "opencode" => UbiqIcon::HarnessOpencode,
        "copilot" | "GitHub Copilot" => UbiqIcon::HarnessCopilotCli,
        "grok" | "Grok CLI" => UbiqIcon::HarnessGrok,
        _ => UbiqIcon::HarnessAny,
    }
}

pub mod canvas;
pub mod controls;
pub mod files;
pub mod icons;
pub mod menu;
pub mod overlay;
pub mod panel;
pub mod settings;

pub use controls::{
    badge, card, check_box, choice_pill, disclosure, elided, elided_with, field, ghost_button,
    icon_button, meter, mono, pill, primary_button, progress_ring, progress_ring_in, removable_tag,
    section_label, slab, state_chip, status_dot, stepper, toggle_pill,
};
pub use files::{
    file_row, filter_bar, kind_icon, row_font, row_height, row_indent, twisty, view_switch,
};
pub use icons::UbiqIcon;
pub use menu::{ContextItem, Picker, PickerStyle, context_menu, context_panel};
pub use overlay::{confirm_modal, modal, modal_note, modal_sized, prompt_modal};
pub use panel::{Tab, panel, panel_header, tab_strip};
pub use settings::{column, heading, hint_row, label_block, label_hint, nav_item, setting_row};
