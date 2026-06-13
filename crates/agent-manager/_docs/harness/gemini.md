# Gemini CLI

Stable id: `gemini`
Display name: Gemini CLI
Vendor: Google

## Quick reference

| Field          | Value                                                                  |
|----------------|------------------------------------------------------------------------|
| Stable id      | `gemini`                                                               |
| Display name   | Gemini CLI                                                             |
| Vendor         | Google                                                                 |
| Global root    | `~/.gemini/` (or `$GEMINI_CLI_HOME`)                                   |
| Project root   | `<project>/.gemini/` and `<project>/GEMINI.md` (and any ancestor/descendant) |
| Config format  | JSON (`settings.json`), Markdown (`GEMINI.md`), TOML (commands / policies) |

## On-disk layout

### Global (`~/.gemini/`)

```
~/.gemini/
├── settings.json              # user-level settings (nested schema, v0.3.0+)
├── GEMINI.md                  # global hierarchical memory
├── commands/<name>.toml       # user-level custom slash commands (subdirs = namespaces)
├── agents/<name>.md           # user-level sub-agent definitions
├── skills/<name>/SKILL.md     # user-level Agent Skills (alias: ~/.agents/skills/)
├── extensions/<name>/         # installed extensions (each has gemini-extension.json)
├── policies/<file>.toml       # Policy Engine rules (user tier)
├── trustedFolders.json        # folder trust decisions
├── tmp/<project_hash>/        # checkpoints, shell_history, plans/
├── mcp-oauth-tokens.json      # OAuth tokens for remote MCP servers
├── cli-browser-profile/       # default Chrome profile for browser_agent
└── bin/litert/                # LiteRT-LM binary for the experimental Gemma router

# System-wide (admin):
#   Linux:   /etc/gemini-cli/system-defaults.json, /etc/gemini-cli/settings.json
#   macOS:   /Library/Application Support/GeminiCli/system-defaults.json, /Library/Application Support/GeminiCli/settings.json
#   Windows: C:\ProgramData\gemini-cli\system-defaults.json, C:\ProgramData\gemini-cli\settings.json
# Paths overridable via GEMINI_CLI_SYSTEM_DEFAULTS_PATH and GEMINI_CLI_SYSTEM_SETTINGS_PATH.
```

### Project (`<project>/`)

```
<project>/
├── GEMINI.md                          # project-level hierarchical memory (and any ancestor/descendant)
└── .gemini/
    ├── settings.json                  # project settings (overrides user; ignored in untrusted workspaces)
    ├── GEMINI.md                      # project-level memory
    ├── commands/<name>.toml           # project custom slash commands
    ├── agents/<name>.md               # project sub-agents
    ├── skills/<name>/SKILL.md         # workspace Agent Skills (alias: <project>/.agents/skills/)
    ├── sandbox.Dockerfile             # optional custom Docker image (built when BUILD_SANDBOX=1)
    ├── sandbox-macos-<profile>.sb     # optional custom macOS Seatbelt profile
    ├── .env                           # project env vars (always loaded)
    ├── .geminiignore                  # project-level ignore file (gitignore-style)
    ├── plans/                         # default per-extension plan-artifact location
    └── policies/*.toml                # workspace-tier policies (CURRENTLY DISABLED, issue #18186)
```

## Discovery precedence

For most file-based concepts the precedence is (lowest → highest):

1. Built-in (shipped with the CLI).
2. Extension-bundled copies.
3. User-global (`~/.gemini/...` or `~/.agents/...` alias).
4. Workspace (`<project>/.gemini/...` or `<project>/.agents/...` alias) —
   highest, overrides user and extension.

Per concept:

- **Settings** — strict layered merge: defaults → system defaults → user
  → project → system overrides → env vars → CLI flags. Project overrides
  user.
- **Custom commands** — project `<project>/.gemini/commands/` wins over
  `~/.gemini/commands/`. Extension commands are lowest; on collision
  with user/project, they get the `<ext>.<name>` dot prefix.
- **Sub-agents** — same as commands: project overrides user; extension
  lowest.
- **Skills** — built-in < extension < user < workspace. Within the
  same tier, `.agents/skills/` alias beats `.gemini/skills/`.
- **GEMINI.md** — global (`~/.gemini/GEMINI.md`) → workspace
  ancestors/descendants → JIT on tool file access. Concatenated in
  order, not overridden.
- **MCP servers** — `mcpServers` is *merged* across the four settings
  locations. Per `mcp-server.md`, an extension's MCP server is
  overridden by a same-named entry in your local `settings.json` (env
  merged, scalars replaced, `excludeTools` unioned, `includeTools`
  intersected).
- **Policy engine** — strict tier ordering: Default (1) < Extension
  (2) < Workspace (3, currently disabled) < User (4) < Admin (5).
  `final_priority = tier_base + (toml_priority / 1000)`.

The default memory filename `GEMINI.md` is overridable via
`context.fileName` (array, e.g. `["AGENTS.md", "CONTEXT.md",
"GEMINI.md"]`).

## Feature matrix

| Feature        | Support | Where it lands                                                  |
|----------------|---------|-----------------------------------------------------------------|
| Rules          | full    | `GEMINI.md` (global, project, ancestors, descendants, JIT)      |
| Skills         | full    | `skills/<id>/SKILL.md` (user + project) and inside extensions   |
| MCP            | full    | `mcpServers` in `settings.json` (4 layers: default/system/user/project) + `gemini-extension.json` |
| Agents         | full    | `agents/<id>.md` (user + project) and inside extensions         |
| Slash commands | full    | `commands/<id>.toml` (user + project) and inside extensions     |
| Permissions    | full    | `general.defaultApprovalMode` + `policies/<file>.toml` (Policy Engine) + `security.folderTrust.*` |
| Policies       | full    | `policies/*.toml` (Policy Engine, user + admin tiers; workspace tier disabled) |

## Skills

### Locations (lowest → highest)

1. Built-in skills (shipped with the CLI).
2. `~/.gemini/extensions/<name>/skills/<id>/SKILL.md` (extension-bundled).
3. `~/.gemini/skills/<id>/SKILL.md` (user) — also `~/.agents/skills/`.
4. `<project>/.gemini/skills/<id>/SKILL.md` (workspace) — also
   `<project>/.agents/skills/`.

### Format

A directory containing **`SKILL.md`** with YAML frontmatter and an
optional Markdown body. Optional subdirs: `scripts/`, `references/`,
`assets/`.

```markdown
---
name: security-audit
description: |
  Expertise in auditing code for security vulnerabilities. Use when the user
  asks to "check for security issues" or "audit" their changes.
---

# Security Auditor

You are an expert security researcher. When auditing code:

1. Look for common vulnerabilities (OWASP Top 10).
2. Check for hardcoded secrets or API keys.
3. Suggest remediation steps for any findings.
```

Required frontmatter: `name` (matches directory name), `description`
(specific, trigger-word rich). Optional: `license`, `compatibility`,
`metadata`.

### Lifecycle

Only `name` + `description` are loaded into context; the full body is
injected only when the model calls `activate_skill` and the user
consents. The skill's directory is added to the allowed file paths
after activation.

### CLI

`/skills list|enable|disable|reload|link`,
`gemini skills install <git-url>`,
`gemini skills list --all`,
`gemini skills uninstall <name> --scope user|workspace`. Skills can be
zipped into a `.skill` file via
`node scripts/package_skill.cjs <path>`.

### Extensions (related packaging concept)

Extensions are a **distribution unit** that can contain skills,
commands, MCP servers, hooks, themes, sub-agents, and a `GEMINI.md`.
A skill can also exist completely outside any extension.

Extension manifest (`<extension>/gemini-extension.json`):

```json
{
  "name": "my-extension",
  "version": "1.0.0",
  "description": "My awesome extension",
  "mcpServers": {
    "my-server": {
      "command": "node",
      "args": ["${extensionPath}/my-server.js"],
      "cwd": "${extensionPath}"
    }
  },
  "contextFileName": "GEMINI.md",
  "excludeTools": ["run_shell_command"],
  "settings": [
    {
      "name": "API Key",
      "description": "The API key for the service.",
      "envVar": "MY_SERVICE_API_KEY",
      "sensitive": true
    }
  ],
  "plan": { "directory": ".gemini/plans" }
}
```

Extension-bundled subdirs: `gemini-extension.json`, `GEMINI.md` (or
`contextFileName`), `commands/<group>/<name>.toml`, `skills/`,
`agents/`, `hooks/hooks.json`, `policies/*.toml`, `themes`, `.env`.

Variables: `${extensionPath}`, `${workspacePath}`, `${/}` (platform
path separator).

## Sub-agents

**Supported, preview feature.**

### Built-in sub-agents

| Name                   | Purpose                                                              |
|------------------------|----------------------------------------------------------------------|
| `codebase_investigator`| Deep codebase analysis, dependency mapping.                          |
| `cli_help`             | Expert on Gemini CLI itself.                                          |
| `generalist`           | General-purpose isolated loop for large multi-step subtasks.          |
| `browser_agent`        | (Experimental) Chrome automation via `chrome-devtools-mcp`; off by default. |

### Custom sub-agents

Locations (lowest → highest):

1. `~/.gemini/agents/*.md` (user).
2. `<project>/.gemini/agents/*.md` (workspace, shared via VCS).
3. Bundled inside an extension at `<extension>/agents/*.md` (lowest).

### Format

```markdown
---
name: security-auditor
description: Specialized in finding security vulnerabilities in code.
kind: local
tools:
  - read_file
  - grep_search
model: gemini-3-flash-preview
temperature: 0.2
max_turns: 10
---

You are a ruthless Security Auditor. Your job is to analyze code for
potential vulnerabilities. When you find one, explain it and suggest a
fix. Do not fix it yourself.
```

Frontmatter keys:

| Key            | Type    | Notes                                                                |
|----------------|---------|----------------------------------------------------------------------|
| `name`         | string  | **Required.** Slug: lowercase letters, numbers, hyphens, underscores. |
| `description`  | string  | **Required.** Visible to the main agent for delegation.               |
| `kind`         | string  | `local` (default) or `remote` (A2A).                                 |
| `tools`        | list    | Allowlist. Wildcards: `*`, `mcp_*`, `mcp_<server>_*`. Omit to inherit. |
| `mcpServers`   | object  | Inline MCP servers isolated to this agent.                            |
| `model`        | string  | Override the parent model.                                            |
| `temperature`  | number  | Default `1`.                                                          |
| `max_turns`    | number  | Default `30`.                                                         |
| `timeout_mins` | number  | Default `10`.                                                         |

### Isolation, recursion, policies

- Each sub-agent runs in its own context loop, cannot call other
  sub-agents (recursion protection).
- Sub-agents are addressable in `policy.toml` as virtual tool names:
  `[[rule]] toolName = "codebase_investigator" decision = "deny"`.
- Per-subagent policies: add `subagent = "name"` to a `[[rule]]` block.
- Global enable/disable: `experimental.enableAgents: false` disables
  the entire subsystem.

Remote sub-agents (A2A): `kind: remote` in the agent file; auth via a
separate flow (see `docs/core/remote-agents.md`).

## MCP servers

### Locations

- `~/.gemini/settings.json` → `mcpServers` (user).
- `<project>/.gemini/settings.json` → `mcpServers` (project).
- System default / system override settings files.
- Extension manifest: `<extension>/gemini-extension.json` →
  `mcpServers`.

Global filtering (applies to all sources) lives under the top-level
`mcp` object:

- `mcp.serverCommand` — string
- `mcp.allowed` — array of server names (allowlist; overrides everything)
- `mcp.excluded` — array of server names (denylist)

### `settings.json` shape

```jsonc
{
  "mcp": {
    "allowed":  ["my-trusted-server"],
    "excluded": ["experimental-server"]
  },
  "mcpServers": {
    "pythonTools": {
      "command": "python",
      "args":    ["-m", "my_mcp_server", "--port", "8080"],
      "cwd":     "./mcp-servers/python",
      "env":     { "DATABASE_URL": "$DB_CONNECTION_STRING", "API_KEY": "${EXTERNAL_API_KEY}" },
      "timeout": 15000,
      "trust":   false,
      "includeTools": ["safe_tool", "file_reader"],
      "excludeTools": ["dangerous_tool"]
    },
    "httpServer": {
      "httpUrl": "http://localhost:3000/mcp",
      "headers": { "Authorization": "Bearer your-api-token" },
      "timeout": 5000
    },
    "sseServer": {
      "url":                "https://api.example.com/sse",
      "headers":            { "X-Api-Key": "abc123" },
      "authProviderType":   "service_account_impersonation",
      "targetAudience":     "YOUR_IAP_CLIENT_ID.apps.googleusercontent.com",
      "targetServiceAccount": "your-sa@your-project.iam.gserviceaccount.com"
    }
  }
}
```

### Server properties

Required (one of): `command` (stdio), `url` (SSE), `httpUrl` (streamable
HTTP). If multiple are given, the precedence is `httpUrl` > `url` >
`command`.

Optional: `args`, `env` (supports `$VAR`, `${VAR}`, `%VAR%`), `cwd`,
`headers`, `timeout` (ms; default 600 000), `trust` (boolean — bypass
all confirmations for this server's tools), `includeTools` (allowlist,
intersection across merged sources), `excludeTools` (denylist, unioned;
**takes precedence over `includeTools`**), `description`, `oauth`,
`targetAudience`, `targetServiceAccount`.

### Naming and FQNs

Every discovered MCP tool gets an FQN: `mcp_<serverName>_<toolName>`.
**Avoid underscores in server names** (the policy parser splits on the
first `_` after `mcp_` and will misidentify the server).

### CLI management

`gemini mcp add [-s user|project] [-t stdio|sse|http] [--env KEY=val]
[--header ...] [--trust] [--include-tools ...] [--exclude-tools ...]
<name> <commandOrUrl> [args...]`, `gemini mcp list`, `gemini mcp remove
<name>`. Default scope: `project`. Tool inspection: `/mcp list`,
`/mcp auth <name>`, `/mcp enable/disable/reload/schema/desc`.

## Slash commands

Three prefixes are recognised in the prompt: `/` (slash meta-commands),
`@` (file injection), `!` (shell passthrough).

### Custom commands

Locations:

- User-global: `~/.gemini/commands/<name>.toml` (subdirs = namespaces).
- Project-local: `<project>/.gemini/commands/<name>.toml`.
- Extension-bundled: `~/.gemini/extensions/<name>/commands/<group>/<name>.toml`.
- MCP prompts: any MCP server exposing prompts via `prompts/list` —
  they become `/<prompt-name>` automatically.

Precedence: project > user > extension. On collision with user/project,
the extension command is renamed to `/<ext>.<name>`.

### File format — TOML

```toml
# ~/.gemini/commands/git/commit.toml   ->  /git:commit
description = "Generates a Git commit message based on staged changes."

prompt = """
Please generate a Conventional Commit message based on the following git diff:
```diff
!{git diff --staged}
```
"""
```

Subdirs become namespaces with `:`: `commands/git/commit.toml` →
`/git:commit`.

Fields:

- `prompt` (string, required) — the prompt body, with optional
  `{{args}}`, `!{...}`, `@{...}` placeholders.
- `description` (string, optional) — shown in `/help`; falls back to
  filename.

Substitution syntax:

- `{{args}}` — replaced by everything the user typed after the command
  name. Raw outside `!{...}`, shell-escaped inside `!{...}`.
- `!{shell command}` — runs the shell command, injects its output
  (after a security confirmation dialog).
- `@{path}` — injects file contents (multimodal for images / PDFs /
  audio) or a directory listing. Respects `.gitignore` and
  `.geminiignore`. Processed before `!{...}` and `{{args}}`.

If `prompt` contains no `{{args}}`, the user-supplied arguments are
appended after two newlines.

### Built-in slash commands (catalogue)

`/about`, `/agents`, `/auth`, `/bug`, `/chat` (alias `/resume`),
`/clear`, `/commands`, `/compress`, `/copy`, `/directory` (`/dir`),
`/docs`, `/editor`, `/extensions`, `/help` (`/?`), `/hooks`, `/ide`,
`/init`, `/mcp`, `/memory`, `/model`, `/permissions`, `/plan`,
`/policies`, `/privacy`, `/quit` (`/exit`), `/restore`, `/rewind`,
`/resume`, `/settings`, `/shells` (`/bashes`), `/setup-github`,
`/skills`, `/stats`, `/terminal-setup`, `/theme`, `/tools`,
`/upgrade`, `/vim`.

## Authentication

Gemini CLI is the only harness in this set that supports **multiple
simultaneous auth methods, picked per project, plus a UI-driven
sign-out/sign-in flow for OAuth**. The active method is stored in
`settings.json` under `security.auth.selectedType` and shown in the
footer chip.

The supported methods are:

- `oauth-personal` — Google account OAuth (free tier, 60 req/min,
  1000 req/day).
- `oauth-enterprise` — Google Workspace / Code Assist Enterprise.
- `gemini-api-key` — Google AI Studio key.
- `vertex-ai` — Google Cloud Vertex AI.
- `cloud-shell` — Gemini in Cloud Shell (no setup needed; uses the
  shell's ADC).

The selection lives in `security.auth.selectedType`; the method
below it (in the same file) is where the credential actually goes.

### Gemini API key (Google AI Studio)

The simplest method. Free tier quotas apply.

- Env var (process): `GEMINI_API_KEY` **or** `GOOGLE_API_KEY`.
- Project file: `GEMINI_API_KEY=...` in `.env` at the project root
  (loaded by Gemini CLI's dotenv loader).
- User file: `~/.gemini/.env` (same key, applies to all projects).
- Settings file: `security.auth.selectedType = "gemini-api-key"` plus
  the key in `security.auth.apiKey` (discouraged — use the env vars
  or `.env` files instead).

The CLI does **not** accept `--api-key` as a flag. There is no
`gemini auth login` for the API key path; the only way to set it
is via env, `.env`, or `settings.json`. The settings-file value is
not redacted in the rendered output — keep it out of version control.

### Google account OAuth (free tier)

`gemini auth` (or `/auth` inside the REPL) opens a browser, the user
approves, the resulting token is stored in `~/.gemini/oauth_creds.json`
(plaintext, mode `0600`). Refresh happens automatically; the file's
`expiry_date` field is the source of truth for the next refresh.

The `security.auth.selectedType = "oauth-personal"` setting just
declares that this method is allowed; the actual sign-in is
interactive.

`/auth signout` (or `gemini auth signout`) clears the cached token.

### Google Workspace / Code Assist Enterprise

For Workspace admins who have enrolled the org in Code Assist
Enterprise. Same OAuth flow as the personal method, but the consent
screen shows your org's app and the resulting account has the
org's quota and the org's data-governance policy applied.

- Settings: `security.auth.selectedType = "oauth-enterprise"`.
- Sign in: `gemini auth` (or `/auth`), pick "Enterprise".
- Optional: `GOOGLE_CLOUD_PROJECT` env var to pin the GCP project
  that owns the Code Assist entitlement.

### Vertex AI (project-scoped)

- `gcloud auth application-default login` to set up ADC.
- Settings:
  ```json
  {
    "security": {
      "auth": {
        "selectedType": "vertex-ai",
        "useVertex": true,
        "project": "my-gcp-project",
        "location": "us-central1"
      }
    }
  }
  ```
- `GOOGLE_GENAI_USE_VERTEXAI=1` env var is an alternative toggle
  when the rest of the config is the AI-Studio-shaped default.
- The model IDs are Vertex-style (`gemini-2.5-pro`, etc.); the CLI
  does not add the `publishers/google/models/` prefix for you.

### Vertex Express (the third Vertex path)

A separate, lighter Vertex code path that the CLI exposes for users
who have a Google Cloud project but do not want to set ADC up.
Selected with `security.auth.selectedType = "vertex-express"` plus
the project id and a short-lived token. The token is fetched by the
CLI itself, not by `gcloud`.

### Multiple keys per project

If the same project needs more than one Gemini API key (e.g. a CI
service account and a dev workstation), the cleanest pattern is:

- Project `.env` carries the CI key.
- `~/.gemini/.env` carries the developer key.
- The CLI's dotenv loader reads project first, then user, so the
  project value wins when both exist.

To force a specific key in a single shell, override at launch:
`GEMINI_API_KEY=sk-... gemini` — the env at launch time takes
precedence over `.env`.

### Multiple accounts via `/auth`

`/auth` lists every OAuth account that has previously signed in on
this machine; the user picks one with arrow keys. Tokens are kept
in `oauth_creds.json` keyed by email. To remove a stale account,
delete the matching block from `oauth_creds.json` (the CLI does not
expose a "forget account" command).

### `.env` precedence

The CLI loads dotenv files in this order, **later wins**:

1. Project root: `<cwd>/.env`.
2. Project ancestor walk: walks up the directory tree to the first
   ancestor with no `.geminiignore` (or to `/`).
3. User global: `~/.gemini/.env`.

For `GEMINI_API_KEY` and `GOOGLE_API_KEY`, the env-var-at-launch
value is the highest precedence — `GEMINI_API_KEY=x gemini` beats
any `.env` file. This is the canonical override path.

### Launch-time env override

The CLI re-reads `GEMINI_API_KEY` and `GOOGLE_API_KEY` on every
prompt, not just at startup. This means a shell script can
swap keys mid-session:

```bash
export GEMINI_API_KEY="sk-dev"   # current shell
gemini                           # picks up the dev key
```

This is also the path used by the agent-manager TUI to switch
profiles without restarting the CLI.

### Headless / CI

- API key path: `GEMINI_API_KEY` in the CI secret store, exported
  before the `gemini` invocation. Do **not** commit `.env` with a
  real key; `.env` is not gitignored by default — the project must
  add it.
- OAuth path: device-code flow is **not** supported by Gemini CLI
  (unlike Codex). For CI, use a service account key + Vertex AI
  instead.
- The flag `--prompt` (or the positional arg) is the non-interactive
  prompt. `--output-format json` plus `--yolo` (auto-approve
  everything) is the standard CI combo.

### Troubleshooting

- `Error: API key not valid` → env var name is wrong (must be
  `GEMINI_API_KEY` or `GOOGLE_API_KEY`, case-sensitive) or the key
  is from the wrong project.
- `selectedType` warning in `settings.json` → the field is the
  **v0.3.0+** schema; legacy `selectedAuthType` is silently
  ignored. Update the file.
- `oauth_creds.json` is missing → run `/auth` to re-sign-in.
- `quota exceeded` for `oauth-personal` → switch to
  `gemini-api-key` with a paid AI Studio key, or move to Vertex.
- `--api-key foo` was rejected → no such flag; the CLI only reads
  from env / `.env` / `settings.json`.

## Permissions

Gemini CLI has **three** independent layers: approval mode, the Policy
Engine, and Folder Trust. Plus a separate Sandbox configuration.

### 1. Approval mode (lightweight)

Set via `--approval-mode <default|auto_edit|plan|yolo>` (or `--yolo`),
or `general.defaultApprovalMode` in `settings.json`, or `/permissions`
and `/settings`.

- `default` — prompt for every write tool.
- `auto_edit` — auto-approve edit tools (`replace`, `write_file`),
  prompt for others.
- `plan` — read-only mode; toggled by `/plan`. Plan artifacts go to
  `general.plan.directory` (default: system temp) or
  `<extension>/.gemini/plans/`.
- `yolo` — auto-approve everything; **CLI-only**, can be disabled
  enterprise-side via `security.disableYoloMode: true` or
  `admin.secureModeEnabled: true`.

### 2. Policy Engine (TOML, fine-grained — canonical permission system)

| Tier       | Path                                                                                |
|------------|-------------------------------------------------------------------------------------|
| Default    | (built-in, immutable)                                                                |
| Extension  | `<extension>/policies/*.toml`                                                        |
| Workspace  | `<project>/.gemini/policies/*.toml` (currently disabled, issue #18186)               |
| User       | `~/.gemini/policies/*.toml`                                                          |
| Admin (Linux)    | `/etc/gemini-cli/policies/`                                                  |
| Admin (macOS)    | `/Library/Application Support/GeminiCli/policies/`                          |
| Admin (Windows)  | `C:\ProgramData\gemini-cli\policies\`                                      |
| Supplemental     | `--admin-policy <path>` flag or `adminPolicyPaths` / `policyPaths` in `settings.json` |

```toml
# ~/.gemini/policies/git.toml
[[rule]]
toolName      = "run_shell_command"
commandPrefix = "git"
decision      = "ask_user"
priority      = 100

[[rule]]
mcpName  = "my-custom-server"
toolName = "search"
decision = "allow"
priority = 200

[[rule]]
subagent     = "pr-creator"
toolName     = "run_shell_command"
commandPrefix = "git push"
decision     = "allow"
priority     = 150
denyMessage  = "Deletion is permanent"

[[rule]]
toolName = "codebase_investigator"
decision = "deny"
priority = 500
deny_message = "Deep codebase analysis is restricted for this session."
```

Priority formula: `final_priority = tier_base + (toml_priority / 1000)`
where `tier_base` ∈ {Default=1, Extension=2, Workspace=3, User=4,
Admin=5}. Higher wins.

Special keys:

- `mcpName` (string) — preferred over FQN wildcards; combines with
  `toolName` to form `mcp_<mcpName>_<toolName>`.
- `commandPrefix` (string|string[]) / `commandRegex` — sugar for
  `toolName="run_shell_command"`.
- `argsPattern` (regex) — matched against stable JSON of tool args.
- `modes` (array) — `default` | `autoEdit` | `plan` | `yolo`.
- `interactive` (bool) — restrict to interactive or headless sessions.
- `denyMessage` (string) — surfaced to the model.
- `allowRedirection` (bool) — allow `>`, `>>`, `<`, `<<`, `<<<` in
  matched shell commands.

**Persistence of "Always allow":** when the user picks "Allow for all
future sessions", the engine records a rule whose `modes` include the
current mode and all *more permissive* ones (e.g. approving in
`default` propagates to `autoEdit` and `yolo`, but **not** to `plan`).

Built-in default policies: read-only tools allowed; write tools
default to `ask_user`; `yolo` adds a high-priority allow-all; remote
agent delegation defaults to `ask_user`; local sub-agents are checked
individually.

### 3. Folder Trust

- Enable: `security.folderTrust.enabled: true` in `settings.json`
  (off by default).
- Storage: `~/.gemini/trustedFolders.json` (overridable via
  `GEMINI_CLI_TRUSTED_FOLDERS_PATH`).
- Trust dialog: *Trust folder* / *Trust parent folder* / *Don't trust*.
- **Untrusted workspace restrictions:** ignores project `settings.json`,
  ignores project `.env`, no extension install / uninstall / update,
  prompts on every tool, no auto memory loading, **MCP servers do not
  connect**, custom commands are not loaded.
- CI bypass: `--skip-trust` flag or `GEMINI_CLI_TRUST_WORKSPACE=true`.
- IDE precedence: IDE trust signal > `trustedFolders.json`.

### Sandbox

Three ways (precedence order): CLI flag `gemini -s` / `--sandbox`, env
`GEMINI_SANDBOX=true|docker|podman|sandbox-exec|runsc|lxc`, or
`tools.sandbox: true | "docker" | "podman" | "lxc" | "sandbox-exec" |
{ "command": "docker", "image": "..." }`.

- `tools.sandboxAllowedPaths: ["..."]` — bind-mount additional paths
  (also `SANDBOX_MOUNTS` env var, `from:to:opts`).
- `tools.sandboxNetworkAccess: false` (default off) — allow network in
  the sandbox.
- `security.toolSandboxing: false` (default) — opt into per-tool
  sandboxing instead of full-process.

Backends: macOS Seatbelt (`sandbox-exec`) with profiles
`permissive-open` / `permissive-proxied` / `restrictive-open` /
`restrictive-proxied` / `strict-open` / `strict-proxied`; Docker /
Podman (default image `ghcr.io/google/gemini-cli:latest`; custom image
via `tools.sandbox.image`, `GEMINI_SANDBOX_IMAGE`, or `BUILD_SANDBOX=1`
with a `<project>/.gemini/sandbox.Dockerfile`); Windows native; gVisor
/`runsc`; LXC / LXD (experimental).

Sandbox expansion: when a sandboxed command fails, the CLI pops a
"Sandbox Expansion Request" allowing the user to add specific paths or
network access for that single run.

## Policies / Rules / Memory

### `GEMINI.md` files

- **Default name:** `GEMINI.md` (overridable via `context.fileName`).
- **Global:** `~/.gemini/GEMINI.md`.
- **Workspace:** `<project>/GEMINI.md` (plus any ancestor up to the
  project root, plus descendant directories that contain their own
  `GEMINI.md`).
- **JIT:** when a tool accesses a file or directory, the CLI scans that
  directory and its ancestors up to a trusted root for `GEMINI.md`,
  and loads it on demand.
- **Format:** plain Markdown. No frontmatter required.
- **Imports:** Memory Import Processor supports `@./path.md` and
  `@/abs/path.md`. Processed before the surrounding content.

### Hierarchical loading rules

1. Global `~/.gemini/GEMINI.md` (always loaded if present).
2. Workspace ancestor + descendant `GEMINI.md` files (capped by
   `context.discoveryMaxDirs`, default 200).
3. JIT `GEMINI.md` on tool access, up to a trusted root.

Contents are **concatenated in order** and sent with every prompt; they
do not override each other.

### `/memory` command

`show` (print the full merged context), `refresh`/`reload` (force
re-scan), `list` (print which `GEMINI.md` paths are currently in scope).

### System prompt override

- `GEMINI_SYSTEM_MD=true` (uses `./.gemini/system.md`) or
  `GEMINI_SYSTEM_MD=/path/to/file.md` to fully replace the built-in
  system prompt.
- `GEMINI_WRITE_SYSTEM_MD=true` once to dump the built-in prompt for
  review.

### Auto memory

`experimental.autoMemory: true` (default off) auto-extracts memory
patches and skills from past sessions in the background, writing them
as unified-diff `.patch` files under `<projectMemoryDir>/.inbox/<kind>/`
for human review via `/memory inbox`. Nothing is applied automatically.

## Format quirks / gotchas

- **Settings schema versioning.** v0.3.0 introduced a *nested* schema
  (`general.vimMode`, `ui.theme`, `tools.sandbox`, `mcpServers.*`,
  `security.folderTrust.enabled`, etc.). The flat names
  (`selectedAuthType`, `coreTools`, `excludeTools`, `theme`, `sandbox`)
  still appear in older blog posts / third-party tools but are
  deprecated:

  | Flat (deprecated)        | Nested (current)                          |
  |--------------------------|-------------------------------------------|
  | `selectedAuthType`       | `security.auth.selectedType`              |
  | `coreTools`              | `tools.core` (allowlist) / `tools.allowed` |
  | `excludeTools`           | `tools.exclude` (legacy) or policy rule    |
  | `theme`                  | `ui.theme`                                 |
  | `sandbox`                | `tools.sandbox` (boolean or string)        |

  JSON schema: `https://raw.githubusercontent.com/google-gemini/gemini-cli/main/schemas/settings.schema.json`.

- **Environment-variable expansion** in `settings.json`: `$VAR`,
  `${VAR}`, `${VAR:-default}`.

- **Workspace-tier policies are broken** (issue #18186). Use User or
  Admin tiers.

- **MCP server naming** — must not contain underscores; the policy
  parser splits `mcp_<server>_<tool>` on the first underscore after
  `mcp_` and silently misroutes rules.

- **MCP stdio servers** only report "Connected" in `gemini mcp list`
  if the current folder is trusted (`gemini trust`).

- **Folder trust** disables project `settings.json` and project `.env`,
  blocks extension install / uninstall, blocks MCP connection, blocks
  custom command loading — drastically more aggressive than Claude
  Code's permission model.

- **Custom commands** are not reloaded automatically — must run
  `/commands reload` (or restart) after editing a `.toml` file.

- **Skills** are not reloaded automatically — must run `/skills reload`
  (or restart).

- **`GEMINI_API_KEY`, `GOOGLE_API_KEY`** are auto-redacted from MCP
  subprocess env; explicitly declared `env` blocks bypass this.

- **`GEMINI_SYSTEM_MD=true` ≠ `GEMINI_WRITE_SYSTEM_MD=true`**: the
  first replaces the system prompt at runtime; the second writes the
  current built-in prompt to disk for editing.

- **Telemetry env var** set to `true` *enables* the corresponding
  setting (e.g. `GEMINI_TELEMETRY_ENABLED=true` overrides
  `telemetry.enabled`). Any other value disables it.

- **Pre-installed agents vary by version**; `browser_agent` requires
  `agents.overrides.browser_agent.enabled: true` and Chrome 144+.

- **Conflict resolution is consistent**: project > user > extension.
  Extension commands get the `<ext>.<name>` dot prefix on collision.

- **`GEMINI.md` is renamed by `context.fileName`** (array of names in
  priority order).

## Renderer notes (planned)

`agent-manager`'s Gemini CLI renderer should:

1. **Write the right file in the right scope.**
   - `~/.gemini/settings.json` (user) and/or
     `<project>/.gemini/settings.json` (project). JSON, **nested**
     schema. Project wins on collision. Project `settings.json` is
     ignored entirely in untrusted workspaces.
   - `~/.gemini/GEMINI.md` (user) and/or `<project>/GEMINI.md`
     (project, plus any ancestor/descendant).
   - `~/.gemini/commands/<name>.toml` and/or
     `<project>/.gemini/commands/<group>/<name>.toml` — TOML.
     Subdirs become namespaces with `:`.
   - `~/.gemini/agents/<name>.md` and/or
     `<project>/.gemini/agents/<name>.md` — Markdown with YAML
     frontmatter.
   - `~/.gemini/skills/<skill-name>/SKILL.md` and/or
     `<project>/.gemini/skills/<skill-name>/SKILL.md` (alias
     `~/.agents/skills/`, `<project>/.agents/skills/`).
   - `~/.gemini/extensions/<name>/gemini-extension.json` (plus
     optional `commands/`, `skills/`, `agents/`, `policies/`,
     `hooks/`, `themes`, `GEMINI.md`).
   - `~/.gemini/policies/<file>.toml` (Policy Engine, user tier only;
     workspace tier is currently broken).
2. **Manage JSON carefully.** Use
   `schemas/settings.schema.json` for validation. Avoid the flat-key
   names (`selectedAuthType`, `coreTools`, etc.) — use the nested
   equivalents (`security.auth.selectedType`, `tools.core`,
   `tools.allowed`).
3. **Generate TOML `prompt` fields** that may include shell-escaped
   `!{...}` blocks. Keep `{{args}}` in the prompt unless the renderer
   is performing argument substitution itself.
4. **Generate `gemini-extension.json` manifests** with `name`, `version`,
   optional `description`, optional `mcpServers`, optional
   `contextFileName`, optional `excludeTools`, optional `settings[]`
   (with `sensitive: true` for keychain storage), optional `themes[]`,
   optional `plan.directory`. Use `${extensionPath}` and `${/}`
   variables for portable paths.
5. **Generate `SKILL.md` frontmatter** with `name` (matches directory
   name) and `description` (the trigger). Description is the *only*
   metadata loaded into context until activation — make it specific.
6. **Generate sub-agent frontmatter** with `name`, `description`,
   optional `kind`, `tools[]` (with `*`/`mcp_*` wildcards), optional
   `mcpServers{}`, `model`, `temperature`, `max_turns`,
   `timeout_mins`. Body becomes the system prompt.
7. **Policy files are TOML, not JSON.** `[[rule]]` blocks with
   `toolName`, `mcpName`, `subagent`, `argsPattern` / `commandPrefix` /
   `commandRegex`, `decision` (`allow` | `deny` | `ask_user`),
   `priority` (0–999), `modes`, `denyMessage`, `allowRedirection`. Tier
   is determined by *location*, not by a `tier` field.
8. **Be aware of folder trust.** A sync tool writing to
   `<project>/.gemini/settings.json` has no effect in an untrusted
   workspace. Either have the user `gemini trust` first, or write only
   to `~/.gemini/`.
9. **MCP server naming**: never use underscores. Use `my-server`, not
   `my_server`.
10. **MCP server merging** is per-property (env merged, scalars
    replaced, `excludeTools` unioned, `includeTools` intersected). To
    "veto" a tool from an extension-bundled server, add it to
    `excludeTools` in your own `mcpServers.<name>` block.
11. **Sandbox configuration** can be expressed as
    `tools.sandbox: "docker" | true | { command, image }`, or via env
    (`GEMINI_SANDBOX`) / CLI flag.
12. **Restart required** for many settings; custom commands and skills
    need `/commands reload` / `/skills reload` — but `settings.json`
    changes often do not.
13. **Prefer the CLI over hand-editing** for the global caches
    (`gemini extensions install/uninstall/update`,
    `gemini skills install/uninstall/link`, `gemini mcp add/list/remove`,
    `gemini trust`).

The renderer **does not** own `~/.gemini/tmp/<project_hash>/`
(checkpoints, shell history) and should not hand-edit it.

## Sources

- Repo — <https://github.com/google-gemini/gemini-cli>
- Docs index — <https://github.com/google-gemini/gemini-cli/blob/main/docs/index.md>
- Configuration reference —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md>
- Settings (UI catalogue) —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/settings.md>
- `GEMINI.md` —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/gemini-md.md>
- Skills —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/skills.md>
- Creating skills —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/creating-skills.md>
- Sub-agents —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/core/subagents.md>
- Extensions (overview) —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/extensions/index.md>
- Writing extensions —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/extensions/writing-extensions.md>
- Extensions reference —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/extensions/reference.md>
- Custom commands —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/custom-commands.md>
- Commands reference —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/commands.md>
- Sandbox —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/sandbox.md>
- Trusted folders —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/trusted-folders.md>
- Policy engine —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/policy-engine.md>
- MCP server tool —
  <https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md>
- Settings JSON schema —
  <https://raw.githubusercontent.com/google-gemini/gemini-cli/main/schemas/settings.schema.json>
