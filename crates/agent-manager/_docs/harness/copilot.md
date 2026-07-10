# GitHub Copilot

Stable id: `copilot`
Display name: GitHub Copilot (VS Code / Copilot CLI / github.com)
Vendor: GitHub / Microsoft

> **Front-ends.** GitHub Copilot has three surfaces: **VS Code** (the
> editor extension), **Copilot CLI** (`copilot` binary), and
> **github.com** (Copilot Chat web, Copilot coding agent, Copilot code
> review). They share the `.github/` config layout but have different
> user-profile roots.

## Quick reference

| Field          | Value                                                                                                                                              |
|----------------|----------------------------------------------------------------------------------------------------------------------------------------------------|
| Stable id      | `copilot`                                                                                                                                          |
| Display name   | GitHub Copilot                                                                                                                                     |
| Vendor         | GitHub / Microsoft                                                                                                                                 |
| Global root    | VS Code: `~/Library/Application Support/Code/User/` (macOS), `~/.config/Code/User/` (Linux), `%APPDATA%\Code\User\` (Windows). CLI: `~/.copilot/`. Interop: `~/.claude/`. |
| Project root   | `<project>/.github/` and `<project>/.vscode/`                                                                                                      |
| Config format  | JSON, JSONC, Markdown with YAML frontmatter                                                                                                        |

## On-disk layout

### Global (user-profile; front-end dependent)

| Front-end             | Global root                                            | What's there                                                                                                          |
|-----------------------|--------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------|
| **VS Code (macOS)**   | `~/Library/Application Support/Code/User/`             | `mcp.json`, `settings.json` (`chat.*`, `github.copilot.*`), per-profile `prompts/`, `agents/`, `instructions/`, `hooks/`, `skills/` |
| **VS Code (Linux)**   | `~/.config/Code/User/`                                 | same as above                                                                                                          |
| **VS Code (Windows)** | `%APPDATA%\Code\User\`                                 | same as above                                                                                                          |
| **Copilot CLI**       | `~/.copilot/`                                          | `copilot-instructions.md`, `agents/`, `skills/`, `lsp-config.json`, `installed-plugins/`, `mcp.json`                  |
| **github.com**        | user account settings (no on-disk file)                | Personal instructions live in github.com → Copilot → Personal instructions                                            |
| **Interop**           | `~/.claude/`                                           | `CLAUDE.md`, `rules/`, `settings.json`, `agents/`, `skills/`                                                          |

```
# CLI
~/.copilot/
├── copilot-instructions.md            # global personal instructions
├── agents/<name>.agent.md             # user custom agents
├── skills/<name>/SKILL.md             # user skills
├── prompts/<name>.prompt.md           # user prompt files
├── instructions/<name>.instructions.md # user instructions
├── hooks/                             # user hooks
├── lsp-config.json                    # user LSP config
├── installed-plugins/                 # CLI-installed plugins
└── mcp.json                           # CLI MCP config (does NOT read .vscode/mcp.json)

# Interop
~/.claude/
├── CLAUDE.md
├── rules/                             # *.rules.md with paths: frontmatter
├── settings.json
├── agents/<name>.md                   # plain .md (no .agent.md)
├── skills/<name>/SKILL.md
└── commands/<name>.md
```

### Project (`<project>/`)

```
<project>/
├── .github/
│   ├── copilot-instructions.md              # repo-wide rules
│   ├── instructions/<name>.instructions.md  # path-scoped rules
│   ├── agents/<name>.agent.md               # custom agents (VS Code + CLI + github.com)
│   ├── prompts/<name>.prompt.md             # custom prompt files
│   ├── skills/<name>/SKILL.md               # Agent Skills (also .claude/skills/, .agents/skills/)
│   ├── hooks/<name>.json                    # lifecycle hooks
│   ├── copilot/mcp.json                     # CLI MCP config
│   ├── lsp.json                             # CLI LSP servers
│   └── plans/                               # default per-extension plan-artifact location
├── .vscode/
│   ├── mcp.json                             # VS Code MCP servers
│   └── settings.json                        # chat.* / github.copilot.* keys
├── .devcontainer/
│   └── devcontainer.json                    # customizations.vscode.mcp.servers
├── .claude/                                 # Claude-format interop
│   ├── agents/<name>.md                     # plain .md, NOT .agent.md
│   ├── rules/                               # *.rules.md with paths: frontmatter
│   ├── settings.json
│   └── skills/
├── .agents/                                 # agent-neutral compat
│   └── skills/
├── AGENTS.md                                # open standard, picked up by VS Code
└── CLAUDE.md                                # Claude interop, picked up by VS Code
```

## Discovery precedence

Copilot's docs describe **several** precedence orders, and they differ
by concept. A sync tool must reproduce the relevant one.

### Instructions (`.github/copilot-instructions.md` + `*.instructions.md`)

1. **Personal / user** (highest priority).
2. **Repository** (`.github/copilot-instructions.md`).
3. **Organization** (lowest; defined at GitHub-org level; requires
   `github.copilot.chat.organizationInstructions.enabled`).

Path-scoped `*.instructions.md` files: **all** files whose `applyTo`
glob matches the file being edited are loaded (additive). Repo-wide +
path-scoped are combined, not "one wins". For VS Code, the same file
at multiple levels is deduplicated by filename.

### Copilot CLI instruction discovery order (from `cli-best-practices.md`)

| Location                                         | Scope                  |
|--------------------------------------------------|------------------------|
| `~/.copilot/copilot-instructions.md`             | All sessions (global)  |
| `.github/copilot-instructions.md`                | Repository             |
| `.github/instructions/**/*.instructions.md`      | Repository (modular)   |
| `AGENTS.md` (in Git root or cwd)                 | Repository             |
| `COPILOT.md`, `GEMINI.md`, `CODEX.md`             | Repository             |

Additional dirs via `COPILOT_CUSTOM_INSTRUCTIONS_DIRS` env var
(comma-separated). The CLI looks for an `AGENTS.md` and any
`.github/instructions/**/*.instructions.md` files in each of those
dirs.

### Custom agents

Multiple locations are merged; the agent dropdown deduplicates by
`name` (or filename when `name` is absent). Lowest-level config wins
on collision:

- Enterprise-level → organization-level → repository-level.
- CLI user-profile vs repo: "If you have custom agents with the same
  name in both locations, the one in your home directory will be used,
  rather than the one in the repository."

### Prompt files

Loaded from every enabled `chat.promptFilesLocations` entry. Default:
`{ ".github/prompts": true }`. Workspace prompt files shadow user
prompt files on the same name.

### Custom instructions files (VS Code)

Loaded from every enabled `chat.instructionsFilesLocations` entry.
Default: `{ ".github/instructions": true, "~/.claude/rules": false }`.
User-level `~/.copilot/instructions/` is auto-loaded from the VS Code
user profile, not configured here.

### Agent Skills

Loaded from `chat.agentSkillsLocations` (default includes
`.github/skills`, `.claude/skills`, `~/.copilot/skills`,
`~/.claude/skills`).

### MCP servers

Workspace `.vscode/mcp.json` and user-profile `mcp.json` are both
loaded; servers with the same name are deduped (workspace wins by
convention; the merge order is not documented as a strict override).
CLI `~/.copilot/mcp.json` is a **separate** file; the CLI does **not**
read `.vscode/mcp.json`.

### Parent-repository discovery (monorepos)

With `chat.useCustomizationsInParentRepositories: true`, VS Code walks
up from each workspace folder to the nearest `.git` folder and
gathers customizations from every folder in between (inclusive).
Applies to: `copilot-instructions.md`, `AGENTS.md`, `CLAUDE.md`,
`*.instructions.md`, `.prompt.md`, `.agent.md`, `SKILL.md`, and hooks.
Off by default.

### Tool list priority (within a chat session)

1. Tools listed in the **prompt file** (`tools:` in `.prompt.md`).
2. Tools listed in the **referenced custom agent** (`tools:` in
   `.agent.md`, or the agent pointed to by the prompt file's `agent:`).
3. Default tools of the currently selected built-in agent (`ask` /
   `agent` / `plan`).

If both prompt and agent are used, the prompt's `tools` list wins.

## Feature matrix

| Feature        | Support | Where it lands                                                                                       |
|----------------|---------|------------------------------------------------------------------------------------------------------|
| Rules          | full    | `.github/copilot-instructions.md` + `.github/instructions/*.instructions.md` (with `applyTo`) + `AGENTS.md` + `CLAUDE.md` |
| Skills         | full    | `.github/skills/<id>/SKILL.md` (Agent Skills open standard, 2026)                                    |
| MCP            | full    | `.vscode/mcp.json` (VS Code) + `.github/copilot/mcp.json` (CLI) + plugin `.mcp.json`                 |
| Agents         | full    | `.github/agents/<id>.agent.md` (also `.claude/agents/<id>.md` for interop)                           |
| Slash commands | full    | `.github/prompts/<id>.prompt.md` and `.github/skills/<id>/SKILL.md`                                  |
| Permissions    | full    | `chat.tools.*.autoApprove` in `settings.json` + `chat.permissions.default` + `PreToolUse` hooks       |
| Policies       | full    | `.github/copilot-instructions.md` (repo-wide) + path-scoped `*.instructions.md` (with `applyTo` + `excludeAgent`) + `AGENTS.md` + `CLAUDE.md` |

## Skills

> **Terminology note.** "Skill" in Copilot means **two** different
> things: a **prompt file** (`.prompt.md`, the older "reusable prompt"
> concept) and an **Agent Skill** (`SKILL.md` in a folder, the 2026
> open standard from <https://agentskills.io>). Both appear as slash
> commands in chat. Both can be referenced as `tool:`
> (`#tool:webapp-testing`).

### Agent Skills (`SKILL.md`)

**Locations** (from `chat.agentSkillsLocations`):

```json
{
  ".github/skills":    true,
  ".claude/skills":    true,
  "~/.copilot/skills": true,
  "~/.claude/skills":  true
}
```

**Directory layout** (open `agentskills.io` standard):

```
.github/skills/
  webapp-testing/
    SKILL.md           # required
    test-template.js   # optional resources
    examples/          # optional
```

**File format:** Markdown with required YAML frontmatter. The `name`
field in the frontmatter **must equal the parent directory name**
(kebab-case, `[a-z0-9-]+`, max 64 chars). Names with slashes, colons,
dots, or namespace prefixes cause the skill to silently fail to load.

Frontmatter keys:

| Key                        | Required | Description                                                                                                |
|----------------------------|----------|------------------------------------------------------------------------------------------------------------|
| `name`                     | **Yes**  | Kebab-case identifier matching the directory name.                                                          |
| `description`              | **Yes**  | What the skill does **and when to use it** (max 1024 chars). Drives auto-discovery.                          |
| `argument-hint`            | No       | Hint text shown when invoked as a slash command.                                                            |
| `user-invocable`           | No       | `true` / `false`. Whether it appears in the `/` menu. Default `true`.                                        |
| `disable-model-invocation` | No       | `true` / `false`. Whether the model can auto-load it. Default `false`.                                       |
| `context`                  | No       | `inline` (default) or `fork` (run in a dedicated subagent; return only the final result).                    |

Minimal example:

```markdown
---
name: webapp-testing
description: Guide for testing web applications using Playwright. Use this when asked to create or run browser-based tests.
---

# Web Application Testing with Playwright

## When to use this skill
Use this skill when you need to create or debug Playwright tests.
```

### Prompt files (`.prompt.md`)

**Locations** (from `chat.promptFilesLocations`, default
`{ ".github/prompts": true }`):

| Scope        | Default location                                                |
|--------------|-----------------------------------------------------------------|
| Workspace    | `<repo>/.github/prompts/` (recursive subdirs allowed)            |
| User profile | VS Code user-data prompts folder, or `~/.copilot/prompts/` (CLI) |

**File name convention:** `<name>.prompt.md`. Slash command: `/<name>`.

Frontmatter keys:

| Key             | Required | Description                                                                                             |
|-----------------|----------|---------------------------------------------------------------------------------------------------------|
| `description`   | No       | Short description shown on hover.                                                                       |
| `name`          | No       | Name used after typing `/` in chat. Defaults to the file name.                                            |
| `argument-hint` | No       | Hint text in the chat input field.                                                                       |
| `agent`         | No       | `ask` / `agent` / `plan` / name of a custom agent. Defaults to current agent.                             |
| `model`         | No       | Model name. Defaults to the model picker selection.                                                       |
| `tools`         | No       | Tool / tool-set names; supports `<server>/*` for all of an MCP server.                                    |

Minimal example:

```markdown
---
agent: 'agent'
model: GPT-4o
tools: ['search/codebase', 'vscode/askQuestions']
description: 'Generate a new React form component'
---
Your goal is to generate a new React form component based on the templates
in the Github repo contoso/react-templates.
```

Body supports Markdown link references to other workspace files and
`${input:variableName}` (and `${input:variableName:placeholder}`) for
user prompts, plus `${selection}` for the editor selection.

Both Skills and Prompt files are exposed as slash commands in chat;
docs explicitly note: *"Agent skills also appear as slash commands
alongside prompt files."*

## Custom agents / Chat modes

> **Terminology drift.** "Custom agent" and "custom chat mode" are the
> same thing. Files were originally named `.chatmode.md`; they were
> renamed to `.agent.md` in 2025. Renaming existing `.chatmode.md`
> files to `.agent.md` and moving them to a `chat.agentFilesLocations`
> folder migrates them.

**Locations** (`chat.agentFilesLocations`, default
`{ ".github/agents": true }`):

| Scope                          | Default location                                                  |
|--------------------------------|-------------------------------------------------------------------|
| Workspace                      | `<repo>/.github/agents/` (recursive subdirs)                      |
| Workspace (Claude interop)     | `<repo>/.claude/agents/` (plain `.md` files)                      |
| User profile (VS Code)         | VS Code user-data agents folder                                   |
| User profile (Copilot CLI)     | `~/.copilot/agents/`                                              |
| Organization / Enterprise      | GitHub-org-level agents (requires `github.copilot.chat.organizationCustomAgents.enabled: true`) |

**File name convention:** `<name>.agent.md`. Filename may only contain
`.`, `-`, `_`, `a-z`, `A-Z`, `0-9`. Display name defaults to the
filename (minus `.agent.md`) unless `name:` is set.

**Format:** Markdown with YAML frontmatter + Markdown body (the body
is the agent's system prompt).

VS Code frontmatter keys:

| Key                        | Required                             | Type                      | Description                                                                                                                            |
|----------------------------|--------------------------------------|---------------------------|----------------------------------------------------------------------------------------------------------------------------------------|
| `description`              | **Yes** (GitHub docs) / No (VS Code) | string                    | Shown as placeholder in the chat input.                                                                                                |
| `name`                     | No                                   | string                    | Display name; defaults to the file name.                                                                                               |
| `argument-hint`            | No                                   | string                    | Hint text in the chat input.                                                                                                            |
| `tools`                    | No                                   | list of strings           | Tool / tool-set names. `<server>/*` enables all of an MCP server. Empty list `[]` = no tools. Omitted = all tools.                       |
| `agents`                   | No                                   | list of strings           | Names of agents available as subagents. `*` = all, `[]` = none. Requires `agent` in `tools`.                                            |
| `model`                    | No                                   | string or list of strings | Single model name or a prioritized list. Qualified form `Model Name (vendor)`, e.g. `GPT-5 (copilot)`.                                   |
| `user-invocable`           | No                                   | boolean                   | Whether the agent appears in the picker. Default `true`.                                                                                |
| `disable-model-invocation` | No                                   | boolean                   | Whether the model can auto-invoke this agent as a subagent. Default `false`.                                                             |
| `infer`                    | No                                   | boolean                   | **Deprecated.** Use `user-invocable` + `disable-model-invocation` instead.                                                                |
| `target`                   | No                                   | string                    | `vscode` or `github-copilot`. Restricts where the agent is available; omit = both.                                                       |
| `mcp-servers`              | No                                   | object (YAML)             | **VS Code/IDE-only behaviour: ignored.** On github.com / Copilot CLI it defines per-agent MCP servers.                                  |
| `handoffs`                 | No                                   | list                      | Each entry: `label`, `agent`, `prompt`, optional `send` (default `false`), optional `model`.                                            |
| `hooks`                    | No                                   | object (Preview)          | Agent-scoped hooks. Requires `chat.useCustomAgentHooks: true`.                                                                           |

Minimal example:

```markdown
---
description: Generate an implementation plan for new features or refactoring existing code.
name: Planner
tools: ['web/fetch', 'search/codebase', 'search/usages']
model: ['Claude Opus 4.5', 'GPT-5.2']
handoffs:
  - label: Implement Plan
    agent: agent
    prompt: Implement the plan outlined above.
    send: false
---

# Planning instructions
You are in planning mode. Your task is to generate an implementation plan...
```

### Tool aliases (case-insensitive; same table across VS Code and github.com)

| Primary    | Aliases                                       | Purpose                                |
|------------|-----------------------------------------------|----------------------------------------|
| `execute`  | `shell`, `Bash`, `powershell`                 | Run a shell command                    |
| `read`     | `Read`, `NotebookRead`                        | Read a file                            |
| `edit`     | `Edit`, `MultiEdit`, `Write`, `NotebookEdit`  | Edit / write a file                    |
| `search`   | `Grep`, `Glob`                                | Search the codebase                    |
| `agent`    | `custom-agent`, `Task`                        | Invoke a subagent                       |
| `web`      | `WebSearch`, `WebFetch`                       | Fetch / search the web                 |
| `todo`     | `TodoWrite`                                   | Structured task lists                  |

### Relationship: prompt vs agent vs skill

- A **prompt file** is a *one-off invocation*: `/my-prompt some args`
  runs the body once, optionally under a chosen agent.
- A **custom agent** is a *persistent persona* in the agents dropdown,
  bundling its own `tools`, `model`, `agents` allowlist, `handoffs`,
  and instructions.
- A prompt can **select** a custom agent via the `agent:` frontmatter
  key. Tools precedence in that combined session: prompt's `tools` >
  agent's `tools` > built-in default tools.
- An **Agent Skill** is similar to a prompt but folder-shaped: ships
  with scripts and resources, loads progressively, and is also a
  slash command.

## MCP servers

### Locations of the `mcp.json` file

| Front-end         | Workspace path                                                    | User-profile path                                                                                                                       |
|-------------------|-------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------|
| VS Code           | `<repo>/.vscode/mcp.json`                                         | `~/Library/Application Support/Code/User/mcp.json` (macOS), `~/.config/Code/User/mcp.json` (Linux), `%APPDATA%\Code\User\mcp.json` (Windows) |
| Copilot CLI       | `<repo>/.github/copilot/mcp.json` (also `~/.copilot/mcp.json`)    | `~/.copilot/mcp.json`                                                                                                                   |
| github.com        | configured via repo/org settings (not a checked-in file)          | n/a                                                                                                                                     |
| Dev container     | `<repo>/.devcontainer/devcontainer.json` → `customizations.vscode.mcp.servers` | n/a                                                                                                                       |

> The top-level key in VS Code's `mcp.json` and in
> `.github/copilot/mcp.json` is **`servers`**. In a **plugin's**
> `plugin.json` / `.mcp.json` the top-level key is **`mcpServers`**.

### `mcp.json` top-level structure

```json
{
  "servers": { ... },
  "inputs":  [ ... ],
  "sandbox": { ... }
}
```

- `servers` — object: server name → server config.
- `inputs` — array of input variable definitions (for API keys etc.).
- `sandbox` — optional: filesystem + network access rules for
  sandboxed servers. Only effective on macOS and Linux. (Distinct from
  `chat.agent.sandbox.*`.)

### `servers` entry schema

**stdio (local subprocess; most common):**

| Field             | Required | Description                                                                              |
|-------------------|----------|------------------------------------------------------------------------------------------|
| `type`            | Yes      | `"stdio"` (default in VS Code — can be omitted)                                          |
| `command`         | Yes      | Executable name (e.g. `npx`, `node`, `python`, `docker`)                                  |
| `args`            | No       | Array of args                                                                            |
| `cwd`             | No       | Working dir (defaults to workspace folder)                                                |
| `env`             | No       | Env vars; values can be string/number/null; supports `${input:...}` placeholders           |
| `envFile`         | No       | Path to a `.env` file (e.g. `"${workspaceFolder}/.env"`)                                  |
| `dev`             | No       | Dev mode: `watch` (glob to watch), `debug` (Node / Python only).                            |
| `sandboxEnabled`  | No       | `true` / `false` — run inside the mcp.json `sandbox` block.                                 |

**HTTP / SSE (remote):**

| Field       | Required | Description                                                                                |
|-------------|----------|--------------------------------------------------------------------------------------------|
| `type`      | Yes      | `"http"` or `"sse"` (VS Code tries HTTP stream first, falls back to SSE)                   |
| `url`       | Yes      | Server URL. Can be `unix:///path/server.sock#/subpath` or `pipe:///pipe/name` for local sockets. |
| `headers`   | No       | Object of HTTP headers, supports `${input:...}` placeholders.                              |
| `oauth`     | No       | `{ "clientId": "...", "enterpriseManaged": false }`                                          |

```json
{
  "servers": {
    "memory": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-memory"],
      "env": { "API_KEY": "${input:api-key}" },
      "sandboxEnabled": true
    },
    "context7": { "type": "http", "url": "https://mcp.context7.com/mcp" },
    "slack": {
      "type": "http",
      "url": "https://mcp.slack.com/mcp",
      "oauth": { "clientId": "example-client-id" }
    }
  }
}
```

### `inputs` — interactive prompts for secrets

Three input types: `promptString`, `pickString`, `command`. Each entry
is referenced as `${input:id}` from `env` / `headers` / `url` / `args`.
VS Code prompts on first server start, then stores the value.

### `sandbox` block (mcp.json, macOS/Linux only)

```json
{
  "servers": {
    "myServer": { "type": "stdio", "command": "npx", "args": ["-y", "@example/mcp-server"], "sandboxEnabled": true }
  },
  "sandbox": {
    "filesystem": {
      "allowWrite": ["${workspaceFolder}"],
      "denyRead":   ["${userHome}/.ssh"],
      "denyWrite":  []
    },
    "network": {
      "allowedDomains": ["api.example.com", "*.cdn.example.com"],
      "deniedDomains":  []
    }
  }
}
```

When sandboxed, tool calls are auto-approved.

## Slash commands

### Built-in (VS Code Copilot Chat)

`/explain`, `/fix`, `/tests`, `/new`, `/init`, `/create-prompt`,
`/create-instruction`, `/create-skill`, `/create-agent`,
`/create-hook`, `/yolo` (alias `/autoApprove`), `/disableYolo`,
`/prompts`, `/instructions`, `/agents`, `/skills`, `/hooks`,
`/newWorkspace`, `/setupTests`, `/startDebugging`, `/troubleshoot`.

Built-in Copilot CLI commands: `/login`, `/model`, `/agent`,
`/feedback`, `/experimental`, `/lsp`, `/banner`.

### Custom

Custom slash commands come from two sources, both surfaced under `/`:

| Source                | File                                          | Slash command   |
|-----------------------|-----------------------------------------------|-----------------|
| Prompt file (`.prompt.md`) | `<repo>/.github/prompts/<name>.prompt.md` | `/<name>`       |
| Agent Skill (`SKILL.md`)   | `<repo>/.github/skills/<skill>/SKILL.md`  | `/<name>`       |
| Custom agent          | `<repo>/.github/agents/<name>.agent.md`       | agents picker, not under `/` (unless `user-invocable: true` AND `disable-model-invocation: true`) |
| MCP server prompts    | provided at runtime by the MCP server         | `/<server>.<prompt>` |
| Plugin commands       | from installed plugins                        | `/<plugin>:<command>` for Claude-format plugins |

Settings controlling slash command exposure:

- `chat.promptFilesLocations` — which folders to scan for `.prompt.md`.
  Default: `{ ".github/prompts": true }`.
- `chat.agentSkillsLocations` — which folders to scan for `SKILL.md`.
- `chat.promptFilesRecommendations` — show specific prompt files as
  recommended actions on a new chat.
- `chat.agentFilesLocations` — which folders to scan for `.agent.md`.
  Default: `{ ".github/agents": true }`.

## Authentication

GitHub Copilot is the only harness in this set whose auth is
**inherited from the host's GitHub identity** — there is no per-CLI
API key by default. The CLI shells out to the same auth stores VS
Code uses, and adds an API-key path for the Copilot SDK only.

The CLI's own auth command set:

- `copilot auth login` — opens a browser to the GitHub OAuth flow.
- `copilot auth logout` — clears the local token.
- `copilot auth status` — shows the active account and the auth
  method.
- `copilot auth setup-token` — issues a short-lived token for CI
  use (requires an interactive session first).

### GitHub.com (personal account)

OAuth device flow. Token storage:

- macOS: Keychain (`gh:GitHub.com`).
- Linux: `~/.config/gh/hosts.yml` (mode `0600`), with the
  `oauth_token:` field for the active host.
- Windows: Credential Manager.

`gh auth login` (from the GitHub CLI) puts the same token in the
same place; the Copilot CLI reuses the GitHub CLI's auth when
present, so installing `gh` and running `gh auth login` is the
shortest path to a working `copilot` install.

### GitHub Enterprise / Business / Enterprise Cloud with SSO

For users on a paid plan (Business, Enterprise, or Enterprise
Cloud), the same OAuth flow is used, but the consent screen shows
the org's app and the resulting token is tied to that org's
Copilot entitlement. The CLI's `/model` picker will only show
models the org has enabled; `claude-3.7-sonnet` may be absent on a
Business plan that has not opted in.

SSO is enforced by GitHub, not by the CLI: if the org requires
SSO, the device flow will redirect through the IdP (Okta, Entra
ID, etc.) and the resulting token is a regular Copilot token —
no special CLI flag is needed.

### GitHub Enterprise Server (self-hosted)

For orgs running their own GitHub instance. The CLI accepts a
`--host` flag (or `GH_HOST` env var) pointing at the instance.
Auth uses the same device flow as github.com, against the
instance's own OAuth app. The Copilot entitlement must be
**purchased through the instance**, not through github.com.

### BYOK (use your own API key)

For users who want to use a non-Copilot model behind the same CLI.
Configure in `~/.copilot/config.json` (or via the `mcp.json` of
the host editor):

```jsonc
{
  "models": {
    "gpt-4o": {
      "provider": "openai",
      "apiKey": "{env:OPENAI_API_KEY}"
    },
    "claude-3.5-sonnet": {
      "provider": "anthropic",
      "apiKey": "{env:ANTHROPIC_API_KEY}"
    }
  }
}
```

The `provider` field is the literal string the SDK expects
(`openai`, `anthropic`, `azure`, `google`, `bedrock`, `vertex`,
`openai-completions`). The `apiKey` field accepts:

- A literal string (not recommended).
- `"{env:VAR_NAME}"` — substituted at call time.
- `"{file:./relative/path}"` — read from a project file.

The same `{env:...}` / `{file:...}` substitution syntax is
**opencode's**; Copilot reuses it.

> **`customOAIModels` is deprecated.** The older `chat.customOAIModels`
> VS Code setting still works in the editor for ad-hoc
> OpenAI-compatible endpoints, but the Copilot CLI does not read
> it. Use the `models` block in `~/.copilot/config.json` for the
> CLI. See
> <https://docs.github.com/en/copilot/customizing-copilot/using-your-own-api-key>
> for the current SDK-BYOK guide.

### `GITHUB_TOKEN` and `COPILOT_TOKEN` env vars

For CI and scripted use, the CLI accepts:

- `GITHUB_TOKEN` (or `GH_TOKEN`) — a fine-grained PAT with the
  `copilot` scope. Treated as a github.com user; no device flow.
- `COPILOT_TOKEN` — a Copilot-specific token issued by
  `copilot auth setup-token` from an interactive session. Scoped
  to the model catalogue the user has access to; rotates every
  ~24h.

`COPILOT_TOKEN` is preferred over `GITHUB_TOKEN` in CI: it does
not carry the user's full GitHub scopes and is the supported
path per the official CI guide.

### SecretStorage (VS Code extension)

When the CLI is launched from the VS Code Copilot extension, the
token is read from VS Code's `SecretStorage` API
(`vscode.ExtensionContext.secrets`) — the encrypted store managed
by VS Code, not a file the user can read. The CLI does not
support `--token` for this path; it inherits whatever VS Code has.

### Multiple accounts

GitHub CLI's `gh auth switch` is the canonical way to move between
GitHub identities. The Copilot CLI re-reads the active `gh`
identity on every prompt, so:

```bash
gh auth switch --user personal-user
copilot                           # uses personal-user's Copilot
gh auth switch --user work-user
copilot                           # uses work-user's Copilot
```

This is the only multi-account pattern the CLI natively supports.
There is no per-project `copilot auth switch`; if the project
needs a specific identity, the calling script must set
`GH_HOST` / `GITHUB_TOKEN` explicitly before launching.

### Precedence summary

Highest to lowest for a single `copilot` invocation:

1. `COPILOT_TOKEN` env var.
2. `GITHUB_TOKEN` / `GH_TOKEN` env var.
3. VS Code `SecretStorage` (only when launched from the
   extension).
4. `gh` CLI's active host token (`~/.config/gh/hosts.yml`).
5. `copilot auth login` cached token in OS keychain.

For the SDK BYOK path, `models.<id>.apiKey` is resolved first;
only if it returns empty does the CLI fall back to the user's
GitHub-issued Copilot quota.

### Headless / CI

The supported CI pattern is:

1. Run `copilot auth login` once on a developer machine.
2. Run `copilot auth setup-token` and copy the printed token into
   the CI secret store as `COPILOT_TOKEN`.
3. In the CI job, export `COPILOT_TOKEN` before invoking the CLI.

Never commit the token; never commit `GITHUB_TOKEN` with broad
scopes. Fine-grained PATs with only the `copilot` scope are
sufficient.

For self-hosted runners that need full GitHub access (not just
Copilot), `GITHUB_TOKEN` with `repo:read` and `copilot` is the
minimal set.

### Troubleshooting

- `401 Not logged in` → `gh auth status` shows the active host;
  if it is not what you expect, `gh auth switch --user <name>`.
- `404 model not found` → your plan does not include the
  requested model. Run `copilot auth status` to see the plan.
- SSO loop → the org requires SSO but the active token was
  issued before the SSO policy was applied; rerun
  `gh auth login` and complete the IdP step.
- `BYOK 401` → the env var named in `{env:...}` is unset; the
  literal string `"{env:OPENAI_API_KEY}"` is being sent to the
  provider.
- `COPILOT_TOKEN expired` → re-run `copilot auth setup-token`
  on a developer machine and update the secret.

### Credential capture & reuse (agent-manager)

> How `am account capture` / `am account login` snapshot and replay this
> harness's login into an ephemeral run. Records file **structure and non-secret
> metadata only** — token values are copied opaquely.

- **Bundle files (the credential snapshot):**
  - `~/.copilot/config.json` — **required**; Copilot CLI's own store, holding
    `copilotTokens` (keyed `<host>:<login>`), `lastLoggedInUser`, `loggedInUsers`.
  - `~/.config/gh/hosts.yml` — *recommended*; GitHub CLI interop, `oauth_token`
    per host/user. Capture both for full multi-account fidelity.
- **Relocation lever:** no dedicated override — set `HOME` to relocate
  `~/.copilot/` and `~/.config/gh/`.
- **Force file storage (skip keychain):** file storage is the default when no
  token env var is set; there is no keychain mode to disable for `~/.copilot/`.
  On macOS the `gh` fallback *can* read the Keychain (`gh:GitHub.com`), so a
  clean snapshot prefers the plaintext `hosts.yml`/`config.json`.
- **Login command (fresh-auth-into-temp):** `HOME=/tmp/x copilot auth login` (or
  `gh auth login`). Headless/CI: inject `COPILOT_TOKEN` or `GITHUB_TOKEN` as a
  reference account instead of snapshotting (`copilot auth setup-token` mints a
  token for that path). Env precedence: `COPILOT_TOKEN` → `GITHUB_TOKEN`/`GH_TOKEN`
  → `hosts.yml` → macOS Keychain.
- **Extractable metadata (non-secret):**

  | field | source | identifies |
  |---|---|---|
  | `lastLoggedInUser.login` | `~/.copilot/config.json` | active GitHub username *(identifying)* |
  | `lastLoggedInUser.host` | `~/.copilot/config.json` | GitHub instance (e.g. `github.com`) |
  | `loggedInUsers[]` | `~/.copilot/config.json` | all authenticated accounts |
  | `copilotTokens` keys | `~/.copilot/config.json` | `<host>:<login>` pairs (values are secret) |
  | `github.com.user` | `~/.config/gh/hosts.yml` | active `gh` username |

- **Do not copy:** `session-state/`, `session-store.db*`,
  `command-history-state.json`, `logs/` — session/machine-bound state.

## Permissions

Copilot's permission system has **three layers**:

### Layer 1 — Permission levels (per session)

Set in the chat input dropdown. Persisted with `chat.permissions.default`.

| Level                  | Description                                                                                       | Setting                              |
|------------------------|---------------------------------------------------------------------------------------------------|--------------------------------------|
| **Default Approvals**  | Use configured approval rules; show confirmation dialogs.                                        | `chat.permissions.default: "default"` |
| **Bypass Approvals**   | Auto-approve all tool calls.                                                                      | `chat.permissions.default: "autoApprove"` |
| **Autopilot** (Preview)| Auto-approve all + auto-respond to clarifying questions + auto-retry on errors.                   | `chat.permissions.default: "autopilot"` |

### Layer 2 — Tool auto-approval settings (in `settings.json`)

| Setting                                          | Type    | Default                                                                                                                                            | Purpose                                                                 |
|--------------------------------------------------|---------|----------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------|
| `chat.tools.global.autoApprove`                  | bool    | `false`                                                                                                                                            | Auto-approve every tool. Removes all security prompts.                  |
| `chat.tools.terminal.autoApprove`                | object  | `{ "rm": false, "rmdir": false, "del": false, "kill": false, "curl": false, "wget": false, "eval": false, "chmod": false, "chown": false, "/^Remove-Item\\b/i": false }` | Per-command allow/deny. `true` = auto, `false` = block. Patterns wrapped in `/.../` are regex (with optional flags). |
| `chat.tools.terminal.enableAutoApprove`          | bool    | `true`                                                                                                                                             | Master kill switch for the terminal auto-approve feature.               |
| `chat.tools.terminal.ignoreDefaultAutoApproveRules` | bool | `false`                                                                                                                                            | Disable the built-in default allow/deny rules.                          |
| `chat.tools.terminal.blockDetectedFileWrites`    | string  | `"outsideWorkspace"`                                                                                                                                | Require approval for file writes outside the workspace.                 |
| `chat.tools.edits.autoApprove`                   | object  | `{}`                                                                                                                                                | Map of file-path glob patterns → require approval before edit.         |
| `chat.tools.eligibleForAutoApproval`             | object  | `[]`                                                                                                                                                | Map of tool names → `false` to force manual approval.                   |
| `chat.tools.urls.autoApprove`                    | object  | `[]`                                                                                                                                                | URL allowlist.                                                          |
| `chat.agent.enabled`                             | bool    | `true`                                                                                                                                              | Master switch for the agent mode.                                       |
| `chat.agent.maxRequests`                         | number  | `25`                                                                                                                                                | Max requests per session.                                              |
| `chat.agent.sandbox.enabled`                     | string  | `"off"`                                                                                                                                            | `"off"` / `"on"` (full FS+net isolation) / `"allowNetwork"` (FS only).  |
| `chat.agent.sandbox.FileSystem.mac`              | object  | `{}`                                                                                                                                                | `{ allowRead, allowWrite, denyRead, denyWrite }` arrays of paths.       |
| `chat.agent.sandbox.FileSystem.linux`            | object  | `{}`                                                                                                                                                | Same shape.                                                             |
| `chat.agent.networkFilter`                       | bool    | `false`                                                                                                                                             | Master switch for network domain filtering on agent tools.             |
| `chat.agent.allowedNetworkDomains`               | array   | `[]`                                                                                                                                                | Glob domains the agent can call.                                        |
| `chat.agent.deniedNetworkDomains`                | array   | `[]`                                                                                                                                                | Glob domains blocked. Denied > allowed.                                |

Example `chat.tools.terminal.autoApprove`:

```jsonc
{
  "chat.tools.terminal.autoApprove": {
    "mkdir": true,
    "/^git (status|show\\b.*)$/": true,
    "del": false,
    "/dangerous/": false
  }
}
```

A command is auto-approved only if **every** subcommand matches a `true`
entry and **no** subcommand matches a `false` entry. For whole-line
matching, use the object form: `"foo": { "approve": true,
"matchCommandLine": true }`.

> **Important:** Wildcards like `*` are **not** glob patterns; they are
> exact token matches. `"*": true` does **not** allow everything.

### Layer 3 — Hooks

Hooks are programmatic gatekeepers. A hook can deny a single tool
call without stopping the session.

**Hook events:** `SessionStart`, `UserPromptSubmit`, `PreToolUse`,
`PostToolUse`, `PreCompact`, `SubagentStart`, `SubagentStop`, `Stop`.

**Hook locations:**

- Workspace: `<repo>/.github/hooks/*.json` (every `.json` file).
- User: `~/.copilot/hooks/` (folder) and `~/.claude/settings.json`
  (single file, Claude interop).
- Agent-scoped: `hooks:` field in a `.agent.md`'s frontmatter (preview,
  requires `chat.useCustomAgentHooks: true`).
- Configurable via `chat.hookFilesLocations` (default:
  `{ ".github/hooks": true, ".claude/settings.local.json": true, ".claude/settings.json": true, "~/.claude/settings.json": true }`).

Hook configuration format (same as Claude Code):

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "type": "command",
        "command": "./scripts/validate-tool.sh",
        "windows": "powershell -File scripts\\validate.ps1",
        "linux":   "./scripts/validate-linux.sh",
        "osx":     "./scripts/validate-mac.sh",
        "cwd": ".",
        "env": { "AUDIT_LOG": ".github/hooks/audit.log" },
        "timeout": 15
      }
    ]
  }
}
```

**Hook input** (for `PreToolUse`):

```json
{
  "timestamp": "2026-02-09T10:30:00.000Z",
  "cwd": "/path/to/workspace",
  "sessionId": "...",
  "hookEventName": "PreToolUse",
  "tool_name": "editFiles",
  "tool_input": { "files": ["src/main.ts"] },
  "tool_use_id": "tool-123"
}
```

**Hook output** (for `PreToolUse`):

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "...",
    "updatedInput": { ... },
    "additionalContext": "..."
  }
}
```

**Permission decision priority:** `deny` > `ask` > `allow`.

**Exit codes:** `0` = success (parse stdout JSON), `2` = blocking
error (stop processing, show stderr to model), other = non-blocking
warning.

> **Cross-tool quirk:** VS Code's hook tools are named `editFiles` /
> `create_file` / `replace_string_in_file`; Claude Code's are `Write` /
> `Edit`. VS Code reads `.claude/settings.json` but currently **ignores
> matcher values** and does not translate tool names.

### Custom-agent-scoped permissions

A custom agent can restrict tool access by listing `tools:`. An empty
list `[]` disables all tools; omitted = all tools.

## Policies / Rules / Memory

The "always-on" instructions system in Copilot has **four** file types
that all behave differently.

### 1. Repository-wide: `.github/copilot-instructions.md`

- **Location:** exactly `<repo-root>/.github/copilot-instructions.md`.
- **Format:** plain Markdown, no frontmatter required.
- **Auto-loaded:** yes — VS Code reads it automatically, GitHub.com
  reads it automatically, Copilot CLI reads it automatically.
- **Scope:** the entire repo, every chat request, every PR review.
- **Disabling per PR review:** per-repo on/off toggle in repo Settings
  → Copilot → Code review.
- **Maximum length:** roughly 2 pages.

```markdown
# Project general coding standards

## Naming Conventions
- Use PascalCase for component names, interfaces, and type aliases
- Use camelCase for variables, functions, and methods
- Prefix private class members with underscore (_)
- Use ALL_CAPS for constants

## Error Handling
- Use try/catch blocks for async operations
- Implement proper error boundaries in React components
- Always log errors with contextual information
```

### 2. Path-scoped: `.github/instructions/<name>.instructions.md`

- **Location:** `<repo>/.github/instructions/` (recursive subdirs
  allowed).
- **Format:** Markdown with **YAML frontmatter** containing the
  `applyTo` glob.
- **Auto-loaded:** when the file the agent is editing matches
  `applyTo`.
- **Allowed frontmatter keys:** `name`, `description`, `applyTo`. (No
  tool allowlist, no model override — that belongs in custom agents.)
- **Special key `excludeAgent`:** `"code-review"` or `"cloud-agent"`
  — prevents that agent from reading the file. (Key is documented for
  files used on github.com; the VS Code parser tolerates it.)
- **Multiple `applyTo` patterns:** comma-separated, e.g.
  `applyTo: "**/*.ts,**/*.tsx"`.

```markdown
---
applyTo: "**/*.py"
---
# Python coding standards
- Follow the PEP 8 style guide.
- Use type hints for all function signatures.
- Write docstrings for public functions.
- Use 4 spaces for indentation.
```

**Glob semantics:**

- `*` — files in the current dir only
- `**` or `**/*` — files at any depth
- `*.py` — `.py` files in the current dir
- `**/*.py` — `.py` files at any depth
- `src/*.py` — `.py` files directly in `src/`, not in subdirs
- `src/**/*.py` — `.py` files in `src/` or any subdir
- `**/subdir/**/*.py` — `.py` files inside any `subdir` at any depth

**Loader rules:** All matching files are loaded additively
(repo-wide + every matching path-scoped). Order is not guaranteed.
If `applyTo` is missing, the file is **not** auto-applied; it can
still be referenced by Markdown link from another instruction or
prompt file (controlled by `chat.includeReferencedInstructions`).

### 3. `AGENTS.md` (open standard)

- **Location:** any directory in the repo. VS Code picks the **nearest**
  `AGENTS.md` to the file being edited.
- **Format:** plain Markdown.
- **Auto-loaded:** yes (when `chat.useAgentsMdFile: true`, default).
- **Nested:** `chat.useNestedAgentsMdFiles: true` (experimental, default
  `false`) enables discovery of multiple `AGENTS.md` files in subfolders.
- **Precedence:** the nearest `AGENTS.md` to the file being worked on
  wins on conflict.
- **Also recognised on github.com:** Copilot cloud agent + Copilot code
  review look for `AGENTS.md`, `CLAUDE.md`, or `GEMINI.md` in the repo
  root.

### 4. `CLAUDE.md` (Claude interop)

- **Locations:** `<repo>/CLAUDE.md`, `<repo>/.claude/CLAUDE.md`,
  `~/.claude/CLAUDE.md`, plus `CLAUDE.local.md` for uncommitted
  personal overrides.
- **Format:** plain Markdown.
- **Auto-loaded:** yes (when `chat.useClaudeMdFile: true`, default).
- **Sibling to `AGENTS.md`** — VS Code treats it the same way.
- **`.claude/rules/` interop:** uses a `paths:` frontmatter property
  (array of globs) instead of `applyTo`. Defaults to `**` (all files)
  when omitted.

### VS Code instruction discovery order

1. Built-in / default.
2. Files in **every** enabled `chat.instructionsFilesLocations` entry.
   Default: `{ ".github/instructions": true, "~/.claude/rules": false }`.
3. User profile (per-profile `instructions` folder and
   `~/.copilot/instructions/`).
4. If `chat.useCustomizationsInParentRepositories: true`, walks up to
   the nearest `.git` folder and gathers customizations from every
   folder in between.

### Settings controlling the instruction loader

| Setting                                            | Default                                       | Effect                                                                                      |
|----------------------------------------------------|-----------------------------------------------|---------------------------------------------------------------------------------------------|
| `chat.instructionsFilesLocations`                  | `{ ".github/instructions": true, "~/.claude/rules": false }` | Map of glob path → bool. `false` disables that location entirely.                          |
| `chat.includeApplyingInstructions`                 | `true`                                        | When `true`, files whose `applyTo` matches are auto-attached.                                |
| `chat.includeReferencedInstructions`               | `false`                                       | When `true`, instructions referenced via Markdown links are auto-attached.                    |
| `github.copilot.chat.codeGeneration.useInstructionFiles` | `true`                                  | Auto-add `.github/copilot-instructions.md` to chat requests.                                |
| `github.copilot.chat.organizationInstructions.enabled` | `true`                                     | Discover organization-level instructions.                                                    |
| `chat.useCustomizationsInParentRepositories`       | `false`                                       | Walk up to parent repo root in monorepos.                                                     |
| `chat.useAgentsMdFile`                             | `true`                                        | Recognise `AGENTS.md`.                                                                        |
| `chat.useNestedAgentsMdFiles`                      | `false` (experimental)                        | Recognise nested `AGENTS.md`.                                                                 |
| `chat.useClaudeMdFile`                             | `true`                                        | Recognise `CLAUDE.md`.                                                                        |

### Settings-based instructions (legacy, partially deprecated)

Three settings accept an array of `{ text, file }` objects for
specific flows (the only **non-file-based** instructions left in VS
Code):

| Setting                                                | Purpose                                |
|--------------------------------------------------------|----------------------------------------|
| `github.copilot.chat.reviewSelection.instructions`     | Code review (selection)                |
| `github.copilot.chat.commitMessageGeneration.instructions` | AI-generated commit messages       |
| `github.copilot.chat.pullRequestDescriptionGeneration.instructions` | AI-generated PR descriptions |

`github.copilot.chat.codeGeneration.instructions` and the
test-generation equivalent were **deprecated in VS Code 1.102** in
favour of file-based instructions.

## Orchestration / headless invocation

### Non-interactive launch

Argv: `copilot -p "<prompt>" --output-format json --allow-all --no-ask-user [--model <id>] [--resume <session-id>]`.

- `-p` passes the prompt as a CLI argument (no stdin write).
- `--output-format json` selects machine-readable output.
- `--allow-all` auto-approves every tool; `--no-ask-user` suppresses interactive questions.
- On Windows the invocation must be routed through PowerShell to avoid `cmd.exe` argument mangling.

### Output stream protocol

Newline-delimited JSON on stdout, one event per line. Event shapes:

```json
{"type":"session.start","data":{"sessionId":"...","selectedModel":"..."}}
{"type":"assistant.message_delta","data":{"deltaContent":"..."}}
{"type":"assistant.reasoning","data":{"content":"..."}}
{"type":"tool.execution_complete","data":{"toolCallId":"...","success":true,"result":{},"model":"..."}}
{"type":"result","sessionId":"...","exitCode":0}
{"type":"session.error","data":{"message":"..."}}
```

Canonical mapping:

- Assistant text — accumulate `assistant.message_delta` `.data.deltaContent` fields in order.
- Reasoning — `assistant.reasoning` `.data.content`.
- Tool result — `tool.execution_complete` `.data.result`.
- Completion — `result` (final event); `.exitCode` is authoritative.
- Error — `session.error` `.data.message`.

### Model & reasoning at launch

- Model: `--model <id>`. Available models depend on the GitHub plan (cross-reference Authentication).
- No dedicated reasoning-effort flag is exposed by the CLI; reasoning text, when present, arrives as `assistant.reasoning` events.

### MCP at launch

The Copilot CLI has no per-run MCP-injection flag; it reads MCP from its own config files (`~/.copilot/mcp.json`, `<repo>/.github/copilot/mcp.json`). A coordinator that needs run-scoped MCP writes those files before launch. Note the CLI does **not** read `.vscode/mcp.json`. (Cross-reference MCP servers.)

### Skills at launch

A coordinator materialises skills into `<workdir>/.github/skills/<name>/SKILL.md` before launch. Always-on context goes into `AGENTS.md` (or `.github/copilot-instructions.md`) in the working directory. (Cross-reference Skills and Policies/Rules/Memory.)

### Tool approval in headless mode

`--allow-all` + `--no-ask-user` make the run fully unattended: tools execute without confirmation and clarifying questions are suppressed. There is no on-stream approval handshake to answer (unlike the stream-json control protocol of some harnesses). For finer control, `PreToolUse` hooks (see Permissions → Layer 3) can still deny individual calls.

### Process lifecycle

- Framing: prompt in argv, events out on stdout (NDJSON), diagnostics on stderr.
- Cancellation: close the stdout reader on cancel and collect the process exit status; the `result` event's `exitCode` is the authoritative outcome.
- Session resume: pass `--resume <session-id>` (value from a prior `session.start` event) to continue a previous session.
- Minimum version: the `--output-format json` envelope is stable from **Copilot CLI ≥ 1.0.0**.

### Model discovery & selection (agent-manager)

> How `am <harness> --list-models` enumerates models and `am <harness> --model <id>`
> selects one. Facts verified against the installed binary on 2026-07-10.

- **Discover (list models):** `copilot help config` → `model:` setting lists available models (depends on GitHub plan; static list in help docs). Needs network/auth: no (list shown in help without auth).
- **Select at launch (passthrough):** `--model <id>` CLI flag, or `COPILOT_MODEL` environment variable.
- **Model id format:** Bare model name (e.g., `gpt-5.4`, `claude-sonnet-4.5`) or optional qualified form `<name> (<vendor>)`.
- **Example ids (verified):** `gpt-5.4`, `gpt-5.4-mini`, `claude-sonnet-4.5`, `claude-haiku-4.5`, `gemini-3.5-flash`, `kimi-k2.7-code`.
- **Default model:** Resolved per precedence (highest first): `COPILOT_MODEL` env → `--model` flag → `~/.copilot/settings.json` → `model` key → last selected model in session.

## Format quirks / gotchas

1. **The "Skills" word is overloaded.** A "skill" in older Copilot docs
   means a `.prompt.md` file. A "Skill" (capital S) in 2026 docs means
   the new `SKILL.md` standard. Both are slash commands, both scan
   similar folders, both can be invoked with `/`.
2. **Three top-level keys for MCP config, depending on the file.**
   `servers` — VS Code's `.vscode/mcp.json`, the CLI's
   `.github/copilot/mcp.json`. `mcpServers` — plugin `.mcp.json` and
   the `mcpServers` field in `plugin.json`. `inputs` + `sandbox` —
   only in the user/workspace `mcp.json`, not in plugin format.
3. **`.agent.md` vs `.chatmode.md`.** Custom agents used to be called
   "custom chat modes" and stored in `.chatmode.md`. The file
   extension was renamed. If you find old `.chatmode.md` files in
   the wild, rename + move to `.github/agents/`. There is no
   automatic migration.
4. **Claude-format vs VS Code format for agents and instructions.**
   Agents in `.claude/agents/` use plain `.md` (no extension change)
   and accept `tools` as a **comma-separated string** and a separate
   `disallowedTools` field. VS Code's `.agent.md` uses `tools` as a
   **YAML array** of tool names. Instructions in `.claude/rules/` use
   a `paths:` (array) frontmatter, not `applyTo:` (string).
5. **`mcp.json` `sandbox` block ≠ `chat.agent.sandbox.*` setting.**
   They are independent mechanisms. The mcp.json `sandbox` only
   applies to MCP servers that have `sandboxEnabled: true` (and is
   macOS/Linux only). The `chat.agent.sandbox.*` settings apply to
   **all** agent terminal commands (also macOS/Linux only).
6. **Tool-name casing.** Tool aliases are case-insensitive on
   github.com but VS Code tool names (`editFiles`,
   `search/codebase`, `create_file`, `replace_string_in_file`) are
   camelCase. If a custom agent lists `"Bash"`, GitHub's cloud agent
   maps it to `bash`; VS Code does its own mapping via
   `chat.tools.terminal.*`.
7. **`applyTo` is a string, `paths` is an array.** Functionally
   equivalent but the **key name differs** by source format.
   `applyTo: "**/*.py"` is **not** valid in `.claude/rules/`.
8. **`name` field rules differ.** In an `SKILL.md`, the `name` must
   equal the parent directory name and contain only `[a-z0-9-]`. In
   a `.agent.md` filename, only `. - _ a-z A-Z 0-9` are allowed. In
   `plugin.json`, the plugin `name` must be kebab-case `[a-z0-9-]`.
   Slashes, colons, and dots silently fail in skills and plugins —
   there is no error message.
9. **`chat.tools.terminal.autoApprove` does not support glob
   patterns.** It is exact token match (or `/regex/`). The only way
   to "allow everything" is to set `chat.tools.global.autoApprove:
   true` (or use the `chat.permissions.default` Bypass/Autopilot
   level).
10. **VS Code's `chat.tools.terminal.ignoreDefaultAutoApproveRules`
    flips both allow and deny defaults off.** If you set this to
    `true`, you must define every rule yourself.
11. **Hooks: tool names differ between VS Code and Claude Code.** VS
    Code uses `editFiles`, `createFile`, `replace_string_in_file`;
    Claude uses `Edit`, `Write`. A shared script must inspect
    `tool_name` and branch.
12. **Hooks: matcher values are ignored.** VS Code parses Claude
    Code's `matcher: "Edit|Write"` syntax for compatibility but does
    **not** apply it. All hooks run on every matching event.
13. **Personal instructions location differs by front-end.**
    github.com: account UI (no on-disk file). Copilot CLI:
    `~/.copilot/copilot-instructions.md`. VS Code: per-profile
    `instructions/` folder. The CLI's
    `~/.copilot/copilot-instructions.md` is **not** read by VS Code
    and vice versa.
14. **Trust model for plugins is different.** Plugin MCP servers
    are **implicitly trusted** at install time and **do not** show
    the workspace MCP trust prompt. Workspace and user `mcp.json`
    servers do show a trust prompt on first start.
15. **Settings Sync.** User-level prompts, instructions, and skills
    can be synced across devices via Settings Sync (toggle "Prompts
    and Instructions" in `Settings Sync: Configure`). Workspace
    files are not synced — they live with the repo.
16. **`.devcontainer.json` MCP path.** It is
    `customizations.vscode.mcp.servers` (not
    `customizations.vscode.mcpConfig`). VS Code will copy those
    servers into the remote `mcp.json` at container build time.
17. **Workspace trust is the first security boundary.** Even with
    auto-approve rules defined, tools do not run on untrusted
    workspaces without explicit user opt-in. This is by design and
    is **not** configurable.
18. **Default MCP type.** In VS Code's `mcp.json`, omitting `type`
    defaults to `stdio`. To run an HTTP server you **must** set
    `"type": "http"`.
19. **Model name format.** In `model:` fields, the qualified form is
    `Model Name (vendor)`, e.g. `GPT-5 (copilot)`, `Claude Sonnet
    4.5 (copilot)`. Bare names (`GPT-5`) are also accepted and
    resolve via the current model picker.
20. **`argument-hint` is purely cosmetic.** It is shown as greyed-
    out text in the chat input; it is **not** parsed; the prompt
    body is what actually runs.

## Renderer notes (planned)

`agent-manager`'s Copilot renderer should:

1. **Default to writing into `.github/`** — the single source of
   truth for almost every check-in-able Copilot customisation. Fall
   back to `.vscode/mcp.json` only for VS Code-specific MCP config
   not yet supported in `.github/copilot/mcp.json`.
2. **Implement a separate model per file type.** There is no
   canonical schema; each file (`.agent.md`, `.prompt.md`,
   `.instructions.md`, `SKILL.md`, `mcp.json`, `plugin.json`) has its
   own ad-hoc JSON/YAML schema.
3. **YAML frontmatter is parsed leniently.** Preserve unknown keys
   on round-trip; only enforce the documented schema on emit.
4. **`tools` lists are the union of three things:** built-in tool
   names, tool-set names, and `<server>/<tool>` references. Preserve
   `<server>/*` shorthands (all of an MCP server) and `[]` (no
   tools, distinct from "all tools").
5. **`mcp.json` is the only file with non-trivial top-level
   structure** (`servers` + `inputs` + `sandbox`). Preserve all
   three blocks; the `inputs` block is critical for portable
   secrets.
6. **MCP transport type detection:** branch on `command` vs `url` if
   `type` is omitted.
7. **Path resolution.** Several fields support VS Code's variable
   substitution: `${workspaceFolder}`, `${userHome}`, `${env:VAR}`,
   `${input:id}`. Leave these as-is rather than expanding at render
   time.
8. **Respect user customisations** of the discovery folders. A safe
   strategy: keep default entries that already include `.github/...`
   and only add new entries when the user opts in.
9. **Three places the same MCP server can be declared.** Pick the
   right slot based on the target front-end; don't duplicate.
10. **CLI global personal instructions:** write
    `~/.copilot/copilot-instructions.md` for the CLI; either write
    the same content to the VS Code user-data `instructions/`
    folder, or add an entry to `chat.instructionsFilesLocations`
    pointing at `~/.copilot/instructions/`.
11. **Handoff `agent:` value** is the agent's `name` (not its
    filename). Validate that referenced agent names exist.
12. **Hooks are cross-tool compatible but not cross-tool
    identical.** Treat hooks as VS Code-native and warn the user
    before writing a hook into `.claude/settings.json`.
13. **There is no explicit "permissions file".** Permission rules
    are scattered across `chat.tools.terminal.autoApprove`,
    `chat.tools.edits.autoApprove`, `chat.tools.urls.autoApprove`,
    `chat.tools.eligibleForAutoApproval`, the `tools:` list in
    `.agent.md`/`.prompt.md`, and `PreToolUse` hooks with
    `permissionDecision`. The renderer must update several
    settings.json keys.
14. **Plugin format is its own beast.** Don't try to write
    `.plugin/plugin.json` unless the user explicitly opts into the
    plugin distribution model.
15. **`infer` is dead, long live `user-invocable` +
    `disable-model-invocation`.** Emit the new keys.
16. **`excludeAgent` in `*.instructions.md`** is currently only
    honoured by the github.com cloud agent and code review. Emit it
    for forward compatibility; do not rely on it for VS Code
    targeting.
17. **Enforce filename rules per file type.** Force
    `*.prompt.md` / `*.agent.md` / `*.instructions.md` extensions;
    force `SKILL.md` (uppercase, exact) on skill manifests with
    matching parent directory name; force `mcp.json` filename for
    MCP config; restrict agent filenames to `[.a-zA-Z0-9_-]+`.
18. **Pick the right file set per target front-end** (github.com /
    VS Code / Copilot CLI). A renderer should query the target at
    render time and emit accordingly.

## Sources

- Adding repository custom instructions —
  <https://docs.github.com/en/copilot/customizing-copilot/adding-repository-custom-instructions-for-github-copilot>
- Copilot CLI custom instructions —
  <https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-custom-instructions>
- Copilot CLI custom agents —
  <https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/create-custom-agents-for-cli>
- Cloud agent custom agents —
  <https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/create-custom-agents>
- Custom agents configuration reference —
  <https://github.com/github/docs/blob/main/content/copilot/reference/custom-agents-configuration.md>
- CLI best practices —
  <https://github.com/github/docs/blob/main/content/copilot/how-tos/copilot-cli/cli-best-practices.md>
- `AGENTS.md` open standard — <https://github.com/agentsmd/agents.md>
- Copilot code review instructions —
  <https://github.blog/ai-and-ml/github-copilot/unlocking-the-full-power-of-copilot-code-review-master-your-instructions-files/>
- VS Code custom instructions —
  <https://code.visualstudio.com/docs/agent-customization/custom-instructions>
- VS Code prompt files —
  <https://code.visualstudio.com/docs/agent-customization/prompt-files>
- VS Code custom agents —
  <https://code.visualstudio.com/docs/agent-customization/custom-agents>
- VS Code Agent Skills — <https://code.visualstudio.com/docs/agent-customization/agent-skills>
- VS Code customization overview —
  <https://code.visualstudio.com/docs/agent-customization/overview>
- VS Code MCP servers —
  <https://code.visualstudio.com/docs/agent-customization/mcp-servers>
- VS Code MCP configuration reference —
  <https://code.visualstudio.com/docs/agents/reference/mcp-configuration>
- VS Code hooks —
  <https://code.visualstudio.com/docs/agent-customization/hooks>
- VS Code agent tools / permissions —
  <https://code.visualstudio.com/docs/agents/agent-tools>
- VS Code settings reference —
  <https://code.visualstudio.com/docs/agents/reference/copilot-settings>
- VS Code agent plugins —
  <https://code.visualstudio.com/docs/agent-customization/agent-plugins>
- Copilot CLI repo — <https://github.com/github/copilot-cli>
