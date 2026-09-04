//! The driver: what turns a keystroke into an edit on whichever input has focus.
//!
//! This is the only module that knows an `EditorState` from a `TextareaState`, and the only one
//! that names gpui. The command set itself is in `state/vim/`, over a `&str` and a byte range, so
//! that it can be tested without a window.
//!
//! **Interception, not observation.** `App::observe_keystrokes` fires after dispatch has already
//! happened and cannot consume anything; `App::intercept_keystrokes` runs before binding matching
//! and honours `cx.stop_propagation()`, which is what stops a bare `d` from typing a letter.

use gpui::{App, Context, Subscription, Window};
use gpui_component::WindowExt as _;
use gpui_component::input::{AnyInputState, EditorState, Redo, TextareaState, Undo};

use crate::app::{AppState, CloseEditor, SaveFile};
use crate::state::vim::{Doc, Effect, Key, VimMode, VimState};

/// Register the interceptor for this window. The subscription is held for the window's life, the
/// way every other one in `boot.rs` is.
pub(super) fn install(window: &Window, cx: &mut Context<AppState>) -> Subscription {
    let handle = window.window_handle();
    let this = cx.entity().downgrade();

    cx.intercept_keystrokes(move |event, window, cx| {
        // One host, several windows, one app-wide interceptor: a key typed in another window is
        // not this window's business.
        if window.window_handle() != handle {
            return;
        }
        let Some(this) = this.upgrade() else {
            return;
        };
        let key = translate(&event.keystroke);
        let swallowed = this.update(cx, |app, cx| handle_key(app, key, window, cx));
        if swallowed {
            cx.stop_propagation();
        }
    })
}

/// A gpui keystroke as the command set sees it, or nothing when it carries a modifier vim does not
/// claim — `cmd-s`, `cmd-w` and `cmd-shift-f` have to keep working inside a buffer.
///
/// **Shift is folded into the key, and it has to be.** The platform resolves a shifted key two
/// different ways (`gpui_macos/src/events.rs`): shifted punctuation arrives already shifted, with
/// the flag cleared — `shift-4` is `$` — but a shifted *letter* arrives lowercase with the flag
/// still set, because a keymap writes that one as `shift-a`. Read naively, every uppercase command
/// in the set would be its lowercase twin: `A` would append at the cursor instead of the end of the
/// line, and `D` would move right.
fn translate(keystroke: &gpui::Keystroke) -> Option<Key> {
    let modifiers = &keystroke.modifiers;
    if modifiers.platform || modifiers.function || modifiers.alt {
        return None;
    }
    let shifted_letter = modifiers.shift
        && keystroke.key.len() == 1
        && keystroke.key.chars().all(|c| c.is_ascii_lowercase());
    Some(Key {
        key: if shifted_letter {
            keystroke.key.to_ascii_uppercase()
        } else {
            keystroke.key.clone()
        },
        ctrl: modifiers.control,
        shift: modifiers.shift,
    })
}

/// Whether the keystroke was swallowed. Every early return is a key the input goes on to receive
/// exactly as it does today.
fn handle_key(
    app: &mut AppState,
    key: Option<Key>,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> bool {
    if !app.workbench.settings.ui.vim_mode {
        return false;
    }
    let Some(key) = key else { return false };

    // Modal editing belongs to buffers and multi-line boxes. A filter field or a search box is a
    // question, not a document, and `hjkl` in one has to type letters.
    let input = match window.focused_input(cx) {
        Some(AnyInputState::Editor(state)) => Target::Editor(state),
        Some(AnyInputState::Textarea(state)) => Target::Textarea(state),
        _ => {
            // Focus left every input vim drives; the next one it reaches starts fresh.
            app.vim_focus = None;
            return false;
        }
    };

    // An input the user has only just moved to starts in the mode its kind deserves: a buffer is
    // navigated, a composer is typed into and sent.
    if app.vim_focus != Some(input.entity_id()) {
        app.vim_focus = Some(input.entity_id());
        app.vim = VimState {
            mode: match input {
                Target::Editor(_) => VimMode::Normal,
                Target::Textarea(_) => VimMode::Insert,
            },
            ..VimState::default()
        };
    }

    if !app.vim.claims(&key) {
        return false;
    }

    let mut vim = std::mem::take(&mut app.vim);
    let effects = input.with(cx, |doc| crate::state::vim::step(&mut vim, &key, doc));
    app.vim = vim;

    // The ex commands are the two effects that are about the window rather than about the buffer,
    // so they are answered here, where `AppState` is in hand, and never reach `Target`.
    let (window_effects, buffer_effects): (Vec<_>, Vec<_>) = effects
        .into_iter()
        .partition(|effect| matches!(effect, Effect::Save | Effect::Close { .. }));
    input.apply(buffer_effects, window, cx);
    for effect in window_effects {
        match effect {
            // The same paths `cmd-s` and `cmd-w` take, rather than a second way to save and close.
            Effect::Save => app.save_active_file(&SaveFile, window, cx),
            Effect::Close { discard: false } => app.close_active_editor(&CloseEditor, window, cx),
            Effect::Close { discard: true } => app.discard_active_editor(cx),
            _ => {}
        }
    }
    cx.notify();
    true
}

/// The two states modal editing drives. An enum rather than a trait: there are two of them, they
/// are both `InputBaseState` underneath, and the component does not export a trait that says so.
enum Target {
    Editor(gpui::Entity<EditorState>),
    Textarea(gpui::Entity<TextareaState>),
}

/// Runs `$body` against whichever concrete state this is.
macro_rules! on {
    ($self:expr, |$state:ident| $body:expr) => {
        match $self {
            Target::Editor($state) => $body,
            Target::Textarea($state) => $body,
        }
    };
}

impl Target {
    fn entity_id(&self) -> gpui::EntityId {
        on!(self, |state| state.entity_id())
    }

    /// Snapshot the buffer and run `f` over it.
    ///
    /// ponytail: the whole buffer is copied to a `String` on every claimed keystroke — O(n) per
    /// keypress. The upgrade is taking the re-exported `Rope` in `Doc` instead, which is a
    /// signature change and nothing else.
    fn with<R>(&self, cx: &mut App, f: impl FnOnce(Doc<'_>) -> R) -> R {
        on!(self, |state| {
            let state = state.read(cx);
            let text = state.text().to_string();
            f(Doc {
                text: &text,
                sel: state.selected_range(),
            })
        })
    }

    fn apply(&self, effects: Vec<Effect>, window: &mut Window, cx: &mut App) {
        for effect in effects {
            match effect {
                Effect::Select(range) => on!(self, |state| state
                    .update(cx, |state, cx| state.set_selected_range(range, cx))),
                Effect::Replace(range, text) => on!(self, |state| state.update(cx, |state, cx| {
                    state.set_selected_range(range, cx);
                    state.replace(text, window, cx);
                })),
                // The component's undo stack is not reachable from here, so these two go back out
                // as its own actions rather than as a call.
                Effect::Undo => window.dispatch_action(Box::new(Undo), cx),
                Effect::Redo => window.dispatch_action(Box::new(Redo), cx),
                Effect::Yank(text) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
                // Partitioned off in `handle_key`: these are about the window, and `Target` has
                // only the buffer.
                Effect::Save | Effect::Close { .. } => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::translate;
    use gpui::{Keystroke, Modifiers};

    fn keystroke(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: None,
        }
    }

    /// The platform hands a shifted letter over lowercase with the flag still set, so the command
    /// set would read every uppercase command as its lowercase twin without this.
    #[test]
    fn a_shifted_letter_arrives_uppercase() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::none()
        };
        let key = translate(&keystroke("a", shift)).expect("claimed");
        assert_eq!(key.key, "A");
        assert_eq!(key.ch(), Some('A'));
    }

    /// Shifted punctuation is already resolved by the platform, and must not be touched again.
    #[test]
    fn shifted_punctuation_is_left_alone() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::none()
        };
        for punctuation in ["$", ":", "%", "{", "*"] {
            let key = translate(&keystroke(punctuation, shift)).expect("claimed");
            assert_eq!(key.key, punctuation);
        }
    }

    #[test]
    fn an_unshifted_letter_is_left_alone() {
        let key = translate(&keystroke("a", Modifiers::none())).expect("claimed");
        assert_eq!(key.key, "a");
    }

    /// `cmd-s` and friends belong to the window, so vim never sees them.
    #[test]
    fn a_command_keystroke_is_not_claimed() {
        let platform = Modifiers {
            platform: true,
            ..Modifiers::none()
        };
        assert!(translate(&keystroke("s", platform)).is_none());
    }
}
