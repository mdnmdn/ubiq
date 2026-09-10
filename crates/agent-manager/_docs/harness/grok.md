# Grok CLI

Stable id: `grok`
Display name: Grok CLI
Vendor: **xAI** — the official `grok` CLI (Rust binary), talking to xAI's own Grok API.

> **Identity correction (verified 2026-09-10):** earlier revisions of this document described a
> *different* program — the community project `superagent-ai/grok-cli`, published to npm as
> `@vibe-kit/grok-cli`, which happens to install a same-named `grok` binary. The harness actually
> installed and driven by `crates/agent-manager/src/harness/grok.rs` is xAI's own official CLI,
> version **1.0.13**, captured live on 2026-09-10 — see
> [`_docs/wip/grok-acp-capture.md`](../../../../_docs/wip/grok-acp-capture.md) for the raw frames.
> Sections below are corrected where the capture contradicts the old npm-CLI assumptions (binary
> identity, on-disk layout, CLI surface, ACP behaviour, model/reasoning, permissions). Sections not
> yet re-verified against 1.0.13 (Sub-agents, MCP servers, Skills, Slash commands, most of
> Authentication) still describe the npm CLI's documented behaviour and are marked unverified
> inline — treat them as unconfirmed for the installed binary until re-checked.

## Quick reference

| Field         | Value                                                                          |
|---------------|--------------------------------------------------------------------------------|
| Stable id     | `grok`                                                                          |
| Display name  | Grok CLI                                                                        |
| Vendor        | xAI (official CLI, Rust binary; verified 2026-09-10, version 1.0.13)           |
| Global root   | `~/.grok/` — `auth.json`, `config.toml`, `docs/user-guide/*.md`, `sessions/`, `bin/` (verified 2026-09-10) |
| Project root  | `<repo>/.grok/settings.json`, `<repo>/AGENTS.md`, `<repo>/.agents/skills/` (unverified against 1.0.13 — carried over from the npm CLI) |
| Config format | TOML (`config.toml`, verified); JSON (`auth.json`, verified); other files below unverified |

There **is** an official xAI first-party terminal coding agent: the `grok` CLI, a Rust binary,
distinct from the community npm package `@vibe-kit/grok-cli` (`superagent-ai/grok-cli`) that an
earlier revision of this document assumed. Both install a command literally named `grok`, which is
almost certainly why the two got conflated — check `grok --version` against `1.0.13` (or later) to
confirm which one is on `PATH`. Everything below describes the xAI binary except where noted
otherwise.

## On-disk layout

### Global (`~/.grok/`) — verified 2026-09-10

The live 1.0.13 install carries, at `~/.grok/`:

```
~/.grok/
├── auth.json                 # OAuth credential (cached_token), plaintext JSON
├── config.toml                # global config
├── docs/
│   └── user-guide/*.md        # the bundled user guide (e.g. 15-agent-mode.md, 22-permissions-and-safety.md)
├── sessions/                   # saved conversation sessions
└── bin/                        # (present; contents not inventoried)

# Skills (Agent Skills open standard), user tier — unverified against 1.0.13,
# carried over from the npm CLI's documented layout:
~/.agents/skills/<name>/SKILL.md
```

`user-settings.json` and `workspace-trust.json`, both load-bearing in the npm CLI's layout, have
**not** been confirmed present or absent for the official binary — do not assume either exists
until checked live. `agent-manager`'s provisioner (`harness/grok.rs`) still writes MCP config to
`<HOME>/.grok/user-settings.json` on the assumption the official binary reads the same file; this
has not been re-verified against 1.0.13 (see "MCP servers" below).

### Project (`<repo>/`) — unverified against 1.0.13

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

This project-tier layout is carried over from the npm CLI and has not been re-verified against the
official binary; `agent-manager`'s `Grok::provision` relies only on the global tier (`HOME`
relocation) today, so a project-tier mismatch would not currently affect provisioning.

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
reference for the npm CLI; not re-checked for the official binary, but
`harness/grok.rs`'s Class-C isolation lever — relocating `HOME`
wholesale — is unaffected either way since that is its only lever
regardless of a config-dir override existing). To relocate the global
tier a coordinator must set `HOME` (and, on Windows, the platform home)
for the child process.

Model resolution order (highest first) — the `-m`/`--model` and
`--reasoning-effort` entries are verified 2026-09-10 against 1.0.13; the
rest is carried over from the npm CLI and unverified:

1. `GROK_MODEL` environment variable (npm CLI; unverified for 1.0.13).
2. `-m` / `--model <id>` CLI flag (verified 2026-09-10) — for `grok agent
   stdio`, this and `--reasoning-effort <level>` go *between* `agent` and
   `stdio`: `grok agent --model grok-4.6 --reasoning-effort high stdio`
   (see "ACP mode" below).
3. Project settings — `.grok/settings.json` → `model` (npm CLI; unverified).
4. User settings — `~/.grok/user-settings.json` → `defaultModel` (npm CLI; unverified).
5. Built-in default (verified: `grok-4.6` is the current default via `session/new`'s
   `models.currentModelId`).

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
| Permissions    | partial | `--permission-mode <mode>` (6 values, passthrough only, verified 2026-09-10); `--always-approve`/`_meta.yoloMode` on the ACP path; no allow/deny rule file |
| Structured I/O | partial | ACP over `grok agent stdio`; generic `AcpBridge`. Verified 2026-09-10 that `session/new` **requires** a preceding `authenticate` call the bridge does not make today, and that models/reasoning arrive in vendor `_meta` shapes the bridge does not read — see "ACP mode" below and `_docs/wip/grok-acp-capture.md` |
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

### Credential capture & reuse (agent-manager)

> How `am account capture` / `am account login` snapshot and replay this
> harness's login into an ephemeral run. Records file **structure and non-secret
> metadata only** — token values are copied opaquely.

> **Disk correction:** the sections above describe an API-key model
> (`~/.grok/user-settings.json → apiKey`). On disk the live login is OAuth 2.0
> (OIDC to `https://auth.x.ai`) stored in **`~/.grok/auth.json`** — trust disk.
> The API-key path still works via `GROK_API_KEY` but is not what an interactive
> subscription login writes.

- **Bundle files (the credential snapshot):**
  - `~/.grok/auth.json` — **required**; JSON keyed by `<oidc_issuer>::<user_id>`
    (e.g. `https://auth.x.ai::<uuid>`), each entry holding `key` (JWT),
    `refresh_token`, `expires_at`, plus identity fields.
  - `~/.grok/user-settings.json` — *optional*; only if an `apiKey` / model
    override is in use.
- **Relocation lever:** no config-dir override env var — set `HOME` to relocate
  the whole `~/.grok/` tree.
- **Force file storage (skip keychain):** N/A — Grok is **always plaintext file**
  (mode `0600`), no OS keychain integration. The ideal case for capture.
- **Login command (fresh-auth-into-temp):** no documented `grok auth login`
  verb; the interactive TUI triggers the OAuth flow on first run under a fresh
  `HOME`. Headless: inject `GROK_API_KEY` instead of snapshotting OAuth.
- **Extractable metadata (non-secret):**

  | field | source | identifies |
  |---|---|---|
  | `email` | `auth.json → <entry>.email` | account email *(identifying — redact)* |
  | `user_id` / `principal_id` | `auth.json → <entry>.user_id` | user account id (UUID) |
  | `team_id` | `auth.json → <entry>.team_id` | team/org membership (UUID) |
  | `expires_at` | `auth.json → <entry>.expires_at` | token expiry (ISO 8601) |
  | `auth_mode` / `oidc_issuer` | `auth.json → <entry>.*` | auth type (`oidc`) + provider |

- **Do not copy:** `sessions/`, `projects/`, `logs/`, `worktrees.db`,
  `models_cache.json`, `agent_id`, `*.lock` — session/machine-bound state.

## Permissions

Corrected 2026-09-10 against the installed 1.0.13 binary and its bundled user guide
(`~/.grok/docs/user-guide/15-agent-mode.md`, `22-permissions-and-safety.md`; see
`_docs/wip/grok-acp-capture.md` §4). The npm CLI's workspace-trust/sandbox-flag model described
below has **not** been re-verified and may not apply to the official binary at all.

The official binary's permission surface has three layers, none of which is an allow/deny rule
file:

1. **Top-level `--permission-mode <mode>`** (verified 2026-09-10) — one of `default`,
   `acceptEdits`, `auto`, `dontAsk`, `bypassPermissions`, `plan`. This is the TUI/headless
   passthrough lever; `Grok::modes()` lists these six and `Grok::unattended_mode()` names
   `bypassPermissions` as the ask-nothing one.
2. **`--always-approve`** (alias `--yolo`, verified 2026-09-10) — an *agent-mode* flag, i.e. it
   goes between `agent` and the transport name (`grok agent --always-approve stdio`), and is
   process-wide auto-approval for both the TUI and `grok agent stdio`.
3. **Per-session `_meta.yoloMode: true`** on `session/new` (verified 2026-09-10) — the ACP-only,
   per-conversation equivalent of `--always-approve`; `_meta.autoMode: true` selects Grok's `auto`
   mode instead. Also documented on `session/new` `_meta`: `rules`, `systemPromptOverride`,
   `agentProfile`.

**`grok agent stdio` (the structured/ACP path) has no `--permission-mode` selector at all** — that
flag exists only at the top level, outside agent mode. `Grok::provision`'s `IoModes::Structured`
arm therefore emits `--model`/`--reasoning-effort` (both accepted between `agent` and `stdio`) but
deliberately no permission flag; `spec.policy.permission_mode` only reaches the process on the
passthrough path, as `--permission-mode <id>` in top-level argv. Wiring `_meta.yoloMode` into the
ACP bridge's `session/new` call is a change to `io/acp_client.rs`, out of this harness module's
scope.

The npm CLI's workspace-trust + sandbox-flag description (unverified for 1.0.13, kept for
reference):

1. **Workspace trust** — `~/.grok/workspace-trust.json` records per-
   directory trust decisions. Running `grok` in an untrusted directory
   prompts for trust before tools execute.
2. **Sandbox flags** (microVM isolation, primarily for `/verify` and
   `--verify`): `--sandbox` / `--no-sandbox`, `--allow-net`,
   `--allow-host <host>`, `--port <n>`. These gate network and host
   access for sandboxed runs rather than per-tool approval.

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

### ACP mode (`grok agent stdio`)

`grok agent stdio` starts Grok as a real **Agent Client Protocol endpoint** on its own stdio,
speaking newline-delimited JSON-RPC 2.0 — captured live 2026-09-10 (`grok` 1.0.13). This is the
launch `am` drives for every structured run of this harness, through the harness-neutral
`AcpBridge` (`src/io/acp_client.rs`) rather than any Grok-specific wire — see
[`../io-modes.md`](../io-modes.md) and, for the full captured frames, `_docs/wip/grok-acp-capture.md`
(authoritative over the summary below; this section will go stale before that capture does).

Structured argv, verified 2026-09-10:

```
grok agent [--model <id>] [--reasoning-effort <level>] stdio [passthrough_args...]
```

`agent`'s own options — including `-m`/`--model` and `--reasoning-effort` (alias `--effort`) —
go **between `agent` and `stdio`**, not after it; putting them after `stdio` does not work. There
is still no `--prompt` (the prompt is a `session/prompt` request over the wire) and no `--session`
(a resume is `session/load` against the id the previous run reported).

Four behaviours the capture establishes that `src/io/acp_client.rs` does not yet handle — listed
here for reference; fixing the bridge itself is out of this harness module's scope:

- **`authenticate` is mandatory.** `session/new` fails with `-32000 "Authentication required"`
  unless `{"method":"authenticate","params":{"methodId":"cached_token"}}` precedes it. The bridge
  never sends it today, so every Grok ACP conversation dies at `session/new`.
- **Models and reasoning effort arrive in vendor shapes**, not the ACP-standard `configOptions`/
  `modes` the bridge reads: `session/new`'s `_meta["x.ai/sessionConfig"].options` mixes
  `category: "model"` and `category: "mode"` entries (Grok calls reasoning effort a "mode" — it is
  not a permission mode). Setting either is `session/set_mode` (effort) or `session/set_model`
  (model); `session/set_config_option`, the only setter the bridge sends, names an option Grok
  never advertises.
- **No permission mode over ACP** — see "Permissions" above.
- **Subagents stream on a second `sessionId`** rather than the `subagent_spawned` extension the
  bridge's `is_delegate` heuristic expects, so a delegate's transcript (and its prompt) currently
  renders as ordinary user/assistant turns instead of a nested delegate.

The `--format json` NDJSON stream (below) stays **unused** for structured runs: its event names
are documented but its per-field shapes are not, and ACP's shapes are specified — the reason this
harness goes through ACP rather than a Grok-specific NDJSON bridge.

### Model & reasoning at launch

Verified 2026-09-10 against 1.0.13:

- **Model:** `-m`/`--model <id>` (passthrough, top-level) or `--model <id>` between `agent` and
  `stdio` (agent mode / ACP). Currently offered ids: `grok-4.6`, `grok-4.5` (`grok models` lists
  them). `GROK_MODEL` env is carried over from the npm CLI and unverified for 1.0.13.
- **Reasoning effort:** `--reasoning-effort <level>` (alias `--effort`), same placement rule as
  `--model`. Values are per-model: `grok-4.6` accepts `xhigh | high | medium | low` (default
  `high`); `grok-4.5` accepts `high | medium | low`. This is a real, separate flag — the npm CLI's
  "effort is a property of the model id, no separate flag" claim does not hold for the official
  binary.
- `RunSpec::thinking` (`crates/agent-manager/src/spec.rs`) is the harness-neutral field
  `Grok::provision` reads for this; see `harness/grok.rs`'s `IoModes::Structured` and
  `IoModes::Passthrough` arms.

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
stream as of 2026-07-09. ACP mode is where an approval does reach the
caller: `session/request_permission` is an agent-to-client request, and
`AcpBridge` parks it until the caller answers.

### Process lifecycle

- Framing: prompt in the `--prompt` flag value; events out on stdout as
  NDJSON under `--format json`; diagnostics on stderr.
- Cancellation: signal the process group (no documented stdin-close
  handshake, since the prompt is an argv flag, not a stdin stream).
- Minimum CLI version: not documented. The `--format json` event set is
  characterised from the current README as of 2026-07-09.

### Model discovery & selection (agent-manager)

> How `am <harness> --list-models` enumerates models and `am <harness> --model <id>`
> selects one.

> **Correction (verified 2026-09-10):** the official binary has a real `grok models` CLI
> command — there is no `~/.grok/models_cache.json` to read instead (that file belonged to the
> npm CLI this section originally described). `Grok::discover_models` (`harness/grok.rs`) now
> shells out to `grok models` and parses model ids off stdout. The exact output layout has not
> been captured verbatim, so the parser is a best-effort token scan for `grok-<version>`-shaped
> words rather than a format-specific one; tighten it once a real capture of the command's stdout
> exists.

- **Discover (list models):** `grok models` (verified 2026-09-10 that the command exists; its
  exact output format is not yet captured). Needs network/auth: unconfirmed.
- **Select at launch (passthrough):** `-m` / `--model <id>` CLI flag (verified). `GROK_MODEL`
  environment variable is carried over from the npm CLI and unverified for 1.0.13.
- **Select at launch (agent mode / ACP):** `--model <id>` between `agent` and `stdio` (verified
  2026-09-10 — see "ACP mode" above).
- **Model id format:** e.g. `grok-4.6`, `grok-4.5` (verified 2026-09-10).
- **Default model:** `grok-4.6`, per `session/new`'s `models.currentModelId` (verified 2026-09-10).

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
- **Superseded — there IS an auto-approve flag.** `--always-approve`/`--yolo` (agent mode) and
  `_meta.yoloMode` (per ACP session) both exist, verified 2026-09-10 — see "Permissions" above.
  The npm CLI's "no auto-approve flag, pre-trust the workspace or use `--batch-api`" claim below
  does not hold for the official binary; kept for reference against the npm CLI only.
- **Prompt is an argv flag** (`--prompt <text>`), not a positional arg
  and not stdin. This differs from opencode (positional) and CodeBuddy
  (stdin NDJSON).
- **`user-settings.json` is written mode `0600`.** Preserve permissions
  when editing.
- **`--max-tool-rounds` default is 400.** Lower it for bounded CI runs.
- **Relocating `HOME` isolates config but has been observed NOT to isolate
  session/log writes.** A real launch with `HOME` set to an ephemeral dir
  still wrote to the user's real `~/.grok/sessions/…` and `~/.grok/logs/…` —
  a known non-invasiveness gap, not yet closed. The *mechanism* previously
  written here (Node's `os.homedir()` vs. `os.userInfo().homedir`) assumed a
  Node.js binary; the 2026-09-10 capture confirms the installed `grok` is a
  **Rust** binary, so that explanation does not apply and is removed. Why the
  leak happens for the Rust binary is unconfirmed — re-verify against 1.0.13.
  Until then, treat the isolation lever as config/skills-only, not full-home
  isolation.

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

Because there is **no per-run MCP flag** and no approval handshake on the
`--format json` stream, the Grok renderer is closer to a "materialise
files + passthrough" model than the flag-driven CodeBuddy renderer. A
structured run sidesteps both by going through ACP instead.

## Sources

- [`_docs/wip/grok-acp-capture.md`](../../../../_docs/wip/grok-acp-capture.md) — live capture
  against the installed `grok` 1.0.13 (xAI, Rust), 2026-09-10: binary identity, `~/.grok/` layout,
  `grok agent [options] stdio` argv, ACP `authenticate`/model/mode/subagent behaviour,
  `--permission-mode` values, `--always-approve`/`_meta.yoloMode`. **Authoritative over the rest of
  this document** wherever the two disagree.
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
