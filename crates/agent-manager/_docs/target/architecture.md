# Architecture (target design)

## The runtime pipeline

A single `am claude …` invocation (or one lib-mode `run()` call) flows through
six stages. Every stage is a library module; the CLI and any TUI are thin
drivers on top.

```
  flags + config file            catalog (registry)
        │                              │
        ▼                              ▼
   ┌─────────┐    resolve ids    ┌──────────┐
   │ resolve │ ────────────────▶ │ RunSpec  │   the fully-resolved, harness-agnostic plan
   └─────────┘                   └────┬─────┘
                                      │
                                      ▼
                                ┌───────────┐   turn the RunSpec into the harness's
                                │ provision │   native config, written to an EPHEMERAL dir
                                └────┬──────┘
                                      │  (ephemeral config dir + launch argv + env)
                                      ▼
                                ┌───────────┐   optional: wrap the launch in an
                                │ isolate   │   isol8 sandbox
                                └────┬──────┘
                                      │
                                      ▼
                                ┌───────────┐   spawn the real harness binary,
                                │  run      │   supervise it, own its lifecycle
                                └────┬──────┘
                                      │
                        ┌─────────────┴─────────────┐
                        ▼                            ▼
                 ┌────────────┐               ┌────────────┐
                 │  io bridge │               │  session   │  record transcript,
                 │ (passthru/ │               │  store     │  exit code, metadata
                 │  ACP/JSONL)│               └────────────┘
                 └────────────┘
```

The **`resolve → RunSpec → provision`** spine is the heart of the design. Note
that `provision` is exactly the old sync engine's *renderer*, re-pointed: it
still turns a set of MCP servers into the JSON shape Claude Code wants, but it
writes into a throwaway directory and hands the path to `run`, instead of
writing into `~/.claude` and stopping. See the transition plan for how the old
code is repurposed.

## The core model: `RunSpec`

`RunSpec` is the fully-resolved, harness-agnostic description of one run. It is
the boundary between "figuring out what to run" (resolve) and "actually running
it" (provision + run). In lib mode an embedder builds it directly; in CLI mode
`resolve` builds it from flags + config + catalog.

```rust
/// Everything needed to launch one agent run. Harness-agnostic.
pub struct RunSpec {
    /// Which harness to wrap (`claude-code`, `codex`, `opencode`, …).
    pub harness: HarnessId,

    /// Resolved skills to inject (from the catalog or inline).
    pub skills: Vec<SkillRef>,

    /// Resolved MCP servers to inject (from the catalog, inline, or in-process).
    pub mcps: Vec<McpRef>,

    /// Hooks to wire into the harness's native hook slots.        (P2/P3)
    pub hooks: Vec<HookRef>,

    /// Which account/credential profile the run authenticates with. (P2)
    pub account: Option<AccountId>,

    /// Always-on instructions / first prompt to seed.               (P2)
    pub initial: Option<Instructions>,

    /// Where the ephemeral config dir lives and whether to keep it.
    pub config: ConfigStrategy,     // Ephemeral (default) | Fixed(path)

    /// Sandbox settings (isol8) — off by default.                   (P3)
    pub isolation: Isolation,

    /// How `am` talks to the agent and exposes it outward.          (P2/P3)
    pub io: IoModes,                // default: Passthrough

    /// Verbatim extra args forwarded to the harness binary.
    pub passthrough_args: Vec<String>,

    /// Working directory for the agent.
    pub cwd: PathBuf,
}
```

`McpRef` is deliberately a small enum so lib mode can inject something the CLI
never could:

```rust
pub enum McpRef {
    /// A catalog entry resolved by id (stdio/sse/http server definition).
    Catalog(McpServer),
    /// An inline definition passed directly (lib mode or `--mcp-json`).
    Inline(McpServer),
    /// An in-process server hosted BY the embedding program (lib mode only).
    InProcess(InProcessMcpHandle),
}
```

## The provisioner and the "custom config folder" bridge

This is the mechanism that makes the whole wrapper possible, so it gets its own
section.

Every supported harness already accepts **some way to be pointed at a config it
did not choose** — that seam is documented per-harness in
[`../harness/`](../harness/). The provisioner's job is to write into that seam:

| Harness      | Injection seam the provisioner writes to                                  |
|--------------|---------------------------------------------------------------------------|
| Claude Code  | a temp workdir with `.claude/skills/…` + `CLAUDE.md`, `--mcp-config <file>` + `--strict-mcp-config`, `--append-system-prompt`; account via `env`/`CLAUDE_CONFIG_DIR` / a private `HOME`. |
| Codex        | `AGENTS.md` + `agents/…` in the workdir, MCP via its config file, account via env. |
| opencode     | `AGENTS.md` + `agent/…`, MCP + skills in `opencode.json`, ACP launch mode. |

The provisioner therefore emits three things from a `RunSpec`, per harness:

1. an **ephemeral config directory** (default under the OS temp/state dir, e.g.
   `~/.local/state/agent-manager/runs/<run-id>/`), populated with skills, MCP
   config, memory/instructions, hooks;
2. the **launch argv** (binary + flags, e.g. `--mcp-config`, `--strict-mcp-config`);
3. the **child environment** (config-dir env vars, account credentials as
   references, env hygiene — e.g. stripping inherited `CLAUDECODE*` vars so a
   nested run doesn't inherit the parent session).

The directory is created before launch and cleaned up after, unless
`ConfigStrategy::Fixed(path)` is set (useful for debugging "what did `am`
actually generate?") or the run is recorded for later resume.

**Key invariant — the user's real config is never touched.** Provisioning writes
only into the ephemeral dir. `~/.claude`, `.codex`, `opencode.json` are read
*only* by `am catalog import`, never written by a run. This is the sharpest
break from the old sync tool and the thing to protect in review.

## Module layout (target)

The library keeps the "all real logic in the lib, thin binary" rule. Proposed
shape (introduced incrementally — see the transition plan for what lands when):

```
src/
├── lib.rs            # crate root, re-exports, #![forbid(unsafe_code)]
├── main.rs           # thin binary: parse argv, dispatch, return
├── cli/              # clap surface (feature = "cli")
│   ├── mod.rs        #   Args, top-level dispatch
│   ├── run.rs        #   `am <harness> …`
│   └── catalog.rs    #   `am catalog ls|import|…`
├── spec.rs           # RunSpec, McpRef, SkillRef, IoModes, ConfigStrategy, …
├── resolve.rs        # (flags + config file + catalog) -> RunSpec
├── settings.rs       # load/merge the toml|yaml settings file
├── registry/         # the catalog (trait + fs-backed impl)
│   ├── mod.rs        #   Registry trait, CatalogEntry types
│   ├── fs.rs         #   filesystem-backed registry (config + folders)
│   └── import.rs     #   ingest ~/.claude, ~/.agent, … into the catalog
├── harness/          # per-harness knowledge (Harness trait + impls)
│   ├── mod.rs        #   Harness trait, HarnessId, all()/by_id()
│   ├── claude.rs
│   ├── codex.rs
│   └── opencode.rs
├── provision.rs      # RunSpec -> ephemeral config dir + argv + env (per harness)
├── run.rs            # spawn + supervise the child; owns the process lifecycle
├── io/               # I/O bridging (see io-modes.md)
│   ├── mod.rs        #   IoBridge trait
│   ├── passthrough.rs
│   ├── jsonl.rs      #   Claude stream-json input
│   └── acp.rs        #   ACP input/output
├── account.rs        # account catalog + credential-reference injection   (P2)
├── session.rs        # session history store (list/resume/record)          (P2/P3)
└── isolate.rs        # isol8 integration                                    (P3)
```

Not every module exists in Phase 1. P1 needs `spec`, `resolve`, `settings`,
`registry` (+ `import`), `harness`, `provision`, `run`, and `io/passthrough`.
The rest arrive with their phase.

## The `Harness` trait

Where the old design had a `Harness` *struct* of static facts, the target design
needs a `Harness` *trait* with behavior, because each harness differs in how it
is provisioned, launched, and (later) spoken to:

```rust
pub trait Harness {
    fn id(&self) -> HarnessId;

    /// Populate `dir` from the spec and return how to launch.
    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch>;

    /// Which I/O modes this harness can support (passthrough always; ACP/JSONL vary).
    fn io_support(&self) -> IoSupport;
}

pub struct Launch {
    pub program: String,        // "claude"
    pub args: Vec<String>,      // ["--mcp-config", "<dir>/mcp.json", "--strict-mcp-config", …]
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,// hygiene: CLAUDECODE, CLAUDE_CODE_ENTRYPOINT, …
}
```

The per-harness runtime facts that fill this in are already written down in
[`../harness/<id>.md`](../harness/) (see e.g. the "Orchestration / headless
invocation", "MCP at launch", and "Skills at launch" sections of
`claude-code.md`). Implementing a harness = transcribing that doc into a
`provision()` + `io_support()`.

## Invariants

- **The user's real harness config is read-only.** A run writes only to the
  ephemeral dir. Only `catalog import` reads the user's `~/.claude`/`~/.agent`.
- **`RunSpec` is the single boundary.** Resolve produces it; provision/run
  consume it. No stage reaches back to flags or the config file.
- **Front-end-agnostic core.** No `clap`/terminal types below `cli/`. Lib mode
  must compile with `default-features = false`.
- **No secret material on disk by `am`.** Accounts inject *references*, not
  secrets (see [overview](./overview.md) and the account section of the
  roadmap).
- **Failure is per-run and clean.** A failed provision leaves no partial state
  in the user's real dirs; the ephemeral dir is removed (or preserved only on
  explicit request / recorded session).
- **No `unsafe`** (`#![forbid(unsafe_code)]`), errors bubble as `anyhow::Result`
  at module boundaries — unchanged from the old conventions.
