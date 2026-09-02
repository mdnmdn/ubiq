---
id: inbox-terminal-interaction
title: Proposal — terminal interaction enhancements
kind: proposal
status: proposal
summary: Unified plan for terminal pane interaction: keyboard pass-through audit, mouse text selection, copy/paste (building on the existing clipboard proposal), OS file drops, clickable hyperlinks, and a defocus escape chord.
read_when: you are deciding how the user interacts with a focused terminal pane — selecting text, copying, pasting, dropping files, or leaving the terminal
updated: 2026-09-02
depends_on: [feat-panes, feat-workbench]
---

# Proposal — terminal interaction enhancements

**Terminal panes are currently write-only from the user's perspective.** The harness draws a screen,
keystrokes reach it, and that is the whole of the interaction. The emulator's mouse handlers are
placeholders, there is no text selection or copy/paste, no way to drop a file path from the OS, and
no keyboard chord to release focus back to the workbench without clicking a non-terminal panel.

This proposes six capabilities, ordered by user impact:

1. **Keyboard pass-through audit** — confirm every special keystroke reaches the harness correctly
2. **Mouse text selection** — click and drag to select, double/triple click for word/line
3. **Copy and paste** — platform-native shortcuts, building on the existing clipboard proposal
4. **OS file drops** — drag a file from Finder/Explorer into a terminal pane
5. **Clickable hyperlinks** — OSC 8 links from harnesses and regex-detected URLs
6. **Defocus escape chord** — a keyboard shortcut to release the terminal's keyboard grip

## 1. Keyboard pass-through audit

**Every special keystroke already reaches the harness.** `keystroke_to_bytes`
(`vendor/gpui-terminal/src/input.rs:116-258`) converts all GPUI keystrokes to terminal bytes
without checking the `command` modifier — so `Cmd+C` becomes `Ctrl+C` (0x03, SIGINT), not a
clipboard copy. The full map:

| Keystroke | Terminal bytes | Status |
|-----------|---------------|--------|
| Tab | `\t` | Correct |
| Shift+Tab | `\x1b[Z` (backtab) | Correct |
| Page Up | `\x1b[5~` | Correct |
| Page Down | `\x1b[6~` | Correct |
| Escape | `\x1b` | Correct |
| Shift+char | Uppercase character (via `key_char`) | Correct |
| Arrow keys | `\x1b[A`/`\x1bOA` (APP_CURSOR) | Correct |
| Ctrl+letter | ASCII control chars 0x01–0x1A | Correct |
| Alt+key | ESC + key | Correct |
| Home/End | `\x1b[H` / `\x1b[F` | Correct |
| Insert/Delete | `\x1b[2~` / `\x1b[3~` | Correct |
| F1–F12 | Respective `\x1bOP`–`\x1b[24~` | Correct |

**What this changes: nothing.** The audit confirms the input pipeline is complete. The only
interception is the key handler callback (`view.rs:718-724`), which Ubiq does not currently set. The
next section proposes setting one — for clipboard shortcuts — but all other keystrokes continue
flowing through unchanged.

**One observation.** Because `keystroke_to_bytes` never checks `modifiers.command`, a `Cmd+C` on
Mac arrives at the harness as `Ctrl+C` (0x03, SIGINT). This is correct for terminals but wrong for
users who expect clipboard copy. The fix is the key handler described in §3 below — it intercepts
`Cmd+C` before `keystroke_to_bytes` runs, so the user gets clipboard and the harness keeps its
SIGINT.

## 2. Mouse text selection

**The infrastructure exists but is not wired.** `vendor/gpui-terminal/src/mouse.rs` provides:
- `pixel_to_cell()` — converts pixel coordinates to grid cell coordinates
- `Selection` / `SelectionType` — Simple, Word (double-click), Line (triple-click) selection types
- `selection_type_from_clicks()` — maps click count to selection type
- `mouse_button_report()` — SGR (1006) mouse reporting for harnesses that request it
- `scroll_report()` — scroll wheel in mouse mode, arrow-key translation in alternate screen

The view's mouse handlers (`view.rs:734-778`) are placeholders with `// TODO` comments. Alacritty's
`Term` holds an `Option<Selection>` and exposes `selection_to_string()`, `selection_new()`,
`selection_update()`, and `selection_clear()`.

### What mouse selection enables

| Action | Behaviour |
|--------|-----------|
| Click | Position cursor (if harness supports mouse) or start selection |
| Click+drag | Select text between start and current cell |
| Double-click | Select the word under the cursor |
| Triple-click | Select the entire line under the cursor |
| Click (no drag) | In mouse-reporting mode: send click to harness. In normal mode: clear selection |
| Scroll wheel | In alternate screen: send scroll to harness. In normal screen: scroll back/forward through scrollback |

### The mouse-reporting conflict

Some harnesses (vim, htop, btop) request SGR mouse reporting via `\x1b[?1006h`. When active, mouse
events must go to the harness, not create a text selection. The resolution:

- **When mouse reporting is OFF** (normal mode): mouse events create/update text selection.
  `on_mouse_down` starts or extends a selection. `on_mouse_move` updates the endpoint. `on_mouse_up`
  finalises the selection and copies it to the clipboard.
- **When mouse reporting is ON**: mouse events go to the harness via `mouse_button_report()`. No
  selection is created. The harness owns the mouse.
- **Scroll wheel**: in alternate screen (vim, less, etc.), sends scroll to harness. In normal screen,
  scrolls the pane's scrollback buffer. This matches every terminal emulator's behaviour.

### Selection rendering

The renderer (`vendor/gpui-terminal/src/render.rs`) needs a new pass that reads `Term`'s selection
range and paints selected cells with inverted or highlighted colours. The theme already has
`selection_background` — the renderer applies it to cells within the selection's start and end
points. This is the most visible piece of work in this section.

### Phase 2: scrollback navigation

Scrolling the pane's view through the scrollback buffer (up/down arrows or scroll wheel in normal
mode) is a separate piece that reuses the same infrastructure. The alacritty `Term` stores
scrollback lines; the renderer offsets which lines it draws. This is filed under G22 alongside
selection.

## 3. Copy and paste

**This section references and extends the existing clipboard proposal.** The full design is in
[`terminal-clipboard-proposal.md`](./terminal-clipboard-proposal.md). This summary covers the
interaction model.

### The shortcut set

| Shortcut | Mac | Windows/Linux | Behaviour |
|----------|-----|---------------|-----------|
| Copy | `Cmd+C` | `Ctrl+Shift+C` | Copy selected text to system clipboard |
| Paste | `Cmd+V` | `Ctrl+Shift+V` | Paste from system clipboard into the harness |
| SIGINT | `Ctrl+C` | `Ctrl+C` | Unchanged — sent to the PTY as 0x03 |

`Ctrl+C` is SIGINT on every platform, including Mac. The clipboard shortcut is a different key.

### How it works

The key handler on `TerminalView` intercepts copy/paste before `keystroke_to_bytes`:

**Copy** reads `Term::selection_to_string()` and writes it to the system `Clipboard` via
`arboard`. If nothing is selected, the keystroke falls through to the harness (Cmd+C with no
selection in vim should still reach the application).

**Paste** reads from the system `Clipboard` and wraps the text in bracketed paste:

```
\x1b[200~ <text> \x1b[201~
```

Bracketed paste (`XTerm` protocol) tells the harness the input came from a paste, not keystrokes.
Applications that support it (bash, zsh, vim, emacs, kitty, alacritty) treat pasted text differently
— bash does not history-search it, vim does not execute it as keystrokes. The escape sequences are
harmless when the harness ignores them.

### OSC 52 round-tripping

Harnesses can also reach the clipboard via OSC 52 escape sequences:

- **Store** (harness writes to clipboard): `TerminalEvent::ClipboardStore` is already dispatched
  (`event.rs:134-136`). Wire `with_clipboard_store_callback` at terminal creation.
- **Load** (harness reads from clipboard): `TerminalEvent::ClipboardLoad` is received but not
  handled (`view.rs:821-823`). Read clipboard, base64-encode, write the response back to the PTY.

This enables tmux, vim, and any application using OSC 52 to read and write the system clipboard.

### What this adds beyond the clipboard proposal

Mouse selection creating the selection that Cmd+C copies. The two features are independent —
keyboard copy reads whatever is selected, whether by mouse or by the harness's own selection
mechanism — but they are most useful together. The clipboard proposal's Phase 3 (mouse selection)
is the same work as §2 above.

## 4. OS file drops

**Dragging a file from Finder/Explorer into a terminal pane should paste its path.** This is how
every terminal emulator works: drop a file, get its absolute path inserted at the cursor position.

### The mechanism

GPUI supports `on_drop` handlers on elements. The terminal view registers one:

```rust
.on_drop(cx.listener(Self::on_drop))
```

The handler receives the OS drop data (file paths), and writes them to the PTY as bracketed paste
— one path per line if multiple files are dropped. This matches the standard terminal behaviour:
Kitty, Alacritty, iTerm2, and Windows Terminal all paste dropped file paths as bracketed paste.

### Path format

| Platform | Format | Example |
|----------|--------|---------|
| macOS | POSIX path | `/Users/mdn/project/src/main.rs` |
| Linux | POSIX path | `/home/mdn/project/src/main.rs` |
| Windows | Short or long path | `C:\Users\mdn\project\src\main.rs` |

The path is the raw OS path. The harness decides what to do with it — a shell inserts it into the
command line, vim inserts it as text, and so on.

### When mouse reporting is active

If the harness has requested mouse reporting, the drop still pastes the path. File drops are
operating-system events, not mouse events — they do not have cell coordinates and cannot be
reported via SGR. Every terminal emulator pastes on drop regardless of mouse mode.

### What this does not do

- Does not auto-open the file in the editor. The harness decides.
- Does not pass the path as a command-line argument. The harness decides.
- Does not interpret the path. It is bytes in the PTY, nothing more.

## 5. Defocus escape chord

**The problem.** When a terminal has keyboard focus, there is no way to release it without clicking
a non-terminal panel. The user's hands are on the keyboard; they should not have to reach for the
mouse. In a multiplexer with several panes, releasing focus from one pane to interact with the
dock's tabs, the explorer, or the chat requires a click — which breaks the keyboard-driven
workflow.

### The chord: `Escape` (standalone)

**One keystroke, not a chord.** The `Escape` key is the natural defocus because:

- It already means "cancel" or "leave" in every TUI application
- It has no useful terminal meaning when sent alone (ESC alone is `\x1b`, which most applications
  interpret as "prefix of an escape sequence" and ignore after a brief timeout)
- It is the key users reach for when they want to "get out of" something
- No harness uses bare Escape as a meaningful command — it always precedes a sequence or is part of
  a chord the harness itself defines (like vim's `ESC` to enter Normal mode, which is the harness
  handling it, not the terminal emulator)

**The implementation.** The key handler on `TerminalView` checks for a standalone Escape press
(no modifiers). When detected, instead of writing `\x1b` to the PTY, it:

1. Calls `blur_panes()` on `AppState` — the same method the dock calls when a non-terminal panel
   becomes active
2. Returns `true` from the key handler — consuming the keystroke

This puts the window back into the state where no pane holds the keyboard. The user can then:
- Click a different pane's tab to focus it
- Use keyboard shortcuts bound at the workbench level (Cmd+S, etc.)
- Navigate the dock, explorer, or chat

### Why not `Ctrl+Esc` or `Alt+Esc`

The user suggested these alternatives. They work, but they have drawbacks:

- **`Ctrl+Esc`**: on macOS, this chord is unused by the OS but not by applications — some use it
  for "show desktop" or "Mission Control" variants. It is also not a standard terminal chord.
- **`Alt+Esc`**: on macOS, this cycles through windows in reverse order — a system shortcut that
  should not be intercepted.
- **Bare `Escape`**: universally understood as "exit" or "cancel", has no conflicting system
  meaning, and is the standard way to exit insert mode in vim and similar applications.

**The risk with bare Escape** is that it adds a small delay to Escape sequences sent to the harness
— the terminal must wait to confirm the user did not press Escape followed by a character (which
would form an escape sequence like `\x1b[A` for Up arrow). This is the same trade-off every
terminal emulator makes when intercepting Escape, and the delay is typically 20–50ms, which is
imperceptible.

**An alternative to the delay**: only intercept Escape when no other key follows within the timeout.
This is already how `keystroke_to_bytes` works — it converts Escape to `\x1b` immediately and lets
alacritty's state machine decide whether it is the start of a sequence. The key handler would need
to defer the defocus by one event loop tick, checking whether the next keystroke completes a
sequence. This is more complex and only necessary if the delay is noticeable.

### Alternative: `Ctrl+Esc` as the chord

If bare Escape's delay is unacceptable, `Ctrl+Esc` is the fallback. It requires no delay (it is
unambiguously a single chord), and the conflict with macOS's system shortcuts is unlikely in
practice because Ubiq's window would have to be focused AND holding keyboard focus in a terminal
pane for the chord to fire — at which point the system shortcut has no window to act on.

### What defocus does to the UI

- The terminal pane keeps drawing (an agent working in the background stays visible)
- The focused panel in the dock reverts to whatever was last focused (or none)
- `pending_focus` is set to `None`
- The `Focus` message is **not** sent to the host — the coordinator's `focused` map still records
  the last pane, which is correct because the harness does not care whether someone is looking

## 6. Clickable hyperlinks

**Terminal output often contains URLs and file paths that the user wants to open.** Today, the user
must copy the URL, switch to a browser, paste it, and return — or mentally note a file path and
navigate to it in the explorer. Every modern terminal emulator makes links clickable: hover shows a
pointer cursor and underline, click opens the URL in the default browser or editor.

### Two sources of links

**OSC 8 hyperlinks.** Harnesses can annotate text with a URL using the OSC 8 escape sequence:

```
\e]8;;https://example.com\e\\click here\e]8;;\e\\
```

alacritty_terminal 0.25.1 — the version Ubiq uses — **fully supports this**. The VTE parser
processes OSC 8 and stores a `Hyperlink { id, uri }` on each `Cell` via `Cell::set_hyperlink()`.
The data is already in the grid. The vendored renderer (`vendor/gpui-terminal/src/render.rs`)
completely ignores it: it checks `Flags::UNDERLINE` but never reads `cell.hyperlink()`, so an
OSC 8 link renders as plain text with no visual indication and no click behaviour.

**Regex-detected URLs.** Most harnesses do not emit OSC 8 — they print bare URLs in output:

```
https://github.com/user/repo/issues/42
See https://docs.example.com/setup for details.
```

These are not annotated and require the terminal to detect them by pattern matching. This is the
approach Kitty, Alacritty, WezTerm and VS Code's terminal all take as a fallback: scan visible
text for URL patterns and treat matches as clickable regions.

### What clickable links enable

| Action | Behaviour |
|--------|-----------|
| Hover over an OSC 8 link | Cursor changes to pointer; link gets undercurl/underline highlight |
| Hover over a detected URL | Same visual feedback |
| Click an OSC 8 link | Opens `hyperlink.uri()` in the system browser via `cx.open_url()` |
| Click a detected URL | Opens the matched URL in the system browser |
| Cmd+click (optional) | Same as click — some terminals use Cmd as a "link mode" modifier, but a bare click is more discoverable |

### The implementation has three layers

**Layer 1: Visual feedback in the renderer.** The painting code in `render.rs` needs to read
`cell.hyperlink()` during the cell-batching pass. When a cell carries a hyperlink:

- Apply an undercurl or coloured underline (the theme's `accent` token, or a dedicated
  `link_underline` token)
- Record the cell's screen position and URI in a hit-test map

For regex-detected URLs, the renderer runs a second pass over visible text after painting: scan
each line for URL patterns, record matching ranges in the same hit-test map, and apply the same
visual styling. This pass runs only on lines that changed since the last paint (the dirty-line
bitmap alacritty already maintains).

**Layer 2: Hit-testing on hover and click.** The mouse handlers in `view.rs` — which are currently
placeholders — need to:

1. On `on_mouse_move`: convert pixel position to cell coordinates via `pixel_to_cell()`, look up
   the hit-test map, and change the cursor to a pointer when over a link. GPUI supports
   `window.set_cursor_style(CursorStyle::PointingHand)`.
2. On `on_mouse_down` (left click): look up the cell in the hit-test map. If it carries a URI,
   call `cx.open_url(uri)` and consume the event. If not, fall through to selection logic.

The hit-test map is rebuilt on every paint (it is cheap — one entry per linked cell, typically
a handful per screen) and stored alongside the renderer. The map is a `Vec<(CellRange, String)>`
mapping a row/column range to a URI.

**Layer 3: OSC 52 integration (optional, later).** Some terminals let the user copy a link's URL
to the clipboard on Cmd+C when hovering over it. This is a natural extension of the clipboard
proposal's copy logic — the key handler checks whether the cursor is over a link before falling
through to `selection_to_string()`.

### The regex pattern

The URL pattern should match the common cases without false positives on code output:

```
https?://[^\s<>"')\]]+
```

This matches `http://` and `https://` URLs terminated by whitespace, angle brackets, quotes, and
brackets — the standard terminators. It does not match trailing punctuation (commas, periods,
semicolons) that are typically not part of the URL.

File paths are a separate question. Detecting `/path/to/file` or `~/project/src/main.rs` as
clickable links risks false positives on code identifiers and import paths. Most terminals do not
auto-detect file paths — they leave that to the harness (which can emit OSC 8). This proposal
does not regex-detect file paths, only URLs.

### What this does not do

- Does not make every underlined text clickable — only text with an actual OSC 8 annotation or a
  matched URL pattern
- Does not auto-open URLs without user interaction — click is required
- Does not detect file paths — only `http://` and `https://` URLs
- Does not interfere with mouse selection — a click on a link opens it; a click on non-link text
  starts a selection (when mouse reporting is off)

### Theme tokens

Two new tokens for the link styling:

| Token | Purpose | Default |
|-------|---------|---------|
| `link_underline` | Colour of the undercurl on hoverable links | `accent` (the project's accent colour) |
| `link_underline_hover` | Colour when the pointer is over the link | `accent` brightened by 15% |

Both go in `crates/ubiq/src/theme.rs` and follow the token-not-colour rule (`D10`).

## 7. Implementation order

| Phase | What ships | Depends on |
|-------|-----------|------------|
| **1. Keyboard audit + defocus** | Confirm pass-through, implement Escape defocus chord | Nothing — the audit is a doc update, defocus is a key handler addition |
| **2. Copy/paste** | Cmd+C/V (Mac), Ctrl+Shift+C/V (Win/Linux), bracketed paste, OSC 52 | Key handler infrastructure from Phase 1 |
| **3. Clickable links** | OSC 8 visual styling, URL regex detection, hover cursor, click-to-open | Renderer changes (same layer as selection) |
| **4. Mouse selection** | Click/drag selection, double/triple click, copy-on-release | Selection rendering in the vendored emulator |
| **5. File drops** | OS file path paste via bracketed paste | GPUI's `on_drop` handler |

Phases 1 and 2 are independently valuable and can be shipped in a single PR. Phase 3 and 4 share
renderer work and benefit from being together. Phase 5 is straightforward and independent.

## 8. What this adds to the tree

| Component | Change |
|-----------|--------|
| `vendor/gpui-terminal/src/view.rs` | `with_key_handler` set in `TerminalView` for clipboard + defocus; `on_drop` handler; mouse selection wired through `on_mouse_down/up/move`; hyperlink hit-test on hover/click |
| `vendor/gpui-terminal/src/mouse.rs` | Already complete — no changes needed |
| `vendor/gpui-terminal/src/clipboard.rs` | Already complete — no changes needed |
| `vendor/gpui-terminal/src/render.rs` | Selection highlight rendering pass; OSC 8 hyperlink visual styling (undercurl); URL regex detection and hit-test map |
| `vendor/gpui-terminal/src/terminal.rs` | Expose selection methods as thin wrappers over `Term` |
| `crates/ubiq/src/app.rs` | Wire key handler, clipboard callbacks, and defocus logic at terminal creation (`open_pane`, ~line 1731) |
| `crates/ubiq/src/ui/terminal.rs` | No changes — the `TerminalView` handles everything internally |
| `crates/ubiq/src/theme.rs` | Two new tokens: `link_underline`, `link_underline_hover` |

**No new crates.** `arboard` is already a dependency of `gpui-terminal`. **No new bus messages** —
clipboard, file drops, and hyperlinks are local to the UI process.

## 9. What this asks to be decided

- **Bare `Escape` is the defocus chord**, with a small delay for escape-sequence disambiguation.
  If that delay proves noticeable, fall back to `Ctrl+Esc`.
- **Mouse selection is a later phase** — it requires renderer changes and is not needed for
  keyboard copy/paste to work.
- **File drops paste paths as bracketed paste**, matching every major terminal emulator.
- **`Ctrl+C` is SIGINT on every platform.** The clipboard shortcut is `Cmd+C` on Mac and
  `Ctrl+Shift+C` elsewhere. This is the same decision as the existing clipboard proposal.
- **Mouse reporting mode disables selection.** The harness owns the mouse when it has requested it.
  This matches Kitty, Alacritty and iTerm2 behaviour.
- **Clickable links detect `http://` and `https://` URLs only**, not file paths. File path
  detection produces too many false positives in code output; harnesses that want clickable paths
  can emit OSC 8.
- **Click opens links, not Cmd+click.** Bare click is more discoverable and matches Kitty and
  WezTerm. Cmd+click adds a modifier that users must discover.

## Related docs

- [`terminal-clipboard-proposal.md`](./terminal-clipboard-proposal.md) — the full clipboard design
  this builds on
- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — the pane rules this
  lives within
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the theme tokens selection highlight uses
- [`../backlog.md`](../backlog.md) — G22 covers mouse selection and scrollback as a known gap
