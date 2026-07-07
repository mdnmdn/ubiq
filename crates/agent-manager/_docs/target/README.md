# `_docs/target/` — the target design (agent-runtime era)

> **This is the direction we are building toward.** It supersedes the
> config-sync design archived in [`../old/`](../old/). Implementation happens
> incrementally, one session at a time, guided by the
> [roadmap](./roadmap.md) and the [transition plan](../transition-plan.md).

## One-line pitch

`agent-manager` (CLI name: **`am`**) is a **wrapper for a running agent
harness**. Instead of running `claude` / `codex` / `opencode` directly, you run
them *through* `am`, which injects skills, MCP servers, an account, initial
instructions, and hooks — assembled from a **catalog** — into an **ephemeral
per-run config**, optionally inside an **isolated environment**, and can
**abstract the agent's I/O** (passthrough, ACP, JSONL, AG-UI) for embedding in
larger tools.

```bash
am claude --mcps postgres,figma --skills web-designer --safe
#  │        └── inject from the catalog, into an ephemeral config dir
#  └── wrap & launch the real `claude` binary
```

## Two ways to use it

- **CLI mode** — run an agent from your terminal with catalog-driven injection.
- **Lib mode** — embed the crate in a bigger tool (e.g. the Ubiq multiplexer),
  construct a run spec programmatically (including in-process custom MCPs), and
  drive the agent through an abstracted I/O channel.

## Read in this order

1. [`overview.md`](./overview.md) — the vision, the responsibilities, the two
   modes, and how it differs from the old sync tool.
2. [`architecture.md`](./architecture.md) — the runtime pipeline, the core
   `RunSpec` model, the ephemeral-config *provisioner*, and the module layout.
3. [`cli.md`](./cli.md) — the `am` command surface and how CLI flags merge with
   the config file.
4. [`registry.md`](./registry.md) — the MCP/skill *catalog*: the trait, the
   on-disk layout, and `am catalog import`.
5. [`io-modes.md`](./io-modes.md) — passthrough vs. abstracted I/O; the input
   protocols (JSONL, ACP) and output protocols (ACP, AG-UI).
6. [`mcp-as-skill.md`](./mcp-as-skill.md) — exposing an MCP server *as a skill*
   to keep the context window lean.
7. [`roadmap.md`](./roadmap.md) — the phased plan (P1 → P2 → P3) and the
   future/uncommitted features.

## Related, unchanged

- [`../harness/`](../harness/) — per-harness runtime contracts (launch flags,
  output stream protocol, injection seams). **Authoritative and still current**
  — the target design consumes these directly.
- [`../reference/multica.md`](../reference/multica.md) — how a real
  orchestrator drives these harnesses end-to-end.
