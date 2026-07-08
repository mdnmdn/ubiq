# agent-manager

> A wrapper for a running AI agent harness.
> Configure once, launch anywhere.

> **Direction change — Phase 1 landed.** `agent-manager` has pivoted from a
> *config-sync tool* to an *agent-runtime wrapper*. Instead of running `claude`
> / `codex` directly, you run them **through** `agent-manager` (CLI name `am`),
> which injects skills, MCP servers, and (P2) an account/instructions/hooks into
> an **ephemeral per-run config** and launches the real harness.
>
> **Phase 1 is implemented** for Claude Code end-to-end: `am claude --mcps … --skills …`
> resolves a run, provisions a throwaway config dir, and launches the real
> harness through a PTY, propagating its exit code — plus a filesystem catalog
> (`am catalog ls|show|path|import`). See the Status section below.
>
> - The **target design** lives in [`_docs/target/`](_docs/target/) — start at
>   [`_docs/target/README.md`](_docs/target/README.md).
> - The **migration record** (what each old `src/` file became) is in
>   [`_docs/transition-plan.md`](_docs/transition-plan.md).
> - The **previous** (config-sync) design is archived in
>   [`_docs/old/`](_docs/old/) for reference.
>
> codex / opencode / … are not wrapped yet — they are added by writing more
> `Harness` impls, no core change. When in doubt, the `target/` docs win.

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
└── src/                   # Phase-1 implementation (see transition-plan for history)
    ├── lib.rs             # crate root (#![forbid(unsafe_code)])
    ├── main.rs            # thin binary entry point → cli::run()
    ├── config.rs          # resource types (Skill/McpServer/McpTransport)
    ├── spec.rs            # RunSpec + McpRef/SkillRef/ConfigStrategy/IoModes/Policy (core)
    ├── settings.rs        # discover + load the am.toml/.yaml settings file (core)
    ├── resolve.rs         # (flags + settings + catalog) → RunSpec, replace-by-default (core)
    ├── registry/          # the catalog (core)
    │   ├── mod.rs         #   Registry trait, entries, OverlayRegistry, root resolution
    │   ├── fs.rs          #   FsRegistry (catalog.toml + mcp/*.json + skills/*/)
    │   └── import.rs      #   read-only ingest of ~/.claude, ~/.agent, project dirs
    ├── harness/           # the Harness trait + impls (core)
    │   ├── mod.rs         #   Harness trait, Launch, IoSupport, resolve()/all()
    │   └── claude.rs      #   Claude Code provisioner (CLAUDE_CONFIG_DIR bridge)
    ├── provision.rs       # RunSpec → ephemeral config dir + Launch (core)
    ├── run.rs             # PTY spawn/supervise + exit-code + cleanup (feature: pty)
    ├── io/                # I/O bridging (feature: pty)
    │   ├── mod.rs
    │   └── passthrough.rs #   raw-tty pump (SIGWINCH resize, cooked-mode restore)
    ├── cli/               # the `am` command surface (feature: cli)
    │   ├── mod.rs         #   dispatch: reserved words vs `am <harness>`
    │   ├── run.rs         #   `am <harness> [flags] [-- passthrough]`
    │   └── catalog.rs     #   `am catalog ls|show|path|import`
    └── tui.rs             # ratatui front end (parked, feature: tui)
```

The library in `src/lib.rs` owns all real logic; `src/main.rs` is a thin shim.
Modules marked **(core)** build with `--no-default-features` for lib mode; the
runner (`run`/`io`) and CLI are feature-gated. For how each old `src/` file was
repurposed (config-sync → wrapper), see
[`_docs/transition-plan.md`](_docs/transition-plan.md).

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

| id            | display name      | status                         |
|---------------|-------------------|--------------------------------|
| `claude-code` | Claude Code       | **wrapped** (P1, `Harness` impl) |
| `codex`       | Codex             | documented (`Harness` impl TBD) |
| `copilot`     | GitHub Copilot    | documented (`Harness` impl TBD) |
| `gemini`      | Gemini CLI        | documented (`Harness` impl TBD) |
| `opencode`    | opencode          | documented (`Harness` impl TBD) |

Only `claude-code` has a `Harness` implementation today (`src/harness/claude.rs`);
the others have a runtime contract in `_docs/harness/` and become wrappable by
transcribing that doc into a new `Harness` impl — no core change.

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

The `am` surface is live for Claude Code:

```bash
cargo build
cargo run -- claude --print-config          # provision only; show dir + argv + env
cargo run -- claude --mcps postgres --skills web-designer   # launch for real
cargo run -- claude -- --version            # everything after `--` goes to claude
cargo run -- catalog ls                     # list catalog skills + MCPs
cargo run -- catalog import --dry-run       # preview ingest of ~/.claude etc.
```

Testing (the PTY passthrough integration tests want a non-interactive stdin):

```bash
cargo test  -p agent-manager < /dev/null
cargo build -p agent-manager --no-default-features   # core must build without cli/pty
cargo clippy -p agent-manager --all-features -- -D warnings
```

The binary is still built as `agent-manager`; `am` is the intended installed
alias. See [`_docs/target/cli.md`](_docs/target/cli.md) for the full surface.

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

Alpha. **Phase 1 is complete** for Claude Code end-to-end (verified against a
real `claude` launch and a CI-safe fake harness):

- [x] core model (`RunSpec`) + filesystem catalog (`am catalog ls|show|path`)
- [x] settings + resolve (flags/config → `RunSpec`, replace-by-default merge)
- [x] `Harness` trait + Claude provisioner (`am claude --print-config`)
- [x] PTY passthrough runner (`am claude …` launches for real, exit code propagated)
- [x] `am catalog import` (read-only ingest of `~/.claude` / `~/.agent` / project dirs)

Next (P2): accounts + `am account`, initial prompt/instructions, more `Harness`
impls (codex/opencode via JSONL/ACP), in-process MCP for lib mode. See
[`_docs/target/roadmap.md`](_docs/target/roadmap.md).
