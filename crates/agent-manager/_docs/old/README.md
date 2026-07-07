# `_docs/old/` — superseded design (config-sync era)

> **These documents describe the *previous* direction of `agent-manager` and
> are kept for reference only. They no longer describe what we are building.**

## What this was

The original `agent-manager` was a **config-sync tool**: author one
harness-agnostic `.agent-manager.toml` (rules, skills, MCP, sub-agents) and
`agent-manager sync` would *render* that config into every harness's native
config directory (`~/.claude/…`, `.codex/…`, `opencode.json`, …), keeping them
drift-free.

- `architecture.md` — the sync pipeline (`UnifiedConfig → sync engine → N harnesses`).
- `config-format.md` — the `.agent-manager.toml` four-table shape.
- `project-structure.md` — the crate layout for the sync engine + TUI.

## Why it changed

We pivoted from **managing config files on disk** to **wrapping the running
agent**. Instead of writing into the user's `~/.claude`, `agent-manager` (now
`am`) *launches* the agent itself — `am claude …` — and injects skills, MCPs,
an account, initial instructions, and hooks into an **ephemeral, per-run config
directory**, optionally inside an isolated environment. See the new target
design in [`../target/`](../target/) and the migration path in
[`../transition-plan.md`](../transition-plan.md).

## What survives the pivot

Not everything here is dead. The parts that carry over:

- **Harness rendering knowledge.** The mechanics of "turn a set of MCP servers
  into the JSON shape Claude Code expects" are exactly what the new
  *provisioner* uses to build the ephemeral config dir — it just writes to a
  temp dir and launches, instead of writing to `~/.claude` and stopping.
- **The `_docs/harness/*` docs.** Those already document each harness's
  **runtime** contract (launch flags, output stream protocol, MCP/skill
  injection seams) and are still authoritative. They live one level up, not
  here.
- **The unified resource model** (`Rule`, `Skill`, `McpServer`, `Agent`). The
  types survive; what changes is that they feed a launcher instead of a
  file-sync engine.

The config-*sync* command (`agent-manager sync` writing into the user's real
harness dirs) is the piece that is retired as the product's primary purpose. It
may return later as an optional `am config apply` convenience, but it is no
longer the point of the tool.
