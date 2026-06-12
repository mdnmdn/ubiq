# Ubiq — App Description

## What is Ubiq

Ubiq is a harness multiplexer — a tmux-like application for hosting and orchestrating multiple interactive AI agent harnesses side by side. Each agent runs in a real terminal pane (PTY) with full TUI support: colors, cursor, resize, raw keystrokes.

---

## Core Concepts

### Agent Type

An agent type is a definition for a harness that can be spawned. Each agent type specifies:

- **name** — identifier (e.g. `claude`, `opencode`, `gemini`, `codex`, `copilot`)
- **command** — the binary to execute (e.g. `claude`, `opencode`)
- **description** — human-readable label
- **default_args** — arguments passed to the command on spawn

Agent types are defined in `src-tauri/agents.toml` and loaded at startup into an `AgentRegistry`. The UI lists available agent types so the user can pick one when creating a session or spawning a workspace.

### Session

A session is a working context that groups related agent workspaces. A session has:

- **id** — unique UUID
- **name** — user-chosen label (e.g. "backend-refactor")
- **home_folder** — base directory for the session (default: `./_workspace`)
- **created_at** — timestamp (epoch seconds as string)

A session starts with zero workspaces. The user attaches to a session to see and interact with its workspaces. Multiple sessions can exist simultaneously, each independent.

**Lifecycle:**
1. User creates a session → `CreateSession` → orchestrator stores it, sends `SessionCreated`
2. UI auto-attaches → `AttachToSession` → orchestrator sends `SessionAttached` with workspace list
3. UI spawns initial workspace → `SpawnWorkspace` → PTY created, process started
4. User can detach/reattach at any time → sessions persist in memory

### Workspace

A workspace is a running instance of an agent inside a session. A workspace has:

- **id** — unique UUID
- **session_id** — parent session
- **agent_type** — which agent is running (e.g. `claude`)
- **folder** — working directory for the agent process
- **cols / rows** — terminal dimensions (default 80x24)
- **running** — whether the process is alive

A workspace owns:
- A **PTY** (pseudo-terminal) via `portable-pty`
- A **child process** (the agent binary) running in that PTY
- A **reader thread** that reads PTY output and sends it through the bus
- A **writer** that receives UI keystrokes and writes them to the PTY

When a workspace is spawned, the orchestrator creates the PTY, launches the agent process, and starts streaming output. The UI creates an xterm.js terminal pane bound to that workspace.

**Lifecycle:**
1. `SpawnWorkspace` → orchestrator creates PTY pair, spawns child process
2. Reader thread starts, streaming `TerminalOutput` events to UI
3. UI sends `TerminalInput` → orchestrator writes to PTY → child process receives input
4. Child process exits → reader thread sends `WorkspaceExited` → UI shows "[Process exited]"

---

## The Bus

All communication between the UI and the orchestrator flows through a single bidirectional channel called **the bus**. There are no direct calls between UI and orchestrator — every interaction is a bus message.

### Message Format

Messages are JSON objects with a tagged enum structure:

```json
{
  "type": "CreateSession",
  "payload": {
    "name": "my-project",
    "agent_type": "claude",
    "home_folder": null
  }
}
```

The `type` field identifies the message variant. The `payload` field contains the data (structure varies by type). Some messages have no payload (e.g. `ListSessions`).

On the Rust side, `BusMessage` is a serde-tagged enum:
```rust
#[serde(tag = "type", content = "payload")]
pub enum BusMessage {
    ListSessions,
    CreateSession { name: String, agent_type: String, home_folder: Option<String> },
    // ...
}
```

### Transport

**Rust → JS (events):**
The `Bus` struct calls `app.emit("bus:message", json_string)` to send a Tauri event. The JS bus listens on `"bus:message"` and dispatches by `type`.

**JS → Rust (commands):**
The JS bus calls `invoke("bus_command", { message: json_string })`. The Rust `bus_command` Tauri command deserializes the message and dispatches it to the orchestrator's `handle_message()`.

```
JS bus.send(msg)
  → invoke("bus_command", { message: JSON.stringify(msg) })
  → Rust bus_command(state, message) deserializes BusMessage
  → orchestrator.handle_message(msg) processes it
  → orchestrator calls bus.send(response) via AppHandle::emit("bus:message", json)
  → JS listen("bus:message") receives and dispatches by type
  → handler(payload) updates UI
```

### Why a Single Bus

The bus is designed to be serializable. Today it's an in-process channel (Tauri events). Later it can become:
- A Unix domain socket (two-process mode)
- A TCP/QUIC connection (distributed mode)

The orchestrator and UI never bypass the bus. This discipline keeps the later split cheap — only the transport layer changes, not the message types or business logic.

---

## Message Reference

### UI → Orchestrator (Commands)

#### `ListSessions`
No payload. Requests all sessions. Orchestrator responds with `SessionList`.

#### `CreateSession`
```json
{ "name": "my-project", "agent_type": "claude", "home_folder": "./_workspace" }
```
- `name` — session label
- `agent_type` — must exist in agent registry
- `home_folder` — optional, defaults to `./_workspace`

Creates the home directory if it doesn't exist. Responds with `SessionCreated`.

#### `ListAgentTypes`
No payload. Requests all registered agent types. Orchestrator responds with `AgentTypes`.

#### `AttachToSession`
```json
{ "session_id": "uuid" }
```
Attaches the UI to a session. Orchestrator responds with `SessionAttached` containing the session info and all its workspaces.

#### `DetachFromSession`
```json
{ "session_id": "uuid" }
```
Detaches the UI from a session. Session and workspaces persist in memory.

#### `SpawnWorkspace`
```json
{ "session_id": "uuid", "agent_type": "claude", "folder": null }
```
- `session_id` — parent session
- `agent_type` — which agent to run
- `folder` — optional, defaults to session's `home_folder`

Creates a PTY, spawns the process, starts the reader thread. Responds with `WorkspaceSpawned`.

#### `TerminalInput`
```json
{ "workspace_id": "uuid", "bytes": [104, 101, 108, 108, 111] }
```
Sends raw bytes (keystrokes) to a workspace's PTY. No response — the PTY output comes back as `TerminalOutput` events.

#### `TerminalResize`
```json
{ "workspace_id": "uuid", "cols": 120, "rows": 40 }
```
Resizes a workspace's PTY via `TIOCSWINSZ`. The child process receives `SIGWINCH`.

### Orchestrator → UI (Events)

#### `SessionList`
```json
{ "sessions": [ { "id": "...", "name": "...", "home_folder": "...", "created_at": "..." } ] }
```

#### `SessionCreated`
```json
{ "session": { "id": "...", "name": "...", "home_folder": "...", "created_at": "..." } }
```

#### `AgentTypes`
```json
{ "types": [ { "name": "claude", "command": "claude", "description": "...", "default_args": [...] } ] }
```

#### `SessionAttached`
```json
{
  "session": { "id": "...", "name": "...", "home_folder": "...", "created_at": "..." },
  "workspaces": [ { "id": "...", "agent_type": "claude", "folder": "...", "cols": 80, "rows": 24, "running": true } ]
}
```

#### `WorkspaceSpawned`
```json
{ "workspace": { "id": "...", "session_id": "...", "agent_type": "claude", "folder": "...", "cols": 80, "rows": 24, "running": true } }
```

#### `TerminalOutput`
```json
{ "workspace_id": "uuid", "bytes": [27, 91, 51, 50, ...] }
```
Raw PTY output bytes. Sent continuously while the process is running. The UI writes these directly to xterm.js.

#### `WorkspaceExited`
```json
{ "workspace_id": "uuid", "code": 0 }
```
Process exited. The UI marks the workspace as stopped and shows "[Process exited]".

#### `WorkspaceError`
```json
{ "workspace_id": "uuid", "error": "Failed to spawn: no such file" }
```

#### `Status`
```json
{ "message": "Session created" }
```
General status info, shown in the status bar for 5 seconds.

#### `Error`
```json
{ "message": "Unknown agent type: foo" }
```
Error message, shown in the status bar.

---

## Orchestrator Logic

The orchestrator is the single source of truth for all state. It lives in `src-tauri/src/orchestrator.rs` and is protected by a `Mutex` in `AppState`.

### Message Dispatch

```rust
pub fn handle_message(&mut self, msg: BusMessage) {
    match msg {
        BusMessage::ListSessions => self.list_sessions(),
        BusMessage::CreateSession { name, agent_type, home_folder } => { ... }
        BusMessage::ListAgentTypes => self.list_agent_types(),
        BusMessage::AttachToSession { session_id } => { ... }
        BusMessage::DetachFromSession { session_id } => { ... }
        BusMessage::SpawnWorkspace { session_id, agent_type, folder } => { ... }
        BusMessage::TerminalInput { workspace_id, bytes } => { ... }
        BusMessage::TerminalResize { workspace_id, cols, rows } => { ... }
        _ => {} // Response messages are never received by orchestrator
    }
}
```

### Internal State

```
Orchestrator
├── bus: SharedBus               (for sending events to UI)
├── agent_registry: AgentRegistry (loaded from agents.toml)
├── sessions: HashMap<SessionId, Session>
│   └── Session
│       ├── info: SessionInfo
│       └── workspace_ids: HashSet<WorkspaceId>
├── workspaces: HashMap<WorkspaceId, WorkspaceInfo>
└── workspace_io: HashMap<WorkspaceId, WorkspaceIO>
    └── WorkspaceIO
        ├── master: Box<dyn MasterPty>  (for resize)
        ├── writer: Box<dyn Write>      (for input)
        └── child: Box<dyn Child>       (for kill/exit)
```

### PTY Spawning Flow (SpawnWorkspace)

1. Validate session exists and agent type is registered
2. Determine workspace folder (use session home if not specified)
3. Create directory if it doesn't exist
4. `native_pty_system().openpty(PtySize { rows: 24, cols: 80 })` — creates PTY pair
5. `CommandBuilder::new(agent.command)` + default args + set cwd
6. `pair.slave.spawn_command(cmd)` — launches child process
7. `pair.master.try_clone_reader()` — get output reader
8. `pair.master.take_writer()` — get input writer
9. Store master/writer/child in `workspace_io`
10. Spawn `std::thread` that reads from reader and sends `TerminalOutput` via bus
11. Send `WorkspaceSpawned` event back to UI

### Terminal I/O

**Input (UI → process):**
`TerminalInput` arrives → find workspace in `workspace_io` → `writer.write_all(&bytes)`

**Output (process → UI):**
Reader thread loops on `reader.read(&mut buf)` → sends `TerminalOutput { workspace_id, bytes }` via bus → on EOF, sends `WorkspaceExited`

**Resize:**
`TerminalResize` arrives → `master.resize(PtySize { rows, cols })` → child receives `SIGWINCH`

---

## UI Logic

The UI lives in `src/main.js` and manages two main classes:

### App (state machine)

States: `welcome` → `session`

- **welcome**: Shows sidebar + welcome message. Requests session list and agent types.
- **session**: Shows sidebar + session header + workspace panes. Attached to a session.

**Event handling:**
- `SessionList` → renders sidebar items
- `AgentTypes` → stores in memory (used by create dialog and workspace spawning)
- `SessionCreated` → auto-attaches to new session
- `SessionAttached` → switches to session view, spawns initial workspace
- `WorkspaceSpawned` → creates WorkspacePane instance
- `TerminalOutput` → routes to correct WorkspacePane by workspace_id
- `Error` / `Status` → updates status bar

### WorkspacePane (xterm.js binding)

Each workspace gets a `WorkspacePane` that:
1. Creates DOM structure: `.workspace-pane` > `.pane-header` (badge + status dot) + `.pane-content`
2. Initializes `Terminal` with dark theme
3. Loads `FitAddon` + `WebLinksAddon`
4. Opens terminal in `.pane-content`
5. Uses `ResizeObserver` to auto-fit → sends `TerminalResize`
6. Wires `terminal.onData` → encodes as `Uint8Array` → sends `TerminalInput`
7. Listens for `TerminalOutput` → writes bytes to terminal
8. Listens for `WorkspaceExited` → updates status dot, shows exit message

---

## File Structure

```
src-tauri/
├── agents.toml          # Agent type definitions (TOML)
├── src/
│   ├── messages.rs      # BusMessage enum, SessionInfo, WorkspaceInfo, AgentTypeInfo
│   ├── bus.rs           # Rust bus: Bus struct, SharedBus = Arc<Bus>
│   ├── agent.rs         # AgentRegistry: loads TOML, provides get/list/has
│   ├── orchestrator.rs  # Orchestrator: session/workspace CRUD, PTY I/O
│   └── lib.rs           # Tauri setup, AppState, bus_command handler

src/
├── bus.js               # JS bus: Bus class, invoke() + listen()
└── main.js              # UI: App class, WorkspacePane class, xterm.js

index.html               # Layout: sidebar, session header, panes, create dialog
```

---

## Supported Agents

| Agent | Command | Description |
|-------|---------|-------------|
| claude | `claude` | Anthropic Claude Code |
| opencode | `opencode` | OpenCode — open-source AI coding agent |
| gemini | `gemini` | Google Gemini CLI |
| codex | `codex` | OpenAI Codex CLI |
| copilot | `copilot` | GitHub Copilot CLI |

New agents can be added by editing `src-tauri/agents.toml`. The UI will list them automatically on next launch.
