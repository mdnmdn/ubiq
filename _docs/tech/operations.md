---
id: tech-operations
title: Operations
kind: tech
status: current
summary: Prerequisites, the complete command reference, what a first build costs, the checks a change has to pass before it lands, and the runbook for a tool an agent cannot run.
read_when: you are setting the project up, running or testing it, adding a command, or an agent reports that it cannot run a tool
updated: 2026-09-11
verified: 2026-09-11
code_anchors: [Justfile, crates/ubiq-host/src/environment.rs, crates/agent-manager/src/isolate.rs, _tools/docs.py, _tools/icns.py, _tools/webassets.py, _tools/Info.plist, _devops/scripts/bundle-version.sh, crates/ubiq-app/src/lib.rs, crates/ubiq-host/src/remote.rs, crates/ubiq-app/build.rs, crates/ubiq-app/res/ubiq-app.rc, .github/workflows/create-release.yml, .github/workflows/release-macos.yml, .github/workflows/release-windows.yml]
depends_on: [tech-structure]
review_cycle: monthly
---

# Operations

## Prerequisites

| Tool | For | Notes |
|---|---|---|
| Rust toolchain | Everything | Edition 2024, so a recent stable |
| [`just`](https://just.systems) | Every command below | The Justfile is the command surface |
| [`uv`](https://docs.astral.sh/uv/) | The `_tools/` scripts | Each script declares its own dependencies inline; no environment to create |
| A C toolchain and system graphics libraries | GPUI | Xcode command line tools on macOS; on Linux, the X11 and Wayland development packages |

GPUI is pulled from git rather than a published crate, so the first build compiles Zed's rendering
stack from source. Expect it to take a long time and a lot of disk. Later builds are incremental.

## The command surface

**This document owns the command list.** `just` with no arguments prints it; a command that exists
only as a shell incantation in someone's history is a command that does not exist.

### Running

| Command | Does |
|---|---|
| `just dev` | Run Ubiq |
| `just verbose` | Run Ubiq with `RUST_LOG=debug`, which collects every subsystem at debug |
| `just build` | Release build of the whole workspace |
| `ubiq --serve` | Run a headless host listening for a remote UI on `0.0.0.0:7420` — no window |
| `ubiq --serve=<addr>` | Same, on a chosen address — a bare port, an `ip:port`, or an ip on the default port |
| `ubiq --bind <addr>` | The address to listen on, on its own or refining a `--serve` value |
| `ubiq --port <n>` | The port to listen on, likewise |

**A served run opens no window.** It is the machine's host, not a copy of the interface: it keeps
the terminal it was started in, reports through the same log writer on standard error that
`just dev` uses, and runs until it is stopped. It also takes no part in the one-application-per-root
handoff — handing its arguments to a window running elsewhere would leave nothing listening — so a
headless host and a local window can share a machine, though not usefully a config root.

**On Windows a plain launch leaves its console behind.** `ubiq` with no serve flag detaches
through `detach_console` in `crates/ubiq-app/src/lib.rs`, so a launch from Explorer or a shortcut
is a window and nothing else; a launch from a terminal keeps that terminal's window but reports
nothing more into it. Every `--serve` spelling stays attached, with the banner on stdout and the
log writer on standard error. A bootstrap failure in a detached run raises a dialog (`gui_fatal`)
rather than writing to a console that is gone. Other platforms are untouched.

`--serve` binds every interface, which is the point of the bare flag; `--serve=127.0.0.1:7420` is
how a tunnel-only setup is spelled. `--serve`'s own value is always attached with `=`, never a
separate argument — `argv_paths` skips a `-`-prefixed token without knowing the next one belongs to
it, so a separated value would be read as a project path. `--bind` and `--port` take either
spelling, because `argv_paths` consumes them by name, and each wins over the matching half of a
`--serve` value: `--serve --port 9000` is the short way to say `--serve=0.0.0.0:9000`. Either of
them alone also asks for a server — nothing else in Ubiq has a port to set. A bad value or a bind
failure prints an error and exits 2 rather than leaving something that looks like it worked and is
not listening. On success a banner goes to stdout, not through `tracing`, so no log filter can hide
it: the bind address, a generated token, and a `http://<ip>:<port>?token=<token>` string to paste
into a remote UI.

**Whoever holds that token gets a terminal on this machine, and the connection is not encrypted.**
There is no TLS and no other authentication — tunnel the connection (an SSH tunnel or similar) if
the network is not trusted. `crates/ubiq-host/src/remote.rs` is the listener;
[`architecture.md`](./architecture.md) covers the rule it follows, and [`../backlog.md`](../backlog.md)
(`G168`) covers what it lacks. A peer has ten seconds to finish its handshake and unlimited time
afterwards, so a port scan or a stalled dial costs one connection thread for ten seconds rather than
for the life of the process, and an attached window that says nothing all afternoon is left alone.

`RUST_LOG` decides what the log collector keeps, and the collector feeds both the log console and a
writer on standard error — so `just dev` in a terminal reports without the console being open. With
`RUST_LOG` unset, Ubiq's own modules and the harness library are collected down to debug and
everything else only when it complains. What the console does with the records is
[`../features/logs.md`](../features/logs.md).

### The harness library

| Command | Does |
|---|---|
| `just am <args>` | Run the `am` CLI — `just am claude --print-config` provisions a run and prints what it would launch, without launching it |
| `just host` | Build the host and prove no drawing crate reaches its dependency tree |
| `just ui` | Build the interface and prove it never names the host, and never names a type size of its own — a literal `text_size(px(N))` outside `theme.rs` fails it |
| `just core` | Build the library the way Ubiq consumes it, with default features off. **This is the check that matters** — it fails the moment a CLI or terminal type leaks into the core |

### Checks

| Command | Does |
|---|---|
| `just check` | Type-check the workspace, tests and examples included |
| `just clippy` | Lint, warnings as errors |
| `just fmt` | Format |
| `just test` | Test the workspace with stdin closed |
| `just verify` | `check`, `clippy`, `test`, `host`, `ui`, `docs-lint` — what a change has to pass |

`just test` closes stdin deliberately. The library's passthrough tests spawn real pseudo-terminals,
and an interactive stdin makes them hang rather than fail, which is the worse of the two outcomes.
The application's own tests drive the coordinator over the bus and start real processes in
pseudo-terminals for the same reason; they need no display.

### Packaging

| Command | Does |
|---|---|
| `just icns` | Build `target/AppIcon.icns` from the logo in `assets/` — the ten representations an `.iconset` needs, assembled by `iconutil` |
| `just bundle` | Assemble `target/Ubiq.app`: release build of the binary, `AppIcon.icns`, and `_tools/Info.plist`. macOS-only, unsigned, for a local `.app` |
| `just bundle-win` | Assemble `target/ubiq-windows-x86_64/`: the release build's `ubiq.exe`, with `res/AppIcon.ico` embedded as its icon. Windows-only |

`just bundle` requires macOS and Xcode's command line tools for `iconutil`. The bundle is unsigned
and carries no hardened-runtime entitlements, so it launches locally but is not ready to distribute.

`just bundle-win` is the Windows counterpart — no icon-assembly step, because the icon is not a side
file: `crates/ubiq-app/build.rs` links `res/AppIcon.ico` into the executable through
`res/ubiq-app.rc`, using `windres` on the GNU toolchain and `rc.exe` on MSVC, and the `cfg(windows)`
gate keeps it out of macOS builds. `AppIcon.ico` is generated from `assets/logo-white-on-blue.png`
at 16 through 256 pixels. Like every recipe in this file it expects a POSIX shell on the path (Git
Bash, which a Windows runner carries).

A tag matching `v*` runs `.github/workflows/release-macos.yml` and
`.github/workflows/release-windows.yml` together: each workflow assembles its platform bundle and
attaches the zip to the GitHub release of that tag. `.github/workflows/create-release.yml` is the
manual counterpart — a `workflow_dispatch` whose `platforms` choice is `both`, `macos` or `windows`
— and it calls those two workflows as reusable jobs so one run ships either platform or both. Each
platform workflow also accepts a direct `workflow_dispatch` of its own.

### Documentation

| Command | Does |
|---|---|
| `just docs-lint [paths]` | The mechanical checks over `_docs/` |
| `just docs-touched [paths]` | Which documents your change owes an update. No arguments reads the working diff |
| `just docs-index` | Regenerate the catalogue in the index and the tree in the code map |
| `just docs-check` | Fail if a generated block is stale, without writing |
| `just docs-drift` | Documents whose anchored files moved after they were last verified |
| `just docs-graph` | The `depends_on` graph: roots, isolated documents, over-connected hubs |
| `just diagram <source>` | Render one diagram source to PNG |

What the check ids mean, and what to do about each, is in `_docs/_meta/librarian.md`.

### Web assets

| Command | Does |
|---|---|
| `just web-assets [args]` | Re-snapshot the Excalidraw mirror from jsDelivr and rewrite the host's hash manifest |
| `just web-assets-verify` | Re-fetch every file in that manifest and report CDN drift |

The snapshot is taken when the pinned bundle version changes, never at runtime, and downloads land in
a scratch directory outside the tree. `web-assets-verify` is the cheap check for the one event worth
failing on: jsDelivr regenerating a `+esm` bundle under a hash the manifest is committed to.

### Housekeeping

`just clean` removes build output, `just update` updates dependencies, `just audit` checks them for
advisories.

## Before a change lands

1. `just verify` passes.
2. `just docs-touched` names no document you left unverified.
3. Anything unresolved has a line in [`../backlog.md`](../backlog.md).

The order matters: a green build with stale documentation is the failure mode this whole library
exists to prevent.

## Adding a command

Add the recipe to the Justfile with a one-line comment above it — `just --list` prints those
comments, so the comment is the help text. Then add its row to the table above, in the same commit.
A script under `_tools/` is always fronted by a recipe; nobody should need to remember its path or
its arguments.

## Environment

Ubiq resolves one config root, and everything it remembers between runs lives under it — the
project catalogue, per-project view state, and the caches. The order is `--config-root`, then
`UBIQ_CONFIG_DIR`, then the nearest `ubiq.toml` walking up from the working directory, then
`~/.config/ubiq`. A `ubiq.toml` is a bootstrap file and says one thing, `config_root`; this
repository commits one pointing at `_data/config`, which is ignored by git, so running from a
checkout never touches the catalogue a user works with all day. A malformed one is an error rather
than a fallback, and the status bar says when the root is not the default.

Four variables affect a run:

| Variable | Effect |
|---|---|
| `RUST_LOG` | Log filter, through `tracing-subscriber`. `just verbose` sets it to `debug` |
| `CARGO_TARGET_DIR` | Redirects build output, useful when disk is tight |
| `UBIQ_CONFIG_DIR` | The config root, below `--config-root` and above a `ubiq.toml` |
| `UBIQ_VERSION` | The bundle version baked into the binary. Set it to pin a build to a known string; left unset, the Justfile computes one |

`UBIQ_VERSION` is exported near the top of the Justfile as `` `_devops/scripts/bundle-version.sh` ``,
so every recipe's cargo invocations carry it into `option_env!("UBIQ_VERSION")` at compile time —
see [`architecture.md`](./architecture.md) for the `version` module that reads it. The script's
precedence: `UBIQ_VERSION` set in the environment, taken verbatim; else the git tag starting with `v`
on a clean, tagged `HEAD`; else, with no git available, `dev-<cargo version>-<UTC timestamp>`; else
`dev-<UTC timestamp>-<8-char git hash>`.

Harness configuration is the embedded library's business, including every environment variable it
sets for a run. Those live with that crate — see [`agent-manager.md`](./agent-manager.md).

## When an agent cannot run a tool

An agent reporting `cargo: operation not permitted`, `python: operation not permitted` or an `ls`
that cannot list a directory the user can list is the commonest report Ubiq gets, and it is almost
never a broken installation. It is the confinement policy answering a question nobody told it about:
the shipped layers grant a toolchain's **default** location, and this machine put it somewhere else.
The runbook below takes about two minutes and ends in a line in `environment.toml`.

### Read the failure first

The wording says which half of the problem it is, and they have opposite fixes.

| What the agent saw | What it means | Where the fix is |
|---|---|---|
| `operation not permitted`, from the shell or from `ls` | The policy denied the path. The binary is there and the run cannot reach it | `environment.toml`, below |
| `command not found` | Nothing was denied — the name is not on the run's `PATH` | `PATH` in `environment.toml`, or `agent_commands` in Settings for a harness binary |
| `cannot find GOROOT`, `DOTNET_CLI_HOME not set`, `.. is not a directory` | The tool ran and could not find its own root. A variable is missing, not a grant | The `[env]` table |
| The harness hangs on its splash screen, with no error | A denied lookup the harness blocks on, not a path | `DEV_LAYERS` in `crates/agent-manager/src/isolate.rs` |
| A confined `swift` or `xcodebuild` is denied | isol8's `integrations/xcode` layer is in `BROKEN_LAYERS`, so the SDK and toolchain paths are named by hand | `APPLE_SDK_RO_ROOTS` / `APPLE_RW_HOME_ROOTS` in `crates/agent-manager/src/isolate.rs` |
| `swift build` fails even with the SDK granted | SwiftPM shells out to `sandbox-exec` itself, and a sandbox cannot nest | Run it unconfined, with `--disable-sandbox` |

Confirm the run is confined before anything else: `env | grep ISOL8_SANDBOXED` inside the pane
prints `ISOL8_SANDBOXED=1` when it is. Ubiq also logs `confined` for the spawn.

### Ask the tool what it needs, outside the pane

**A sandbox cannot nest, so nothing inside a confined pane can test a policy.** Every command in
this step belongs in an ordinary terminal, in a login shell — which is also the environment that
still has the variables a GUI launch of Ubiq lost.

```
which -a cargo                 # every path the name resolves through
readlink -f "$(which cargo)"   # a shim's real target: isol8 does not follow it for you
env | grep -E 'CARGO_HOME|RUSTUP_HOME|PYENV_ROOT|GO(ROOT|PATH|MODCACHE)|JAVA_HOME|SDKMAN_DIR'
```

Three answers matter: where the binary is, where the tool **writes** — a registry, a module cache, a
package store — and which variable names that root. Reaching a binary is rarely what is missing:
every absolute directory on the run's `PATH` carries a read-only grant. What a build writes to is
what has to be named.

### Name it in `environment.toml`

`<config root>/environment.toml`, the machine's own description —
[`agent-manager.md`](./agent-manager.md) owns the format, and this is the decision it turns on:

- **The tool has a standard variable** — `CARGO_HOME`, `RUSTUP_HOME`, `GOROOT`, `GOPATH`,
  `GOMODCACHE`, `PYENV_ROOT`, `MISE_DATA_DIR`, `NPM_CONFIG_PREFIX`, `JAVA_HOME`, `DOTNET_ROOT`,
  `PUB_CACHE` and their siblings in `TOOLCHAIN_ROOT_VARS`. Put it in `[env]` with an **absolute**
  path. That is one line for both halves of the answer: the run gets the variable, and the isolate
  stage grants what it names, writable or not according to that table.
- **It has none** — a Flutter SDK, a vendored toolchain, a shared cache on another volume. Add a
  `[[grants]]` block with `write = true` only when the tool genuinely writes there. Flutter does,
  into its own `bin/cache`; an SDK tree usually does not.

Then restart Ubiq: the file is read once, at startup, like every other policy input. A grant on a
path that does not exist yet is inert rather than an error, so a cache root can be granted before
its first use — which is exactly what makes the first run of a tool work.

### When it is still denied

Reach for the policy itself rather than for another grant.

```
isol8 @diag -- cargo --version                    # what a denial was, in isol8's own words
isol8 --show-policies -- cargo --version | less   # the merged layer stack and every rendered grant
```

Two failures look like a missing grant and are not. A **shim pointing outside** a granted tree needs
its target granted too — isol8 grants a subtree, it does not follow a symlink out of one. A **layer
in `BROKEN_LAYERS`** takes the whole policy down rather than its own feature, so a run that stopped
starting at all after a layer was added is that, and `crates/agent-manager/src/isolate.rs` documents
each one.

Turning isolation off in Settings is a diagnosis, not a fix: it says the policy is the cause, and the
answer is still a line in `environment.toml`. Leaving it off gives every agent the whole machine.

## Related docs

- [`project-structure.md`](./project-structure.md) — what each folder and crate is
- [`agent-manager.md`](./agent-manager.md) — the library's own commands and configuration
- `_docs/_meta/authoring.md` — the documentation duty `docs-touched` enforces
