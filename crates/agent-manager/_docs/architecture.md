# Architecture

## High-level pipeline

```
                  +--------------------+
   .agent-manager |  UnifiedConfig     |  <-- single source of truth
       .toml ---->|  (config.rs)       |
                  +---------+----------+
                            |
                            v
                  +---------+----------+
                  |   sync engine      |  <-- idempotent
                  |   (sync.rs)        |
                  +---------+----------+
                            |
        +---------+---------+---------+---------+
        v                   v                   v
   +----+----+         +----+----+         +----+----+
   | Claude  |         |  Codex  |   ...   | opencode|
   |  Code   |         |         |         |         |
   +---------+         +---------+         +---------+
```

The library is the engine. The binary and the TUI are two different drivers
sitting on top of the same engine.

## Modules

| Module      | Responsibility                                              |
|-------------|-------------------------------------------------------------|
| `config`    | The unified, harness-agnostic data model. Pure types.       |
| `harness`   | Knowledge of each concrete harness: paths, format quirks.   |
| `project`   | Discover a project root, load its `.agent-manager.toml`.    |
| `sync`      | Project a `UnifiedConfig` onto one or more harnesses.       |
| `cli`       | `clap` subcommand definitions and dispatch.                 |
| `tui`       | `ratatui` interactive front end.                            |

## Sync invariants

- **Idempotent.** Re-running `agent-manager sync` after no source-side change
  is a no-op (no touched files, no churn).
- **Source wins.** Anything in the unified config is authoritative for the
  files it owns. Anything in a harness directory that is *not* produced by
  `agent-manager` is left alone.
- **Per-harness isolation.** A sync to harness A never writes to harness B's
  files, even if the file formats are similar.
- **Failure isolation.** If a sync to harness B fails, harnesses A and C are
  still considered done.

## Adding a new harness

1. Add a new file under `_docs/harness/<id>.md` describing the harness's
   on-disk layout, supported features, and format quirks.
2. Add an entry to the `Harness::all()` list in `src/harness.rs`.
3. Extend `sync::sync` with the renderers for that harness's features.
4. Add at least one golden test under `tests/` for the new harness's
   rendered output.
