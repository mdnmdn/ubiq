# Kimi

Stable id: `kimi`
Display name: Kimi
Vendor: Moonshot AI (Kimi)
Status: Reference — characterised from observed runtime contract; not yet an agent-manager sync target.

## Quick reference

| Field        | Value                                                                                          |
|--------------|------------------------------------------------------------------------------------------------|
| Stable id    | `kimi`                                                                                         |
| Display name | Kimi                                                                                           |
| Vendor       | Moonshot AI (Kimi)                                                                             |
| Status       | Reference — characterised from observed runtime contract; not yet an agent-manager sync target. |
| Global root  | Not documented as of 2026-06-28                                                                |
| Project root | Not documented as of 2026-06-28 (skills observed at `<workdir>/.kimi/skills/`)                 |

## On-disk layout

Not documented as of 2026-06-28 — this reference characterises Kimi CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

Observed artefact: skills are materialised into `<workdir>/.kimi/skills/<name>/SKILL.md` before launch (see Skills).

## Discovery precedence

Not documented as of 2026-06-28 — this reference characterises Kimi CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Feature matrix

| Feature          | Support           | Where it lands                                                   |
|------------------|-------------------|------------------------------------------------------------------|
| Rules            | n/a (no renderer) | `AGENTS.md` in workdir honoured as always-on context (observed)  |
| Skills           | n/a (no renderer) | `<workdir>/.kimi/skills/<name>/SKILL.md` (observed)              |
| MCP              | n/a (no renderer) | `mcpServers` array in ACP `session/new` at runtime (observed)    |
| Agents           | n/a (no renderer) | Not documented as of 2026-06-28                                  |
| Slash commands   | n/a (no renderer) | Not documented as of 2026-06-28                                  |
| Auth             | n/a (no renderer) | Not documented as of 2026-06-28                                  |
| Permissions      | n/a (no renderer) | ACP `session/request_permission` handshake at runtime (observed) |
| Policies / Rules | n/a (no renderer) | `AGENTS.md` in workdir (observed)                                |

## Skills

### Location

Skills are materialised into the working directory before `kimi acp` is launched:

```
<workdir>/.kimi/skills/<name>/SKILL.md
```

### Format

Standard Agent Skills shape: Markdown file with YAML frontmatter. Minimum required keys:

| Key           | Required | Notes                          |
|---------------|----------|--------------------------------|
| `name`        | yes      | Must match the directory name. |
| `description` | yes      | Shown to the model.            |

### Minimal skill

```markdown
---
name: git-release
description: Create consistent releases and changelogs
---

## What I do
- Draft release notes from merged PRs
- Propose a version bump
```

### Discovery

An external coordinator materialises skills into `<workdir>/.kimi/skills/<name>/SKILL.md` before invoking `kimi acp`. Kimi discovers skills from this directory at startup. Always-on context goes into `AGENTS.md` in the working directory (see Policies / Rules / Memory).

## Sub-agents

Not documented as of 2026-06-28 — this reference characterises Kimi CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## MCP servers

MCP servers are not configured on disk for a single run. An external coordinator supplies them via the `mcpServers` array inside the ACP `session/new` message, filtered to the transport types advertised by the harness in the `initialize` response (see Orchestration / headless invocation for the full wire shape).

Ambient on-disk MCP configuration for `kimi acp` is not documented as of 2026-06-28 — the native configuration surface has not been verified against vendor documentation.

## Slash commands

Not documented as of 2026-06-28 — this reference characterises Kimi CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Authentication

Not documented as of 2026-06-28 — this reference characterises Kimi CLI from its observed non-interactive runtime contract; the native configuration surface has not been verified against vendor documentation. Kimi CLI is a product of Moonshot AI; credentials are expected to be Moonshot AI credentials, but the specific env var names and storage locations have not been confirmed against vendor documentation.

## Permissions

Not documented as of 2026-06-28 — this reference characterises Kimi CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

Tool approval at runtime is handled via the ACP `session/request_permission` handshake; see Tool approval in headless mode under Orchestration / headless invocation.

## Policies / Rules / Memory

`AGENTS.md` in the working directory is honoured as always-on context (observed). An external coordinator places `AGENTS.md` at `<workdir>/AGENTS.md` before invoking `kimi acp`.

Broader policies, per-directory walk rules, and global memory files are not documented as of 2026-06-28 — this reference characterises Kimi CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Orchestration / headless invocation

### Non-interactive launch

```
kimi acp
```

No additional flags select machine-readable output or suppress interactive prompts — `kimi acp` is inherently non-interactive. The working directory must be set in the child process environment before launch. Model and session parameters are injected via ACP messages after the process starts (see Model & reasoning at launch).

`--yolo` and `--auto-approve` CLI flags are silently ignored by `kimi acp`; auto-approval must be implemented through the ACP `session/request_permission` handshake (see Tool approval in headless mode).

### Output stream protocol

`kimi acp` implements the **Agent Client Protocol (ACP)** — JSON-RPC 2.0 over stdio, newline-framed. All messages are newline-terminated JSON objects exchanged on stdin (coordinator to harness) and stdout (harness to coordinator).

**Handshake sequence:**

1. Coordinator sends `initialize`. Harness responds with capabilities including `agentCapabilities.mcpCapabilities.http` and/or `.sse` — the advertised MCP transport types.
2. Coordinator sends `session/new` with `{ cwd, mcpServers, model, sessionId }`.
3. Coordinator optionally sends `session/set_model` with `{ sessionId, modelId }` to override the model.
4. Coordinator sends `session/prompt` with `{ sessionId, prompt: [{ "type": "text", "text": "..." }] }`.
5. To resume a prior session: send `session/resume` in place of `session/new`.

**Event stream — `session/update` notifications:**

All streamed events arrive as JSON-RPC `session/update` notifications on stdout. The `type` field inside the notification payload identifies the event category:

| `type` value          | Canonical category | Notes                                                              |
|-----------------------|--------------------|--------------------------------------------------------------------|
| `agent_message_chunk` | assistant text     | Streaming text delta.                                              |
| `agent_thought_chunk` | reasoning          | Internal reasoning / thinking delta.                               |
| `tool_call`           | tool call          | A tool invocation initiated by the agent.                          |
| `tool_call_update`    | tool result        | Tool result update; `state: "complete"` signals the call is final. |
| `usage_update`        | usage              | Token-count update.                                                |
| `turn_end` / `endTurn`| completion         | Signals end of the current agent turn.                             |

### Model & reasoning at launch

- **Model:** supplied as the `model` field in the `session/new` message body.
- **Model override mid-session:** send `session/set_model` with `{ sessionId, modelId }` at any point before `session/prompt`.
- **Reasoning effort:** not documented as of 2026-06-28; no reasoning-effort parameter has been observed in the ACP contract.

Authentication (Moonshot AI credentials) must be established in the harness process environment before launch; credentials are not passed via ACP messages. Cross-reference Authentication.

### MCP at launch

MCP servers are supplied inside the `session/new` message as the `mcpServers` array. Include only transports that the harness advertised in the `initialize` response (`http`, `sse`, or both).

**stdio server** (local subprocess):

```json
{
  "name": "my-server",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-everything"],
  "env": [
    { "name": "MY_VAR", "value": "value" }
  ]
}
```

**Remote server** (HTTP or SSE):

```json
{
  "type": "http",
  "name": "remote-server",
  "url": "https://mcp.example.com/mcp",
  "headers": [
    { "name": "Authorization", "value": "Bearer your-token" }
  ]
}
```

Do not write an MCP config to disk and expect `kimi acp` to discover it; the `mcpServers` array in `session/new` is the only confirmed injection channel.

### Skills at launch

An external coordinator materialises skills into `<workdir>/.kimi/skills/<name>/SKILL.md` before invoking `kimi acp`. Always-on context is placed in `<workdir>/AGENTS.md`. Cross-reference Skills and Policies / Rules / Memory.

### Tool approval in headless mode

`kimi acp` silently ignores `--yolo` and `--auto-approve` CLI flags. Auto-approval is achieved exclusively through the ACP approval handshake:

1. Harness sends a `session/request_permission` notification.
2. Coordinator responds immediately with:

```json
{ "outcome": "selected", "optionId": "approve_for_session" }
```

`optionId: "approve_for_session"` approves the tool for the remainder of the session. A coordinator running headlessly must implement this handshake; failing to respond stalls the run indefinitely.

### Process lifecycle

- **Framing:** JSON-RPC 2.0 newline-delimited on stdin/stdout; diagnostics on stderr.
- **Cancellation:** close stdin, send a JSON-RPC cancel notification, then await the stdout reader and stderr draining before reaping the process.
- **Session resume:** send `session/resume` (in place of `session/new`) referencing the prior `sessionId`.
- **Minimum CLI version:** not documented as of 2026-06-28; characterised from observed runtime contract.

Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28.

## Format quirks / gotchas

- **Tool-call argument accumulation is required.** `kimi acp` streams tool-call arguments token-by-token via repeated `tool_call_update` notifications. Accumulate all `tool_call_update` payloads for a given call until `state: "complete"` is received; do not treat the call as final before that point.
- **Tool names arrive title-cased / humanised.** Tool names come in a humanised form (e.g. `"Read file: path/to/file"`) rather than as canonical tool identifiers. Normalise incoming tool names back to canonical tool IDs before dispatching or logging.
- **`--yolo` / `--auto-approve` flags are silently ignored.** Passing these flags to `kimi acp` has no effect. Implement auto-approval via the `session/request_permission` ACP handshake; do not rely on CLI flags.
- **MCP is supplied at runtime, not on disk.** For a single run, MCP servers are delivered via `mcpServers` in `session/new`. Do not write an MCP config to disk expecting `kimi acp` to discover it.
- **Filter `mcpServers` to advertised transports.** The `initialize` response declares which transport types the harness supports. Supply only matching transport types in `session/new`; unsupported types risk silent failure.
- **`AGENTS.md` is the always-on context mechanism.** Place `AGENTS.md` in the working directory before launch. This is the only confirmed always-on instruction mechanism.
- **Skills directory is `.kimi/skills/`, not `.agents/skills/`.** Materialise skills under `<workdir>/.kimi/skills/<name>/SKILL.md`; do not use other well-known skill directories for this harness.

## Renderer notes (planned)

`agent-manager`'s Kimi renderer is not yet planned (status: not yet an agent-manager sync target). When a renderer is implemented, it should:

1. **Skills** — materialise `<workdir>/.kimi/skills/<name>/SKILL.md` before invoking `kimi acp`. Use the standard `SKILL.md` frontmatter shape (`name` and `description` at minimum; `name` must equal the directory name).
2. **Always-on context** — write `<workdir>/AGENTS.md` before launch.
3. **MCP** — do not write on-disk MCP config; supply `mcpServers` as an array in the ACP `session/new` message, filtered to the transports advertised in the `initialize` response.
4. **Tool approval** — implement the `session/request_permission` → `{"outcome":"selected","optionId":"approve_for_session"}` handshake; do not rely on CLI flags.
5. **Tool-call argument accumulation** — buffer `tool_call_update` payloads until `state: "complete"` before treating a tool call as final.
6. **Tool name normalisation** — map humanised / title-cased tool names back to canonical tool IDs after receipt.
7. **Files not owned by the renderer** — leave all pre-existing files in `<workdir>/` outside `.kimi/` and `AGENTS.md` untouched.

## Sources

- Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28. No vendor documentation URL has been verified against a stable canonical reference.
- Agent Client Protocol (ACP) — generic ACP protocol shape (JSON-RPC 2.0 over stdio, `initialize` / `session/new` / `session/prompt` / `session/update` flow); Kimi-specific behaviour verified against observed `kimi acp` stdio traffic as of 2026-06-28.
- Moonshot AI / Kimi — vendor homepage: <https://www.moonshot.ai> (general product reference; specific CLI documentation not verified against a stable URL as of 2026-06-28).
