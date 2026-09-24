---
id: inbox-tui-ecosystem
title: The terminal interface's crate map — what each screen buys
kind: inbox
status: proposal
summary: "The ratatui and extra-widget landscape mapped onto the terminal interface's screens: which part adopts a crate, which adapts one to its own theme and keyboard, which is written, and which is delegated to the terminal's own incumbent — against the lockfile's 0.30 graph and the seed's §5 line that the pane's emulator stays alacritty's. The map is a second client's view of host facts, never a re-draw of the window: four adopts (nucleo-matcher, ratatui-textarea, tui-tree-widget, ansi-to-tui), two already resolving (syntect, pulldown-cmark), the rest written on core widgets, and editing, git, prose and paging delegated to `$EDITOR`, tig and `bat`/`glow`. Adds no register rows."
read_when: you are choosing a widget crate for a terminal-interface screen, or arguing why a part is written rather than bought
updated: 2026-09-24
depends_on: [inbox-tui, inbox-tui-mode-fit, inbox-tui-release]
---

# The terminal interface's crate map — what each screen buys

## Purpose

The [mode-fit analysis](./mode-fit-analysis.md) decides what a terminal draws and
[tui-release-scope.md](./tui-release-scope.md) decides when. This document decides what each drawn
screen is built from: which ratatui ecosystem crate it adopts, which it adapts, which it writes, and
which it leaves to the terminal's own incumbent. Every part is a second client's view of a host fact
— never a re-draw of the window. It is a map of the landscape as of 2026-09-24, not a lockfile — the
verdicts are the point, the versions are a snapshot.

## The lens — five tests a candidate crate must pass

1. **It sits on the lockfile's graph.** The tree resolves ratatui 0.30.2 — `ratatui-core` 0.1.2,
   `ratatui-widgets`, the backends — and crossterm 0.28. The version was *chosen*: ratatui 0.29 pins a
   `unicode-width` that cannot coexist with the `^0.2.2` merman's renderer needs (`crates/agent-manager/Cargo.toml`).
   A widget pinning 0.29, or dragging a second `unicode-width`, forks the tree twice over.
2. **It stays on the right side of the tree checks.** A candidate lives in `crates/ubiq-tui`, names
   the contract crate and ratatui and nothing else of Ubiq's, and survives the terminal-only build —
   no GPUI, no window crate, no host crate. `just tui` and the phase-4 narrowed build are what
   enforce it.
3. **It draws with our tokens or not at all.** `D164` makes the TUI's theme file the second file in
   the tree allowed to name a colour. A widget whose defaults are a palette of its own — syntect
   themes, `tui-markdown`'s style sheet — is adapted, never imported.
4. **It does not bring a VT parser toward the panes.** The seed's §5 line is absolute: the emulator
   is alacritty's, walked into a ratatui buffer, and any crate that parses a terminal byte stream for
   a *live pane* is rejected. `ansi-to-tui` renders text that has already materialised; it is not a
   parser of a pane's stream, so it stays legal.
5. **It is maintained, MIT/Apache, and small, or written.** A dependency that is unmaintained or
   API-unstable earns a fork; a fork earns a question whether the widget is worth buying at all.

## The terminal is not the window in another medium

A TUI that re-draws the window screen for screen fails the [mode-fit scorecard](./mode-fit-analysis.md)
twice over: it competes with incumbents for no reason, and it abandons what the medium does best. The
seed's §2 rule carries the same reading — *a second interface is a second client, and nothing else* —
and this map reads it as a **delegation line**: where the terminal already ships a better answer, Ubiq
does not port and does not buy; it composes. The Git drop was this rule applied once; here it is the
general law:

| The job | The incumbent | The TUI does |
|---|---|---|
| Edit a file | `$EDITOR` — nvim, helix, vi | Open it, never build a buffer. The composer is a form field; a file is the editor's |
| Read a file | `bat`, `less` | Preview in a pane in the fzf idiom; to actually read it, `bat` |
| Read markdown prose | `glow` | The scorecard's `glow`/`less` audience stays the shell's; in-app prose is only the preview case |
| Review git | `tig`, lazygit, `git diff`/delta | Nothing — the mode is dropped on exactly this line |
| Hunt files and symbols | `fzf`, `fd`, `rg` | The picker and palette adopt the *idiom* — prompt, matcher, preview pane — not the binary, because their answer feeds host state and so lives in-app |
| Split and session | tmux, zellij | Tolerate an outer multiplexer (a different leader key, never a war for the screen); the leader-key tiling of phase 3 is the in-TUI answer when there is no multiplexer |
| Follow logs | `tail -f` | The console is host-owned, so it builds — its shape is `tail`: one window, two controls |
| Search in files | `rg` + `fzf` | In-app results that feed state; the shell's own search stays the shell's |

The measure of a screen is therefore not "does it match the window's" — it is *did it use what the
medium gives it for free*. A part in the map below lands on one of four verdicts. **Adopted** is a
shaped interaction the medium lacks. **Written** is a host stream, a core widget already, or a piece
of SSH reality. **Delegated** is the third verdict implicit in every exclusion: the incumbent is a key
away, and no crate and no screen is spent on it.

## The map

### Phase 0 — attach, Control, palette, status line

| Part | Candidates | Verdict | Why |
|---|---|---|---|
| Frame loop, terminal init | ratatui 0.30 `run`/`init`; `crates-tui` as the app-structure reference | **adopt** (core + pattern) | `crossterm` event poll with a timeout, one `draw` per event, configurable key→action — the reference app's shape is exactly a small/medium TUI |
| Control pages — host readings, usage meter | built-in `Table` | **core** | The screen is already a fixed-column table. The scorecard's "no bar, no percentage" holds, so `Chart`/`BarChart`/`Sparkline` exist in core and stay unused here |
| Command palette | write over `nucleo-matcher`; `ratatui-labs` command-palette PRD as the state-machine reference | **write** (state machine), **adapt** (matcher) | The lab PRD is experimental — take its action model, not its crate. The palette's grammar is the window navigator's (release scope); a matcher is the only buyable piece (below) |
| Status line | write: one `Paragraph` + `ansi-to-tui` for segments that arrive coloured | **write** | A read-only line. `ratatui-toolkit`'s status bar is a component set no screen needs yet |

### Phase 1 — projects, explorer, search, viewer

| Part | Candidates | Verdict | Why |
|---|---|---|---|
| Project list, search-results list | built-in `List` / `Table` | **core** | Host polling answers sorted rows; a list widget draws them |
| Explorer tree | `tui-tree-widget` 0.24 | **adapt** | `Tree`/`TreeState` keeps open/selected state and renders collapsible rows — the explorer idiom verbatim. Wrap the host's tree model behind it and map its emphasis styles to tokens |
| File picker (fuzzy) | `nucleo-matcher` 0.3 | **adopt** | The `ranger/nnn/fzf` idiom the scorecard names; the matcher is helix-maintained, fzf-scored, ~6× skim, grapheme-correct. `skim` is a whole finder that owns its event loop — this is one screen in an app |
| Project search input | `ratatui-textarea` | **adopt** | The find field is a one-ish line with editing; see phase 2's composer |
| Text viewer, syntax colour | `syntect` 5.3 (already in the lockfile) for scopes; `tui-syntax-highlight` as the code-block reference | **adopt** (syntect), **adapt** (highlighting) | The window renderer already proves the `syntect scope → token` path; the TUI maps scopes to *its* tokens, not syntect's themes. `tui-syntax-highlight` advertises syntect themes — borrowed for shape, not imported. The in-app viewer is the preview pane; reading whole is `bat`'s job (delegation line) |

### Phase 1 (option) — Tasks board

| Part | Candidates | Verdict | Why |
|---|---|---|---|
| Columns and cards | built-in `List`/`Table`; `tui-cards` and `kanban-tui` as layout references | **write** on core | The board is host data end to end, and `kanban-tui`'s real trick is the one Ubiq already has: task state lives in a service crate, not the UI. Draw columns as lists of card rows |
| Field-at-a-time task panel | forms row (below) | see forms | One task, one field at a time — three form widgets, not a board crate |

### Phase 2 — agents and conversations

| Part | Candidates | Verdict | Why |
|---|---|---|---|
| Composer | `ratatui-textarea` (maintained fork of `tui-textarea`, published against ratatui 0.30) | **adopt** | Emacs keys, undo/redo, soft wrap, selection, yank — a mini editor for the composer and the palette's one-line input alike. The composer is a field, not a file |
| Transcript | built-in `List`/`Paragraph` + `Scrollbar` | **write** on core | The fold is host-owned (`G342` move at phase 3); no crate adds the fold, and the transcript is one pane's text |
| Tool blocks and their diffs | write a gutter over `Paragraph`; `gitdiff-tui`, `hunk-tui-diff` as layout references | **write** | A diff *inside a tool block* is one pane's text, not a review application |
| Permission prompts, pending-ask answer, delegate list, harness picker | forms row; built-in `List` | **core** + forms | The ask is a form with one answer and a "needs you" strip; the delegates are a list |
| Lifecycle glyph and activity | write token glyphs; `throbber-widgets-tui` only if a later phase wants a real spinner | **write** | The lifecycle is a one-cell glyph; the window draws it with its own tokens, and so does the TUI |
| Context and token readouts | built-in `Paragraph` | **core** | Label/value text again |

### Phase 3 — panes

| Part | Candidates | Verdict | Why |
|---|---|---|---|
| The pane itself | the grid holder under `vendor/gpui-terminal/` (`alacritty_terminal` 0.25.1) | **write** the walk | The seed's §5 is adopted whole. The ecosystem's own PTY/terminal `tui-*` widgets parse with a different VT crate; a pane would behave differently in the two interfaces and every emulator bug would be reproduced twice. Their grid→buffer trick is worth taking; their parser is not |
| Tiling behind the leader | built-in `Layout` constraints | **write** | Split, zoom, swap over host streams with a keyboard lease — no crate models that, because no crate owns the panes' back end |
| Redraw budget, dirty flags | the seed's §5 frame loop | **write** | A cap and a coalescer are app logic, not a widget |

### Chore screens and shared chrome

| Part | Candidates | Verdict | Why |
|---|---|---|---|
| Logs | write a `Paragraph` window over the ring + `ansi-to-tui` | **write** | The console is a filter over one-line rows with two controls. `tui-logger` is a receiver over its own sinks; this is a host family |
| Notifications | built-in `List`, level-coloured | **core** | Level colour maps to tokens; no crate needed |
| Forms — settings, mute rules, task fields, KB source add/edit | `tui-prompts`, `tui-input`, `ratatui-widgets` `Button`/`Checkbox`; `ratiform` if a multi-field form appears | **adapt** | Small form fields, all theme-mapped; adopt `ratiform`-style field state only where a screen earns it |
| Popups and confirm | `tui-popup` for a centered modal if a screen ever needs one; otherwise a framed `Paragraph` | **write** unless earned | The release scope's screens have no modal yet; writing a framed block is the cheap default |
| Context-menu rows | none | — | The scorecard makes every context row a palette entry; there is no separate menu surface |
| Clipboard | OSC 52 to the outer terminal | **write** | The seed's §5 line: over SSH there is exactly one answer and it is an escape sequence, not a crate |

### The kept-out list's reversal paths

| Kept out | If reopened, the crate is | Note |
|---|---|---|
| Markdown | `tui-markdown` 0.3 | pulldown-cmark 0.13 + syntect + `ansi-to-tui` — all already resolve in the lockfile, so this is the cheapest reversal. Default style sheet is adapted to tokens |
| Editing | a form field is `ratatui-textarea`; a *file* is `$EDITOR`'s — `ratatui-code-editor` and the tree-sitter road exist and are declined | The delegation line as a rule: Ubiq never ships an edit buffer. When editing returns, it is stored in a key that opens the file in the editor the user already has |
| Charts, plans, graphs | `ratatui-plt` (scientific plotting) and the core `Chart` family | Both exist and are deliberately unused: the modes that draw them were scored `no` or "no bars" |

## What is bought vs written, and why the division

Adopt (four): **nucleo-matcher**, **ratatui-textarea**, **tui-tree-widget**, **ansi-to-tui** —
each is maintained, small, sits on ratatui 0.30, and buys a shaped interaction (fuzzy pick, multi-line
edit, collapse/select, ANSI→style) that Ubiq would otherwise reinvent. Syntect and pulldown-cmark are
already resolving and are adopted by touch.

Everything else is written on core widgets, for one of five reasons:

- **The part is a host stream, not a buffer.** The transcript, the logs, the notifications and the
  pane surface come from the host per-asker; a widget that owns its own content would hold a second
  copy that drifts. List, Paragraph and Layout draw them.
- **The part is already a core widget.** Control, readouts, status, search results — the scorecard's
  "text that happens to be drawn with a GPU" applies to the widgets too.
- **Buying would fork behavior.** The pane's emulator (§5) and anything near a VT stream. One grid,
  one parser, one set of bugs to chase.
- **SSH has no better answer.** The clipboard is the escape sequence; there is no crate to adopt.
- **The medium already ships it, so nothing is bought or written.** Editing, git, prose, paging — the
  delegation table above sends those to incumbents; a widget would be a port dressed as a purchase.

## Housekeeping

- Every buy lands in `crates/ubiq-tui` behind the `tui` feature. `crates/agent-manager` is untouched
  by all of this except the release scope's phase-2 question of whether the key loop borrows its stub's
  or the window's harness story.
- The narrow-build checks are the seed's §7: `just tui` (no window/host crate in the tree) and the
  terminal-only build in CI at phase 4.
- If a widget insists on a second crossterm major, ratatui's `crossterm_0_28` feature flag is the
  reconciliation — the flag exists precisely for this.
- **No register rows here.** Adopting a crate is a `Cargo.lock` change under `D164`'s "own crate"
  shape, not a rule change. The only rows the set proposes are the ones the release scope already
  reserves.

## Related docs

- [`ratatui-tui-proposal.md`](./ratatui-tui-proposal.md) — whose §5 emulator line this map obeys, and whose §7 defines the crate it buys into
- [`tui-release-scope.md`](./tui-release-scope.md) — the screens these crates draw, in release order
- [`mode-fit-analysis.md`](./mode-fit-analysis.md) — the include lists this map draws against, and the "no bars" line it inherits
- [`decisions.md`](../../tech/decisions.md) — `D162` (the `ui` feature split this extends) and the reserved `D164`