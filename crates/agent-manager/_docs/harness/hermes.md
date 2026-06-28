# Hermes

Stable id: `hermes`
Display name: Hermes
Vendor: Not documented
Status: Reference — characterised from observed runtime contract; not yet an agent-manager sync target.

## Quick reference

| Field        | Value                                                                                        |
|--------------|----------------------------------------------------------------------------------------------|
| Stable id    | `hermes`                                                                                     |
| Display name | Hermes                                                                                       |
| Vendor       | Not documented                                                                               |
| Global root  | Not documented as of 2026-06-28                                                              |
| Project root | `<workdir>/` (observed: `AGENTS.md` + `.agent_context/` subdirectory)                       |
| Protocol     | Agent Client Protocol (ACP) — JSON-RPC 2.0 over stdio, newline-framed                       |
| Status       | Reference — characterised from observed runtime contract; not yet an agent-manager sync target. |

## On-disk layout

Not documented as of 2026-06-28 — this reference characterises Hermes from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

### Global

Not documented.

### Project (`<workdir>/`)

```
<workdir>/
├── AGENTS.md                             # always-on context (open standard; observed)
└── .agent_context/
    └── skills/
        └── <name>/
            └── SKILL.md                  # project skills (agent-neutral fallback path; observed)
```

## Discovery precedence

Not documented as of 2026-06-28 — this reference characterises Hermes from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

Observed behaviour:

1. **Always-on context** — `AGENTS.md` in the working directory is loaded on every turn.
2. **Skills** — `<workdir>/.agent_context/skills/<name>/SKILL.md` (agent-neutral fallback path).
3. **MCP servers** — supplied at session start via the `mcpServers` array in the `session/new` ACP message; not read from a config file on disk.

## Feature matrix

| Feature          | Support           | Where it lands                                                                            |
|------------------|-------------------|-------------------------------------------------------------------------------------------|
| Rules            | n/a (no renderer) | `AGENTS.md` in working directory (open standard; observed seam)                           |
| Skills           | n/a (no renderer) | `<workdir>/.agent_context/skills/<name>/SKILL.md` (agent-neutral fallback; observed seam) |
| MCP              | n/a (no renderer) | `mcpServers` array in ACP `session/new` message (observed seam)                           |
| Agents           | n/a (no renderer) | Not documented                                                                            |
| Slash commands   | n/a (no renderer) | Not documented                                                                            |
| Auth             | n/a (no renderer) | Not documented                                                                            |
| Permissions      | n/a (no renderer) | `HERMES_YOLO_MODE=1` env var; on-stream `session/request_permission` handshake (observed) |
| Policies / Rules | n/a (no renderer) | `AGENTS.md` in working directory (open standard; observed seam)                           |

## Skills

### Locations

```
<workdir>/.agent_context/skills/<name>/SKILL.md   # project (agent-neutral fallback path; observed)
```

Global skills location is not documented as of 2026-06-28.

### Format

Standard agent-neutral `SKILL.md` with YAML frontmatter. Required keys:

| Key           | Required | Notes                                     |
|---------------|----------|-------------------------------------------|
| `name`        | yes      | Must match the containing directory name. |
| `description` | yes      | Shown to the model as the skill summary.  |

Additional frontmatter keys (e.g. `license`, `compatibility`, `metadata`) follow the Agent Skills open standard (<https://agentskills.io>) and are accepted.

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

### Discovery / invocation notes

Skills are loaded from `<workdir>/.agent_context/skills/` before launch. A coordinator materialises skill directories before starting `hermes acp` — changes to the directory after launch are not observed. Cross-reference Orchestration / headless invocation — Skills at launch.

## Sub-agents

Not documented as of 2026-06-28 — this reference characterises Hermes from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## MCP servers

MCP servers are not supplied through a config file on disk. They are passed to Hermes at session start via the `mcpServers` array inside the `session/new` ACP message, filtered to the transports Hermes advertised in its `initialize` response (`agentCapabilities.mcpCapabilities.http` and `.sse` booleans).

### stdio entry

```json
{
  "name": "my-mcp",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-everything"],
  "env": [
    { "name": "MY_ENV_VAR", "value": "value" }
  ]
}
```

### Remote HTTP entry

```json
{
  "type": "http",
  "name": "my-remote-mcp",
  "url": "https://mcp.example.com/mcp",
  "headers": [
    { "name": "Authorization", "value": "Bearer your-token" }
  ]
}
```

For SSE: `"type": "sse"`.

The coordinator must filter the `mcpServers` list to transports advertised by the `initialize` reply before sending `session/new`. No file is written to disk for MCP configuration. Cross-reference Orchestration / headless invocation — MCP at launch for the full injection sequence.

## Slash commands

Not documented as of 2026-06-28 — this reference characterises Hermes from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Authentication

Not documented as of 2026-06-28 — this reference characterises Hermes from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Permissions

Not documented as of 2026-06-28 — this reference characterises Hermes from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

Observed approval mechanism: setting `HERMES_YOLO_MODE=1` in the environment enables automatic tool approval, suppressing interactive permission prompts for the entire process lifetime. When `HERMES_YOLO_MODE` is absent, Hermes emits a `session/request_permission` JSON-RPC notification for each tool call requiring approval; an external coordinator auto-approves by replying:

```json
{"outcome": "selected", "optionId": "approve_for_session"}
```

Cross-reference Orchestration / headless invocation — Tool approval in headless mode.

## Policies / Rules / Memory

Not documented as of 2026-06-28 — this reference characterises Hermes from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

Observed behaviour: `AGENTS.md` in the working directory is honoured as always-on context (open standard). It is prepended to the system prompt on every turn. No additional policy or memory mechanism has been observed.

## Orchestration / headless invocation

### Non-interactive launch

```
HERMES_YOLO_MODE=1 hermes acp
```

- `hermes acp` is the subcommand that starts the ACP JSON-RPC server on stdio.
- `HERMES_YOLO_MODE=1` enables automatic tool approval for fully unattended runs.
- No additional flag is required to select machine-readable output; the ACP protocol is the wire format.

### Output stream protocol

Agent Client Protocol (ACP) — JSON-RPC 2.0 over stdio, one JSON object per line (newline-framed).

**Handshake sequence:**

1. An external coordinator sends `initialize`; Hermes replies advertising `agentCapabilities.mcpCapabilities.http` and `.sse` booleans.
2. Coordinator sends `session/new` with params `{ cwd, mcpServers, model, sessionId }`.
3. (Optional) Coordinator sends `session/set_model` with `{ sessionId, modelId }`.
4. Coordinator sends `session/prompt` with `{ sessionId, prompt: [{ type: "text", text: "..." }] }`.

**Notification stream** (method `session/update` or `session/notification`; carries an `update` field):

| `update` value        | Canonical category     | Notes                                             |
|-----------------------|------------------------|---------------------------------------------------|
| `agent_message_chunk` | Assistant text         | Streamed; concatenate fragments in order.         |
| `agent_thought_chunk` | Reasoning              | Internal reasoning; may be omitted.               |
| `tool_call`           | Tool call              | Tool invocation initiated by the model.           |
| `tool_call_update`    | Tool result            | State `complete` carries the result.              |
| `usage_update`        | Cumulative token usage | Updated as tokens are consumed.                   |
| `turn_end` / `endTurn`| Turn completion        | Signals the end of a prompt/response cycle.       |

To resume a prior session: `session/resume` with the prior `sessionId`.

### Model & reasoning at launch

- **Model**: set via the `model` field in `session/new` params and/or a subsequent `session/set_model` call (`{ sessionId, modelId }`).
- **Reasoning effort**: not documented as of 2026-06-28 — Hermes ACP exposes no reasoning-effort parameter.

Cross-reference Authentication for provider credential injection; credentials are not carried in ACP messages.

### MCP at launch

MCP servers are passed inside `session/new` as the `mcpServers` array. The coordinator must filter the list to transports Hermes advertised in its `initialize` reply (`agentCapabilities.mcpCapabilities.http` and `.sse` booleans) before sending `session/new`.

Entry shapes (cross-reference MCP servers for field details):

- **stdio**: `{ name, command, args, env: [{ name, value }] }`
- **Remote HTTP**: `{ type: "http", name, url, headers: [{ name, value }] }`
- **Remote SSE**: `{ type: "sse", name, url, headers: [{ name, value }] }`

No file is written to disk for MCP configuration; the coordinator holds all server definitions and injects them at session start. Ambient or inherited MCP servers from disk are not expected — the full list must be provided explicitly each session.

### Skills at launch

A coordinator materialises skills into `<workdir>/.agent_context/skills/<name>/SKILL.md` before launching `hermes acp`. Always-on context goes into `AGENTS.md` in the working directory. Cross-reference Skills and Policies / Rules / Memory.

### Tool approval in headless mode

Two mechanisms (use one or both):

1. **`HERMES_YOLO_MODE=1`** — set in the child process environment before launch; Hermes approves all tool calls automatically without emitting `session/request_permission`.
2. **On-stream handshake** — when `HERMES_YOLO_MODE` is absent, Hermes emits a `session/request_permission` notification for each tool call. An external coordinator auto-approves by replying:

   ```json
   {"outcome": "selected", "optionId": "approve_for_session"}
   ```

   `approve_for_session` covers all subsequent calls of the same tool for the remainder of the session.

### Process lifecycle

- **Framing**: JSON-RPC 2.0 over stdio; one JSON object per line. Events arrive on stdout; diagnostics on stderr.
- **Cancellation**: close stdin → Hermes cancels its context → await the reader and stderr goroutines before the coordinator considers the process done.
- **Session resume**: send `session/resume` with the prior `sessionId` to continue an existing session.
- **Minimum CLI version**: not documented.

## Format quirks / gotchas

- **`HERMES_YOLO_MODE=1` is required for fully unattended runs.** Without it, `session/request_permission` blocks the run until a reply is sent; a coordinator that omits both the env var and the on-stream handler will hang.
- **Stderr carries provider errors.** A stderr sniffer must watch for rate-limit and auth-failure patterns; an otherwise-`turn_end` turn may be semantically failed if the underlying provider reported an error on stderr.
- **Filter `mcpServers` before `session/new`.** Include only entries whose transport type was advertised in the `initialize` reply. Sending an `http` entry when `.mcpCapabilities.http` is false has undefined behaviour.
- **Both `session/update` and `session/notification` are observed notification method names.** Treat both as equivalent update carriers; inspect the `update` field for event type.
- **`approve_for_session` covers all subsequent calls of the same tool.** Per-call approval using a different `optionId` is not documented as of 2026-06-28.
- **Session ids are coordinator-supplied in `session/new`.** Use a stable UUID per run; pass the same id to `session/resume` when continuing.
- **`AGENTS.md` is always-on context, not an optional instruction file.** Instructions that must persist across turns must be written to `AGENTS.md` before launch; ACP message content does not substitute for it.
- **Skills are loaded from disk, not from ACP.** Materialise the skill directory before starting `hermes acp`; changes after launch are not observed.

## Renderer notes (planned)

`agent-manager`'s Hermes renderer should:

1. **Rules → always-on context**: write `<workdir>/AGENTS.md` before launching `hermes acp`. This is the only observed persistent instruction mechanism.
2. **Skills**: materialise each skill into `<workdir>/.agent_context/skills/<name>/SKILL.md` before launch. Frontmatter must carry at least `name` (must match the directory name) and `description`.
3. **MCP**: hold server definitions in coordinator state; inject them as the `mcpServers` array in the `session/new` ACP message. Filter to transports advertised by `initialize`. Do not write MCP config to disk.
4. **Tool approval**: set `HERMES_YOLO_MODE=1` in the child process environment for fully unattended runs. Alternatively, implement the `session/request_permission` / `approve_for_session` on-stream handshake in the coordinator's stdio reader.
5. **Stderr monitoring**: attach a sniffer to the child's stderr stream; classify provider error patterns (rate-limit, auth failure) and propagate them as turn-level errors even when the JSON-RPC stream indicates `turn_end`.
6. **Session lifecycle**: generate a UUID `sessionId` per run; pass it in `session/new`; store it for `session/resume`. Cancellation: close stdin, then await process exit before releasing resources.
7. **Files the renderer does not own**: any files inside `<workdir>` not written by the renderer (source code, user config, etc.) must be left untouched.

## Sources

- Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28.
- Agent Client Protocol (ACP) — JSON-RPC 2.0 over stdio; referenced generically (no vendor URL available as of 2026-06-28).
