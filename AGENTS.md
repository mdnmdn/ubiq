# AGENTS.md

**Ubiq** — a harness multiplexer. A desktop application that hosts several interactive AI coding
agents (Claude Code, Codex, Gemini CLI, opencode, Copilot CLI) side by side, each in a real terminal
pane, under one window. Think tmux, with the panes specialised for agent harnesses. Rust throughout:
a GPUI application in `crates/ubiq`, and the harness-management library it embeds in
`crates/agent-manager`.

## Finding things

Read `_docs/INDEX.md` first — it names the two or three documents your task needs, and nothing more.

## Leaving things in order

Your change updates the documents it touched, in the same commit — `_docs/_meta/authoring.md` says
which ones and how. `just docs-touched` names them from your diff. Never create, move or split a
document; file it instead.

## Directives

- Use subagents, including cheaper models, for search and mechanical work — do not burn the main
  context on it.
- Keep it simple.
- Say what the tree actually does. A `status: draft` document describes a settled design; the gaps
  between it and the code are rows in `_docs/backlog.md`, not hedges in prose.

## Architecture rules

- **The UI and the coordinator talk only through the message set in `crates/ubiq/src/messages.rs`.**
  No direct call, no shared handle, no callback that skips it — even though they share a process.
- **The UI never assumes the pseudo-terminal is local.** No path, no process handle, no file
  descriptor crosses into UI code. A pane is an ID plus a byte stream.
- **The coordinator renders nothing** and has no opinion about layout or colour.
- **Every message carries a pane ID**, including in the single-pane case.
- **Terminal bytes stay opaque.** Ubiq writes no VT parser and no terminal state engine.
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
- **An exited harness leaves its pane** showing its last screen. Nothing disappears from under the
  user.
- **The coordinator's reader is never blocked by a slow UI** — that stalls the harness itself.
- **Accounts carry credential references, never credential material.**
- **The word "session" means two things.** Ubiq's session is a named grouping of panes with a
  folder; the library's session is a resumable harness conversation. Say which one you mean.

## Commands

`just` runs everything — `dev`, `build`, `check`, `clippy`, `fmt`, `test`, `verify`, `am`, `core`,
and the `docs-*` recipes. `just verify` is what a change has to pass. Detail in
`_docs/tech/operations.md`.
