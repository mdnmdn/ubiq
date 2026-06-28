# agent-manager

> Unified configuration manager for AI coding agent harnesses.
> One config, every harness.

`agent-manager` is a CLI + TUI (Rust, [ratatui](https://ratatui.rs)) that lets
you describe **one** configuration for an AI coding agent — rules, policies,
skills, MCP servers, sub-agents — and project it onto **every** supported
harness so the behaviour stays consistent across tools.

## Why

Most AI coding harnesses (Claude Code, Codex, GitHub Copilot, opencode, ...)
each invent their own way to store:

- project / user rules
- reusable skills
- MCP server definitions
- sub-agent definitions

The result: the same setup has to be written 4 different times in 4 different
formats, in 4 different directories. Drift is inevitable. `agent-manager` is
the single source of truth that keeps them in sync.

## Goals

1. **Author once.** A single TOML file describes rules, skills, MCP, agents.
2. **Apply everywhere.** The same file is rendered into the right shape for
   each enabled harness and written to the right location on disk.
3. **Drift-free.** `agent-manager sync` is idempotent: re-running it produces
   no changes once you're up to date.
4. **Inspectable.** `agent-manager status` shows, per harness, what is
   installed, what is divergent, and what is unsupported.
5. **Two front ends.** A `clap`-driven CLI for scripts and a `ratatui` TUI for
   interactive exploration.

## Non-goals

- Talking to the harnesses at runtime (we only manage their config files).
- Acting as an MCP server / client ourselves.
- Being a replacement for any single harness — we only orchestrate them.

## Repository layout

```
agent-manager/
├── AGENTS.md              # this file
├── Cargo.toml             # library + binary in one package
├── _docs/                 # design + per-harness notes (humans)
│   ├── architecture.md
│   ├── config-format.md
│   ├── project-structure.md
│   ├── harness/           # one file per supported harness
│   │   ├── claude-code.md
│   │   ├── codex.md
│   │   ├── copilot.md
│   │   ├── gemini.md
│   │   └── opencode.md
│   └── reference/         # external-system reads (cite refs/ submodules)
│       └── multica.md
├── refs/                  # external projects as git submodules (reference only)
│   └── multica/           # git@github.com:multica-ai/multica.git
└── src/
    ├── lib.rs             # crate root
    ├── main.rs            # thin binary entry point
    ├── cli.rs             # clap subcommands
    ├── config.rs          # unified, harness-agnostic config model
    ├── harness.rs         # knowledge of each supported harness
    ├── project.rs         # discover + load a project
    ├── sync.rs            # project unified config -> concrete harnesses
    └── tui.rs             # ratatui front end
```

The library in `src/lib.rs` owns all real logic. `src/main.rs` is intentionally
a thin shim: parse args, dispatch, return. Everything else (CLI subcommands,
TUI, sync engine) is a library module so it can be reused and tested.

## Unified config at a glance

A project opts in by dropping a `.agent-manager.toml` at its root:

```toml
[project]
name = "my-app"
description = "Internal admin tool."

[[rules]]
id = "no-secrets"
title = "Never log secrets"
body = "rules/no-secrets.md"

[[skills]]
id = "agent-browser"
path = "skills/agent-browser"

[[mcp]]
id = "browser"
[mcp.transport]
type = "stdio"
command = "npx"
args = ["-y", "@agent-browser/mcp"]

[[agents]]
id = "reviewer"
path = "agents/reviewer.md"
```

`agent-manager sync` walks the enabled harnesses and writes the right files
in the right places for each one.

## Supported harnesses

| id            | display name      | status     |
|---------------|-------------------|------------|
| `claude-code` | Claude Code       | supported  |
| `codex`       | Codex             | supported  |
| `copilot`     | GitHub Copilot    | supported  |
| `gemini`      | Gemini CLI        | supported  |
| `opencode`    | opencode          | supported  |

### Reference harnesses (documented, not yet sync targets)

These have a doc under `_docs/harness/` but **no sync renderer yet**. They are
characterised primarily from their observed non-interactive runtime contract
(launch flags, output stream protocol, model/MCP/skill injection seams), with
their native config surface marked "Not documented" where unverified.

| id            | display name      | binary / mode             | status     |
|---------------|-------------------|---------------------------|------------|
| `cursor`      | Cursor Agent      | `cursor-agent` (stream-json) | reference  |
| `codebuddy`   | CodeBuddy         | `codebuddy` (stream-json, Claude-compatible) | reference |
| `antigravity` | Antigravity       | `agy` (text + log scrape) | reference  |
| `openclaw`    | OpenClaw          | `openclaw agent` (json)   | reference  |
| `pi`          | pi                | `pi --mode json`          | reference  |
| `hermes`      | Hermes            | `hermes acp` (ACP)        | reference  |
| `kimi`        | Kimi CLI          | `kimi acp` (ACP)          | reference  |
| `kiro`        | Kiro              | `kiro-cli acp` (ACP)      | reference  |
| `qoder`       | Qoder             | `qodercli --acp` (ACP)    | reference  |

See [`_docs/harness/`](_docs/harness/) for the per-harness details
(file locations, supported features, format quirks). The required
structure for every doc in that directory is defined in
[`_docs/harness/structure.md`](_docs/harness/structure.md). For a system-level
view of how a real orchestrator drives all of these harnesses, see
[`_docs/reference/multica.md`](_docs/reference/multica.md).

## Build & run

```bash
cargo build
cargo run -- status
cargo run -- sync
cargo run -- tui
```

## Conventions for contributors

- **One source of truth per concept.** If a rule lives in the unified config,
  it must not be hand-edited in a harness directory.
- **No `unsafe`.** Enforced via `#![forbid(unsafe_code)]` in `src/lib.rs`.
- **Module-level docs.** Every public module has a `//!` header explaining
  what it owns and how it fits in.
- **All real logic in the library.** `src/main.rs` stays under ~20 lines.

## Status

Pre-alpha. The library skeleton is in place; the sync engine and TUI are
stubs. The next milestones are:

- [ ] `status` / `inspect` commands reading real harness directories
- [ ] sync engine: rules + skills round-trip
- [ ] sync engine: MCP server rendering per harness
- [ ] sync engine: sub-agents rendering per harness
- [ ] ratatui TUI
