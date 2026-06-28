# Codex

Stable id: `codex`
Display name: Codex
Vendor: OpenAI

## Quick reference

| Field          | Value                                                                          |
|----------------|--------------------------------------------------------------------------------|
| Stable id      | `codex`                                                                        |
| Display name   | Codex                                                                          |
| Vendor         | OpenAI                                                                         |
| Global root    | `$CODEX_HOME` (default `~/.codex/`)                                            |
| Project root   | `<repo>/.codex/` (trusted projects only)                                       |
| Config format  | TOML (`config.toml`), Markdown (`AGENTS.md`), TOML (custom agents)             |

## On-disk layout

### Global (`~/.codex/`)

```
~/.codex/
├── config.toml                       # main user config (TOML)
├── <name>.config.toml                # per-profile config (selected with --profile)
├── auth.json                         # file-based credential cache
├── history.jsonl                     # session transcripts
├── log/                              # default log directory
├── hooks.json                        # optional legacy hooks file
├── agents/<name>.toml                # personal custom sub-agents
├── rules/*.rules                     # Starlark execpolicy files
├── AGENTS.md                         # global instruction memory
├── AGENTS.override.md                # optional override of AGENTS.md
└── .agents/skills/<name>/SKILL.md    # per-user agent skills

# System-wide (Unix only):
/etc/codex/config.toml                # system-tier config
/etc/codex/skills/                    # admin skills
/etc/codex/rules/                     # admin execpolicy
```

> **Note:** Codex user skills live under **`~/.agents/skills/`**, **not**
> `~/.codex/skills/`. This follows the open `agentskills.io` standard.

### Project (`<project>/.codex/`)

```
<project>/.codex/             # trusted projects only
├── config.toml               # project config (walked root→CWD, closest wins)
├── <name>.config.toml        # project-scoped profile files (rare)
├── hooks.json                # legacy hooks file
├── agents/<name>.toml        # project custom sub-agents
├── rules/*.rules             # project execpolicy
├── AGENTS.md                 # layered project instructions
└── AGENTS.override.md        # per-directory override
```

Project root detection defaults to "directory containing `.git`";
override with `project_root_markers = [".git", ".hg", ".sl"]` or `[]` to
disable walking.

## Discovery precedence

### Config-layer precedence (highest → lowest)

1. CLI flags and `--config key=value` overrides (dot notation).
2. Project config: every `.codex/config.toml` walked from project root
   → CWD, closest wins. **Trusted projects only.**
3. Profile file: `~/.codex/<name>.config.toml` when `--profile <name>`
   is passed.
4. User config: `~/.codex/config.toml`.
5. System config: `/etc/codex/config.toml` (Unix).
6. Built-in defaults.

> **Quirk:** as of Codex 0.134.0, legacy `[profiles.<name>]` tables
> inside `config.toml` are **no longer read**; the top-level
> `profile = "name"` selector is also removed. Use per-profile TOML
> files (`<name>.config.toml`).

### `AGENTS.md` discovery precedence (highest → lowest)

1. Global: in `$CODEX_HOME`, `AGENTS.override.md` if present, else
   `AGENTS.md`. Only the first non-empty file is used at this level.
2. Project: starting at the project root, walk down to CWD. At each
   directory, check in order: `AGENTS.override.md` → `AGENTS.md` →
   `project_doc_fallback_filenames` entries. **At most one file per
   directory.**
3. Merge order: root-down, joined with blank lines. Files closer to CWD
   appear later and effectively override earlier guidance.
4. Empty files are skipped. Combined size is capped at
   `project_doc_max_bytes` (default **32 KiB**).

### Custom-sub-agent naming precedence

If a user/built-in agent's `name` matches one in `~/.codex/agents/` or
`.codex/agents/`, the custom file wins. Built-ins: `default`, `worker`,
`explorer`.

### Project trust

Untrusted → the entire `.codex/` layer is skipped (config, hooks, rules,
custom sub-agents). User and system layers still load.

## Feature matrix

| Feature        | Support | Where it lands                                          |
|----------------|---------|---------------------------------------------------------|
| Rules          | full    | `AGENTS.md` (project + user) + `AGENTS.override.md`     |
| Skills         | full    | `.agents/skills/<id>/SKILL.md` (user + project + admin) |
| MCP            | full    | `[mcp_servers.<id>]` in `config.toml`                   |
| Agents         | full    | `agents/<id>.toml` (user + project)                     |
| Slash commands | partial | Built-in only (no user-defined slash command files)     |
| Permissions    | full    | `approval_policy` + `sandbox_mode` **or** `default_permissions` + `[permissions.*]` |
| Policies       | full    | `AGENTS.md` + `rules/*.rules` (execpolicy) + `developer_instructions` per agent |

## Skills

### Locations (scope matrix)

| Scope   | Path                                      | Use                                                |
|---------|-------------------------------------------|----------------------------------------------------|
| `REPO`  | `$CWD/.agents/skills`                     | Checked-in per-folder workflows                   |
| `REPO`  | Each ancestor dir's `.agents/skills` up to repo root | Shared area in a parent folder         |
| `REPO`  | `$REPO_ROOT/.agents/skills`               | Root skills, available to any subfolder            |
| `USER`  | `~/.agents/skills`                        | Per-user skills, all repos                         |
| `ADMIN` | `/etc/codex/skills`                       | Machine/container-wide                             |
| `SYSTEM`| Bundled with the Codex binary             | `skill-creator`, `plan`, etc.                      |

> Project trust does **not** gate skills (only `.codex/` layers).
> Symlinks are supported and followed.

### Format (`SKILL.md` frontmatter)

```markdown
---
name: skill-name
description: Explain exactly when this skill should and should not trigger.
---

Skill instructions for Codex to follow.
```

Required frontmatter: `name`, `description`. The description drives
implicit matching; make it specific and trigger-word rich.

### Discovery / invocation

- **Explicit:** `$skill-name` (or `/skills` in the CLI to pick).
- **Implicit:** Codex matches `description` against the user prompt and
  decides to load `SKILL.md`.

Progressive disclosure: only name + description + path are loaded into
context initially; the full `SKILL.md` is loaded on selection. Initial
budget is ~2% of the model context window (default 8 000 chars).

### Optional `agents/openai.yaml` (next to `SKILL.md`)

```yaml
interface:
  display_name: "Optional user-facing name"
  short_description: "Optional user-facing description"

policy:
  allow_implicit_invocation: false

dependencies:
  tools:
    - type: "mcp"
      value: "openaiDeveloperDocs"
      transport: "streamable_http"
      url: "https://developers.openai.com/mcp"
```

`allow_implicit_invocation: false` makes the skill only `$skill`-invokable.

### Disabling without deletion

```toml
# ~/.codex/config.toml
[[skills.config]]
path = "/path/to/skill/SKILL.md"
enabled = false
```

## Sub-agents

### Locations

- Built-in roles (always present, not files): `default`, `worker`,
  `explorer`. Spawn only when the user explicitly asks.
- Personal custom: `~/.codex/agents/<name>.toml`.
- Project custom: `<repo>/.codex/agents/<name>.toml` (trusted only).
- The `name` field is the source of truth; filename matching is a
  convention, not a requirement.

### Custom-agent TOML schema

Required:
- `name` (string)
- `description` (string — human-facing guidance for when to use)
- `developer_instructions` (string)

Optional (inherited from the parent session if omitted):
- `nickname_candidates` (array of strings)
- `model`
- `model_reasoning_effort`
- `sandbox_mode` (e.g. force `read-only` for an explorer)
- `mcp_servers` (re-declared per agent if needed)
- `[[skills.config]]` entries (enable/disable specific skills)
- Any other valid `config.toml` key — a custom-agent file is loaded
  as a **config layer** for spawned sessions.

### Example

```toml
# ~/.codex/agents/reviewer.toml
name = "reviewer"
description = "PR reviewer focused on correctness, security, and missing tests."
model = "gpt-5.4"
model_reasoning_effort = "high"
sandbox_mode = "read-only"
developer_instructions = """
Review code like an owner.
Prioritize correctness, security, behavior regressions, and missing test coverage.
"""
nickname_candidates = ["Atlas", "Delta", "Echo"]

[mcp_servers.openaiDeveloperDocs]
url = "https://developers.openai.com/mcp"
```

### Global keys under the parent `[agents]` table

| Key                              | Type   | Default | Purpose                                            |
|----------------------------------|--------|---------|----------------------------------------------------|
| `agents.max_threads`             | number | `6`     | Concurrent open agent-thread cap                   |
| `agents.max_depth`               | number | `1`     | Nesting depth (root = 0)                           |
| `agents.job_max_runtime_seconds` | number | `1800`  | Per-worker timeout for `spawn_agents_on_csv`       |

### Relationship to skills

Skills load into the active session's prompt; sub-agents spawn a new
session with its own `developer_instructions`, `model`, `sandbox_mode`,
MCP servers, and skill allowlist.

## MCP servers

### Location

`config.toml` — both `~/.codex/config.toml` and
`<repo>/.codex/config.toml` (trusted). **There is no separate `mcp.json`**
— MCP is just another table in the same TOML.

Top-level MCP keys:

- `mcp_oauth_callback_port` (int)
- `mcp_oauth_callback_url` (string)
- `mcp_oauth_credentials_store` (`auto` | `file` | `keyring`)

### Schema (`[mcp_servers.<id>]`)

**stdio (local process):**
- `command` (string, required) — launcher command
- `args` (array of strings)
- `env` (map<string,string>)
- `env_vars` (array of strings **or** `{ name, source }` records)
- `cwd` (string)
- `experimental_environment` (`local` | `remote`)

**streamable HTTP** (the only HTTP transport Codex documents; no
separate `sse` field):
- `url` (string, required)
- `bearer_token_env_var` (string)
- `http_headers` (map<string,string>)
- `env_http_headers` (map<string,string>)

**Common to both:**
- `startup_timeout_sec` (default `10`) / `startup_timeout_ms` (alias)
- `tool_timeout_sec` (default `60`)
- `enabled` (bool) — disable without removing
- `required` (bool) — fail startup if the server can't initialize
- `enabled_tools` / `disabled_tools` (arrays; `disabled_tools` is
  applied **after** `enabled_tools`)
- `default_tools_approval_mode` (`auto` | `prompt` | `approve`)
- `tools.<tool>.approval_mode` — per-tool override
- `oauth_resource` (string, RFC 8707), `scopes` (array)

### Examples

```toml
[mcp_servers.context7]
command = "npx"
args = ["-y", "@upstash/context7-mcp"]
env_vars = ["LOCAL_TOKEN"]

[mcp_servers.context7.env]
MY_ENV_VAR = "MY_ENV_VALUE"
```

```toml
[mcp_servers.figma]
url = "https://mcp.figma.com/mcp"
bearer_token_env_var = "FIGMA_OAUTH_TOKEN"
http_headers = { "X-Figma-Region" = "us-east-1" }
```

```toml
[mcp_servers.chrome_devtools]
url = "http://localhost:3000/mcp"
enabled_tools = ["open", "screenshot"]
disabled_tools = ["screenshot"]
default_tools_approval_mode = "prompt"
startup_timeout_sec = 20
tool_timeout_sec = 45
```

## Slash commands

**Codex does not currently expose user-defined slash command files.**
All slash commands are built-in and triggered from the CLI's `/` popup.

Built-in (full list, CLI):

`/permissions`, `/ide`, `/keymap`, `/vim`, `/sandbox-add-read-dir`
(Windows), `/agent`, `/apps`, `/plugins`, `/hooks`, `/clear`, `/compact`,
`/copy`, `/diff`, `/exit`, `/quit`, `/experimental`, `/approve`,
`/memories`, `/skills`, `/feedback`, `/init`, `/logout`, `/mcp`,
`/mention`, `/model`, `/fast`, `/plan`, `/goal`, `/personality`, `/ps`,
`/stop`, `/fork`, `/side`, `/btw`, `/raw`, `/resume`, `/new`, `/review`,
`/status`, `/debug-config`, `/statusline`, `/title`, `/theme`.

Selected semantics:

- `/init` — generates an `AGENTS.md` scaffold.
- `/review` — runs a working-tree review using `review_model` if set.
- `/permissions` — switches between presets (Auto, Read Only, custom
  profiles). Mutates the live session.
- `/approve` — retries one action the automatic reviewer denied.
- `/mcp` — lists MCP servers (`/mcp verbose` adds diagnostics).
- `/agent` — switches active agent thread.
- `/skills` — opens the skill picker.
- `/compact` — triggers context compaction.

Skills surface through `/skills` and trigger via `$skill-name`; that is
the closest Codex gets to user-defined slash commands.

## Authentication

Codex supports four first-class auth methods plus a fifth escape hatch
for custom endpoints. The active method is recorded in
`~/.codex/auth.json` (`auth_mode` field) and is what the CLI sends on
the next request. Switching methods overwrites that file.

### OpenAI API key

The default for direct API users.

- Env var: `OPENAI_API_KEY`.
- File: `~/.codex/auth.json` with `{"OPENAI_API_KEY": "sk-..."}`.
- File: project-level `auth.json` (per-folder, like `.codex/config.toml`).
- `codex login --api-key <key>` writes the key to `auth.json`.
- `codex logout` clears the entry.

### ChatGPT subscription OAuth (Plus / Pro / Team / Enterprise)

OAuth flow launched by `codex login` (or `codex login --device-code` for
headless). The CLI opens a browser, the user confirms, the resulting
access + refresh tokens are written to:

- macOS: Keychain (`Codex Auth`).
- Linux: `~/.codex/auth.json` (mode `0600`), with a refresh-token
  block keyed by `chatgpt_account_id`.
- Windows: Credential Manager.

`codex login status` reports the active account. `codex logout` clears
both the access and refresh token.

Refresh tokens are rotated silently on every successful API call; the
file's `last_refresh` timestamp is updated. Stale tokens (>30 days)
trigger a forced re-login.

### Multiple accounts / profiles

`codex login` only stores one account at a time. To switch, log out
and log back in. For per-project isolation, set
`CODEX_HOME=<separate-dir>` to keep the entire `~/.codex` (including
`auth.json`) in a project-scoped location.

For per-profile switching without logging out, the v0.134.0+ per-file
config pattern lets you point `[profiles.<name>].model_provider` at a
named provider block, and each provider can carry its own
`env_key` / `wire_api` — the active key then depends on the resolved
profile, not on `auth.json` directly.

### Azure OpenAI

`model_provider = "azure"` (in config.toml) plus these env vars:

- `AZURE_OPENAI_API_KEY`
- `AZURE_OPENAI_ENDPOINT` (e.g. `https://<resource>.openai.azure.com`)
- `AZURE_OPENAI_API_VERSION`
- `AZURE_OPENAI_DEPLOYMENT` (the model deployment name)

AAD/managed-identity is supported via the `azure_ad_token_provider`
helper script set as `model_providers.azure.azure_ad_token_provider`
in `config.toml`.

### OpenAI-compatible endpoint (OSS / custom)

`model_provider = "oss"` with `model_providers.oss` carrying:

- `base_url` (e.g. `http://localhost:11434/v1` for Ollama, or
  `https://api.openai.com/v1` for a self-hosted proxy).
- `env_key` (env var name, NOT a value — Codex reads the env at call
  time).
- `wire_api = "chat"` (or `"responses"` for the new endpoint).
- Optional `http_headers` for static headers.

For local models (Ollama, llama.cpp) use `env_key = "OPENAI_API_KEY"`
and a dummy value like `"ollama"`.

### Vertex AI / Bedrock

Codex itself does **not** support Bedrock/Vertex directly. Two
workarounds:

- **Bedrock via OSS provider**: set `base_url` to a LiteLLM proxy
  fronting Bedrock, with `env_key = "AWS_BEARER_TOKEN_BEDROCK"` for
  short-lived bearer tokens.
- **Vertex via OSS provider**: same pattern, LiteLLM in front of
  Vertex.

The CLI treats these as opaque OpenAI-shaped endpoints; cost / quota
info is not surfaced.

### Headless / CI (CI-friendly auth)

For CI, prefer **API key in the secret store** plus the
`codex exec --api-key "$OPENAI_API_KEY"` flag (or
`--api-key-file` pointing at a one-line file in the runner's secret
mount). Never run `codex login` from a CI runner; the browser flow
will hang waiting for a callback, and the resulting token in
`auth.json` will be tied to the runner's ephemeral filesystem.

For ChatGPT-OAuth in CI, use the **device-code flow**:
`codex login --device-code`. The CLI prints a URL and a code; the
user approves in a browser on any machine; the runner then proceeds.
This is the only OAuth pattern supported in headless environments.

The flag `--enable/disable auth.json lookup` controls whether the
CLI falls back to `auth.json` or insists on an env var. The default
is "env > auth.json > error".

### Precedence summary

Highest to lowest:

1. CLI flag (`--api-key`, `--api-key-file`).
2. Active `model_providers.<name>.env_key` env var (the named
   provider, resolved from the active profile).
3. `OPENAI_API_KEY` env var.
4. `auth.json` (`OPENAI_API_KEY` field).
5. `auth.json` (`chatgpt` block, refresh-token path).
6. `codex login` device-code flow (interactive, only on `login`).

Note: `auth.json` is **read but not refreshed** during a normal
session — the file is the bootstrap, and in-memory tokens are
maintained by the running process.

### Token storage and `cli_auth_credentials_store`

Codex can be told to use the OS keychain for the `OPENAI_API_KEY`
block in `auth.json`. Set in `config.toml`:

```toml
cli_auth_credentials_store = "keyring"   # default on macOS
# alternatives: "file" (plaintext), "auto"
```

`"keyring"` writes to:

- macOS: Keychain (`Codex Auth` service).
- Linux: Secret Service (GNOME Keyring / KWallet via D-Bus).
- Windows: Credential Manager.

`"file"` keeps the key in `auth.json` (mode `0600`). `"auto"` picks
the best available store per platform.

### Troubleshooting

- `401 invalid_api_key` → key in `auth.json` is stale; rerun
  `codex login --api-key`.
- `Refresh token expired` → user re-auth required; the CLI will
  prompt on next non-interactive call.
- `Azure endpoint returns 404` → deployment name doesn't match the
  model you're trying to use; check `AZURE_OPENAI_DEPLOYMENT`.
- `Ollama returns 400 model not found` → `base_url` should end in
  `/v1` and the model name should be the local tag, not an OpenAI
  model name.
- `auth.json has no key` → set `CODEX_HOME` to a directory you own,
  or run `codex login` to bootstrap it.

## Permissions

Codex has **two parallel systems**. Choose one per run; they do not
compose.

### System A: legacy `sandbox_mode` + `approval_policy`

`approval_policy` accepts:

- `"untrusted"` — prompt for everything outside the safe read set.
- `"on-request"` — prompt for actions that need to leave the sandbox,
  use the network, etc. (default; `on-failure` is deprecated).
- `"never"` — never prompt (still respects sandbox).
- A granular object: `approval_policy = { granular = {
  sandbox_approval, rules, mcp_elicitations, request_permissions,
  skill_approval } }` — each `bool` toggles whether that prompt
  category can surface (`true`) or fails closed (`false`).

Related: `approvals_reviewer = "user" | "auto_review"`; `allow_login_shell`
(bool, default `true`); `web_search` (`"cached"` | `"live"` | `"disabled"`);
`personality` (`"friendly"` | `"pragmatic"` | `"none"`); reasoning-effort
keys; `[sandbox_workspace_write]` sub-table; `[windows]` sub-table
(Windows-native only).

### System B: beta permission profiles

`default_permissions = "<name>"` selects the active profile. Built-ins:
`:read-only`, `:workspace`, `:danger-full-access`. Custom names go under
`[permissions.<name>]`, with `extends` for inheritance from another
named profile or from `:read-only` / `:workspace` (not
`:danger-full-access`).

```toml
default_permissions = "project-edit"

[permissions.project-edit]
description = "Project editing with OpenAI API access."
extends = ":workspace"

[permissions.project-edit.workspace_roots]
"~/code/app"       = true
"~/code/shared-lib" = true

[permissions.project-edit.filesystem]
":minimal"            = "read"
"~"                   = "deny"          # default-deny home
"~/Documents"         = "deny"
"~/Documents/codex"   = "write"          # carve-out

[permissions.project-edit.filesystem.":workspace_roots"]
"."             = "write"
".devcontainer" = "read"
"**/*.env"      = "deny"

[permissions.project-edit.network]
enabled            = true
allow_local_binding = false
domains = { "api.openai.com" = "allow", "tracking.example.com" = "deny" }
unix_sockets = { "/var/run/docker.sock" = "allow" }
```

Filesystem values: `read`, `write`, `deny` (precedence deny > write > read).
Network `domains` map: `exact`, `*.example.com` (subdomains),
`**example.com` (apex + subdomains), `*` (global allow-only). Deny
always wins.

Special path tokens: `:root`, `:minimal`, `:workspace_roots`, `:tmpdir`,
`:slash_tmp`, `/abs/path`, `~/path`.

### Sandbox modes

| Mode                | What it allows                                         | Network           |
|---------------------|--------------------------------------------------------|-------------------|
| `read-only`         | read everywhere; nothing written                       | off               |
| `workspace-write`   | writes inside workspace roots + temp dirs; `.git`, `.codex`, `.agents` inside writable roots are read-only | off by default; opt in via `[sandbox_workspace_write].network_access` |
| `danger-full-access`| no sandbox (use only in already-isolated environments) | unrestricted      |

OS-level enforcement: macOS Seatbelt via `sandbox-exec`; Linux/WSL2
`bubblewrap` + `seccomp` (Landlock as fallback); Windows native
(`elevated` strong, `unelevated` weaker).

### Rules (execpolicy) — orthogonal layer

`rules/*.rules` is a Starlark DSL that controls which commands can run
**outside the sandbox**:

```python
# ~/.codex/rules/default.rules
prefix_rule(
    pattern = ["gh", "pr", "view"],
    decision = "prompt",
    justification = "Viewing PRs is allowed with approval",
    match = ["gh pr view 7888"],
    not_match = ["gh pr --repo openai/codex view 7888"],
)
```

Fields: `pattern` (required), `decision` (`allow` | `prompt` |
`forbidden`; most restrictive wins), `justification` (string, surfaced
in prompts/rejections), `match` / `not_match` (inline unit tests).
Test with `codex execpolicy check`.

## Policies / Rules / Memory

Codex has three distinct concept-areas that map to "rules":

### 1. `AGENTS.md` memory (Markdown instructions)

Format: plain Markdown, no required frontmatter. Optional. Layered.
Truncated at `project_doc_max_bytes` (default 32 KiB).

Locations and precedence: see **Discovery precedence** above. Tunables in
`config.toml`:

```toml
project_doc_fallback_filenames = ["TEAM_GUIDE.md", ".agents.md"]
project_doc_max_bytes = 65536
```

### 2. `.rules` execpolicy (Starlark)

Locations: `~/.codex/rules/`, `<repo>/.codex/rules/`, `/etc/codex/rules/`.
When you allow a command in the TUI, Codex writes to
`~/.codex/rules/default.rules`.

### 3. `developer_instructions` (in custom-agent files)

The required `developer_instructions` string in `agents/<name>.toml` is
the per-agent system-prompt guidance.

## Orchestration / headless invocation

A coordinator drives Codex headlessly by speaking JSON-RPC 2.0 over stdio to the **app-server** sub-command.

### Non-interactive launch

- Launch: `codex app-server --listen stdio://` (requires Codex ≥ 0.100.0).
- The model is **not** a CLI flag — it is sent inside the RPC handshake.
- I/O is newline-framed JSON-RPC on stdin/stdout; one JSON object per line.
- Spawn in its own process group (`setpgrp` / `os/exec` `SysProcAttr.Setpgid`) so the entire tree can be killed as a unit on cancel.

### Output stream protocol

JSON-RPC 2.0 over stdio. Handshake sequence (client → server unless noted):

1. `initialize` request — params: `clientInfo`, `capabilities.experimentalApi` → server response.
2. `initialized` notification.
3. `thread/start` (new thread) or `thread/resume` (existing) — params include `model`, `cwd`, `developerInstructions`, `persistExtendedHistory`; response carries `thread.id`.
4. `thread/name/set` (optional).
5. `turn/start` — params `{ threadId, input: [{ type: "text", text: "…" }], effort? }`; response carries `turn.id`.

Codex emits one of **two notification dialects**; detect which on the first inbound notification and handle both:

- **Legacy** — `codex/event` notifications with a `msg.type` field: `task_started`, `agent_message`, `exec_command_begin` / `exec_command_end`, `patch_apply_begin` / `patch_apply_end`, `task_complete`, `turn_aborted`.
- **v2 / raw** — discrete method names: `turn/started`, `turn/completed`, `item/started` + `item/completed` (field `itemType` ∈ `commandExecution` | `fileChange` | `agentMessage`), `thread/status/changed` (`status.type == "idle"`), and `error` (`willRetry: false` = terminal).

Canonical category mapping:

| Category        | Legacy                          | v2 / raw                                          |
|-----------------|---------------------------------|---------------------------------------------------|
| Assistant text  | `agent_message`                 | `item/completed` (agentMessage)                   |
| Tool call       | `exec_command_begin`            | `item/started` (commandExecution)                 |
| Tool result     | `exec_command_end`              | `item/completed` (commandExecution)               |
| Completion      | `task_complete`                 | `turn/completed` or `thread/status/changed` idle  |

Token usage: `turn.usage` in the v2 `turn/completed` (keys `usage` / `token_usage` / `tokens`); fallback is scanning `~/.codex/sessions/YYYY/MM/DD/*.jsonl` for `token_count` events.

### Model & reasoning at launch

- **Model:** the `model` field inside `thread/start` / `thread/resume` params — not a CLI flag.
- **Reasoning effort:** `config.model_reasoning_effort` inside `thread/start` / `thread/resume`, or top-level `effort` inside `turn/start`. Values: `none | minimal | low | medium | high | xhigh`.
- Discoverable per-model allowed set and default: `codex debug models --bundled` (JSON; fields `supported_reasoning_levels`, `default_reasoning_level`). Available since Codex ≥ 0.131.0.

### MCP at launch

- Codex reads MCP from `[mcp_servers.<id>]` tables in `config.toml`.
- For an isolated run, point `CODEX_HOME` at a per-run directory and write a `config.toml` there.
- Wrap coordinator-managed entries in a comment-delimited block so hand-authored tables survive rewrites:

```toml
# BEGIN managed mcp_servers
[mcp_servers.my-tool]
command = "npx"
args = ["-y", "my-mcp-tool"]
# END managed mcp_servers
```

- To enforce only the managed set, strip inherited `[mcp_servers.*]` tables from the user's `~/.codex/config.toml` before the run. (Cross-reference the MCP servers section for the per-server schema.)

### Skills at launch

Copy skills into the per-run `$CODEX_HOME/.agents/skills/<name>/SKILL.md` (workspace skills take precedence over user-installed `~/.agents/skills/`). Always-on context goes into `AGENTS.md` in the working directory. (Cross-reference Skills and Policies / Rules / Memory.)

### Tool approval in headless mode

The app-server issues server→client approval requests; auto-accept all of them to stay unattended:

| Request method / type                   | Auto-accept response                                            |
|-----------------------------------------|-----------------------------------------------------------------|
| `item/commandExecution/requestApproval` / `execCommandApproval`  | `{ "decision": "accept" }`        |
| `item/fileChange/requestApproval` / `applyPatchApproval`         | `{ "decision": "accept" }`        |
| `item/permissions/requestApproval`      | grant `network` + `fileSystem`, scoped to `"turn"`             |
| `mcpServer/elicitation/request`         | `{ "action": "accept", "content": null }`                      |

### Process lifecycle

- **Framing:** newline-delimited JSON-RPC in both directions over stdio.
- **Cancellation:** close stdin to signal the app-server to stop → wait ~10 s for the reader to drain → `Wait` up to ~10 s more → if still alive, `SIGKILL` the entire process group (negative PID on Unix).
- **Minimum versions:** `app-server --listen stdio://` requires Codex ≥ 0.100.0; per-model reasoning discovery requires ≥ 0.131.0.

## Format quirks / gotchas

- **Profiles moved out of `[profiles.<name>]`** in 0.134.0 — use
  `<name>.config.toml` per-profile files.
- **Project config keys silently ignored in untrusted projects**:
  `openai_base_url`, `chatgpt_base_url`, `apps_mcp_product_sku`,
  `model_provider`, `model_providers`, `notify`, `profile`, `profiles`,
  `experimental_realtime_ws_base_url`, `otel`. Put them in
  `~/.codex/config.toml`.
- **Relative paths in project config** (e.g. `model_instructions_file`)
  resolve from the `.codex/` folder containing the `config.toml`, not
  from CWD.
- **Project root detection** is `.git` by default; override with
  `project_root_markers`, or `[]` to disable walking.
- **Empty `AGENTS.md` is silently ignored.**
- **`project_doc_max_bytes` is combined** across the whole chain, not
  per file. Default 32 KiB.
- **MCP `disabled_tools` is applied after `enabled_tools`.** Effective
  list = `enabled_tools ∩ ¬disabled_tools`.
- **Two parallel permission systems** are mutually exclusive. Pick one
  per run.
- **Custom agent files are config layers**, not a separate schema. They
  can override any `config.toml` key.
- **Agent name collisions:** a custom-agent `name` matching a built-in
  (e.g. `explorer`) wins.
- **Skills live under `.agents/skills/`, not `.codex/skills/`.**
- **`AGENTS.override.md` is per-directory**, not global.
- **Hooks:** both `hooks.json` and inline `[[hooks.<Event>]]` in
  `config.toml` are accepted; if a layer has both, Codex loads both and
  warns.
- **No OXM-style "rule files for slash commands"** — use
  `$skill-name` (a skill) for `/`-style invocation.
- **Web search** is controlled by top-level `web_search`; legacy
  `features.web_search*` booleans map to it.
- **`request_permissions`** is a real Codex tool — gate it via the
  granular approval policy.
- **Protected paths inside workspace-write** (don't write into these via
  sync): `<writable_root>/.git`, `<writable_root>/.codex`,
  `<writable_root>/.agents`.

## Renderer notes (planned)

`agent-manager`'s Codex renderer should:

1. Always write to **`config.toml`** for unified rules / MCP / settings:
   - User: `~/.codex/config.toml`.
   - Project: `<repo>/.codex/config.toml` (only effective when the
     project is trusted; warn if untrusted).
   - Project config **cannot** override `openai_base_url`, `model_provider`,
     `model_providers`, `notify`, `profile`, `profiles`, `otel`,
     `chatgpt_base_url`, `experimental_realtime_ws_base_url`,
     `apps_mcp_product_sku`.
2. **Profiles → one TOML file per profile**, not a `[profiles]` table.
3. **MCP servers → `[mcp_servers.<id>]` table.** Distinguish by fields:
   `command`/`args`/`env` = stdio; `url`/`bearer_token_env_var`/
   `http_headers` = streamable HTTP. Honor per-tool `enabled_tools` /
   `disabled_tools` / `approval_mode`.
4. **Skills → write directories of `SKILL.md`.** User: `~/.agents/skills/`.
   Project: place `.agents/skills/<id>/` at the desired scope (folder,
   ancestor, or root). Optional `agents/openai.yaml` for UI / policy /
   MCP dependencies. To disable a built-in or user skill without
   removal, add `[[skills.config]]` to `~/.codex/config.toml`.
5. **Custom sub-agents → `agents/<id>.toml`.** Required: `name`,
   `description`, `developer_instructions`. Use the `name` field as the
   identifier; built-ins (`default`, `worker`, `explorer`) can be
   overridden by matching `name`.
6. **Slash commands** are not user-defined files. To express a "slash
   command" in the unified config, create a **skill** and document it
   as `$skill-name` / `/skills`. There is no `commands/foo.md` location.
7. **Permissions → pick one system.** Legacy:
   `approval_policy` + `sandbox_mode` + `[sandbox_workspace_write]`.
   Beta: `default_permissions` + `[permissions.<name>]` with
   `filesystem` / `network` / `workspace_roots`. They do **not** compose.
8. **`AGENTS.md` memory → single Markdown body** at user (`~/.codex/AGENTS.md`
   or `~/.codex/AGENTS.override.md`) and project
   (`<repo>/AGENTS.md`, plus any per-subdir `<dir>/AGENTS.override.md`).
   Keep total ≤ `project_doc_max_bytes` (default 32 KiB) across the
   whole chain. Empty files are ignored.
9. **Rules / execpolicy → Starlark `.rules` files** in
   `~/.codex/rules/` or `<repo>/.codex/rules/`. Not the same as
   `AGENTS.md` — this is for shell-command decisions outside the
   sandbox.
10. **Hooks** → either `<layer>/hooks.json` or inline
    `[[hooks.<Event>]]` in `config.toml`. Use only one representation
    per layer. Events: `PreToolUse`, `PermissionRequest`, `PostToolUse`,
    `PreCompact`, `PostCompact`, `SessionStart`, `SubagentStart`,
    `SubagentStop`, `UserPromptSubmit`, `Stop`.
11. **Disabled without deletion:** `mcp_servers.<id>.enabled = false`
    (MCP), `[[skills.config]]` with `enabled = false` (skills),
    `features.<name> = false` (features).

The renderer **does not** own `/etc/codex/...` (system / managed) and
**does not** write into protected paths (`<writable_root>/.git`,
`<writable_root>/.codex`, `<writable_root>/.agents`) when the project
is in `workspace-write` mode.

## Sources

- Repo — <https://github.com/openai/codex>
- Docs hub — <https://developers.openai.com/codex>
- Config basics — <https://developers.openai.com/codex/config-basic>
- Config advanced — <https://developers.openai.com/codex/config-advanced>
- Config reference — <https://developers.openai.com/codex/config-reference>
- Config sample — <https://developers.openai.com/codex/config-sample>
- AGENTS.md — <https://developers.openai.com/codex/guides/agents-md>
- Skills — <https://developers.openai.com/codex/skills>
- Sub-agents — <https://developers.openai.com/codex/subagents>
- MCP — <https://developers.openai.com/codex/mcp>
- Permissions (beta profiles) — <https://developers.openai.com/codex/permissions>
- Rules (execpolicy) — <https://developers.openai.com/codex/rules>
- Approvals & security — <https://developers.openai.com/codex/agent-approvals-security>
- Slash commands — <https://developers.openai.com/codex/cli/slash-commands>
