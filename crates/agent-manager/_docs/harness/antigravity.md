# Antigravity

Stable id: `antigravity`
Display name: Antigravity
Vendor: Google (Antigravity)
Status: Reference — characterised from observed runtime contract; not yet an agent-manager sync target.

## Quick reference

| Field         | Value                                                                        |
|---------------|------------------------------------------------------------------------------|
| Stable id     | `antigravity`                                                                |
| Display name  | Antigravity                                                                  |
| Vendor        | Google (Antigravity)                                                         |
| Binary        | `agy`                                                                        |
| Global root   | Not documented as of 2026-06-28 — this reference characterises Antigravity from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation. |
| Project root  | Not documented as of 2026-06-28 — see above.                                |
| Config format | Not documented as of 2026-06-28 — see above.                                |
| Status        | Reference — characterised from observed runtime contract; not yet an agent-manager sync target. |

## On-disk layout

Not documented as of 2026-06-28 — this reference characterises Antigravity from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

The single on-disk path observed at runtime is the skills directory (agent-neutral path):

```
<workdir>/
└── .agents/
    └── skills/
        └── <name>/
            └── SKILL.md
```

And the always-on context file read at runtime:

```
<workdir>/
└── AGENTS.md
```

## Discovery precedence

Not documented as of 2026-06-28 — this reference characterises Antigravity from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Feature matrix

| Feature          | Support             | Where it lands                                                         |
|------------------|---------------------|------------------------------------------------------------------------|
| Rules            | n/a (no renderer)   | `AGENTS.md` in the working directory (agent-neutral open standard); no vendor-specific rules file observed |
| Skills           | n/a (no renderer)   | `<workdir>/.agents/skills/<name>/SKILL.md` (agent-neutral path); observed seam only |
| MCP              | n/a (no renderer)   | Not documented / no observed launch seam                               |
| Agents           | n/a (no renderer)   | Not documented as of 2026-06-28                                        |
| Slash commands   | n/a (no renderer)   | Not documented as of 2026-06-28                                        |
| Auth             | n/a (no renderer)   | Not documented as of 2026-06-28                                        |
| Permissions      | n/a (no renderer)   | `--dangerously-skip-permissions` flag bypasses all tool approval headlessly; no per-tool rule surface observed |
| Policies / Rules | n/a (no renderer)   | `AGENTS.md` in the working directory (agent-neutral open standard); no additional policy surface observed |

## Skills

### Location (observed)

```
<workdir>/.agents/skills/<name>/SKILL.md
```

This is the agent-neutral path observed at runtime. No Antigravity-native skills directory path has been verified against vendor documentation.

### Format

Standard Agent Skills shape (`SKILL.md` with YAML frontmatter). Minimum required frontmatter keys:

| Key           | Required | Notes                                           |
|---------------|----------|-------------------------------------------------|
| `name`        | yes      | Must match the directory name.                  |
| `description` | yes      | Shown to the model as a skill description.      |

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

A coordinator materialises skills into `<workdir>/.agents/skills/<name>/SKILL.md` before launch. Always-on context is delivered via `AGENTS.md` in the working directory (cross-reference Policies / Rules / Memory).

## Sub-agents

Not documented as of 2026-06-28 — this reference characterises Antigravity from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## MCP servers

Not documented / no observed launch seam. `agy` exposes no MCP-injection flag or environment variable at launch time as of 2026-06-28. No method for supplying per-run MCP servers has been observed.

## Slash commands

Not documented as of 2026-06-28 — this reference characterises Antigravity from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Authentication

Not documented as of 2026-06-28 — this reference characterises Antigravity from its observed non-interactive runtime contract (see Orchestration / headless invocation); the native configuration surface has not been verified against vendor documentation.

## Permissions

`--dangerously-skip-permissions` is the only observed permission-bypass mechanism. Passing this flag at launch allows `agy` to run all tools without interactive confirmation. No per-tool allow/deny/ask rule surface has been observed.

## Policies / Rules / Memory

`AGENTS.md` in the working directory is honoured as always-on context. This is the agent-neutral open standard also used by Claude Code, Codex, Gemini CLI, and opencode.

No Antigravity-native memory file format (equivalent to `CLAUDE.md`, `GEMINI.md`, etc.) has been observed or verified against vendor documentation as of 2026-06-28.

## Orchestration / headless invocation

### Non-interactive launch

```
agy -p "<prompt>" --dangerously-skip-permissions \
    [--model "<display name>"] \
    --print-timeout <duration> \
    --log-file <tmpfile> \
    [--conversation <uuid>] \
    [--add-dir <cwd>]
```

- `-p "<prompt>"` — the prompt is a CLI argument, not piped on stdin.
- `--dangerously-skip-permissions` — suppresses all interactive tool-approval prompts; always required for headless operation.
- `--model "<display name>"` — the exact human-readable display string from `agy models` (e.g. `"Claude Opus 4.6 (Thinking)"`), not a provider/model slug. Omit to use `agy`'s own default.
- `--print-timeout <duration>` — wall-clock budget for the print-mode run. Accepts Go duration strings (e.g. `20m0s`). **Must always be passed** — when omitted, `agy` silently defaults to 5 minutes, which guillotines any turn whose tools outlive that budget. Pass a sufficiently large value (e.g. `24h0m0s`) when no cap is intended.
- `--log-file <tmpfile>` — path to a temporary file where `agy` writes glog-format structured signals. Required for session-id extraction and timeout detection.
- `--conversation <uuid>` — resume a prior session. Omit for a fresh session.
- `--add-dir <cwd>` — grants the agent access to the specified directory (the task working directory).

### Output stream protocol

Protocol family: **plain text on stdout plus structured data scraped from the `--log-file`.**

There is no structured event stream on stdout. Every non-empty stdout line is treated as assistant text. The coordinator streams these lines as text events and accumulates them as the final output.

**Stdout mapping:**

| Event type     | Source       | Description                                              |
|----------------|--------------|----------------------------------------------------------|
| Assistant text | stdout       | Every non-empty line is an assistant text fragment.      |
| Tool calls     | stdout       | Not emitted as distinct events; `agy` may print "I will run X" prose lines interleaved with the response. |
| Tool results   | stdout       | Not emitted as distinct events.                          |
| Usage / tokens | (none)       | Token usage is not available; always empty.              |
| Error          | `--log-file` | `agent executor error: <message>` line in glog format.   |
| Completion     | stdout EOF   | Stream end after `cmd.Wait()` returns.                   |

**Log-file signals (glog format):**

| Signal            | Pattern in log                                              | Action                                   |
|-------------------|-------------------------------------------------------------|------------------------------------------|
| Session id        | `conversation=<uuid>`                                       | Extract UUID; pass as `--conversation` on next turn. |
| Print-mode timeout| `Print mode: timed out after <N> polls`                    | Surface as timeout, not success (see Process lifecycle). |
| Provider error    | `agent executor error: <message>`                          | Surface as a failed run.                 |

### Model & reasoning at launch

- **Model:** `--model "<display name>"` where the value is the exact human-readable display string that `agy models` prints (e.g. `"Claude Opus 4.6 (Thinking)"` or `"Gemini 2.5 Pro"`). This is **not** a provider/model slug. Spaces and parentheses need no shell quoting when passed as a single exec argument.
- **Model validation:** The coordinator must validate the display name against the `agy models` catalogue before launch. `agy` exits 0 on an unknown model with empty output — a silent no-op indistinguishable from a completed task. Validation is fail-open: if the catalogue cannot be discovered, pass the value through and let `agy` resolve it.
- **Reasoning effort:** Not a separate flag. Thinking-mode variants appear as distinct model display names (e.g. `"Claude Opus 4.6 (Thinking)"`). Select reasoning effort by choosing the appropriate display name.
- Cross-reference Authentication for provider credential setup (not restated here).

### MCP at launch

Not documented / no observed launch seam. No flag or environment variable for supplying MCP servers to a single `agy` run has been observed as of 2026-06-28.

### Skills at launch

A coordinator materialises skills into `<workdir>/.agents/skills/<name>/SKILL.md` before launch (agent-neutral path). Always-on context goes into `AGENTS.md` in the working directory. Cross-reference Skills and Policies / Rules / Memory.

### Tool approval in headless mode

`--dangerously-skip-permissions` runs every tool without confirmation. There is no on-stream approval handshake to answer. The flag must be present on every headless invocation.

### Process lifecycle

- **Input framing:** prompt in argv (`-p "<prompt>"`); no stdin interaction.
- **Output framing:** assistant text on stdout (plain text, one line at a time); structured signals in the `--log-file` (glog format).
- **Exit code:** `agy` exits 0 in all observed cases — including print-mode timeout and provider errors. Exit code alone cannot distinguish success from failure. The coordinator **must** scrape the `--log-file` after `cmd.Wait()` to detect timeouts and provider errors.
- **Cancellation:** close the stdout reader, then send a termination signal to the process; allow ~10 seconds for drain before sending a harder kill.
- **Session resume:** pass `--conversation <uuid>` where the UUID was previously captured from the `--log-file` `conversation=<uuid>` line.
- **Minimum CLI version:** `--model` flag support was added in `agy` 1.0.6. Earlier versions ignore the flag.

## Format quirks / gotchas

- **Exit code 0 does not mean success.** `agy` exits 0 on print-mode timeout, provider errors, and unknown model names. The coordinator must scrape the `--log-file` for the `Print mode: timed out after <N> polls` and `agent executor error:` markers to distinguish these from a genuine completed turn.
- **`--print-timeout` has no disabled sentinel.** Omitting it silently caps every turn at 5 minutes. Always pass an explicit value — use a large sentinel (e.g. `24h0m0s`) when no cap is intended.
- **Model value is a display string, not a slug.** Pass the exact string from `agy models` output (e.g. `"Claude Opus 4.6 (Thinking)"`). A near-miss (extra space, dropped suffix) causes a silent no-op exit 0 with empty output.
- **Model validation is fail-open.** If the `agy models` catalogue cannot be fetched, let `agy` resolve the value rather than blocking the run on a discovery failure.
- **No structured event stream on stdout.** Every non-empty stdout line is assistant text. Tool calls and tool results are not emitted as parseable events.
- **Session id is not on stdout.** Capture it from the `--log-file` `conversation=<uuid>` line. The CLI logs this repeatedly per turn; use the last match.
- **Token usage is not available.** `agy` does not surface per-turn token usage. Leave the usage field empty rather than report misleading zeros.
- **No MCP injection seam.** There is no flag or env var for supplying MCP servers per-run. Do not attempt to inject MCP configuration at launch.
- **`-i` / `--prompt-interactive` requires a TTY.** Never pass these flags in headless operation.
- **`-c` / `--continue` must not be used for session resume.** Use `--conversation <uuid>` instead.
- **`--add-dir` controls working-directory access.** Pass the task `cwd` here to grant the agent access to the project directory.
- **Thinking variants are distinct model names.** There is no separate reasoning-effort flag; select `"Claude Opus 4.6 (Thinking)"` vs `"Claude Opus 4.6"` as the model display name.

## Renderer notes (planned)

No agent-manager sync renderer is planned for Antigravity as of 2026-06-28. This document characterises the runtime contract only.

When a renderer is implemented, it should:

1. **Rules / memory** → write `AGENTS.md` into the task working directory before launch. No Antigravity-native rules file path has been verified.
2. **Skills** → write `<workdir>/.agents/skills/<name>/SKILL.md` (agent-neutral path) before launch. Frontmatter: at minimum `name` (must equal directory name) and `description`.
3. **MCP** → no action until a launch seam is documented.
4. **Model** → pass `--model "<display name>"` with the exact string from `agy models`. Validate against the catalogue before launch; fail-open if the catalogue is unavailable.
5. **Session resume** → extract the `conversation=<uuid>` from the `--log-file` after each turn and persist it; pass as `--conversation <uuid>` on the next turn.
6. **Timeout detection** → after `cmd.Wait()`, scan the `--log-file` for `Print mode: timed out after \d+ polls` before classifying the result as completed.
7. **Provider error detection** → scan the `--log-file` for `agent executor error:` lines and surface the last match as a failed run.
8. **Files the renderer owns:** `AGENTS.md` and `.agents/skills/` in the working directory.
9. **Files the renderer must not touch:** anything outside the above paths.

## Sources

- Runtime contract — characterised from observed non-interactive CLI behaviour as of 2026-06-28. No vendor documentation URL has been verified for the configuration surface described in this document.
