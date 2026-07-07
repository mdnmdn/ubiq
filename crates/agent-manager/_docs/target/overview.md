# Overview — what `agent-manager` is (target design)

## The pivot in one paragraph

`agent-manager` used to be a **config-sync tool**: it wrote your rules/skills/MCP
into each harness's real config directory and stopped. It is now an **agent
runtime wrapper**: it *launches* the harness itself and injects everything the
run needs into an **ephemeral, throwaway configuration** that exists only for
the lifetime of that run. Your `~/.claude` is no longer the target of a sync —
it is, at most, a *source* the catalog can import from. The tool's job moved
from "keep files in sync" to "compose and run an agent".

## The mental model

```
        you                          agent-manager (am)                 the harness
   ┌───────────┐        ┌──────────────────────────────────────┐     ┌──────────────┐
   │ am claude │  ───▶  │ 1. resolve  (flags + config → RunSpec)│     │              │
   │  --mcps … │        │ 2. provision (RunSpec → ephemeral dir)│ ──▶ │   claude     │
   │  --skills…│        │ 3. isolate  (optional isol8 sandbox)  │     │  (real bin)  │
   └───────────┘        │ 4. launch & supervise the process     │ ◀── │              │
        ▲               │ 5. bridge I/O (passthrough | ACP | …) │     └──────────────┘
        └───────────────┤ 6. record the session                 │
        (tty or events) └──────────────────────────────────────┘
```

The harness runs as a real child process against a config directory `am`
generated. The harness has no idea it is being wrapped — it just sees a normal
config folder, a normal set of MCP servers, a normal account. That is the whole
trick: **`am` speaks each harness's native config + launch contract**, so it can
inject anything the harness natively supports without the harness cooperating.

## Responsibilities

The wrapper owns the full "compose a run" surface. In roughly the order they
land during a launch:

| # | Responsibility            | What it means                                                                 | Phase |
|---|---------------------------|-------------------------------------------------------------------------------|-------|
| 1 | **Inject skills & MCPs**  | Pull selected skills/MCP servers from the catalog into the ephemeral config.   | P1    |
| 2 | **Set a custom config folder** | Point the harness at the generated dir (e.g. `CLAUDE_CONFIG_DIR`, `--mcp-config`). | P1 |
| 3 | **Inject initial instructions** | Seed always-on memory / a first prompt for the run.                       | P2    |
| 4 | **Inject the account**    | Choose which of several accounts (e.g. multiple Claude logins) the run uses.    | P2    |
| 5 | **Inject hooks**          | Wire lifecycle hooks into the harness's native hook slots.                      | P2/P3 |
| 6 | **Manage historical sessions** | Persist / list / resume past runs and their transcripts.                  | P2/P3 |
| 7 | **Run in an isolated environment** | Launch inside [isol8](https://github.com/mdnmdn/isol8) for filesystem/network confinement. | P3 |
| 8 | **Inject custom MCP (lib mode)** | Let an embedding program register an *in-process* MCP server the agent can call. | P2/P3 |
| 9 | **Abstract I/O**          | Optionally replace passthrough tty with structured input/output (ACP, JSONL, AG-UI). | P2/P3 |

Each row maps to a module in [`architecture.md`](./architecture.md) and to a
milestone in [`roadmap.md`](./roadmap.md).

## The two modes

### CLI mode

Run an agent directly from the terminal. Configuration is declared as a **mix**
of CLI flags and a `toml`/`yaml` settings file (flags win; see
[`cli.md`](./cli.md)). The available skills and MCP servers come from a
**catalog** whose root is set by flag / env / config (see
[`registry.md`](./registry.md)).

```bash
am claude --mcps postgres,figma --skills web-designer --safe
am catalog ls
am catalog import           # ingest ~/.claude, ~/.agent, … into the catalog
```

The default I/O mode in the CLI is **passthrough**: the agent's tty is wired
straight to your terminal, exactly as if you had run the harness yourself. `am`
only *configures and launches*; the interaction is standard console.

### Lib mode

Embed the crate (`use agent_manager::…`) inside a larger tool — for example the
Ubiq harness multiplexer. The embedder:

- builds a `RunSpec` programmatically instead of parsing flags,
- can register **custom in-process MCP servers** (a library callback the agent
  reaches over stdio/loopback — not a subprocess from the catalog),
- drives the agent through an **abstracted I/O** channel (send input as ACP or
  JSONL; receive output as ACP or AG-UI events) instead of a raw tty.

Lib mode is why the crate must stay front-end-agnostic: the `clap` CLI and any
TUI are optional features layered on top of a core that has no terminal
assumptions. (This mirrors the existing `frontend`/`cli`/`tui` Cargo feature
split.)

## What `am` is *not*

- **Not** a terminal emulator or a multiplexer. That is Ubiq's job; `am` is a
  library Ubiq can embed.
- **Not** a secrets manager. Account injection wires *references* (env var
  names, keyring entries, credential file paths) into the harness's native auth
  slot; the secret material stays in the OS keyring / the user's shell / a
  file the user controls.
- **Not** an MCP server or client of its own — except the *custom in-process
  MCP* an embedder registers in lib mode, which `am` merely hosts and exposes to
  the wrapped agent.
- **Not** (primarily) a config-sync tool anymore. Rendering into the user's real
  `~/.claude` may survive as an optional convenience, but it is no longer the
  purpose. See [`../old/`](../old/).

## Why this is worth doing

- **One command, any harness, any account.** `am claude` and `am codex` take the
  same flags; switching account or harness is a flag, not a re-login dance.
- **Reproducible runs.** A `RunSpec` (flags + config + catalog) fully determines
  what the agent sees. No "works on my machine because my `~/.claude` happens to
  have that MCP".
- **Clean context.** The catalog + [MCP-as-skill](./mcp-as-skill.md) keep only
  the tools a given run needs in the agent's context, instead of every MCP the
  user has ever installed.
- **Embeddable.** Lib mode + abstracted I/O make `am` the substrate a bigger
  orchestrator (Ubiq, a web UI, a CI job) drives without shelling out and
  scraping a tty.
