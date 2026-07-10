# Grok CLI

Stable id: `grok`
Display name: Grok CLI
Vendor: superagent-ai (community; npm `@vibe-kit/grok-cli`, binary `grok`) — talks to xAI's Grok API. Not affiliated with xAI Corp.

## Quick reference

| Field         | Value                                                                          |
|---------------|--------------------------------------------------------------------------------|
| Stable id     | `grok`                                                                          |
| Display name  | Grok CLI                                                                        |
| Vendor        | superagent-ai/grok-cli (community, open-source; uses xAI's Grok API)           |
| Global root   | `~/.grok/` (no override env var documented as of 2026-07-09)                    |
| Project root  | `<repo>/.grok/settings.json`, `<repo>/AGENTS.md`, `<repo>/.agents/skills/`      |
| Config format | JSON (`user-settings.json`, `settings.json`), Markdown (`AGENTS.md`, `SKILL.md`) |

There is **no official xAI first-party terminal coding agent** as of
2026-07-09. The most widely used and documented Grok terminal agent is
the community project **`superagent-ai/grok-cli`**, published to npm as
**`@vibe-kit/grok-cli`** with the binary **`grok`**. Recent releases also
ship a shell installer and a Bun global package (`grok-dev`); all three
distributions install the same `grok` command. This doc is written
against `superagent-ai/grok-cli`.

## On-disk layout

### Global (`~/.grok/`)

```
~/.grok/
├── user-settings.json        # global settings: apiKey, defaultModel, mcpServers, subAgents, telegram, hooks (mode 0600)
└── workspace-trust.json      # per-directory trust decisions (permission model)

# Skills (Agent Skills open standard), user tier:
~/.agents/skills/<name>/SKILL.md
```

### Project (`<repo>/`)

```
<repo>/
├── AGENTS.md                 # always-on project instructions (merged git-root → cwd)
├── AGENTS.override.md        # per-directory override, wins over AGENTS.md in that dir
└── .grok/
    ├── settings.json         # project settings: model, mcpServers, sandbox config
    ├── computer/             # screenshots from the built-in `computer` sub-agent
    └── generated-media/      # output of generate_image / generate_video tools

# Skills (Agent Skills open standard), project tier:
<repo>/.agents/skills/<name>/SKILL.md
```

Notes:

- The cheat-sheet corpus also mentions `<repo>/.grok/GROK.md` as a
  "custom project context" file. The repo README documents `AGENTS.md`
  (not `GROK.md`) as the instruction file; treat `AGENTS.md` as
  canonical and `.grok/GROK.md` as unverified as of 2026-07-09.
- Skills live under `.agents/skills/` (the agent-neutral path), **not**
  under `.grok/`.

## Discovery precedence

The global config directory `~/.grok/` is derived from the OS home
directory. **No `GROK_CONFIG_DIR`-style override env var is documented
as of 2026-07-09** (checked the repo README and DeepWiki config
reference). To relocate the global tier a coordinator must set `HOME`
(and, on Windows, the platform home) for the child process.

Model resolution order (highest first), per the config reference:

1. `GROK_MODEL` environment variable.
2. `-m` / `--model` CLI flag (single run).
3. Project settings — `.grok/settings.json` → `model`.
4. User settings — `~/.grok/user-settings.json` → `defaultModel`.
5. Built-in `DEFAULT_MODEL` fallback.

API key resolution (observed order): `-k` / `--api-key` flag →
`GROK_API_KEY` env → `apiKey` in `~/.grok/user-settings.json`.

Instruction files (`AGENTS.md`) are **merged** from the git root down to
the current directory; an `AGENTS.override.md` in a directory takes
precedence over `AGENTS.md` in that same directory. Project settings
override user settings per key.

## Feature matrix

| Feature        | Support | Where it lands                                                        |
|----------------|---------|-----------------------------------------------------------------------|
| Rules          | full    | `AGENTS.md` (project, git-root → cwd) + `AGENTS.override.md`           |
| Skills         | full    | `.agents/skills/<name>/SKILL.md` (project + `~/.agents/skills/` user)  |
| MCP            | full    | `.grok/settings.json` / `~/.grok/user-settings.json` → `mcpServers`    |
| Agents         | full    | `~/.grok/user-settings.json` → `subAgents[]`                          |
| Slash commands | partial | Built-in TUI commands only; no documented custom-command file format  |
| Auth           | full    | `GROK_API_KEY` / `-k` / `apiKey`; `GROK_BASE_URL` for endpoint         |
| Permissions    | partial | Workspace trust (`~/.grok/workspace-trust.json`) + sandbox flags; no allow/deny rule file |
| Policies       | full    | `AGENTS.md` (always-on instruction content)                           |

"Support" is the `agent-manager` view of how completely the feature is
expressible via the sync engine, not a statement about Grok's own
capability.

## Skills

Grok CLI implements the **Agent Skills open standard**
(<https://agentskills.io>): a directory with a `SKILL.md` whose YAML
frontmatter carries at least `name` and `description`.

### Locations

```
<repo>/.agents/skills/<name>/SKILL.md    # project (agent-neutral path)
~/.agents/skills/<name>/SKILL.md         # user
```

`/skills` in the TUI lists the installed skills. Note the path is the
agent-neutral `.agents/skills/`, shared with other harnesses, not a
`grok`-specific directory.

### Format

```markdown
---
name: git-release
description: Draft release notes and a version bump from merged PRs.
---

## What I do
- Summarise merged PRs into release notes
- Propose a semver bump
- Emit a copy-pasteable `gh release create` command
```

The repo also references a separate set of **hardcoded system-prompt
"skills"** compiled into the binary (`src/utils/skills.ts`); those are
not user-authored files and are out of scope for a sync engine.

## Sub-agents

Grok CLI has first-class sub-agents, **on by default**.

### Built-in (reserved names)

`general`, `explore`, `vision`, `verify`, `computer`. These names cannot
be reused for custom agents. Foreground delegation uses the `task` tool
(e.g. `explore`, `general`, `computer`); background read-only deep dives
use the `delegate` tool. The `computer` sub-agent drives host desktop
automation via `agent-desktop` (macOS; saves screenshots under
`.grok/computer/`).

### Custom

Defined inline in `~/.grok/user-settings.json` under the `subAgents`
array; managed from the TUI with `/agents`. Each entry:

```json
{
  "subAgents": [
    {
      "name": "security-review",
      "model": "grok-4.3",
      "instruction": "Prioritize security implications and suggest concrete fixes."
    }
  ]
}
```

| Key           | Type   | Required | Notes                                                      |
|---------------|--------|----------|------------------------------------------------------------|
| `name`        | string | yes      | Must not collide with a reserved built-in name.            |
| `model`       | string | yes      | Grok model id (e.g. `grok-4.3`, `grok-code-fast-1`).       |
| `instruction` | string | yes      | System-prompt text for this sub-agent.                     |

There is no per-file markdown sub-agent format; custom sub-agents live
only in `user-settings.json`.

## MCP servers

MCP servers are configured under the **`mcpServers`** key in either
`.grok/settings.json` (project) or `~/.grok/user-settings.json` (user).
The DeepWiki config reference also names a `mcp` key on the user-settings
schema; the README, cheat-sheet, and MCP-integration source all use
`mcpServers`, so emit `mcpServers`.

Servers can also be managed with subcommands:

- `grok mcp add <name>` — register a server (interactive/flags).
- `grok mcp add-json <name> <json>` — register from an inline JSON blob.
- `grok mcp list` — list configured servers.
- `grok mcp test <name>` — check connectivity.
- `grok mcp remove <name>` — delete a server.

Or `/mcps` in the TUI.

### Transport shape

Per the MCP runtime (`src/mcp/runtime.ts`): "`stdio` transports must have
a command" and "remote transports (`http`, `sse`) must have a valid
URL." Per-server fields observed: `command`, `args`, `env`, `cwd` for
stdio; `type`, `url`, `headers` for remote; `label` / `id` common.

### stdio (subprocess)

```json
{
  "mcpServers": {
    "everything": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-everything"],
      "env": { "MY_ENV_VAR": "value" }
    }
  }
}
```

### http / sse (remote)

```json
{
  "mcpServers": {
    "sentry": {
      "type": "http",
      "url": "https://mcp.sentry.dev/mcp",
      "headers": { "Authorization": "Bearer your-token" }
    }
  }
}
```

The precise optional field set (`cwd`, `label`, `id`) is derived from
source analysis (DeepWiki) rather than a published schema; verify against
`src/mcp/runtime.ts` before relying on the optional fields. The `type`
discriminator and the `command`/`url` requirement are confirmed.

## Slash commands

Grok CLI exposes built-in TUI slash commands, including `/agents`
(manage sub-agents), `/skills` (list skills), `/mcps` (manage MCP
servers), `/remote-control` (Telegram pairing), `/pair`, and `/verify`
(build/test/smoke-check in a sandbox).

**No custom slash-command file format is documented** as of 2026-07-09.
Unlike Claude Code / opencode, Grok CLI does not fold user-authored
slash commands into the Skills concept via a `commands/` directory.
Custom behaviour is expressed through sub-agents (`subAgents`) and
skills (`.agents/skills/`) instead.

## Authentication

Grok CLI authenticates to **xAI's Grok API** with a single API key.
There is no OAuth, cloud-provider delegation, or multi-provider map.

### API key

Supplied, in precedence order:

1. `-k` / `--api-key <key>` CLI flag.
2. `GROK_API_KEY` environment variable.
3. `apiKey` in `~/.grok/user-settings.json`:

   ```json
   { "apiKey": "your_key_here" }
   ```

Get a key from <https://x.ai> (console at console.x.ai).

### Endpoint / base URL

- `GROK_BASE_URL` (or `-u` / `--base-url <url>`) overrides the endpoint.
  Default: `https://api.x.ai/v1`. This is the seam for OpenAI-compatible
  proxies / gateways pointed at Grok-shaped models.
- `GROK_MAX_TOKENS` caps the response token budget.

### Multiple accounts / headless

There is no `/profile` or account-switch command. For per-run credential
isolation, pass `-k`/`GROK_API_KEY` and `-u`/`GROK_BASE_URL` in the child
process environment. Interactive auth is not required for `--prompt`
mode, so headless/CI runs only need the env vars set.

## Permissions

Grok CLI does **not** expose an allow/deny/ask rule file. Its permission
surface is two mechanisms:

1. **Workspace trust** — `~/.grok/workspace-trust.json` records per-
   directory trust decisions. Running `grok` in an untrusted directory
   prompts for trust before tools execute.
2. **Sandbox flags** (microVM isolation, primarily for `/verify` and
   `--verify`): `--sandbox` / `--no-sandbox`, `--allow-net`,
   `--allow-host <host>`, `--port <n>`. These gate network and host
   access for sandboxed runs rather than per-tool approval.

**No unattended auto-approve / permission-bypass flag** (a
`--dangerously-skip-permissions` / `--yolo` equivalent) is documented as
of 2026-07-09. For unattended runs the documented path is `--batch-api`
(xAI Batch API) and/or pre-trusting the workspace so no interactive
trust prompt blocks the run.

## Policies / Rules / Memory

`AGENTS.md` is the always-on instruction file, prepended to the system
prompt every turn.

| Tier      | File                              | Notes                                                     |
|-----------|-----------------------------------|-----------------------------------------------------------|
| Project   | `<repo>/AGENTS.md`                | Merged from git root down to cwd.                         |
| Directory | `<dir>/AGENTS.override.md`        | Overrides `AGENTS.md` in the same directory.              |

- Format is plain Markdown, no required frontmatter.
- Merge is additive from git root → cwd; the nearest `AGENTS.override.md`
  wins for its directory.
- No user-tier (`~/.grok/AGENTS.md`) global memory file is documented;
  global always-on content is not a documented feature as of
  2026-07-09.

## Orchestration / headless invocation

### Non-interactive launch

```
grok --prompt "<text>" [--format json] [--directory <dir>] \
     [--model <id>] [--max-tool-rounds <n>] [--session <id>] \
     [--api-key <key>] [--base-url <url>] [--batch-api]
```

- `--prompt` / `-p <text>` runs a single prompt then exits. The prompt is
  passed as the flag value (not a positional arg, not on stdin).
- `--format json` selects the machine-readable output stream.
- `--directory` / `-d <dir>` sets the working directory.
- `--max-tool-rounds <n>` caps agentic tool iterations (default 400).
- `--session <id>` (or `--session latest`) resumes a saved session.
- `--batch-api` routes the run through xAI's Batch API for lower-cost
  unattended execution (delayed result).
- Headless `--prompt` mode does **not** require terminal-UI support.

### Output stream protocol

`--format json` emits a **newline-delimited JSON event stream** (one
semantic, step-level record per line) instead of human-readable text.
Documented event `type` values: `step_start`, `text`, `tool_use`,
`step_finish`, `error`.

This is the same event family as opencode's `run --format json`
(both are superagent-ai / sst lineage). Canonical mapping (by analogy
with opencode; verify field paths against the running CLI):

| Category        | Event `type`  | Notes                                            |
|-----------------|---------------|--------------------------------------------------|
| Step boundary   | `step_start`  | Start of an agentic step.                         |
| Assistant text  | `text`        | Model output text.                                |
| Tool call/result| `tool_use`    | Carries tool name, input, and result state.       |
| Usage / finish  | `step_finish` | Token counts and step completion.                 |
| Error           | `error`       | Error name + message.                             |
| Completion      | stream end    | After the final `step_finish`.                    |

If a coordinator needs the exact per-field JSON shape, capture a live
`grok --prompt "..." --format json` run — the README documents the event
**names** but not their full field layout as of 2026-07-09.

### Model & reasoning at launch

- Model: `-m` / `--model <id>`, or `GROK_MODEL` env. No separate
  reasoning-effort flag is documented; effort is a property of the model
  id (e.g. `grok-code-fast-1`). Cross-reference Authentication for the
  provider credential.

### MCP at launch

There is **no run-scoped MCP flag** (no `--mcp-config <path>` and no
inline-env override are documented). MCP servers are read from
`.grok/settings.json` (project) or `~/.grok/user-settings.json` (user).
To supply a controlled server set for a single run, a coordinator writes
`<workdir>/.grok/settings.json` with the desired `mcpServers` block
before launch and runs with `--directory <workdir>`. Suppression of
ambient user-tier servers is not documented; use an isolated `HOME`
if the user's `~/.grok/user-settings.json` servers must not leak in.

### Skills at launch

A coordinator materialises skills into
`<workdir>/.agents/skills/<name>/SKILL.md` before launch (project tier).
Always-on context goes into `<workdir>/AGENTS.md`. (Cross-reference
Skills and Policies / Rules / Memory.)

### Tool approval in headless mode

No on-stream approval handshake and no auto-approve flag are documented.
Keep runs unattended by pre-trusting the workspace
(`~/.grok/workspace-trust.json`) and/or using `--batch-api`. There is no
documented `control_request`/`control_response` protocol on the JSON
stream as of 2026-07-09.

### Process lifecycle

- Framing: prompt in the `--prompt` flag value; events out on stdout as
  NDJSON under `--format json`; diagnostics on stderr.
- Cancellation: signal the process group (no documented stdin-close
  handshake, since the prompt is an argv flag, not a stdin stream).
- Minimum CLI version: not documented. The `--format json` event set is
  characterised from the current README as of 2026-07-09.

## Format quirks / gotchas

- **No config-dir override env var.** `~/.grok/` follows the OS home
  directory. To redirect the global tier, set `HOME` for the child; there
  is no `GROK_CONFIG_DIR`.
- **Prefer the project seam for per-run isolation.** Write
  `<workdir>/.grok/settings.json`, `<workdir>/AGENTS.md`, and
  `<workdir>/.agents/skills/` and launch with `--directory <workdir>` —
  this avoids touching the user's `~/.grok/` entirely.
- **MCP key is `mcpServers`, not `mcp`.** The DeepWiki schema names a
  user-settings `mcp` key; every other source and the README use
  `mcpServers`. Emit `mcpServers`.
- **Skills live under `.agents/skills/`, not `.grok/`.** The path is
  agent-neutral and shared with other harnesses.
- **Instruction file is `AGENTS.md`, not `GROK.md`.** `AGENTS.override.md`
  wins per directory. A `.grok/GROK.md` file appears only in third-party
  cheat-sheets and is unverified.
- **Sub-agents are JSON-only** (`subAgents[]` in `user-settings.json`),
  not per-file markdown. Reserved names: `general`, `explore`, `vision`,
  `verify`, `computer`.
- **No custom slash-command file format.** Express custom behaviour as
  sub-agents or skills.
- **No allow/deny permission rules.** Permission is workspace trust +
  sandbox flags; there is no per-tool rule file.
- **No auto-approve flag.** Pre-trust the workspace or use `--batch-api`
  for unattended runs.
- **Prompt is an argv flag** (`--prompt <text>`), not a positional arg
  and not stdin. This differs from opencode (positional) and CodeBuddy
  (stdin NDJSON).
- **`user-settings.json` is written mode `0600`.** Preserve permissions
  when editing.
- **`--max-tool-rounds` default is 400.** Lower it for bounded CI runs.

## Renderer notes (planned)

`agent-manager`'s Grok renderer should:

1. **Isolate via the project seam.** Launch with `--directory <workdir>`
   and write all managed config under `<workdir>/` so the user's
   `~/.grok/` is never mutated. If global isolation is also required, set
   an ephemeral `HOME` for the child process (there is no
   `GROK_CONFIG_DIR`).
2. **Rules → memory:** write `<workdir>/AGENTS.md` (Markdown). Use a
   managed marker block (e.g. `<!-- agent-manager:begin --> …
   <!-- agent-manager:end -->`) so user-authored content is preserved.
   Do not emit `GROK.md` (unverified).
3. **Skills** → write `<workdir>/.agents/skills/<id>/SKILL.md`.
   Frontmatter carries `name` (= `<id>`) and `description`.
4. **MCP** → write `<workdir>/.grok/settings.json` → `mcpServers.<id>`.
   stdio: `{ "type": "stdio", "command", "args", "env" }`. http/sse:
   `{ "type": "http", "url", "headers" }`. There is no `--mcp-config`
   flag, so the file is the only injection channel.
5. **Sub-agents** → emit into `~/.grok/user-settings.json` →
   `subAgents[]` (`name`, `model`, `instruction`) **only if** operating on
   the global tier; there is no project-tier sub-agent file. Skip
   reserved names.
6. **Auth** → pass `GROK_API_KEY` (and optionally `GROK_BASE_URL`,
   `GROK_MODEL`) in the child environment; do not write the key into a
   file the renderer does not own.
7. **Headless** → `grok --prompt <text> --format json --directory
   <workdir>`. Parse NDJSON events (`step_start`, `text`, `tool_use`,
   `step_finish`, `error`). Pre-trust the workspace or use `--batch-api`
   for unattended runs (no auto-approve flag exists).
8. **Files the renderer must not own:** the user's real
   `~/.grok/user-settings.json` and `~/.grok/workspace-trust.json` (only
   touch them for global-tier sub-agents/MCP with a marker discipline).
   **Files the renderer owns:** everything it writes under the ephemeral
   `<workdir>/.grok/`, `<workdir>/AGENTS.md`, and
   `<workdir>/.agents/skills/`.

Because there is **no structured `control_request` approval handshake**
and **no per-run MCP flag**, the Grok renderer is closer to a
"materialise files + passthrough" model than the flag-driven CodeBuddy
renderer.

## Sources

- Repo README — <https://github.com/superagent-ai/grok-cli> — canonical:
  install (`install.sh` / `bun add -g grok-dev`), binary `grok`,
  `--prompt`/`-p`, `--format json` event names (`step_start`, `text`,
  `tool_use`, `step_finish`, `error`), `--batch-api`, `--directory`,
  `--max-tool-rounds` (default 400), `--session latest`, `subAgents` in
  `~/.grok/user-settings.json`, reserved sub-agent names, `AGENTS.md` /
  `AGENTS.override.md`, skills under `.agents/skills/<name>/SKILL.md`,
  `mcpServers` in `.grok/settings.json`, env vars `GROK_API_KEY`,
  `GROK_BASE_URL` (default `https://api.x.ai/v1`), `GROK_MODEL`,
  `GROK_MAX_TOKENS`, `TELEGRAM_BOT_TOKEN`.
- npm package — <https://www.npmjs.com/package/@vibe-kit/grok-cli> —
  npm distribution (`npm i @vibe-kit/grok-cli`), binary `grok`, default
  endpoint `https://api.x.ai/v1`, MCP support, project- and global-level
  custom instructions.
- DeepWiki config reference —
  <https://deepwiki.com/superagent-ai/grok-cli/7-configuration-and-customization>
  — config paths `~/.grok/user-settings.json` (mode `0600`),
  `.grok/settings.json`, `AGENTS.md`/`AGENTS.override.md`; settings keys
  (`apiKey`, `defaultModel`, `mcp`, `telegram`, `model`); env vars and
  model resolution order; skills referenced in `src/utils/skills.ts`.
- DeepWiki MCP integration —
  <https://deepwiki.com/superagent-ai/grok-cli/5.1-mcp-server-integration>
  — `mcpServers` key; `src/mcp/runtime.ts`; stdio requires `command`,
  remote (`http`/`sse`) requires `url`; per-server fields `command`,
  `args`, `env`, `cwd`, `type`, `url`, `headers`, `label`, `id`.
- Cheat sheet — <https://cheatsheets.zip/grok-cli> — CLI flags
  (`-k`/`--api-key`, `-m`/`--model`, `-p`/`--prompt`, `-d`/`--directory`,
  `-u`/`--base-url`, `--max-tool-rounds`), `grok mcp` subcommands
  (`add`, `add-json`, `list`, `test`, `remove`), `grok git
  commit-and-push`, config files (mentions `.grok/GROK.md`, unverified).
- MCP support announcement (Superagent blog, referenced; host
  unreachable at fetch time 2026-07-09) —
  `https://www.superagent.sh/blog/grok-cli-mcp-support`.
