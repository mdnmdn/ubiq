# Cursor Agent

Stable id: `cursor`
Display name: Cursor Agent
Vendor: Cursor (Anysphere)
Status: Reference — characterised from observed runtime contract; not yet an agent-manager sync target.

## Quick reference

| Field         | Value                                                                                             |
|---------------|---------------------------------------------------------------------------------------------------|
| Stable id     | `cursor`                                                                                          |
| Display name  | Cursor Agent                                                                                      |
| Vendor        | Cursor (Anysphere)                                                                                |
| Binary        | `cursor-agent`                                                                                    |
| Global root   | Not verified — see On-disk layout                                                                 |
| Project root  | `<workdir>/.cursor/`                                                                              |
| Config format | JSON (`mcp.json`), Markdown (`SKILL.md`, `AGENTS.md`)                                            |
| Status        | Reference — characterised from observed runtime contract; not yet an agent-manager sync target.   |

## On-disk layout

Not documented as of 2026-06-28 — this reference characterises Cursor Agent from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

The following project-level paths are confirmed from observed runtime behaviour.

### Project (`<workdir>/`)

```
<workdir>/
├── AGENTS.md                          # always-on context (open standard)
└── .cursor/
    ├── mcp.json                       # MCP server definitions (top-level key: mcpServers)
    └── skills/<name>/
        └── SKILL.md                   # skill definition (one folder per skill)
```

### Data dir (`$CURSOR_DATA_DIR/`)

An external coordinator supplies an isolated data directory at runtime to bypass
interactive trust and approval prompts:

```
$CURSOR_DATA_DIR/
├── .workspace-trusted                 # marker file; presence signals workspace trust
└── projects/<slug>/
    └── mcp-approvals.json             # pre-seeded MCP approval keys
```

Approval keys are computed as `sha256(path + server)[:16]` (hex).

### Global

Not verified against vendor documentation as of 2026-06-28.

## Discovery precedence

Not documented as of 2026-06-28 — this reference characterises Cursor Agent from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

The following loading behaviour is confirmed from observed runtime contract:

1. **`AGENTS.md`** in the working directory — always-on context, loaded on every run.
2. **`.cursor/mcp.json`** in the working directory — MCP server definitions (`mcpServers` key).
3. **`.cursor/skills/<name>/SKILL.md`** in the working directory — skills, one directory per skill.
4. **`$CURSOR_DATA_DIR`** — coordinator-supplied data directory for approval state and workspace
   trust; bypasses interactive prompts.

## Feature matrix

| Feature          | Support           | Where it lands                                                                    |
|------------------|-------------------|-----------------------------------------------------------------------------------|
| Rules            | n/a (no renderer) | `<workdir>/AGENTS.md` (always-on context, observed)                               |
| Skills           | n/a (no renderer) | `<workdir>/.cursor/skills/<name>/SKILL.md` (observed)                             |
| MCP              | n/a (no renderer) | `<workdir>/.cursor/mcp.json` → `mcpServers` key (observed)                        |
| Agents           | n/a (no renderer) | Not documented as of 2026-06-28                                                   |
| Slash commands   | n/a (no renderer) | Not documented as of 2026-06-28                                                   |
| Auth             | n/a (no renderer) | Not documented as of 2026-06-28                                                   |
| Permissions      | n/a (no renderer) | `--yolo` flag; `$CURSOR_DATA_DIR/projects/<slug>/mcp-approvals.json` (observed)   |
| Policies / Rules | n/a (no renderer) | `<workdir>/AGENTS.md` (observed)                                                  |

## Skills

### Locations

```
<workdir>/.cursor/skills/<name>/SKILL.md    # project (observed discovery path)
```

Global skill location is not verified against vendor documentation as of 2026-06-28.

### Format

Markdown file with YAML frontmatter. Confirmed keys from the Agent Skills open standard
(<https://agentskills.io>):

| Key           | Required | Notes                                               |
|---------------|----------|-----------------------------------------------------|
| `name`        | yes      | Must match the folder name.                         |
| `description` | yes      | Shown to the model to describe the skill.           |

### Minimal skill

```markdown
---
name: git-release
description: Create consistent releases and changelogs
---

## What I do
- Draft release notes from merged PRs
- Propose a version bump
- Provide a copy-pasteable release command
```

### Discovery notes

- A coordinator materialises skills into `<workdir>/.cursor/skills/<name>/SKILL.md` before launch.
- Always-on context goes into `AGENTS.md` in the working directory (cross-reference
  Policies / Rules / Memory).
- `cursor-agent` does not support `--system-prompt`; skill content and policy instructions
  must be delivered via on-disk files.

## Sub-agents

Not documented as of 2026-06-28 — this reference characterises Cursor Agent from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

## MCP servers

MCP servers are defined in `<workdir>/.cursor/mcp.json` under the top-level key `mcpServers`.

### Shape

```json
{
  "mcpServers": {
    "<server-name>": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-example"],
      "env": { "MY_VAR": "value" }
    }
  }
}
```

The `mcpServers` key mirrors the standard MCP JSON convention used by Claude Code and other
compatible harnesses.

### Transports

- **stdio** — launch a subprocess; fields: `command` (string), `args` (string[]), optional `env`.
- **HTTP/SSE** — pass `url` in place of `command`/`args`. Exact field shape not verified against
  vendor documentation as of 2026-06-28.

### Headless approval bypass

An external coordinator pre-seeds MCP approval state into `$CURSOR_DATA_DIR` before launch:

- `projects/<slug>/mcp-approvals.json` — approval keys, each computed as
  `sha256(path + server)[:16]` (hex).
- `.workspace-trusted` — marker file at the `$CURSOR_DATA_DIR` root; suppresses Cursor's
  interactive workspace-trust prompts.

This combination allows `cursor-agent` to load and invoke MCP servers in headless mode without
any interactive dialogue. (Cross-reference Permissions and Orchestration / headless invocation.)

## Slash commands

Not documented as of 2026-06-28 — this reference characterises Cursor Agent from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

## Authentication

Not documented as of 2026-06-28 — this reference characterises Cursor Agent from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

## Permissions

### Headless bypass

`--yolo` is the auto-approve flag for non-interactive runs. When passed, `cursor-agent` executes
all tools without confirmation; there is no on-stream approval handshake for a coordinator to
answer.

### MCP approval state

An external coordinator bypasses Cursor's interactive MCP approval prompts by:

1. Setting `CURSOR_DATA_DIR` to an isolated coordinator-managed directory (one per run or session).
2. Writing `$CURSOR_DATA_DIR/projects/<slug>/mcp-approvals.json` containing pre-computed approval
   keys (`sha256(path + server)[:16]` hex).
3. Writing `$CURSOR_DATA_DIR/.workspace-trusted` to signal workspace trust.

`CURSOR_DATA_DIR` must be isolated per concurrent run to avoid approval-key collisions.

### Native permission configuration

Not documented as of 2026-06-28 — this reference characterises Cursor Agent from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

## Policies / Rules / Memory

`cursor-agent` honours `AGENTS.md` in the working directory as always-on context (prepended to
the system prompt on every run). This follows the open-standard `AGENTS.md` convention.

- **Scope:** project-level working-directory file confirmed. Global rules file location is not
  verified as of 2026-06-28.
- **Injection mechanism:** file is read from disk at launch; no CLI flag is required.
- **`--system-prompt` is not supported.** All always-on instructions must be injected via
  `AGENTS.md` or skill files.

## Orchestration / headless invocation

### Non-interactive launch

```
cursor-agent -p "<prompt>" --output-format stream-json --yolo \
  [--workspace <cwd>] [--model <id>] [--resume <session-id>]
```

| Flag                          | Required | Notes                                                     |
|-------------------------------|----------|-----------------------------------------------------------|
| `-p <prompt>`                 | yes      | The user prompt as a CLI argument; not written to stdin.  |
| `--output-format stream-json` | yes      | Selects machine-readable NDJSON output on stdout.         |
| `--yolo`                      | yes      | Auto-approves all tools; no interactive confirmation.     |
| `--workspace <cwd>`           | no       | Sets the working directory; defaults to process cwd.      |
| `--model <id>`                | no       | Model identifier (see Model & reasoning at launch).       |
| `--resume <session-id>`       | no       | Resume a prior session by session ID.                     |

`--system-prompt` and `--max-turns` are **not supported**. Inject instructions via `AGENTS.md`
and `.cursor/skills/`.

The flags `-p`, `--output-format`, and `--yolo` are coordinator-reserved and may not be
overridden by user-configured custom args.

### Output stream protocol

Newline-delimited JSON (NDJSON) on stdout. Each line MAY be prefixed with `stdout:` or `stderr:`
— an external coordinator strips the prefix (case-insensitive, optional whitespace and `:`/`=`
separator) before JSON parsing.

**Event types:**

```json
{"type":"system","subtype":"init","session_id":"<id>"}
{"type":"system","subtype":"error","error":"<msg>","detail":"<detail>"}
{"type":"assistant","session_id":"<id>","message":{"model":"<id>","content":[
  {"type":"text","text":"<prose>"},
  {"type":"output_text","text":"<prose>"},
  {"type":"thinking","text":"<reasoning>"},
  {"type":"tool_use","id":"<call-id>","name":"<tool>","input":{}}
],"usage":{}}}
{"type":"tool_use","session_id":"<id>","tool_name":"<tool>","tool_id":"<call-id>","parameters":{}}
{"type":"tool_result","session_id":"<id>","tool_id":"<call-id>","output":"<text>"}
{"type":"text","session_id":"<id>","part":{"text":"<prose>"}}
{"type":"step_finish","session_id":"<id>","part":{"tokens":{"input":0,"output":0,"cache":{"read":0}},"cost":0.0}}
{"type":"result","session_id":"<id>","model":"<id>","result":"<final text>","is_error":false,
  "inputTokens":0,"outputTokens":0,"cacheReadTokens":0,"cacheWriteTokens":0,
  "usage":{},"total_cost_usd":0.0}
{"type":"error","error":"<msg>","detail":"<detail>"}
```

**Canonical event mapping:**

| Category       | Event type(s)                                                    | Notes                                                                                              |
|----------------|------------------------------------------------------------------|----------------------------------------------------------------------------------------------------|
| Status         | `system` (subtype `init`)                                        | Signals run started.                                                                               |
| Assistant text | `assistant` (content blocks `text` / `output_text`) and `text`  | Both forms carry assistant prose; handle both.                                                     |
| Reasoning      | `assistant` (content block `thinking`)                           | Extended thinking block.                                                                           |
| Tool call      | `assistant` (content block `tool_use`) and top-level `tool_use` | Both forms carry call ID and input; handle both.                                                   |
| Tool result    | `tool_result`                                                    | Carries call ID and output string.                                                                 |
| Usage          | `step_finish` (per-step) and `result` (session totals)           | Prefer `result` usage when present; fall back to accumulated `step_finish` counts otherwise.       |
| Error          | `system` (subtype `error`) and `error`                           | Both carry message in `error` and `detail` fields.                                                 |
| Completion     | `result`                                                         | Final event; an external coordinator cancels the run context immediately on receipt.               |

**Token usage field variants:**

Token counts appear in both snake_case and camelCase across top-level result fields and the nested
`usage` object. A coordinator applies first-non-zero reconciliation per field:

| Logical field  | Top-level `result` fields  | Nested `usage` object fields                                                                                      |
|----------------|----------------------------|-------------------------------------------------------------------------------------------------------------------|
| Input tokens   | `inputTokens`              | `input_tokens`, `inputTokens`                                                                                     |
| Output tokens  | `outputTokens`             | `output_tokens`, `outputTokens`                                                                                   |
| Cache read     | `cacheReadTokens`          | `cached_input_tokens`, `cachedInputTokens`, `cacheReadTokens`, `cache_read_input_tokens`, `cacheReadInputTokens`  |
| Cache write    | `cacheWriteTokens`         | `cacheWriteTokens`, `cache_creation_input_tokens`, `cacheCreationInputTokens`                                     |

When the `result` event carries any non-zero usage field (top-level or nested `usage` object),
those session totals take precedence over accumulated `step_finish` per-step counts.

### Model & reasoning at launch

- **Model:** `--model <id>`. The exact model ID format (with or without provider prefix) is not
  verified against vendor documentation as of 2026-06-28.
- **Reasoning effort:** not exposed. No flag, env var, or config key for reasoning effort has been
  observed.

### MCP at launch

MCP servers are read from `<workdir>/.cursor/mcp.json` (`mcpServers` key). An external coordinator
prepares this file before launch and sets `CURSOR_DATA_DIR` to an isolated directory containing:

- `projects/<slug>/mcp-approvals.json` — pre-seeded approval keys (`sha256(path + server)[:16]`
  hex); bypasses Cursor's interactive per-server approval prompts.
- `.workspace-trusted` — marker file at the data-dir root; suppresses interactive workspace-trust
  prompts.

No `--mcp` or inline MCP flag is available. The file must be written to disk before the process
is launched. (Cross-reference MCP servers and Permissions.)

### Skills at launch

A coordinator materialises skills into `<workdir>/.cursor/skills/<name>/SKILL.md` before invoking
`cursor-agent`. One directory per skill; each `SKILL.md` carries YAML frontmatter with at least
`name` and `description`. (Cross-reference Skills.)

Always-on context is delivered via `AGENTS.md` in the working directory. (Cross-reference
Policies / Rules / Memory.)

### Tool approval in headless mode

`--yolo` auto-approves all tool calls. There is no on-stream approval handshake; a coordinator
does not need to respond to approval events. The flag is coordinator-reserved and always injected
at launch.

### Process lifecycle

- **Prompt delivery:** prompt is a CLI argument (`-p <prompt>`), not written to stdin.
- **Output framing:** NDJSON on stdout; diagnostics on stderr. Lines may carry a `stdout:` or
  `stderr:` prefix that must be stripped before JSON parsing.
- **Session ID:** carried in `session_id` on most events. Pass `--resume <id>` to continue a
  prior session.
- **Background worker:** `cursor-agent` keeps a background worker alive after emitting the final
  `result` event. A coordinator must cancel its run context immediately on receiving `result` and
  apply a ~500 ms wait-delay before terminating the process to allow graceful shutdown.
- **Cancellation:** close the stdout reader, wait ~500 ms, then terminate the process. There is no
  documented in-band cancel message.
- **Minimum CLI version:** not documented as of 2026-06-28.

## Format quirks / gotchas

- **Strip line prefixes before parsing.** Each stdout line MAY arrive as `stdout: {...}` or
  `stderr: {...}`. Strip the prefix (case-insensitive, optional whitespace and `=`/`:` separator)
  before calling JSON unmarshal.
- **Two assistant event shapes coexist.** Text and tool_use blocks appear both inside
  `assistant.message.content` and as standalone top-level `tool_use` events. A coordinator must
  handle both forms.
- **`result` is the protocol boundary, not EOF.** Treat receipt of the `result` event as the
  signal to stop reading and cancel the run; do not wait for the process to exit on its own.
- **Background worker survives `result`.** Cancel immediately on `result` and use a ~500 ms
  wait-delay before hard-termination. Without the wait, the process may be killed mid-cleanup.
- **Token usage has dual field names.** Both snake_case and camelCase variants of every usage
  field may appear in the same event. Use first-non-zero reconciliation per field.
- **Session totals beat per-step counts.** If the `result` event includes any usage field, those
  session totals supersede accumulated `step_finish` counts; fall back to step accumulation only
  when the `result` event carries no usage.
- **`--system-prompt` is not supported.** All instructions must be injected via `AGENTS.md` and
  `.cursor/skills/`.
- **`--max-turns` is not supported.** No flag to cap agentic iterations has been observed.
- **`--yolo`, `-p`, and `--output-format` are coordinator-reserved.** User-configured custom args
  may not override these three flags.
- **`CURSOR_DATA_DIR` must be isolated per concurrent run.** Sharing a data directory between
  parallel runs risks approval-key collisions and workspace-trust state corruption.
- **No inline MCP config.** MCP servers must be written to `<workdir>/.cursor/mcp.json` on disk
  before launch; there is no env-var or flag for inline MCP configuration.

## Renderer notes (planned)

`agent-manager`'s Cursor Agent renderer does not yet exist. When implemented, it should:

1. **Rules → always-on context:** write `<workdir>/AGENTS.md` before launch. This is the sole
   observed injection point for always-on instructions.
2. **Skills:** materialise skills into `<workdir>/.cursor/skills/<name>/SKILL.md` before launch.
   One directory per skill; frontmatter carries at least `name` and `description`.
3. **MCP:** write `<workdir>/.cursor/mcp.json` with a top-level `mcpServers` object before
   launch. Prepare an isolated `CURSOR_DATA_DIR` with pre-seeded approval keys and the
   `.workspace-trusted` marker; set the env var before spawning.
4. **Permissions:** always pass `--yolo`; populate `$CURSOR_DATA_DIR/projects/<slug>/mcp-approvals.json`
   with keys computed as `sha256(path + server)[:16]` hex.
5. **Model:** pass `--model <id>` at launch.
6. **Session resume:** pass `--resume <session-id>` when continuing a prior session.
7. **Lifecycle:** cancel run context on receipt of `result`; apply ~500 ms wait-delay before
   hard-termination.
8. **Files the renderer owns:** `<workdir>/AGENTS.md`, `<workdir>/.cursor/mcp.json`,
   `<workdir>/.cursor/skills/` (entire subtree), and the coordinator-managed `$CURSOR_DATA_DIR`.
9. **Files the renderer must not touch:** any other Cursor IDE configuration or settings files
   outside the above paths.

## Sources

- Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28.
- Cursor documentation — <https://docs.cursor.com> (vendor reference; specific config surface not
  verified against this reference as of 2026-06-28).
