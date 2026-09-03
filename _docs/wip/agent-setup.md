---
id: wip-agent-setup
title: Wiring a real agent into the agent pane
kind: wip
status: draft
summary: The protocol, the library work and the order of packages that turn the mocked agents screen into a real conversation with a composed harness — and the honest inventory of what today's library cannot yet deliver.
read_when: you are picking up the next agent-integration package, or judging whether a proposed conversation message belongs on the wire
updated: 2026-09-03
verified: 2026-09-03
code_anchors: [crates/ubiq-host/src/agent.rs, crates/agent-manager/src/io/model.rs, crates/agent-manager/src/io/structured.rs, crates/ubiq-proto/src/work.rs, crates/ubiq/src/state/chat.rs, crates/agent-manager/src/profile.rs]
depends_on: [tech-agent-manager, feat-workbench, feat-chat]
review_cycle: monthly
---

# Wiring a real agent into the agent pane

## Where this starts

A workspace is composed by the library and confined by default: the spawn path runs `RunSpec` →
provision → `Launch` → pseudo-terminal, and a pane shows the harness's own screen. That is the
**passthrough** half. The agents screen, the chat panel and the orchestration graph draw a mock: the
composer sends `SendToAgent`, the host appends the user's own line to a fixture `WorkAgent.thread`
and answers with the record, and nothing generates a reply.

This document is the plan for the other half — a harness driven with structured events, rendered as
a conversation — and for the four things around it the user needs before that is usable: agent
definitions, credentials, a default home, and a debug surface.

## The inventory, and what each item costs

Read this table before designing anything. Everything in it was verified against the tree, and four
rows contradict what a reader would reasonably assume.

| What is true today | What it costs us |
|---|---|
| `AgentId` **is** `WorkspaceId` — one type, deliberately, "until a workspace outlives its pane" | Nothing to reconcile: a real agent and a pane are already one identity |
| `IoBridge` is two methods, both `&mut self`, and `next_event` **blocks** | The host needs one pump thread per structured workspace; `send` and `next_event` cannot be called concurrently without splitting the bridge |
| Only **Claude** and **Codex** accept input after launch. opencode and Copilot bridges are one-shot: the prompt goes in through argv, `send` is a no-op | Only two harnesses can back a conversation column. The other two are single-turn runs wearing the same trait |
| **Every bridge auto-approves.** Claude's reader answers every `control_request` with `allow`; Codex auto-accepts every approval RPC; opencode runs `--dangerously-skip-permissions`; Copilot runs `--allow-all --no-ask-user` | A permission prompt in the UI would be theatre — the tool has already run. This is the one item with a security consequence, and it gates any "ask me first" feature |
| No mode, no reasoning effort, no model switch exists in the library. `Policy` carries an opaque `permission_mode` string, passed through per harness | Modes and thinking levels have no home yet. `ModelInfo` is `id` + `description` + `default`, discovered per harness |
| `to_acp` is a **stateless one-event mapper** that drops `ApprovalRequest`, `Usage`, `Result` and `SessionStarted`, and emits no JSON-RPC envelope, no session id and no turn brackets | There is no ACP endpoint today. The vocabulary is right; the protocol is not implemented |
| `AgentEvent` has no turn boundary, no delta-versus-whole distinction, no tool verb, no diff, no plan | The chat panel's `Block`, `ToolCall` and `DiffLine` cannot be filled from it without inventing the missing half in the host |
| `WorkAgent.thread` is `Vec<Turn>` of `{ from, text }`, replaced whole on every `AgentChanged` | A token stream would re-send the entire conversation per token |
| The library cannot kill a process **group** — it is `#![forbid(unsafe_code)]` with no `libc` | A cancelled turn can leave grandchildren. multica solves this with process groups; we cannot copy that directly |
| The mapper reads four `assistant` content-block kinds and ignores every other field of every event | Tokens, cost, subagent provenance, rate limits and stop reasons are all on the wire and none of them reach a consumer. The next section is what is actually there |
| `AgentParams` is named in the library's own architecture doc and **does not exist** | Corrected in this pass. The type the docs promised is exactly what packages P3 needs |

## The shape: one workspace, two faces

The decision everything else hangs off. A workspace is one composed run of one harness. It has a
**face**, chosen when it is spawned and never after:

- a **terminal face** — passthrough, a pseudo-terminal, drawn as a pane. What ships today.
- a **conversation face** — structured I/O over pipes, drawn as a column on the agents screen or as
  the chat panel's transcript.

`IoModes` in the library already is this choice, and `AgentId = WorkspaceId` already says the two are
one thing. So the wire needs no new identity and no second lifecycle: `SpawnWorkspace` grows a face,
`CloseWorkspace` closes either, and `PaneExited` and its conversation sibling both mean "the harness
is gone". A harness cannot have both faces at once — a child's stdout is either a terminal or a pipe
— and pretending otherwise is what a second identity would cost us.

## Decision 1 — ACP-shaped, not ACP-transported

**The vocabulary is ACP's. The transport stays the in-memory bus.** `D9` says Ubiq embeds the
library rather than shelling out to `am`; putting JSON-RPC between two halves of one process would
undo that for no gain. But the *names* and the *event shapes* should be ACP's, in three places: the
library's neutral event model, the bus family, and the mapper that already exists.

Three reasons, in order of weight:

1. **Nine harnesses in the library's reference table speak ACP natively** — `hermes`, `kimi`,
   `kiro` and `qoder` are launched as `<binary> acp`. An ACP-shaped neutral model makes an inbound
   ACP bridge a *reader of its own vocabulary* instead of a third translation.
2. **`io/acp.rs` becomes a real adapter rather than a lossy projection**, which is what makes
   `am --output acp` worth having for a client that is not Ubiq.
3. **The UI's render model is already ACP-shaped** by coincidence: `agent_message_chunk` is a
   markdown `Block`, `agent_thought_chunk` is a thinking block, `tool_call` and `tool_call_update`
   are a `ToolCall` with its status and diff.

The vocabulary to adopt, from the published schema — `refs/acp-protocol.md` is the reference, and
`refs/multica` holds no ACP code, so an earlier claim that these were confirmed against its clients
was unsupported:

| ACP | Meaning | Where it lands here |
|---|---|---|
| `initialize`, `authenticate` | capability exchange, login state | `Harness` capability query; the login pane in P5 |
| `session/new`, `session/load` | start, and reattach while replaying the history first | spawning a conversation face; `SessionMeta.harness_session_id`. `session/load` is the canonical resume; `session/resume` is one agent family's deviation |
| `session/prompt`, `session/cancel` | a turn, and its interruption | `AgentInput::Prompt`, `AgentInput::Interrupt` |
| `session/update` with `sessionUpdate:` | everything streamed back | the new `AgentEvent` variants |
| `agent_message_chunk`, `agent_thought_chunk` | assistant text, reasoning | `AssistantText`, `Thinking` — already named this way in `to_acp` |
| `tool_call`, `tool_call_update` | a tool's start and its progress or completion | `ToolCall`, `ToolResult`, with the fields the UI's blocks need |
| `plan` | the agent's todo list, flat, each entry with a priority and a status | the orchestration screen's steps, and a block in a transcript |
| `session/request_permission` | ask the human, embedding the whole tool call so the client can show what it is authorising | a real `ApprovalRequest`, with options — P7 |
| `session/set_config_option`, `config_option_update` | every session-level knob the harness advertises — the model, the mode, the thinking level — as one list. `session/set_mode` is the deprecated predecessor | P3 |
| `fs/read_text_file`, `fs/write_text_file`, `terminal/*` | the agent asking the *client* to act | deferred: this is what Ubiq's own MCP surface would answer |

Two corrections to what a reader would assume from the harness documents.
**`session/set_model`, `availableModels` and `currentModelId` are not in the schema at all** — but a
model picker is still core ACP, expressed as a config option whose `category` is `model`. That is
one mechanism for the model, the mode, the model's parameters and the thinking level, and it is what
`session/set_mode` was deprecated in favour of. And ACP's `terminal/*` is a side-channel for the
agent to run a command and read its output — there is no method to type into it and none to resize
it, so it is not a pane and cannot host an interactive harness. That stays Ubiq's job.

What ACP does not give us, and stays ours: which pane a conversation is drawn in, which project it
belongs to, and the arrangement over it. That is the same split `tech/agent-manager.md` already
draws.

## Decision 2 — discovered vocabularies, never a flattened enum

Models, thinking levels and modes are **lists a harness advertises**, each entry an id, a label and
an optional description. Not enums, not a shared taxonomy, and not the four constants
`crates/ubiq/src/state/chat.rs` holds today.

multica learned this the hard way and wrote the reason down: Claude's effort levels are
`low|medium|high|xhigh|max` and Codex's are `none|minimal|low|medium|high|xhigh`, `xhigh` is
Opus-only, `max` is session-only, and opencode's variants can be extended by local config. What the
user picks has to **round-trip exactly** through the harness's own vocabulary, so a shared enum
would either lie or lose entries. Their catalog is also **per model, not per harness**, and cached
with a TTL keyed on the binary's version — because a CLI upgrade changes the answer.

The same holds for modes. Claude's `--permission-mode` takes `default|acceptEdits|plan|bypassPermissions`;
that is the harness's list, and "Plan / Edit / Ask" is a label set the UI invented. A mode is a
discovered entry whose id is passed through, which is what `Policy` already does for the string it
carries.

**ACP reached the same conclusion, and settled it in favour of one generic mechanism.** A session
advertises its config options — each an id, a name, an optional description, a category, and either
a current value with a list of choices or a boolean — and either side may change one. The dedicated
mode methods are deprecated and gone from the v2 draft; models never had methods of their own. So
the model, the mode, the thinking level and whatever a harness invents next are one shape.

That is what P3 builds, and it means the composer's pickers are generated from a list rather than
enumerated in code — so a harness that grows a fourth knob needs no change in `crates/ubiq`.

## The protocol

### Library side

`AgentParams` becomes real — the per-turn and per-session knobs, in the shape multica's `ExecOptions`
proved: a model id, a thinking level id, a mode id, a permission preset, a resume id. It is what a
conversation face is started with and what a mid-session change carries.

`AgentEvent` grows the vocabulary above. The four that matter most, because the UI cannot draw
without them:

- **turn framing** — a turn starts and ends, so `RunState` is driven by the host rather than guessed.
- **delta versus whole** — a chunk is an append to the current block; today's `AssistantText` is
  ambiguous and the host would have to guess whether to append or replace.
- **a tool call the UI can render** — ACP's shape fits `feat-chat`'s tool block almost field for
  field: a stable id, a title, a `kind` from a fixed set (read, edit, delete, move, search, execute,
  think, fetch, other) which is the block's coloured verb, a status (pending, in progress, completed,
  failed), the paths it touched, and content entries that are either text or a **diff** carrying the
  path with its old and new text. That last one is what the expanded edit block draws. An update is
  a patch naming the same id, so a tool call that starts pending and completes later is two messages
  about one block rather than two blocks. Mapping a harness's raw tool JSON into that shape is
  *harness knowledge*, so it belongs in the library, not in `crates/ubiq-host`.
- **a permission request with its options** — the whole tool call, plus the choices, each an id, a
  name and a kind: allow once, allow always, reject once, reject always. Embedding the tool call is
  deliberate in ACP and right for us: the dialog has to show *what* is being authorised, and a
  request carrying only an id would send the UI looking for it.

`AgentInput` grows `SetMode`, `SetModel` and an answer that names a permission option. `Harness`
grows a capability query so the composer can only offer what the harness has.

### What Claude actually puts on the wire

Verified by running `claude -p --output-format stream-json --verbose` against Claude Code 2.1.259,
not read from a document. Everything the status surfaces need is already there, and the bridge keeps
almost none of it.

| Event | Fields that matter | What it gives the UI |
|---|---|---|
| `rate_limit_event` | `rate_limit_info.unifiedWindows.{five_hour,seven_day}.utilization`, `resetsAt`, `status`, `overageStatus` | how much of the user's window is spent, and when it resets — a gauge nobody had planned for |
| `system` / `init` | `session_id`, `model`, `permissionMode`, `tools[]`, `agents[]`, `skills[]`, `slash_commands[]`, `mcp_servers[]` each with a `status`, `capabilities[]`, `claude_code_version`, `apiKeySource` | capability discovery, **free and per session**: the modes, the tools, the subagent types this run can spawn, and whether each MCP server is connected or needs auth |
| `assistant` | `message.usage.{input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens}`, `message.model`, `parent_tool_use_id`, `request_id` | tokens **per message as the turn streams**, not only at the end — and `parent_tool_use_id` is the subagent attribution |
| `result` | `total_cost_usd`, `stop_reason`, `num_turns`, `ttft_ms`, `duration_ms`, `permission_denials[]`, `modelUsage.<model>.{inputTokens, outputTokens, cacheReadInputTokens, cacheCreationInputTokens, contextWindow, maxOutputTokens, costUSD, thinkingTokens}`, `subagent_stats.{spawned, max_depth, completed, failed, killed, by_type}` | the turn's cost, its shape, and the context window **per model** |

Three consequences worth stating plainly:

- **The context ring is computable and currently wrong.** `context_pct` is
  `(input + cache_read + cache_creation) / contextWindow`, and `contextWindow` arrives per model —
  200 000 for Sonnet, 1 000 000 for the Opus variant this probe ran on. `crates/ubiq/src/state/chat.rs`
  hard-codes a 200 000 constant, so a million-token model would draw a ring four fifths full at the
  start.
- **Subagents are attributable, and the UI already has the shape for it.** A `Task` tool call is the
  spawn; every message a subagent produces carries `parent_tool_use_id` naming that call. `WorkAgent`
  already has `parent`, and the orchestration graph already draws a connector from it — so a subagent
  becomes a child agent rather than a new concept.
- **A bug, found while checking this, and it starts in a document.** The Claude runtime contract in
  `crates/agent-manager/_docs/harness/claude-code.md` writes `modelUsage` with snake_case keys;
  the real event uses camelCase — `inputTokens`, `outputTokens`, `contextWindow`. `extract_usage`
  followed the document, so its per-model branch matches nothing and only survives through its
  fallback to the top-level `usage` object, which *is* snake_case. Per-model attribution and every
  cache-token field are silently dropped, and `AgentEvent::Usage` has nowhere to put them anyway.
  The document is the thing to fix first, since the code and its test both faithfully implement it.

Three smaller drops worth listing, because each is a feature the UI would otherwise have to invent:
**per-message `usage` and `model`** on every `assistant` event go unread, so live per-turn accounting
is invisible; a **`tool_result` carrying `status: "async_launched"`** is the one progress signal
between a tool starting and finishing, and it passes through as opaque content; and every `system`
event whose subtype is not `init` falls through the mapper, which is where `rate_limit_event` and
anything added later go to die.

### Ubiq side

A **conversation family** on the wire, beside the work family. The rules it must obey, all of which
already exist as rules elsewhere:

- **The host is the only writer.** `feat-workbench` states it: nothing writes into a transcript, and
  a screen that drew its own half of a conversation would be inventing the other half too. So a
  delta arrives as a message and is applied; the UI never synthesises a reply. `crates/ubiq/src/state/chat.rs`'s
  canned reply goes away with this.
- **Deltas, not records.** `AgentChanged` re-sends the whole `WorkAgent`; a token stream cannot. A
  conversation delta names the workspace, carries a sequence number and one update, and the UI
  appends. A late joiner asks for the whole conversation once and follows deltas after.
- **The pump is never blocked by a slow UI.** The same rule that governs a pseudo-terminal's reader:
  the bus mailbox is unbounded and the pump thread never waits on the window.
- **Coalescing is the UI's business.** The host forwards what the harness said; a window that cannot
  draw 200 chunks a second coalesces on its side.

`WorkAgent.thread` and `Turn` are replaced by the richer conversation, not extended. Keeping a
two-field `Turn` beside a block model would leave two half-truths about the same conversation.

## Packages, in order

The order is deliberately a **thin vertical slice first, fully observable, then iterate**. One
harness, no isolation, no composition, nothing configurable — and both halves of the traffic written
down, so the features after it are discovered from real frames rather than designed from documents.

### P1 — One Claude conversation, end to end, with everything logged — **landed**

The whole flow at its narrowest: `SpawnWorkspace` with a conversation face starts `claude` through
the existing stream-json bridge — **no sandbox, no skills, no MCP servers, no account selection, no
model or mode** — the host pumps its events onto the bus, and the agents column renders them.

**Both layers are logged, and that is the point of the package.** Two streams, distinguishable:

- the **harness frames**, raw and unmapped, exactly as the child wrote them, and exactly as we write
  back to it. This is the record that tells us what a harness really emits — the probe above is a
  one-off of what this makes permanent.
- the **bus messages**, after mapping, as the UI receives them.

`Subsystem::Harness` exists in the log console for the first and nothing has ever filled it (`G25`).
The pair is what makes a disagreement between "what Claude said" and "what the UI drew" a two-line
diff rather than a debugging session. Raw frames carry prompts and file contents, so the raw stream
is opt-in and off by default — a log the user might paste into an issue must not carry their code.

Isolation stays off here, and conveniently cannot be otherwise: `--isolate` with structured I/O is
refused today, so "no sandbox" is the current behaviour rather than a decision to unwind later.

**Done when** a prompt typed in a column reaches Claude, its reply streams back into the transcript,
and the log console shows the frames that produced it beside the messages the window received.

What it settled, beyond the slice itself: the vocabulary is `D53`, the family is documented in the
transport contract, the wire reference is [`../../refs/acp-protocol.md`](../../refs/acp-protocol.md),
and the conversation view is one component rather than one per screen. What it left open is `G92`
through `G99`. Three vocabulary items are on the wire and unimplemented — the permission request,
the config options and the plan — and P3 and P7 are what fill them.

### P2 — The debug sink, and a second variation

A `SinkSection` with the transcript and composer on one side and the log console on the other, over
one conversation. P1 makes it possible; this makes it comfortable, and it is where every later
package gets looked at.

Then the second harness variation, which is what proves the seam is a seam: either **Claude over
ACP** through Zed's adapter, or **Codex** over its JSON-RPC bridge. Claude-over-ACP is the more
interesting of the two, because it makes ACP an *input* protocol here for the first time and the
same harness then has two variations to diff against each other — the same conversation, two wire
formats, one rendered result. It also needs an ACP client bridge, which does not exist yet: today's
`io/acp.rs` is an output mapper only.

**Watch for:** a one-shot harness (opencode, Copilot) accepts no second prompt — the composer must
be told by the capability query, not discover it by sending into a void. And `spawn_piped` inherits
this process's environment unless a launch says otherwise, which is why `Launch::env_clear` now
reaches it.

**Done when** the same prompt, run as both variations, produces the same transcript, and the two
raw logs show why any difference exists.

### P2b — Status from what the stream already carries

Cheap, and the visible payoff of P1's logging: the run pill driven by real activity, the token count
and the context ring from real usage, the cost of a turn, and the rate-limit window. Every field is
in the table above and every render target already exists — `Activity`, `tokens`, `context_pct` and
the footer are drawn from fixtures today.

This is also where the two token bugs get fixed: the camelCase mismatch in the bridge, and the
hard-coded context window in the interface.

**Done when** a column's footer reports the real model, the real tokens and a ring computed from
that model's real context window.

### P2c — The session record, made portable, harness by harness

A harness writes its own transcript to disk, and it is richer than anything it streams: Claude's
session file carries the sidechain flag, per-message uuids, the parent tool-use id, timestamps and
full tool payloads — the whole record, not the projection the wire carries.

**And Ubiq is currently destroying it.** Claude keeps those transcripts under `projects/<hash>/`
*inside* its configuration directory, and the library relocates that directory into the throwaway
run directory — which the pane deletes when it closes. Correct for configuration, wrong for the
record, and invisible until someone goes looking for a conversation that should still exist. The
harness contract already documents that path as state a credential import must not copy; this is the
same path wanted for the opposite reason.

So: at teardown, before the sweep, copy the harness's own transcript into the library's session
store beside the `SessionMeta` it already writes. `ConfigAnchor` already knows where each harness
keeps its files, which is why this is a per-harness step rather than one implementation — and why it
is worth doing one harness at a time, starting with the one P1 uses.

That record is what makes the statistics worth having: tokens, cost, duration and turn count per
session, per model and per agent definition, over conversations that outlive the pane that ran them.
The live stream can only report the turn in front of you; the file is the history.

**Watch for:** a captured transcript holds prompts, file contents and tool output. It is the user's
data, it belongs under Ubiq's own root rather than in a project, and deleting a session has to mean
deleting it.

**Done when** a conversation's full record survives closing its pane, and a second run of the same
agent definition can be compared with the first on tokens, cost and duration.

### P3 — Models, thinking and modes

The library discovers each per harness and per model, caches with a TTL keyed on the binary's
version, and advertises them as one list of options in the config-option shape above; the composer's
four constants become a projection of that; a change mid-session goes down as an input where the
harness supports it and is refused where it does not. `ModelInfo` grows a provider and a per-model
thinking catalog, following multica's shape.

**Where the vocabulary comes from differs per harness, and none of it is a protocol answer.** Claude
is asked by starting a one-shot session; Codex answers `codex debug models`; Copilot's ids are parsed
out of help text; opencode prints one per line; Grok reads a cache file it only writes once
authenticated. Four of those five can fail for reasons the user can fix, so a harness that cannot
answer must offer "whatever the harness defaults to" rather than an empty picker.

**Done when** the composer offers Claude's effort levels for a Claude column and Codex's for a Codex
one, with no shared enum between them.

### P4 — Agent definitions

An agent is **a profile with a harness pin** — the library already says so, and `am agent <name>`
already runs one. So this is not a new concept, it is a UI over `FsProfileStore`: the settings
overlay's Harnesses section lists profiles, creates and edits them, and `FsProfileStore` persists
each as `<root>/<id>/profile.toml`. The one field an agent definition needs and a profile lacks is a
**mode**, which `Profile` should carry beside its existing isolation default rather than having the
host invent a parallel store.

The new-pane menu then offers *defined agents* above bare harnesses, and the composer's harness
picker becomes an agent picker.

**Done when** a user defines "reviewer — codex, gpt-5, plan mode, work account" in settings and
starts it from the new-pane menu.

### P5 — Credentials, through a login pane

The insight that makes this cheap: **a login is an interactive subprocess that needs a real
terminal, and Ubiq already owns terminals.** `Harness::login` answers a `LoginPlan` — a launch plus
the credential files to capture — so Ubiq opens a pane for that launch exactly as it opens one for a
harness, waits for the exit, and calls `capture_login`. The browser OAuth flow the user completes is
the harness's own, unmodified.

Two paths, and the UI should offer both because every harness supports both: the pane flow for
subscription OAuth, and a form for a pasted key, which writes an `Account` holding an env-var
reference and never the secret. Accounts carry credential *references*; bodies live in the
`SecretStore`, and the engine choice — files, a local vault, or the OS keychain — is a setting.

**Watch for:** the copy-back gap. A token the harness refreshes inside the run directory is
discarded when the pane closes, so the next run re-seeds the older one. It is recorded in the
library's own open points, and P5 is where a user first feels it.

**Done when** a user with no `~/.claude` can log in from Ubiq, and a run started afterwards is
authenticated.

### P6 — The default agent home

Isolation currently gives every run an ephemeral `$HOME`, which is right for a one-off and wrong for
an agent that should keep its caches and its login. `HomeMode::Managed` and the `@managed/<id>` form
already exist for this; what is missing is the policy: **a defined agent gets a persistent home
keyed by its definition, an ad-hoc run gets an ephemeral one.** The home lives under Ubiq's config
root, never inside a project, because `D30` forbids writing there.

**Done when** a defined agent's second run finds its own cache warm, and an ad-hoc run still starts
clean.

### P7 — Real permissions

Gated on library work, and worth stating plainly: **until the bridges stop auto-approving, Ubiq must
not draw a permission prompt.** A dialog that appears after the tool ran is worse than no dialog,
because it teaches the user that the dialog means something. So P7 is: make each bridge hold the
approval, surface it with its options, and answer it from the input side — then, and only then, the
UI asks.

The option kinds to honour are ACP's four — allow once, allow always, reject once, reject always —
because "always" is what makes the feature bearable and it has to be remembered somewhere. Where is
an open question: per conversation is the protocol's scope, per agent definition is what a user
would expect to persist.

**Done when** a Claude run's file write waits for a click, and denying it visibly stops the tool.

## Deferred, deliberately

Skills and MCP composition on the wire (the catalogue projection, `G31` and `G78`); Ubiq's own MCP
surface so a hosted agent can call back into the window (`G7`, and the library's in-process MCP is
the mechanism); resuming a conversation after a restart; agents on remote hosts. None of them block
P1 to P6.

## Traps

- **Auto-approval is a security trap, not a feature gap.** See P7.
- **A confined structured run is not possible yet.** `--isolate` with `--io structured` is refused,
  because a bridge spawns its own piped child and isol8 needs the descriptors. The seam now exists
  in isol8 as a stdio entry point, so this becomes a small change to how a bridge is handed its
  pipes — but it is a change to every bridge, not a flag.
- **No process-group kill.** Cancelling a turn kills the child, not its tree. multica uses process
  groups; the library forbids unsafe code and has no `libc`, so this needs either a dependency or an
  isol8 call.
- **A one-shot harness in a conversation column** will look broken the moment a second prompt is
  typed. The capability query has to reach the composer, or those harnesses stay terminal-only.
- **The blocking pull.** `send` and `next_event` both take `&mut self`, so the pump thread owns the
  bridge and a prompt has to reach it through a channel rather than by calling it directly.

## Open questions

1. **Does the conversation family replace the work family's agent half, or sit beside it?** The
   orchestration graph and the tasks board draw `WorkAgent` records that are mocks; a real agent has
   no `role`, `note` or `parent` yet. Replacing means those screens change with P2.
2. **Depend on ACP's published types, or keep our own ACP-shaped ones?** The objection I expected —
   that it would drag in an async runtime — does not hold: the schema crate is types only, and the
   SDK's runtime layer is executor-agnostic rather than tokio-bound. So the real question is
   narrower: taking the types buys conformance and a moving target, since the protocol has a v2 in
   draft and the stable v1 is what every peer speaks today. My inclination is the types crate for
   the event vocabulary and none of the runtime, since the transport stays our bus.
3. **One pump thread per workspace, or one per window?** Per workspace is simpler and matches how a
   pseudo-terminal reader already works; per window bounds the thread count if someone opens forty.
4. **Where does "allow always" live?** Per conversation is the protocol's scope; per agent
   definition is what a user would expect to survive a restart. P7 needs an answer.

## Related docs

- [`../tech/agent-manager.md`](../tech/agent-manager.md) — the boundary every package here crosses
- [`../features/chat.md`](../features/chat.md) — the render model a conversation fills
- [`../features/workbench.md`](../features/workbench.md) — the agents screen, its columns and the settings overlay
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the conversation family is documented once it exists
- [`../../refs/isol8-pty-seam-update.md`](../../refs/isol8-pty-seam-update.md) — the isol8 seam, and what a confined structured run still needs
