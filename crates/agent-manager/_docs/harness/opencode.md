# opencode

Stable id: `opencode`
Display name: opencode
Vendor: sst/opencode (GitHub: `anomalyco/opencode`)

## Quick reference

| Field          | Value                                                              |
|----------------|--------------------------------------------------------------------|
| Stable id      | `opencode`                                                         |
| Display name   | opencode                                                           |
| Vendor         | sst/opencode (formerly sst, now anomalyco)                         |
| Global root    | `~/.config/opencode/` (or `OPENCODE_CONFIG_DIR`)                   |
| Project root   | `<repo>/opencode.json` and/or `<repo>/.opencode/` (plural subdirs) |
| Config format  | JSONC (`opencode.json`), Markdown (memory / agents / commands)     |

## On-disk layout

### Global (`~/.config/opencode/`)

```
~/.config/opencode/
├── opencode.json             # or opencode.jsonc — runtime config
├── tui.json                  # or tui.jsonc — TUI-only settings
├── AGENTS.md                 # global rules / memory
├── agents/<name>.md          # global sub-agents (plural; agent/ accepted)
├── commands/<name>.md        # global slash commands (plural; command/ accepted)
├── skills/<name>/SKILL.md    # global skills (plural; one folder per skill)
├── plugins/<name>/           # plugins, plural
├── tools/<name>.ts           # custom tools, plural
├── themes/<name>.json        # custom themes, plural
└── modes/                    # legacy alias for agents in some places

# Claude / agent-neutral compat fallbacks (read by opencode):
~/.claude/CLAUDE.md
~/.claude/skills/<name>/SKILL.md
~/.agents/skills/<name>/SKILL.md
```

Two env vars can relocate / extend the global tier:

- `OPENCODE_CONFIG=/abs/path/to/file.json` — single-file config override
  (loaded between global and project).
- `OPENCODE_CONFIG_DIR=/abs/path/to/dir` — a second `.opencode`-shaped
  directory; loaded after the standard global + `.opencode` layer.
- `OPENCODE_CONFIG_CONTENT=<inline JSON>` — last-write inline override.
- `OPENCODE_TUI_CONFIG=/abs/path/to/tui.json` — alternate TUI config.

### Project (`<project>/`)

```
<project>/
├── opencode.json             # JSON/JSONC project config
├── tui.json                  # optional project-specific TUI settings
├── AGENTS.md                 # project rules / memory
├── CLAUDE.md                 # optional Claude-compat fallback for memory
└── .opencode/
    ├── agents/<name>.md      # project sub-agents (plural)
    ├── commands/<name>.md    # project slash commands (plural)
    ├── skills/<name>/SKILL.md
    ├── plugins/
    ├── tools/<name>.ts
    ├── themes/<name>.json
    └── modes/

# Claude / agent-neutral compat (read by opencode):
.claude/skills/<name>/SKILL.md
.agents/skills/<name>/SKILL.md
```

Config is walked up from CWD to the git worktree root. Both
`opencode.json` and `.opencode/` may exist together.

## Discovery precedence

Order (later overrides earlier; configs are **merged**, not replaced):

1. **Remote** — fetched from the authenticated provider's
   `.well-known/opencode` endpoint (organisational defaults).
2. **Global** — `~/.config/opencode/opencode.json` (and `tui.json`).
3. **Custom file** — `OPENCODE_CONFIG=<path>` environment variable.
4. **Project** — `opencode.json` walked up from CWD to git root
   (plus project `tui.json`).
5. **`.opencode/` directories** — agents, commands, plugins, skills,
   tools, themes, modes (global + project).
6. **Inline** — `OPENCODE_CONFIG_CONTENT=<json>`.
7. **Managed files** — `/Library/Application Support/opencode/`
   (macOS), `/etc/opencode/` (Linux), `%ProgramData%\opencode\`
   (Windows).
8. **macOS MDM** — `ai.opencode.managed` plist (highest, not user-
   overridable).

**Merging** happens per top-level key. For `AGENTS.md` the lookup uses a
"first match in each category wins" rule. For skills the lookup is
union-based across all locations (last-name-wins for collisions).

**Claude compatibility** can be turned off with env vars:

- `OPENCODE_DISABLE_CLAUDE_CODE=1` — disable all `.claude` support
  (memory + skills).
- `OPENCODE_DISABLE_CLAUDE_CODE_PROMPT=1` — disable only
  `~/.claude/CLAUDE.md`.
- `OPENCODE_DISABLE_CLAUDE_CODE_SKILLS=1` — disable only
  `.claude/skills/`.

## Feature matrix

| Feature        | Support | Where it lands                                            |
|----------------|---------|-----------------------------------------------------------|
| Rules          | full    | `AGENTS.md` (project + user) + `instructions: [...]`      |
| Skills         | full    | `skills/<id>/SKILL.md` (project + user, plural dir)       |
| MCP            | full    | `mcp.<id>` block in `opencode.json`                       |
| Agents         | full    | `agents/<id>.md` (project + user, plural dir)             |
| Slash commands | full    | `commands/<id>.md` (project + user, plural dir)           |
| Permissions    | full    | `permission` block in `opencode.json` (and per-agent)     |
| Policies       | full    | `experimental.policies` (action-level resource gating)   |

## Skills

### Locations

```
.opencode/skills/<name>/SKILL.md            # project (opencode-native)
~/.config/opencode/skills/<name>/SKILL.md   # global
.claude/skills/<name>/SKILL.md              # project (Claude-compat)
~/.claude/skills/<name>/SKILL.md             # global (Claude-compat)
.agents/skills/<name>/SKILL.md              # project (agent-neutral)
~/.agents/skills/<name>/SKILL.md             # global (agent-neutral)
```

Project skills are discovered by walking up from CWD to the git
worktree. All sources are merged into one flat namespace; skill names
must be unique across locations.

### Format

Markdown file with **YAML frontmatter**. Only these keys are recognised;
anything else is ignored.

| Key             | Required | Notes                                                                 |
|-----------------|----------|-----------------------------------------------------------------------|
| `name`          | yes      | Must match the folder name. Regex `^[a-z0-9]+(-[a-z0-9]+)*$`. 1–64.   |
| `description`   | yes      | 1–1024 chars. Shown to the model in the `skill` tool description.     |
| `license`       | no       | Free-form string.                                                     |
| `compatibility` | no       | Free-form (e.g. `opencode`).                                          |
| `metadata`      | no       | Map of string → string.                                               |

### Minimal skill

```markdown
---
name: git-release
description: Create consistent releases and changelogs
license: MIT
compatibility: opencode
metadata:
  audience: maintainers
  workflow: github
---

## What I do
- Draft release notes from merged PRs
- Propose a version bump
- Provide a copy-pasteable `gh release create` command
```

The agent sees an `<available_skills>` XML snippet listing every skill
(name + description) and loads the full body on demand via
`skill({ name: "git-release" })`.

### Commands vs skills (important distinction)

- A **command** is a *user-invoked* slash command (`/test`); its content
  is sent verbatim as a prompt template.
- A **skill** is a *model-invoked* package: the agent sees a list of
  available skills in the `skill` tool description and loads one.

Both can coexist: a skill can describe a workflow, a command can be the
keyboard shortcut that asks the agent to invoke that skill.

## Sub-agents

### Types

- **Primary agents** — own the main conversation; cycled via `Tab`
  (or `switch_agent` keybind).
- **Subagents** — invoked from a primary via the `task` tool or by
  the user typing `@name`.

Built-ins: `build` (primary, default), `plan` (primary, edits default
to ask), `general` (subagent), `explore` (subagent, read-only),
`scout` (subagent, read-only external), `compaction` (primary, hidden),
`title` (primary, hidden), `summary` (primary, hidden).

### Locations

```
~/.config/opencode/agents/<name>.md   # global
<project>/.opencode/agents/<name>.md  # project
```

Plus inline in `opencode.json` under the `agent` key. Filename = agent
name; `review.md` is invoked as `@review`.

### Markdown form (preferred)

```markdown
---
description: Reviews code for quality and best practices
mode: subagent
model: anthropic/claude-sonnet-4-20250514
temperature: 0.1
permission:
  edit: deny
  bash: deny
---

You are in code review mode. Focus on:
- Code quality and best practices
- Potential bugs and edge cases
- Performance implications
- Security considerations
```

### JSON form

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "agent": {
    "code-reviewer": {
      "description": "Reviews code for best practices and potential issues",
      "mode": "subagent",
      "model": "anthropic/claude-sonnet-4-20250514",
      "prompt": "You are a code reviewer…",
      "permission": { "edit": "deny" }
    }
  }
}
```

### Frontmatter / JSON keys

| Key          | Type                                             | Notes                                                                 |
|--------------|--------------------------------------------------|-----------------------------------------------------------------------|
| `description`| string                                           | **Required.** What the agent does.                                    |
| `mode`       | `"primary"` \| `"subagent"` \| `"all"`           | Defaults to `"all"`.                                                  |
| `model`      | `"provider/model-id"`                            | Optional. Subagents inherit from invoker if unset.                   |
| `prompt`     | string or `{file:...}` ref                       | System prompt content or path.                                        |
| `temperature`| number 0.0–1.0                                   | Model-specific defaults if unset.                                     |
| `top_p`      | number 0.0–1.0                                   | Alternative to temperature.                                           |
| `steps`      | integer                                          | Max agentic iterations. (`maxSteps` is deprecated.)                   |
| `disable`    | boolean                                          | Soft-disable the agent.                                               |
| `hidden`     | boolean                                          | Hide from `@` autocomplete.                                           |
| `color`      | hex (`"#ff6b6b"`) or theme token                  | UI tint.                                                              |
| `permission` | object                                           | Per-tool permissions, same schema as global `permission`.             |
| `tools`      | object                                           | **Deprecated** (boolean per-tool). Use `permission`.                  |

Unknown keys are forwarded to the provider as model options (e.g.
`reasoningEffort`, `textVerbosity` for OpenAI).

### Discovery precedence for agents

Project agent files override global agent files of the same name;
markdown agents and JSON `agent:` entries are merged with markdown
taking precedence for matching keys.

## MCP servers

MCP servers live **only inside the `mcp` block of `opencode.json`**
(project, global, or remote-config tier). There is no per-server file
convention.

### Shape

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "<server-name>": {
      "type": "local" | "remote",
      "enabled": true,
      "timeout": 5000
      // … type-specific fields …
    }
  }
}
```

### `type: "local"` (stdio)

```jsonc
{
  "mcp": {
    "mcp_everything": {
      "type": "local",
      "command": ["npx", "-y", "@modelcontextprotocol/server-everything"],
      "environment": { "MY_ENV_VAR": "value" },
      "enabled": true,
      "timeout": 5000
    }
  }
}
```

| Option         | Type        | Required | Description                                       |
|----------------|-------------|----------|---------------------------------------------------|
| `type`         | `"local"`   | yes      | Local stdio transport.                            |
| `command`      | string[]    | yes      | Argv to launch the server.                        |
| `environment`  | object      | no       | Extra env vars for the process.                   |
| `enabled`      | boolean     | no       | Default true.                                     |
| `timeout`      | number ms   | no       | Tool-list fetch timeout. Default 5000.            |

### `type: "remote"` (HTTP / SSE / streamable — single union)

opencode does **not** expose separate `sse` vs `http` types. Both are
`remote`; the transport is negotiated.

```jsonc
{
  "mcp": {
    "sentry": {
      "type": "remote",
      "url": "https://mcp.sentry.dev/mcp",
      "enabled": true,
      "headers": { "Authorization": "Bearer {env:SENTRY_TOKEN}" },
      "oauth": {
        "clientId":     "{env:MY_MCP_CLIENT_ID}",
        "clientSecret": "{env:MY_MCP_CLIENT_SECRET}",
        "scope":        "tools:read tools:execute"
      },
      "timeout": 5000
    }
  }
}
```

| Option    | Type                | Required | Description                                      |
|-----------|---------------------|----------|--------------------------------------------------|
| `type`    | `"remote"`          | yes      | Remote HTTP transport.                           |
| `url`     | string              | yes      | MCP endpoint URL.                                |
| `headers` | object              | no       | Static headers.                                  |
| `oauth`   | object \| `false`   | no       | `false` to disable auto-OAuth on 401.            |
| `enabled` | boolean             | no       | Default true.                                    |
| `timeout` | number ms           | no       | Default 5000.                                    |

OAuth state is stored at `~/.local/share/opencode/mcp-auth.json`.

### Per-agent gating

MCP tools register as `<server-name>_<tool>`. Disable globally then
re-enable per agent:

```jsonc
{
  "mcp":   { "my-mcp": { "type": "local", "command": ["bun", "x", "my-mcp"] } },
  "tools": { "my-mcp*": false },
  "agent": { "my-agent": { "tools": { "my-mcp*": true } } }
}
```

Wildcards: `*` (zero+ chars), `?` (one char).

## Slash commands

### Built-in (always present)

`/connect`, `/compact` (alias `/summarize`), `/details`, `/editor`,
`/exit` (alias `/quit`, `/q`), `/export`, `/help`, `/init`, `/models`,
`/new` (alias `/clear`), `/redo`, `/sessions` (alias `/resume`,
`/continue`), `/share`, `/themes`, `/thinking`, `/undo`, `/unshare`.

### Custom

Locations: `~/.config/opencode/commands/<name>.md` (global) and
`<project>/.opencode/commands/<name>.md` (project). Or inline under
the `command` key in `opencode.json`.

### Markdown form

```markdown
---
description: Run tests with coverage
agent: build
model: anthropic/claude-3-5-sonnet-20241022
subtask: false
---

Run the full test suite with coverage report and show any failures.
Focus on the failing tests and suggest fixes.
```

Frontmatter keys:

| Key           | Required | Notes                                                       |
|---------------|----------|-------------------------------------------------------------|
| `description` | no (recommended) | Shown in `/` autocomplete.                          |
| `agent`       | no       | Which agent runs it. If a subagent, defaults to `subtask: true`. |
| `subtask`     | no       | Force the command to spawn a subagent invocation.            |
| `model`       | no       | Override the model.                                          |

The body is a prompt template with these placeholders:

- `$ARGUMENTS` — everything after the command name.
- `$1`, `$2`, … — positional args (space-separated; quote multi-word).
- `` !`shell command` `` — inline bash whose stdout is injected.
- `@path/to/file` — file content reference.

### JSON form

```jsonc
{
  "command": {
    "test": {
      "template":    "Run the full test suite with coverage…",
      "description": "Run tests with coverage",
      "agent":       "build",
      "model":       "anthropic/claude-haiku-4-5"
    }
  }
}
```

`template` is **required** in JSON form. Custom commands may shadow
built-ins.

## Authentication

Opencode is the only harness in this set with a **single, uniform
provider map** that covers every supported LLM vendor. Authentication
is two-layered: (1) **provider credentials** (which model to talk to
and how to authenticate) and (2) the credential value itself
(env var, OAuth, or hard-coded). The `provider` block in
`opencode.json` selects (1); env vars, the `/connect` flow, or
explicit `options.apiKey` provide (2).

The full catalogue of supported providers is documented at
<https://opencode.ai/docs/providers/>.

### Anthropic (Claude)

Three auth options:

- `ANTHROPIC_API_KEY` env var.
- OAuth via `claude auth login` (then `setCacheKey` is unnecessary —
  opencode auto-discovers the same Keychain entry Claude Code uses).
- `apiKey` set inline in `opencode.json` (discouraged).

```jsonc
{
  "provider": {
    "anthropic": {
      "models": {
        "claude-sonnet-4-5": { "name": "Claude Sonnet 4.5" }
      }
    }
  }
}
```

### OpenAI

`OPENAI_API_KEY` env var. OpenAI-compatible endpoints (Azure, local
Ollama, OpenRouter) use the `options.baseURL` + `name` override:

```jsonc
{
  "provider": {
    "openai": {
      "name": "Ollama (local)",
      "options": { "baseURL": "http://localhost:11434/v1" },
      "models": { "llama3.3:70b": { "name": "Llama 3.3 70B" } }
    }
  }
}
```

The `name` field disambiguates the provider in the model picker.
The `models` map declares which model IDs are selectable; anything
not listed is not selectable from the UI but can be sent in `--model`.

### Google Gemini / Vertex

Two providers in the catalogue:

- `google` (Gemini API key via `GOOGLE_API_KEY` or
  `GEMINI_API_KEY`).
- `google-vertex` (Vertex AI, ADC-based, requires `GOOGLE_CLOUD_PROJECT`
  and `GOOGLE_CLOUD_LOCATION`).

Both providers share the model ID namespace (`gemini-2.5-pro`, etc.),
and the active provider is selected by which one has a valid
credential at launch.

### GitHub Copilot (via OAuth)

The unique feature of opencode: signing into GitHub unlocks Copilot's
model catalogue **without** needing a Copilot Business licence per
machine. Triggered by `/connect` in the TUI; the CLI opens a
browser, the user approves the opencode device, and the resulting
token is written to the opencode auth store (separate from
`~/.config/gh/hosts.yml`).

Once connected, the provider id `github-copilot` becomes available
in the model picker with model IDs like `gpt-4o`, `claude-3.5-sonnet`,
`o1-preview` — the actual backend is whatever Copilot's router
serves.

### Amazon Bedrock

`provider.bedrock` in `opencode.json`. AWS credentials follow the
standard chain (env, `~/.aws/credentials`, IAM role). Optional
`options.region` override; the model IDs are Bedrock-style
(`us.anthropic.claude-sonnet-4-5-...`).

For Bearer-token auth (e.g. for CodeWhisperer-style keys), set
`AWS_BEARER_TOKEN_BEDROCK` in the env and opencode picks it up
automatically.

### Azure OpenAI

`provider.azure`. Configuration:

```jsonc
{
  "provider": {
    "azure": {
      "options": {
        "baseURL": "https://<resource>.openai.azure.com/openai/deployments",
        "apiKey": "{env:AZURE_OPENAI_API_KEY}",
        "headers": { "api-version": "2024-10-21" }
      },
      "models": {
        "gpt-4o": { "name": "GPT-4o (Azure)" }
      }
    }
  }
}
```

The `headers.api-version` is required — Azure OpenAI does not infer
it from the URL.

### Custom provider (LiteLLM, OpenRouter, self-hosted)

Use `provider.<custom-name>` with `options.baseURL` and `name`. The
`apiKey` field can be:

- A literal string (not recommended).
- `"{env:VAR_NAME}"` — substituted from the env at call time.
- `"{file:./relative/path}"` — read from a project file (useful for
  one-line `.secrets/openai.key` mounted by a deploy pipeline).

```jsonc
{
  "provider": {
    "litellm": {
      "name": "LiteLLM proxy",
      "options": {
        "baseURL": "http://proxy.local:4000/v1",
        "apiKey": "{env:LITELLM_MASTER_KEY}"
      }
    }
  }
}
```

The `{env:...}` and `{file:...}` substitution syntax is opencode's
own and does not work in Claude Code / Codex / Gemini / Copilot.

### `/connect` command

The TUI slash command `/connect` is the auth-bootstrap UI. It lists
every supported provider, and for each one shows:

- A "Use env var" option (just point at the right env var name).
- A "Sign in with OAuth" option for providers that have a browser
  flow (GitHub Copilot, Anthropic, Google).
- A "Paste key" option (writes to the opencode auth store, not to
  a file).

`/connect` does not write to `opencode.json`; it writes to the
auth store (typically `~/.local/share/opencode/auth.json` on Linux,
`~/Library/Application Support/opencode/auth.json` on macOS). The
`opencode.json` is then updated to reference the credential
indirectly.

### `setCacheKey`

In multi-account setups, `setCacheKey` (set inside
`provider.<name>.options`) namespaces the credential lookup so that
two projects with different GitHub Copilot accounts (personal vs
employer) coexist. Default is `"default"`; per-project override:

```jsonc
{
  "provider": {
    "github-copilot": {
      "options": { "setCacheKey": "personal" }
    }
  }
}
```

### Multiple accounts / profiles

Opencode supports multiple providers in the same `opencode.json`.
The `model` field on a per-agent or per-command basis selects which
provider to use for that scope. The `--model` CLI flag overrides
for a single session. There is no `/profile` command — switching
is a config change.

For dev-vs-work split, the recommended pattern is two `opencode.json`
files (`opencode.personal.json` / `opencode.work.json`) symlinked
into the active path.

### Precedence summary

For a given model call, opencode resolves the credential in this
order:

1. `options.apiKey` literal in `opencode.json`.
2. `options.apiKey` with `{env:VAR}` substitution.
3. `options.apiKey` with `{file:path}` substitution.
4. The env var named by `options.apiKey` (e.g. `OPENAI_API_KEY`,
   `ANTHROPIC_API_KEY`).
5. The OS keychain entry, keyed by `setCacheKey` (for OAuth tokens).
6. The auth store written by `/connect`.

Managed config (admin-pushed) **overrides** user config, including
the provider map. The managed provider map can pin a model and a
key source, removing the user's ability to switch providers.

### Headless / CI

The `OPENCODE_CONFIG` env var points at an alternate `opencode.json`
(used to swap providers per CI job). The CLI's `--print` and
`--model` flags work without a TTY. OAuth via `/connect` requires
a TTY; for CI, use `apiKey` with `{env:CI_OPENCODE_KEY}` so the
secret stays in the runner's secret store.

### Troubleshooting

- `401 invalid api key` → env var name doesn't match the provider's
  expected name; the catalogue at <https://opencode.ai/docs/providers/>
  lists the env var each provider reads.
- `404 model not found` → the model ID is not in `provider.<x>.models`;
  add it or use the `--model` flag to bypass.
- `baseURL rejected` → the URL is missing a `/v1` suffix for
  OpenAI-shaped endpoints.
- `/connect` hangs → behind a corporate proxy; set `HTTPS_PROXY`
  and retry.

## Permissions

### Top-level config

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "permission": {
    "*":    "ask",
    "bash": "allow",
    "edit": "deny"
  }
}
```

`permission` may also be a single string applied to everything:
`"permission": "allow"`.

### Actions

- `"allow"` — run automatically.
- `"ask"` — prompt the user; UI offers `once` / `always` (for the rest
  of the session) / `reject`.
- `"deny"` — block.

### Recognised permission keys

| Key                  | Gates                                              | Granular object? |
|----------------------|----------------------------------------------------|------------------|
| `read`               | `read` tool (matched against file path)           | yes              |
| `edit`               | `edit`, `write`, `apply_patch`                     | yes              |
| `glob`               | `glob` (matched against pattern)                   | yes              |
| `grep`               | `grep` (matched against regex)                     | yes              |
| `list`               | `list`                                             | yes              |
| `bash`               | `bash` (matched against the parsed command)        | yes              |
| `task`               | `task` (matched against subagent type)             | yes              |
| `external_directory` | any tool touching paths outside the worktree       | yes (path patterns) |
| `lsp`                | LSP tool                                           | yes (currently non-granular in practice) |
| `skill`              | `skill` (matched against skill name)               | yes              |
| `todowrite`          | `todowrite`, `todoread`                            | shorthand only   |
| `webfetch`           | `webfetch` (matched against URL)                   | shorthand only   |
| `websearch`          | `websearch` (matched against query)                | shorthand only   |
| `question`           | `question`                                         | shorthand only   |
| `doom_loop`          | safety guard when the same tool input repeats 3×  | shorthand only   |

Keys are matched as **wildcard patterns against the tool name**, so
they apply equally to built-in, custom, and MCP tools — e.g.
`"mymcp_*": "deny"` denies every tool from a given MCP server.

### Granular (object) syntax

```jsonc
{
  "permission": {
    "bash": {
      "*":          "ask",
      "git *":      "allow",
      "npm *":      "allow",
      "rm *":       "deny",
      "grep *":     "allow"
    },
    "edit": {
      "*":                                       "deny",
      "packages/web/src/content/docs/*.mdx":     "allow"
    },
    "external_directory": {
      "~/projects/personal/**": "allow"
    }
  }
}
```

- Patterns: `*` (zero+ chars), `?` (one char). All other chars match
  literally.
- **Order matters: last matching rule wins.** Convention: put `*`
  first, narrower exceptions after.
- Home expansion: `~` and `$HOME` are expanded at the start of a
  pattern. This only affects pattern *spelling* — paths outside the
  worktree still need an explicit `external_directory` allow.

### Defaults if nothing is set

- Most permissions: `"allow"`.
- `doom_loop` and `external_directory`: `"ask"`.
- `read`: `"allow"`, but `.env` files are denied:
  ```jsonc
  {
    "permission": {
      "read": {
        "*":             "allow",
        "*.env":         "deny",
        "*.env.*":       "deny",
        "*.env.example": "allow"
      }
    }
  }
  ```

### Per-agent overrides

Global + agent permissions are merged; agent rules win for matching
keys. Markdown agents (under their YAML frontmatter) accept the exact
same `permission:` block.

### Special key: `permission.task`

Controls which subagents an agent may invoke via the `task` tool. A
`deny` entry removes the subagent entirely from the `task` tool
description so the model never sees it. Users can still call any
subagent directly via `@name`.

```jsonc
{
  "agent": {
    "orchestrator": {
      "permission": {
        "task": {
          "*":              "deny",
          "orchestrator-*": "allow",
          "code-reviewer":  "ask"
        }
      }
    }
  }
}
```

## Policies / Rules / Memory

opencode has **two related but distinct features** here:

1. **Memory / rules** — Markdown files (`AGENTS.md`, `CLAUDE.md`)
   prepended to the system prompt.
2. **Policies** — JSON statements under `experimental.policies` that
   gate access to *named resources* (e.g. LLM providers). Sibling of
   permissions, not the same thing.

### Memory: locations

| Tier     | File                          | Notes                                                  |
|----------|-------------------------------|--------------------------------------------------------|
| Project  | `<project>/AGENTS.md`         | Discovered by walking up from CWD. Commit this.        |
| Project, Claude-compat | `<project>/CLAUDE.md` | Used only if no `AGENTS.md` in the same upward walk.   |
| Global   | `~/.config/opencode/AGENTS.md` | Personal global rules.                                |
| Global, Claude-compat | `~/.claude/CLAUDE.md` | Used only if no `~/.config/opencode/AGENTS.md` exists. |

### Memory: precedence

> "The first matching file wins in each category."

So per category: `AGENTS.md` beats `CLAUDE.md`. Both the local and
global winners are then both loaded; they don't replace each other
across tiers.

### Memory: format

Plain Markdown, no required frontmatter. Use the `instructions` key in
`opencode.json` to declaratively layer in extra files:

```jsonc
{
  "instructions": [
    "CONTRIBUTING.md",
    "docs/guidelines.md",
    ".cursor/rules/*.md",
    "packages/*/AGENTS.md",
    "https://raw.githubusercontent.com/my-org/shared-rules/main/style.md"
  ]
}
```

Accepts file paths, globs, and remote URLs (5 s fetch timeout). All
entries are concatenated with the AGENTS.md tier into the system
prompt. opencode does **not** parse `@file` references inside
`AGENTS.md` automatically.

### Policies (experimental)

Distinct from permissions: permissions gate tool actions; policies
gate access to **named resources** at config-resolution time.

```jsonc
{
  "experimental": {
    "policies": [
      { "effect": "deny",  "action": "provider.use", "resource": "*" },
      { "effect": "allow", "action": "provider.use", "resource": "anthropic" }
    ]
  }
}
```

- Fields: `effect: "allow" | "deny"`, `action`, `resource`.
- Currently only `action: "provider.use"` is implemented; `resource`
  is a provider id (e.g. `openai`) and supports `*` / `?` wildcards.
- Last matching statement wins (same convention as permissions).
- **Precedence is inverted:** the *global* `experimental.policies`
  wins over the project's. A repo can't re-enable a provider you
  denied globally.

These supersede the older `disabled_providers` / `enabled_providers`
arrays (still accepted; `disabled_providers` wins over
`enabled_providers`).

## Orchestration / headless invocation

### Non-interactive launch

Argv: `opencode run --format json --dangerously-skip-permissions [--dir <cwd>] [--model <provider/model-id>] [--variant <level>] [--prompt <text>] [--session <id>] <prompt>`.

- `run` is the non-interactive subcommand; the prompt is a positional argument.
- `--format json` selects machine-readable output.
- Set `PWD=<cwd>` in the child env to override opencode's working-directory discovery.
- On Windows, resolve the real `opencode.exe` inside the npm package to bypass the `.cmd` shim.

### Output stream protocol

Newline-delimited JSON on stdout, one event per line. Event shapes:

```json
{"type":"step_start","sessionID":"..."}
{"type":"text","part":{"text":"..."},"sessionID":"..."}
{"type":"tool_use","part":{"tool":"bash","callID":"...","state":{"status":"complete","input":{...},"output":"..."}},"sessionID":"..."}
{"type":"error","error":{"name":"UnknownError","data":{"message":"..."}}}
{"type":"step_finish","part":{"tokens":{"input":0,"output":0,"cache":{"read":0,"write":0}}}}
```

Canonical mapping: assistant text = `text`; tool call/result = `tool_use` (carries both input and output in `state`); usage = `step_finish.part.tokens`; error = `error`; completion = stream end after `step_finish`.

### Model & reasoning at launch

- Model: `--model <provider/model-id>` (e.g. `anthropic/claude-sonnet-4-5`).
- Reasoning effort: `--variant <name>`. The valid variant names per model come from `opencode models --verbose` (each model's `variants` map); custom names declared in `opencode.json` are also valid.

### MCP at launch

A coordinator drives opencode headlessly by passing run-scoped MCP through the `OPENCODE_CONFIG_CONTENT` env var carrying inline JSON of the form `{"mcp":{...}}` (merged at the "local" scope, so it takes precedence over user/project config). No file is written to the workdir. The value accepts either opencode-native `{"mcp":{name:{type:"local"|"remote",...}}}` or a Claude-style `{"mcpServers":{...}}` block that is translated (`command`+`args` → `{type:"local","command":[...]}`, `url` → `{type:"remote","url":...}`). (Cross-reference the MCP servers section for the per-server schema.)

### Skills at launch

A coordinator materialises skills into `<workdir>/.opencode/skills/<name>/SKILL.md` before launch (plural `skills/` dir). Always-on context goes into `AGENTS.md` in the working directory. (Cross-reference Skills and Policies/Rules/Memory.)

### Tool approval in headless mode

`--dangerously-skip-permissions` runs every tool without confirmation; there is no on-stream approval handshake to answer. (For attended use, the `permission` block — see Permissions — gates tools instead.)

### Process lifecycle

- Framing: prompt in argv, events out on stdout (NDJSON), diagnostics on stderr.
- Cancellation: send `SIGTERM` to the process group, wait ~5 s, then `SIGKILL` the group; close the stdout reader afterward.
- Session resume: pass `--session <id>` to continue a prior session.

## Format quirks / gotchas

- **JSONC**, not strict JSON, is supported in `opencode.json` and
  `tui.json`. Strip comments before serializing.
- **Plural-only directory names are canonical**: `agents/`, `commands/`,
  `skills/`, `tools/`, `plugins/`, `themes/`, `modes/`. Singular
  (`agent/`, `command/`, …) is accepted only for back-compat; emit
  plural.
- **MCP has no `sse` / `http` distinction.** Both are `type: "remote"`.
- **`mcp` lives only in JSON.** No `.opencode/mcp/foo.json` convention.
- **Skills folder is mandatory.** A skill is `skills/<name>/SKILL.md`;
  you cannot inline a skill in `opencode.json`.
- **Skill `name` must equal the directory name** and match
  `^[a-z0-9]+(-[a-z0-9]+)*$`.
- **`tools` boolean key is deprecated** in favor of `permission`.
- **Permission rule ordering matters** — last match wins. Put `*`
  first, specific overrides later.
- **External paths still need `external_directory`**, even after `~/`
  expansion makes a pattern look "absolute".
- **`AGENTS.md` does not auto-load `@file` references.** Use
  `instructions` in `opencode.json` for declarative layering.
- **Memory project vs Claude-compat are alternatives, not merges**:
  `AGENTS.md` beats `CLAUDE.md` *within the same tier*.
- **Policy precedence is inverted** vs everything else.
- **Configs are merged top-level key by top-level key.** Affects how a
  sync tool should compose its own output — overwrite an entire
  managed file rather than partial-write into a user file you don't
  own.
- **MCP tool names get the server name as a prefix.** `gh_grep` server
  with a `search` tool becomes the tool `gh_grep_search`; permission
  patterns like `gh_grep_*` match accordingly.
- **`opencode.json` vs `tui.json` split is enforced.** Theme /
  keybind / scroll keys at the top level of `opencode.json` are
  deprecated; opencode auto-migrates them but warns. Write theme
  settings to `tui.json` only.
- **Custom tool file names become tool names**, with each named
  export becoming `<filename>_<exportname>`. There is no separate
  manifest — *the filesystem is the manifest*.
- **Plugins** loaded from npm are listed in `plugin: []` (singular
  key name, even though the on-disk directory is `plugins/`). The
  plural/singular split is intentional.
- **Built-in agents can be customized** by name (`agent.build`,
  `agent.plan`, etc.) — they're configured the same way as user
  agents.
- **Hidden agents** (`compaction`, `title`, `summary`) are real
  agents; they don't appear in the `@`-menu.
- **Variable substitution** in JSON: `{env:VAR_NAME}` and
  `{file:path/to/file}`. Home expansion is text-only.

## Renderer notes (planned)

`agent-manager`'s opencode renderer should:

1. **Rules → memory**: write `<project>/AGENTS.md` (Markdown) and/or
   `~/.config/opencode/AGENTS.md` (global). If you want to keep the
   source of truth in a separate file, instead write the path into
   `opencode.json` `instructions: ["..."]` and leave `AGENTS.md`
   untouched. Don't also create `CLAUDE.md` next to `AGENTS.md` in the
   same tier unless you mean it as a fallback.
2. **Skills** → write `<root>/.opencode/skills/<id>/SKILL.md` (project)
   and/or `~/.config/opencode/skills/<id>/SKILL.md` (global). Frontmatter
   only carries `name` (must equal `<id>`), `description`, optionally
   `license` / `compatibility` / `metadata`.
3. **Commands (slash commands)** → markdown form
   `<root>/.opencode/commands/<id>.md`. Frontmatter: `description`,
   `agent`, `model`, `subtask` (all optional). Body is the prompt
   template — support `$ARGUMENTS`, `$1`/`$2`/…, `` !`bash` ``,
   `@file`. File name = command id. Alternatively, emit
   `command: { id: { template, description, agent, model, subtask } }`
   in `opencode.json`; both forms coexist; markdown wins.
4. **Sub-agents** → markdown form `<root>/.opencode/agents/<id>.md`.
   Required frontmatter: `description`. For *primary* agent
   customisation (`build`, `plan`), prefer the JSON form
   `agent.build = { … }` in `opencode.json` (opencode treats built-ins
   specially).
5. **MCP** → emit entirely under `opencode.json` → `mcp.<id>`. stdio:
   `{ type: "local", command: ["argv0", …], environment, enabled }`.
   HTTP/SSE/Streamable: `{ type: "remote", url, headers?, oauth? }`.
   No per-server file. Use `{env:VAR}` substitution rather than
   literal values.
6. **Permissions** → top-level `permission: { … }`. Use object form
   for fine-grained rules on `bash`, `edit`, `read`, `glob`, `grep`,
   `list`, `task`, `external_directory`, `lsp`, `skill`. Use string
   `"allow"` / `"ask"` / `"deny"` for the rest. Keep `*` first,
   specific overrides later (last-match-wins). Per-agent overrides go
   under `agent.<name>.permission`. Avoid the legacy `tools` boolean
   form.
7. **Policies** → `experimental.policies: [ { effect, action,
   resource }, … ]`. Currently only `action: "provider.use"` is
   meaningful. Remember global wins over project.
8. **Hygiene**:
   - Always include `"$schema": "https://opencode.ai/config.json"`
     (and `"https://opencode.ai/tui.json"` in `tui.json`).
   - Keep TUI-only settings (`theme`, `keybinds`, scroll / mouse /
     attention) in `tui.json`, not `opencode.json`.
   - Use the *plural* names in `.opencode/...`.
   - Be idempotent: opencode merges configs; sort keys for
     deterministic output.
   - Only touch the keys you manage; leave foreign keys alone.

## Sources

- Index — <https://opencode.ai/docs/>
- Config — <https://opencode.ai/docs/config/>
- Agents — <https://opencode.ai/docs/agents/>
- Commands — <https://opencode.ai/docs/commands/>
- Skills — <https://opencode.ai/docs/skills/>
- MCP servers — <https://opencode.ai/docs/mcp-servers/>
- Permissions — <https://opencode.ai/docs/permissions/>
- Rules — <https://opencode.ai/docs/rules/>
- Policies — <https://opencode.ai/docs/policies/>
- Tools — <https://opencode.ai/docs/tools/>
- Themes — <https://opencode.ai/docs/themes/>
- Keybinds — <https://opencode.ai/docs/keybinds/>
- Formatters — <https://opencode.ai/docs/formatters/>
- Custom tools — <https://opencode.ai/docs/custom-tools/>
- TUI — <https://opencode.ai/docs/tui/>
- Repo — <https://github.com/sst/opencode> (now `github.com/anomalyco/opencode`)
- JSON schemas — `https://opencode.ai/config.json`, `https://opencode.ai/tui.json`, `https://opencode.ai/theme.json`
