---
id: tech-operations
title: Operations
kind: tech
status: current
summary: Prerequisites, the complete command reference, what a first build costs, and the checks a change has to pass before it lands.
read_when: you are setting the project up, running or testing it, or adding a command
updated: 2026-09-04
verified: 2026-09-04
code_anchors: [Justfile, _tools/docs.py, _tools/icns.py, _tools/Info.plist, _devops/scripts/bundle-version.sh]
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
| `just ui` | Build the interface and prove it never names the host |
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

`just bundle` requires macOS and Xcode's command line tools for `iconutil`. The bundle is unsigned
and carries no hardened-runtime entitlements, so it launches locally but is not ready to distribute.

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
precedence: `UBIQ_VERSION` already in the environment, verbatim; else the git tag starting with `v`
on a clean, tagged `HEAD`; else, with no git available, `dev-<cargo version>-<UTC timestamp>`; else
`dev-<UTC timestamp>-<8-char git hash>`.

Harness configuration is the embedded library's business, including every environment variable it
sets for a run. Those live with that crate — see [`agent-manager.md`](./agent-manager.md).

## Related docs

- [`project-structure.md`](./project-structure.md) — what each folder and crate is
- [`agent-manager.md`](./agent-manager.md) — the library's own commands and configuration
- `_docs/_meta/authoring.md` — the documentation duty `docs-touched` enforces
