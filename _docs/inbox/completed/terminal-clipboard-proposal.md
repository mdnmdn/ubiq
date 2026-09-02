---
id: inbox-terminal-clipboard
title: Proposal — terminal clipboard integration
kind: proposal
status: proposal
summary: Platform-native copy and paste in terminal panes — Cmd+C/V on Mac, Ctrl+C/V on Windows, with Ctrl+C staying as SIGINT; OSC 52 clipboard load wired; mouse selection to clipboard.
read_when: you are deciding how terminal panes handle copy and paste, or how clipboard events from harnesses reach the system clipboard
updated: 2026-09-02
depends_on: [feat-panes, feat-workbench]
---

# Proposal — terminal clipboard integration

**Terminal panes have no application-level copy and paste.** Every keystroke, including `Cmd+C` and
`Cmd+V` on Mac, is converted to a terminal escape sequence and written to the PTY. `Ctrl+C` becomes
`0x03` (SIGINT) — which is correct terminal behaviour — but there is no way for the user to copy
selected text or paste from the system clipboard using the keyboard. The `Clipboard` struct wrapping
`arboard` exists but is never instantiated. The OSC 52 clipboard-store callback is wired but the
clipboard-load (paste from clipboard into a harness) is a TODO stub. Mouse selection is scaffolded
in `mouse.rs` with `Selection` and `pixel_to_cell` but the mouse handlers in `view.rs` are
placeholders.

This proposes wiring all of it: platform-native copy/paste, OSC 52 round-tripping, and mouse
selection to clipboard.

## 1. Where it stands

**The input pipeline.** `keystroke_to_bytes()` (`vendor/gpui-terminal/src/input.rs:116-258`) converts
every GPUI keystroke to terminal bytes. `Ctrl+A`–`Ctrl+Z` map to ASCII control characters 0x01–0x1A.
There is no check for `Cmd` modifier at all — GPUI's `Keystroke::modifiers` carries `command`,
`control`, `alt` and `shift`, but `keystroke_to_bytes` only reads `control`, `alt` and `shift`.

**The key handler.** `TerminalView` exposes `with_key_handler()` (`view.rs:576-582`), a callback
that runs before `keystroke_to_bytes`. If it returns `true`, the keystroke is consumed and never
reaches the PTY. Ubiq's terminal creation (`crates/ubiq/src/app.rs:1731-1741`) does not set a key
handler, so every keystroke flows through.

**The clipboard wrapper.** `Clipboard` (`vendor/gpui-terminal/src/clipboard.rs`) wraps `arboard` with
`copy(&str)`, `paste() -> String`, and `clear()`. It is exported from `lib.rs` but never constructed
in the application layer.

**OSC 52 store.** `ClipboardStoreCallback` (`view.rs:275-298`) is dispatched when a harness writes
`ESC ] 52 ; <sel> ; <data> BEL` to request a clipboard write. Ubiq does not set this callback.

**OSC 52 load.** `TerminalEvent::ClipboardLoad` (`event.rs:68,138-141`) arrives when a harness reads
`ESC ] 52 ; <sel> ; ? BEL` to request a clipboard read. The handler in `view.rs:821-823` is a
`TODO` stub — the event is received and dropped.

**Mouse selection.** `mouse.rs` defines `Selection`, `SelectionType`, `pixel_to_cell()`, and
`selection_type_from_clicks()`. `view.rs:736-777` has `on_mouse_down`, `on_mouse_up`, `on_mouse_move`
as placeholders with `// TODO: Implement mouse selection` comments.

**Alacritty's selection API.** `Term::selection_to_string()` returns `Option<String>` — the text
currently selected in the grid. It handles Simple, Block, Semantic and Lines selection types, line
wrapping, wide characters, and tab stops. The selection is stored on the `Term` as
`Option<Selection>`, and updated by mouse events through `Selection::update()`.

## 2. What this decides

Not whether terminals need clipboard — they do. Whether Ubiq intercepts Cmd/Ctrl at the application
layer via the existing key handler, or whether it leaves copy/paste to the harness alone:

- the shortcut set and platform mapping — §3;
- how selected text reaches the clipboard — §4;
- how paste reaches the PTY — §5;
- what OSC 52 round-tripping means — §6;
- what mouse selection requires — §7;
- and what happens when Ctrl+C must stay SIGINT on Mac — §8.

## 3. The shortcut set

**Three shortcuts, two platforms, one rule.** On Mac, the user expects `Cmd+C` to copy and
`Cmd+V` to paste, the way every native application works. On Mac, `Ctrl+C` must still send SIGINT
to the harness. On Windows and Linux, `Ctrl+C` is SIGINT and `Ctrl+V` is paste — there is no `Cmd`
key.

| Shortcut | Mac | Windows/Linux | Behaviour |
|----------|-----|---------------|-----------|
| `Cmd+C` / `Ctrl+Shift+C` | `Cmd+C` | `Ctrl+Shift+C` | Copy selected text to clipboard |
| `Cmd+V` / `Ctrl+Shift+V` | `Cmd+V` | `Ctrl+Shift+V` | Paste from clipboard into the harness |
| `Ctrl+C` | SIGINT (0x03) | SIGINT (0x03) | Unchanged — sent to the PTY |

The `Ctrl+Shift+C` / `Ctrl+Shift+V` alternatives exist because `Ctrl+C` on Mac must remain SIGINT.
On Windows/Linux, `Ctrl+C` is already SIGINT and `Ctrl+V` is paste — the shortcuts are consistent
with what the terminal already does, just intercepted at the application layer instead of left to
the harness.

**Why `Ctrl+Shift` and not `Ctrl+C` alone.** On Mac, `Ctrl+C` in a terminal means interrupt. That
convention is older than Ubiq and more important than a clipboard shortcut. Breaking it would make
every harness that catches SIGINT (vim, less, git, docker-compose) behave differently in Ubiq than
in any other terminal. The cost of `Shift` is one extra finger; the cost of breaking SIGINT is
every TUI application.

## 4. Copy — selected text to clipboard

**The key handler intercepts Cmd+C (Mac) or Ctrl+Shift+C (Win/Linux).** It calls
`Term::selection_to_string()` through the `TerminalState`'s `with_term()` accessor, and on
`Some(text)`, writes it to the system `Clipboard`. The keystroke is consumed — it never reaches
`keystroke_to_bytes`.

The flow:

```text
Cmd+C pressed
  → KeyHandler fires (view.rs:718-724)
  → TerminalState::with_term(|term| term.selection_to_string())  (terminal.rs:302-308)
  → Some(text) → Clipboard::copy(&text)  (clipboard.rs:79-82)
  → return true  (consume the event)
  → None → return false  (nothing selected, pass through)
```

**`selection_to_string()` is the right API.** It handles all four selection types (Simple, Block,
Semantic, Lines), line wrapping, wide characters, and tab stops. It returns `None` when nothing is
selected, which is the signal to fall through to the harness — `Cmd+C` with no selection in vim
should still reach the application.

**Clipboard lifetime.** `Clipboard::new()` opens a system clipboard handle. Creating one per copy is
the arboard-recommended pattern — the handle is short-lived and the OS manages the actual storage.
There is no reason to hold a persistent handle.

## 5. Paste — clipboard text into the harness

**The key handler intercepts Cmd+V (Mac) or Ctrl+Shift+V (Win/Linux).** It reads from the system
`Clipboard`, and if the text is non-empty, writes it to the PTY stdin with bracketed paste wrapping:

```text
\x1b[200~ <text> \x1b[201~
```

Bracketed paste (`XTerm` protocol) tells the harness that the input came from a paste, not from
keystrokes. Applications that support it (bash, zsh, vim, emacs, kitty, alacritty) treat pasted
text differently — bash does not history-search it, vim does not execute it as keystrokes.

**When the harness does not support bracketed paste.** The escape sequences are harmless — they are
ignored by applications that do not recognise them, and the text arrives as literal bytes. The worst
case is that `\x1b[200~` and `\x1b[201~` appear as garbage in a very old application, which is the
same trade-off every modern terminal makes.

**Multi-line paste.** A clipboard containing newlines is pasted as-is within the bracketed wrapper.
The harness decides what to do with each line. This matches iTerm2, Kitty and Alacritty behaviour.

## 6. OSC 52 round-tripping

**Store (harness writes to clipboard).** `TerminalEvent::ClipboardStore(text)` is already dispatched
(`event.rs:134-136`). Ubiq wires `with_clipboard_store_callback(|window, cx, text| { Clipboard::copy(text) })`.
This enables tmux, vim, and any application using OSC 52 to write to the system clipboard.

**Load (harness reads from clipboard).** `TerminalEvent::ClipboardLoad` is received but not handled
(`view.rs:821-823`). The alacritty event carries a callback that expects the clipboard content to be
written back to the PTY. The implementation:

1. Read from `Clipboard::paste()`.
2. Base64-encode the text (OSC 52 protocol requires base64).
3. Write `ESC ] 52 ; <sel> ; <base64> ESC \` (or `BEL`) to the PTY stdin.

This requires a callback-style or channel-based return path from `process_events` to the PTY writer.
The cleanest approach is a new `ClipboardLoadCallback` type on `TerminalView`, matching the existing
`ClipboardStoreCallback` pattern.

**When the harness does not support OSC 52.** The escape sequence is ignored. No harm done.

## 7. Mouse selection

**Phase 1: click to start selection, drag to extend, release to end.** `on_mouse_down` records the
start cell via `pixel_to_cell()`. `on_mouse_move` updates the selection end. `on_mouse_up` finalises
the selection and copies it to the clipboard (if the user was not holding a modifier — standard
terminal behaviour copies-on-mouse-up only when not in mouse-reporting mode).

**Phase 2: click semantics.** Single click = character selection. Double click = word selection.
Triple click = line selection. `selection_type_from_clicks()` already maps click counts to
`SelectionType`.

**The selection lives on the `Term`.** `Term` holds `Option<Selection>`, and `selection_to_string()`
reads it. To set it programmatically, `Term` exposes `selection_update()` which is called by mouse
event handlers. The mouse handlers in `view.rs` need to:

1. Convert pixel coordinates to cell coordinates via `pixel_to_cell()`.
2. Call `Term::selection_new()` (Simple, Block, Semantic or Lines) on click.
3. Call `Term::selection_update()` on mouse move.
4. Read the selection on mouse up via `selection_to_string()`.

This is the most complex piece — it requires holding the `Term` mutex across mouse events and
coordinating with the renderer to show the selection highlight. The renderer (`render.rs`) would
need to read the selection range and paint selected cells with inverted or highlighted colours.

**What mouse selection does not replace.** Keyboard copy via `Cmd+C` works independently — it reads
the same `Term::selection_to_string()`. Mouse selection is the way the selection is *created*;
keyboard copy is the way it is *used*.

## 8. Ctrl+C as SIGINT on Mac

**The critical invariant: Ctrl+C is SIGINT, always, on every platform.** `keystroke_to_bytes`
converts `Ctrl+C` to `0x03` (`input.rs:188-213`). The key handler must not intercept `Ctrl+C` on
Mac — it must only intercept `Cmd+C`.

The key handler logic:

```rust
|event| {
    let ks = &event.keystroke;
    let copy = if cfg!(target_os = "macos") {
        ks.modifiers.command && ks.key == "c"
    } else {
        ks.modifiers.control && ks.modifiers.shift && ks.key == "c"
    };
    let paste = if cfg!(target_os = "macos") {
        ks.modifiers.command && ks.key == "v"
    } else {
        ks.modifiers.control && ks.modifiers.shift && ks.key == "v"
    };
    // ... handle copy/paste, return false for everything else
}
```

**Why not intercept Ctrl+C and re-inject SIGINT when the user wants it.** There is no reliable way
to know when the user "wants" SIGINT versus clipboard copy. In a shell, it is always SIGINT. In
vim's visual mode, it might be copy. The only correct rule is: `Ctrl+C` is always SIGINT at the
terminal level, and the clipboard shortcut is a different key.

## 9. What this adds to the tree

| Component | Change |
|-----------|--------|
| `vendor/gpui-terminal/src/view.rs` | Add `clipboard_load_callback` field and `with_clipboard_load_callback()` builder; implement `ClipboardLoad` dispatch in `process_events`; add `Clipboard` import |
| `vendor/gpui-terminal/src/view.rs` | `on_mouse_down/up/move` call through to `TerminalState` selection methods |
| `vendor/gpui-terminal/src/event.rs` | `TerminalEvent::ClipboardLoad` carries the base64 clipboard type from alacritty's `Event::ClipboardLoad` |
| `vendor/gpui-terminal/src/terminal.rs` | Expose selection methods: `selection_to_string()`, `selection_new()`, `selection_update()`, `selection_clear()` — thin wrappers over `Term`'s own methods |
| `crates/ubiq/src/app.rs` | Wire the key handler, clipboard store callback, and clipboard load callback at terminal creation (line 1731) |
| `crates/ubiq/src/app.rs` | Instantiate `Clipboard` in the key handler closure |

**No new crates.** `arboard` is already a dependency of `gpui-terminal`. No new message types in
`ubiq-proto` — clipboard is local to the UI process, not a bus concern.

## 10. Failure

| When | What happens |
|------|--------------|
| Nothing is selected and the user presses Cmd+C | `selection_to_string()` returns `None`; key handler returns `false`; keystroke falls through to harness (harmless) |
| Clipboard is empty and the user presses Cmd+V | `Clipboard::paste()` returns `Err`; key handler returns `false`; no bytes written to PTY |
| Harness does not support bracketed paste | The `\x1b[200~` sequences are ignored; text arrives as literal bytes |
| Harness does not support OSC 52 | The escape sequence is ignored |
| Clipboard system is unavailable (headless CI) | `Clipboard::new()` returns `Err`; copy/paste silently fail; key handler returns `false` |
| Mouse selection in a harness with mouse reporting | The harness receives both the selection and mouse events; the selection is for the application to use, not Ubiq to interpret |
| User drags to select in a harness with SGR mouse reporting | Mouse events go to the harness via `mouse_button_report()`; selection is visual only and copied on release if reporting is off |

## 11. Phases

1. **Keyboard copy and paste.** The key handler in `app.rs` intercepts `Cmd+C`/`Cmd+V` (Mac) and
   `Ctrl+Shift+C`/`Ctrl+Shift+V` (Win/Linux). Copy calls `selection_to_string()` via
   `TerminalState::with_term()`. Paste reads `Clipboard::paste()` and writes bracketed paste to the
   PTY. `Clipboard` is instantiated per-operation. No mouse changes, no OSC 52 changes.
2. **OSC 52 store and load.** Wire `with_clipboard_store_callback` at terminal creation. Add
   `ClipboardLoadCallback` to `TerminalView` and handle `TerminalEvent::ClipboardLoad` in
   `process_events`. The callback reads clipboard, base64-encodes, writes to PTY.
3. **Mouse selection.** Implement `on_mouse_down/up/move` to create and update `Term` selections.
   Add selection highlight rendering. Copy-on-mouse-up when mouse reporting is off.

Phase 1 is the user-visible fix — it resolves the immediate pain of not being able to Cmd+C/V in
terminals. Phases 2 and 3 are independently valuable and do not block each other.

## 12. What this asks to be decided

- Platform-native shortcuts are intercepted at the `KeyHandler` layer, not by modifying
  `keystroke_to_bytes`. The input module stays pure terminal; the application layer decides what to
  intercept.
- `Ctrl+C` is SIGINT on every platform, including Mac. The clipboard shortcut is `Cmd+C` on Mac and
  `Ctrl+Shift+C` elsewhere.
- Paste uses bracketed paste wrapping (`\x1b[200~` / `\x1b[201~`), which is the standard protocol
  and harmless when the harness ignores it.
- OSC 52 load is wired as a callback on `TerminalView`, matching the existing store callback pattern.
- Mouse selection is a later phase — it requires renderer changes and is not needed for keyboard
  copy/paste to work.

## Related docs

- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — the pane rules this lives within
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the UI conventions the key handler follows
