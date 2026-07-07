# agent-manager

> A wrapper for a running AI agent harness.
> Configure once, launch anywhere.

> **⚠️ Direction change (in progress).** `agent-manager` is pivoting from a
> *config-sync tool* to an *agent-runtime wrapper*. Instead of running `claude`
> / `codex` directly, you run them **through** `agent-manager` (CLI name `am`),
> which injects skills, MCP servers, an account, initial instructions, and hooks
> into an **ephemeral per-run config** and launches the real harness.
>
> - The **target design** lives in [`_docs/target/`](_docs/target/) — start at
>   [`_docs/target/README.md`](_docs/target/README.md).
> - The **migration path** from today's code is in
>   [`_docs/transition-plan.md`](_docs/transition-plan.md).
> - The **previous** (config-sync) design is archived in
>   [`_docs/old/`](_docs/old/) for reference.
>
> The rest of this file still reflects the older framing except where noted;
> when in doubt, the `target/` docs win.

`agent-manager` is a CLI + library (Rust) that **wraps a running agent
harness**. You run `am claude --mcps postgres,figma --skills web-designer` and
it composes the run — pulling skills/MCPs from a catalog, selecting an account,
seeding instructions — into a throwaway config directory, then launches the real
harness against it. It has two modes: a **CLI** for the terminal and a **library**
for embedding in bigger tools (e.g. the Ubiq multiplexer).

## Why

Most AI coding harnesses (Claude Code, Codex, GitHub Copilot, opencode, ...)
each invent their own way to store skills, MCP servers, accounts, and
instructions — and each stores them *globally*, so every run drags in every tool
you ever installed. There is no clean way to say "run *this* agent with *these*
skills and *that* account, reproducibly, without touching my global config".

`agent-manager` is that missing layer: a wrapper that composes a run from a
catalog and launches the harness against an ephemeral config, leaving the user's
real config untouched.

## Goals

1. **Compose a run.** Inject skills, MCPs, an account, initial instructions, and
   hooks from a catalog into one launch.
2. **Ephemeral & non-invasive.** Provision a throwaway config dir; never write
   to the user's real `~/.claude` etc. during a run.
3. **Reproducible.** A `RunSpec` (flags + settings + catalog) fully determines
   what the agent sees.
4. **Two modes.** A `clap` CLI for the terminal and a front-end-agnostic library
   for embedding, with optionally abstracted I/O (passthrough / ACP / JSONL /
   AG-UI).
5. **Any harness, any account.** The same flags wrap Claude Code, Codex,
   opencode, … and switch accounts without a re-login dance.

## Non-goals

- Being a terminal multiplexer / emulator (that's Ubiq — `am` is a library it
  embeds).
- Being a secrets manager (accounts inject *references*, not secret material).
- Being an MCP server/client of its own (except hosting an embedder's
  *in-process* MCP in lib mode).
- Config-*sync* into the user's real dirs as the primary purpose — retired; see
  [`_docs/old/`](_docs/old/).

## Repository layout

```
agent-manager/
├── AGENTS.md              # this file
├── Cargo.toml             # library + binary in one package
├── _docs/                 # design + per-harness notes (humans)
│   ├── target/            # ⭐ the design we are building toward (start here)
│   │   ├── README.md      #    index
│   │   ├── overview.md    #    vision, responsibilities, two modes
│   │   ├── architecture.md#    runtime pipeline, RunSpec, provisioner, modules
│   │   ├── cli.md         #    the `am` command surface
│   │   ├── registry.md    #    the MCP/skill catalog
│   │   ├── io-modes.md    #    passthrough / ACP / JSONL / AG-UI
│   │   ├── mcp-as-skill.md#    expose an MCP as a skill
│   │   └── roadmap.md     #    phased plan (P1 → P2 → P3)
│   ├── transition-plan.md # migration from today's code to Phase 1
│   ├── old/               # archived config-sync design (superseded)
│   ├── harness/           # per-harness runtime contracts (current, authoritative)
│   │   ├── claude-code.md
│   │   ├── codex.md
│   │   ├── copilot.md
│   │   ├── gemini.md
│   │   └── opencode.md
│   └── reference/         # external-system reads (cite refs/ submodules)
│       └── multica.md
├── refs/                  # external projects as git submodules (reference only)
│   └── multica/           # git@github.com:multica-ai/multica.git
└── src/                   # CURRENT skeleton (config-sync era; see transition-plan)
    ├── lib.rs             # crate root
    ├── main.rs            # thin binary entry point
    ├── cli.rs             # clap subcommands (to be replaced: `am <harness>` + `am catalog`)
    ├── config.rs          # resource types (Skill/McpServer/… — survive the pivot)
    ├── harness.rs         # per-harness knowledge (to become the Harness trait)
    ├── project.rs         # discover + load a project (to become settings discovery)
    ├── sync.rs            # renderers (to become the ephemeral-config provisioner)
    └── tui.rs             # ratatui front end (parked)
```

The library in `src/lib.rs` owns all real logic. `src/main.rs` is intentionally
a thin shim: parse args, dispatch, return. Everything else is a library module
so it can be reused (lib mode) and tested. For how each current `src/` file is
repurposed, see [`_docs/transition-plan.md`](_docs/transition-plan.md).

## How a run works (target)

Instead of syncing config files, `agent-manager` composes and launches a run:

```bash
am claude --mcps postgres,figma --skills web-designer --safe
```

```
flags + settings + catalog  ─▶ resolve ─▶ RunSpec ─▶ provision ─▶ (isolate) ─▶ run
                                                        │                         │
                              ephemeral config dir ◀────┘        real harness ◀───┘
                              (never the user's ~/.claude)         (passthrough tty
                                                                    or abstracted I/O)
```

The full model — `RunSpec`, the provisioner (the repurposed old sync renderer),
the `Harness` trait, and the module layout — is in
[`_docs/target/architecture.md`](_docs/target/architecture.md).

## Supported harnesses

| id            | display name      | status     |
|---------------|-------------------|------------|
| `claude-code` | Claude Code       | supported  |
| `codex`       | Codex             | supported  |
| `copilot`     | GitHub Copilot    | supported  |
| `gemini`      | Gemini CLI        | supported  |
| `opencode`    | opencode          | supported  |

### Reference harnesses (documented, not yet wrapped)

These have a doc under `_docs/harness/` but **no `Harness` implementation yet**.
They are characterised primarily from their observed non-interactive runtime
contract (launch flags, output stream protocol, model/MCP/skill injection
seams) — which is exactly what the target design's provisioner needs — with
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

The current binary still exposes the old (stub) subcommands; the target `am`
surface (`am <harness>`, `am catalog …`) is not built yet. See
[`_docs/target/cli.md`](_docs/target/cli.md) for where it is headed.

```bash
cargo build
cargo run -- status        # (legacy stub)
```

## Conventions for contributors

- **The user's real harness config is read-only during a run.** A run writes
  only to the ephemeral config dir; only `catalog import` reads `~/.claude` etc.
- **`RunSpec` is the boundary.** Resolve produces it; provision/run consume it.
- **Front-end-agnostic core.** No `clap`/terminal types below `cli/`; the core
  must build with `--no-default-features` for lib mode.
- **No `unsafe`.** Enforced via `#![forbid(unsafe_code)]` in `src/lib.rs`.
- **Module-level docs.** Every public module has a `//!` header explaining
  what it owns and how it fits in.
- **All real logic in the library.** `src/main.rs` stays under ~20 lines.

## Status

Pre-alpha, **mid-pivot**. The config-sync skeleton is in place but largely
stubbed; the agent-runtime design is documented and not yet implemented. The
plan and next milestones are in
[`_docs/transition-plan.md`](_docs/transition-plan.md). Phase-1 headline:

- [ ] core model (`RunSpec`) + filesystem catalog (`am catalog ls`)
- [ ] settings + resolve (flags/config → `RunSpec`)
- [ ] `Harness` trait + Claude provisioner (`am claude --print-config`)
- [ ] PTY passthrough runner (`am claude …` launches for real)
- [ ] `am catalog import` (ingest `~/.claude` / `~/.agent`)
