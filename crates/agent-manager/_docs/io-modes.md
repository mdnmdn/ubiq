# I/O modes

When `am` runs an agent it can interact with it in two very different ways.
Which one applies is set per run (`--io` in the CLI, `IoModes` in lib mode) and
is constrained by what the harness supports (`Harness::io_support()`).

There are **two independent axes**:

- **Input mode** — how `am` *drives* the agent (feeds it prompts, answers tool
  approvals).
- **Output mode** — how `am` *exposes* the agent's activity to whatever is
  embedding it.

## Passthrough (default)

The simplest mode and the CLI default: the agent runs on a real PTY and its tty
is wired straight to the user's terminal. `am` **only configures and launches**;
the interaction is standard console, exactly as if the user had run the harness
directly.

```
  user's terminal  ⇄  PTY  ⇄  claude
        (am is invisible in the middle: forwards bytes, signals, resize, exit code)
```

Requirements for faithful passthrough:

- allocate a PTY; forward stdin/stdout/stderr byte-for-byte;
- propagate terminal resize (`SIGWINCH` → `TIOCSWINSZ`);
- forward signals (Ctrl-C, etc.) to the child;
- exit with the **child's** exit code.

In passthrough there is no structured input/output — `am`'s value is purely the
provisioning (skills/MCP/account/config-dir injection) that happened *before*
launch.

## Abstracted I/O

For embedding `am` in a larger tool (a web UI, a CI job, the Ubiq
multiplexer), passthrough isn't enough — the embedder wants structured events,
not a byte stream to screen-scrape. So `am` can replace the tty with a
structured channel.

### Input modes — how `am` talks to the agent

Each harness speaks its own wire protocol; `am` normalizes all of them to/from
the same `AgentInput`/`AgentEvent` model via a per-harness `IoBridge`
implementation. There is no single shared "structured" protocol on the wire —
`--io structured` just means "don't use the tty, use whatever protocol this
harness's bridge speaks":

| Harness         | Mechanism                                                                 |
|------------------|---------------------------------------------------------------------------|
| **Claude Code**  | stream-json (NDJSON): launch headless (`-p --input-format stream-json --output-format stream-json`); write the prompt as an NDJSON line on stdin; answer `control_request` tool-approvals with `control_response`. Contract fully spelled out in [`./harness/claude-code.md`](./harness/claude-code.md). |
| **codex**        | JSON-RPC over `codex app-server`: launch the `app-server` subcommand and exchange JSON-RPC requests/notifications over its stdio. See [`./harness/codex.md`](./harness/codex.md). |
| **opencode**     | NDJSON one-shot: launch `opencode run --format json`, which streams one NDJSON event per line and exits. See [`./harness/opencode.md`](./harness/opencode.md). |
| **GitHub Copilot** | NDJSON one-shot: launch headless (`-p --output-format json`), which streams one NDJSON event per line and exits. See [`./harness/copilot.md`](./harness/copilot.md). |

Input mode is picked to match the harness — you cannot drive one harness's
bridge with another's wire format. `Harness::io_support()` reports whether a
structured bridge is available (`structured: bool`); `resolve`/the CLI
rejects an impossible `--io structured` request with a clear error naming the
harness.

**Some harnesses take no second turn.** opencode and Copilot deliver the
prompt once, at launch (via argv), and run to completion; there is nothing
left to send once the process is up. Claude Code and codex, by contrast, stay
open for further prompts, cancellation, and permission answers across the
life of the process. `IoBridge::input` (below) is where this distinction
becomes a type-level signal rather than a fact a caller has to already know.

### Output modes — how `am` exposes the agent outward

Independently of how `am` drives the agent, it can normalize the agent's
activity into a protocol the embedder consumes:

| Mode          | Consumer                                   | What it emits                                        |
|---------------|--------------------------------------------|------------------------------------------------------|
| **ACP events**| ACP-aware clients / orchestrators          | Normalized ACP session updates (messages, tool calls, results). |
| **AG-UI events** | a web/UI front-end following the AG-UI event schema | UI-oriented events (streamed text, tool state, etc.). |

So a typical embedded run might be: **input = JSONL** (because the harness is
Claude Code) while **output = AG-UI** (because a web front-end is rendering it).
`am` sits in the middle translating: it reads Claude's stream-json events and
re-emits them as AG-UI events, and it takes UI input and writes it as Claude
stream-json on stdin.

```
   embedder ──(AG-UI in)──▶  am  ──(JSONL stdin)──▶  claude
   embedder ◀─(AG-UI out)──  am  ◀─(JSONL stdout)──  claude
```

`crate::io::{to_acp, to_agui}` are **core** (`src/io/acp.rs`, `src/io/agui.rs`)
— stateless, best-effort mappers from one `AgentEvent` to one ACP
`session/update` value / one AG-UI event value. `to_acp` is a rename rather
than a translation: the neutral model *is* ACP's `session/update` vocabulary
(see below), so mapping is moving the discriminant from `type` to
`sessionUpdate` and re-casing keys to camelCase, not reshaping data.
`to_agui` is a genuine translation onto a different schema, and stays a
stateless, one-event-in → one-value-out mapping: it does not emit AG-UI's
full run lifecycle framing (`TEXT_MESSAGE_START`/`END`, `RUN_STARTED`'s
`threadId`, …). Either way, an event with no reasonable representation on
the target protocol maps to `None` and is skipped by the caller. Selectable
on the CLI via `--io structured --output acp` or `--output agui` (alias
`ag-ui`); default `--output events` (or the flag omitted) keeps the raw
`AgentEvent` NDJSON, byte-for-byte.

## The neutral model

`AgentInput`/`AgentEvent` (`crate::io::{AgentInput, AgentEvent}`, core — no
feature gate) are `am`'s **harness-neutral** internal model, defined in
`crate::io::model`.

### The vocabulary is ACP's

`AgentEvent`'s variants and their fields *are* the Agent Client Protocol's
`session/update` vocabulary, minus two things: the JSON-RPC envelope, and the
session id. `refs/acp-protocol.md` is the full wire reference this
transcribes; this document doesn't restate its shapes, only how `am` uses
them.

**An event carries no session identity, deliberately.** Whoever holds the
table of live bridges attaches one: an embedder keys events by its own id
(Ubiq attaches a pane id and puts them on its bus), and a future `am acp`
server built on top of this would attach ACP's `sessionId` to the very same
event. Identity belongs to the multiplexer, never to the event — which is
what lets one bridge be read by several fronts (Ubiq's bus, an ACP server, a
recorded transcript) with no second mapping. `AgentEvent::SessionStarted`'s
`session_id` is not that identity: it is the *harness's own* id for the
conversation — what a resume needs, not what a consumer keys events by.

Three ACP `session/update` variants have no counterpart here because ACP
carries them at the protocol level instead of the event stream:
`SessionStarted` is the result of `session/new`, `PermissionRequest` is a
request back to the client (`session/request_permission`), and `TurnEnded`
is the `session/prompt` response's `stopReason`. They exist as `AgentEvent`
variants anyway, because a bridge has one stream, not a JSON-RPC peer with a
protocol layer to put them in.

### Content

```rust
pub enum Content {
    Text { text: String },
    Image { data: String, mime_type: String, uri: Option<String> },
    Audio { data: String, mime_type: String },
    ResourceLink { uri: String, name: String, mime_type: Option<String>, title: Option<String>, description: Option<String>, size: Option<u64> },
    Resource { resource: ResourceContents },
}
```

ACP's `ContentBlock` shape, unchanged. A harness that only speaks text
produces `Content::Text` and nothing else; the other variants exist so a
bridge that has richer content does not have to invent a shape for it.

### Tool calls

```rust
pub struct ToolCall {
    pub id: String,
    pub title: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    pub content: Vec<ToolContent>,
    pub locations: Vec<ToolLocation>,
    pub raw_input: Option<serde_json::Value>,
}

pub struct ToolCallUpdate {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<ToolKind>,
    pub status: Option<ToolStatus>,
    pub content: Option<Vec<ToolContent>>,
    pub locations: Option<Vec<ToolLocation>>,
    pub raw_output: Option<serde_json::Value>,
}
```

`ToolKind` is ACP's ten kinds (`Read`, `Edit`, `Delete`, `Move`, `Search`,
`Execute`, `Think`, `Fetch`, `SwitchMode`, `Other`); a bridge maps its
harness's tool names onto them and falls back to `Other` rather than
inventing an eleventh. `ToolStatus` is `Pending | InProgress | Completed |
Failed`, with `Pending` covering both "input still streaming" and "waiting
on a human".

**An absent field in `ToolCallUpdate` means unchanged, and `content`/
`locations` replace the whole collection rather than appending** — both are
ACP's rules, and a consumer that applies them the other way either loses
half an edit or reads a cleared field as still present.

### Config: models, modes, and thinking levels are one mechanism

```rust
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<ConfigCategory>,
    #[serde(flatten)]
    pub value: ConfigValue,  // Select { current_value, options } | Boolean { current_value }
}

pub enum ConfigCategory {
    Mode,
    Model,
    ModelConfig,
    ThoughtLevel,
    Other(String),
}
```

Upstream deprecated `session/set_mode` in favour of `session/set_config_option`
and never had dedicated model methods at all — a model, a mode, and a
thinking level are all just config options with a different `category` hint.
So the neutral model does not give models, modes, and thinking levels
separate mechanisms either: `AgentEvent::ConfigOptionUpdate { options }`
carries all of them, complete each time (setting one can change another —
picking a mode can change which models are on offer — so a partial update
would leave a stale picker on screen), and `AgentInput::SetConfigOption {
config_id, value }` changes any of them by id. A harness that grows a fifth
kind of knob needs no change here; `ConfigCategory::Other` accepts a name
this crate doesn't know about yet rather than failing to parse it.

`AgentEvent::CurrentModeUpdate { current_mode_id }` still exists alongside
this — it's ACP's now-legacy mode-only notification, kept for a harness that
only ever reports mode changes that way.

### Usage: suppressed rather than guessed

```rust
pub struct Cost { pub amount: f64, pub currency: String }

AgentEvent::UsageUpdate {
    used: u64,   // tokens currently occupying the context window
    size: u64,   // the context window's total size, in tokens
    cost: Option<Cost>,
    model: Option<String>,
}
```

`used / size` is the context ring a consumer draws — which is why no
consumer needs a context-window constant of its own. **The rule every bridge
follows: emit no `UsageUpdate` at all when the context window is unknown.** A
ratio with an invented denominator is worse than no ratio.

This is why the bridges don't agree on whether they report usage at all.
Claude Code's stream-json only states a model's context window once, in a
`result` event's `modelUsage.<model>.contextWindow` (camelCase — see
[`./harness/claude-code.md`](./harness/claude-code.md) §"Output stream
protocol"); `crate::io::jsonl::Mapper` remembers each model's window as it
learns one, and only then starts emitting `UsageUpdate` for that model's
per-message usage. Codex, opencode, and Copilot never report a context
window at all, so their bridges emit no `UsageUpdate` ever — not a
best-effort one with `size: 0`.

### Permissions

```rust
pub enum PermissionKind { AllowOnce, AllowAlways, RejectOnce, RejectAlways }

pub struct PermissionOption { pub option_id: String, pub name: String, pub kind: PermissionKind }

pub enum PermissionOutcome {
    Cancelled,
    Selected { option_id: String },
}
```

`AgentEvent::PermissionRequest { request_id, tool_call, options }` is
answered with `AgentInput::AnswerPermission { request_id, outcome,
updated_input }`. `updated_input` lets a caller rewrite the tool's input
before it runs — how a caller forces, say, a background command into the
foreground. Every pending permission request must be answered
`PermissionOutcome::Cancelled` when a turn is cancelled.

### The events, complete

```rust
pub enum AgentEvent {
    SessionStarted { session_id: Option<String>, model: Option<String>, mode: Option<String>, tools: Vec<String>, agents: Vec<String> },
    UserMessageChunk { content: Content, message_id: Option<String> },
    AgentMessageChunk { content: Content, message_id: Option<String> },
    AgentThoughtChunk { content: Content, message_id: Option<String> },
    ToolCall { #[serde(flatten)] call: ToolCall },
    ToolCallUpdate { #[serde(flatten)] update: ToolCallUpdate },
    Plan { entries: Vec<PlanEntry> },
    AvailableCommandsUpdate { commands: Vec<CommandInfo> },
    CurrentModeUpdate { current_mode_id: String },
    ConfigOptionUpdate { options: Vec<ConfigOption> },
    SessionInfoUpdate { title: Option<String>, updated_at: Option<String> },
    UsageUpdate { used: u64, size: u64, cost: Option<Cost>, model: Option<String> },
    PermissionRequest { request_id: String, tool_call: ToolCallUpdate, options: Vec<PermissionOption> },
    TurnEnded { stop_reason: StopReason, error: Option<String> },
    Log { level: String, message: String },
}
```

Serialized `#[serde(tag = "type", rename_all = "snake_case")]`, so a
`--io structured` run prints one tagged-JSON `AgentEvent` per line on stdout
(e.g. `{"type":"agent_message_chunk","content":{"type":"text","text":"…"}}`).
`ToolCall`/`ToolCallUpdate` flatten their fields beside the tag rather than
nesting under a wrapper key — this is exactly what makes the ACP projection
in `to_acp` a rename rather than a reshape.

### Input

```rust
pub enum AgentInput {
    Prompt { content: Vec<Content> },
    Cancel,
    AnswerPermission { request_id: String, outcome: PermissionOutcome, updated_input: Option<serde_json::Value> },
    SetConfigOption { config_id: String, value: ConfigSetting },
}
```

`Prompt` carries a `Vec<Content>` rather than a bare string so a caller can
send images/resources alongside text, wherever the harness's bridge
understands them; `AgentInput::prompt(text)` is the plain-text convenience
constructor. `Cancel` interrupts the turn in flight — every pending
permission request must then be answered `PermissionOutcome::Cancelled`.
`ConfigSetting` is `Text(String) | Flag(bool)`, matching `ConfigValue`'s two
shapes.

The Claude bridge maps `AgentInput`/`AgentEvent` to/from stream-json; the
codex bridge maps them to/from the `app-server` JSON-RPC contract; the
opencode and Copilot bridges map them to/from their respective one-shot
NDJSON streams. Keeping a neutral internal model is what lets input and
output modes be chosen independently.

## The `IoBridge` trait

Each per-harness bridge is an implementation of a small trait
(`crate::io::IoBridge`, core — no feature gate):

```rust
pub trait AgentInputSink: Send + Sync {
    fn send(&self, input: AgentInput) -> crate::Result<()>;
}

pub trait IoBridge: Send {
    fn send(&mut self, input: AgentInput) -> crate::Result<()>;
    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>>;
    fn input(&self) -> Option<Arc<dyn AgentInputSink>> {
        None
    }
}
```

`IoBridge` is `Send` because `next_event` **blocks**: an embedder that has
anything else to do puts the bridge on a thread of its own and reads it
there, rather than polling. That thread then owns `&mut self`, which is a
problem the moment a prompt needs to arrive from somewhere else — a UI
thread, an HTTP handler — while the pump thread is parked in `next_event`.
`input()` is the way out: it hands back a detached, `Send + Sync` handle
(`AgentInputSink`) that feeds the same underlying process from any thread,
independent of whoever owns the `IoBridge` itself. `JsonlBridge`
(`src/io/jsonl.rs`) shares its child's stdin as an `Arc<Mutex<Option<
ChildStdin>>>` between `send`, its `AgentInputSink`, and its own reader
thread's auto-allow writes, which is exactly the shape this seam is for; see
[am-as-library.md](./am-as-library.md) §6 for the embedder-facing pattern.

**A `None` from `input()` is a real capability signal, not an omission.**
Today `JsonlBridge` (Claude Code) and `CodexBridge` answer `Some` — both
harnesses stay open for a second prompt, so a caller can keep the process
and feed it more. `OpencodeBridge` and `CopilotBridge` answer `None`,
inherited from the trait default, because both harnesses are one-shot: the
prompt goes in via argv at launch, and there is no second turn to send into.
A caller that gets `None` learns this from the type rather than by writing
into a bridge that silently drops everything after the first prompt.

`crate::io::spawn_piped` (also core) is the shared entry point every
structured bridge uses to start its process: it builds a
`std::process::Command` from a `Launch`, applies `env_remove` then `env`, and
wires piped stdin/stdout (stderr inherited).

## Logging

Every bridge depends on `tracing` unconditionally — it is not feature-gated,
because a bridge with no logging is a bridge nobody can debug in the field.
Two levels matter:

- **`trace!`** — every raw harness frame, in both directions (what actually
  crossed stdin/stdout, before or after mapping).
- **`debug!`** — every mapped `AgentEvent`/`AgentInput`.

Raw frames sit one level below everything else because they carry prompts
and file contents — an embedder's default filter collects `debug` and
leaves `trace` out until someone asks for it by name.

## Phasing

- **Passthrough** — the CLI default. `IoModes` has one variant
  (`Passthrough`).
- **Structured** — the neutral `AgentInput`/`AgentEvent` model + `IoBridge`
  trait + `spawn_piped` helper are **core**; `IoModes::Structured` is wired
  through the CLI as `--io structured`. Concrete per-harness bridges exist
  for Claude Code (`src/io/jsonl.rs`), codex (`src/io/codex.rs`), opencode
  (`src/io/opencode.rs`), and GitHub Copilot (`src/io/copilot.rs`).
- **Output adapters** — `to_acp`/`to_agui` (`src/io/acp.rs`, `src/io/agui.rs`)
  project `AgentEvent` onto ACP `session/update` values and AG-UI event
  values respectively, selectable via `--output acp`/`--output agui`. Both
  are stateless, one-event-in → one-value-out mappers; a fuller, stateful
  adapter that tracks message/tool-call lifecycles and emits full protocol
  framing (JSON-RPC envelopes for a real ACP server, AG-UI's run/thread
  lifecycle events) is future work.
