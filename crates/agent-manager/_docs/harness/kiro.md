# Kiro

Stable id: `kiro`
Display name: Kiro
Vendor: AWS (Kiro)
Status: Reference — characterised from observed runtime contract; not yet an agent-manager sync target.

## Quick reference

| Field        | Value                                                                                          |
|--------------|------------------------------------------------------------------------------------------------|
| Stable id    | `kiro`                                                                                         |
| Display name | Kiro                                                                                           |
| Vendor       | AWS (Kiro)                                                                                     |
| Global root  | Not documented as of 2026-06-28                                                                |
| Project root | `<workdir>/.kiro/` (observed; skills subdir confirmed)                                         |
| Status       | Reference — characterised from observed runtime contract; not yet an agent-manager sync target. |

## On-disk layout

Not documented as of 2026-06-28 — this reference characterises Kiro from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

The only confirmed project-scoped path is the skills directory:

```
<workdir>/
├── AGENTS.md                    # always-on context (observed)
└── .kiro/
    └── skills/
        └── <name>/
            └── SKILL.md
```

## Discovery precedence

Not documented as of 2026-06-28 — this reference characterises Kiro from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Feature matrix

| Feature          | Support           | Where it lands                                                                |
|------------------|-------------------|-------------------------------------------------------------------------------|
| Rules            | n/a (no renderer) | Observed seam: `<workdir>/AGENTS.md` (always-on context)                      |
| Skills           | n/a (no renderer) | Observed seam: `<workdir>/.kiro/skills/<name>/SKILL.md`                       |
| MCP              | n/a (no renderer) | Observed seam: `mcpServers` array in `session/new` RPC message                |
| Agents           | n/a (no renderer) | Not documented as of 2026-06-28                                               |
| Slash commands   | n/a (no renderer) | Not documented as of 2026-06-28                                               |
| Auth             | n/a (no renderer) | Not documented as of 2026-06-28 (see Authentication)                          |
| Permissions      | n/a (no renderer) | Observed seam: `session/request_permission` on-stream handshake (see Orchestration / headless invocation) |
| Policies / Rules | n/a (no renderer) | Observed seam: `<workdir>/AGENTS.md` (see Policies / Rules / Memory)          |

## Skills

### Location

```
<workdir>/.kiro/skills/<name>/SKILL.md
```

An external coordinator materialises skills into this directory before launching `kiro-cli acp`. The directory shape follows the Agent Skills open standard: one folder per skill, each containing a `SKILL.md` file.

### Format

Markdown file with YAML frontmatter. The standard `SKILL.md` shape applies:

| Key           | Required | Notes                                              |
|---------------|----------|----------------------------------------------------|
| `name`        | yes      | Must match the folder name.                        |
| `description` | yes      | Shown to the model in the skill tool description.  |

Additional frontmatter keys are permitted; unknown keys are carried through and may be ignored by the harness.

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

Kiro's skill invocation mechanism has not been verified against vendor documentation as of 2026-06-28. The `<workdir>/.kiro/skills/` path is the observed coordinator-materialisation target.

## Sub-agents

Not documented as of 2026-06-28 — this reference characterises Kiro from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## MCP servers

MCP servers are supplied to Kiro via the `mcpServers` array inside the `session/new` JSON-RPC message (see Orchestration / headless invocation). There is no on-disk MCP configuration path that has been verified as of 2026-06-28.

The array is filtered at runtime to the transports advertised in the `initialize` response (`agentCapabilities.mcpCapabilities.http` and/or `.sse`).

### stdio server

```json
{
  "name": "filesystem",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/project"],
  "env": [
    { "name": "MY_VAR", "value": "my-value" }
  ]
}
```

### Remote server (HTTP or SSE)

```json
{
  "type": "http",
  "name": "remote-mcp",
  "url": "https://mcp.example.com/mcp",
  "headers": [
    { "name": "Authorization", "value": "Bearer your-token" }
  ]
}
```

`type` is `"http"` or `"sse"` per the advertised `mcpCapabilities`. The `env` and `headers` fields use **arrays of `{name, value}` objects**, not maps.

## Slash commands

Not documented as of 2026-06-28 — this reference characterises Kiro from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Authentication

Not documented as of 2026-06-28 — this reference characterises Kiro from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

Kiro is an AWS product associated with AWS Builder ID and IAM Identity Center; an external coordinator running in CI should supply appropriate AWS credentials in the process environment. Specifics have not been verified against vendor documentation.

## Permissions

Not documented as of 2026-06-28 — this reference characterises Kiro from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

In headless operation, per-tool approval is handled at two levels: the `--trust-all-tools` launch flag and the on-stream `session/request_permission` handshake. See Tool approval in headless mode under Orchestration / headless invocation.

## Policies / Rules / Memory

`AGENTS.md` is honoured by Kiro as always-on context (observed). Place project rules in `<workdir>/AGENTS.md`; it is prepended to the system prompt on every turn.

All other memory and policy surfaces are not documented as of 2026-06-28 — this reference characterises Kiro from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Orchestration / headless invocation

### Non-interactive launch

```
kiro-cli acp --trust-all-tools
```

- `acp` selects the Agent Client Protocol subcommand, which puts Kiro into JSON-RPC 2.0 over stdio mode.
- `--trust-all-tools` pre-authorises all tool calls at launch time; no interactive confirmation prompts are issued at the process level. An external coordinator must also auto-approve any `session/request_permission` messages that arrive on the stream (see Tool approval in headless mode).
- Model, MCP servers, and working directory are not passed as command-line flags; they are supplied via ACP messages after launch.

### Output stream protocol

Wire format: **JSON-RPC 2.0 over stdio, newline-framed** (Agent Client Protocol, ACP).

**Handshake sequence (coordinator sends → Kiro responds):**

1. `initialize` → Kiro responds with a result that includes `agentCapabilities.mcpCapabilities.http` and/or `.sse` (the transports Kiro will accept for MCP servers in `session/new`).
2. `session/new { cwd, mcpServers, model, sessionId }` → Kiro acknowledges and begins the session.
3. (Optional) `session/set_model { sessionId, modelId }` → updates the active model mid-session.
4. `session/prompt { content, prompt, sessionId }` → Kiro begins processing; both `content` and `prompt` fields are required (Kiro deviation; see Format quirks / gotchas).

**Notifications (Kiro → coordinator, method `session/update`):**

| `type` value          | Content                         | Canonical category |
|-----------------------|---------------------------------|--------------------|
| `agent_message_chunk` | text fragment                   | assistant text     |
| `agent_thought_chunk` | reasoning fragment              | reasoning          |
| `tool_call`           | tool name + input               | tool call          |
| `tool_call_update`    | result, `state: "complete"`     | tool result        |
| `usage_update`        | token counts                    | usage              |
| `turn_end` / `endTurn`| —                               | completion         |

**Completion signal:**

Kiro signals task completion by emitting a `goal_complete` tool call notification. If `goal_complete` was already emitted, a subsequent `session/prompt` may return JSON-RPC error `-32603 "failed to generate a response"`; this must be treated as successful completion rather than a failure (see Format quirks / gotchas).

### Model & reasoning at launch

- **Model**: supplied in the `model` field of `session/new`. May be updated mid-session via `session/set_model { sessionId, modelId }`. Model IDs follow Bedrock cross-region inference conventions (e.g. `us.anthropic.claude-sonnet-4-5-v1:0`).
- **Reasoning**: no reasoning-effort control has been documented or observed for Kiro as of 2026-06-28.
- Authentication with the model provider is handled by the Kiro process itself via its own credential chain (see Authentication); an external coordinator does not pass API keys via ACP messages.

### MCP at launch

MCP servers are declared in the `mcpServers` array inside `session/new`:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "session/new",
  "params": {
    "sessionId": "s-01",
    "cwd": "/path/to/project",
    "model": "us.anthropic.claude-sonnet-4-5-v1:0",
    "mcpServers": [
      {
        "name": "filesystem",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/project"],
        "env": []
      },
      {
        "type": "http",
        "name": "remote-mcp",
        "url": "https://mcp.example.com/mcp",
        "headers": [{ "name": "Authorization", "value": "Bearer your-token" }]
      }
    ]
  }
}
```

An external coordinator passes only the servers it manages via this array. Whether ambient Kiro MCP configuration is suppressed or merged has not been verified as of 2026-06-28.

Cross-reference: MCP servers section for the per-server field shapes.

### Skills at launch

An external coordinator materialises skills into `<workdir>/.kiro/skills/<name>/SKILL.md` before launching `kiro-cli acp`. Always-on context goes into `<workdir>/AGENTS.md`.

Cross-reference: Skills and Policies / Rules / Memory.

### Tool approval in headless mode

Two layers of approval are required for fully unattended operation:

1. **Launch flag**: `--trust-all-tools` pre-authorises all tools at the process level.
2. **On-stream handshake**: if Kiro emits a `session/request_permission` notification, an external coordinator responds with:
   ```json
   { "outcome": "selected", "optionId": "approve_for_session" }
   ```
   This approves the tool for the remainder of the session.

Both layers must be active. `--trust-all-tools` alone does not eliminate on-stream permission requests.

### Process lifecycle

- **Framing**: JSON-RPC 2.0 messages are newline-delimited on stdin (coordinator → Kiro) and stdout (Kiro → coordinator). Stderr carries diagnostics and should be captured separately.
- **Cancellation**: close stdin, send the `cancel` JSON-RPC notification, then await the stdout reader and drain stderr.
- **Session resume**: send `session/load` (not `session/resume`) with the session ID to resume a prior session.
- **Minimum CLI version**: not documented; runtime contract characterised from observed non-interactive CLI behaviour as of 2026-06-28.

## Format quirks / gotchas

- **`session/prompt` requires both `content` and `prompt` fields.** Sending only one of the two may be rejected or silently dropped. Both must be present in the params object.
- **Session resume uses `session/load`, not `session/resume`.** Sending `session/resume` will not resume a prior session.
- **`goal_complete` is the authoritative task-completion signal.** Watch for a `tool_call` notification whose tool name is `goal_complete`; that marks the task as done, independent of `turn_end`.
- **`-32603` after `goal_complete` is success, not failure.** If `goal_complete` was already emitted and a subsequent `session/prompt` returns JSON-RPC error `-32603 "failed to generate a response"`, treat the run as successful completion. Do not retry or surface this as an error.
- **`--trust-all-tools` alone is not sufficient for fully unattended operation.** Also auto-approve any `session/request_permission` handshakes on the stream with `{"outcome":"selected","optionId":"approve_for_session"}`.
- **`env` and `headers` in `mcpServers` are arrays of `{name, value}` objects, not maps.** `{"env": {"KEY": "value"}}` is wrong; `{"env": [{"name": "KEY", "value": "value"}]}` is correct.
- **Model IDs use Bedrock cross-region inference format** (e.g. `us.anthropic.claude-sonnet-4-5-v1:0`). Standard short Anthropic model IDs may not be accepted.
- **Skills are materialised before launch, not injected via ACP messages.** Write `<workdir>/.kiro/skills/<name>/SKILL.md` to disk before calling `kiro-cli acp`.
- **`AGENTS.md` in the working directory is always-on context.** Write it before launch; there is no ACP message to supply inline system-prompt content.

## Renderer notes (planned)

This harness is not yet an agent-manager sync target (Status: Reference — characterised from observed runtime contract).

When a renderer is implemented it should:

1. **Rules → memory**: write `<workdir>/AGENTS.md` before launching `kiro-cli acp`. Kiro honours this file as always-on context prepended to the system prompt.
2. **Skills**: materialise each skill as `<workdir>/.kiro/skills/<name>/SKILL.md` with standard `SKILL.md` frontmatter (`name`, `description`) before launch. The folder name must equal the `name` value.
3. **MCP**: pass servers in the `mcpServers` array inside `session/new`, not via any on-disk config file. Use `{name, command, args, env:[{name,value}]}` for stdio; `{type:"http"|"sse", name, url, headers:[{name,value}]}` for remote. Filter to transports advertised in `initialize`.
4. **Model**: set in `session/new` `model` field; update mid-session via `session/set_model { sessionId, modelId }`.
5. **Tool approval**: always launch with `--trust-all-tools` and auto-approve `session/request_permission` events on the stream.
6. **Session resume**: use `session/load`, not `session/resume`.
7. **Completion detection**: watch for `goal_complete` tool call; treat subsequent `-32603` errors as successful completion.

Files the renderer does **not** own: user-level Kiro configuration outside the working directory, any content in `AGENTS.md` not generated by the sync engine.

## Sources

- Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28.
- Agent Client Protocol (ACP) — JSON-RPC 2.0 over stdio; handshake (`initialize` → `session/new` → `session/prompt`) and notification (`session/update`) shapes observed generically across ACP-compliant harnesses.
- AWS Kiro — <https://kiro.dev> (vendor product page; configuration surface not verified against this reference as of 2026-06-28).
