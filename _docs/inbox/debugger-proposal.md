---
id: inbox-debugger
title: Proposal — agentic multi-language debugging through DAP
kind: proposal
status: proposal
summary: One Debug Adapter Protocol client in Rust with one adapter per language (debugpy, delve, js-debug, CodeLLDB, netcoredbg, java-debug), a debug worker in the host, a Debug message family, a built-in MCP server so agents drive the same session the human sees, the IDE-mode UI it needs, a phased build order, the decisions it would add and the open questions.
read_when: you are deciding how Ubiq debugs code, how an agent gets breakpoints and stepping, which DAP adapters to support and how they install, or what the IDE mode needs for breakpoints
updated: 2026-10-26
depends_on: [feat-workbench-ide, feat-workbench, tech-architecture, tech-transport, tech-decisions, inbox-hitl-dialogs, inbox-policy-registry, inbox-acp-client, inbox-plugin-system]
---

**Ubiq would embed one DAP client in Rust and one adapter per language, so that a human and an
agent debug the same session: the human through breakpoint glyphs and a call-stack panel, the agent
through a Ubiq-hosted MCP server whose tools are the same requests.** An agent that today runs `pdb`
or `dlv` in a terminal is blind and spends tokens on prompts and screen scrape; a structured,
budgeted debugger is the cheaper and the safer way to let it inspect a running program.

## 1. Why DAP, and why in Ubiq

DAP is the protocol VS Code speaks to every debugger: Content-Length framed JSON, requests,
responses and events, and one adapter process per language. Adding a language is installing an
adapter, not writing a debugger.

DAP-to-MCP bridges already exist (verify the list before relying on it):

| Bridge | Note |
|---|---|
| debugmcpdev/mcp-debugger | MIT, the most active of the four |
| ctagard/dap-mcp | thin DAP wrapper |
| ai-debugger-inc/aidb | multi-language |
| ztripez/mcp-dap | small |

Each spawns its own adapter, so the agent's session is invisible to the human's UI and the human's
breakpoints are invisible to the agent. That is the argument for building in the host: the session
is one object, with two kinds of client.

## 2. Architecture

The rules in [`AGENTS.md`](../../AGENTS.md) hold: the UI and the host talk only through messages,
every message carries an id, and the UI never assumes the debuggee is local.

| Piece | Where | What it is |
|---|---|---|
| `ubiq-dap` | new leaf crate, precedent `crates/ubiq-db` | Default feature: the model. A host-only feature: process and socket transport. DAP framing, request, response and event types, the session state machine, the launch-config model, the launch.json parser |
| Debug worker | `crates/ubiq-host/src/watch/mod.rs` is the pattern | Job plus mailbox, start a job and get a handle, as the Files, Git, Search and Index workers do. Owns adapter processes, sockets and reverse requests. Feature-gated like `db` |
| Debug family | a new debug module in ubiq-proto, precedent `crates/ubiq-proto/src/db.rs` | Messages plus a `DebugSessionId` from `ulid_id!` in `crates/ubiq-proto/src/ids.rs` |
| MCP server | `crates/ubiq-host/src/mcp/catalogue.rs`, `crates/ubiq-host/src/mcp/server.rs` | A new built-in row, served by the existing loopback listener, dispatched by a module like `crates/ubiq-host/src/mcp/sql/` |

### The messages

UI to host: launch, attach, stop, set breakpoints, continue, step, pause, stack trace, variables,
evaluate. Host to UI: started, stopped, continued, output, terminated, and the responses. A new row
goes in the [transport contract](../tech/transport-contract.md), with a dispatch arm in the
coordinator, a receiver in `crates/ubiq/src/app/wire.rs` and an arm in the id mapping in
`crates/ubiq/src/app/hosts.rs`. The pane id rule is met by the session carrying the pane id of the
terminal that holds the debuggee, or the project's pane when it has none.

### Reverse requests

DAP lets the adapter ask the client to do things. Two matter.

| Request | What Ubiq does |
|---|---|
| `runInTerminal` | The debuggee gets a real pty pane. `SpawnWorkspace` in `crates/ubiq-proto/src/messages.rs` takes a shell or harness name, not an argv, so this needs a new spawn message carrying project id, argv, cwd, env and a title. Answered with the process id |
| `startDebugging` | js-debug children and debugpy subprocesses. Becomes a child session under one parent, shown as a tree |

The client advertises `supportsRunInTerminalRequest` and `supportsStartDebuggingRequest` in
`initialize`.

### The adapter catalogue

Where the knowledge of how to launch an adapter lives is a real question, because the base rule is
that Ubiq never hard-codes how to launch a harness and `crates/agent-manager` owns that. An adapter
is not a harness, but the shape is the same: a name, a launch command, a transport, a probe.
The recommendation is a data table inside `ubiq-dap`, because agent-manager has no UI dependency
and no reason to know about debuggers; the table would be data, not branching code, so it can move
later. Open question 2.

### The MCP server

A new built-in server (`UBIQ_DEBUG`, next to `UBIQ_ASK` and `UBIQ_SQL_READ` in the catalogue),
injected through `StartConversation.mcps` like the others. Answers are TOON, like the SQL server's.
Section 4 lists the tools.

### Remote hosts

The adapter, its sockets and the debuggee are all on the host's side of the wire; the UI sees only
messages. A remote host works unchanged, which is the point of keeping DAP out of the UI.

## 3. Per language: what to install and how

Auto-detect is the same for every adapter: PATH, then the managed directory, then a `--version`
probe. TCP is preferred over stdio wherever an adapter offers it, because a socket survives the
adapter's own stdout noise. Sizes and flags marked (verify) came from reading release notes and
have not been run here.

| Language | Adapter | Licence | Install | Needs | Transport | Size |
|---|---|---|---|---|---|---|
| Python | debugpy | MIT | `pip install debugpy` | Python 3.8 or later (verify) | stdio, or `--host --port` | about 4 MB |
| Go | delve | MIT | `go install`, brew, scoop | Go toolchain | TCP only, port parsed from output | about 30 MB (verify) |
| JS and TS | js-debug | MIT | release tarball | Node on PATH, 18 or later (verify) | TCP | about 1.3 MB |
| Rust, C, C++ | CodeLLDB | MIT | platform .vsix | none, bundles LLDB | TCP | 45 to 56 MB |
| .NET | netcoredbg | MIT | release archive | .NET SDK | stdio, or server port (verify) | about 3.5 MB |
| Java | java-debug in jdtls | EPL (verify) | Maven Central jar plus jdtls | JDK 21 or later for jdtls (verify) | TCP, port from a jdtls command | 50 MB and up, plus a JDK |

### Managed install

Adapters that Ubiq installs go under `<config root>/dap/<adapter>/`, so an install never touches
the user's global environment. The flow copies the ACP adapter install: the settings page lists
each adapter as detected, installed or downloadable, and an install button runs it. The messages to
copy are `AcpRegistryStatus::Downloadable` and `AcpAdapterInstalled` in
`crates/ubiq-proto/src/messages.rs`, and the probe is what `CheckAgentCommand` does for a harness.
Auto-install versus detect-only is open question 3.

### Python: debugpy

- **Install.** `pip install debugpy` into the project venv, or `pip install --target` the managed
  directory for an adapter that is independent of the project. uv and pipx work the same way.
- **Launch.** `python -m debugpy.adapter`, stdio by default, or with `--host 127.0.0.1 --port <p>`.
  `--log-dir` is useful for the settings page's diagnostics.
- **Config.** Type `debugpy`. Launch takes `program`, `module` or `code`, plus `args`, `cwd`, `env`,
  `python`, `console`, `justMyCode`, `stopOnEntry`, `subProcess`. Attach takes `connect` with host
  and port; the target is started as `python -m debugpy --listen 5678 --wait-for-client app.py`.
- **runInTerminal.** Sent when `console` is not `internalConsole`.
- **Quirk.** The adapter may run on a different Python than the debuggee. The `python` field picks
  the project's venv interpreter, so the adapter can live in the managed directory.
- **Detect.** A `pyproject.toml`, `requirements.txt` or `.py` entry file; a `.venv` in the project.

### Go: delve

- **Install.** `go install github.com/go-delve/delve/cmd/dlv@latest`, `brew install delve` or
  scoop's `delve`.
- **Launch.** `dlv dap --listen=127.0.0.1:0` prints `DAP server listening at: 127.0.0.1:NNNNN`.
  The host reads that line for the port. There is no stdio mode.
- **Config.** `mode` is debug, test, exec, replay or core; plus `program`, `args`, `cwd`, `env`,
  `buildFlags`, `stopOnEntry`, `substitutePath`, `hideSystemGoroutines`. Attach is `mode: local`
  with a process id, or remote.
- **Quirks.** macOS asks for DevTools permission on first run. Delve builds the program itself, so
  `buildFlags` carries tags and the race flag.
- **Detect.** `go.mod`.

### JavaScript and TypeScript: js-debug

- **Install.** The asset `js-debug-dap-v<ver>.tar.gz` from the vscode-js-debug releases on GitHub,
  latest seen v1.140.0. It is OS-independent, so one download serves every platform.
- **Launch.** `node js-debug/src/dapDebugServer.js <port> 127.0.0.1`, TCP (verify the CLI).
- **Config.** Type `pwa-node`. Launch takes `program`, `args`, `cwd`, `env`, `envFile`,
  `runtimeExecutable`, `runtimeArgs`, `console`, `stopOnEntry`, `sourceMaps`, `outFiles`,
  `skipFiles`. Attach takes port and address, or a process id. TypeScript runs through `tsx` or
  `ts-node` as `runtimeExecutable`, or through source maps and `outFiles`.
- **Quirk, the large one.** Sessions are a tree. The adapter sends `startDebugging`, and each child
  needs a new connection to the same server carrying `__pendingTargetId`. Worker threads and child
  processes arrive this way. The session model has to be a tree from the start, which is why it is
  one here and not a flat list.
- **Detect.** `package.json`, `tsconfig.json`.

### Rust, C and C++: CodeLLDB

- **Install.** The platform .vsix from the CodeLLDB releases, latest seen v1.12.3: darwin
  arm64 and x64, linux x64, arm64 and armhf, win32 x64. A vsix is a zip; the adapter is
  `extension/adapter/codelldb` and takes `--port <n>` (verify). It bundles its own LLDB and the Rust
  pretty-printers, and takes `sourceLanguages: ["rust"]`.
- **Lighter alternative.** `lldb-dap`, Apache-2.0 with the LLVM exception, ships with Xcode command
  line tools (`xcrun -f lldb-dap`), brew `llvm`, apt `lldb` (versioned name) and winget
  `LLVM.LLVM`. Stdio by default; `--port` and `--connection` on newer builds (verify). Rust
  pretty-printers need `initCommands` importing from `$(rustc --print sysroot)/lib/rustlib/etc`.
- **Linux fallback.** `gdb -i dap` from gdb 14, GPL. `runInTerminal` is probably unsupported (verify).
- **Quirks.** Build the debug profile; find the binary with `cargo build --message-format=json`.
  macOS needs the Xcode command line tools for `debugserver`, and the DevTools permission. On
  Windows MSVC PDBs work better with CodeLLDB.
- **Detect.** `Cargo.toml`, `CMakeLists.txt`.

### .NET: netcoredbg

- **Install.** Samsung's netcoredbg, latest seen 3.2.0-1092: `netcoredbg-linux-amd64.tar.gz`,
  `-linux-arm64.tar.gz`, `-osx-arm64.zip`, `-win64.zip`. There is no macOS x64 asset, so an Intel Mac
  builds from source. On macOS remove quarantine with `xattr -d com.apple.quarantine`.
- **Launch.** `netcoredbg --interpreter=vscode` over stdio, or `--server=<port>` (verify).
- **Config.** `program` (the built .dll), `args`, `cwd`, `env`, `stopAtEntry`, `console`,
  `justMyCode`, `enableStepFiltering`. Attach takes a process id.
- **Prerequisites.** The .NET SDK, and a Debug build with PDBs, so a launch is preceded by
  `dotnet build`.
- **Licence.** Microsoft's `vsdbg` is licensed for Visual Studio and VS Code only, so it is not an
  option.
- **Detect.** `*.csproj`, `*.sln`.

### Java: java-debug inside jdtls

The heaviest, and last. java-debug is an Eclipse bundle that runs inside the Eclipse JDT language
server; there is no standalone mode.

- **Install.** The plugin jar `com.microsoft.java.debug.plugin-<ver>.jar` from Maven Central (0.53.1
  seen, check for newer), or built with `./mvnw` on JDK 17 or later. jdtls from `brew install jdtls`,
  the Eclipse tarball `jdt-language-server-<ver>.tar.gz` from download.eclipse.org/jdtls, or scoop
  (verify).
- **Flow.** Start jdtls over stdio with `initializationOptions.bundles` set to the jar, then send
  `workspace/executeCommand` with `vscode.java.startDebugSession`. It returns a port, and DAP runs
  over TCP to it. The commands that resolve the classpath, main class and Java executable are used
  for the launch config. Ubiq therefore needs a minimal LSP client for jdtls.
- **Config.** Type `java`: `mainClass`, `projectName`, `args`, `vmArgs`, `cwd`, `env`, `classPaths`,
  `modulePaths`, `console`. Attach takes `hostName` and `port`, against a JVM started with
  `-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=*:5005`.
- **Quirks.** jdtls starts slowly and needs a `-data` workspace directory per project.
- **Detect.** `pom.xml`, `build.gradle`.

## 4. The agent surface

The tools of the MCP server. Every tool names a session, defaulting to the one that is active.

| Tool | What it does |
|---|---|
| debug_configs | List launch.json entries and the detected ones |
| debug_launch, debug_attach | Start a session from a config; HITL gated |
| debug_stop | End the session |
| set_breakpoints | Per file; takes condition, hitCondition and logMessage |
| continue, step_over, step_in, step_out, pause | Execution control |
| wait_until_stopped | Block up to a timeout, then answer with the stop reason |
| stack | Frames, with a depth cap |
| variables | By reference, with depth and max_items caps |
| evaluate | An expression in a frame; HITL gated |
| debug_output | The program's output since a cursor |

Design rules:

- **One shared session.** The UI shows what the agent did live, attributed to it, and the agent sees
  the human's breakpoints and stops. Nothing about a session is private to the MCP side.
- **Logpoints and conditional breakpoints first.** A batch of logpoints and one run costs a
  fraction of an interactive step-and-look loop, and the guide tool says so.
- **A token budget.** Depth and item caps, truncation with a count of what was cut, TOON for
  tabular answers.
- **HITL gates** on launch, attach, evaluate and setVariable, because each runs code or changes a
  running program. No general policy registry exists ([policy registry](./policy-registry-proposal.md)),
  so the gate is an ask-style registered dialog ([HITL dialogs](./hitl-registered-dialogs.md)); the
  precedent is `crates/ubiq-host/src/mcp/ask.rs`.
- **A guide.** Server `initialize` instructions or a guide tool teach the loop above. The spike skill
  has the precedent for an agent-facing server that does this.

## 5. UI changes in IDE mode

What exists, and what has to be added. See [the IDE workbench](../features/workbench-ide.md).

### The editor

The editor is gpui-component's `EditorState`, built in `crates/ubiq/src/app/boot.rs`. No gutter
click or breakpoint-marker API was found (verify against the pinned version).

| Option | Cost |
|---|---|
| Upstream change to gpui-component: gutter markers and click events | Preferred; slowest, and Ubiq pins a version |
| An overlay element drawn over the gutter | Works now; fragile against scrolling, folding and wrapping |
| Abuse diagnostics, as DB mode does through `diagnostics_mut` | Gets a glyph on a line; cannot take a click or tell kinds apart |

What is needed: glyphs for enabled, disabled, conditional, logpoint and unverified breakpoints;
click to toggle; a current-execution-line highlight; inline values (later); hover evaluation through
a hover provider, with the completion provider set through `lsp_mut` in `boot.rs` as the precedent.

Stopping on a breakpoint has to open the file at the line. No such API exists; the only cursor call
is `set_cursor_position` in `crates/ubiq/src/app/picker.rs`, so an open-file-at-line helper is a
prerequisite of phase 3.

### Panels

`PanelKind` in `crates/ubiq/src/state/dock.rs` gets new variants, each with its class, home and name
and an arm in the dock's body function in `crates/ubiq/src/ui/dock/mod.rs`.

| Panel | Kind | Notes |
|---|---|---|
| Call stack | edge, like Outline and GitRefs | Threads, then frames; child sessions as a tree |
| Variables | edge | Lazy expansion by variables reference |
| Watch | edge | Expressions re-evaluated at each stop |
| Breakpoints | edge | All breakpoints, enable and disable, who set each |
| Debug console | bottom tab, like Logs and DbSql | Output plus a REPL line that evaluates |

### Chrome and input

| Item | Where |
|---|---|
| Debug toolbar: continue, pause, step, restart, stop; a session picker when several | The title bar, or floating over the editor |
| Key bindings F5, Shift-F5, F9, F10, F11, Shift-F11 | `crates/ubiq/src/app/mod.rs`; only f1 and shift-f1 are bound today, and the context is "Workbench" |
| Navigator commands: start a config, toggle a breakpoint | The command palette |
| Status indicator | `crates/ubiq/src/ui/status_bar.rs`, a closed function today |
| Settings section for adapters: detected, installed, paths, install buttons | Through `SettingsSectionSpec` in `crates/ubiq/src/ext/settings.rs` |
| Theme tokens: breakpoint, current line, stopped-frame | `crates/ubiq/src/theme.rs`, a value in both palettes |

A "Debug" rail mode through `RailModeSpec` in `crates/ubiq/src/ext/rail.rs` is possible. The
recommendation is panels inside the IDE mode first: a debug session is a view of the code, not a
separate place.

### Closing, attribution

- **Tab close.** A debug session owns a process, so it follows D103 and D130: its own `debug_close`
  setting, Hide or Close, next to `TabClose` in `crates/ubiq/src/state/settings.rs`. Would add a
  decision.
- **Attribution.** Each breakpoint, step and evaluate shows who made it, the human or which agent.
  When an agent is driving, the toolbar shows it, and a human step either takes over or is refused.
  Open question 4.

### Seams

Dock panels, MCP servers, commands and status items are closed in `ubiq_app::Contributions`
(`crates/ubiq-app/src/lib.rs`); only rail modes, settings sections, bar menus, runner kinds,
viewers, task providers and runner sources are open. A Studio-only version would need those seams
opened in the base first. Open question 1.

## 6. Phasing

| Phase | Scope | Rough cost |
|---|---|---|
| 0 | A spike under `spike/debug` (the spike skill): Python and Go, the MCP tools, minimal panels | 1 to 2 weeks |
| 1 | `ubiq-dap`, the host worker, the Debug family, Python and Go, the managed install | 3 weeks |
| 2 | Node with child sessions, Rust, launch.json import | 2 to 3 weeks |
| 3 | The editor gutter, open-at-line, hover evaluation | 2 to 4 weeks, depending on the upstream answer |
| 4 | .NET | 1 week |
| 5 | Java through jdtls and a minimal LSP client | 2 to 3 weeks |
| 6 | HITL gate hardening, inline values, remote hosts end to end | 2 weeks |

Phases 0 to 2 are useful without any gutter: an agent sets logpoints by tool, the human reads the
panels.

## 7. Decisions this would add

- Debugging is DAP, with one client in `ubiq-dap` and the adapter as the per-language part, rather
  than a bridge that spawns its own adapter. **Cost:** Ubiq owns the session state machine and the
  reverse requests.
- The agent and the human share one debug session, and the MCP server is a client of it, not a
  second one. **Cost:** attribution and handoff become required, not optional.
- A debug session's close behaviour is its own setting, following D103 and D130.
- Adapters install under `<config root>/dap/` and are never installed globally. **Cost:** Ubiq
  carries a download and a probe per adapter.
- Debugger launch and evaluate are gated by a registered dialog. **Cost:** an agent cannot debug
  unattended until the policy registry exists.

## 8. Open questions

1. Whether this lands in the base or in Studio. Studio would need the dock, MCP and command seams
   opened in the base first.
2. Where the adapter catalogue lives: `crates/agent-manager`, which owns launch knowledge for
   harnesses, or a data table in `ubiq-dap`. The latter is recommended.
3. Auto-install versus detect-only, per adapter. A 50 MB download the user did not ask for is not
   free.
4. Who drives when the agent and the human both act: a lock, a handoff, or last writer wins.
5. A gpui-component upstream change versus an overlay, for the gutter.
6. Java: ship jdtls, or require it already installed.
7. How much of launch.json variable substitution to support: `${workspaceFolder}`, `${file}`,
   `${env:X}`, and inputs.
8. Whether breakpoints are per agent, so two agents on one program do not stop each other.

## Related docs

- [The IDE workbench](../features/workbench-ide.md) — the mode the panels and the gutter live in
- [The workbench](../features/workbench.md) — rail modes and the seams
- [Architecture](../tech/architecture.md) — the rules the Debug family follows
- [Transport contract](../tech/transport-contract.md) — where the Debug rows go
- [Decisions](../tech/decisions.md) — D103 and D130, the close semantics
- [HITL registered dialogs](./hitl-registered-dialogs.md) — the gate on launch and evaluate
- [Policy registry](./policy-registry-proposal.md) — what would replace the per-tool dialog
- [ACP client bridge](./acp-client-bridge.md) — the adapter install this one copies
- [Plugin system](./plugin-system-proposal.md) — the other proposal that adds a bus client
