# Qoder

Stable id: `qoder`
Display name: Qoder
Vendor: Qoder (identity not fully verified — see Quick reference)
Status: Reference — characterised from observed runtime contract; not yet an agent-manager sync target.

## Quick reference

| Field         | Value                                                                                   |
|---------------|-----------------------------------------------------------------------------------------|
| Stable id     | `qoder`                                                                                 |
| Display name  | Qoder                                                                                   |
| Vendor        | Qoder (vendor identity not fully verified; characterised from observed CLI behaviour)   |
| Status        | Reference — characterised from observed runtime contract; not yet an agent-manager sync target. |
| Binary        | `qodercli`                                                                              |
| Global root   | Not documented as of 2026-06-28 (see On-disk layout)                                   |
| Project root  | Not documented as of 2026-06-28 (see On-disk layout)                                   |
| Protocol      | ACP — JSON-RPC 2.0 over stdio, newline-framed                                          |

## On-disk layout

Not documented as of 2026-06-28 — this reference characterises Qoder from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

The one on-disk path observed in practice is the skills directory, which a coordinator materialises at:

```
<workdir>/
└── .qoder/
    └── skills/
        └── <name>/
            └── SKILL.md
```

No global configuration tree has been verified. Do not invent paths beyond what is listed here.

## Discovery precedence

Not documented as of 2026-06-28 — this reference characterises Qoder from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

The one discovery behaviour observed in practice: `AGENTS.md` in the working directory is honoured as always-on context (see Policies / Rules / Memory).

## Feature matrix

`agent-manager` does not yet have a sync renderer for Qoder. All Support values reflect the observed non-interactive contract seams, not a complete audit of the harness's own feature set.

| Feature           | Support              | Where it lands / notes                                               |
|-------------------|----------------------|----------------------------------------------------------------------|
| Rules             | n/a (no renderer)    | `AGENTS.md` in workdir is honoured as always-on context (observed)   |
| Skills            | n/a (no renderer)    | `<workdir>/.qoder/skills/<name>/SKILL.md` (observed seam)            |
| MCP               | n/a (no renderer)    | `mcpServers` array inside `session/new` ACP message (observed seam)  |
| Agents            | n/a (no renderer)    | Not verified against vendor documentation                            |
| Slash commands    | n/a (no renderer)    | Not verified against vendor documentation                            |
| Auth              | n/a (no renderer)    | Not verified against vendor documentation                            |
| Permissions       | n/a (no renderer)    | Bypass via `--yolo` flag; no in-band permission config observed      |
| Policies / Rules  | n/a (no renderer)    | `AGENTS.md` honoured; broader policy surface not verified            |

## Skills

### Location (observed)

```
<workdir>/.qoder/skills/<name>/SKILL.md
```

A coordinator materialises skill directories here before launching `qodercli`. No global skills path has been verified.

### Format

Standard Agent Skills shape: a directory named `<name>` containing a single `SKILL.md` file with YAML frontmatter. The following frontmatter keys are the minimum required by the Agent Skills open standard (<https://agentskills.io>); Qoder-specific extensions have not been verified.

| Key           | Required | Notes                                          |
|---------------|----------|------------------------------------------------|
| `name`        | yes      | Should match the directory name.               |
| `description` | yes      | Shown to the model as the skill's purpose.     |

### Minimal skill

```markdown
---
name: fix-lint
description: Apply lint fixes and summarise changes
---

Run the project linter, fix all auto-fixable issues, and list what changed.
```

### Discovery / invocation notes

Qoder discovers skills under `.qoder/skills/` in the working directory. How the model is informed of available skills (e.g. an `<available_skills>` tool injection similar to opencode) has not been verified against vendor documentation as of 2026-06-28.

## Sub-agents

Not documented as of 2026-06-28 — this reference characterises Qoder from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## MCP servers

MCP servers are supplied to Qoder **at session start**, injected inside the `session/new` ACP message as the `mcpServers` array. There is no persistent per-project config file for MCP that has been verified against vendor documentation.

### Transport variants

Two transport types are observed in the ACP `session/new` handshake:

**stdio (local subprocess)**

```json
{
  "name": "my-server",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-everything"],
  "env": [
    { "name": "MY_VAR", "value": "example-value" }
  ]
}
```

**Remote HTTP / SSE**

```json
{
  "type": "http",
  "name": "remote-server",
  "url": "https://mcp.example.com/mcp",
  "headers": [
    { "name": "Authorization", "value": "Bearer example-token" }
  ]
}
```

Both forms are elements of the `mcpServers` array in `session/new`. The array is filtered to the transport types advertised by the harness during the `initialize` handshake (via `agentCapabilities.mcpCapabilities.http` / `.sse`).

## Slash commands

Not documented as of 2026-06-28 — this reference characterises Qoder from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Authentication

Not documented as of 2026-06-28 — this reference characterises Qoder from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

Authentication credentials are not injected via the ACP protocol messages documented here; the mechanism by which `qodercli` authenticates with its model provider has not been verified.

## Permissions

Not documented as of 2026-06-28 — this reference characterises Qoder from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

In the observed non-interactive contract, `qodercli` is always launched with `--yolo`, which runs in bypass-permissions / auto-approval mode. Tool approval via the in-band ACP `session/request_permission` handshake is also observed (see Orchestration / headless invocation — Tool approval in headless mode). No file-based permission configuration has been verified.

## Policies / Rules / Memory

`AGENTS.md` in the working directory is honoured as always-on context. A coordinator should write project-level instructions to `<workdir>/AGENTS.md` before launching `qodercli`.

Broader policy and memory configuration (global rules files, subdirectory walking, in-config instructions keys) has not been verified against vendor documentation as of 2026-06-28.

Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28.

## Orchestration / headless invocation

### Non-interactive launch

```
qodercli --yolo --acp
```

- `--yolo` — bypass-permissions / auto-approval mode. Always required for headless use; there is no partial approval mode.
- `--acp` — selects the Agent Client Protocol (ACP) JSON-RPC 2.0 over stdio transport. This is the current flag form; a legacy `acp` subcommand was replaced by this flag.

No positional prompt argument is used. The prompt is delivered as a JSON-RPC message after the handshake (see Output stream protocol). There is no `--format` flag; the protocol is the format.

Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28.

### Output stream protocol

**Wire format:** JSON-RPC 2.0, newline-framed, over stdin/stdout. Each line is a complete JSON object (request, response, or notification).

**Handshake sequence (coordinator drives):**

1. **`initialize`** — coordinator sends; harness responds advertising capabilities, including `agentCapabilities.mcpCapabilities.http` and/or `.sse` (used to filter the MCP transport list).

2. **`session/new`** — coordinator sends with:

   ```json
   {
     "jsonrpc": "2.0",
     "method": "session/new",
     "params": {
       "cwd": "/abs/path/to/workdir",
       "sessionId": "example-session-id",
       "model": "example-model-id",
       "mcpServers": [ /* see MCP servers */ ]
     },
     "id": 1
   }
   ```

3. **`session/set_model`** (optional) — sent after `session/new` to override the model mid-session:

   ```json
   {
     "jsonrpc": "2.0",
     "method": "session/set_model",
     "params": { "sessionId": "example-session-id", "modelId": "example-model-id" },
     "id": 2
   }
   ```

4. **`session/prompt`** — delivers the user turn:

   ```json
   {
     "jsonrpc": "2.0",
     "method": "session/prompt",
     "params": {
       "sessionId": "example-session-id",
       "prompt": [ { "type": "text", "text": "Fix the failing tests." } ]
     },
     "id": 3
   }
   ```

   To resume a prior session: use `session/resume` instead of `session/new`, passing the prior `sessionId`.

**Inbound notifications (harness → coordinator)** — all arrive as `session/update` JSON-RPC notifications:

| `update.type` value    | Canonical category | Shape / notes                                                  |
|------------------------|--------------------|----------------------------------------------------------------|
| `agent_message_chunk`  | assistant text     | Streaming text fragment.                                       |
| `agent_thought_chunk`  | reasoning          | Internal reasoning fragment (if model emits it).              |
| `tool_call`            | tool call          | Tool invocation with name and arguments.                       |
| `tool_call_update`     | tool result        | Tool result; `state: "complete"` when the call is finished.   |
| `usage_update`         | usage              | Token counts for the current turn.                             |
| `turn_end` / `endTurn` | completion         | Signals the agent has finished the current prompt response.   |

### Model & reasoning at launch

- **Model:** passed as the `model` field in `session/new` and/or overridden via a subsequent `session/set_model` message (`{ sessionId, modelId }`).
- **Reasoning effort:** no dedicated reasoning control has been observed in the ACP contract as of 2026-06-28. Do not invent a flag or field.

Cross-reference Authentication for how `qodercli` authenticates with the model provider; that mechanism is not part of the ACP protocol.

### MCP at launch

MCP servers are injected inside the `session/new` message as the `mcpServers` array (see MCP servers for per-server shapes). The array is filtered by the coordinator to the transports advertised in the `initialize` response. No MCP config file is written to disk; the entire MCP surface is delivered in-band.

### Skills at launch

A coordinator materialises skills into `<workdir>/.qoder/skills/<name>/SKILL.md` before launching `qodercli`. Always-on context goes into `<workdir>/AGENTS.md`. Both paths must be in place before the `qodercli` process starts; they are not hot-reloaded mid-session.

Cross-reference Skills and Policies / Rules / Memory.

### Tool approval in headless mode

Two complementary mechanisms:

1. **`--yolo` flag** (launch-time) — runs in bypass-permissions / auto-approval mode. All tool calls proceed without an approval handshake. This is the primary headless mechanism.

2. **`session/request_permission` (in-band)** — if the harness sends a permission request despite `--yolo`, the coordinator auto-approves by responding:

   ```json
   {
     "jsonrpc": "2.0",
     "result": { "outcome": "selected", "optionId": "approve_for_session" },
     "id": <matching request id>
   }
   ```

   A coordinator should implement this handler defensively even when `--yolo` is in use.

### Process lifecycle

- **Framing:** JSON-RPC 2.0 newline-delimited on stdin (coordinator → harness) and stdout (harness → coordinator). Diagnostics on stderr.
- **Pipe behaviour quirk:** `qodercli` keeps its stdout and stdin pipes **open** after returning the `turn_end` / `endTurn` notification. A coordinator must apply a bounded drain grace (approximately 2 seconds) after the completion notification before force-closing the pipes. Do not treat EOF as the completion signal.
- **Cancellation:** close stdin, send a `session/cancel` request if the protocol exposes one, then await the stdout reader and stderr draining before terminating the process.
- **Session resume:** send `session/resume` with the prior `sessionId` instead of a new `session/new`.
- **Minimum CLI version:** not documented as of 2026-06-28. Verify the `--acp` flag is present before relying on this contract (the legacy `acp` subcommand form should be treated as unsupported).

## Format quirks / gotchas

- **`--acp` is a flag, not a subcommand.** The legacy `acp` positional subcommand was replaced by the `--acp` flag. Always use the flag form.
- **`--yolo` is mandatory for headless use.** There is no partial-approval mode; omitting it causes interactive permission prompts that block a non-TTY coordinator.
- **Stdout stays open after `turn_end`.** Do not close the reader on the first completion notification; apply a ~2 s drain grace window before force-closing.
- **MCP is in-band only.** No MCP config file is written to the workdir; the full `mcpServers` array is delivered inside `session/new`.
- **`mcpServers` is filtered by advertised capabilities.** Read `agentCapabilities.mcpCapabilities.http` / `.sse` from the `initialize` response before constructing the list; omit transports the harness does not support.
- **Skills must be on disk before launch.** There is no hot-reload mechanism; materialise `.qoder/skills/<name>/SKILL.md` before the process starts.
- **`AGENTS.md` in the workdir is always-on context.** Write project instructions there before launch.
- **`session/request_permission` may arrive even under `--yolo`.** Always implement the auto-approve handler defensively.
- **Vendor identity is not fully verified.** Do not hard-code vendor-specific URLs or documentation references; treat the binary name `qodercli` and the `--yolo --acp` argv as the stable contract surface.

## Renderer notes (planned)

`agent-manager` does not yet have a sync renderer for Qoder. When one is built, it should:

1. **Rules / memory** — write `<workdir>/AGENTS.md` with project-level instructions before spawning `qodercli`. This is the only always-on context path observed.
2. **Skills** — materialise `<workdir>/.qoder/skills/<name>/SKILL.md` for each skill before launch. Frontmatter must include at minimum `name` and `description`.
3. **MCP** — construct the `mcpServers` array and deliver it inside the `session/new` ACP message; do not write any MCP config file to disk. Filter the array to transports advertised in `initialize`.
4. **Model** — inject the model id as the `model` field in `session/new`. Use `session/set_model` for mid-session overrides.
5. **Tool approval** — always pass `--yolo` at launch and always implement the `session/request_permission` auto-approve response handler.
6. **Pipe drain** — after receiving `turn_end` / `endTurn`, wait up to ~2 s for the pipes to drain before force-closing; do not rely on EOF as the completion signal.
7. **Cancellation** — on cancel: close stdin, send a cancel signal if the protocol exposes one, await drain, then terminate the process.
8. **Files the renderer does not own** — any pre-existing `AGENTS.md` the user has committed; any user-created files under `.qoder/` not managed by the renderer.
9. **Files the renderer owns** — `<workdir>/AGENTS.md` (when it writes it); `<workdir>/.qoder/skills/<name>/SKILL.md` for each renderer-managed skill.

## Sources

- Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28. No vendor documentation URL has been verified.
- Agent Client Protocol (ACP) — JSON-RPC 2.0 over stdio; generic protocol reference only; vendor identity not fully verified.
- Agent Skills open standard — <https://agentskills.io> (skill `SKILL.md` + frontmatter shape).
