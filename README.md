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
