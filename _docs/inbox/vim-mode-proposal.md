---
id: inbox-vim-mode
title: Proposal — vim mode for the editor and textareas
kind: proposal
status: proposal
summary: Modal editing in the code editor and multi-line textareas — normal, insert, visual and replace modes, operator-pending with text objects, command-line search, and a status indicator — built as a Ubiq-side addon that drives gpui-component's existing editing primitives.
read_when: you are deciding how the editor handles keyboard input, or whether modal editing belongs in the component library or in Ubiq
updated: 2026-09-02
depends_on: [feat-workbench]
---

# Proposal — vim mode for the editor and textareas

**The editor and textareas are insert-only.** Every keystroke inserts a character or performs a
shortcut; there is no way to navigate with `hjkl`, select with `v`, delete with `d`, or compose
multi-key operators like `ciw` or `dap`. The `gpui-component` library's `InputBaseState<M>` has all
the low-level primitives — cursor movement, word/line boundaries, selection, text manipulation — but
exposes them only through standard macOS keybindings. There is no mode concept, no operator stack,
and no extension point for adding one: the `InputModeKind` trait is sealed with three compile-time
markers.

This proposes building vim mode as a **Ubiq-side module** that wraps the editor *and* multi-line
textareas (the chat composer, the agent input, the task description), intercepts keystrokes, and
translates vim commands into the component's existing API. It does not modify `gpui-component`.

The same `InputBaseState<M>` engine powers `EditorState`, `TextareaState`, and `InputState`. The
movement, selection, and editing methods (`left()`, `insert()`, `selected_text()`, etc.) are defined
on the shared base, not on the sealed `InputModeKind` trait — so vim commands work identically on
editors and textareas. The only difference is what Enter means: in the chat composer, bare Enter
submits the message; in the editor, it inserts a newline.

## 1. Where it stands

**The editor engine.** `InputBaseState<EditorMode>` (`gpui-component::input::base::state.rs`) is a
5300-line shared engine with: `left()`, `right()`, `up()`, `down()`, `home()`, `end()`,
`page_up()`, `page_down()`, `move_to_previous_word()`, `move_to_next_word()`, `move_to_start()`,
`move_to_end()`, `start_of_line()`, `end_of_line()`, `select_left()`, `select_right()`,
`select_word()`, `select_line()`, `select_all()`, `insert()`, `replace()`, `backspace()`,
`delete()`, `delete_to_beginning_of_line()`, `delete_to_end_of_line()`,
`delete_previous_word()`, `delete_next_word()`, `indent()`, `outdent()`, `undo()`, `redo()`,
`set_value()`, `replace_all()`, `selected_text()`, `cursor_position()`,
`set_cursor_position()`. These are the operations a vim mode needs to drive.

**No mode switching.** `InputModeKind` is `sealed` (`kind.rs:46-52`) — only `InputMode`,
`TextareaMode`, and `EditorMode` exist. The `readonly` flag on the state rejects user edits but
keeps the editor appearance normal; it is the closest concept to "normal mode" but has no
operator stack, no cursor shape changes, and no mode-dependent keybindings.

**The keybinding system.** All editor keybindings are registered under the `"Input"` context
(`state.rs:122-269`). GPUI's `KeyBinding::new(key, action, Some("Input"))` routes actions based on
the focused element's context. A vim mode would need to dynamically change the context or intercept
keystrokes at a higher level.

**No addon mechanism.** Unlike Zed's editor, `gpui-component`'s `Editor` has no `register_addon`
trait. The `Editor` facade (`ui/src/input/editor.rs`) is a thin wrapper over `EditorState` that
delegates rendering to `TextElement`. There is no hook for an external module to attach mode state
or intercept input before the engine sees it.

**The textarea landscape.** The chat composer (`ui/chat/composer.rs`) uses `TextareaState` with
`submit_on_enter(true)` — bare Enter emits `PressEnter { shift: false }`, which triggers
`send_chat()`; Shift+Enter inserts a newline. The agent inspector's input
(`ui/agents/inspector.rs`) follows the same pattern with `send_to_agent()`. The task description
(`ui/board/form.rs`) and project about (`ui/sink/project.rs`) use `TextareaState` without
`submit_on_enter`, so bare Enter inserts a newline. All of these would benefit from vim-style
navigation and editing, but the chat and agent inputs need Enter to retain its submit behaviour.

**What exists that helps.** The `Editor` and `Textarea` elements render a `Stateful<Div>` with
event handlers (`on_key_down`, `on_mouse_down`). The application can subscribe to the
`EditorState`/`TextareaState` entity and listen for `InputEvent` events. The `Escape` action
already exists and is dispatched by the engine. The cursor is rendered as a rectangular element whose
shape can be overridden by the element's style. The `submit_on_enter` flag on `TextareaState` is
already checked by the engine before dispatching Enter, so the vim module can read it to decide
whether Enter should submit or insert.

## 2. What this decides

Not whether vim users want modal editing — they do. Whether the mode is a thin wrapper that drives
the existing editor and textarea, or a deep fork of the engine:

- where the mode state lives — §3;
- how keystrokes are intercepted — §4;
- which commands exist — §5;
- how Enter behaves in textareas — §6;
- how the cursor looks per mode — §7;
- what a status indicator shows — §8;
- and what this does not try to be — §9.

## 3. Where the mode state lives

**A `VimState` struct in a new module `crates/ubiq/src/ui/vim/`.** It holds:

| Field | Type | Purpose |
|-------|------|---------|
| `mode` | `VimMode` | Current mode: Normal, Insert, Visual, VisualLine, VisualBlock, Replace, Command |
| `last_mode` | `Option<VimMode>` | The mode before the current one, for `Ctrl-O` return |
| `operator_stack` | `Vec<Operator>` | Pending operators for multi-key commands (`d`, `c`, `y`, `g`, `z`) |
| `count` | `u32` | Numeric prefix (`3j` = move 3 lines down) |
| `register` | `Option<char>` | Named register (`"a` before a delete/yank) |
| `stick_to_col` | `bool` | Whether `j`/`k` should preserve the preferred column |
| `search_state` | `SearchState` | Forward/backward search state for `n`/`N` |

`VimState` is **not** stored inside `EditorState` or `TextareaState` — it is a separate entity or
a field on a wrapper struct that owns the `Entity<T>`. This keeps the component's type signatures
unchanged and lets vim mode be toggled per-editor, per-textarea, or per-window.

**The wrapper struct is generic over the state type:**

```rust
pub struct VimInput<T> {
    state: Entity<T>,
    vim: VimState,
}

impl VimInput<EditorState> {
    pub fn new_editor(editor: Entity<EditorState>) -> Self { ... }
}

impl VimInput<TextareaState> {
    pub fn new_textarea(textarea: Entity<TextareaState>) -> Self { ... }
}
```

`VimInput<EditorState>` is what `AppState` holds for each open file when vim mode is enabled.
`VimInput<TextareaState>` wraps the chat composer and agent input. Both implement `Render` by
delegating to `Editor::new()` or `Textarea::new()` and overlaying the status indicator.

## 4. How keystrokes are intercepted

**Two viable routes, and one is clearly better.**

**Route A — `cx.observe_keystrokes()`.** GPUI provides a way to observe all keystrokes before they
reach the focused element. The vim module registers a keystroke observer that checks the current
mode and, in Normal mode, intercepts keystrokes that are vim commands (`h`, `j`, `k`, `l`, `w`,
`d`, `c`, etc.) before they reach the editor. In Insert mode, it passes everything through except
`Escape`. This is how Zed's vim crate works.

**Route B — rebind keys per mode.** Register different `KeyBinding`s under different context
strings, and dynamically switch the editor's key context when the mode changes. This is cleaner in
theory but requires the component to expose a context-switching mechanism it does not have.

**Route A, because it requires no component changes.** The keystroke observer runs before the
editor's own handlers. In Normal mode, the observer intercepts `h`/`j`/`k`/`l`/`w`/`b`/`e` and
calls the corresponding movement methods on the editor. It intercepts `d`/`c`/`y` and pushes onto
the operator stack, waiting for a motion or text object. It intercepts `i`/`a`/`o`/`I`/`A`/`O`
and switches to Insert mode. In Insert mode, only `Escape` is intercepted; everything else passes
through.

The observer has access to the `Entity<EditorState>` and can call its methods directly:

```rust
cx.observe_keystrokes(window, |event, window, cx| {
    let vim = /* get VimState */;
    match vim.mode {
        VimMode::Normal => handle_normal(event, &vim_editor, &mut vim, window, cx),
        VimMode::Insert => handle_insert(event, &mut vim, window, cx),
        VimMode::Visual { .. } => handle_visual(event, &vim_editor, &mut vim, window, cx),
        _ => false, // pass through
    }
});
```

The observer returns `true` to consume the keystroke (preventing it from reaching the editor) or
`false` to let it through.

## 5. The command set

**Phase 1 — the 80/20.** The commands a vim user reaches for in the first five minutes:

| Category | Commands |
|----------|----------|
| **Mode switching** | `i`, `I`, `a`, `A`, `o`, `O`, `v`, `V`, `Ctrl-V`, `R`, `Escape` |
| **Movement** | `h`, `j`, `k`, `l`, `w`, `b`, `e`, `0`, `$`, `^`, `gg`, `G`, `Ctrl-D`, `Ctrl-U`, `Ctrl-F`, `Ctrl-B` |
| **Editing** | `x`, `X`, `dd`, `D`, `cc`, `C`, `S`, `yy`, `Y`, `p`, `P`, `u`, `Ctrl-R` |
| **Operators** | `d` + motion/text-object, `c` + motion/text-object, `y` + motion/text-object |
| **Text objects** | `iw`, `aw`, `i"`, `a"`, `i(`, `a(`, `i{`, `a{`, `i[`, `a[`, `it`, `at` |
| **Search** | `/`, `?`, `n`, `N`, `*`, `#` |
| **Misc** | `.`, `>>`, `<<`, `==`, `J` |

**Phase 2 — the next tier.** Commands that round out the experience:

| Category | Commands |
|----------|----------|
| **Modes** | `Ctrl-O` (temporary normal), `R` (replace mode) |
| **Motions** | `f`, `F`, `t`, `T`, `;`, `,`, `%`, `0`–`9` (counts), `Ctrl-W` commands |
| **Operators** | `gU`, `gu` (case), `gq` (format), `>` / `<` as operators |
| **Visual** | `o` (swap end), `gv` (reselect), `~`, `U`, `u` in visual |
| **Windows** | `Ctrl-W` split/close/navigate (if the dock supports it) |
| **Buffers** | `:bn`, `:bp`, `:bd` mapped to tab operations |

**What this does not include.** Macros (`q`, `@`), ex commands beyond `:w`, `:q`, `:e`, `:bn`,
`:bp`, `:bd`, registers beyond the default and named delete/yank registers, folding commands
(`z` commands), or multi-file operations. These are Phase 3 or later, if at all.

**Operator-pending is the core of vim.** When the user presses `d`, the vim state enters
`OperatorPending { operator: Delete }`. The next keystroke is a motion (`w`, `$`, `gg`) or a text
object (`iw`, `a("`). The state computes the range and calls the editor's `replace()` or
`delete()` methods. If the keystroke is neither, the operator is cancelled and the stack is
cleared.

## 6. Enter in textareas — submit vs newline

**The problem.** In the chat composer, bare Enter submits the message. In vim Normal mode, `Enter`
naturally means "move to the beginning of the next line" — the same as `j^`. These conflict. The
user must not have to leave vim mode to send a chat message, and they must not accidentally send a
message when they meant to insert a newline.

**The rule.** The textarea's `submit_on_enter` flag decides what bare Enter does in Insert mode:

| `submit_on_enter` | Insert mode bare Enter | Normal mode Enter |
|---|---|---|
| `true` (chat, agent) | Submits (`PressEnter`) | Moves to next line (`j^`) |
| `false` (task, project) | Inserts a newline | Moves to next line (`j^`) |

In Insert mode, the vim module checks the flag before forwarding Enter to the engine. If
`submit_on_enter` is `true`, it emits `PressEnter` (which the existing subscription in `app.rs`
handles as a send). If `false`, it lets the engine insert a newline. In Normal mode, Enter is
always a motion regardless of the flag — this matches vim behaviour where Enter in Normal mode is
a movement, not an insert.

**Shift+Enter always inserts a newline.** This is already the behaviour the component provides: the
engine only emits `PressEnter { shift: false }` for bare Enter, and `PressEnter { shift: true }`
for Shift+Enter. The chat subscription ignores the shift variant, so the textarea inserts a newline.
No vim-specific handling is needed for this case.

**Why not map `Enter` to a different vim key.** Some vim users expect `<CR>` to mean "move to
next line" always. But the chat composer's Enter-to-submit convention predates vim mode and is the
primary way users send messages. A vim user who wants to insert a newline in Insert mode uses
Shift+Enter, which works today. The cost of remapping bare Enter to "always newline" is that the
user must discover a different way to send chat messages — and there is no vim convention for "send
the contents of this buffer to a server."

**Scope.** This only affects `VimInput<TextareaState>` where `submit_on_enter` is `true`. The chat
composer and agent input are the only two. The task description and project about textareas are
unaffected because their bare Enter already inserts a newline.

## 7. The cursor per mode

**Different shapes signal the mode.** The editor's cursor is rendered by the `TextElement` in
`element.rs`. Its shape is a property of the editor's presentation style, set via
`set_editor_style()`. The vim module changes the cursor style when the mode changes:

| Mode | Cursor shape | Colour |
|------|-------------|--------|
| Normal | Block (full cell) | Default cursor colour |
| Insert | Line (thin vertical bar) | Default cursor colour |
| Replace | Underline | Default cursor colour |
| Visual | Block (full cell), inverted | Selection colour |
| Command | Line (thin vertical bar) | Default cursor colour |

The `InputEditorStyle` struct carries the cursor style. Changing it via `set_editor_style()`
triggers a re-render with the new shape.

## 8. The status indicator

**A small label in the editor's bottom-left corner** showing:

```
── NORMAL ──
── INSERT ──
── VISUAL ──
── REPLACE ──
```

When an operator is pending, it shows the operator:

```
── d ──
── c ──
── y ──
```

When a count is active:

```
── 3j ──
```

This is rendered as an overlay element on top of the editor, not by the component library. The
positioning is straightforward — a `relative(0.)` positioned `div` at the bottom-left of the
editor's bounds.

## 9. What this does not try to be

- **Not a full vim clone.** No ex command line (`:set number`, `:%s/`), no macros, no marks, no
  folds, no digraphs, no omnifunc, no vimscript. The editor already has search and replace via the
  component's find bar; vim mode adds `n`/`N` to navigate matches.

- **Not a replacement for the editor's own keybindings.** When vim mode is off, the editor and
  textareas work exactly as they do today. When vim mode is on, it is a per-editor/per-textarea
  toggle, not an application-wide switch. A user can have some editors in vim mode and others in
  insert mode. The chat composer can be in vim mode while a task description textarea is not.

- **Not modifying gpui-component.** The vim module is entirely Ubiq-side. It drives the public API
  (`insert`, `replace`, `left`, `right`, `select_word`, etc.) and observes keystrokes. No changes
  to the component library.

- **Not competing with the terminal's vim.** This is for the code editor, not for vim running
  inside a terminal pane. The two are independent.

## 10. What this adds to the tree

| Component | Change |
|-----------|--------|
| `crates/ubiq/src/ui/vim/mod.rs` | New module: `VimState`, `VimMode`, `VimInput<T>`, `Operator`, mode transitions |
| `crates/ubiq/src/ui/vim/normal.rs` | Normal mode: keystroke dispatch, operator-pending, motions |
| `crates/ubiq/src/ui/vim/insert.rs` | Insert mode: intercepts `Escape`, `Ctrl-O`, and Enter-in-textarea |
| `crates/ubiq/src/ui/vim/visual.rs` | Visual/VisualLine/VisualBlock: selection extension, operators on selection |
| `crates/ubiq/src/ui/vim/motion.rs` | Motion computation: from keystroke sequence to byte range |
| `crates/ubiq/src/ui/vim/text_objects.rs` | Text object computation: `iw`, `aw`, `i"`, `a(`, etc. |
| `crates/ubiq/src/ui/vim/search.rs` | `/`, `?`, `n`, `N`, `*`, `#` — drives the component's search session |
| `crates/ubiq/src/ui/vim/replace.rs` | Replace mode: like insert but overwrites instead of inserting |
| `crates/ubiq/src/ui/vim/status.rs` | The status indicator overlay |
| `crates/ubiq/src/state/editor.rs` | `OpenFile` gains an `Option<VimInput<EditorState>>` field |
| `crates/ubiq/src/app.rs` | Keystroke observer registration for editors and textareas; `chat_input` and `agent_input` wrapped in `VimInput<TextareaState>` when vim is enabled |
| `crates/ubiq/src/ui/viewer/mod.rs` | `buffer()` function uses `VimInput<EditorState>` when vim is enabled |
| `crates/ubiq/src/ui/chat/composer.rs` | Composer renders `VimInput<TextareaState>` when vim is enabled; toolbar shows vim mode indicator |
| `crates/ubiq/src/ui/agents/inspector.rs` | Agent input wrapped in `VimInput<TextareaState>` when vim is enabled |

**No new crates. No changes to gpui-component. No new bus messages.** Vim mode is local to the
UI process.

## 11. Failure

| When | What happens |
|------|--------------|
| User types `d` in Normal mode, then `Escape` | Operator stack cleared; nothing deleted |
| User types `3j` but there are only 2 lines left | Cursor moves to the last line; count is consumed |
| User types `ciw` but nothing is a word | Operator cancelled; cursor does not move |
| User is in Insert mode and the editor is readonly | No effect (readonly already blocks edits) |
| User toggles vim mode off while in Normal mode | Mode resets to Insert; operator stack cleared; cursor shape updates |
| User types `:` | Command mode is not implemented in Phase 1; the keystroke is consumed with no effect |
| User types `dd` on the last line | The line is deleted; cursor stays at the end |
| User types `/pattern` | The component's find bar opens with the pattern; `n`/`N` navigate matches |
| User presses bare Enter in chat composer in Normal mode | Cursor moves to next line (vim convention); user presses `i` then Enter to submit |
| User presses bare Enter in chat composer in Insert mode | Message is submitted (existing `submit_on_enter` behaviour preserved) |
| User presses Shift+Enter in chat composer in Normal mode | A newline is inserted (Shift+Enter always inserts, regardless of mode) |

## 12. Phases

1. **Normal + Insert + basic movements (editors and textareas).** `VimState` with Normal and
   Insert modes. `hjkl`, `wb e0$^`, `iIoOaA`, `xX`, `ddDccCyyPP`, `u Ctrl-R`, `Escape`. Cursor
   shape changes. Status indicator. Enter-in-textarea behaviour (§6). The 80% of vim that 80% of
   users need, working on both `EditorState` and `TextareaState`.
2. **Visual mode and operators.** `v`, `V`, `Ctrl-V`. Operators `d`, `c`, `y` in visual mode.
   `>>`, `<<`, `==`. Text objects `iw`, `aw`, `i"`, `a"`, `i(`, `a(`. Operator-pending with
   count prefix.
3. **Search and more motions.** `/`, `?`, `n`, `N`, `*`, `#`. `f`, `F`, `t`, `T`. `%`. `Ctrl-D`,
   `Ctrl-U`. `gg`, `G`.
4. **Replace mode and `Ctrl-O`.** `R` enters replace mode. `Ctrl-O` in Insert mode performs one
   Normal-mode command and returns.
5. **Command mode and registers.** `:` with a minimal command set (`:w`, `:q`, `:e`, `:bn`,
   `:bp`, `:bd`). Named registers for delete/yank.

Phase 1 stands alone and is worth doing on its own. Phase 2 is the core of vim's editing power.
Phases 3–5 are refinements.

## 13. What this asks to be decided

- Vim mode is a Ubiq-side module that drives `gpui-component`'s public API, not a modification
  to the component library. The `InputModeKind` trait stays sealed.
- Keystroke interception is via `cx.observe_keystrokes()`, not via keybinding context switching.
  This requires no component changes and runs before the editor's own handlers.
- The mode state lives on `OpenFile` or a wrapper, not inside `EditorState` or `TextareaState`.
  Vim mode is per-editor and per-textarea, not per-application.
- The wrapper struct `VimInput<T>` is generic over the state type, supporting both
  `EditorState` and `TextareaState` with the same mode logic.
- In textareas with `submit_on_enter`, bare Enter in Insert mode submits (preserving existing
  behaviour); in Normal mode, Enter is always a movement. Shift+Enter always inserts a newline.
- The cursor shape is changed via `set_editor_style()`, which the component already supports.
- The status indicator is a Ubiq-rendered overlay, not a component feature.
- Phase 1 ships with Normal, Insert and basic commands on editors and textareas. The operator
  stack and text objects arrive in Phase 2.

## Related docs

- [`../features/workbench.md`](../features/workbench.md) — the editor and the tab strip this lives within
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the theme tokens the cursor and status indicator use
