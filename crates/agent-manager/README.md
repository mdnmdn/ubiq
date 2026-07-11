# agent-manager

> A wrapper for a running AI agent harness — compose a run, launch anywhere.

`agent-manager` (CLI name `am`) runs an agent harness (Claude Code today; Codex,
opencode, … next) **through** a thin wrapper. Instead of `claude`, you run
`am claude --mcps postgres,figma --skills web-designer`: it resolves the run,
provisions a **throwaway per-run config directory** with exactly those skills and
MCP servers, and launches the real harness against it — leaving your real
`~/.claude` untouched.

It has two modes: a **CLI** for the terminal, and a front-end-agnostic **library**
for embedding in bigger tools (e.g. the [Ubiq](../../) multiplexer).

> **Status: Phase 3 complete.** Wraps Claude Code, Codex, and opencode end-to-end
> through a PTY with skill/MCP injection, account selection, initial instructions/prompt,
> structured I/O, session history + resume, isolation (isol8), hooks, and output adapters (ACP/AG-UI).
> See [Status](#status). The design docs live in
> [`_docs/target/`](_docs/target/); the config-sync tool this replaced is archived in
> [`_docs/old/`](_docs/old/).

## The problem

Most AI coding harnesses (Claude Code, Codex, opencode, …) store skills, MCP
servers, accounts, and instructions **globally**, so every run drags in every
tool you ever installed, tied to whichever account you last logged into. There is
no clean way to say "run *this* agent with *these* skills and *that* account,
reproducibly, without mutating my global config."

## The solution

`am` is that missing layer. It composes a run from a **catalog** and launches the
harness against an **ephemeral config dir**:

```bash
# Launch Claude Code with just these MCP servers + skills, in an isolated config.
am claude --mcps postgres,figma --skills web-designer

# Launch with account, initial instructions, and structured I/O.
am claude --account work --instructions ./system.md --io structured

# Launch codex or opencode (both wrapped now).
am codex --skills reviewer --account work
am opencode --prompt "find bugs" --io structured

# Launch Grok CLI (passthrough; ephemeral $HOME isolates ~/.grok).
am grok --mcps postgres --skills reviewer --account xai

# Inspect what would be provisioned, without launching.
am claude --print-config

# Forward flags straight to the harness (everything after `--`).
am claude -- --model opus -p "summarise the repo"
```

Under the hood: `flags + settings + catalog → resolve → RunSpec → provision →
run`. The provisioner writes skills, an `mcp.json`, and (optionally) a permission
policy into a temp dir and points Claude Code at it via `CLAUDE_CONFIG_DIR` +
`--mcp-config … --strict-mcp-config`; the runner spawns the real `claude` in a
PTY, forwards the tty, resizes on `SIGWINCH`, and exits with the child's code.
The user's real `~/.claude` is **never written** during a run.

## The catalog

`--mcps`/`--skills` resolve ids against a catalog (a filesystem store by
default):

```bash
am catalog ls                       # list available skills + MCP servers
am catalog show postgres            # print a resolved definition
am catalog path                     # print the active catalog root
am catalog import                   # read-only ingest of ~/.claude, ~/.agent, …
am catalog import --dry-run         # preview; write nothing
```

Layout of a catalog root (`~/.config/agent-manager/catalog` by default, override
with `--catalog` / `AM_CATALOG`):

```
<catalog-root>/
├── catalog.toml        # optional: inline [[mcp]] definitions
├── mcp/<id>.json       # one file per MCP server (harness-native shape)
└── skills/<id>/SKILL.md
```

A project may add or override entries via `<project>/.agent-manager/catalog`
(project wins on id collision). Full details in
[`_docs/target/registry.md`](_docs/target/registry.md).

## Accounts

Select a credential profile to use with `--account <id>` or set a default in settings:

```bash
am account ls                     # list available accounts
am account use work               # set default account
am account import                 # add accounts from well-known locations
```

Accounts are stored under `~/.config/agent-manager/accounts/` (env: `AM_ACCOUNTS`).
An account holds credential *references*, never secrets: environment variable names
(`api_key_env`, `auth_token_env`), a `base_url`, a credential helper command, and/or
a private `home` directory holding a captured login. A `home`-based login is **seeded**
(copied) into the run's relocated config dir — `am` never overrides the child's `HOME`,
so your toolchain (`nvm`/`mise`/`pyenv`, shell rc, PATH) stays intact.

## Profiles

A **profile** is a named, reusable base — an account, composition defaults, and an
optional isolation policy — that a run draws from, with per-run flags overriding it.
Profiles support **inheritance** (`extends`) at both the defaults and config-overlay
levels.

```bash
am profile ls                                   # list profiles
am profile create work --account work --harness claude --model sonnet
am profile create reviewer --extends work --model haiku   # inherits account/harness
am profile show reviewer                        # print the flattened profile
am profile use work                             # set the default profile

am claude --profile work                        # run with the profile
am claude --profile work --model haiku          # per-run flag overrides the profile
am agent reviewer -- -p "find bugs"             # run a profile as a frozen agent
```

Precedence is **flag > profile > per-harness settings > defaults**. Profiles live under
`~/.config/agent-manager/profiles/<name>/` (env: `AM_PROFILES`) as `profile.toml` plus an
optional `base/<harness>/` config overlay (symlinked into each run, leaf-of-the-chain
wins, never clobbering `am`-managed files). With no `--profile`, an implicit `default`
is used if present; otherwise a bare `am <harness>` reuses your existing login by seeding
it from your real home (zero-config). Ephemeral run dirs are GC'd after
`AM_RUNS_TTL_DAYS` (default 7).

## Settings

Optional settings file (`am.toml` / `agent-manager.toml` / `.am.toml` /
`.agent-manager.toml`, TOML or YAML), discovered by walking up from the CWD to
the git root, then `~/.config/agent-manager/config.toml`:

```toml
catalog = "~/.agent-manager/catalog"

[defaults]                 # applied to every `am <harness>` run
mcps = ["github"]

[harness.claude]           # per-harness defaults
mcps = ["postgres"]

[presets.safe]             # what `--safe` expands to
permission_mode = "restricted"
deny = ["Bash(rm *)", "WebFetch"]
```

Merge is **replace by default**: the highest layer that mentions a key (CLI flag
> per-harness > defaults) wins outright — it does not union. See
[`_docs/target/cli.md`](_docs/target/cli.md).

## Install

From source (Rust 2024 edition toolchain):

```bash
cargo build --release
./target/release/agent-manager --help      # installed alias: `am`
```

## Project layout

```
agent-manager/
├── AGENTS.md              # contributor + agent guide (start here)
├── Cargo.toml             # library + binary; features: cli, pty, inproc-mcp
├── _docs/                 # design docs (target/), harness contracts, archive (old/)
└── src/
    ├── config.rs          # resource types (Skill/McpServer/McpTransport)
    ├── spec.rs            # RunSpec + refs/strategies (core)
    ├── settings.rs        # settings-file discovery + load (core)
    ├── resolve.rs         # flags + settings + catalog → RunSpec (core)
    ├── account.rs         # account catalog + credential-reference injection (core, P2)
    ├── registry/          # the catalog: trait, FsRegistry, overlay, import (core)
    ├── harness/           # Harness trait + implementations (core)
    │   ├── claude.rs      # Claude Code (P1)
    │   ├── codex.rs       # Codex (P2)
    │   ├── grok.rs        # Grok CLI (passthrough; ephemeral-HOME bridge)
    │   └── opencode.rs    # opencode (P2)
    ├── provision.rs       # RunSpec → ephemeral config dir + Launch (core)
    ├── io/                # I/O bridging (core: model + bridges; pty-gated: passthrough)
    │   ├── model.rs       # neutral AgentInput/AgentEvent (core, P2)
    │   ├── structured.rs  # IoBridge trait (core, P2)
    │   ├── jsonl.rs       # Claude stream-json (core, P2)
    │   ├── codex.rs       # Codex JSON-RPC (core, P2)
    │   ├── opencode.rs    # opencode NDJSON (core, P2)
    │   ├── acp.rs         # ACP event adapter (core, P3)
    │   ├── agui.rs        # AG-UI event adapter (core, P3)
    │   └── passthrough.rs # raw PTY (pty-gated)
    ├── mcp/               # in-process MCP (feature: inproc-mcp, P2)
    ├── session.rs         # session history + metadata (core, P3)
    ├── isolate.rs         # isol8 integration (core, P3)
    ├── run.rs             # PTY spawn/supervise (feature: pty)
    └── cli/               # the `am` command surface (feature: cli)
        ├── run.rs         # `am <harness> …`
        ├── catalog.rs     # `am catalog …`
        ├── account.rs     # `am account …` (P2)
        └── session.rs     # `am session …` (P3)
```

Modules marked *(core)* build with `--no-default-features` for lib-mode
embedding. All real logic lives in the library; `src/main.rs` is a thin shim.

## Conventions for contributors

- **The user's real harness config is read-only during a run.** A run writes
  only to the ephemeral config dir; only `catalog import` reads `~/.claude` etc.
- **`RunSpec` is the boundary.** Resolve produces it; provision/run consume it.
- **Front-end-agnostic core.** No `clap`/terminal types below `cli/`; core builds
  with `--no-default-features`.
- **No `unsafe`.** Enforced via `#![forbid(unsafe_code)]`.
- **Module-level docs.** Every public module has a `//!` header.

See [`AGENTS.md`](AGENTS.md) for the full contributor guide.

## Status

Alpha. **Phase 1 complete** for Claude Code; **Phase 2 complete**; **Phase 3 complete**:

**Phase 1 ✅**
- [x] core model (`RunSpec`) + filesystem catalog (`am catalog ls|show|path`)
- [x] settings + resolve (replace-by-default merge)
- [x] `Harness` trait + Claude provisioner (`am claude --print-config`)
- [x] PTY passthrough runner (`am claude …` launches for real)
- [x] `am catalog import`

**Phase 2 ✅**
- [x] `am account` commands; accounts store credential references, never secrets
- [x] `--instructions` and `--prompt` seeding
- [x] `Harness` impls for Codex and opencode (both support passthrough + structured I/O)
- [x] neutral I/O model + bridges for JSONL (Claude), JSON-RPC (codex), NDJSON (opencode)
- [x] in-process MCP for lib mode

**Phase 3 ✅**
- [x] isolation (`--isolate[=profile]` via isol8)
- [x] session history (`am session ls|show|resume`)
- [x] output adapters (`--output <events|acp|agui>`)
- [x] hooks (`--hooks a,b`, wired into harness-native slots)
- [x] MCP-as-skill schema stepping stone (`expose`, `summary`, `--mcp-as-skill`, generated SKILL.md)

Roadmap in [`_docs/target/roadmap.md`](_docs/target/roadmap.md).

## License

[Sustainable Use License](../../LICENSE) (same as the Ubiq workspace).
