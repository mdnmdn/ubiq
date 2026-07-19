# Roadmap

The implementation is rolled out in phases. Each phase is independently useful and is
implemented across multiple sessions. **Phase 1, Phase 2, and Phase 3 are complete.**
Next: OAuth MCP, web mode, and ACP server (see "Beyond").

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

## Phase 3 — isolation, sessions, output protocols, hooks ✅ (shipped)

**Goal:** production-grade runs and outward-facing event streams. **Done.**

- **Isolation:** `--isolate[=profile]` wraps the harness launch in an
  [isol8](https://github.com/mdnmdn/isol8) sandbox (filesystem/network confinement). Configurable
  via settings `[isolate] command` template (default `isol8 run {profile_opt} -- {cmd}`).
- **Session history:** `am session ls|show|resume`; persist transcripts +
  metadata under `am`'s own state dir; resume a prior run via `am session resume <id>` or
  direct `--resume <harness-session-id>` (Claude + opencode; codex deferred to app-server).
- **Output protocols:** `--output <events|acp|agui>` on structured runs; stateless best-effort
  mappers (`crate::io::{to_acp, to_agui}`) over the neutral event model, covering core event types
  (AssistantText, Thinking, ToolCall, ToolResult, SessionStarted, Result).
- **Hooks:** per-run hook selection (`--hooks a,b` / settings `[defaults].hooks`); wired into
  harness-native hook slots (Claude `settings.json`, Codex `hooks.json`; opencode no-op).
- **MCP-as-skill:** schema + stepping stone only — `[[mcp]] expose = "tools" | "skill"` and `summary`
  in catalog; `--mcp-as-skill a,b` CLI flag; provisioner generates `SKILL.md` pointers (the deferred-load /
  proxy-tool expand-on-demand mechanism is deferred to a later step).

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
| Isolated environment (isol8)       | P3 ✅ |
| Session history + resume           | P3 ✅ |
| Hooks (wired into harness slots)   | P3 ✅ |
| Output protocols (ACP / AG-UI)     | P3 ✅ |
| MCP-as-skill (schema + stepping stone) | P3 ✅ |
| OAuth MCP / web mode / ACP server  | future|
