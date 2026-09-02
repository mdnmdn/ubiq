# AGENTS.md

**Ubiq** — a harness multiplexer. A desktop application that hosts several interactive AI coding
agents (Claude Code, Codex, Gemini CLI, opencode, Copilot CLI) side by side, each in a real terminal
pane, under one window. Think tmux, with the panes specialised for agent harnesses. Rust throughout:
four crates — `crates/ubiq-proto` (the contract, the bus, the log sink), `crates/ubiq-host` (the
headless host: processes, pseudo-terminals, the project catalogue), `crates/ubiq` (the GPUI
interface), and `crates/ubiq-app` (the binary, the only thing that names both halves) — plus the
harness-management library they embed in `crates/agent-manager`.

## Finding things

Read `_docs/INDEX.md` first — it names the two or three documents your task needs, and nothing more.

## Leaving things in order

Your change updates the documents it touched, in the same commit — `_docs/_meta/authoring.md` says
which ones and how. `just docs-touched` names them from your diff. Never create, move or split a
document; file it instead.

## IMPORTANT Directives

- Agressively use subagents, including cheaper models, for all the grunt work, delegate in every
  occasion it makes sense, do not burn the main context on it.
- Keep it simple.
- Say what the tree actually does. A `status: draft` document describes a settled design; the gaps
  between it and the code are rows in `_docs/backlog.md`, not hedges in prose.
- Use tools and scripts in a smart way: always use the most efficient way to do the operation: if it's cheper using 
  default tools, use them, if it's better or safer using bash or script use them, if an operation is batched or complex
  evaulate to create a new `_tools`

## Architecture rules

- **The UI and the host talk only through the message set in `crates/ubiq-proto/src/messages.rs`.**
  No direct call, no shared handle, no callback that skips it — even though they share a process.
  This is a crate boundary, not a convention: `crates/ubiq` does not depend on `crates/ubiq-host`,
  and `just ui` and `just host` check that nothing reintroduced it.
- **The UI never assumes the pseudo-terminal is local.** No path, no process handle, no file
  descriptor crosses into UI code. A pane is an ID plus a byte stream.
- **The coordinator renders nothing** and has no opinion about layout or colour.
- **Every message carries a pane ID**, including in the single-pane case.
- **Terminal bytes stay opaque.** Ubiq writes no VT parser and no terminal state engine.
- **One host per process**, started by the binary before the first window. A window attaches to it
  and gets a client; pane messages route to the window that owns the pane, project messages reach
  every window.
- **No literal colour outside `crates/ubiq/src/theme.rs`.** Every colour is a token, with a value in
  both palettes.
- **Ubiq never names a harness config path and never hard-codes how to launch one.**
  `crates/agent-manager` owns all of that, and Ubiq embeds it. New harness support is a change
  there, not here.
- **`crates/agent-manager` keeps no UI dependency** and must keep building with
  `--no-default-features` — `just core` is that check.

## Domain rules — what no agent can infer from the code

- **A pane is a terminal, not a text buffer.** Harnesses drive a full screen: alternate screen,
  absolute cursor addressing, raw keystrokes, resize redraw. Rendering their output as a scrolling
  log is wrong at the design level, not the detail level.
- **A resize is incomplete until the harness knows.** Geometry must reach the pseudo-terminal so the
  kernel signals the process. A pane that resizes visually while its harness believes the old size
  is the classic corruption bug.
- **Exactly one pane holds focus** and receives keystrokes. Unfocused panes keep drawing.
- **An exited harness closes its pane.** Typing `exit` or sending EOF (Ctrl+D) takes the tab with
  it; closing the tab is still what kills a harness that has not already ended.
- **The coordinator's reader is never blocked by a slow UI** — that stalls the harness itself.
- **Accounts carry credential references, never credential material.**
- **The word "session" means two things.** Ubiq's session is a named grouping of panes with a
  folder; the library's session is a resumable harness conversation. Say which one you mean.

## Commands

`just` runs everything — `dev`, `build`, `check`, `clippy`, `fmt`, `test`, `verify`, `am`, `core`,
`host`, `ui`,
and the `docs-*` recipes. `just verify` is what a change has to pass. Detail in
`_docs/tech/operations.md`.
