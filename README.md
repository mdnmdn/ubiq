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

Ubiq is licensed under the [Sustainable Use License](LICENSE) (fair-code, source-available).

You may use and modify Ubiq for **personal use** or **internal business purposes** inside your organization. You may not host Ubiq (or a substantially similar product) as a paid service, white-label it, or sell a product whose value derives substantially from Ubiq — unless you have a separate commercial agreement with the licensor.

**Allowed**

- Personal use on your own machines
- Running Ubiq internally for your company's engineering team
- Modifying Ubiq for your own internal needs
- Paid consulting or support (setup, customization, workflows) for organizations using Ubiq internally

**Not allowed** (without a commercial license)

- Charging users to access a hosted multiplexer built on Ubiq
- Selling or distributing a rebranded Ubiq (or close substitute) as your product

Contributors are asked to sign the [Contributor License Agreement](CONTRIBUTOR_LICENSE_AGREEMENT.md). For commercial hosting, embedding, or resale rights, contact the licensor.
