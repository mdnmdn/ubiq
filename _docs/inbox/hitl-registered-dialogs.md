---
id: inbox-hitl-dialogs
title: "Proposal — Human-in-the-loop with no parked call: armed dialogs that fire at turn end"
kind: proposal
status: proposal
summary: A second ask mode in which the agent registers a dialog through an MCP tool that returns immediately, Ubiq arms it against the conversation, and the modal is raised when the turn ends rather than while a tool call is parked — so no harness timeout can reach it. The registration call is itself the structured signal, which removes the need for a text sentinel in the agent's last message; `ask_user_question` and `crates/ubiq-host/src/ask.rs` stay for the mid-sequence pauses the new mode cannot serve.
read_when: you are building the turn-end ask mode, changing `ubiq-ask`, serving ACP elicitation, or deciding how an agent asks the user a question
updated: 2026-09-24
depends_on: [tech-transport, tech-agent-manager, ref-acp-protocol, feat-chat, tech-architecture]
code_anchors:
  - crates/ubiq-host/src/ask.rs
  - crates/ubiq-host/src/mcp/ask.rs
  - crates/ubiq/src/state/ask.rs
  - crates/agent-manager/src/io/acp_client.rs
  - crates/agent-manager/src/io/jsonl.rs
---

# Proposal — Human-in-the-loop with no parked call

`ubiq-ask`'s `ask_user_question` is the one tool in Ubiq that does not answer itself. The call
parks on `crates/ubiq-host/src/ask.rs`'s `Asks` table for up to `ASK_TIMEOUT_SECS` (one hour) while
a person is asked, on a thread of its own so the listener stays free (`D138`, `D155`). The harness
in front of that call has a timeout of its own and it is much shorter than an hour, so the harness
gives up before the human does. T-87 asks how to make that timer longer; this proposal asks the
better question — how to ask a human with **no parked call at all**, so there is no timer to fight.

The answer proposed here is **arm-and-fire**: the agent registers a dialog through a tool that
returns in milliseconds, Ubiq holds the registration armed against that conversation, and the modal
is raised when the turn ends. The user's answer is sent back as the next prompt.

---

## 1. What the protocols already offer

### ACP has a first-class elicitation channel, and Ubiq refuses it

ACP v1 defines `elicitation/create` as a client method beside `session/request_permission`, with
two modes: `form` (a flat JSON Schema of primitives and enums, rendered as a form) and `url`. The
client advertises it at `initialize` under `clientCapabilities.elicitation`, with `form` and `url`
sub-objects; an omitted or null field means unsupported. `elicitation/complete` is the agent's
notification that an out-of-band URL interaction finished.

`crates/agent-manager/src/io/acp_client.rs` advertises `fs.readTextFile`, `fs.writeTextFile` and
`session.configOptions.boolean`, and **deliberately not** `terminal` or `elicitation`; its
`serve_request` answers `elicitation/create` `-32601 method not found`, and `take_notification`
drops `elicitation/complete`. That refusal is protocol-legal precisely because the capability was
never advertised.

This channel has the property the whole proposal is chasing: the agent issues a JSON-RPC request
and **blocks on it with no timeout of its own** — the module docs state this for
`session/request_permission`, and the same machinery carries elicitation. There is no parked MCP
call and nothing to time out. It is strictly better than anything below, on the paths that have it.

It is also not sufficient on its own, for two reasons. It is agent-initiated: Ubiq cannot make an
agent elicit, only answer one that does, and the ACP agents Ubiq drives are not observed emitting
it. And it does not exist on the native path at all.

### The native `claude-code` path has no equivalent

`crates/agent-manager/src/io/jsonl.rs` reads Claude Code's `stream-json`. The only
`control_request` subtype it maps is `can_use_tool`, which becomes an `AgentEvent::PermissionRequest`
and stays outstanding until the caller answers; every other subtype is answered an error by the
reader, because nothing downstream would ever answer it. `crates/agent-manager/_docs/harness/claude-code.md`
records exactly two subtypes on the wire — `can_use_tool` inbound and `interrupt` outbound. There is
no elicitation subtype and no general "ask the user" request.

`can_use_tool` is not a substitute. It is scoped to one tool call's allow/deny with
`permission_suggestions`, not to an arbitrary question, and it is the mechanism Ubiq already spends
on permission modes.

### MCP elicitation does not reach either path

MCP's own `elicitation/create` is a **server→client** request. Two things block it here.

Ubiq's MCP listener (`crates/ubiq-host/src/mcp/server.rs`) is streamable HTTP with one JSON body per
`POST`, **stateless, deliberately** — no session id, no SSE stream, nothing remembered between two
requests. A server→client request has no channel to travel on; serving elicitation means adding an
SSE response arm to the listener and a session layer the module's own header argues against.

And the client half is missing where it matters most. Claude Code's interactive TUI renders
elicitation dialogs — its backgrounding documentation says a call waiting on an open elicitation
dialog is not backgrounded — but Ubiq's conversations run Claude Code **headless in stream-json
mode**, where no `control_request` subtype forwards one. The repeated feature requests against
`anthropics/claude-code` for elicitation were closed as duplicates or not planned.

Even if both were solved, MCP elicitation **still leaves the `tools/call` parked**. It makes the
wait legible to a client that knows how to suspend its timer; it does not remove the wait. It is
therefore a T-87 mitigation, not an answer to this card.

### Conclusion on protocols

| Path | First-class HITL channel | Usable today |
|---|---|---|
| ACP (`opencode`, `gemini`, `grok`, `claude-code-acp`) | `elicitation/create`, form mode | Spec yes, Ubiq refuses it, agents not observed using it |
| Native `claude-code` stream-json | none | no |
| MCP over Ubiq's listener | `elicitation/create` | no — stateless HTTP, and the harness does not forward it |

So a portable mechanism cannot be built out of an existing HITL channel. It has to be built out of
what every path does have: **a tool call, and the end of a turn**.

---

## 2. The sentinel is unnecessary

The card's shape puts a verbatim marker — `ASK-UI feedback-for-modal-detail` — in the agent's last
message, because a text sentinel is the one carrier every harness has. That is true of *text*, and
it is the wrong question. The register call is already a structured, in-band event that Ubiq
**serves itself**: `POST /mcps/<agent-id>/ubiq-ask` arrives at
`crates/ubiq-host/src/mcp/ask.rs` with the agent's identity in the URL path and its arguments
validated against `ubiq_proto::ask::check`. Ubiq does not need the model to tell it that a dialog
was registered; Ubiq is the thing that registered it.

Can structured metadata ride the *end* of a turn instead? No, on both paths. ACP's `PromptResponse`
carries only `stopReason`, with no `_meta`; `ContentChunk` has no chunk-level extension field
either. Claude Code's `result` line carries spend, cost and `modelUsage`, nothing an agent authors.
So a *sentinel* would genuinely have to be text — which is the argument for not needing one.

Dropping it removes every failure mode the card asks about:

| Sentinel failure | Under arm-and-fire |
|---|---|
| Spoofed by file content, a diff, a quoted transcript | Cannot happen — the trigger is an authenticated call on Ubiq's own loopback listener, keyed by the agent id in the path |
| Malformed | The registration is schema-checked at the call and refused as an in-band `isError` the model can read and correct, exactly as `ask_user_question` refuses an undrawable ask today |
| Emitted mid-turn | Registration *is* mid-turn, by design; firing is deferred to turn end |
| Duplicated | Several registrations arm several dialogs; they are raised in registration order |
| Names an unknown id | Impossible — Ubiq mints the id and returns it |
| The model forgets to emit it | There is nothing to forget: registering is the whole act |

The one thing the sentinel would buy is *selecting* which of several registered dialogs to raise.
That problem is designed away instead: **a registration is armed for the current turn only.**
Everything armed when the turn ends is raised; nothing survives into the next turn. There is no
durable id space to resolve, and `feedback-for-modal-detail` never has to be a name the model
remembers — the returned handle exists only so the model can cancel its own registration.

---

## 3. The mechanism

1. The agent calls `ubiq-ask`'s new `register_question` with the same `questions` array
   `ask_user_question` takes. The host validates it with `ubiq_proto::ask::check`, mints an `AskId`,
   files it as **armed** against that `AgentId`, and returns `{"registered": "<ask id>"}` on the
   listener's own thread. No thread is spawned, nothing parks.
2. The agent carries on, and ends its turn however it likes. There is no sentinel, no required
   phrasing and no last-message contract.
3. `crates/ubiq-host/src/conversation.rs` already sees `AgentEvent::TurnEnded` and turns it into
   `ConvUpdate::TurnEnded`. On that event, every dialog armed for the conversation is raised as
   `Message::AskUser` — **the message that exists today, unchanged**.
4. The window draws it with the modal that exists today. `crates/ubiq/src/state/ask.rs`'s
   `AskRecord`, `AskDraft`, `AskStage` and the dialog over them need no change: an armed ask is
   `AskStage::Waiting` like any other, and the answer goes back as `Message::AnswerAsk`, unchanged.
5. The host, receiving `AnswerAsk` for an *armed* id rather than a *parked* one, renders the
   `AskOutcome` into prose and submits it as the next turn — the same path `Message::PromptAgent`
   takes. `AskOutcome::Chat` arms nothing and sends nothing: the user talks instead.

The UI does not change. The transport does not change. The whole of the new surface is one MCP
tool, one table of armed asks beside `Asks`, one branch on `TurnEnded`, and one branch in the
`AnswerAsk` handler.

**Registration does not need MCP for the dialog *spec*** — the spec could ride a text block the way
the sentinel would. It should use MCP anyway: the tool call is the only carrier that is
authenticated, schema-validated, refusable with an error the model reads, and invisible in the
transcript. Registering over text would reintroduce every sentinel failure to carry the payload as
well as the trigger.

---

## 4. The weakness, head on

The answer arrives as a **user message**, not a tool result. The card names this and it is real,
but it is narrower than it sounds.

**What is not lost.** Ending a turn does not clear the conversation. The model's full history —
every `tool_use` and its `tool_result` — is replayed on the next turn on both paths. The answer is
appended to a complete record, not delivered to an amnesiac.

**What is lost** is mid-sequence resumption. The model cannot be at step three of a five-step
mechanical loop, pause, and continue at step four with its local plan intact. It must decide to
stop, and re-derive on the next turn what it was doing. Three consequences follow: a model that
ends a turn mid-sequence sometimes re-reads files it had already read; a model may restate its plan
rather than resume it; and anything holding an open resource — a terminal session, a half-applied
edit set, a transaction — cannot be paused this way at all.

**How bad, in practice.** The cases `ask_user_question` is actually used for are
clarification-before-work: which approach, which branch, is this diff what you meant, should I keep
going. Those questions are asked at a natural turn boundary anyway, and for them arm-and-fire is
not a compromise — it is the more honest shape, because the model was going to stop regardless.

**What it rules out.** Per-item confirmation inside a long mechanical sequence — approve each of
forty file moves, confirm each destructive step — and anything mid-tool. For those the parked call
is the correct shape and its timeout is a real constraint to be mitigated, which is T-87's job.

This is why the recommendation is **a second mode, not a replacement.**

---

## 5. Recommendation

**Build arm-and-fire as a second `ubiq-ask` tool. Keep `ask_user_question` and
`crates/ubiq-host/src/ask.rs` exactly as they are. Drop the sentinel.**

`register_question` becomes the tool the harness prompt points at first, because it cannot time out
and costs the harness nothing; `ask_user_question` stays for the pauses that must happen mid-turn,
and T-87's mitigations still apply to it. The two share the vocabulary in `ubiq_proto::ask`, the two
transport messages, and the whole of the dialog in `crates/ubiq`. The only thing that differs is
what the outcome is delivered into: a channel, or a prompt.

**What it costs.**

- **Latency the user sees.** A dialog registered early in a turn is not raised until the turn ends,
  which may be many tool calls later. The tool's description has to say "register this as the last
  thing you do", and a model that ignores it produces a question that arrives late rather than one
  that arrives wrong.
- **No mid-sequence pause**, as §4 sets out. Two modes is the price of covering both.
- **Two tools doing similar things**, which a model can pick wrongly between. The descriptions carry
  that weight, and picking `ask_user_question` wrongly degrades to today's behaviour.
- **A turn that ends without a dialog the model expected** is silent: if the model registers and
  then the turn fails, the armed row must be dropped, not raised over an error.
- **A second delivery format to write** — the outcome as prose for a prompt, not JSON for a tool
  result — and the transcript has to show the dialog and its answer rather than that prose verbatim.

**Where it works.** Everywhere Ubiq runs a conversation, on every harness, today: it needs only an
MCP tool call and a turn boundary, both of which every path in `crates/agent-manager/src/io/` already
produces. There is no harness-capability gate and no fallback, because there is nothing to fall back
from — `ask_user_question` is not a fallback, it is the other mode.

**The one thing worth building beside it.** Advertise `clientCapabilities.elicitation.form` in
`acp_client.rs` and serve `elicitation/create` into the same modal. It is the only path with no
parked call *and* no turn boundary, so it beats arm-and-fire wherever an agent uses it. It is
additive, it is smaller than this proposal, and it is agent-initiated — which is why it cannot be
the answer on its own.

---

## Related docs

- [`tech/transport-contract.md`](../tech/transport-contract.md) — `AskUser`, `AnswerAsk`, `AskEnded`
- [`references/acp-protocol.md`](../references/acp-protocol.md) — what ACP says on the wire
- [`tech/agent-manager.md`](../tech/agent-manager.md) — the boundary the two IO bridges sit behind
- [`features/chat.md`](../features/chat.md) — the transcript the dialog is anchored into

## Next steps

- Decide whether `register_question` replaces `ask_user_question` in the default harness prompt.
- Confirm against a live `opencode` and `grok` session whether either ever emits `elicitation/create`.
- Decide what the transcript shows for a registered dialog and its answer.
