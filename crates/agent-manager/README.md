# agent-manager

> One config, every harness.

`agent-manager` is a CLI + TUI (Rust, [ratatui](https://ratatui.rs)) for managing
AI coding agent configuration. Describe your rules, policies, skills, MCP
servers, and sub-agents **once** in a single TOML file and project that
configuration onto every supported harness — Claude Code, Codex, GitHub
Copilot, Gemini CLI, opencode, and more — so behavior stays consistent across
tools and drift is impossible.

## The problem

Most AI coding harnesses (Claude Code, Codex, GitHub Copilot, opencode, ...)
each invent their own way to store:

- project / user rules
- reusable skills
- MCP server definitions
- sub-agent definitions

The same setup has to be written four different times in four different
formats, in four different directories. Drift is inevitable.

## The solution

`agent-manager` is the single source of truth that keeps them in sync. A
project opts in by dropping a `.agent-manager.toml` at its root:

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

## Goals

1. **Author once.** A single TOML file describes rules, skills, MCP, agents.
2. **Apply everywhere.** The same file is rendered into the right shape for
   each enabled harness and written to the right location on disk.
3. **Drift-free.** `agent-manager sync` is idempotent: re-running it produces
   no changes once you're up to date.
4. **Inspectable.** `agent-manager status` shows, per harness, what is
   installed, what is divergent, and what is unsupported.
5. **Two front ends.** A `clap`-driven CLI for scripts and a `ratatui` TUI
   for interactive exploration.

## Supported harnesses

| id            | display name      | status     |
|---------------|-------------------|------------|
| `claude-code` | Claude Code       | supported  |
| `codex`       | Codex             | supported  |
| `copilot`     | GitHub Copilot    | supported  |
| `gemini`      | Gemini CLI        | supported  |
| `opencode`    | opencode          | supported  |

See [`_docs/harness/`](_docs/harness/) for per-harness details (file
locations, supported features, format quirks).

## Install

From source (requires Rust 1.75+):

```bash
cargo install --path .
```

Or build the repo directly:

```bash
cargo build --release
./target/release/agent-manager --help
```

## Usage

```bash
# Show what would change for every enabled harness
agent-manager status

# Render the unified config onto every enabled harness on disk
agent-manager sync

# Launch the interactive TUI
agent-manager tui
```

## Project layout

```
agent-manager/
├── AGENTS.md              # contributor + agent guidelines
├── Cargo.toml             # library + binary in one package
├── LICENSE                # MIT
├── README.md              # this file
├── _docs/                 # design + per-harness notes (humans)
│   ├── architecture.md
│   ├── config-format.md
│   ├── project-structure.md
│   └── harness/           # one file per supported harness
│       ├── claude-code.md
│       ├── codex.md
│       ├── copilot.md
│       └── opencode.md
└── src/
    ├── lib.rs             # crate root (forbids unsafe)
    ├── main.rs            # thin binary entry point
    ├── cli.rs             # clap subcommands
    ├── config.rs          # unified, harness-agnostic config model
    ├── harness.rs         # knowledge of each supported harness
    ├── project.rs         # discover + load a project
    ├── sync.rs            # project unified config -> concrete harnesses
    └── tui.rs             # ratatui front end
```

All real logic lives in the library (`src/lib.rs`); `src/main.rs` is a thin
shim that parses args, dispatches, and returns. CLI subcommands, TUI, and the
sync engine are all library modules so they can be reused and tested.

## Conventions for contributors

- **One source of truth per concept.** If a rule lives in the unified config,
  it must not be hand-edited in a harness directory.
- **No `unsafe`.** Enforced via `#![forbid(unsafe_code)]` in `src/lib.rs`.
- **Module-level docs.** Every public module has a `//!` header explaining
  what it owns and how it fits in.
- **All real logic in the library.** `src/main.rs` stays under ~20 lines.

See [`AGENTS.md`](AGENTS.md) for the full contributor guide.

## Status

Pre-alpha. The library skeleton is in place; the sync engine and TUI are
stubs. Next milestones:

- [ ] `status` / `inspect` commands reading real harness directories
- [ ] sync engine: rules + skills round-trip
- [ ] sync engine: MCP server rendering per harness
- [ ] sync engine: sub-agents rendering per harness
- [ ] ratatui TUI

## License

[Sustainable Use License](../../LICENSE) (same as the Ubiq workspace).
