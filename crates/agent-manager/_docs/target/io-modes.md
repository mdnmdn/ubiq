# I/O modes

When `am` runs an agent it can interact with it in two very different ways.
Which one applies is set per run (`--io` in the CLI, `IoModes` in lib mode) and
is constrained by what the harness supports (`Harness::io_support()`).

There are **two independent axes**:

- **Input mode** — how `am` *drives* the agent (feeds it prompts, answers tool
  approvals).
- **Output mode** — how `am` *exposes* the agent's activity to whatever is
  embedding it.

## Passthrough (default, Phase 1)

The simplest mode and the CLI default: the agent runs on a real PTY and its tty
is wired straight to the user's terminal. `am` **only configures and launches**;
the interaction is standard console, exactly as if the user had run the harness
directly.

```
  user's terminal  ⇄  PTY  ⇄  claude
        (am is invisible in the middle: forwards bytes, signals, resize, exit code)
```

Requirements for faithful passthrough (Phase-1 acceptance criteria):

- allocate a PTY; forward stdin/stdout/stderr byte-for-byte;
- propagate terminal resize (`SIGWINCH` → `TIOCSWINSZ`);
- forward signals (Ctrl-C, etc.) to the child;
- exit with the **child's** exit code.

In passthrough there is no structured input/output — `am`'s value is purely the
provisioning (skills/MCP/account/config-dir injection) that happened *before*
launch.

## Abstracted I/O (Phase 2+)

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
| **Claude Code**  | stream-json (NDJSON): launch headless (`-p --input-format stream-json --output-format stream-json`); write the prompt as an NDJSON line on stdin; answer `control_request` tool-approvals with `control_response`. Contract fully spelled out in [`../harness/claude-code.md`](../harness/claude-code.md). |
| **codex**        | JSON-RPC over `codex app-server`: launch the `app-server` subcommand and exchange JSON-RPC requests/notifications over its stdio. See [`../harness/codex.md`](../harness/codex.md). |
| **opencode**     | NDJSON one-shot: launch `opencode run --format json`, which streams one NDJSON event per line and exits. See [`../harness/opencode.md`](../harness/opencode.md). |

Input mode is picked to match the harness — you cannot drive one harness's
bridge with another's wire format. `Harness::io_support()` reports whether a
structured bridge is available (`structured: bool`); `resolve`/the CLI
rejects an impossible `--io structured` request with a clear error naming the
harness.

> **Note:** "ACP" and "AG-UI" are **Phase 3 output adapters** layered over the
> neutral `AgentEvent` model (see "Output modes" below) — they are not a P2
> input mechanism. No harness here is driven "over ACP"; each is driven over
> its own native protocol as listed above.

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

### The `IoBridge` trait

Each per-harness bridge is an implementation of a small trait
(`crate::io::IoBridge`, core — no feature gate):

```rust
pub trait IoBridge {
    /// Feed the agent one unit of input (a prompt, a tool-approval answer).
    fn send(&mut self, input: AgentInput) -> crate::Result<()>;
    /// Pull the next normalized event, or `None` at end of stream.
    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>>;
}
```

`AgentInput` / `AgentEvent` (`crate::io::{AgentInput, AgentEvent}`, also core)
are `am`'s **harness-neutral** internal model:

```rust
pub enum AgentInput {
    Prompt { text: String },
    ApproveTool { request_id: String, decision: ApprovalDecision, updated_input: Option<serde_json::Value> },
    Interrupt,
}
pub enum AgentEvent {
    SessionStarted { session_id: Option<String> },
    AssistantText { text: String },
    Thinking { text: String },
    ToolCall { id: Option<String>, name: String, input: serde_json::Value },
    ToolResult { id: Option<String>, content: serde_json::Value },
    ApprovalRequest { request_id: String, tool_name: String, input: serde_json::Value },
    Usage { input_tokens: Option<u64>, output_tokens: Option<u64> },
    Result { success: bool, error: Option<String> },
    Log { level: String, message: String },
}
```

Both derive `Serialize`/`Deserialize` with `#[serde(tag = "type")]`, so a
`--io structured` run prints one tagged-JSON `AgentEvent` per line on stdout
(e.g. `{"type":"assistant_text","text":"…"}`). Note `AgentInput::Prompt` is a
*struct* variant (`{ text: String }`), not a tuple newtype — serde's
internally tagged representation can't merge a bare scalar into the tag
object, only a map/struct, so every variant carries named fields.

The Claude bridge maps `AgentInput`/`AgentEvent` to/from stream-json; the
codex bridge maps them to/from the `app-server` JSON-RPC contract; the
opencode bridge maps them to/from `opencode run --format json`'s NDJSON; a
future AG-UI *output* adapter maps `AgentEvent` to the AG-UI schema. Keeping a
neutral internal model is what lets input and output modes be chosen
independently.

`crate::io::spawn_piped` (also core) is the shared entry point every
structured bridge uses to start its process: it builds a
`std::process::Command` from a `Launch`, applies `env_remove` then `env`, and
wires piped stdin/stdout (stderr inherited).

## Phasing

- **Phase 1** — passthrough only. `IoModes` has one variant (`Passthrough`).
- **Phase 2** — the neutral `AgentInput`/`AgentEvent` model + `IoBridge` trait
  + `spawn_piped` helper land as **core** (this step, C1); `IoModes::Structured`
  is added to the spec and `--io structured` is wired through the CLI.
  Concrete per-harness bridges (Claude stream-json, codex `app-server`
  JSON-RPC, opencode NDJSON) land in follow-up steps (C2/C3/C4) — until a
  given harness's bridge lands, `Harness::io_support().structured` is `false`
  and `Harness::structured_bridge` returns a "not supported yet" error by
  default.
- **Phase 3** — ACP and AG-UI **output** adapters; the web/headless surface that
  consumes them.
