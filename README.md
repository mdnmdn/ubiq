# Ubiq

A tmux-like harness multiplexer for hosting and orchestrating multiple interactive AI agent CLIs side by side.

## Supported Agents

- **Claude Code** (`claude`)
- **OpenCode** (`opencode`)
- **Gemini CLI** (`gemini`)
- **Codex** (`codex`)
- **Copilot CLI** (`copilot`)

## Tech Stack

- **Backend:** Rust + Tauri v2 + portable-pty
- **Frontend:** JavaScript + xterm.js + Vite
- **IPC:** Custom bus with serde-tagged message protocol
- **Harness library:** [`agent-manager`](crates/agent-manager/) — portable, harness-agnostic
  knowledge of where each agent stores its config and how to launch it

## Workspace layout

This repository is a Cargo workspace:

```
ubiq/
├── Cargo.toml              # workspace manifest
├── crates/
│   └── agent-manager/      # portable harness-management library (+ standalone CLI/TUI)
│       ├── src/            # config model, harness registry, project discovery, sync
│       └── _docs/          # per-harness reference material (config locations, formats)
└── src-tauri/             # the Ubiq Tauri app (depends on agent-manager)
```

`agent-manager` is the single source of truth for *which* harnesses exist, *how*
to launch them, and *where* their configuration lives. Ubiq's agent registry is
seeded from it; an optional `src-tauri/agents.toml` overrides or extends the
built-in definitions. See [`crates/agent-manager/_docs/`](crates/agent-manager/_docs/)
for the harness reference notes that drive future config-sync work.

## Prerequisites

- [Rust](https://rustup.rs/)
- [Node.js](https://nodejs.org/) (v18+)
- [Just](https://just.systems/) (task runner)
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

## Getting Started

```bash
# Install dependencies
just install

# Start development server
just dev
```

## License

[MIT](LICENSE)
