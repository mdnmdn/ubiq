# Roadmap

The target design lands in phases. Each phase is independently useful and is
implemented across multiple sessions. **Phase 1 and Phase 2 are complete.**
Next: Phase 3 adds isolation (isol8), session history, output adapters (ACP/AG-UI), and hooks.

## Phase 1 — CLI wrapper with catalog injection ✅ (shipped)

**Goal:** `am claude --mcps … --skills …` launches the real harness in
passthrough with an ephemeral, catalog-provisioned config. **Done** for Claude
Code end-to-end (PTY passthrough, exit-code propagation, `am catalog …`).

- CLI surface: `am <harness> …` + `am catalog …`.
- Catalog: filesystem-backed `Registry` (config + folders), `catalog ls`.
- `catalog import`: ingest `~/.claude` / `~/.agent` / project dirs (read-only).
- `resolve`: flags + settings file → `RunSpec`.
- `provision`: `RunSpec` → ephemeral config dir + launch argv + env, for the
  first harness (Claude Code) — inject **skills** and **MCPs**, set the custom
  config folder.
- `run`: PTY passthrough, faithful signals/resize/exit-code.
- One harness end-to-end (Claude Code), with the `Harness` trait shaped so codex
  / opencode slot in later.

**Done when:** a user with a populated catalog can run
`am claude --mcps postgres --skills web-designer`, the agent has exactly those
tools/skills, the user's real `~/.claude` is untouched, and Ctrl-C / exit codes
behave as if `am` weren't there.

## Phase 2 — accounts, initial prompt, agent trait & structured input ✅ (shipped)

**Goal:** choose an account, seed instructions, and drive the agent
programmatically (lib mode). **Done.**

- **Account injection + catalog:** `am account ls|use|import`; inject credential
  *references* (env var / keyring / credential file / private `HOME`) into the
  harness's native auth slot. `am` stores references, never secret material.
- **Initial instructions / prompt:** `--instructions`, `--prompt` seeded into
  the ephemeral config / first message.
- **`Harness` trait implementations** for structured input:
  - Claude Code — **JSONL** (stream-json) input.
  - opencode — **NDJSON** (run --format json) input.
  - codex — **JSON-RPC** (app-server) input.
- **Neutral I/O model** (`AgentInput` / `AgentEvent`) and the `IoBridge` trait.
- **Custom in-process MCP (lib mode):** an embedder registers an MCP the wrapped
  agent can call (`McpRef::InProcess`).

**Verified:** a lib-mode embedder can build a `RunSpec` (harness + account +
in-process MCP + initial prompt), run it with JSONL or JSON-RPC or NDJSON input, and read back
normalized events. All three harnesses wrap and launch end-to-end.

## Phase 3 — isolation, sessions, output protocols, hooks

**Goal:** production-grade runs and outward-facing event streams.

- **Isolation:** `--isolate` runs the harness inside
  [isol8](https://github.com/mdnmdn/isol8) (filesystem/network confinement).
- **Session history:** `am session ls|show|resume`; persist transcripts +
  metadata; resume a prior run (`--resume`).
- **Output protocols:** ACP-events and AG-UI-events output adapters over the
  neutral event model.
- **Hooks:** wire lifecycle hooks into each harness's native hook slots.
- **MCP-as-skill:** implement the deferred-load / proxy-tool mechanism
  (see [`mcp-as-skill.md`](./mcp-as-skill.md)).

## Beyond — mentioned, not committed

- **OAuth MCP auth** — first-class OAuth flow for MCP servers that need it.
- **Web mode** — run headless; UI over web/HTTP + WebSocket with xterm.js;
  expose the agent via **AG-UI**.
- **Expose the agent via ACP** — make an `am`-wrapped agent an ACP server other
  clients can connect to.

## Phase → responsibility map

Cross-reference with the responsibilities table in
[`overview.md`](./overview.md):

| Responsibility                     | Phase |
|------------------------------------|-------|
| Inject skills & MCPs               | P1 ✅ |
| Custom config folder               | P1 ✅ |
| Catalog + import                   | P1 ✅ |
| Passthrough run                    | P1 ✅ |
| Inject account / account catalog   | P2 ✅ |
| Initial instructions / prompt      | P2 ✅ |
| Agent trait (JSONL / JSON-RPC / NDJSON input) | P2 ✅ |
| Custom in-process MCP (lib mode)   | P2 ✅ |
| Isolated environment (isol8)       | P3    |
| Session history                    | P3    |
| Hooks                              | P3    |
| Output protocols (ACP / AG-UI)     | P3    |
| MCP-as-skill                       | P3    |
| OAuth MCP / web mode / ACP server  | future|
