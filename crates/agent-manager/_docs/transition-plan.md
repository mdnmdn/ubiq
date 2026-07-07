# Transition plan — from config-sync to agent-runtime wrapper

> **Purpose.** This document is the bridge between where the code is *today*
> (the config-sync skeleton described in [`old/`](./old/)) and **Phase 1** of the
> target design (the agent-runtime wrapper described in [`target/`](./target/)).
> It is a plan, not an implementation — nothing here is code yet.

## 1. Where we are today

The crate is a pre-alpha **config-sync** skeleton:

- `config.rs` — `UnifiedConfig` + `Rule` / `Skill` / `McpServer` / `Agent` /
  `McpTransport`. **Real, working types.**
- `harness.rs` — intended to hold per-harness static facts. **Stub.**
- `project.rs` — discover/load a `.agent-manager.toml`. **Stub.**
- `sync.rs` — render the unified config into each harness's real dirs. **Stub.**
- `cli.rs` — `status` / `sync` / `tui` / `inspect` subcommands. **Stubs that
  print "not yet implemented".**
- `tui.rs` — ratatui front end. **Stub.**
- `_docs/harness/*` — per-harness contracts including the **runtime** launch
  seams. **Rich and current.**
- Cargo features `frontend` / `cli` / `tui` already split the front ends from a
  reusable core — the lib-mode foundation is in place.

**The important fact:** almost nothing real has been built yet, so the pivot is
cheap. The valuable assets are the *resource types* (`config.rs`) and the
*harness runtime docs* (`_docs/harness/`), and both survive.

## 2. What survives, what is repurposed, what is retired

| Today                                   | Fate      | Becomes / notes                                                                 |
|-----------------------------------------|-----------|---------------------------------------------------------------------------------|
| `config.rs` resource types              | **survive** | `Skill` / `McpServer` / `McpTransport` feed the catalog + `RunSpec`. `Rule`/`Agent` stay for instructions/personas. |
| `UnifiedConfig` (top-level)             | **reshape** | Splits into *settings file* (`settings.rs`) + *catalog* (`registry/`) + *run plan* (`spec.rs`). |
| `harness.rs` (static `Harness` struct)  | **repurpose** | Becomes the `Harness` **trait** with `provision()` / `io_support()`.          |
| `sync.rs` (render → user dirs)          | **repurpose** | The *renderers* become `provision.rs` (render → **ephemeral** dir + launch). The *write-to-`~/.claude`* behavior is **retired** as the primary path. |
| `project.rs` (discover `.agent-manager.toml`) | **repurpose** | Becomes settings-file discovery in `settings.rs` (the walk-up logic is reusable). |
| `cli.rs` (`status`/`sync`/`inspect`)    | **replace** | New surface: `am <harness>` + `am catalog …` (see [target/cli.md](./target/cli.md)). |
| `tui.rs`                                | **park**  | Keep the stub behind the `tui` feature; not on the Phase-1 path.               |
| `_docs/harness/*`                       | **keep**  | Already documents launch flags / stream protocol / injection seams. Authoritative. |
| `_docs/architecture|config-format|project-structure.md` | **archived** | Moved to [`old/`](./old/).                                    |

The one genuinely retired idea is **"sync into the user's real config dirs as
the product's purpose."** It may return as an optional `am config apply`, but it
leaves the critical path.

## 3. The key insight that makes the transition cheap

`provision` (new) ≈ `sync` renderer (old) with two changes:

1. **Target dir:** write to an *ephemeral* run dir, not `~/.claude`.
2. **Then launch:** hand the dir + argv + env to `run`, instead of stopping.

So the hard part of the old design that was never finished — "turn a set of MCP
servers / skills into the exact shape a given harness wants" — is the *same*
work, and it is already **spec'd in `_docs/harness/`** (e.g. the "MCP at launch"
and "Skills at launch" sections of `claude-code.md`). We are not throwing away a
finished renderer; we are pointing an *unfinished* one at a different directory
and adding a launch step. That is the whole reason the pivot is low-cost.

## 4. Phase-1 work breakdown

Ordered so each step compiles and is testable. Each is a candidate for one
session.

### Step 0 — docs & scaffolding (this change)
- Archive old docs → `old/`; write `target/` + this plan. ✅ (this PR)
- No code yet.

### Step 1 — core model (`spec.rs`)
- Introduce `RunSpec`, `HarnessId`, `SkillRef`, `McpRef` (incl. `InProcess`
  variant, unused until P2), `ConfigStrategy`, `IoModes` (one variant:
  `Passthrough`).
- Reuse `McpServer` / `Skill` from `config.rs` (rename module later if desired).
- Pure types, no I/O. Unit-testable.

### Step 2 — catalog (`registry/`)
- `Registry` trait + `FsRegistry` (reads `catalog.toml` + `mcp/*.json` +
  `skills/*/`).
- `am catalog ls` / `am catalog show` / `am catalog path`.
- Golden tests on a fixture catalog under `tests/fixtures/catalog/`.

### Step 3 — settings + resolve (`settings.rs`, `resolve.rs`)
- Load/merge the toml|yaml settings file (reuse `project.rs` walk-up).
- `resolve(flags, settings, registry) -> RunSpec`.
- Pin down the **merge semantics open question** (additive vs replace) here.

### Step 4 — harness trait + Claude provisioner (`harness/`, `provision.rs`)
- `Harness` trait; `harness/claude.rs` transcribing `_docs/harness/claude-code.md`
  (temp workdir with `.claude/skills/…`, `--mcp-config` + `--strict-mcp-config`,
  env hygiene).
- `provision(spec) -> (ephemeral_dir, Launch)`.
- `am claude --print-config` prints the generated dir + argv + env (no launch) —
  the first user-visible, testable milestone.

### Step 5 — runner (`run.rs`, `io/passthrough.rs`)
- PTY spawn (reuse `portable-pty`, already used elsewhere in the Ubiq
  workspace), passthrough stdin/stdout/stderr, signal + resize forwarding,
  child exit-code propagation.
- Ephemeral-dir lifecycle (create → run → clean up; `--keep-config` to retain).

### Step 6 — `catalog import`
- Read-only ingest of `~/.claude` / `~/.agent` / project dirs into the catalog.
- `--dry-run`, collision handling.

At the end of Step 5 the Phase-1 headline works end-to-end for Claude Code;
Step 6 makes it adoptable. codex / opencode are added by writing more `Harness`
impls, no core change.

## 5. Sequencing against the current stubs

- **Do not delete** `sync.rs` / `harness.rs` / `project.rs` up front. Port their
  reusable logic into the new modules, then remove the emptied stubs in the same
  step that replaces them, so the tree never has two half-things doing the same
  job.
- **`cli.rs` is replaced wholesale** — the `status`/`sync`/`inspect` commands
  don't map to the new surface. Land the new `cli/` alongside, then delete the
  old `cli.rs`.
- **`tui.rs` stays parked** behind its feature flag; revisit only if/when the
  web/TUI surface (P3+) needs it.
- **Cargo features:** keep `frontend`/`cli`/`tui`. Add `pty` (or fold into
  `cli`) for the runner. The core (`spec`/`resolve`/`registry`/`harness`/
  `provision`) must build with `--no-default-features` for lib mode.

## 6. Risks & watch-items

- **Ephemeral-dir correctness.** The single most important invariant is *never
  write to the user's real harness dirs during a run*. Add a test that runs a
  provision against a fake `$HOME` and asserts it stayed empty.
- **Passthrough fidelity.** Signals, resize, and exit codes are easy to get
  subtly wrong and hard to notice. Treat them as explicit acceptance criteria.
- **Merge-semantics decision.** Flag/settings additive-vs-replace (Step 3) is a
  small decision with large ergonomics consequences; pin it before building
  `resolve`, not after.
- **Harness drift.** The provisioner is only as correct as `_docs/harness/*`.
  When a harness changes its launch contract, the doc is the thing to update
  first, then the `Harness` impl.

## 7. Decisions & remaining open questions

**Decided (2026-07):**

1. **Binary name.** Keep the `[[bin]]` as `agent-manager`; ship `am` as an
   alias/symlink. No `Cargo.toml` bin rename.
2. **Settings merge semantics — replace, not additive.** A higher-precedence
   layer (CLI flag > per-harness config > defaults) *replaces* the same key; it
   does not union. Extending means listing the full set. An `--add-*` sugar may
   come later. (Pins Step 3; see [target/cli.md](./target/cli.md).)
3. **Catalog scope — global + project overlay.** A global catalog plus an
   optional `<project>/.agent-manager/catalog` that layers on top; project
   entries add new ids or override global ids of the same name (project wins).
   Implemented as `OverlayRegistry(global, project)`. (See
   [target/registry.md](./target/registry.md).)
4. **Reserved subcommand vs harness shape.** `am <harness>` (no `run` verb), with
   `catalog`/`account`/`session` reserved. Matches the spec examples.

**Still open:**

5. **Config-format file name.** Keep `.agent-manager.toml` for the *settings*
   file, or pick a new name to avoid confusion with the retired sync config?
6. **`--add-*` sugar.** Whether/when to add `--add-mcps` / `--add-skills` on top
   of the replace-by-default base.
