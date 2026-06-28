# OpenClaw

Stable id: `openclaw`
Display name: OpenClaw
Vendor: Not documented
Status: Reference — characterised from observed runtime contract; not yet an agent-manager sync target.

## Quick reference

| Field         | Value                                                                              |
|---------------|------------------------------------------------------------------------------------|
| Stable id     | `openclaw`                                                                         |
| Display name  | OpenClaw                                                                           |
| Vendor        | Not documented                                                                     |
| Global root   | Not documented                                                                     |
| Project root  | Not documented                                                                     |
| Config format | Not documented                                                                     |
| Status        | Reference — characterised from observed runtime contract; not yet an agent-manager sync target. |

## On-disk layout

Not documented as of 2026-06-28 — this reference characterises OpenClaw from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

The only on-disk paths confirmed by observed runtime behaviour are:

- **Skills directory** (project-scoped): `<workdir>/skills/<name>/SKILL.md` — an external
  coordinator materialises skill directories here before launch.
- **Always-on context** (project-scoped): `AGENTS.md` in the working directory — loaded
  by OpenClaw as always-on context on every run.
- **MCP config** (per-run): a synthesised config file at the path supplied via
  `OPENCLAW_CONFIG_PATH` — written by an external coordinator before each invocation.

## Discovery precedence

Not documented as of 2026-06-28 — this reference characterises OpenClaw from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

The only load-order detail confirmed by observed runtime behaviour:

1. **Per-run MCP config** (`OPENCLAW_CONFIG_PATH`) — coordinator-supplied, takes effect for
   the current invocation only.
2. **User-global config** (`$include` via `OPENCLAW_INCLUDE_ROOTS`) — optional inclusion of
   a user-level config into the per-run config file.
3. **Working-directory context** (`AGENTS.md`, `skills/`) — loaded from `--cwd` / `cmd.Dir`.

## Feature matrix

`agent-manager` does not yet sync to OpenClaw; all features are `n/a (no renderer)`. Observed
seams that a future renderer could target are noted.

| Feature          | Support             | Where it lands / observed seam                             |
|------------------|---------------------|------------------------------------------------------------|
| Rules            | n/a (no renderer)   | `AGENTS.md` in the working directory (observed)            |
| Skills           | n/a (no renderer)   | `<workdir>/skills/<name>/SKILL.md` (observed)              |
| MCP              | n/a (no renderer)   | `OPENCLAW_CONFIG_PATH` per-run config file (observed)      |
| Agents           | n/a (no renderer)   | Registered via `openclaw agents add/update` (observed)     |
| Slash commands   | n/a (no renderer)   | Not documented                                             |
| Auth             | n/a (no renderer)   | Not documented                                             |
| Permissions      | n/a (no renderer)   | Not documented                                             |
| Policies / Rules | n/a (no renderer)   | `AGENTS.md` in working directory (observed)                |

## Skills

### Location

Skills are materialised into the working directory by an external coordinator before each
`openclaw agent` invocation:

```
<workdir>/skills/<name>/SKILL.md
```

OpenClaw discovers skills from this directory at startup. The directory is not created
automatically; an external coordinator is responsible for writing it.

### Format

Each skill is a directory containing a `SKILL.md` file with YAML frontmatter. The standard
Agent Skills shape (`agentskills.io`) is used:

| Key           | Required | Notes                                          |
|---------------|----------|------------------------------------------------|
| `name`        | yes      | Must match the directory name.                 |
| `description` | yes      | Shown to the model as the skill's purpose.     |

Additional frontmatter keys (`license`, `compatibility`, `metadata`) may be included; their
handling by OpenClaw has not been verified.

### Minimal skill

```markdown
---
name: code-review
description: Review code for correctness and style
---

Focus on:
- Logic errors and edge cases
- Naming and clarity
- Consistency with existing patterns
```

### Always-on context

`AGENTS.md` placed in the working directory is treated as always-on context by OpenClaw
(i.e., it is prepended to the system prompt on every turn). An external coordinator writes
this file before invocation to supply standing instructions.

## Sub-agents

Not documented as of 2026-06-28 — this reference characterises OpenClaw from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

## MCP servers

MCP servers for a single run are supplied through a coordinator-synthesised config file.
The path to this file is passed via the `OPENCLAW_CONFIG_PATH` environment variable.

A user-global config can be included into the per-run file with `$include` directives; the
roots available for inclusion are listed in the `OPENCLAW_INCLUDE_ROOTS` environment variable.

The schema and key names used within the config file are not documented as of 2026-06-28.
An external coordinator writes the entire file for each run; the ambient/user-global config
is not merged automatically unless `OPENCLAW_INCLUDE_ROOTS` and `$include` are used
explicitly.

## Slash commands

Not documented as of 2026-06-28 — this reference characterises OpenClaw from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

## Authentication

Not documented as of 2026-06-28 — this reference characterises OpenClaw from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

## Permissions

Not documented as of 2026-06-28 — this reference characterises OpenClaw from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

In `agent` mode, OpenClaw runs autonomously with no on-stream tool-approval handshake
observed. There is no `--dangerously-skip-permissions` equivalent flag documented; the
`agent` subcommand appears to run fully unattended without requiring one.

## Policies / Rules / Memory

### Always-on context

`AGENTS.md` in the working directory is loaded by OpenClaw on every run as always-on
context (the open `AGENTS.md` standard). An external coordinator is responsible for writing
this file to the working directory before each invocation.

### Wider native config surface

The native policy, rules, and memory configuration surface (beyond `AGENTS.md`) is not
documented as of 2026-06-28 — this reference characterises OpenClaw from its observed
non-interactive runtime contract (see Orchestration / headless invocation); the native
configuration surface has not been verified against vendor documentation.

## Orchestration / headless invocation

### Non-interactive launch

```
openclaw agent [--local] --json --session-id <id> [--timeout <secs>] [--agent <agent-id>] --message "<prompt>"
```

- `agent` is the mandatory subcommand for non-interactive, headless execution.
- `--json` selects machine-readable output on stdout.
- `--session-id <id>` identifies the session (an external coordinator supplies a stable id
  to enable session resumption).
- `--timeout <secs>` optionally caps the run duration.
- `--agent <agent-id>` selects the registered OpenClaw agent to use (see Model & reasoning
  at launch); omitted when the user supplies `--agent` through custom args.
- `--message "<prompt>"` delivers the user prompt. System-prompt content is prepended to
  the prompt value inline (no separate `--system-prompt` flag exists at this subcommand).
- `--local` is present when the coordinator targets embedded/local execution. It is
  **dropped** when operating in "gateway" mode so that OpenClaw dials its configured remote
  Gateway instead.

### Output stream protocol

OpenClaw writes its `--json` output to **stdout**. Stderr carries log overflow (security
warnings, tool errors) and is not part of the structured output.

The stdout output is in one of two formats; a consumer tries the bulk-JSON fast path first,
then falls back to NDJSON scanning:

#### Bulk JSON (dominant format as of 2026-06-28)

A single JSON blob, often pretty-printed across multiple lines. Leading non-JSON log lines
are stripped before parsing.

```json
{
  "payloads": [
    { "text": "..." }
  ],
  "meta": {
    "durationMs": 1234,
    "agentMeta": {
      "sessionId": "...",
      "model": "...",
      "usage": {
        "input": 100,
        "output": 200,
        "cacheRead": 0,
        "cacheWrite": 0
      }
    }
  }
}
```

`meta.agentMeta.model` carries the actual LLM identifier (e.g. `deepseek-chat`,
`claude-sonnet-4`) reported by the runtime, which may differ from the OpenClaw agent id
passed via `--agent`.

Usage field names are multi-aliased across protocol versions; a consumer must check all
variants:

| Token class       | Field aliases                                                                   |
|-------------------|---------------------------------------------------------------------------------|
| Input tokens      | `input`, `inputTokens`, `input_tokens`                                          |
| Output tokens     | `output`, `outputTokens`, `output_tokens`                                       |
| Cache read tokens | `cacheRead`, `cachedInputTokens`, `cached_input_tokens`, `cache_read`, `cache_read_input_tokens` |
| Cache write tokens| `cacheWrite`, `cacheCreationInputTokens`, `cache_creation_input_tokens`, `cache_write` |

#### NDJSON (forward-compatibility / fallback)

One JSON event per line on stdout. Event types:

```json
{"type":"step_start","sessionId":"..."}
{"type":"text","text":"...","sessionId":"..."}
{"type":"tool_use","tool":"<name>","callId":"...","input":{...},"sessionId":"..."}
{"type":"tool_result","tool":"<name>","callId":"...","text":"...","sessionId":"..."}
{"type":"error","text":"..." ,"sessionId":"..."}
{"type":"error","error":{"name":"...","data":{"message":"..."}},"sessionId":"..."}
{"type":"lifecycle","phase":"error"|"failed"|"cancelled","text":"...","sessionId":"..."}
{"type":"step_finish","usage":{...},"sessionId":"..."}
```

Canonical event mapping:

| Category          | Event type(s)                                      |
|-------------------|----------------------------------------------------|
| Assistant text    | `text` (`.text` field)                             |
| Tool call         | `tool_use` (`.tool`, `.callId`, `.input`)          |
| Tool result       | `tool_result` (`.tool`, `.callId`, `.text`)        |
| Usage             | `step_finish` (`.usage`) and/or bulk `meta.agentMeta.usage` |
| Error             | `error`; `lifecycle` with `.phase` of `error`, `failed`, or `cancelled` |
| Completion        | stream end after `step_finish` (NDJSON) or end of bulk blob |

### Model & reasoning at launch

OpenClaw has **no `--model` flag**. The model is bound when an agent is registered:

```
openclaw agents add   --model <model-id> [--name <display-name>] <agent-id>
openclaw agents update --model <model-id> <agent-id>
openclaw agents list
```

At run time, the agent (and therefore its bound model) is selected with `--agent <agent-id>`.
The model identifier carried by an external coordinator is thus an **agent id**, not a model
slug. The actual LLM identifier is reported back at run time in `meta.agentMeta.model`
within the bulk JSON result.

Reasoning effort: not documented.

### MCP at launch

An external coordinator writes a per-run config file before each invocation and points
OpenClaw at it with the `OPENCLAW_CONFIG_PATH` environment variable. This file is the
exclusive mechanism for supplying MCP servers for a single run.

To allow `$include` of a user-global config within that file, set `OPENCLAW_INCLUDE_ROOTS`
to the directory roots OpenClaw is permitted to include from.

The per-run config file is fully coordinator-owned and is written fresh for each run; no
ambient or user-level MCP config is inherited unless `OPENCLAW_INCLUDE_ROOTS` and `$include`
are used explicitly.

### Skills at launch

An external coordinator materialises skills into `<workdir>/skills/<name>/SKILL.md` (one
directory per skill) before launching `openclaw agent`. Always-on context is written to
`AGENTS.md` in the same working directory. Both paths must exist before `openclaw agent`
is invoked; OpenClaw does not create them. (Cross-reference Skills and Policies / Rules /
Memory.)

### Tool approval in headless mode

OpenClaw runs fully autonomously in `agent` mode. No on-stream tool-approval handshake is
observed. An external coordinator does not need to answer any approval prompt to keep the
run unattended.

### Process lifecycle

- **Prompt delivery**: via `--message` in argv (not stdin).
- **Output**: structured result on stdout (bulk JSON or NDJSON); diagnostics on stderr.
- **Session resumption**: pass `--session-id <prior-id>` to continue a prior session.
- **Cancellation**: close the stdout reader; os/exec cancels the process when the run
  context is cancelled. The process has up to 10 seconds to exit cleanly before a forced
  termination.
- **Minimum CLI version**: `>= 2026.5.5` is enforced **before every run** (not only at
  agent registration). Builds older than `2026.5.5` wrote their `--json` output to stderr
  instead of stdout; such builds produce no parseable stdout and break the output parser.
  The version is checked by running `openclaw --version` and parsing the three-segment
  dotted version it prints. If the check fails, an actionable error is returned immediately
  and the run is aborted: run `openclaw update` to upgrade.

## Format quirks / gotchas

- **Two output formats, one stdout stream.** The consumer must attempt whole-buffer
  bulk-JSON parsing first; fall back to line-by-line NDJSON scanning only if the bulk parse
  fails. Do not assume the output is NDJSON because some lines look like JSON events.
- **Pretty-printed bulk JSON spans multiple lines.** Do not split on newlines before
  attempting the bulk parse; read the full stdout buffer, then parse.
- **Leading log lines may precede the bulk JSON blob.** Strip lines that do not begin with
  `{` before attempting the bulk parse.
- **Minimum version check runs before every invocation.** Do not cache the version result
  across runs if the binary might be updated between runs.
- **`--model` is not accepted by `openclaw agent`.** Pass `--agent <id>` instead. The
  model is bound at registration time. The coordinator's "model" field maps to an OpenClaw
  agent id, not a model slug.
- **`--system-prompt` is not accepted by `openclaw agent`.** Prepend system-prompt content
  to the `--message` value before invocation.
- **`--local` must be dropped for gateway-mode runs.** It is a blocked arg; do not allow
  user-configured custom args to re-introduce it when gateway mode is active. Mode is the
  single source of truth.
- **`meta.agentMeta.model` is the runtime LLM id, not the agent id.** Use it for usage
  attribution; use `opts.Model` (the agent id) only as a fallback when the runtime value is
  absent.
- **Usage field names are multi-aliased.** Check all aliases listed in Output stream
  protocol; do not assume a single canonical name.
- **`lifecycle` events with phase `"cancelled"` indicate a failed run**, not a clean stop.
  Treat them the same as `"error"` / `"failed"` phases.
- **AGENTS.md must be in the working directory, not a parent.** Write it to `cmd.Dir`
  exactly; OpenClaw loads it from there.
- **Skills directory is `skills/` (singular root, plural dir name), not `.openclaw/skills/`.**
  The path is `<workdir>/skills/<name>/SKILL.md`; there is no hidden dot-directory prefix.

## Renderer notes (planned)

OpenClaw is not yet an agent-manager sync target. When a renderer is implemented it should:

1. **Version gate**: run `openclaw --version` before each invocation; reject builds below
   `2026.5.5` with an upgrade hint.
2. **Agent selection**: treat the coordinator's `model` field as an OpenClaw agent id, not
   a model slug. Enumerate available agent ids with `openclaw agents list` and surface them
   as the "model" picker.
3. **Rules / memory**: write `AGENTS.md` to the working directory before each invocation.
   Do not write `CLAUDE.md`; native compatibility with that filename is not documented.
4. **Skills**: materialise each skill as `<workdir>/skills/<name>/SKILL.md` before launch.
   Use the standard `SKILL.md` shape (`name` + `description` in YAML frontmatter, Markdown
   body).
5. **MCP**: write a per-run config file and set `OPENCLAW_CONFIG_PATH` to its path. Do not
   write into an ambient user config. If user-global config inclusion is needed, set
   `OPENCLAW_INCLUDE_ROOTS` and add `$include` directives in the per-run file.
6. **Output parsing**: attempt whole-buffer bulk-JSON parse first; fall back to NDJSON
   line scanner. Do not treat the two paths as interchangeable.
7. **Cancellation**: close the stdout reader, then wait up to 10 seconds for the process to
   exit; send a hard kill if it does not.
8. **Do not own**: `openclaw agents` registrations are outside the renderer's scope — agents
   are registered once at setup time, not per-run.

## Sources

- Runtime contract — characterised from observed non-interactive CLI behaviour as of
  2026-06-28.
