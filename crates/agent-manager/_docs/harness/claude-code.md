# Claude Code

Stable id: `claude-code`
Display name: Claude Code
Vendor: Anthropic

## Quick reference

| Field          | Value                                                       |
|----------------|-------------------------------------------------------------|
| Stable id      | `claude-code`                                               |
| Display name   | Claude Code                                                 |
| Vendor         | Anthropic                                                   |
| Global root    | `~/.claude/` (and `~/.claude.json` for MCP / OAuth)         |
| Project root   | `./.claude/` (and `./.mcp.json` for project-scoped MCP)    |
| Config format  | JSON (settings), Markdown (memory / skills / agents)        |

## On-disk layout

### Global (`~/.claude/`)

```
~/.claude/
├── settings.json          # user settings (permissions, hooks, mcpServers, env, …)
├── CLAUDE.md              # always-on user memory
├── agents/<name>.md       # user-defined sub-agents
├── skills/<name>/SKILL.md # user-defined skills (Agent Skills open standard)
├── commands/<name>.md     # user-defined slash commands (alias of skills/)
└── plugins/               # installed plugins (managed by `claude plugin`)

~/.claude.json            # supplemental JSON: user-level mcpServers + OAuth tokens
~/.claude/statsig/        # telemetry cache (do not edit)
~/.claude/todos/          # per-session todo state
~/.claude/projects/<hash>/# per-project session transcripts (not managed)
```

### Project (`<project>/`)

```
<project>/
├── .mcp.json                          # opt-in project MCP registry
└── .claude/
    ├── settings.json                  # checked-in project settings
    ├── settings.local.json            # gitignored per-developer overrides
    ├── CLAUDE.md                      # project memory (also: ./CLAUDE.md)
    ├── rules/<name>.md                # path-scoped modular rules
    ├── agents/<name>.md               # project sub-agents
    ├── skills/<name>/SKILL.md         # project skills
    ├── commands/<name>.md             # project slash commands
    └── hooks/                         # conventional location for hook scripts
```

## Discovery precedence

Highest precedence first; later layers are merged on top of earlier ones.

1. **Managed** — system-installed `managed-settings.json`
   (macOS: `/Library/Application Support/ClaudeCode/managed-settings.json`,
   or MDM-delivered). Read-only for the user.
2. **Enterprise remote config** — when configured in the Claude Code admin
   console.
3. **User** — `~/.claude/settings.json`, `~/.claude/CLAUDE.md`,
   `~/.claude/agents/`, `~/.claude/skills/`, `~/.claude/commands/`,
   `~/.claude.json`.
4. **Project** — `.claude/settings.json`, the project's `./CLAUDE.md`
   (or `./.claude/CLAUDE.md`), `.claude/agents/`, `.claude/skills/`,
   `.claude/rules/`.
5. **Local** — `.claude/settings.local.json` (gitignored).

**Merge rules.** `permissions.allow` / `ask` / `deny` arrays **concatenate**
and dedupe across layers. Objects (`mcpServers`, `hooks`) merge by key;
later layers win on per-key conflicts. Scalar keys are replaced.

**CLAUDE.md walk.** From `cwd` upward through every parent directory,
stopping at the git work-tree root or `$HOME`. The first existing
`CLAUDE.md` (or `.claude/CLAUDE.md`) wins at each level. The user-level
`~/.claude/CLAUDE.md` is loaded last. Per-path rules in
`.claude/rules/*.md` are appended after the main memory.

## Feature matrix

| Feature        | Support | Where it lands                                                          |
|----------------|---------|-------------------------------------------------------------------------|
| Rules          | full    | `CLAUDE.md` (project + user) and `.claude/rules/*.md` (path-scoped)     |
| Skills         | full    | `skills/<id>/SKILL.md` (user + project)                                 |
| MCP            | full    | `mcpServers` in `settings.json`, `.mcp.json`, and `~/.claude.json`      |
| Agents         | full    | `agents/<id>.md` (user + project)                                       |
| Slash commands | full    | `commands/<id>.md` (user + project; alias of skills)                    |
| Permissions    | full    | `permissions.{allow,ask,deny}` + `sandbox` in `settings.json`           |

## Skills

### Locations

- User: `~/.claude/skills/<name>/SKILL.md`
- Project: `.claude/skills/<name>/SKILL.md`
- Plugin: `<plugin>/skills/<name>/SKILL.md`

### Format (Agent Skills open standard)

A skill is a **directory** with a `SKILL.md` (YAML frontmatter + Markdown
body) and optional supporting files (scripts, references, assets) in the
same directory.

```yaml
---
name: deploy
description: Deploy the app to staging or production.
allowed-tools: Bash(git push:*), Bash(kubectl apply:*)
model: sonnet
---

# Deploy skill

When the user asks to deploy, run the following checks first…
```

### Frontmatter keys

| Key            | Type            | Notes                                                                 |
|----------------|-----------------|-----------------------------------------------------------------------|
| `name`         | string          | Slash command id; lowercase, hyphens. Defaults to directory name.     |
| `description`  | string          | One-line summary; shown in `/skills` picker; drives auto-invocation.  |
| `allowed-tools`| string or list  | Same rule syntax as `permissions.allow`.                              |
| `model`        | string          | `sonnet` / `opus` / `haiku` / `inherit`.                              |
| `when_to_use`  | string (legacy) | Older alias for `description`.                                        |
| `license`      | string          | Open-standard metadata.                                               |
| `metadata`     | object          | Free-form; passed through verbatim.                                   |

### Invocation

- Manual: `/<skill-name>`.
- Automatic: the model matches `description` to the user's intent.
- From another skill: include the skill's resource files or invoke via
  `name: sub-skill`.

## Sub-agents

### Locations

- User: `~/.claude/agents/<name>.md`
- Project: `.claude/agents/<name>.md`
- Plugin: `<plugin>/agents/<name>.md`

### Format

Markdown file with YAML frontmatter; the body becomes the system prompt.

```yaml
---
name: code-reviewer
description: Reviews PRs and suggests improvements
tools: Read, Grep, Glob, Bash(git diff:*)
model: opus
permissionMode: acceptEdits
skills:
  - code-style
---

You are a senior reviewer. Inspect changes for correctness, style, and tests.
```

### Frontmatter keys

| Key              | Type   | Notes                                                                  |
|------------------|--------|------------------------------------------------------------------------|
| `name`           | string | Sub-agent id; default = filename minus `.md`.                         |
| `description`    | string | **Required.** Used by the main agent for delegation.                   |
| `tools`          | list   | Allowlist of tool names. Omit to inherit all.                         |
| `model`          | string | `sonnet` / `opus` / `haiku` / `inherit`.                               |
| `permissionMode` | string | `default` / `acceptEdits` / `bypassPermissions` / `plan` / `delegate`. |
| `skills`         | list   | Skill names to pre-load into the sub-agent's context.                 |
| `systemPrompt`   | string | Optional override; otherwise the Markdown body is the prompt.          |

User and project agent directories are both scanned; identifiers must be
unique across layers (project wins on collision).

## MCP servers

### Locations

- User: `~/.claude.json` → `mcpServers` (and legacy `~/.claude/.mcp.json`).
- Project: `.mcp.json` at the project root.
- Project: `.claude/settings.json` → `mcpServers`.
- Plugin: `<plugin>/.mcp.json` or `plugin.json` → `mcpServers`.

### Transport variants

- `stdio` (default if `command` is set): spawns a subprocess.
- `sse`: server-sent events, requires `url`, optional `headers`.
- `http`: streamable HTTP, requires `url`, optional `headers`.

### Example (`.mcp.json`)

```json
{
  "mcpServers": {
    "github": {
      "type": "http",
      "url": "https://api.githubcopilot.com/mcp/"
    },
    "playwright": {
      "command": "npx",
      "args": ["-y", "@playwright/mcp@latest"],
      "env": { "DEBUG": "pw:browser" }
    }
  }
}
```

### Per-server fields

`type`, `command`, `args`, `env`, `cwd`, `url`, `headers`, `disabled`,
plus OAuth-specific fields (`clientId`, `clientSecret`, `redirectUri`).

### Project opt-in

Project `.mcp.json` servers are **not** loaded by default. Whitelist
them via `.claude/settings.json`:

```json
{
  "enableAllProjectMcpServers": true,
  "enabledMcpjsonServers": ["github"],
  "disabledMcpjsonServers": ["playwright"]
}
```

`enableAllProjectMcpServers` opts in everything; `enabledMcpjsonServers`
whitelists; `disabledMcpjsonServers` always wins.

## Slash commands

### Built-in

`/clear`, `/compact`, `/help`, `/init`, `/login`, `/logout`, `/mcp`,
`/memory`, `/model`, `/permissions`, `/plan`, `/review`, `/status`,
`/vim`, `/cost`, `/doctor`, `/terminal-setup`, `/agents`, `/skills`,
and others. Full list in the [Commands reference].

### Custom

- Locations: `~/.claude/commands/<name>.md` and `.claude/commands/<name>.md`.
- Format: same as skills (YAML frontmatter + Markdown body).
- Extra frontmatter: `argument-hint` (string shown after the command
  name) and `disable-model-invocation` (boolean).
- Invocation: `/<name> <args>`. Trailing `\` continues the prompt onto
  the next line.

### Custom commands vs. skills

`commands/<name>.md` is treated as `skills/<name>/SKILL.md` (a one-shot
prompt). The flat layout still works; prefer the skills-directory layout
for new work.

## Authentication

Claude Code supports six auth methods, declared in `settings.json` under
`env.ANTHROPIC_AUTH_TOKEN` / `env.ANTHROPIC_API_KEY` or under
`primaryApiKey`, or selected at the CLI with `claude --model`. Each method
has its own credential storage and its own precedence rules.

### Anthropic API (default)

The default auth method. Credentials are read in this order:

1. `ANTHROPIC_API_KEY` environment variable.
2. `env.ANTHROPIC_API_KEY` in any settings file (project > user > managed).
3. `authToken` in `.claude.json` (legacy; new code should use `apiKey`).
4. macOS: Keychain entry created by `claude setup-token`.
5. Linux/Windows: `~/.claude/credentials.json` (unencrypted file).

`claude setup-token` pastes a long-lived console token into the
OS-specific store; `claude auth login` runs the OAuth flow.

### Claude Pro / Max (subscription OAuth)

OAuth sign-in via browser. Token storage:

- macOS: Keychain (`Claude Code-credentials`).
- Linux: `~/.claude/credentials.json` (mode `0600`).
- Windows: Credential Manager.

The token is per-account; `claude auth logout` clears it. The active
account appears in the UI footer and in `claude auth status`. Multiple
accounts are not natively supported — sign out, then sign back in.

### Anthropic via Amazon Bedrock

Set `env.ANTHROPIC_BEDROCK_BASE_URL` and use the
`claude --model us.anthropic.claude-sonnet-4-...` naming. AWS credentials
come from the standard chain: env vars, `~/.aws/credentials`, IAM role.
No Anthropic key is required; Bedrock is reached via the AWS SDK, so
`aws sso login` / `aws configure` must be run first.

Region defaults to `us-east-1`; override with
`env.CLAUDE_CODE_USE_BEDROCK=1` plus `env.AWS_REGION`.

### Anthropic via Google Vertex AI

Set `env.CLAUDE_CODE_USE_VERTEX=1`, `env.ANTHROPIC_VERTEX_PROJECT_ID`,
and `env.CLOUD_ML_REGION` (e.g. `us-east5`). Authentication uses
Application Default Credentials (`gcloud auth application-default login`).
Model IDs are Vertex-style: `claude-sonnet-4@20250514`.

### Anthropic via Microsoft Foundry

Set `env.ANTHROPIC_FOUNDRY_BASE_URL`,
`env.ANTHROPIC_FOUNDRY_API_KEY`, and `env.ANTHROPIC_FOUNDRY_RESOURCE`.
`apiKeyHelper` can be used to fetch a rotating token from a sidecar.

### Custom proxy / LLM gateway

Set `env.ANTHROPIC_BASE_URL` and `env.ANTHROPIC_AUTH_TOKEN` to a
LiteLLM, OpenRouter, or self-hosted gateway. The harness will send the
same `/v1/messages` request shape; the gateway is responsible for
translating to whatever upstream model is configured. This is the
escape hatch used by `agent-manager` to point Claude Code at a custom
local model.

### apiKeyHelper

The `apiKeyHelper` setting (in any settings file) is a shell command
whose **stdout** is used as the API key. Useful for rotating tokens,
reading from a secret store, or fetching short-lived credentials
(Foundry, Bedrock with `GetSessionToken`). Returned value is cached
per-process; restart to refresh.

### Multiple accounts

There is no first-class multi-account switcher. To use a different
account, log out with `claude auth logout` and log back in with
`claude auth login` (or set the env var). The Keychain/credentials
file is single-tenant.

For machine-managed multi-account setups (e.g. CI matrices), set
`ANTHROPIC_API_KEY` per-job and never call `claude auth login` from
CI. Use a different `HOME` per account to keep Keychain entries
isolated.

### Precedence summary

Highest to lowest:

1. CLI flag (`--model`, `--api-key`).
2. `env.*` in `.claude/settings.local.json` (project-local, gitignored).
3. `env.*` in `.claude/settings.json` (project-committed).
4. `env.*` in `~/.claude/settings.json` (user-global).
5. `env.*` in the managed settings (admin-pushed).
6. OS-specific token store (Keychain, Credential Manager,
   `~/.claude/credentials.json`).
7. `apiKeyHelper` output (only when the key is otherwise unset).

Managed settings **override** env vars, not the other way around —
the admin's value wins for the security-sensitive keys (`apiKeyHelper`,
`permissions.defaultMode`).

### Headless / CI

Use `ANTHROPIC_API_KEY` in the CI secret store. Do **not** run
`claude auth login` from a CI runner: the OAuth flow needs a browser
and writes to the local Keychain, which the runner will not have
on next build. For non-interactive use, `--print` + a piped prompt
plus `ANTHROPIC_API_KEY` is the supported pattern.

### Troubleshooting

- `Invalid API key` → `ANTHROPIC_API_KEY` is set but wrong; check
  with `claude auth status`.
- `401 from bedrock` → AWS creds expired; rerun `aws sso login`.
- `claude auth login` hangs → behind a corporate proxy; set
  `HTTPS_PROXY` and retry.
- `apiKeyHelper` returns the same stale value → process is cached;
  restart Claude Code.

### Credential capture & reuse (agent-manager)

> How `am account capture` / `am account login` snapshot and replay this
> harness's subscription login into an ephemeral run. This records file
> **structure and non-secret metadata only** — token values are copied opaquely,
> never parsed into `am`'s account store.

- **Bundle files (the credential snapshot):**
  - `~/.claude/.credentials.json` — **required**; the OAuth token blob (single
    top-level key `claudeAiOauth`). *(Doc historically calls this
    `credentials.json`; the real file name has a leading dot — trust disk.)*
  - `~/.claude.json` — *optional*; carries the `oauthAccount` identity block
    (email/org/plan) and `userID`. Capture it for metadata; the harness re-fetches
    most of it from the API on next launch.
- **Relocation lever:** `HOME` moves the whole tree (`~/.claude/` +
  `~/.claude.json`) — this is the reliable isolation lever `am` uses.
  `CLAUDE_CONFIG_DIR` relocates `.claude/` (and therefore `.credentials.json`)
  but **not** `~/.claude.json`, so it is insufficient alone for a full snapshot.
- **Force file storage (skip keychain):** no documented config knob (unlike
  Codex). Claude Code falls back to the plaintext `.credentials.json` when the OS
  keychain is unreachable — running `login` inside the isol8/iter8 sandbox (no
  Keychain access) reliably forces the file. Verify post-login that
  `$HOME/.claude/.credentials.json` exists.
- **Default backend / observed:** macOS Keychain service `Claude Code-credentials`
  per doc; **observed file-based on this machine** (no Keychain entry) — trust disk.
- **Login command (fresh-auth-into-temp):** `HOME=/tmp/x claude auth login`
  (browser OAuth). No device-code flow is documented; for CI prefer an
  `ANTHROPIC_API_KEY` reference account over an OAuth snapshot.
- **Extractable metadata (non-secret):**

  | field | source | identifies |
  |---|---|---|
  | `subscriptionType` | `.credentials.json → claudeAiOauth.subscriptionType` | plan tier (e.g. `pro`) |
  | `rateLimitTier` | `.credentials.json → claudeAiOauth.rateLimitTier` | rate-limit bucket |
  | `scopes` | `.credentials.json → claudeAiOauth.scopes` | OAuth scopes → auth type = subscription OAuth |
  | `expiresAt` / `refreshTokenExpiresAt` | `.credentials.json → claudeAiOauth.*` | token expiry (epoch ms) |
  | `emailAddress` | `~/.claude.json → oauthAccount.emailAddress` | account email *(identifying — store hashed/redacted)* |
  | `organizationName` / `organizationType` | `~/.claude.json → oauthAccount.*` | org name / plan class (e.g. `claude_pro`) |
  | `billingType` | `~/.claude.json → oauthAccount.billingType` | billing (e.g. `stripe_subscription`) |

- **Do not copy:** `projects/`, `history.jsonl`, `sessions/`, `session-env/`,
  `shell-snapshots/`, `tasks/`, `telemetry/`, `cache/`, `backups/`,
  `file-history/` — session/telemetry/machine-bound state. `~/.claude.json`'s
  `machineID` is machine-bound; let the harness regenerate it.

## Permissions

### Location

The `permissions` key in any of: `~/.claude/settings.json`,
`.claude/settings.json`, `.claude/settings.local.json`, the managed
settings file.

### Format

```json
{
  "permissions": {
    "defaultMode": "acceptEdits",
    "allow": ["Read", "Bash(npm:*)", "Bash(git status)"],
    "ask":   ["Bash(curl *)", "Write(./secrets/**)"],
    "deny":  ["Read(./.env)", "Read(./secrets/**)", "Bash(rm -rf *)", "WebFetch"],
    "disableSandbox": false,
    "sandbox": {
      "filesystem": {
        "allowWrite": ["./tmp/**"],
        "denyWrite":  ["./secrets/**"]
      },
      "network": {
        "allowUnixSockets": ["/var/run/docker.sock"],
        "allowLocalBinding": true
      }
    },
    "additionalDirectories": ["../shared-lib"]
  }
}
```

### Rule syntax

- `Tool` — match the tool by name (`Read`, `Write`, `Edit`, `Bash`,
  `WebFetch`, `WebSearch`, `Glob`, `Grep`, `Agent`, `NotebookEdit`).
- `Tool(spec)` — match the tool call against a glob:
  - `Bash(npm run build:*)`
  - `Bash(git commit -m *)` — trailing `*` = "any continuation"
  - `Read(./.env)`, `Write(./src/**)`, `WebFetch(domain:example.com)`

### Evaluation order

`deny` → `ask` → `allow`. First match wins. Default (no rule matches)
follows `defaultMode` (`default` = ask, `acceptEdits` = auto-approve
edits, `bypassPermissions` = auto-approve all, `plan` = no execution).

`additionalDirectories` extends the writable set beyond CWD for tools
that respect it.

## Policies / Rules / Memory

### CLAUDE.md (long-form memory)

Loaded as additional system instructions. In load order:

1. `~/.claude/CLAUDE.md` (user memory — always loaded).
2. `./CLAUDE.md` at the git work-tree root.
3. `./.claude/CLAUDE.md` (alternative at the repo root).
4. `CLAUDE.md` files in any parent directory between `cwd` and the
   work-tree root.
5. `CLAUDE.md` files in subdirectories of the work-tree (monorepo style).

In-session: `/memory` opens the active memory files in `$EDITOR`.

### Modular rules (`.claude/rules/`)

Each file may use frontmatter:

```yaml
---
paths:
  - "src/api/**"
  - "tests/api/**"
---

# API conventions
All API endpoints must validate input via zod.
```

Rules with `paths` are included only when the agent is working under a
matching glob; rules without `paths` are always loaded.

## Orchestration / headless invocation

### Non-interactive launch

Argv skeleton:

```
claude -p --output-format stream-json --input-format stream-json --verbose \
  --strict-mcp-config --permission-mode bypassPermissions \
  --disallowedTools AskUserQuestion \
  [--model <id>] [--effort <level>] [--max-turns <n>] \
  [--append-system-prompt <text>] [--resume <session-id>] [--mcp-config <path>]
```

- `-p` = print/non-interactive; `--output-format stream-json` emits machine-readable events; `--input-format stream-json` means the prompt is delivered as a JSON line on stdin (not argv).
- `--permission-mode bypassPermissions` + `--disallowedTools AskUserQuestion` keep the run unattended (no approval prompts, no interactive questions).
- Child env hygiene: strip `CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_EXECPATH`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_SSE_PORT` so a nested invocation doesn't inherit the parent session.

### Output stream protocol

Newline-delimited JSON (NDJSON), one object per line on stdout. Prompt delivery is a single NDJSON line written to stdin first:

```json
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<prompt>"}]}}
```

Event shapes emitted on stdout:

```json
{"type":"system","subtype":"init","session_id":"..."}
{"type":"assistant","message":{"content":[{"type":"text","text":"..."},{"type":"thinking","thinking":"..."},{"type":"tool_use","id":"...","name":"...","input":{}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"...","content":[]}]}}
{"type":"result","result":"success","is_error":false,"usage":{},"modelUsage":{"<model-id>":{"input_tokens":0}}}
{"type":"log","log":{"level":"info","message":"..."}}
{"type":"control_request","request_id":"...","request":{"type":"tool_use","tool_use":{"id":"...","name":"...","input":{}}}}
```

Note: token usage is best read from the per-model `modelUsage` map in the `result` event, falling back to the top-level `usage`.

### Model & reasoning at launch

- Model: `--model <id>`. (Cross-reference Authentication.)
- Reasoning effort: `--effort <level>`, values `low | medium | high | xhigh | max`, discoverable from `claude --help`. Per-model allow-list: Opus supports all five (incl. `xhigh`); Sonnet supports `low|medium|high|max` (no `xhigh`); Haiku supports `low|medium|high`. Default `medium`.

### MCP at launch

- `--mcp-config <path>` points at a JSON file (`{"mcpServers":{...}}`) written just before launch; the file is the complete authoritative set for the run.
- `--strict-mcp-config` suppresses every ambient/inherited MCP server, so only the `--mcp-config` file's servers are loaded. (Cross-reference the MCP servers section for the per-server schema.)

### Skills at launch

A coordinator materialises skills into `<workdir>/.claude/skills/<name>/SKILL.md` before launch (the project skills path). Always-on context is written into `<workdir>/CLAUDE.md`, ideally inside a managed marker block so user-authored content is preserved. (Cross-reference Skills and Policies/Rules/Memory.)

### Tool approval in headless mode

With `--permission-mode bypassPermissions`, Claude Code still emits a `control_request` on stdout before each tool call and waits for a `control_response` on stdin. The coordinator answers:

```json
{"type":"control_response","response":{"subtype":"success","request_id":"...","response":{"behavior":"allow","updatedInput":{}}}}
```

`updatedInput` may rewrite the tool input before execution — e.g. forcing `run_in_background: false` so no orphaned background tool survives the parent process. A `tool_result` carrying `status:"async_launched"` signals a still-running background tool.

### Process lifecycle

- Framing: prompt in on stdin (NDJSON), events out on stdout (NDJSON), diagnostics on stderr.
- Cancellation: close stdin, then close the stdout reader to unblock the scanner; allow ~10s for the process to drain before killing.
- Minimum version: the stream-JSON input/output contract is stable from **Claude Code ≥ 2.0.0**.

### Model discovery & selection (agent-manager)

> How `am claude-code --list-models` enumerates models and `am claude-code --model <id>`
> selects one. Facts verified against the installed binary (v2.1.206) on 2026-07-10.

- **Discover (list models):** No dedicated list/JSON command exists. Best sources, both text-only: (1) `claude --help` — the `--model` flag's description enumerates live alias examples (`fable`, `opus`, `sonnet`) and one full-id example (`claude-fable-5`); (2) the interactive-only `/model` picker inside a live TUI session, which prints the complete current set with one-line descriptions and marks the account's current default. `/model` is not scriptable/headless — it requires a real TTY and human keypresses. Needs network/auth: `--help` text is static (bundled, no network); the `/model` picker reflects the logged-in account's plan/entitlements (it showed a promotional-access banner for one model), so treat it as auth-aware. Output: plain text in both cases, no JSON.
- **Select at launch (passthrough):** `--model <id>` on the normal interactive `claude` launch — verified to accept both a short alias (`sonnet`, `opus`, `fable`, `haiku`) and a full id (`claude-opus-4-8`, `claude-fable-5`). `am` should inject this flag directly. If omitted, Claude Code falls back to the `model` key in `~/.claude/settings.json` (persisted by the `/model` picker's "set as default" action), else its own built-in default.
- **Model id format:** two interchangeable forms — a short alias (`default`, `sonnet`, `opus`, `fable`, `haiku`) or a full id `claude-<family>-<version>[-<date>]`. Aliases resolve server-side to whichever concrete id is "latest" for that family/account.
- **Example ids (verified):** `sonnet` → `claude-sonnet-5`; `opus` → `claude-opus-4-8`; `fable` → `claude-fable-5`; every run's `modelUsage` also showed `claude-haiku-4-5-20251001` used internally as a background helper regardless of the selected `--model`.
- **Default model:** the `model` key in `~/.claude/settings.json` if present (verified on this machine: `"model": "opus"`, and an unset `--model` run indeed resolved to `claude-opus-4-8`); otherwise Claude Code's own default, shown by the `/model` picker as "Default (recommended) → Sonnet 5". `--model` on the CLI overrides both for that single invocation.

## Format quirks / gotchas

- `permissions.allow`/`ask`/`deny` arrays concatenate across layers;
  later layers add entries without removing earlier ones unless
  `strictPluginOnlyCustomization` is on.
- `mcpServers` and `hooks` objects merge by key — same name in user and
  project settings means the project definition wins for that key.
- Path globs in permissions use `*` (single segment) and `**` (any
  depth); Bash rules need a space before `*` to be a continuation
  (`git status` is exact, `git status *` matches any args).
- `settings.local.json` is gitignored by `claude /init`; secrets belong
  there, not in the checked-in `settings.json`.
- Sub-agent `tools` field is an allowlist. Omit it to inherit all; an
  empty list forbids all.
- Skill `allowed-tools` uses the same rule syntax as
  `permissions.allow` and stacks with the active session permissions.
- `~/.claude.json` is a separate file from `~/.claude/settings.json`;
  user-level MCP servers commonly live there, not in `settings.json`.
- `.mcp.json` at the project root is opt-in; you must list servers in
  `enabledMcpjsonServers` (or set `enableAllProjectMcpServers: true`).
- `strictPluginOnlyCustomization` (v2.1.82+): when on, only plugins
  contribute `skills`, `agents`, `hooks`, and `mcp`; user and project
  contributions are ignored for those surfaces.

## Renderer notes (planned)

`agent-manager`'s Claude Code renderer should:

1. Read the current `settings.json` (if any), preserve any unknown keys,
   update only `mcpServers`, `permissions`, and `hooks`.
2. Render each `[[rules]]` entry into a `## <id>` section of `CLAUDE.md`,
   keeping user-authored sections intact. Use a managed
   `<!-- agent-manager:begin --> … :end -->` block.
3. Copy each `[[skills]]` folder into `.claude/skills/<id>/` (project)
   and `~/.claude/skills/<id>/` (user), adding minimal frontmatter if
   missing. `commands/<id>.md` may be used as a synonym of the skills
   form.
4. Copy each `[[agents]]` file into `.claude/agents/<id>.md` (project)
   and `~/.claude/agents/<id>.md` (user), preserving YAML frontmatter.
5. Render `[[mcp]]` entries into `.mcp.json` (project) plus
   `settings.json` opt-in lists, or directly into
   `settings.json` → `mcpServers`.
6. Merge `permissions` into the `settings.json` `permissions` block;
   do not drop user permissions when emitting project settings.

The renderer **does not** own `~/.claude.json` (OAuth tokens) and
**does not** touch `~/.claude/projects/<hash>/` (session history).

## Sources

- Overview — <https://docs.claude.com/en/docs/claude-code/overview>
- Skills — <https://docs.claude.com/en/docs/claude-code/skills>
- Sub-agents — <https://docs.claude.com/en/docs/claude-code/sub-agents>
- MCP — <https://docs.claude.com/en/docs/claude-code/mcp>
- Slash commands — <https://docs.claude.com/en/docs/claude-code/slash-commands>
- Permissions — <https://docs.claude.com/en/docs/claude-code/permissions>
- Settings — <https://docs.claude.com/en/docs/claude-code/settings>
- Memory (CLAUDE.md) — <https://docs.claude.com/en/docs/claude-code/memory>
- Hooks — <https://docs.claude.com/en/docs/claude-code/hooks>
- Commands reference — <https://docs.claude.com/en/docs/claude-code/commands>
- Agent Skills open standard — <https://agentskills.io>

[Commands reference]: https://docs.claude.com/en/docs/claude-code/commands
