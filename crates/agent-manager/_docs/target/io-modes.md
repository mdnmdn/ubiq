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

| Mode      | Applies to                        | Mechanism                                                                 |
|-----------|-----------------------------------|---------------------------------------------------------------------------|
| **JSONL** | harnesses with a stream-json contract (e.g. Claude Code) | Launch headless (`-p --input-format stream-json --output-format stream-json`); write the prompt as an NDJSON line on stdin; answer `control_request` tool-approvals with `control_response`. Contract fully spelled out in [`../harness/claude-code.md`](../harness/claude-code.md). |
| **ACP**   | harnesses that speak the Agent Client Protocol (opencode, codex, …) | Launch in ACP mode; exchange ACP JSON-RPC messages over the harness's ACP transport. |

Input mode is picked to match the harness — you cannot drive a JSONL-only
harness over ACP or vice-versa. `Harness::io_support()` reports what is possible;
`resolve` rejects an impossible `--io` request with a clear error.

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

Each mode is an implementation of a small bridge trait that `run` drives:

```rust
pub trait IoBridge {
    /// Feed the agent one unit of input (a prompt, a tool-approval answer).
    fn send(&mut self, input: AgentInput) -> Result<()>;
    /// Pull the next normalized event from the agent.
    fn next_event(&mut self) -> Result<Option<AgentEvent>>;
}
```

`AgentInput` / `AgentEvent` are `am`'s **harness-neutral** internal model. The
JSONL bridge maps them to/from Claude's stream-json; the ACP bridge maps them
to/from ACP; the AG-UI *output* adapter maps `AgentEvent` to the AG-UI schema.
Keeping a neutral internal model is what lets input and output modes be chosen
independently.

## Phasing

- **Phase 1** — passthrough only. `IoModes` exists but has one variant.
- **Phase 2** — JSONL input (Claude) and ACP input (opencode/codex) behind the
  `Harness` trait; the neutral `AgentInput`/`AgentEvent` model.
- **Phase 3** — ACP and AG-UI **output** adapters; the web/headless surface that
  consumes them.
