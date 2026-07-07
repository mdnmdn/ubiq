# Project structure

This document describes the Rust crate layout: what every file owns, what
every module exports, and how the library + binary are wired together.

## Crate shape

Single package, one library, one binary:

```
agent-manager/
├── Cargo.toml
│   ├── [lib]  name = "agent_manager", path = "src/lib.rs"
│   └── [[bin]] name = "agent-manager", path = "src/main.rs"
└── src/
    ├── lib.rs        # crate root, re-exports
    ├── main.rs       # thin shim: parse argv, dispatch, return
    ├── cli.rs        # clap definitions + Command::run dispatch
    ├── config.rs     # unified, harness-agnostic data model
    ├── harness.rs    # per-harness knowledge (paths, ids, capabilities)
    ├── project.rs    # discover + load a project
    ├── sync.rs       # UnifiedConfig -> one or more harnesses
    └── tui.rs        # ratatui front end
```

The `lib` name is `agent_manager` (snake_case, used in `use` statements).
The `bin` name is `agent-manager` (kebab-case, used on the command line and
as the produced executable).

## Module responsibilities

| Module      | Owns                                                                  | Does NOT own                                  |
|-------------|-----------------------------------------------------------------------|-----------------------------------------------|
| `config`    | Types: `UnifiedConfig`, `Rule`, `Skill`, `McpServer`, `Agent`         | Where they are written, what format          |
| `harness`   | `Harness` struct, `Harness::all`, `Harness::by_id`                   | Rendering logic, content generation          |
| `project`   | `Project::discover`, `Project::load`                                  | Sync, status reporting                        |
| `sync`      | `sync(config, targets) -> Vec<SyncReport>`                            | Discovering harnesses, user input            |
| `cli`       | `Args`, `Command`, `Command::run`                                     | Rendering, harness-specific knowledge        |
| `tui`       | `tui::run()` entry point, ratatui event loop                          | Sync engine, config parsing                   |

## Data flow at a glance

```
argv  ---> cli::Args::parse()
                 |
                 v
          cli::Command::run()
                 |
                 v
       +---------+---------+
       |                   |
       v                   v
  project::Project    tui::run()  --(user action)-->  sync::sync
       |                                                   |
       v                                                   v
  config::UnifiedConfig                            Vec<SyncReport>
```

## Conventions

- **`#![forbid(unsafe_code)]`** is set in `src/lib.rs`. Adding `unsafe` requires
  a `Cargo.toml` change with reviewer sign-off.
- **All public items are documented.** `missing_docs` is a `#![warn]` lint.
- **No I/O in pure modules.** `config` is a pure data module. Filesystem
  access lives in `project` and `sync`.
- **Errors bubble as `anyhow::Result`.** Library code may use `thiserror` to
  define structured error types, but the public API of every module is
  `anyhow::Result<T>` for simplicity at the binary boundary.
- **No panics in library code for recoverable errors.** `unwrap` is forbidden
  outside of tests.

## Build profiles

The release profile enables thin LTO and strips symbols to keep the produced
binary small; the dev profile is left at defaults for fast iteration.

```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
```

## Testing

Unit tests live next to the code they exercise (`#[cfg(test)] mod tests` in
the same file). Integration tests live in `tests/` and exercise the library
through its public API. Golden-file fixtures for harness renderers live in
`tests/fixtures/<harness>/`.

```bash
cargo test           # unit + integration
cargo test --doc     # doctests only
cargo clippy --all-targets -- -D warnings
```
