# agent-manager

> A wrapper for a running AI agent harness.
> Configure once, launch anywhere.

> **Phase 3 complete.** `agent-manager` wraps and launches agent harnesses (`claude`,
> `codex`, `opencode`, …) end-to-end with advanced lifecycle support: `am claude --mcps postgres --skills web-designer`,
> `am codex --prompt "…" --hooks validator`, `am session ls|resume`, structured output
> (events/ACP/AG-UI), session history with resume, isolation (isol8), and hooks for lifecycle automation.
> Passthrough PTY (all harnesses) and structured I/O bridges (Claude Code, Codex, opencode, GitHub Copilot) are fully implemented. In-process MCP
> (lib mode) lets embedders inject custom MCPs the harness can call.
>
> **Phase 1 shipped** (Claude Code end-to-end: passthrough PTY, catalog injection, skills/MCPs/roles).
> **Phase 2 shipped** (codex/opencode wrapped, accounts, instructions/prompt, structured I/O, in-process MCP).
> **Phase 3 shipped** (isolation via isol8, session history + resume, output adapters ACP/AG-UI, hooks).
>
> - The **target design** lives in [`_docs/target/`](_docs/target/) — start at
>   [`_docs/target/README.md`](_docs/target/README.md).
> - The **migration record** (what each old `src/` file became) is in
>   [`_docs/transition-plan.md`](_docs/transition-plan.md).
> - The **previous** (config-sync) design is archived in
>   [`_docs/old/`](_docs/old/) for reference.
>
> When in doubt, the `target/` docs win.

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
│   │   ├── grok.md
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
    ├── account.rs         # account catalog + credential-reference injection (core, P2)
    ├── harness/           # the Harness trait + impls (core)
    │   ├── mod.rs         #   Harness trait, Launch, IoSupport, resolve()/all()
    │   ├── claude.rs      #   Claude Code provisioner (CLAUDE_CONFIG_DIR bridge)
    │   ├── codex.rs       #   Codex provisioner (P2, Harness impl)
    │   ├── copilot.rs     #   GitHub Copilot CLI provisioner (Class A, COPILOT_HOME bridge)
    │   ├── grok.rs        #   Grok CLI provisioner (ephemeral-HOME bridge, passthrough-only)
    │   └── opencode.rs    #   opencode provisioner (P2, Harness impl)
    ├── provision.rs       # RunSpec → ephemeral config dir + Launch (core)
    ├── run.rs             # PTY spawn/supervise + exit-code + cleanup (feature: pty)
    ├── io/                # I/O bridging (core: model + bridges; passthrough: pty-gated)
    │   ├── mod.rs         #   neutral AgentInput/AgentEvent model (core)
    │   ├── model.rs       #   AgentInput/AgentEvent/AgentParams (core, P2)
    │   ├── passthrough.rs #   raw-tty pump (SIGWINCH resize, cooked-mode restore; pty)
    │   ├── structured.rs  #   IoBridge trait for harness-neutral structured I/O (core, P2)
    │   ├── jsonl.rs       #   Claude stream-json input bridge (core, P2)
    │   ├── codex.rs       #   Codex JSON-RPC app-server input bridge (core, P2)
    │   ├── opencode.rs    #   opencode NDJSON run input bridge (core, P2)
    │   ├── copilot.rs     #   GitHub Copilot CLI NDJSON output bridge (core)
    │   ├── acp.rs         #   ACP event adapter (core, P3)
    │   └── agui.rs        #   AG-UI event adapter (core, P3)
    ├── mcp/               # in-process MCP hosting (feature: inproc-mcp)
    │   ├── mod.rs         #   McpService trait for embedders (core, P2)
    │   └── server.rs      #   HTTP MCP server for in-process MCPs (feature: inproc-mcp, P2)
    ├── session.rs         # session history + metadata persistence (core, P3)
    ├── isolate.rs         # isol8 sandbox integration (core, P3)
    ├── cli/               # the `am` command surface (feature: cli)
    │   ├── mod.rs         #   dispatch: reserved words vs `am <harness>`
    │   ├── run.rs         #   `am <harness> [flags] [-- passthrough]`
    │   ├── catalog.rs     #   `am catalog ls|show|path|import`
    │   ├── account.rs     #   `am account ls|use|import` (P2)
    │   └── session.rs     #   `am session ls|show|resume` (P3)
    └── tui.rs             # ratatui front end (parked, feature: tui)
```

The library in `src/lib.rs` owns all real logic; `src/main.rs` is a thin shim.
Modules marked **(core)** build with `--no-default-features` for lib mode; `io/passthrough`
and `run` are `pty`-gated; `mcp/server` is feature `inproc-mcp`-gated; CLI is
feature-gated. Core module `io/` is no longer `pty`-gated (structured bridges + neutral model
are core). For how each old `src/` file was repurposed (config-sync → wrapper), see
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
| `codex`       | Codex             | **wrapped** (P2, `Harness` impl) |
| `copilot`     | GitHub Copilot    | **wrapped** (Class A via `COPILOT_HOME`, `Harness` impl; passthrough + structured) |
| `gemini`      | Gemini CLI        | documented (`Harness` impl TBD) |
| `grok`        | Grok CLI          | **wrapped** (passthrough; structured TBD) |
| `opencode`    | opencode          | **wrapped** (P2, `Harness` impl) |

`claude-code`, `codex`, `copilot`, `grok`, and `opencode` each have `Harness`
implementations (`src/harness/{claude,codex,copilot,grok,opencode}.rs`); the
others have a runtime contract in `_docs/harness/` and become wrappable by
transcribing that doc into a new `Harness` impl — no core change. See
[`_docs/harness/harness-implementation-checklist.md`](_docs/harness/harness-implementation-checklist.md)
for the concrete checklist to work through when adding one.

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

The `am` surface is live for Claude Code, Codex, and opencode:

```bash
cargo build
cargo run -- claude --print-config          # provision only; show dir + argv + env
cargo run -- claude --mcps postgres --skills web-designer   # launch for real
cargo run -- claude --prompt "summarize the repo" --io structured  # structured I/O mode
cargo run -- claude --account work --instructions ./system.md    # account + instructions
cargo run -- codex --list-models            # discover the harness's available models
cargo run -- claude --model sonnet          # launch with a specific model
cargo run -- codex --skills reviewer --io structured            # codex with structured I/O
cargo run -- opencode --account personal --io structured        # opencode with account
cargo run -- grok --mcps postgres --skills reviewer             # Grok CLI (passthrough; ephemeral $HOME)
cargo run -- copilot --mcps postgres --skills reviewer          # GitHub Copilot CLI (COPILOT_HOME relocation; real $HOME untouched)
cargo run -- copilot --prompt "summarize the repo" --io structured  # Copilot headless (-p, NDJSON)
cargo run -- claude -- --version            # everything after `--` goes to claude
cargo run -- account ls                     # list available accounts
cargo run -- account use work               # set default account
cargo run -- account login personal --harness codex   # capture a login into a per-account home for reuse
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
alias. The `inproc-mcp` feature enables in-process MCP hosting for lib mode. See
[`_docs/target/cli.md`](_docs/target/cli.md) for the full surface.

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

Alpha. **Phase 1 complete** for Claude Code end-to-end; **Phase 2 complete**; **Phase 3 complete**:

**Phase 1 ✅**
- [x] core model (`RunSpec`) + filesystem catalog (`am catalog ls|show|path`)
- [x] settings + resolve (flags/config → `RunSpec`, replace-by-default merge)
- [x] `Harness` trait + Claude provisioner (`am claude --print-config`)
- [x] PTY passthrough runner (`am claude …` launches for real, exit code propagated)
- [x] `am catalog import` (read-only ingest of `~/.claude` / `~/.agent` / project dirs)

**Phase 2 ✅**
- [x] `am account` commands (ls/use/import); accounts store credential *references*, never secrets
- [x] `--instructions` (seed always-on memory) and `--prompt` (seed initial prompt)
- [x] `Harness` impls for Codex and opencode (both support passthrough and structured I/O)
- [x] neutral `AgentInput`/`AgentEvent` model + `IoBridge` trait (all harnesses)
- [x] structured I/O bridges: Claude (stream-json JSONL), codex (JSON-RPC app-server), opencode (NDJSON run)
- [x] in-process MCP (lib mode): `McpService` trait for embedders, hosted on loopback HTTP MCP endpoint

**Phase 3 ✅**
- [x] isolation (`--isolate[=profile]` via isol8): `src/isolate.rs`, settings template, core-gated
- [x] session history: `am session ls|show|resume`; persistent transcripts + metadata; `--resume <id>` (harness-native)
- [x] output adapters: `--output <events|acp|agui>` on structured runs; stateless best-effort mappers (`src/io/{acp,agui}.rs`)
- [x] hooks: per-run hook selection (`--hooks a,b`); provisioner wires into harness-native slots (Claude/codex/opencode)
- [x] MCP-as-skill schema + stepping stone: `expose = "tools" | "skill"`, `summary`, `--mcp-as-skill` CLI flag, generated SKILL.md pointers (deferred-load mechanism deferred)

See [`_docs/target/roadmap.md`](_docs/target/roadmap.md).
