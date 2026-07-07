# CLI surface (`am`)

> The binary is still built as `agent-manager`; `am` is the intended short
> alias (installed name / symlink). This doc uses `am`.

## Command shape

`am` has a small set of **reserved subcommands** for managing the tool itself,
and otherwise treats the first positional as a **harness name** to wrap:

```
am <harness> [am-flags] [-- harness-args…]     # wrap & run a harness
am catalog   <ls|import|show|path> …            # manage the catalog
am account   <ls|use|import> …                  # manage accounts        (P2)
am session   <ls|show|resume> …                 # manage session history  (P2/P3)
am help | am --version
```

Reserved words (`catalog`, `account`, `session`, `help`) are checked before
harness resolution. A harness id is never one of these, so there is no
collision. Unknown first-positional → looked up in the harness registry; if it
is not a known harness, error with the list of known ids.

## Running a harness

```bash
am claude --mcps postgres,figma --skills web-designer --safe
am codex  --skills reviewer --account work
am opencode --config ./run.toml
```

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
| `--print-config`         | Provision only; print the generated dir + argv + env; don't launch. |
| `--account <id>`         | Account/credential profile to use.                       (P2)       |
| `--instructions <path>`  | Seed always-on instructions.                             (P2)       |
| `--prompt <text>`        | Seed an initial prompt for the run.                      (P2)       |
| `--isolate[=profile]`    | Run inside an isol8 sandbox.                             (P3)       |
| `--io <mode>`            | Abstracted I/O instead of passthrough.                   (P2/P3)    |
| `-- <harness-args…>`     | Everything after `--` is forwarded verbatim to the harness binary.  |

*(1)* `--safe` is a named **preset** resolved from the settings file / built-in
defaults, not a hard-coded flag list — so teams can define what "safe" means.

Anything `am` doesn't recognize after `--` is the harness's own CLI (e.g.
`am claude -- --model opus -p`). This keeps `am` from having to mirror every
harness flag.

## Settings file + flag merge

Configuration is a **mix** of the settings file and CLI flags. Precedence,
highest first:

1. **CLI flags** (`--mcps`, `--account`, …).
2. **`--config <path>`** if given, else the **discovered** settings file.
3. **Environment** (`AM_CATALOG`, `AM_CONFIG`, …).
4. **Built-in defaults.**

### Discovery order for the settings file

`./.agent-manager.toml` (or `.yaml`) in the CWD, walking up to the git root,
then `~/.config/agent-manager/config.toml`. First found wins as the base; CLI
flags layer on top. (This mirrors the harness `CLAUDE.md` walk, so it feels
familiar.)

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
```

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

## Exit codes & passthrough fidelity

In passthrough mode `am` is meant to be invisible: it forwards the tty, forwards
signals, and **exits with the harness's own exit code**. A wrapper that swallows
Ctrl-C or rewrites the exit status would break scripts, so faithful passthrough
is a Phase-1 acceptance criterion (see [io-modes.md](./io-modes.md)).
