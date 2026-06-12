# Project Structure

## Overview

Ubiq is a Tauri v2 application with a JavaScript/Vite frontend and a Rust backend. The build system coordinates both sides through `cargo tauri` commands.

## Directory Layout

```
ubiq/
├── src/                    # Frontend source (JS + xterm.js)
│   └── main.js
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── main.rs         # Entry point
│   │   ├── lib.rs          # Tauri commands and app setup
│   │   └── coordinator.rs  # PTY management
│   ├── icons/              # App icons (generated via `npx tauri icon`)
│   ├── tauri.conf.json     # Tauri config (window, bundle, build hooks)
│   ├── build.rs           # Tauri build script
│   └── Cargo.toml         # Rust dependencies
├── index.html             # HTML shell (loads /src/main.js)
├── vite.config.js         # Vite dev server config
├── package.json           # npm scripts and JS deps
├── Justfile               # Task runner recipes
└── dist/                  # Vite build output (served to Tauri)
```

## Build Pipeline

### `just dev` (Development)

1. `cargo tauri dev` kicks off both frontend and backend
2. **Frontend**: Tauri runs `npm run dev` → Vite starts a dev server on `localhost:5173`
3. **Backend**: Cargo compiles `src-tauri/` and runs the binary
4. Tauri loads `http://localhost:5173` in a native webview
5. Vite watches `src/` for changes (hot reload); Cargo watches `src-tauri/` for changes (auto-recompile)

### `just build` (Production)

1. `cargo tauri build` kicks off both sides
2. **Frontend**: Tauri runs `npm run build` → Vite bundles `src/` into `dist/`
3. **Backend**: Cargo compiles `src-tauri/` in release mode
4. Tauri embeds `dist/` into the final binary (via `frontendDist` in tauri.conf.json)
5. Tauri bundles the app for the current platform (`.app` on macOS, `.exe` on Windows, AppImage on Linux)

## Key Build Configs

### `tauri.conf.json`

- **`build.beforeDevCommand`**: `npm run dev` — starts Vite before the backend compiles
- **`build.beforeBuildCommand`**: `npm run build` — builds frontend before bundling
- **`build.devUrl`**: `http://localhost:5173` — where the webview points in dev mode
- **`build.frontendDist`**: `../dist` — path to Vite output, embedded in the release binary
- **`bundle.icon`**: icon files for platform-specific packaging

### `vite.config.js`

- `clearScreen: false` — keeps terminal output readable alongside Cargo output
- `server.port: 5173` / `strictPort: true` — fixed port so Tauri can find it
- `server.watch.ignored: ['**/src-tauri/**']` — prevents Vite from watching Rust source

### `Cargo.toml`

- `crate-type = ["staticlib", "cdylib", "rlib"]` — builds for Tauri's embedding (cdylib) and as a library (rlib)
- Key deps: `tauri`, `portable-pty`, `tokio`, `uuid`, `serde`

## Justfile Recipes

| Command              | What it does                                      |
| -------------------- | ------------------------------------------------- |
| `just dev`           | Full dev mode (Vite + Cargo + webview)            |
| `just build`         | Production build for current platform             |
| `just build-frontend`| Vite only (no Rust)                               |
| `just check`         | `cargo check` (fast compile check)                |
| `just clippy`        | Rust linting                                      |
| `just format-rust`   | `cargo fmt`                                       |
| `just test`          | Run Rust and JS tests                             |
| `just clean`         | Remove `target/`, `dist/`, `node_modules/`        |
| `just generate-icons`| Generate platform icons from a source PNG         |

## How the Two Halves Connect

- **Frontend → Backend**: JS calls Tauri commands via `invoke()` from `@tauri-apps/api`
- **Backend → Frontend**: Rust emits events via `app.emit()` that JS listens for
- **Dev mode**: Vite serves the frontend; Cargo compiles and runs the backend; Tauri bridges them in a native webview
- **Production**: Vite output is embedded in the Rust binary; no separate process needed
