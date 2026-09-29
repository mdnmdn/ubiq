# CLI surface (`am`)

> The binary is still built as `agent-manager`; `am` is the intended short
> alias (installed name / symlink). This doc uses `am`.

## Command shape

`am` has a small set of **reserved subcommands** for managing the tool itself,
and otherwise treats the first positional as a **harness name** to wrap:

```
am <harness> [am-flags] [-- harness-args…]     # wrap & run a harness
am catalog   <ls|import|show|path> …            # manage the catalog
am skill     <ls|link|add-dir|rm-dir|rm|sources|search|install> …   # manage skills of a catalog layer
am mcp       <ls|add|import|rm|search> …        # manage MCP servers of a catalog layer
am account   <ls|use|import|login|dump|check|renew|rename|delete> …  # manage accounts + credentials
am profile   <ls|show|use|create|login> …       # manage profiles
am session   <ls|show|resume> …                 # manage session history  (ls/show/resume landed)
am help | am --version
```

Reserved words (`catalog`, `skill`, `mcp`, `account`, `session`, `help`) are checked before
harness resolution. A harness id is never one of these, so there is no
collision. Unknown first-positional → looked up in the harness registry; if it
is not a known harness, error with the list of known ids.

## Running a harness

```bash
am claude --mcps postgres,figma --skills web-designer --safe
am codex  --skills reviewer --account work
am opencode --config ./run.toml
```

**Where the harness's config comes from (`D194`).** A harness that runs from a
config home (native Claude Code) runs from the **account's** home when the run
resolves to an account (`ConfigStrategy::Home`) — the same home for every run as
that account, from any profile and any project, concurrently — and from the
user's own default config, untouched, when it resolves to none
(`ConfigStrategy::Native`) — in both cases with a fresh run dir under `AM_RUNS`
as scratch for the run's own files, removed on exit like an ephemeral dir (never
the home). Homes live under `<config dir>/harness-homes/<account>/<harness>/`
(env override: `AM_HOMES`); with no homes root at all a run is `Native` too. A
confined run takes the same strategy, and the sandbox is granted that home
read-write (`IsolateOptions::grant_config_home`: the account's home, or
`Harness::default_homes` under `Native`) and on macOS the Keychain layer. Only a harness that cannot share a home keeps the per-run
ephemeral dir, and no built-in one is such. `am` has no flag naming a config
dir. An API-key, auth-token or helper account applies to any run.

### The core run flags (Phase 1 unless noted)

| Flag                     | Meaning                                                             |
|--------------------------|--------------------------------------------------------------------|
| `--mcps a,b,c`           | Catalog MCP ids to inject (repeatable or comma-separated).          |
| `--skills a,b`           | Catalog skill ids to inject.                                        |
| `--mcp-json <path>`      | Inject an inline MCP definition file (bypasses the catalog).        |
| `--safe`                 | Shorthand policy preset (restricted tools/permissions). *(1)*       |
| `--config <path>`        | Settings file to merge (toml/yaml). Default: discovered (see below).|
| `--catalog <path>`       | Catalog root override (else env, else config, else default).        |
| `--keep-config`          | Don't delete the ephemeral config dir on exit (debugging).          |
| `--print-config`         | Provision only; print the generated dir + argv + env — and, on a confined run, the effective isolation policy (layer stack, grants, home, argv); don't launch. |
| `--account <id>`         | Account/credential profile to use.                                  |
| `--model <id>`           | Launch with a specific harness-native model id (discover with `--list-models`). |
| `--list-models`          | List the models available for this harness and exit (don't launch). |
| `--thinking <level>`     | Launch with a reasoning-effort level, in the harness's own vocabulary (e.g. Claude's `--effort`, Codex's `model_reasoning_effort`). |
| `--permission-mode <id>` | Launch with a permission/sandbox mode — one of the harness's fixed set (see `Harness::modes()`; e.g. Claude's `plan`/`acceptEdits`/…, Codex's `read-only`/`workspace-write`/`danger-full-access`). Highest precedence: overrides just the mode a `--safe` preset expanded, leaving the rest of its policy untouched. |
| `--hooks a,b`            | Enable named hooks (defined in the settings file) for this run.     |
| `--instructions <path>`  | Seed always-on instructions into the harness config.                |
| `--prompt <text>`        | Seed an initial prompt for the first harness message.               |
| `--io <mode>`            | I/O mode: `passthrough` (default) or `structured` (alias `jsonl`).  |
| `--output <mode>`        | `--io structured` event projection: `events` (default), `acp`, or `agui` (alias `ag-ui`). |
| `--isolate[=profile]`    | Confine the run under an isol8 sandbox policy. Bare `--isolate` adds no named layer beyond the built-in defaults; `--isolate=<name>` layers a named profile on top. Refused together with `--io structured`. *(2)* |
| `--no-isolate`           | Opt out of isolation, even when a settings file, a resolved profile, or `[isolate] enabled` would otherwise turn it on. `conflicts_with` `--isolate`. |
| `--resume <id>`          | Resume a prior run by its *harness-native* session id (see below).  |
| `--mcp-as-skill a,b`     | Expose these already-injected catalog mcp ids as a latent skill pointer for this run (see [`mcp-as-skill.md`](./mcp-as-skill.md)). |
| `-- <harness-args…>`     | Everything after `--` is forwarded verbatim to the harness binary.  |

*(1)* `--safe` is a named **preset** resolved from the settings file / built-in
defaults, not a hard-coded flag list — so teams can define what "safe" means.

*(2)* A structured run bridges the harness over pipes, and isol8 spawns with
inherited stdio; combining `--isolate` with `--io structured` has no seam to
confine through yet, so `am` refuses the combination outright rather than
silently running it unconfined. Passthrough is unaffected: on macOS, a
confined passthrough run execs `sandbox-exec` around the harness; on Windows
it instead re-invokes the running binary under `--am-confine`
(`isolate::confine_entrypoint`), which reads the resolved policy from a file
and lets isol8 spawn the harness from inside that process's own console — no
second binary ships. isol8's Windows backend adds no console-creation flag of
its own, so that child simply attaches to whichever console the re-invoked
process runs in, ConPTY included. Only Linux still has no seam: Landlock
applies between `fork` and
`exec` and isol8 keeps its `SandboxChild` constructors private, so `--isolate`
fails there until isol8 grows a rendered form; see
`refs/isol8-pty-seam-update.md`.

Anything `am` doesn't recognize after `--` is the harness's own CLI (e.g.
`am claude -- --model opus -p`). This keeps `am` from having to mirror every
harness flag.

## Settings file + flag merge

Configuration is a **mix** of the settings file and CLI flags. Precedence,
highest first:

1. **CLI flags** (`--mcps`, `--account`, …).
2. **`--config <path>`** if given, else the **discovered** settings file.
3. **Environment** (`AM_CATALOG`, `AM_CONFIG_FILE`, `AM_CONFIG_FOLDER`, …).
4. **Built-in defaults.**

### Config file resolution (precedence)

The settings file is resolved using this hierarchy, **highest → lowest**:

1. **`--config <path>`** (CLI flag): full path to a settings file (toml/yaml).
2. **`AM_CONFIG_FILE`** (environment): full path to a settings file.
3. **`$AM_CONFIG_FOLDER/config.{toml,yaml,yml}`** (environment): a directory containing the settings file.
4. **Project walk**: nearest `am.toml`/`agent-manager.*`/dotfile from CWD, walking up to git root.
5. **`~/.config/agent-manager/config.toml`** (global default; `.yaml`/`.yml` also accepted).

Note: the **catalog root**, **accounts**, and **session state** (`AM_CATALOG`, `AM_ACCOUNTS`, `AM_SESSIONS`) each have independent environment variable overrides and platform-specific fallbacks. They are unaffected by the settings file precedence above.

### Discovery order for the settings file

Walk up from the CWD to the git root. In **each** directory, try these basenames
in order and take the first that exists:

```
am.toml  am.yaml  am.yml
agent-manager.toml  agent-manager.yaml  agent-manager.yml
.am.toml  .am.yaml  .am.yml
.agent-manager.toml  .agent-manager.yaml  .agent-manager.yml
```

If nothing is found in the walk, fall back to the global
`~/.config/agent-manager/config.{toml,yaml,yml}`. First found wins as the base;
CLI flags layer on top. (This mirrors the harness `CLAUDE.md` walk, so it feels
familiar.) Format is chosen by extension: `.toml` → TOML, `.yaml`/`.yml` → YAML.

### Settings file shape (sketch — full schema in a later revision)

```toml
# ~/.config/agent-manager/config.toml  or  ./.agent-manager.toml

catalog = "~/.agent-manager/catalog"        # catalog root (overridable by --catalog/env)

[defaults]                                   # applied to every `am <harness>` run
mcps   = ["github"]
skills = []

[harness.claude]                             # per-harness defaults
account = "work"
mcps    = ["postgres"]

[presets.safe]                               # what `--safe` expands to
permission_mode = "restricted"
deny            = ["Bash(rm *)", "WebFetch"]

[isolate]                                    # confine runs under isol8 by default
enabled = true                               # off unless a flag/profile says otherwise
profile = "default"                          # layer added when a confined run names no profile of its own
home    = "inherit"                          # "inherit" (the real home, the default) | "ephemeral" | "managed"
```

`[isolate]` sets the default; a run's actual isolation is resolved highest-precedence
first: `--no-isolate` (off, unconditionally) → `--isolate=<name>` → bare `--isolate`
(layer from `[isolate].profile`) → the resolved profile's own `isolate` field →
`[isolate].enabled` → off. `home = "managed"` needs a name to key the home by — the
CLI keys it by the run's `--account`, falling back to the harness id.

`home` defaults to `"inherit"`, and the other two answers cost a toolchain. A layer's
`~`-relative grant expands against the run's *effective* home, so under `"ephemeral"` or
`"managed"` every grant the toolchain layers carry — `~/.cargo`, `~/.npm`, `~/.dotnet` —
names a directory inside a home nothing has populated: the run holds `cargo` on its `PATH`
and cannot build. Inheriting grants nothing extra by itself; only the paths a resolved
layer names are reachable inside the home.

### Merge semantics — **replace by default** (decided)

A value from a higher-precedence layer **replaces** the same value from a lower
layer; it does not union with it. Concretely:

- `--mcps a,b` **replaces** whatever `mcps` the settings file provided for this
  run (it is not added to `[defaults].mcps` or `[harness.<id>].mcps`).
- `[harness.claude].mcps` **replaces** `[defaults].mcps` for `am claude`.
- To *extend* rather than replace, list the full set you want (there is no
  implicit append). An explicit `--add-mcps` / `--add-skills` convenience may be
  offered later as sugar, but the base semantics are replace.

This keeps the effective set easy to reason about: the highest layer that
mentions a key wins outright. Both TOML and YAML are accepted; TOML is the
documented default.

## Storage locations & environment variables

All of `am`'s state lives under a single base — **`~/.config/agent-manager/`** on
every platform (Linux, macOS, Windows alike; `am` does not scatter into
OS-specific `Application Support` / `%APPDATA%` / state dirs). Each store can be
relocated independently by its own environment variable (highest precedence),
and some also by a CLI flag.

| Store | Default | Env override | CLI flag |
|-------|---------|--------------|----------|
| Settings file | `~/.config/agent-manager/config.toml` | `AM_CONFIG_FILE` (full path to a config file) · `AM_CONFIG_FOLDER` (a dir holding `config.{toml,yaml,yml}`) | `--config <path>` |
| Accounts | `~/.config/agent-manager/accounts/` | `AM_ACCOUNTS` | — |
| Credentials engine | `files` (default; `keychain` plaintext vault or `os` real OS-encrypted, opt-in) | `AM_CREDENTIALS_ENGINE` · `AM_KEYCHAIN` (vault/keychain dir) | — |
| Catalog | `~/.config/agent-manager/catalog/` | `AM_CATALOG` | `--catalog <path>` |
| Sessions | `~/.config/agent-manager/sessions/` | `AM_SESSIONS` | — |
| Ephemeral run dirs | `~/.config/agent-manager/runs/` | `AM_RUNS` | `--keep-config` (retains, doesn't relocate) |

Notes:

- A **project-local** settings file (`am.toml` …) discovered by walking up to the
  git root still wins over the global `config.toml` default — see the
  precedence list above. The two settings env vars govern **only** the settings
  file; they have no effect on the accounts/catalog/sessions/runs roots, each of
  which has its own env var in the table.
- **Accounts** and **catalog** are user-curated config-like stores. **Sessions**
  (recorded run history) and **run dirs** (per-run ephemeral config, normally
  deleted on exit) are working state; they live under the same base purely for a
  single, predictable location. Point `AM_SESSIONS` / `AM_RUNS` elsewhere (e.g. a
  `tmpfs` or a scratch dir) if you'd rather keep transient state out of `~/.config`.
- `am account use <id>` **writes** the global `config.toml`, always resolving the
  same path the read side uses.

## Catalog commands

```bash
am catalog ls                 # list available skills + MCP servers
am catalog ls --mcps          # filter
am catalog show postgres      # print one entry's resolved definition
am catalog path               # print the active catalog root
am catalog import             # ingest ~/.claude, ~/.agent, … into the catalog
am catalog import --from ~/.claude --dry-run
```

`am catalog import` is the adoption on-ramp: it reads well-known agent config
dirs (`~/.claude`, `~/.agent`, project `.mcp.json`, …) and copies their skills
and MCP definitions into the catalog so they can be injected by id. It **reads**
those dirs; it never writes back to them. Full behavior in
[`registry.md`](./registry.md).

## Skill and MCP commands

`am skill` and `am mcp` write to one catalog layer: the global one (`--catalog` / `AM_CATALOG` /
default), or with `--project` the project's `<cwd>/.agent-manager/catalog`.

```bash
am skill ls                                  # skills of the layer, with where each comes from
am skill link ~/skills/pdf [--id pdf]        # reference a skill folder in place ([[skill]])
am skill add-dir ~/skills                    # scan a folder: every <sub>/SKILL.md is a skill
am skill rm-dir ~/skills
am skill rm pdf                              # installed copy or link (a scanned skill: remove its folder)
am skill sources                             # declared [[skill_source]] entries, or the defaults
am skill search [pdf] [--source anthropics]  # search the sources (git clones are cached, refreshed hourly)
am skill install anthropics/skills/pdf [--id my-pdf]   # <source>/<path> as printed by search

am mcp ls
am mcp add fs -- npx -y @x/fs /tmp --env TOKEN=abc    # local: command after `--`
am mcp add docs --url https://h/mcp [--sse] --header "Authorization: Bearer t"
am mcp import claude.json [--id-prefix p-]   # any harness's format; `-` reads stdin; --id names one unnamed server
am mcp rm fs
am mcp search filesystem [--limit 10]        # official MCP registry
```

Git sources are cached in `cache/skill-sources` beside the global catalog root. Details in
[`registry.md`](./registry.md).

## Account commands

```bash
am account ls                                  # list available accounts
am account use <id>                            # set the default account for future runs
```

Accounts are stored under `~/.config/agent-manager/accounts/` (env override: `AM_ACCOUNTS`).
An account holds credential **references**, never secret material: environment variable names
(`api_key_env`, `auth_token_env`), a `base_url`, and/or a credential helper command. When injected
with `--account <id>`, the account's references are resolved into the harness's native auth
slots. Full account schema in [`overview.md`](./overview.md). A harness login is not one of those
references: it lives in the account's own config home, signed in with `am account login` below,
and `am` neither captures nor copies it (`D194`). An account file written before that may name a
`home`; it is ignored.

### Signing an account in with `am account login`

```bash
am account login <id> --harness <h>    # sign that account's home in
am account logout <id> --harness <h>   # remove that home — the sign-out
am account check <id> --harness <h>    # does it hold a login, and until when
```

`am account login <id> [--harness <h>]` signs the account's config home in
(`D194`) — the harness defaults to `claude-code` and must be one that runs from
a home. It prepares the home (`provision::prepare_home`: created, templates
applied once) and runs the harness's own login into it interactively
(`Harness::login_home`; Claude: `CLAUDE_CONFIG_DIR=<home> claude auth login`).
Nothing is captured, verified or read back: the harness keeps the login in the
home and refreshes it. The account's first terminal run reaches the same place
through the harness's own login screen, so this is the explicit route, not a
required step.

`am account logout` removes that one home (`HomeStore::forget`); removing one
that is not there is success. `am account check` reads the expiry the login
states about itself, in place (`home::login_validity`) — the one token read
`D194` keeps, and an absent file is not proof of no login, since on macOS a
harness may keep it in the Keychain instead.

## Session commands

```bash
am session ls                 # list recorded sessions, newest first
am session show <id>          # show one session's metadata + transcript summary
am session resume <id>        # resume a prior am-recorded session
```

Every real run (passthrough or structured) is recorded under `am`'s own state
dir; `am session resume <id>` re-launches a *recorded* `am` session by
resuming the harness's own conversation, using the session's retained config
dir plus the harness's native resume flag:

- **Claude Code:** `--resume <session-id>` (both passthrough and headless).
- **opencode:** `--session <id>` on the structured `opencode run` form only —
  interactive opencode has no CLI resume flag.
- **codex:** not yet — codex has no CLI resume flag at all; resuming a codex
  session is an app-server `thread/resume` JSON-RPC call, deferred to a later
  (bridge) step.

Resume only works for sessions that captured a harness-native session id
(structured runs) and whose config dir is still on disk (recorded sessions
now retain their config dir rather than deleting it — see "Exit codes &
passthrough fidelity" below). A run from a profile home or the native config
records that strategy in its session meta (`SessionMeta::config`) and resumes
from the same home, with a fresh scratch, whether or not its run dir survived —
the conversation is in the home. Per-run mcps/skills/hooks/account from the
original run are **not** re-applied on resume — only the conversation itself.

There's also a direct, from-scratch form: `am <harness> --resume <id>` takes
a raw harness-native session id (no `am` session history lookup) and injects
the same native resume flag.

## I/O modes

The default is **passthrough**: `am` forwards the harness's terminal I/O directly,
making `am` invisible for interactive use. The `--io structured` mode (alias `--io jsonl`)
emits normalized `AgentEvent`s as NDJSON instead. Each harness supports both:

- **Claude Code**: passthrough (PTY); structured via stream-json NDJSON protocol.
- **Codex**: passthrough (PTY); structured via JSON-RPC over the app-server endpoint.
- **opencode**: passthrough (PTY); structured via NDJSON `opencode run --format json`.

Full I/O bridge details in [`io-modes.md`](./io-modes.md).

## Exit codes & passthrough fidelity

In passthrough mode `am` is meant to be invisible: it forwards the tty, forwards
signals, and **exits with the harness's own exit code**. A wrapper that swallows
Ctrl-C or rewrites the exit status would break scripts, so faithful passthrough
is a Phase-1 acceptance criterion (see [io-modes.md](./io-modes.md)).

### Config-dir retention for recorded sessions

Ephemeral config dirs are normally deleted after the run unless `--keep-config`
is given. When a session is actually being recorded (i.e. a sessions root is
configured and recording started successfully), the config dir is retained
regardless of `--keep-config` — `am session resume` needs it still on disk to
point the harness back at. Runs with no active recorder are unaffected and
keep the original ephemeral-cleanup behavior.
