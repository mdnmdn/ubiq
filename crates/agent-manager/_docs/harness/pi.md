# pi

Stable id: `pi`
Display name: pi
Vendor: Not documented
Status: Reference — characterised from observed runtime contract; not yet an agent-manager sync target.

## Quick reference

| Field          | Value                                                                          |
|----------------|--------------------------------------------------------------------------------|
| Stable id      | `pi`                                                                           |
| Display name   | pi                                                                             |
| Vendor         | Not documented                                                                 |
| Status         | Reference — characterised from observed runtime contract; not yet an agent-manager sync target. |
| Global root    | Not documented as of 2026-06-28                                                |
| Project root   | `<workdir>/.pi/` (skills observed here; broader layout not verified)           |
| Config format  | Not documented as of 2026-06-28                                                |

## On-disk layout

Not documented as of 2026-06-28 — this reference characterises the `pi` CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

The one on-disk location confirmed by observation is the skills directory:

```
<workdir>/
└── .pi/
    └── skills/
        └── <name>/
            └── SKILL.md   # standard Agent Skills shape; YAML frontmatter + Markdown body
```

An `AGENTS.md` file in the working directory is honoured as always-on context (see Policies / Rules / Memory).

## Discovery precedence

Not documented as of 2026-06-28 — this reference characterises the `pi` CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Feature matrix

| Feature          | Support             | Where it lands                                                        |
|------------------|---------------------|-----------------------------------------------------------------------|
| Rules            | n/a (no renderer)   | `AGENTS.md` in the working directory (always-on context, observed)    |
| Skills           | n/a (no renderer)   | `<workdir>/.pi/skills/<name>/SKILL.md` (observed)                    |
| MCP              | n/a (no renderer)   | Not documented / no observed launch seam                              |
| Agents           | n/a (no renderer)   | Not documented as of 2026-06-28                                       |
| Slash commands   | n/a (no renderer)   | Not documented as of 2026-06-28                                       |
| Auth             | n/a (no renderer)   | Not documented as of 2026-06-28                                       |
| Permissions      | n/a (no renderer)   | Not documented as of 2026-06-28                                       |
| Policies / Rules | n/a (no renderer)   | `AGENTS.md` in the working directory (observed seam only)             |

Note on observed seams: the `-p` flag suppresses interactive prompts (autonomous tool execution); `--append-system-prompt` injects extra instructions at launch. No on-stream tool-approval handshake has been observed.

## Skills

### Location (observed)

```
<workdir>/.pi/skills/<name>/SKILL.md
```

A coordinator materialises skills into this directory before launch. The directory name is the skill name; each skill is a single `SKILL.md` file.

### Format

Standard Agent Skills shape (<https://agentskills.io>): a Markdown file with **YAML frontmatter**. At minimum, `name` and `description` are required. Additional frontmatter keys (`license`, `compatibility`, `metadata`) are not verified against `pi`'s own parser but follow the open standard shape.

### Minimal example

```markdown
---
name: git-release
description: Create consistent releases and changelogs
license: MIT
compatibility: pi
---

## What I do
- Draft release notes from merged PRs
- Propose a version bump
- Provide a copy-pasteable release command
```

### Always-on context

`AGENTS.md` in the working directory is honoured as always-on context on every run (observed). This is the open-standard `AGENTS.md` file; no `pi`-specific memory format has been observed.

## Sub-agents

Not documented as of 2026-06-28 — this reference characterises the `pi` CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## MCP servers

Not documented / no observed launch seam as of 2026-06-28. No flag for injecting MCP server configuration at launch has been observed. This section will be updated once a vendor-documented or empirically confirmed MCP injection path is identified.

## Slash commands

Not documented as of 2026-06-28 — this reference characterises the `pi` CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Authentication

Not documented as of 2026-06-28 — this reference characterises the `pi` CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Permissions

Not documented as of 2026-06-28 — this reference characterises the `pi` CLI from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

The `-p` flag runs the agent autonomously with no per-tool confirmation (observed). No on-stream approval handshake has been observed.

## Policies / Rules / Memory

`AGENTS.md` in the working directory is honoured as always-on context on every run (observed). This follows the open `AGENTS.md` standard: plain Markdown, no required frontmatter, prepended to the system prompt.

No global memory file location, no subdirectory walk rules, and no other policy surface have been verified against vendor documentation as of 2026-06-28.

## Orchestration / headless invocation

### Non-interactive launch

```
pi -p --mode json --session <path/to/session.jsonl> [--provider <name>] [--model <id>] [--append-system-prompt <text>] <prompt>
```

- `-p` selects non-interactive (autonomous) mode; the prompt is a positional argument.
- `--mode json` selects the line-delimited JSON event stream on stdout.
- `--session <path>` takes a **file path** to a JSONL transcript; the file must exist before launch (create it empty if starting a new session). Pass the same path on subsequent turns to continue the session.
- `--provider <name>` and `--model <id>` are two separate flags split from a single `provider/model` slug; see Model & reasoning at launch.
- `--append-system-prompt <text>` injects extra instructions that are appended to the system prompt.
- The positional `<prompt>` must be the last argument.

### Output stream protocol

One JSON object per line on stdout (NDJSON). Diagnostics go to stderr.

**Event shapes:**

```json
{"type":"agent_start"}
{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"..."}}
{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"..."}}
{"type":"tool_execution_start","toolCallId":"...","toolName":"...","args":{...}}
{"type":"tool_execution_end","toolCallId":"...","result":"..."}
{"type":"turn_end","message":{"model":"...","usage":{"input":N,"output":N,"cacheRead":N,"cacheWrite":N}}}
```

**Canonical mapping:**

| Category         | Event type(s)                                      | Key fields                                               |
|------------------|----------------------------------------------------|----------------------------------------------------------|
| Status           | `agent_start`                                      | —                                                        |
| Assistant text   | `message_update` where `assistantMessageEvent.type == "text_delta"` | `assistantMessageEvent.delta`         |
| Reasoning        | `message_update` where `assistantMessageEvent.type == "thinking_delta"` | `assistantMessageEvent.delta`      |
| Tool call        | `tool_execution_start`                             | `toolCallId`, `toolName`, `args`                         |
| Tool result      | `tool_execution_end`                               | `toolCallId`, `result`                                   |
| Usage            | `turn_end`                                         | `message.model`, `message.usage.{input,output,cacheRead,cacheWrite}` |
| Error            | `error` (observed in practice)                     | `message` (string)                                       |
| Completion       | Stream end following `turn_end`                    | —                                                        |

**Text stream markup gotcha:** the `text_delta` stream embeds tool-call markup tokens — `<|...|>`, `call:toolName{...}`, `response:...{}` — that a consumer must strip before displaying or storing the text. These tokens are an artefact of the underlying model output format and are not structured events. See Format quirks / gotchas for the stripping strategy.

### Model & reasoning at launch

Model is passed as **two separate flags**, split from a single `provider/model` slug:

```
--provider <name> --model <id>
```

For example, a slug `anthropic/claude-sonnet-4-5` becomes `--provider anthropic --model claude-sonnet-4-5`. A plain model string with no `/` separator passes through as `--model <id>` alone (no `--provider` flag).

Reasoning effort: no flag or mechanism has been observed. Do not pass a reasoning/effort flag.

### MCP at launch

Not documented / no observed launch seam as of 2026-06-28. No MCP injection flag has been observed. A coordinator cannot supply MCP servers for a single `pi` run through any confirmed mechanism.

### Skills at launch

A coordinator materialises skills into `<workdir>/.pi/skills/<name>/SKILL.md` before launch. Always-on context is placed in `AGENTS.md` in the working directory. Cross-reference Skills and Policies / Rules / Memory.

### Tool approval in headless mode

`-p` runs every tool autonomously without confirmation. No on-stream approval handshake has been observed; a coordinator does not need to answer any approval prompts during a run.

### Process lifecycle

- **Framing:** prompt in argv; events on stdout (NDJSON); diagnostics on stderr.
- **Stdin gotcha:** `pi` blocks on a stdin read at startup even though it does not use interactive input. A coordinator must open a stdin pipe, start the process, and **immediately close the pipe** to deliver EOF. Leaving stdin as `nil` (inherited from the parent) can cause `pi` to stall in its event loop waiting for stdin to become readable.
- **Cancellation:** close the stdout reader, then terminate the process (SIGTERM / kill). There is no graceful shutdown handshake.
- **Session resume:** pass `--session <same-path>` to continue a prior session. The coordinator owns the session file path; the file must exist (even if empty) before launch.
- **Minimum CLI version:** not documented. The event contract described here was characterised from observed non-interactive CLI behaviour as of 2026-06-28.

## Format quirks / gotchas

- **Strip tool-call markup from `text_delta`.** The delta stream embeds `<|...|>` control tokens and `call:toolName{...}` / `response:...{}` structured markup. A consumer must strip these before emitting display text. Use a streaming buffer: hold back any trailing bytes that look like the start of a markup token (`call:`, `response:`, or `<` followed by identifier characters) until more bytes arrive or the stream ends, then strip and emit.
- **`message_update` events can be large.** Each `text_delta` event carries the full partial message, not just the new bytes since the last event. Parse the `delta` field inside `assistantMessageEvent`; do not diff successive events.
- **`--session` requires a pre-existing file.** `pi` refuses to start when `--session` points at a missing path. Create the file (empty is fine) before calling `Start`.
- **`--mode json` and `-p` are coordinator-owned.** Do not allow user-supplied `custom_args` to override `--mode`, `--session`, `-p`, or `--print`; doing so breaks the event stream contract.
- **`--provider` and `--model` are two flags, not one.** The native slug `provider/model` must be split at the first `/` before being passed; a single `--model provider/model` argument is not equivalent.
- **Stdin must be closed immediately after `Start`.** See Process lifecycle above. On some systems (e.g. under systemd), leaving stdin open causes `pi` to stall indefinitely.
- **`--append-system-prompt` appends, not replaces.** The flag adds to the harness's own system prompt; it does not override it. Use it for per-run coordinator instructions only.
- **No MCP injection seam.** Do not attempt to pass MCP configuration via env var or flag; no such mechanism has been observed.
- **Session path is the session ID.** The coordinator returns the file path as the opaque session identifier and passes it back as `--session` on the next turn. Do not treat it as an internal file detail.

## Renderer notes (planned)

`agent-manager`'s `pi` renderer should:

1. **Skills** — write `<workdir>/.pi/skills/<name>/SKILL.md` with standard `SKILL.md` frontmatter (`name`, `description`) before launching the process.
2. **Always-on context** — write `<workdir>/AGENTS.md` with the desired system instructions before launch.
3. **Session files** — manage session JSONL file paths in the coordinator's own state directory; create an empty file before launch; pass the path via `--session`; return the path as the session ID.
4. **Model injection** — split the `provider/model` slug at the first `/` and pass `--provider <name> --model <id>` as separate flags.
5. **Extra instructions** — pass coordinator-side per-run instructions via `--append-system-prompt`.
6. **Stdin** — always open a stdin pipe and close it immediately after `cmd.Start()`.
7. **Text stripping** — implement the streaming markup stripper for `text_delta` content before forwarding text to the UI (buffer trailing partial-token bytes; strip complete `call:…{}` and `response:…{}` blocks and `<|…|>` control tokens).
8. **Do not own** — the `pi` binary itself; any `~/.pi/` global config; any provider credentials.

## Sources

- Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28.
- Vendor documentation: Not documented.
