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

### Tool-call timeout

**Documented default: 60 s**, from `tool_timeout_sec` above — the bound on one
`tools/call`, per server, alongside `startup_timeout_sec`'s documented 10 s on
`initialize`. `build_mcp_servers_block` (`src/harness/codex.rs`) writes neither
field, so every server `am` provisions runs at both defaults.

**Progress notifications: unknown.** Codex's configuration surface says nothing
about whether its MCP client sends `_meta.progressToken` on a `tools/call`, nor
whether a `notifications/progress` resets `tool_timeout_sec`. Only a live run
against a server that logs the request settles it.

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

### Credential capture & reuse (agent-manager)

None. `am` neither captures, copies nor roams this login: the harness keeps it in the account's
own config home, signed in there by `am account login` or the first run, and refreshes it itself
(`D194` in Ubiq's `_docs/tech/decisions.md`).

### Shared-home run (agent-manager, `D194`)

> How `Codex::provision_home` runs from a profile's persistent `CODEX_HOME` instead of a per-run
> copy. Both `codex` and `codex-acp` share a home; the differences for `codex-acp` are its own
> bullet below.

- **Home:** the profile's `CODEX_HOME`, holding `auth.json`, `sessions/` and anything the
  profile's user put there. Codex refreshes its own login; nothing is seeded or read back. With no
  profile (`ConfigStrategy::Native`) `CODEX_HOME` is not set and Codex runs from `~/.codex`.
- **Sign-in:** `CODEX_HOME=<home> codex login` (`Codex::login_home`). No `config.toml` is written
  first, so the store is Codex's default for the platform (see "Token storage" above).
- **Per-run settings by `-c key=value`,** ahead of any subcommand (`codex -c … app-server --listen
  stdio://` or `codex -c … [prompt]`), each value an inline TOML value: `model`,
  `model_reasoning_effort`, `sandbox_mode` + `approval_policy = "never"`, one
  `mcp_servers.<id>` inline table per server, and the run's instructions as
  `developer_instructions` (a config-layer key — see "Custom-agent TOML schema"). A `-c` layers
  over the home's `config.toml`, so a server listed there still loads; there is no strict-MCP
  switch. Nothing is written into `scratch`.
- **Per-profile only:** skills (`.agents/skills/<id>/`, MCP-as-skill pointers included) and
  `hooks.json` have no per-run route, so they are written into the home — each skill dir built
  beside it and renamed over the old one, `hooks.json` staged and renamed, under a lock. `RunSpec`
  does not mark which skills come from the profile, so two runs of one profile with different
  skills or hooks overwrite each other, and a skill dropped from the profile stays in the home. A
  native run drops skills and hooks with a warning. `AGENTS.md` and `config.toml` are never
  written.
- **`codex-acp`:** the same home, sign-in (`codex login`) and profile-owned skills and hooks. Its
  structured argv is `codex-acp -c … [passthrough_args…]` — the adapter's `main` parses Codex's own
  `CliConfigOverrides` (read from `zed-industries/codex-acp` `src/main.rs`, not run here) — with
  every `-c` above but `mcp_servers.*`. The MCP servers go in ACP `session/new` / `session/load`
  `mcpServers` instead (`Provisioned::mcp_servers`, sent by `AcpBridge::with_mcp_servers`);
  `codex-acp` advertises `mcpCapabilities.http` only, so an sse server is dropped with a warning,
  and it adds the servers to the home's own, as a `-c` does. A passthrough `codex-acp` pane is the
  real `codex`, composed exactly as above.
- **Transcripts:** rollouts land in `<home>/sessions/YYYY/MM/DD/*.jsonl`, but this document does
  not say how a file names its thread, so `session_transcripts` answers `None`.

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

### ACP mode (`codex-acp`)

`codex-acp` (npm package `@agentclientprotocol/codex-acp`, installed with
`npm install -g @agentclientprotocol/codex-acp` or `npx -y @agentclientprotocol/codex-acp`) starts
an **Agent Client Protocol** endpoint on its own stdio, speaking newline-delimited JSON-RPC 2.0,
and drives the same `codex` binary underneath — the same `$CODEX_HOME` relocation, `config.toml`,
skills and account/login handling as the native `codex` harness above; only the wire differs. `am`
drives it through the harness-neutral `AcpBridge` (`src/io/acp_client.rs`) rather than the
app-server protocol documented below — see [`../io-modes.md`](../io-modes.md).

Structured argv is exactly:

```
codex-acp [-c key=value ...] [passthrough_args...]
```

The `-c` overrides appear only on a shared-home run (see "Shared-home run" above); a fixed-dir run
passes none. Nothing else is on the command line: no `app-server --listen stdio://`, no
`-m`/`--model`, no resume flag. The prompt is a `session/prompt` request over the wire, a resume is
`session/load` against the id the previous run reported, and the model/reasoning effort reach the
run through `config.toml` under `CODEX_HOME` on a fixed dir, or by `-c` under a shared home. MCP
servers are in that `config.toml` on a fixed dir, where the bridge sends `mcpServers: []`; under a
shared home they travel in `session/new`'s `mcpServers`. Passthrough argv is unchanged by any of
this: `codex-acp` is not a TUI, so a pane still gets the real, interactive `codex`.

Verified against `@agentclientprotocol/codex-acp` 1.10.0: the bin is named `codex-acp`, and unlike
Claude's `claude-agent-acp` it answers `--version` directly (prints `<name> <version>`, exits 0),
so no version-probe override is needed.

### Output stream protocol

JSON-RPC 2.0 over stdio. Handshake sequence (client → server unless noted):

1. `initialize` request — params: `clientInfo`, `capabilities.experimentalApi` → server response.
2. `initialized` notification.
3. `thread/start` (new thread) or `thread/resume` (existing) — params include `model`, `cwd`, `developerInstructions`, `persistExtendedHistory`; response carries `thread.id`.
4. `thread/name/set` (optional).
5. `turn/start` — params `{ threadId, input: [{ type: "text", text: "…" }], effort? }`; response carries `turn.id`.

**The protocol's own schema is the source.** `codex app-server generate-json-schema --out <dir>`
and `generate-ts --out <dir>` emit it per installed version, and `just codex-schema-diff` compares
the method set against the snapshot committed at
`crates/agent-manager/tests/fixtures/codex-app-server/methods.txt`. Everything below is 0.161.0's.
Two frame fixtures sit beside it: `live-0.161.0-unauthenticated-turn.ndjson`, captured from a real
app-server on an empty `CODEX_HOME` (redacted host, installation and request ids), and
`schema-0.161.0-turn.ndjson`, a successful turn assembled from the generated types — no
authenticated turn has been captured yet.

Responses carry no `"jsonrpc"` member; route on `id` / `method`. `thread/start`'s response also
states `model`, `modelProvider`, `approvalPolicy`, `sandbox` (`{ type: readOnly | workspaceWrite |
dangerFullAccess | externalSandbox, … }`) and `reasoningEffort`.

Notifications are discrete methods (v2); every one about a thread carries `threadId`. The legacy
`codex/event` wrapper (`msg.type` ∈ `task_started`, `agent_message`, `exec_command_begin` /
`exec_command_end`, `task_complete`, `turn_aborted`) is not in 0.161.0's `ServerNotification` set
and is read only for older binaries.

- **Items** — `item/started` / `item/completed` carry `{ item, threadId, turnId }`; `item` is
  tagged **`type`**: `userMessage`, `agentMessage { text }`, `reasoning { summary[], content[] }`,
  `commandExecution { command, cwd, status, aggregatedOutput, exitCode, durationMs }`,
  `fileChange { changes: [{ path, kind, diff }], status }`, `mcpToolCall { server, tool,
  arguments, result, error }`, `dynamicToolCall`, `webSearch { query }`, `contextCompaction`,
  `collabAgentToolCall`, `subAgentActivity`, `plan`, and more. Status is `inProgress | completed |
  failed | declined`.
- **The user's turn is synthesized, not read.** The reader drops `userMessage` items; `write_input`
  in `io/codex.rs` emits `AgentEvent::UserMessageChunk` with the prompt text on each prompt, before
  the request, as `io/jsonl` does for Claude. One source, so the transcript never shows it twice.
- **Streaming** — `item/agentMessage/delta`, `item/reasoning/summaryTextDelta`,
  `item/reasoning/textDelta`, `item/commandExecution/outputDelta`, each `{ itemId, delta }`.
- **Plan** — `turn/plan/updated { plan: [{ step, status: pending | inProgress | completed }] }`.
- **End of a turn** — `turn/completed { turn: { status: completed | interrupted | failed, error:
  { message, codexErrorInfo } } }` is the one end. `thread/status/changed` (`idle`, `active`,
  `systemError`) follows it and ends nothing. An `error { error, willRetry }` notification is a
  diagnostic: a retrying one is a stream reconnect, and a final one is followed by
  `turn/completed { status: failed }` carrying the same error.
- **Subagents** — a spawned agent runs on its own thread (`Thread.parentThreadId` set), and the
  app-server attaches every new thread to every initialized connection, so its notifications
  arrive on the same stdio carrying the child's `threadId`. Multi-agent v2 names a delegate by
  `agentPath` (`/root/chef`) on the parent's `subAgentActivity` items and on the child thread's
  `source.subAgent.thread_spawn`. The mapper (`io/codex.rs`) titles the delegate by nickname, then
  thread name, then the path's last segment, then role, and retitles an already-announced card when
  a name arrives later; a grandchild nests under the delegate that spawned it, and v2's `wait`
  reads "Wait for agents". A `sleep` item is a step ("Sleep 4s"); a command's title is Codex's
  parsed reading ("Read menu.rs") or the bare command without its `sh -lc` wrapper.

**Token usage** is `thread/tokenUsage/updated { threadId, turnId, tokenUsage: { total, last,
modelContextWindow } }`, each breakdown `{ totalTokens, inputTokens, cachedInputTokens,
cacheWriteInputTokens, outputTokens, reasoningOutputTokens }`. Read from Codex's source, not yet
from an authenticated capture: `total` is the thread's running sum of `last`; the context in use is
`last.totalTokens`; cached and cache-written tokens are counted inside `inputTokens` and reasoning
inside `outputTokens`, with `totalTokens = inputTokens + outputTokens`. The same core event also
emits `account/rateLimits/updated { rateLimits: { primary, secondary: { usedPercent,
windowDurationMins, resetsAt }, planType, rateLimitReachedType, … } }`.

The handshake also reads `account/rateLimits/read` once after `thread/start` (best-effort; an API-key or
signed-out home refuses it) and maps it through the same merge as the push, so the quota ring shows before the
first push.

How `CodexBridge` maps all of this onto `AgentEvent` is stated once, in `src/io/codex.rs`.

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

Whether Codex asks at all is its `approval_policy`: the provisioner writes `never` for the
unattended `danger-full-access` mode and `on-request` for every other mode (Ubiq's `D209`). What it
asks arrives as server→client requests, each with `{ threadId, turnId, itemId }` joining it to the
item it is about. `io/codex.rs` parks every one as a permission request and answers with the
person's pick, in each method's own response shape:

| Request method                          | Answer (`result`)                                                |
|-----------------------------------------|------------------------------------------------------------------|
| `item/commandExecution/requestApproval`, `item/fileChange/requestApproval` | `{ "decision": "accept" \| "acceptForSession" \| "decline" \| "cancel" }` |
| `execCommandApproval`, `applyPatchApproval` (legacy) | `{ "decision": "approved" \| "approved_for_session" \| { "denied": { "rejection" } } \| "abort" }` |
| `item/permissions/requestApproval`      | `{ "permissions": <the requested network/fileSystem>, "scope": "turn" \| "session" }`; a refusal grants `{}` |
| `mcpServer/elicitation/request`         | `{ "action": "accept" \| "decline" \| "cancel", "content": null, "_meta": null }` — only a `url` elicitation, or a form requiring nothing (`content: {}`), is offered; any other is answered `cancel` at once |
| `item/tool/requestUserInput`            | `{ "answers": { "<questionId>": { "answers": ["<label>"] } } }` — one question with fixed options, offered as non-allowing options so no unattended path picks one; anything else is answered `{ "answers": {} }` at once |

A request the app-server withdraws when its turn ends is announced as `serverRequest/resolved
{ requestId }`. Any other server request (`item/tool/call`, `account/chatgptAuthTokens/refresh`,
`attestation/generate`) is refused with JSON-RPC error `-32601` so Codex does not wait on it.

**Steering and per-turn settings.** A prompt sent while a turn is live is `turn/steer { threadId,
input, expectedTurnId }`, falling back to `turn/start` if the turn has ended. A prompt merged into the live turn (a steer, or a `turn/start` the server answers with the live turn's id) is counted by the host as a turn of its own, and Codex ends the merged turn once — so at `turn/completed` the reader emits one extra `TurnEnded` per merged prompt. `turn/start` takes
`model`, `effort`, `sandboxPolicy` and `approvalPolicy` overrides that hold "for this turn and subsequent turns" (the bridge sends the picked sandbox mode as the last two); `model/list` (each
model's `supportedReasoningEfforts`, `defaultReasoningEffort`, `isDefault`, `hidden`) is the
catalogue the pickers are drawn from. The bridge's `ConfigOptionUpdate` is always the whole set —
`model`, `thinking`, `mode`, the ids the host's pre-launch set uses — because the composer replaces
its pickers with each update; a partial set drops the missing selectors.

**Resume and fork.** `thread/resume { threadId }` carries a thread on; `thread/fork { threadId }`
opens a new thread with a copy of its history. Both answer like `thread/start`. The bridge opens
with `thread/resume` from `Provisioned::resume`, and with `thread/fork` when `Provisioned::fork`
(from `RunSpec::fork`) is also set: an account's runs share one `CODEX_HOME`, so a caller that
forks by copying the run directory forks nothing here and has to say so. The new thread's id is the
`SessionStarted` id, which is what a later resume of the fork carries. The first `thread/tokenUsage/updated` after either replays that history and is a baseline, billed nothing.

**Accounts.** `account/read` → `{ account: { type: "chatgpt", email, planType } | { type: "apiKey" }
| null, requiresOpenaiAuth }`; `account/rateLimits/read` refuses an API-key or signed-out home with
`-32600 "codex account authentication required to read rate limits"` (observed).
`account/login/start { type: "chatgptDeviceCode" }` → `{ loginId, verificationUrl, userCode }`, then
`account/login/completed { loginId, success, error }`. The probe's `email` is the snapshot's
`QuotaSnapshot::email`. The device-code sign-in is `Harness::begin_device_login` (with
`Harness::device_login` true for both variants), the trait's face of `begin_codex_device_login`.

**Capabilities.** `IoSupport::steer` is true for the native bridge only (`codex-acp` has no
`turn/steer`): a caller may send a prompt while a turn runs.

### Process lifecycle

- **Framing:** newline-delimited JSON-RPC in both directions over stdio.
- **Framing (`codex-acp`):** also newline-delimited JSON-RPC over stdio, but ACP's own methods
  (`session/prompt`, `session/load`, …) rather than the app-server's `thread/*` / `turn/*` — see
  "ACP mode" above.
- **Interrupting a turn:** the `turn/interrupt` request, which aborts the running turn and leaves
  the thread — and the process — alive for the next `turn/start`. Params are
  `{"threadId": "<id>", "turnId": "<id>"}`, both required; the response is an empty object, and the
  turn's end arrives as the usual `turn/completed` (legacy dialect: `turn_aborted`). The turn id is
  stated in exactly one place, `turn/start`'s own ack (`result.turn.id`), so a client that does not
  keep it has nothing to interrupt with — `io/codex.rs` records it there and takes it on a cancel.
  Interrupting with no turn running answers the error `no active turn to interrupt`, which is a
  race rather than a fault and is logged rather than raised. *(Verified against codex-cli 0.152.1:
  `codex app-server generate-json-schema` emits `TurnInterruptParams` — `threadId` + `turnId`, both
  in `required` — and `turn/interrupt` is one of `ClientRequest`'s three `turn/*` methods beside
  `turn/start` and `turn/steer`.)*
- **Ending the session:** close stdin to signal the app-server to stop → wait ~10 s for the reader to drain → `Wait` up to ~10 s more → if still alive, `SIGKILL` the entire process group (negative PID on Unix). `io/codex.rs` splits the two: `AgentInput::Cancel` sends `turn/interrupt`, `AgentInput::Shutdown` (and `Drop`) closes stdin.
- **Minimum versions:** `app-server --listen stdio://` requires Codex ≥ 0.100.0; per-model reasoning discovery requires ≥ 0.131.0. `turn/interrupt` is part of the same v2 `turn/*` family as `turn/start`, so a build that has one has the other.

### Model discovery & selection (agent-manager)

> How `am codex --list-models` enumerates models and `am codex --model <id>` selects
> one. Facts verified against the installed binary (codex-cli 0.142.5) on 2026-07-10.

- **Discover (list models):** `codex debug models --bundled` (Codex ≥ 0.131.0). Needs network/auth: no — verified it returns the full 6-model catalog even with `CODEX_HOME` pointed at an empty, unauthenticated directory; it is purely the static catalog compiled into the binary. Without `--bundled`, `codex debug models` additionally reads/refreshes `$CODEX_HOME/models_cache.json` and returns the account-curated subset instead (3 of 6 models on this login); with no cache/auth available it silently falls back to the same bundled 6-model list rather than erroring, so listing never hard-fails either way. Output: JSON, top-level `{"models":[...]}` (the cache file additionally wraps this with `fetched_at`/`etag`/`client_version`). Per-model fields verified: `slug`, `display_name`, `description`, `default_reasoning_level`, `supported_reasoning_levels` (array of `{effort, description}`), `shell_type`, `visibility` (`list` | `hide`), `supported_in_api`, `priority`, `additional_speed_tiers`, `service_tiers`, `availability_nux`, `upgrade`, `base_instructions`.
- **Select at launch (passthrough):** top-level `-m`/`--model <id>` flag on the normal interactive `codex` launch (the same flag also exists on `codex exec`) — verified via the run preamble's `model: <id>` line. This is distinct from the headless app-server RPC path documented above, where the model travels inside `thread/start` params, not a CLI flag; for a passthrough tty run, `am` should inject `-m <id>` directly. If omitted, Codex falls back to the `model` key in `$CODEX_HOME/config.toml` (verified: `model = "gpt-5.5"` on disk was used when `-m` was omitted). `-c model=<id>` is an equivalent generic config-override mechanism for the same effect.
- **Model id format:** a flat slug, no vendor prefix and no alias layer — the slug itself is the id, e.g. `gpt-5.5`, `gpt-5.4-mini`, `gpt-5.3-codex`.
- **Example ids (verified):** bundled/static catalog — `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.3-codex`, `gpt-5.2`, `codex-auto-review` (this last one has `visibility: "hide"` — an internal review model, not meant to be user-selected). Account-curated live subset on this login: `gpt-5.5`, `gpt-5.4-mini`.
- **Default model:** the `model` key in `$CODEX_HOME/config.toml` (here: `gpt-5.5`, paired with `model_reasoning_effort = "medium"`); `-m`/`--model` (or `-c model=<id>`) on the CLI overrides it for that invocation only.
- **Reasoning-effort discovery (`Harness::discover_thinking`):** `Codex::discover_thinking` (`src/harness/codex.rs`) reads `default_reasoning_level` / `supported_reasoning_levels` off the *same* `codex debug models --bundled` value `discover_models` already parses (a shared `bundled_models()` helper — one process spawn, not two). Each `{effort, description}` entry becomes a `ThinkingLevel { value: effort, label, description }`; `label` is `effort` title-cased, except `xhigh` → `"Extra high"`. A model whose `supported_reasoning_levels` is empty or absent is left out of the map entirely (no reasoning knob), rather than inserted with an empty `levels` vec.
- **Reasoning effort at launch (`RunSpec.thinking`):** `build_config_toml` emits `model_reasoning_effort = "<value>"` as a top-level key beside `model` (see "Configuration" above), only when `RunSpec.thinking` is set and non-empty — Codex rejects an empty string, so a blank pick is treated the same as unset rather than forwarded. Omitted entirely when unset, keeping `config.toml` byte-identical to before this field existed.
- **Permission modes (`Harness::modes`):** `Codex::modes()` returns the three values [`map_sandbox_mode`](#system-a-legacy-sandbox_mode--approval_policy) already accepts — `read-only`, `workspace-write`, `danger-full-access` — as a fixed list. Fixed CLI/config enum, not probed the way models/effort are. This is the same mapping `RunSpec.policy.permission_mode` already went through before `modes()` existed; `modes()` just names the set `am` offers a picker rather than re-deriving it.

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
