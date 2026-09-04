//! Modal editing: the mode, the half-typed command, and the effects a keystroke asks for.
//!
//! **Nothing in here names gpui.** A keystroke arrives as a key and a set of modifiers, the buffer
//! arrives as a `&str` and a byte range, and what comes back is a list of edits somebody else
//! applies. That is what makes the command set testable without a window: `crates/ubiq/tests/vim.rs`
//! drives this module with plain strings.
//!
//! The driver that connects it to a real input is `app/vim.rs`, and it is the only thing that knows
//! an `EditorState` from a `TextareaState`.

use std::ops::Range;

pub mod motion;
pub mod object;
pub mod search;

mod step;
pub use step::step;

/// Which mode the focused input is in. One per window, not per input: exactly one input holds
/// focus, so a second mode would be a mode nobody is typing in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VimMode {
    /// Keys are commands. The default an editor is focused in.
    Normal,
    /// Keys are text. The default a textarea is focused in, and what `i` reaches.
    #[default]
    Insert,
    /// A selection is being extended, character-wise or line-wise.
    Visual,
    VisualLine,
}

impl VimMode {
    /// What the status bar calls it.
    pub fn label(self) -> &'static str {
        match self {
            VimMode::Normal => "NORMAL",
            VimMode::Insert => "INSERT",
            VimMode::Visual => "VISUAL",
            VimMode::VisualLine => "V-LINE",
        }
    }

    pub fn is_visual(self) -> bool {
        matches!(self, VimMode::Visual | VimMode::VisualLine)
    }
}

/// The buffer, as one keystroke sees it.
///
/// `sel` is a byte range into `text`; it is empty when there is only a caret.
#[derive(Clone, Debug)]
pub struct Doc<'a> {
    pub text: &'a str,
    pub sel: Range<usize>,
}

/// What a keystroke asks the input to do. Every edit is a `Replace`, including a delete (with an
/// empty string) and a paste (over an empty range) — one path through the driver rather than five.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    /// Move the caret, or set the visual selection. The two are the same operation.
    Select(Range<usize>),
    Replace(Range<usize>, String),
    /// The engine cannot reach the component's undo stack, so these go out as its own actions.
    Undo,
    Redo,
    /// Into the unnamed register and the system clipboard both, the way vim's `"+` behaves when
    /// `clipboard=unnamed`.
    Yank(String),
    /// `:w`. Write the active file back — the same path `cmd-s` takes.
    Save,
    /// `:q` and `:qa`. Close the active file's tab. `discard` is `:qa`, which takes an unsaved
    /// buffer with it rather than raising the confirmation `:q` would.
    Close {
        discard: bool,
    },
}

/// A keystroke, reduced to what the command set cares about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Key {
    /// The gpui key name: a single character for a printable key, otherwise `"escape"`,
    /// `"enter"`, `"backspace"`, `"tab"`, `"space"`, an arrow, and so on.
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
}

impl Key {
    pub fn new(key: &str) -> Self {
        Self {
            key: key.to_string(),
            ctrl: false,
            shift: false,
        }
    }

    pub fn ctrl(key: &str) -> Self {
        Self {
            key: key.to_string(),
            ctrl: true,
            shift: false,
        }
    }

    /// The single character this key types, when it types one.
    pub fn ch(&self) -> Option<char> {
        if self.ctrl {
            return None;
        }
        let mut chars = self.key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    }
}

/// The line along the bottom of the screen while `/`, `?` or `:` is open, and what has been typed
/// into it. One type for all three because they read the same — every key is text until Enter runs
/// it or Escape abandons it — and differ only in what the text then means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandLine {
    /// `/`, `?` or `:`. Drawn at the head of the line, and what decides how the text is read.
    pub lead: char,
    pub text: String,
}

/// Everything modal editing remembers between keystrokes.
#[derive(Clone, Debug, Default)]
pub struct VimState {
    pub mode: VimMode,
    /// The keys of a half-typed command, in order — `"d"` after `d`, `"di"` after `di`. Empty
    /// whenever the engine is waiting for the start of a command.
    pub pending: String,
    /// The numeric prefix being typed, or none. `0` is a motion until a count is open, which is
    /// why this is not just a `u32`.
    pub count: Option<u32>,
    /// The unnamed register: what the last delete or yank took.
    pub register: String,
    /// Whether the register holds whole lines, which decides whether `p` pastes before or after.
    pub register_linewise: bool,
    /// Where the visual selection was anchored when it started.
    pub anchor: usize,
    /// Where the cursor is while a visual selection is open. Tracked rather than read back off the
    /// selection, because a selection says where its two ends are and not which one is moving.
    pub cursor: usize,
    /// The selection the last visual mode left behind, for `gv`.
    pub last_visual: Option<Range<usize>>,
    /// The column `j` and `k` try to keep. Vim's "preferred column": walking down through a short
    /// line and out the other side has to come back to where the cursor started.
    pub preferred_col: Option<usize>,
    /// The last `f`/`F`/`t`/`T`, for `;` and `,`.
    pub last_find: Option<(char, bool, bool)>,
    /// The pattern the last `/` or `?` left behind.
    pub search: search::SearchLine,
    /// The command line while it is still open. While this is set every key is text for it.
    pub typing: Option<CommandLine>,
}

impl VimState {
    /// What the status bar shows: the mode, the command being typed over it, or the command line.
    pub fn label(&self) -> String {
        if let Some(line) = &self.typing {
            return format!("{}{}", line.lead, line.text);
        }
        let mut typed = String::new();
        if let Some(count) = self.count {
            typed.push_str(&count.to_string());
        }
        typed.push_str(&self.pending);
        if typed.is_empty() {
            self.mode.label().to_string()
        } else {
            typed
        }
    }

    /// Drop a half-typed command without leaving the mode. What Escape does in Normal mode, and
    /// what an unrecognised key does.
    pub fn clear_pending(&mut self) {
        self.pending.clear();
        self.count = None;
    }

    /// Whether a keystroke would be swallowed rather than typed.
    ///
    /// The driver asks before it intercepts, and the one interesting answer is Escape in Normal
    /// mode with nothing half-typed: vim has nothing to do with it, and swallowing it would trap
    /// the user in whatever modal the textarea sits in.
    pub fn claims(&self, key: &Key) -> bool {
        if self.typing.is_some() {
            return true;
        }
        match self.mode {
            VimMode::Insert => key.key == "escape",
            VimMode::Normal => {
                key.key != "escape" || !self.pending.is_empty() || self.count.is_some()
            }
            VimMode::Visual | VimMode::VisualLine => true,
        }
    }
}
