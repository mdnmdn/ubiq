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

Eight skills under `.claude/skills/` hold the working reference for each area — the rules, the module
maps and the recipes, gathered from the code and the documents it is anchored to. Read the one your
task lands in **before** the documents, and it will tell you which of those you still need. Claude
Code loads them by name; any other harness reads `.claude/skills/<name>/SKILL.md` directly, and the
`reference/` files beside it hold the exhaustive material.

| Skill | Read it when you are touching |
|---|---|
| `ubiq-ui` | Any screen, panel, modal, control, colour or layout in `crates/ubiq` |
| `ubiq-agents` | A harness launch, accounts, profiles, permission modes, a `ConvUpdate`, the chat panel or the agents columns |
| `ubiq-host` | The coordinator, a pane's lifecycle, the project catalogue, a store or a worker thread |
| `ubiq-transport` | A message, the bus and its routing, the framing, an id type, a log subsystem, a remote host |
| `ubiq-files` | The file, search, index or watch workers, the explorer tree, the picker or the search panel |
| `ubiq-web-panels` | A web panel — a tenant, the bridge, the chrome page and its CSP, the vendor mirror |
| `ubiq-icons` | An icon — the registry, the SVG spec, the draw-and-review loop |
| `ubiq-docs` | Finishing any change — the same-commit duty, the frontmatter, the lint, `Dnn` and `Gnn` |

A skill is a reference, not an authority: where one disagrees with the document it was built from,
the document wins and the skill is what you fix.

## Leaving things in order

Your change updates the documents it touched, in the same commit — `_docs/_meta/authoring.md` says
which ones and how. `just docs-touched` names them from your diff. Never create, move or split a
document; file it instead.

## IMPORTANT Directives

- Agressively use subagents, including cheaper models, for all the grunt work, delegate in every
  occasion it makes sense, do not burn the main context on it.
   - Only main agents could spawn subagents
   - Unless the task is trivial, the main agent should act as a pm/coordinator an spawn subagents to performs the tasks. 
   - create subagents with smaller model according to the task (eg sonnet, haiku), use big model for more reasoning tasks
   - max 3 subagents running
- Follow the project conventions and existing patterns if possible
- Be coincise and efficient
- Say what the tree actually does. A `status: draft` document describes a settled design; the gaps
  between it and the code are rows in `_docs/backlog.md`, not hedges in prose.
- Keep it simple as possible, don't overengineer
- it is possible that other people and agents are working in the same worktree: don't stash/revert updates you don't known about
- Use tools and scripts in a smart way: always use the most efficient way to do the operation: if it's cheper using 
  default tools, use them, if it's better or safer using bash or script use them, if an operation is batched or complex
  evaulate to create a new `_tools`
- USE THE DEFAULT TOOLS for targeted updates, do not run scripts
- **NEVER edit code with a script for a single edit.** No `python3 - <<'EOF'`, no `sed -i`, no
  `perl -pi`, no heredoc rewriting a file to change one place. Use Edit/Write.
  A script *is* the smart approach for many edits at once — several hunks across a file, or the same
  change across many files — and a batched tree-wide operation belongs in `_tools`.
  This applies to subagents too: say it in their prompt.
- If you have a technical problem compiling or other automated task notify it and envetually do other remaing activities, don't use
- be brief

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
- **Claude Code's native `stream-json` bridge is never removed.** `io/jsonl.rs` and the
  `claude-code` harness stay, whatever else moves to ACP: `claude-code-acp` is a sibling, not a
  successor, and the native path is the one with no adapter process and no npm dependency between
  Ubiq and the model. It also states three things no ACP adapter does — the per-model context
  window, a delegate's spend split out of the turn's, and the full token breakdown. See `D95`.
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
  it. **Closing the tab only detaches**: the harness keeps running, its screen stays live, and
  reopening a panel for that pane reattaches to it. `Kill harness` is what ends one (`D103`).
- **The coordinator's reader is never blocked by a slow UI** — that stalls the harness itself.
- **Accounts carry credential references, never credential material.**
- **The word "session" means two things.** Ubiq's session is a named grouping of panes with a
  folder; the library's session is a resumable harness conversation. Say which one you mean.

## Commands

`just` runs everything — `dev`, `build`, `check`, `clippy`, `fmt`, `test`, `verify`, `am`, `core`,
`host`, `ui`,
and the `docs-*` recipes. `just verify` is what a change has to pass. Detail in
`_docs/tech/operations.md`.
