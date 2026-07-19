# MCP-as-skill

## The problem it solves

Every MCP server an agent loads spends context budget: its tool list, each
tool's JSON schema, and its descriptions are injected into the system prompt
whether or not the agent ends up using them. Load a dozen MCPs "just in case"
and you have burned thousands of tokens before the first user message — and
given the model more tools to be confused by.

Skills have the opposite shape. A skill is **latent**: only its one-line
description sits in context; the full body loads **on demand** when the agent
decides the skill is relevant. That is exactly the property we want for MCPs the
run *might* need but usually won't.

## The idea

`am` can expose a catalog MCP **as a skill** instead of as a raw, always-on tool
set. The agent sees a cheap one-line skill entry ("Postgres database access —
query, inspect schema, …"). Only when it invokes that skill does the underlying
MCP's tools become available for the task at hand.

```
   without:   [postgres tools ×12 schemas]  ← always in context, always
   with:      "postgres: query a database"  ← one line, expands on demand
```

## How it can work (design sketch — not yet built)

The provisioner, when it sees an MCP entry marked `expose = "skill"`, writes a
**generated skill** into the ephemeral config dir instead of (or in addition to)
the raw MCP config. The generated `SKILL.md`:

- carries a concise `description` derived from the MCP's own metadata, so
  auto-invocation works;
- in its body, tells the agent how to reach the MCP's tools for this task.

Two plausible mechanisms for the "expand on demand" step, to be decided during
implementation:

1. **Deferred load.** The skill body instructs the agent, and `am` (via the I/O
   bridge or a hook) enables the real MCP server for the session once the skill
   fires. Cleanest context saving; needs a harness that lets tools be added
   mid-session.
2. **Proxy tool.** `am` exposes a single thin "call the postgres MCP" tool (one
   schema, not twelve) that the skill documents; behind it, `am` proxies to the
   real MCP. Works on any harness, at the cost of one always-present tool per
   proxied MCP.

The catalog marks intent; the provisioner + the active harness's capabilities
pick the mechanism.

## Catalog declaration (sketch)

```toml
# catalog.toml
[[mcp]]
id = "postgres"
transport = "stdio"
command = "mcp-postgres"
expose = "skill"        # "tools" (default) | "skill"
summary = "Query and inspect a Postgres database."   # seeds the skill description
```

Or per-run, without touching the catalog:

```bash
am claude --mcps postgres --mcp-as-skill postgres
```

## Status

**Landed (Phase 3, step I1): the schema + `SKILL.md`-generation stepping stone.**

- The catalog schema — `[[mcp]]` `expose = "tools" | "skill"` and `summary`
  in `catalog.toml` (see [`registry.md`](./registry.md)) — is implemented and
  parsed by `FsRegistry`.
- A per-run `--mcp-as-skill a,b` CLI flag additionally marks already-injected
  mcp ids for a skill pointer without touching the catalog (see
  [`cli.md`](./cli.md)); it merges (deduped) with any catalog `expose =
  "skill"` entries.
- `resolve()` carries the intent into `RunSpec.mcp_as_skill: Vec<McpAsSkill>`
  (`{ id, summary }`). This is **additive**: the named mcp still lands in
  `RunSpec.mcps` exactly as it does today.
- Every provisioner (Claude Code, Codex, opencode) that sees a non-empty
  `mcp_as_skill` writes one generated `SKILL.md` per entry into its normal
  skills dir (`<config>/skills/<id>/SKILL.md` for Claude Code/opencode,
  `<CODEX_HOME>/.agents/skills/<id>/SKILL.md` for Codex), with a `description:`
  seeded from `summary` (or a generic fallback) and a body that explicitly
  says the MCP is *not yet* deferred.
- Runs that don't use the feature (`mcp_as_skill` empty) produce
  byte-identical provisioned config to before this landed — no new dir/file
  is written.

**Still deferred: the "expand on demand" mechanism.** The generated
`SKILL.md` today is a *documented pointer only* — it does **not** save any
context, because the MCP's tools are still injected as a normal, always-on
tool set alongside it. Making the skill's on-demand nature real (deferred
load vs. proxy tool, per the two mechanisms sketched above, and how much each
depends on harness support) is unbuilt and left to a later step.
