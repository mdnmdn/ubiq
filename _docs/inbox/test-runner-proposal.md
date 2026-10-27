---
id: inbox-test-runner
title: Proposal — a multi-stack test runner with a test explorer, debug-a-test and agent tools
kind: proposal
status: proposal
summary: An agent writes a per-project test recipe (commands, placeholders, output format) and Ubiq owns a closed set of parsers and a host test worker, so tests for Python, Node and TypeScript, Java, .NET, Go and Rust become one structured event stream; a Test message family, an explorer, results and dashboard UI in IDE mode, debug-a-test through the debugger proposal, a built-in MCP server for running and for recipe setup, watch mode and coverage, with a phased build order, the decisions it would add and the open questions.
read_when: you are deciding how Ubiq discovers and runs tests, how an agent gets structured test results, how a test is debugged, or what the IDE mode needs for a test explorer
updated: 2026-10-26
depends_on: [inbox-debugger, feat-panes, feat-workbench-ide, feat-workbench, tech-architecture, tech-transport, tech-decisions]
---

**Ubiq would run tests as structured events instead of terminal text: one test worker in the host,
a per-project recipe that an agent writes and Ubiq validates, a closed set of parsers that Ubiq owns,
and one model that the explorer, the results view, the dashboard and an MCP server all read.** An agent that today
runs `pytest` or `go test` in a pane gets a screenful of text and a process exit code, and spends
tokens guessing which test failed and why; a tree of tests with a status each, a failure with its
message and location, and a rerun-failed button is cheaper for the agent and clearer for the human.

"Task" means a kanban card in this repository, so a test execution is a **run** here.

## 1. What exists, what is missing

**A run today is a pseudo-terminal pane and an exit code.** `ToolDef` in
`crates/ubiq-proto/src/tools.rs` carries a command, arguments, environment and a starting folder;
`RunTool` starts it in a pane ([the pane flow](../features/panes-and-terminals.md), `D101`). The
only results are the raw `TerminalOutput` bytes and `PaneExited.code`. Runner sources
(`RunnerSource` in `crates/ubiq-host/src/runners/mod.rs`; just, make and mise) discover named
targets for the run picker (`D197`), but a target is a command line, not a test.

| Want | Today |
|---|---|
| Know which test failed | An agent reads terminal text; each framework prints differently |
| A tree of tests with a status | Nothing |
| Run one test, one file, a filter | Type the command by hand |
| Rerun the failures, debug one test, coverage, flaky tests | Nothing; debugging needs the [debugger proposal](./debugger-proposal.md) |

A pane is an opaque byte stream by rule, so none of this can be added on top of `RunTool`.

## 2. The model

The shape is borrowed from the VS Code Testing API and neotest: a tree of items, a run that targets
a subset of it, and a stream of events.

| Noun | Meaning |
|---|---|
| Test item | A node: workspace, project or package, file, suite or class, test, parameter case. Carries a name, a location (file, line) and the stable id below |
| Test id | Stable per stack and per project: pytest node id, Go `package/TestX/sub`, Rust `binary::path::name`, .NET fully qualified name, Java `pkg.Class#method`, a Node `file > describe > test` path. The parser owns the format; the rest of Ubiq treats it as an opaque string |
| Profile | Run, Debug or Coverage |
| Selector | Data, not a command: items, files, a name pattern, failed-only, tags. The recipe's placeholders turn it into that project's flags |
| `TestRunId` | A `ulid_id!` in `crates/ubiq-proto/src/ids.rs`, one per run |
| Run event | Discovered, RunStarted, TestStarted, TestPassed, TestFailed, TestSkipped, Output, RunFinished |

A failed event carries the duration, the message, expected and actual when the framework gives them,
the location of the failing frame, the stack and the captured output. A finished event carries the
counts. Results are persisted per project: the last run in full, plus the last N runs as one row per
test, which is what the dashboard and flaky detection read. Where that lives is open question 4.

## 3. Architecture

The rules in [`AGENTS.md`](../../AGENTS.md) hold: the UI and the host talk only through messages,
and the UI never assumes the tests run locally.

| Piece | Where | What it is |
|---|---|---|
| Test crate | a new leaf crate, called `ubiq-test` here (no existing crate has that name), precedent `crates/ubiq-db` | Small and closed: the model, the recipe schema and placeholder expansion, the parsers. No process code in the default feature; spawning behind a host-only feature, as the debugger proposal does for `ubiq-dap` |
| Test worker | `crates/ubiq-host/src/watch/mod.rs` is the pattern | Job plus mailbox, a thread per run. Spawns the test command with pipes, not a pty; owns the process tree and cancellation |
| Test family | a new module in ubiq-proto, precedent `crates/ubiq-proto/src/db.rs` | Messages plus `TestRunId` |
| Presets and parsers registry | `runner_sources` on `ubiq_app::Contributions` (`crates/ubiq-app/src/lib.rs`) is the pattern | An open seam for presets, so Studio or a spike can add one without touching the base |
| MCP server | `crates/ubiq-host/src/mcp/catalogue.rs` | A new built-in row, section 6 |

### The split: the agent writes the recipe, Ubiq parses

**How to run a project's tests is project-specific and variable; reading the result is
deterministic.** Monorepos, custom scripts, docker or compose wrappers, Makefile targets and flag
combinations differ per project and per month, so Ubiq does not hard-code them, which is also the
base rule for launching harnesses. An agent inspects the project and writes a **recipe**; Ubiq owns
a small closed set of parsers and does the running, streaming and display.

### The recipe

A per-project file, a `tests` manifest in the project's Ubiq config (where it lives is open
question 1). It declares one or more **test targets** (a monorepo has several):

| Field | Meaning |
|---|---|
| Label, working directory, environment | Framework label for the icon and grouping; where and with what |
| Commands | Argv templates for discover, run all, run selection, rerun failed, coverage |
| Placeholders | `{ids}`, `{files}`, `{pattern}`, expanded by Ubiq from the selector (the crate does only this) |
| Output format | One parser per command, from the closed set below |
| Debug template | A DAP launch or attach template per the [debugger proposal](./debugger-proposal.md), naming its adapters |
| Shim | Optionally a Ubiq-shipped reporter shim by name, for stacks with no native stream |

The closed parser set: go `test2json`, libtest and nextest `libtest-json`, JUnit XML, TRX, TAP, and
the Ubiq NDJSON event schema that the reporter shims emit. The recipe picks one per command. Shims
are Ubiq-shipped assets under `<config root>/tests/<stack>/`, referenced by name and loaded by flag
or environment variable, **never written into the user's project**.

A few **presets** give a zero-agent default for the common cases (`go test -json`, nextest, pytest
with the shim); everything else comes from an agent. Section 4 is the reference the setup agent is
given, not a set of hard-coded adapters.

### The setup flow

A skill or agent definition plus MCP tools (section 6). The agent inspects the project, drafts the
recipe, then **validates it through Ubiq**: runs discover and a sample run and checks the parser
received events. The user reviews and approves the recipe through a registered dialog
([HITL dialogs](./hitl-registered-dialogs.md)) before it is saved, because its commands run
arbitrary code. When a recipe later breaks, Ubiq says so: a parse failure or zero discovered tests
is a **recipe stale** signal, and the UI offers "ask an agent to fix it", which reopens the flow
with the failing output attached.

### The worker and the messages

A run spawns the command with `Stdio::piped`, reads lines on a thread and sends events in batches,
the shape of the search fallback in `crates/ubiq-host/src/search/fallback.rs`. The pty is kept out
of it because a pty merges stdout and stderr and adds colour and line wrapping that a parser must
undo. The raw output is still wanted by a human, so the worker mirrors it into a console view
(optionally a pty-less log pane, open question 5).

| Direction | Messages |
|---|---|
| UI to host | list tests, run tests (selector, profile), cancel run, query results (a run, a test, history) |
| Host to UI | tests discovered, run event batches, run finished, results |

A new row goes in the [transport contract](../tech/transport-contract.md), with a dispatch arm in
the coordinator, a receiver in `crates/ubiq/src/app/wire.rs` and an arm in the id mapping in
`crates/ubiq/src/app/hosts.rs`. The pane id rule is met as the debugger proposal does: the run
carries the pane id of its console, or the project's pane when it has none. The worker, the process
and the files are all on the host's side, so a remote host works unchanged.

### Reporter shims

Most stacks have no stable live stream in their default output. The shims Ubiq would ship: a pytest
plugin through `-p` and `PYTHONPATH`, a vitest or jest reporter file, a Gradle init script with a
test listener, a JUnit Platform listener jar for Maven, a VSTest logger assembly. Each writes one
JSON object per line. Without a shim the recipe uses the framework's file report (JUnit XML or
TRX) read at the end, which loses live status but keeps the tree. Open question 3.

### Discovery

A recipe's discover command, or, for a stack with no listing command, a static parse. The outline in
`crates/ubiq/src/ui/outline.rs` extracts names and lines through tree-sitter but not attributes or
decorators, so it cannot tell a test from a function; a static parse needs its own queries (Java
`@Test`, `describe` and `it` calls, `t.Run` literals).

## 4. Per stack: the setup agent's reference

This section is what the setup agent is given and what the presets are cut from; it is not code
Ubiq hard-codes. Commands and flags came from reading documentation, and every one marked (verify)
has not been run here. Parsers must be tolerant of output schemas: ignore unknown fields.

| Stack | Detect | Discover | Select a subset | Live results | Debug | Coverage | Watch |
|---|---|---|---|---|---|---|---|
| Python, pytest | pytest.ini, `[tool.pytest.ini_options]`, conftest, dependency | `pytest --collect-only -q` | node id, `-k`, `-m`, `--lf` | injected plugin; `--junitxml` fallback | debugpy | pytest-cov to lcov | Ubiq-side |
| Node, vitest | config file, dependency | `vitest list --json` | file plus `-t` | custom reporter | js-debug | v8 or istanbul lcov | native |
| Node, jest | config file, dependency | `--listTests` (files only) plus static parse | `-t` | custom reporter | js-debug | lcov | native |
| Node, node:test | `node --test` scripts | none; static parse | `--test-name-pattern` | custom reporter | js-debug | `--experimental-test-coverage` (verify) | `--watch` |
| Java | Gradle or Maven plus JUnit | static parse, or JUnit Console Launcher | `--tests`, `-Dtest` | init script or listener jar; XML fallback | java-debug attach | JaCoCo XML | `--continuous` |
| .NET | test project with a test SDK or framework package | `dotnet test --list-tests` | `--filter` | VSTest logger; TRX fallback | netcoredbg | coverlet cobertura | `dotnet watch test` |
| Go | go.mod plus `_test.go` | `go test -list` | `-run` | `go test -json` | delve test mode | `-coverprofile` | Ubiq-side |
| Rust | Cargo.toml | nextest list JSON, or libtest `--list` | nextest filterset, or name `--exact` | nextest libtest-json-plus; libtest text | CodeLLDB | `cargo llvm-cov` lcov | Ubiq-side |

### Python

- **pytest.** Discover with `pytest --collect-only -q` (node ids `path::Class::test[param]`); select
  by node id, `-k`, `-m` or `--lf`. Live results from a shim plugin (`-p`) hooking
  `pytest_runtest_logreport`, or pytest-reportlog JSONL if installed. Coverage: pytest-cov with
  `--cov-report=lcov`. Run under the project's virtualenv interpreter; xdist reports through
  workers, so the plugin must run in the controller (verify).
- **unittest.** No collect flag: a bootstrap script with `TestLoader` and a custom `TestResult`.

### Node and TypeScript

- **vitest.** `vitest list --json` (4.1 adds `--static-parse`; verify the schema); select by file and
  `-t` (file:line, verify); a custom reporter writes NDJSON from `onTestCaseResult`.
- **jest.** `--listTests` yields files only, so names come from a static parse; `-t` selects; a
  custom reporter streams, `--json` is the end-of-run fallback.
- **node:test.** No listing; `--test-name-pattern` selects; a custom reporter or the built-in JUnit
  reporter gives results.
- **Package manager and monorepo.** The lockfile picks `npm`, `pnpm`, `yarn` or `bun`; the nearest
  config wins.
- **Debug.** One process so js-debug sees one target: vitest `--pool=forks` with one fork or
  `--no-file-parallelism` (flags changed in vitest 4, verify), jest `--runInBand`, node
  `--test-isolation=none` (verify); `--testTimeout=0`. Coverage: v8 or istanbul lcov.

### Java

The weakest listing story: JUnit has no first-class list command.

- **Discover.** A static parse of `@Test`, or the JUnit Console Launcher `discover` (1.10 and later,
  verify) with the project classpath, or jdtls with java-test.
- **Select.** Gradle `--tests 'pkg.Class.method'` (`--rerun`, verify); Maven `-Dtest='Class#method'`
  with `-Dsurefire.failIfNoSpecifiedTests=false`.
- **Live.** A Gradle init script `TestListener` writing NDJSON; for Maven a `TestExecutionListener`
  jar through ServiceLoader. Fallback: the XML in `build/test-results/test` or
  `target/surefire-reports`, one file per class.
- **Debug.** `gradle test --debug-jvm` (port 5005, suspended) or `mvn test -Dmaven.surefire.debug`,
  then attach; `maxParallelForks` 1. **Coverage.** JaCoCo XML, not lcov, so a parser to add.

### .NET

- **Two runners.** VSTest and Microsoft.Testing.Platform (MTP); telling them apart from the project
  file, and the MTP flags, need verifying.
- **Discover.** `dotnet test --list-tests` (text, not JSON), or the MTP executable `--list-tests`.
- **Select.** VSTest `--filter "FullyQualifiedName~X"`; MTP differs (verify).
- **Live.** A VSTest logger assembly via `--test-adapter-path` and `--logger`; TRX fallback.
- **Debug.** VSTest `VSTEST_HOST_DEBUG=1` prints the testhost process id and waits: parse it and
  attach netcoredbg. MTP: launch the test executable under netcoredbg. **Coverage.** coverlet
  cobertura. **Watch.** `dotnet watch test`.

### Go

The best stream of any stack.

- **Discover.** `go test -list '.*' ./...` lists top-level tests only; subtests come from a static
  `t.Run` parse or appear when the run reports them.
- **Select.** `-run '^TestX$/^sub$'` with `-count=1` so the cache does not answer.
- **Live.** `go test -json` (`test2json`): `Time`, `Action` (start, run, pause, cont, pass, fail,
  skip, output, bench), `Package`, `Test`, `Elapsed`, `Output`; output arrives interleaved per test.
- **Debug.** delve test mode with `-test.run`, `-test.timeout=0`, `-test.parallel=1`.
  **Coverage.** `-coverprofile`, Go's own format.

### Rust

- **Discover.** `cargo nextest list --message-format json` (documented, stable), or per binary
  `cargo test -- --list --format terse`. Nextest is used when `cargo nextest --version` succeeds.
- **Select.** nextest `-E 'test(=name)'`; libtest `name -- --exact`.
- **Live.** nextest with `NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1` and `--message-format
  libtest-json-plus` is experimental: pin the version, keep the parser tolerant. libtest's own JSON
  is nightly-only (RFC 3558), so plain libtest means pretty text; nextest can write JUnit at the end.
- **Debug.** `cargo test --no-run --message-format=json` gives the executable; run it under CodeLLDB
  with `[name, --exact, --nocapture, --test-threads=1]`, cwd the crate root, cargo environment
  (`CARGO_MANIFEST_DIR`). **Coverage.** `cargo llvm-cov --lcov`. **Watch.** Ubiq-side.

## 5. Debug a test

Per the debugger proposal, the recipe's debug template expands, with the selection, into a
`ubiq-dap` launch or attach config and the session is an ordinary `DebugSessionId` in the debug
worker; the test runner adds no debugger code, and the setup agent writes the template. What makes
a test debuggable:

| Pitfall | Handling |
|---|---|
| A breakpoint outlives the test timeout | Disable timeouts in the debug profile: pytest `--timeout=0`, vitest `--testTimeout=0`, Go `-test.timeout=0` |
| Parallel workers hide the target | Force one: `-p no:xdist` or `-n0`, `--runInBand`, `-test.parallel=1`, `--test-threads=1`, `maxParallelForks` 1 |
| Coverage distorts stepping | The Debug profile never carries coverage flags (`--no-cov`) |
| Build first; attach after suspend | Rust and .NET build before the launch; Java and .NET start suspended, wait for the port or process id, then attach |

The F-key bindings and the toolbar are the debugger proposal's. A test result shows a debug button,
and when the debug session ends the run closes with the result the framework reported, so a human
can debug the failure and still see it on the tree.

## 6. The agent surface

A new built-in server, `UBIQ_TESTS`, slug `ubiq-tests`. The slug `test` already exists in the
catalogue; it is the wiring proof, not a test runner, hence the new name. Answers are TOON. The
recommendation is to add it to `WORKER_MCPS`, the set every worker agent gets.

| Tool | What it does |
|---|---|
| tests_recipe_get | The current recipe, and whether it is flagged stale |
| tests_recipe_propose | Submit a drafted recipe; the user approves it through a dialog before it is saved |
| tests_recipe_validate | Run discover and a sample run of a draft and answer whether the parser got events, with the first parse errors |
| tests_list | A compact tree, filtered by path or pattern, with the last status of each test |
| tests_run | A selector and a profile (run or coverage), waiting or async; answers a run id and a summary |
| tests_results | A run's failures first: message, location, trimmed output and diff; `only_failed` and `max_items` caps |
| tests_rerun_failed | Rerun the failures of a run |
| tests_debug | Start a debug session on one test; answers a `DebugSessionId` usable with the debugger proposal's tools |
| tests_coverage | Uncovered ranges for a file |
| tests_cancel | Stop a run |

Design rules:

- **Failures first, truncated with a count.** A passing run answers in one line; a failing run
  answers the failures, each capped, with how much was cut.
- **Shared runs.** A run an agent starts appears in the explorer, attributed to it, and the agent
  sees the human's runs, as with the debug session.
- **Concurrency.** One run per project and selector at a time; a second request queues or attaches to
  the running one. Open question 7.
- **Trust.** The recipe is the gate: its commands run arbitrary code, so saving or changing one needs
  approval, and a changed recipe is shown as a diff. Running an approved recipe is what the agent's
  own shell can already do, so no gate per run by default; a setting adds one for `tests_run` and
  `tests_debug`. Open question 6.
- **Two guides.** The server's instructions teach the run loop: list, run a narrow selector, read
  failures, fix, rerun failed, then the file, then everything. A setup skill teaches the recipe
  loop: inspect, draft, validate, propose, and reuse a preset where one fits.

## 7. UI changes in IDE mode

See [the IDE workbench](../features/workbench-ide.md). The editor gap is the same as the debugger
proposal's: no gutter marker or click API and no open-file-at-line helper, and those are
prerequisites here too.

### Panels

`PanelKind` in `crates/ubiq/src/state/dock.rs` gets new variants, with arms in
`crates/ubiq/src/ui/dock/mod.rs`.

| Panel | Kind | Notes |
|---|---|---|
| Test explorer | edge, like Outline | A tree with a status icon per node, a filter, run and debug buttons on each node, group by file or suite, show only failed. The tree widget in `crates/ubiq/src/ui/explorer.rs` is the default for every tree; `crates/ubiq/src/ui/db/plan.rs` is the precedent for a collapsible tree with a hot node |
| Test results | bottom tab, like Logs | The selected test: message, expected against actual diff, stack with clickable frames (needs open-at-line), captured output. A filtered console as in `crates/ubiq/src/ui/logs.rs` |
| Test dashboard | centre panel, or a section of the Control screen | Pass, fail and skip counts, duration, slowest tests, flaky tests from history, a per-run timeline, coverage percentage. The precedent is `crates/ubiq/src/ui/stats.rs`; a table can reuse `crates/ubiq/src/ui/db/grid.rs` |

### Editor, chrome and input

| Item | Where |
|---|---|
| Run and debug glyphs above a test, pass and fail markers, a coverage gutter | The editor gutter, with the debugger proposal's options. Until it exists, the explorer plus navigator commands are the fallback |
| "Test at cursor" | tree-sitter queries per language, the same ones static discovery uses |
| Navigator commands: run test at cursor, run file, rerun failed, debug test at cursor | The command palette |
| Key bindings | Chosen next to the debugger's F-keys; `crates/ubiq/src/app/mod.rs` binds under the "Workbench" context and no clash has been checked (verify) |
| Status bar counts | `crates/ubiq/src/ui/status_bar.rs`, a closed function today |
| Run picker | Whether discovered suites appear beside just and make targets in `crates/ubiq/src/ui/run_tool_menu.rs` and `crates/ubiq/src/state/run_picker.rs` (`D197`); open question 10 |
| Settings section per test target: enable, recipe status and "edit with an agent", watch on save | `SettingsSectionSpec` in `crates/ubiq/src/ext/settings.rs` |
| Theme tokens: pass, fail, skip, covered, uncovered | `crates/ubiq/src/theme.rs`, a value in both palettes |

Dock panels, MCP servers, commands and status items are closed in `ubiq_app::Contributions`; only
rail modes, settings sections, bar menus, runner kinds, viewers, task providers and runner sources
are open. The presets registry would be a new open seam, with an icon and label per framework
following `crates/ubiq/src/ext/runner.rs`.

### Watch mode and coverage

**Watch.** A framework's own watch (vitest, jest, `gradle --continuous`, `dotnet watch test`) is a
run that never finishes until cancelled. For the rest, Ubiq subscribes to `ProjectFilesChanged`,
which `crates/ubiq-host/src/watch/mod.rs` already coalesces (150 ms quiet, 1.2 s cap), and reruns the
failed set plus the tests in changed test files; mapping a source file to its tests is a heuristic.

**Coverage.** The recipe names a report file and its format (lcov, JaCoCo, cobertura, Go profile).
The model keeps covered and uncovered line ranges per file, which the dashboard, `tests_coverage`
and the gutter read. It is a profile, never the default: it slows runs and disturbs debugging.

## 8. Phasing

| Phase | Scope | Rough cost |
|---|---|---|
| 0 | The crate (model, recipe schema, placeholder expansion, parsers), the worker, the Test family, the recipe tools and the setup-agent flow, presets for Go, Rust (nextest) and pytest, explorer and results view; no debug | 3 to 4 weeks |
| 1 | The remaining parsers (TAP, TRX) and the reporter shims for vitest, jest and node:test | 2 weeks |
| 2 | Debug a test, after phases 1 and 2 of the debugger proposal | 1 to 2 weeks |
| 3 | .NET and Java shims and reference recipes | 2 weeks |
| 4 | Dashboard history and flaky detection, coverage gutter, watch mode | 2 to 3 weeks |
| 5 | Static discovery polish, test at cursor, "recipe stale" repair flow polish | 1 to 2 weeks |

Phase 0 needs no gutter: an agent sets up the recipe and reads failures, the human reads the
explorer; only the presets are zero-agent.

## 9. Decisions this would add

- Tests are a run model of their own (items, selector, structured events), not a `ToolDef` in a pane.
  **Cost:** a second way to run a command exists beside `RunTool`.
- How to run a project's tests is a per-project recipe that an agent writes and the user approves;
  Ubiq hard-codes no stack's command line, and owns only a closed set of parsers. **Cost:** a
  project has no tests until a recipe exists, except where a preset applies.
- Recipe approval is a registered dialog, and a changed recipe is re-approved. **Cost:** an agent
  cannot reconfigure tests unattended.
- A broken recipe is reported as stale (a parse failure or zero tests) and offered to an agent to
  repair. **Cost:** the worker has to tell "no tests" from "parser saw nothing".
- Live results come from reporter shims in a managed directory, never written into the project, with
  the framework's own file report as the fallback. **Cost:** a shim to maintain per stack.
- The agent and the human share runs, and the MCP server is a client of the worker, not a second
  runner. **Cost:** attribution is required.
- A debugged test is a debug session through the debugger proposal's worker. **Cost:** the test
  runner depends on the debug phases for one of its profiles.
- Run history is persisted per project as one row per test per run, bounded. **Cost:** a new store.

## 10. Open questions

1. Where the recipe lives: committed in the repository, so a team shares it and a clone just works,
   or in Ubiq's project store, so the repository is untouched. Committed is shareable but puts
   commands that run arbitrary code in front of every clone, which makes approval matter more.
2. Whether this lands in the base or in Studio. The same seam argument as the debugger proposal: the
   dock, MCP and command seams are closed.
3. Reporter shims versus only the native formats (JUnit XML, TRX, `test2json`). Shims give live
   status; native formats are simpler and never break on a framework upgrade.
4. Where run history lives and how much: the project store, or a file under the project's data folder.
5. Whether the raw output of a run is a pty-less log pane or only a view in the results panel.
6. Trust: approval on every recipe change is assumed; whether `tests_run` also asks by default, and
   whether a recipe from a cloned repository is trusted until a human has seen it.
7. Concurrency and queueing when an agent and a human both run, or two agents do.
8. How presets and agent recipes coexist: a preset as a fallback when no recipe exists, a template
   the agent starts from, or both; and whether a preset upgrade can silently change behaviour.
9. Recipe versioning: a schema version, and what happens to a recipe when the schema or a parser
   changes.
10. Whether this unifies with the run picker and `ToolDef`, or stays separate, with the picker only
    offering "run tests" as a target.
11. Coverage in the first phase or later.
12. Existing MCP test servers (privsim/mcp-test-runner, `@paretools/test`, jwilger/mcp-pytest-runner,
    djankies/vitest-mcp) are small and of unverified maturity. Each spawns its own run, invisible to
    the UI; in-host wins for shared runs, but the tool names could be borrowed.

## Related docs

- [Debugger proposal](./debugger-proposal.md) — the sessions, adapters and key bindings a debugged test uses
- [Panes and terminals](../features/panes-and-terminals.md) — `RunTool` and the pane flow this does not replace
- [The IDE workbench](../features/workbench-ide.md) — the mode the panels live in
- [Architecture](../tech/architecture.md) — the rules the Test family follows
- [Decisions](../tech/decisions.md) — `D101` and `D197`
- [HITL registered dialogs](./hitl-registered-dialogs.md) — the optional gate on running tests
- [QA checks proposal](./qa-checks-proposal.md) — custom HTTP, SQL and script checks in the same run model
