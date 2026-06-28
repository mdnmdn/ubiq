# CodeBuddy

Stable id: `codebuddy`
Display name: CodeBuddy
Vendor: Tencent

## Quick reference

| Field         | Value                                                                                    |
|---------------|------------------------------------------------------------------------------------------|
| Stable id     | `codebuddy`                                                                              |
| Display name  | CodeBuddy                                                                                |
| Vendor        | Tencent                                                                                  |
| Global root   | Not documented as of 2026-06-28 — assumed to mirror `~/.claude/` (Claude-compatible CLI) |
| Project root  | `./.claude/` (observed; mirrors Claude Code layout)                                      |
| Config format | JSON (settings), Markdown (memory / skills)                                              |
| Status        | Reference — Claude-compatible CLI characterised from observed runtime contract; not yet an agent-manager sync target. |

## On-disk layout

CodeBuddy mirrors the Claude Code on-disk layout. The entries below are characterised from the observed runtime contract. Native CodeBuddy-specific paths or keys not exercised by the runtime contract are not documented as of 2026-06-28.

### Global

```
~/.claude/                         # assumed global root (Claude-compatible)
├── settings.json                  # user settings
├── CLAUDE.md                      # always-on user memory
├── skills/<name>/SKILL.md         # user skills (Agent Skills open standard)
└── agents/<name>.md               # user sub-agents
```

### Project (`<workdir>/`)

```
<workdir>/
├── CLAUDE.md                      # always-on project memory (observed)
├── .mcp.json                      # project MCP registry (Claude-compatible)
└── .claude/
    ├── settings.json              # project settings
    ├── skills/<name>/SKILL.md     # project skills (observed path)
    └── agents/<name>.md           # project sub-agents
```

## Discovery precedence

Not documented as of 2026-06-28 — characterised from observed runtime contract; see Orchestration / headless invocation.

The runtime contract exercises the following layers (highest to lowest as observed):

1. **CLI flags** — `--model`, `--effort`, `--max-turns`, `--append-system-prompt`, `--resume`, `--mcp-config`.
2. **Project** — `<workdir>/CLAUDE.md` and `.claude/skills/<name>/SKILL.md` materialised by an external coordinator before launch.
3. **MCP config file** — `--mcp-config <path>` combined with `--strict-mcp-config` makes this the authoritative and complete server list for the run.

CodeBuddy is expected to follow Claude Code's multi-layer settings merge (managed > enterprise > user > project > local), but native precedence specifics are not confirmed as of 2026-06-28.

## Feature matrix

| Feature          | Support              | Where it lands                                                               |
|------------------|----------------------|------------------------------------------------------------------------------|
| Rules            | n/a (no renderer)    | `CLAUDE.md` (project + assumed user); seam observed via runtime contract     |
| Skills           | n/a (no renderer)    | `.claude/skills/<name>/SKILL.md` (project); seam observed via runtime        |
| MCP              | n/a (no renderer)    | `--mcp-config <path>` → `{"mcpServers":{...}}`; seam observed via runtime    |
| Agents           | n/a (no renderer)    | `.claude/agents/<name>.md` (assumed; Claude-compatible layout)               |
| Slash commands   | n/a (no renderer)    | Not documented as of 2026-06-28                                              |
| Auth             | n/a (no renderer)    | Not documented as of 2026-06-28; see Authentication                          |
| Permissions      | n/a (no renderer)    | `--permission-mode bypassPermissions` observed; see Permissions              |
| Policies / Rules | n/a (no renderer)    | `CLAUDE.md` in workdir observed as always-on context                         |

## Skills

CodeBuddy mirrors the Claude Code skills layout. An external coordinator materialises skills into the project path before launch.

### Locations (observed)

- Project: `<workdir>/.claude/skills/<name>/SKILL.md`
- User: assumed `~/.claude/skills/<name>/SKILL.md` (Claude-compatible; not confirmed from runtime contract)

### Format (Agent Skills open standard)

A skill is a directory with a `SKILL.md` carrying YAML frontmatter and a Markdown body. The frontmatter follows the same Agent Skills open standard as Claude Code.

```yaml
---
name: deploy
description: Deploy the app to staging or production.
allowed-tools: Bash(git push:*), Bash(kubectl apply:*)
---

# Deploy skill

When the user asks to deploy, run the following checks first…
```

### Invocation

Invocation conventions are assumed to mirror Claude Code (manual `/<skill-name>`, automatic by model intent matching). Verified only to the extent the runtime contract exercises it; CodeBuddy-specific divergences are not documented as of 2026-06-28.

## Sub-agents

Not documented as of 2026-06-28 — characterised from observed runtime contract; see Orchestration / headless invocation.

CodeBuddy is expected to support sub-agents in `.claude/agents/<name>.md` (project) and `~/.claude/agents/<name>.md` (user), following the Claude Code YAML-frontmatter format. This is inferred from the Claude-compatible layout; no CodeBuddy-specific sub-agent behaviour has been confirmed from the runtime contract.

## MCP servers

### Location (observed)

A coordinator writes a temporary JSON file and passes it via `--mcp-config <path>`. The file format mirrors Claude Code's `mcpServers` shape. `--strict-mcp-config` suppresses all inherited/ambient servers so only the provided file's servers are active for the run.

### Key

`mcpServers` (top-level key in the config file — same as Claude Code).

### Transport variants (assumed Claude-compatible)

- `stdio`: spawns a subprocess (`command`, `args`, `env`).
- `sse` / `http`: remote server via `url`, optional `headers`.

### Example (`--mcp-config` file)

```json
{
  "mcpServers": {
    "github": {
      "type": "http",
      "url": "https://api.githubcopilot.com/mcp/"
    },
    "local-tool": {
      "command": "npx",
      "args": ["-y", "@example/mcp-tool@latest"],
      "env": { "TOOL_ENV": "value" }
    }
  }
}
```

The coordinator writes this file to a temporary path immediately before launch and removes it after the process exits.

## Slash commands

Not documented as of 2026-06-28 — characterised from observed runtime contract; see Orchestration / headless invocation.

CodeBuddy is expected to follow the same slash command conventions as Claude Code (built-in and custom via `.claude/commands/<name>.md` or `.claude/skills/<name>/SKILL.md`). No CodeBuddy-specific slash command catalogue has been confirmed.

## Authentication

Not documented as of 2026-06-28 — characterised from observed runtime contract; see Orchestration / headless invocation.

CodeBuddy is a Tencent product. Its authentication mechanism (API key, OAuth, Tencent Cloud delegation) is not confirmed from the runtime contract. The external coordinator supplies credentials through the process environment (`cmd.Env = buildEnv(cfg.Env)`); the specific env vars CodeBuddy reads are not documented here.

For headless/CI use, supply credentials via the environment passed at launch. Do not rely on interactive auth flows in a non-TTY context.

## Permissions

Not documented as of 2026-06-28 — characterised from observed runtime contract; see Orchestration / headless invocation.

The runtime contract passes `--permission-mode bypassPermissions` and `--disallowedTools AskUserQuestion` at launch. This keeps every tool call unattended and suppresses interactive user-question prompts.

Individual tool calls still emit a `control_request` event on stdout before execution (see Tool approval in headless mode in the Orchestration section). A coordinator answers each with a `control_response` to allow or modify the call.

Native CodeBuddy permission configuration (settings file keys, allow/deny rule syntax) is not documented as of 2026-06-28.

## Policies / Rules / Memory

CodeBuddy uses `CLAUDE.md` in the working directory as always-on context. This mirrors Claude Code's memory convention.

An external coordinator writes managed instructions into `<workdir>/CLAUDE.md` before launch, ideally inside a managed marker block (e.g. `<!-- agent-manager:begin --> … <!-- agent-manager:end -->`) to preserve any user-authored content.

Native multi-level memory walk behaviour (parent-directory traversal, user-level `~/.claude/CLAUDE.md`, modular `.claude/rules/`) is assumed to mirror Claude Code but is not confirmed from the runtime contract as of 2026-06-28.

## Orchestration / headless invocation

This section is grounded in the observed runtime contract. All flag names and event shapes are verified against the external coordinator's implementation.

### Non-interactive launch

Argv skeleton:

```
codebuddy -p --output-format stream-json --input-format stream-json --verbose \
  --strict-mcp-config --permission-mode bypassPermissions \
  --disallowedTools AskUserQuestion \
  [--model <id>] [--effort <level>] [--max-turns <n>] \
  [--append-system-prompt <text>] [--resume <session-id>] [--mcp-config <path>]
```

- `-p` = print / non-interactive mode.
- `--output-format stream-json` emits machine-readable NDJSON on stdout.
- `--input-format stream-json` means the prompt is delivered as a JSON line on stdin, not as a positional argument.
- `--strict-mcp-config` suppresses every ambient/inherited MCP server; only the `--mcp-config` file's servers are active.
- `--permission-mode bypassPermissions` and `--disallowedTools AskUserQuestion` keep the run fully unattended.
- `--verbose` is passed by the coordinator; its effect on stdout framing is not separately documented.

The following flags are reserved by the coordinator and must not be overridden via user-configured custom arguments: `-p`, `--output-format`, `--input-format`, `--permission-mode`, `--mcp-config`, `--effort`.

### Output stream protocol

Newline-delimited JSON (NDJSON), one object per line on stdout. The protocol is the same `stream-json` event family as Claude Code.

**Prompt delivery** — a single NDJSON line written to stdin before the process reads it:

```json
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<prompt>"}]}}
```

**Events emitted on stdout:**

```json
{"type":"system","session_id":"<id>","subtype":"init"}
{"type":"assistant","message":{"role":"assistant","model":"<id>","content":[{"type":"text","text":"..."},{"type":"thinking","text":"..."},{"type":"tool_use","id":"...","name":"...","input":{}}],"usage":{"input_tokens":0,"output_tokens":0}}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"...","content":[]}]}}
{"type":"result","result":"<text>","is_error":false,"session_id":"<id>","usage":{},"modelUsage":{"<model-id>":{"inputTokens":0,"outputTokens":0}}}
{"type":"log","log":{"level":"info","message":"..."}}
{"type":"control_request","request_id":"<id>","request":{"subtype":"...","tool_name":"...","input":{}}}
```

**Event categories:**

| Category       | `type` value      | Notes                                                               |
|----------------|-------------------|---------------------------------------------------------------------|
| Session init   | `system`          | Carries `session_id`; emitted before the first assistant turn       |
| Assistant text | `assistant`       | `content[].type == "text"`                                          |
| Reasoning      | `assistant`       | `content[].type == "thinking"`                                      |
| Tool call      | `assistant`       | `content[].type == "tool_use"`                                      |
| Tool result    | `user`            | `content[].type == "tool_result"`                                   |
| Log            | `log`             | Diagnostic messages; `log.level` + `log.message`                   |
| Tool approval  | `control_request` | Emitted before each tool execution; coordinator must reply          |
| Completion     | `result`          | Final event; `is_error` distinguishes success from failure          |

Token usage is best read from `modelUsage` in the `result` event (camelCase keys: `inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens`), falling back to the top-level `usage` field on the same event.

### Model and reasoning at launch

- Model: `--model <id>` (pass the model identifier string as the value).
- Reasoning effort: `--effort <level>`, values `low | medium | high | xhigh`. There is no `max` level (unlike Claude Code). Discoverable from `codebuddy --help`. The effort flag applies uniformly across all models; there is no per-model restriction table.

Cross-reference Authentication for credential setup; do not restate provider auth here.

### MCP at launch

- Pass `--mcp-config <path>` where `<path>` is a temporary file containing `{"mcpServers":{...}}` written by the coordinator immediately before launch.
- Pass `--strict-mcp-config` to suppress all ambient/inherited servers. Only the servers listed in the `--mcp-config` file are active for the run.
- The coordinator removes the temp file after the process exits (on both success and error paths).

Cross-reference MCP servers for the per-server JSON schema.

### Skills at launch

A coordinator materialises skills into `<workdir>/.claude/skills/<name>/SKILL.md` before launch (the project skills path). Always-on context is written into `<workdir>/CLAUDE.md`. Use a managed marker block inside `CLAUDE.md` so user-authored content is preserved.

Cross-reference Skills and Policies / Rules / Memory.

### Tool approval in headless mode

CodeBuddy emits a `control_request` event on stdout before each tool call and waits for a matching `control_response` on stdin. Stdin is kept open after the initial prompt write for exactly this handshake.

A coordinator auto-approves by writing:

```json
{"type":"control_response","response":{"subtype":"success","request_id":"<matching-request_id>","response":{"behavior":"allow","updatedInput":<tool-input-object>}}}
```

`updatedInput` may rewrite the tool input before execution (e.g. to force `run_in_background: false`). Pass back the original input unchanged to allow without modification.

The `--permission-mode bypassPermissions` flag enables the auto-approve path, but the `control_request` / `control_response` handshake still occurs in-band on stdin/stdout.

### Process lifecycle

- **Framing:** prompt in on stdin (NDJSON), events out on stdout (NDJSON), diagnostics on stderr.
- **Stdin:** kept open after the prompt write so `control_response` messages can be sent mid-run. The process is not expected to read a second user prompt; stdin is closed after the `result` event lands or on cancellation.
- **Cancellation:** close stdin, then close the stdout reader to unblock the scanner; allow ~10 seconds for the process to drain before sending a kill signal (`cmd.WaitDelay = 10s` in the coordinator).
- **Startup banner:** CodeBuddy (like Claude Code) may emit output on stdout before reading stdin. The coordinator writes the prompt in a dedicated goroutine to prevent pipe-buffer deadlock.
- **Minimum CLI version:** not confirmed as of 2026-06-28. The stream-json protocol is characterised from observed runtime contract behaviour.

## Format quirks / gotchas

- `--effort` does not accept `max`. The valid values are `low | medium | high | xhigh`. Passing `max` will likely error or be ignored; use `xhigh` as the ceiling.
- Stdin must remain open after the prompt write. CodeBuddy uses it for `control_response` messages mid-run. Closing stdin too early aborts pending tool calls.
- The startup banner may arrive on stdout before stdin is read. Write the prompt in a separate goroutine; do not block on the write before starting the stdout scanner.
- `control_request` events arrive in-band on stdout and require an in-band `control_response` on stdin. Missing a response will stall the run indefinitely.
- `modelUsage` keys in the `result` event are camelCase (`inputTokens`, not `input_tokens`). The per-turn `usage` object inside `assistant` messages uses snake_case. Parse both shapes separately.
- `--mcp-config` and `--strict-mcp-config` must be used together for a controlled server set. Without `--strict-mcp-config`, ambient/inherited servers may augment the provided file.
- The `result` event carries `session_id`; capture it for `--resume` on subsequent runs. If the `result` event does not arrive (e.g. early exit), the session ID from the `system` init event should be considered unreliable for resume.
- `--disallowedTools AskUserQuestion` suppresses the interactive user-question tool. Without this flag, the process may pause and wait for interactive input in a non-TTY context.

## Renderer notes (planned)

`agent-manager`'s CodeBuddy renderer is not yet planned as a sync target (Status: reference only). When a renderer is implemented, it should:

1. Mirror the Claude Code renderer's approach: write `CLAUDE.md` with a managed marker block; write skills into `.claude/skills/<name>/SKILL.md`; write the MCP config to a temp file for `--mcp-config`.
2. Supply credentials through the process environment (exact env var names TBD from CodeBuddy docs).
3. Not touch any file outside `<workdir>/` that it did not create, until native CodeBuddy config paths are confirmed.
4. Treat `--effort xhigh` as the maximum reasoning effort (no `max` level).
5. Record the `session_id` from the `result` event for optional session resumption via `--resume`.

## Sources

- Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28. Primary source: external coordinator implementation exercising the `codebuddy` binary with `stream-json` protocol.
- Claude Code doc (protocol reference, mirrored by CodeBuddy) — <https://docs.claude.com/en/docs/claude-code/overview>
- Agent Skills open standard — <https://agentskills.io>
