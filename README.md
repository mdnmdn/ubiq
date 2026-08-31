# Ubiq

A harness multiplexer — tmux for AI coding agents. Ubiq hosts several interactive agent CLIs side by
side, each in a real terminal pane, under one window and one set of controls.

Every harness Ubiq hosts is a full-screen terminal program: it takes over the alternate screen,
addresses the cursor absolutely, and expects raw keystrokes back. So a pane is a genuine terminal
and a harness runs under a genuine pseudo-terminal — Ubiq shuttles bytes between the two and adds
the multiplexing, the configuration and the window around it.

## Supported agents

| Agent | Command |
|---|---|
| Claude Code | `claude` |
| Codex | `codex` |
| opencode | `opencode` |
| Gemini CLI | `gemini` |
| GitHub Copilot CLI | `copilot` |

## Tech stack

- **Application** — Rust + [GPUI](https://www.gpui.rs/), with `gpui-component` for widgets
- **Terminals** — `portable-pty` for pseudo-terminals, and `gpui-terminal` (vendored, under
  `vendor/`) for the emulator each pane is drawn by
- **Harness management** — [`agent-manager`](crates/agent-manager/), the portable library that
  composes and launches a harness run

## Workspace layout

```
ubiq/
├── Cargo.toml            workspace manifest
├── Justfile              every command anyone runs
├── crates/
│   ├── ubiq/             the desktop application
│   └── agent-manager/    harness-management library, plus its `am` CLI
├── vendor/               third-party crates carried in-tree, kept close to upstream
├── _docs/                documentation — start at _docs/INDEX.md
└── _tools/               dev-only scripts, run through `just`
```

`agent-manager` is the single source of truth for *which* harnesses exist, *how* to launch them, and
*where* their configuration lives. It composes a run — skills, MCP servers, an account, instructions
— into a throwaway config directory and launches the real binary against it, leaving your own
`~/.claude` and its siblings untouched. It ships its own CLI and its own documentation; see
[`crates/agent-manager/_docs/`](crates/agent-manager/_docs/).

## Getting started

Prerequisites: a recent Rust stable toolchain, [`just`](https://just.systems),
[`uv`](https://docs.astral.sh/uv/) for the tooling scripts, and a C toolchain with system graphics
libraries for GPUI.

```bash
just dev      # run Ubiq
just verify   # check, clippy, test, docs-lint
just          # list every recipe
```

GPUI is pulled from git, so the first build compiles Zed's rendering stack from source. Expect it to
take a while.

## Documentation

Start at [`_docs/INDEX.md`](_docs/INDEX.md) — it maps the library and routes you to the two or three
documents any given task needs. [`AGENTS.md`](AGENTS.md) is the always-loaded preamble for agents
working on the code.

## Status

Alpha, under active development.

## License

Ubiq is licensed under the [Sustainable Use License](LICENSE) (fair-code, source-available).

You may use and modify Ubiq for **personal use** or **internal business purposes** inside your
organization. You may not host Ubiq (or a substantially similar product) as a paid service,
white-label it, or sell a product whose value derives substantially from Ubiq — unless you have a
separate commercial agreement with the licensor.

**Allowed**

- Personal use on your own machines
- Running Ubiq internally for your company's engineering team
- Modifying Ubiq for your own internal needs
- Paid consulting or support (setup, customization, workflows) for organizations using Ubiq
  internally

**Not allowed** (without a commercial license)

- Charging users to access a hosted multiplexer built on Ubiq
- Selling or distributing a rebranded Ubiq (or close substitute) as your product

Contributors are asked to sign the
[Contributor License Agreement](CONTRIBUTOR_LICENSE_AGREEMENT.md). For commercial hosting,
embedding, or resale rights, contact the licensor.
