---
id: inbox-acp-client
title: Problem — agent-manager speaks ACP outbound only, so no ACP harness can converse
kind: proposal
status: proposal
summary: The library projects AgentEvent onto ACP session/update for consumers to read, but has no ACP client to drive a harness that speaks it — so Grok, whose `grok agent stdio` is an ACP endpoint, reports structured:false, is dropped by harness_choices, and cannot be picked in the New agent dialog; the fix is a generic AcpBridge modelled on the landed Codex JSON-RPC bridge, after which Grok is configuration rather than code.
read_when: you are adding a harness that speaks ACP, wiring an inbound ACP client, or asking why Grok cannot be started as a conversation
updated: 2026-09-09
depends_on: [ref-acp-protocol, tech-agent-manager, tech-architecture, feat-chat, feat-workbench]
---

# Problem — agent-manager speaks ACP outbound only

The library has an ACP module, and it points the wrong way. `crates/agent-manager/src/io/acp.rs`
projects an `AgentEvent` **onto** an ACP `session/update` payload, for an ACP-aware consumer to
read — its own module doc is explicit that "this is still not an ACP endpoint": no JSON-RPC
envelope, no `sessionId`. It is selectable on a structured run as `--output acp`, and the roadmap's
"expose the agent via ACP" entry is the same direction again, an ACP *server* wrapping an
am-driven harness.

What no part of the tree has is the inverse: an ACP **client** that launches a harness which is
itself an ACP agent, speaks JSON-RPC at it, and turns the `session/update` notifications it pushes
back into `AgentEvent`s. Every landed bridge is harness-specific — NDJSON for Claude Code, opencode
and Copilot, JSON-RPC `app-server` for Codex — and none of them is ACP.

> ACP is a standard, and the vocabulary agent-manager already normalises to. It is the one wire a
> bridge could be written once for and reused by every harness that adopts it.

## 1. What this costs today, concretely

Grok is the case that surfaced it. `grok agent stdio` is an ACP endpoint, and the chain from that
fact to the interface is short and entirely mechanical:

1. `crates/agent-manager/src/harness/grok.rs:82` declares `structured: false, multi_turn: false`.
   The comment gives the reason as Grok's `--format json` NDJSON stream being too thinly documented
   to bridge faithfully — which is true of *that* stream, and beside the point now that there is an
   ACP one.
2. `crates/ubiq-host/src/agent.rs:284` sets `AgentTypeInfo.chat = harness.io_support().structured`,
   so the host reports Grok as `chat: false` while still reporting it `available`.
3. `WorkbenchState::harness_choices` (`crates/ubiq/src/state/workbench.rs:473`) filters
   `agent_types` down to `chat == true` before building its rows.
4. So the New agent dialog (`crates/ubiq/src/ui/new_agent.rs`) never draws Grok, and cannot: the
   dialog only ever sends `Message::StartConversation`, which needs a structured bridge to answer.

None of that is a bug, and no layer of it should be patched around. Grok still runs perfectly well
in a terminal pane — `new_pane_rows` keeps offering it, and two unit tests
(`crates/ubiq/src/state/workbench.rs:665`) pin exactly this split. **The one true statement in the
chain is `structured: false`, and the way to change the interface is to make it false no longer.**

The same holds for any future ACP harness, which is the reason to solve this generically rather
than for Grok.

## 2. Why this is cheaper than it looks

Three things are already in the tree, and together they cover most of the work.

**The neutral model *is* ACP's vocabulary.** `src/io/model.rs:8` says so, and it is a design
commitment rather than a coincidence: `AgentInput`/`AgentEvent` were built as ACP's `session/update`
vocabulary minus the JSON-RPC envelope and the session identity. So the inbound mapping is a reverse
rename and re-casing — camelCase to snake_case, `sessionUpdate` back to a discriminant — not a
translation. `to_acp` is ~544 lines of exactly that mapping, in the other direction, to read off.

**A newline-delimited JSON-RPC 2.0 client over stdio already exists.** `src/io/codex.rs` (~1300
lines) drives `codex app-server --listen stdio://`: a reader thread owning stdout for the bridge's
lifetime, request/response correlation by `id`, interleaved server-pushed notifications, and
server→client approval *requests* that carry an `id` and expect a reply. That last one is
structurally `session/request_permission`. Codex is the template, and it is a landed, tested one.

**The wire reference is written.** `_docs/references/acp-protocol.md` is a verified 14-section
reference covering framing, multiplexing, `initialize`, the session lifecycle, `session/prompt`,
all eleven `session/update` discriminants, tool calls, `session/request_permission`, the
modes-vs-models-vs-config resolution, and the client-side methods. Anyone building this should not
need to read the spec upstream.

## 3. What is actually missing

- **A `from_acp` inverse** of `to_acp` — one ACP `session/update` params value to one `AgentEvent`.
- **The handshake and session methods**: `initialize` (with version negotiation),
  `session/new`, `session/prompt`, `session/cancel`. `to_acp`'s module doc already names the three
  events that live at this protocol level rather than in the update vocabulary —
  `SessionStarted` is `session/new`'s result, `TurnEnded` is `session/prompt`'s response and its
  `stopReason`, and `PermissionRequest` is a request back to the client. Those three are precisely
  the seam a client has to own.
- **`session/request_permission`** mapped onto `PermissionRequest` *with a reply path* — the one
  place the bridge is bidirectional in a way the NDJSON bridges never are.
- **Client-side methods ACP lets an agent call back on us** (filesystem, terminal). Decide
  deliberately what to implement and what to refuse; §11 of the reference lists them.
- **A capability declaration** so `io_support()` can say "structured, via ACP" rather than the
  current per-harness boolean carrying an implicit protocol.

## 4. Shape worth arguing about first

Build it as a **generic `AcpBridge`**, not a `GrokBridge`. Then a new ACP harness is a
`harness_identity!` entry and a launch argv, not another 1300-line bridge — and Grok specifically
becomes configuration. This is the whole reason the problem is worth writing down rather than
fixing narrowly.

Two open questions that want a decision before code:

- **Sync or async.** Every existing bridge is deliberately synchronous `std::thread` + `serde_json`
  and **core** — compiling under `--no-default-features` with neither `pty` nor `cli`, so a lib-mode
  embedder can use it. The obvious reference implementations are async. Holding the core discipline
  means not reaching for the official `agent-client-protocol` crate or a futures runtime, and
  writing the JSON-RPC plumbing the way `codex.rs` already does. That is the recommendation, but it
  is a real trade and it is the first thing to settle.
- **Hand-rolled or SDK.** `refs/necoder/crates/acp_client` is an external reference checkout — a
  ~2000-line ACP client wrapping Claude Code via the official `agent-client-protocol` crate, with
  `futures`, `async-io` and its own `host` crate, and Japanese source comments. It is worth reading
  for the protocol *flow* (the initialize handshake and the permission round-trip are live-verified
  there). It is not a drop-in: its dependency shape is the opposite of the core discipline above.

## 5. Notes for whoever picks this up

- **Pin the handshake against real output before writing the mapping.** Grok's ACP behaviour has
  not been observed here — the binary is not installed on this machine, and `grok agent stdio` is
  the user's report rather than something verified in this tree. Capture a real session first; the
  test-run documents under `crates/agent-manager/_docs/test-runs/` are the existing pattern for
  writing down what a harness actually emitted.
- **`crates/agent-manager/_docs/harness/grok.md` does not mention ACP at all.** It documents the
  `--format json` NDJSON stream and the MCP `stdio` transport config, so it is out of date on the
  launch mode that matters here. It needs the `agent stdio` mode added once the capture exists.
- **`_docs/architecture.md:122` claims opencode has an "ACP launch mode."** `OpencodeBridge` is
  plain NDJSON, so either the document is wrong or opencode grew a mode nothing uses. Worth
  resolving while here — if opencode does speak ACP, it is a second consumer of the generic bridge
  and stops being a one-harness argument.
- Nothing in `crates/ubiq` or `crates/ubiq-proto` should need to change. When `structured` flips
  true for an ACP harness, the host reports `chat: true`, `harness_choices` stops filtering it, and
  the New agent dialog draws it with no UI work at all. That is the test that the layering held.
