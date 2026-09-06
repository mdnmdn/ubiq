---
id: wip-agent-setup
title: Wiring a real agent into the agent pane
kind: wip
status: draft
summary: The protocol, the library work and the order of packages behind a real conversation with a composed harness — what has landed, and the honest inventory of what today's library cannot yet deliver.
read_when: you are picking up the next agent-integration package, or judging whether a proposed conversation message belongs on the wire
updated: 2026-09-06
verified: 2026-09-06
code_anchors: [crates/ubiq-host/src/agent.rs, crates/ubiq-host/src/coordinator.rs, crates/agent-manager/src/session.rs, crates/agent-manager/src/harness/mod.rs, crates/agent-manager/src/harness/claude.rs, crates/agent-manager/src/resolve.rs, crates/agent-manager/src/isolate.rs, crates/agent-manager/src/io/model.rs, crates/agent-manager/src/io/jsonl.rs, crates/ubiq-proto/src/work.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/state/conversation.rs, crates/agent-manager/src/profile.rs]
depends_on: [tech-agent-manager, feat-workbench, feat-chat]
review_cycle: monthly
---

# Wiring a real agent into the agent pane

## Where this starts

A workspace is composed by the library and confined by default: `RunSpec` → provision → `Launch` →
pseudo-terminal, and a pane shows the harness's own screen. That is the **passthrough** half. The
conversation half runs too: a Claude conversation streams end to end (P1), an identity can be
signed in from inside Ubiq and saved into a named definition that a conversation starts from (P4 and
P5), the model and the mode are picked before the harness launches (P3), and the chat panel and the
agents column draw one conversation through one component. A conversation is confined by
`isolate_agents` like a pane, and a defined agent keeps a persistent home behind that (P6). What
remains is the thinking level and permissions (P7).

**Two corrections to earlier notes in this tree, both load-bearing.**

`_docs/wip/agent-login-note.md` concluded that a Ubiq run receives no login. It does:
`seed_zero_config_login` (`crates/agent-manager/src/provision.rs:158`) copies the harness's own
login files from the real `$HOME` into the run directory whenever no account is named. The "Not
logged in" transcript that prompted the note was a **stale token**, not missing wiring. Account
selection was still worth building — but for owning several identities, not for repairing
authentication.

And a model **cannot be changed mid-conversation**. `spec.model` is applied at launch
(`crates/agent-manager/src/harness/claude.rs:195`), and every bridge refuses
`AgentInput::SetConfigOption` outright — `io/jsonl.rs:222`, `io/codex.rs:439`, `io/copilot.rs:126`,
`io/opencode.rs:128`. So a model picker has to be answered *before* the harness starts, which is
what shapes P3 below.

## The inventory, and what each item costs

Read this table before designing anything. Everything in it was verified against the tree, and
several rows contradict what a reader would reasonably assume. Rows that P1 and P5 have since made
false are deleted rather than annotated: this is what is true now.

| What is true today | What it costs us |
|---|---|
| `AgentId` **is** `WorkspaceId` — one type, deliberately, "until a workspace outlives its pane" | Nothing to reconcile: a real agent and a pane are already one identity |
| `IoBridge` is two methods, both `&mut self`, and `next_event` **blocks** | The host needs one pump thread per structured workspace; `send` and `next_event` cannot be called concurrently without splitting the bridge |
| Only **Claude** and **Codex** accept input after launch. opencode and Copilot bridges are one-shot: the prompt goes in through argv, `send` is a no-op | Only two harnesses can back a conversation column. The other two are single-turn runs wearing the same trait |
| **Every bridge auto-approves.** Claude's reader answers every `control_request` with `allow`; Codex auto-accepts every approval RPC; opencode runs `--dangerously-skip-permissions`; Copilot runs `--allow-all --no-ask-user` | A permission prompt in the UI would be theatre — the tool has already run. This is the one item with a security consequence, and it gates any "ask me first" feature |
| **Model discovery is implemented for all five harnesses** — `Harness::discover_models` (`harness/mod.rs:501`) is overridden by every one. But it takes **no account and no directory**, so a list is per harness rather than per identity, and Claude's probe reads the *ambient* login; discovery **is** cached now — `FileHarnessCache` (`crates/ubiq-host/src/store/harness.rs`) writes `<config root>/cache/harness-models.toml`, keyed on `(harness, account, version)`, with `version` read off the harness binary's own `--version` — so a hit skips the probe outright and a harness whose version cannot be read bypasses the cache in both directions | A model picker is available today, and its list is the same whichever account was chosen. Per-account lists need the trait signature to change. The catalogue can go stale until the harness binary's version string changes |
| **A thinking / reasoning-effort catalog exists in Rust for two of five harnesses.** `Harness::discover_thinking` (`harness/mod.rs`) returns `BTreeMap<String, ModelThinking>` (`ModelThinking { levels: Vec<ThinkingLevel>, default_level }`, `ThinkingLevel { value, label, description }`), defaulted to empty; `Claude` scrapes `claude --help`'s `--effort` parenthetical and applies it to every model, `Codex` reads `supported_reasoning_levels`/`default_reasoning_level` off the same `codex debug models --bundled` value `discover_models` already parses. opencode, Copilot CLI and Grok CLI still answer the empty default — none of the three exposes a reasoning concept a command can read. `ConfigCategory::ThoughtLevel` is still a *label on an option*, not wired to this catalog | The library-side catalog exists for the two harnesses that support reasoning effort; nothing in the UI or bridge layer consumes it yet, so "thinking budget" is still not a picker anyone can draw |
| **No bridge emits `ConfigOptionUpdate`, and all four reject `SetConfigOption`.** `AgentEvent::ConfigOptionUpdate` (`io/model.rs:623`) has zero producers; `Message::SetAgentConfig` is fully plumbed host-side (`coordinator.rs:647`) and fails one layer down | The config-option mechanism is a shape with nothing behind it. A model chosen at launch works; a model changed mid-turn cannot |
| `Policy` carries an opaque `permission_mode` string, passed through per harness. Claude's `init` event already reports `permissionMode`, and the bridge surfaces it as `SessionStarted.mode` | The mode is the one item of the three that is free — it is already on the wire as `ConvUpdate::Started` |
| **Ubiq writes a session record of its own.** `crates/ubiq-host/src/agent.rs` calls `agent_manager::session::save` the moment a run is composed, into `<ubiq root>/sessions` — a directory it passes explicitly, so `AM_SESSIONS` cannot redirect a user's Ubiq transcripts — and copies the harness's own transcript in at teardown. Ubiq's sessions and agents are otherwise in-memory (`ubiq-host/src/work/mod.rs`), and only tasks persist | A run's record and the harness's own file outlive the run directory. `am session ls` reads the library's store and so does not list Ubiq's runs; and the record is metadata plus a harness file, not an `AgentEvent` transcript. The live conversation state still does not survive a restart |
| `to_acp` is a **stateless one-event mapper** that drops `ApprovalRequest`, `Usage`, `Result` and `SessionStarted`, and emits no JSON-RPC envelope, no session id and no turn brackets | There is no ACP endpoint today. The vocabulary is right; the protocol is not implemented |
| `WorkAgent.thread` is `Vec<Turn>` of `{ from, text }`, replaced whole on every `AgentChanged` | A token stream would re-send the entire conversation per token |
| The library cannot kill a process **group** — it is `#![forbid(unsafe_code)]` with no `libc` | A cancelled turn can leave grandchildren. multica solves this with process groups; we cannot copy that directly |

## Terminology, and the one type behind it

Three words, decided:

- **harness** — one executable. Claude Code, Codex. One per binary.
- **account** — one authentication for a harness.
- **agent** — instructions, a role, capabilities. Can run on different harnesses and accounts.

**For now the interface's "harness" means the pair** (Claude Code + a specific account); the agent
layer is deferred. That is a labelling decision rather than a data-model one, because all three
collapse onto one library type: a **`Profile`** (`crates/agent-manager/src/profile.rs`) is
`{ id, extends, account, harness, defaults: { mcps, skills, model, hooks, instructions }, isolate,
mode }`. Today's pair is a `Profile` with `harness` and `account` set; tomorrow's agent is the same
`Profile` with `defaults.instructions` filled. So the split, when it comes, is a rename in
`crates/ubiq` and no migration — and the persistence and the resolution are the library's, through
`FsProfileStore::save` and `resolve`, with Ubiq holding only the form over them.

## The shape: one workspace, two faces

The decision everything else hangs off. A workspace is one composed run of one harness, with a
**face** chosen when it is spawned and never after: a **terminal face** (passthrough, a
pseudo-terminal, drawn as a pane) or a **conversation face** (structured I/O over pipes, drawn as a
column or in the chat panel). `IoModes` already is that choice and `AgentId = WorkspaceId` already
says the two are one thing, so the wire needs no new identity and no second lifecycle. A harness
cannot wear both at once — a child's stdout is either a terminal or a pipe — and pretending
otherwise is what a second identity would cost.

**A view is not the workspace.** A pane, a column and the chat panel are perspectives on a run the
host owns: closing one ends nothing, and the same conversation is visible from every surface that
lists it. Which means a project accumulates live harnesses, and open question 7 is what to do about
that.

## Decision 1 — ACP-shaped, not ACP-transported

**The vocabulary is ACP's. The transport stays the in-memory bus.** `D9` says Ubiq embeds the
library rather than shelling out to `am`; putting JSON-RPC between two halves of one process would
undo that for no gain. But the *names* and the *event shapes* should be ACP's, in three places: the
library's neutral event model, the bus family, and the mapper that already exists.

Three reasons, in order of weight: nine harnesses in the library's reference table speak ACP
natively (`hermes`, `kimi`, `kiro` and `qoder` launch as `<binary> acp`), so an ACP-shaped neutral
model makes an inbound bridge a reader of its own vocabulary rather than a third translation;
`io/acp.rs` becomes a real adapter rather than a lossy projection; and the UI's render model is
already ACP-shaped by coincidence. **The mapping is settled and recorded elsewhere** — `D53` in
`tech/decisions.md`, the family in `tech/transport-contract.md`, and the wire in
[`../../refs/acp-protocol.md`](../../refs/acp-protocol.md). (`refs/multica` holds no ACP code, so an
earlier claim that these were confirmed against its clients was unsupported.)

Two corrections a reader would not get from the harness documents.
**`session/set_model`, `availableModels` and `currentModelId` are not in the schema at all** — a
model picker is core ACP expressed as a config option whose `category` is `model`, which is one
mechanism for the model, the mode, its parameters and the thinking level, and what
`session/set_mode` was deprecated in favour of. And ACP's `terminal/*` is a side-channel for the
agent to run a command and read its output — no method types into it and none resizes it, so it is
not a pane and cannot host an interactive harness. That stays Ubiq's job, along with which pane a
conversation is drawn in, which project it belongs to, and the arrangement over it.

## Decision 2 — discovered vocabularies, never a flattened enum

Models, thinking levels and modes are **lists a harness advertises**, each entry an id, a label and
an optional description. Not enums, not a shared taxonomy, and not the four constants
`crates/ubiq/src/state/chat.rs` holds today.

multica wrote the reason down: Claude's effort levels are `low|medium|high|xhigh|max` and Codex's
are `none|minimal|low|medium|high|xhigh`, `xhigh` is Opus-only, `max` is session-only, and
opencode's can be extended by local config. What the user picks has to **round-trip exactly**
through the harness's own vocabulary, so a shared enum would either lie or lose entries — and the
catalog is **per model, not per harness**, and worth caching against the binary's version, because
an upgrade changes the answer. Modes are the one exception, and only because they are few and
fixed: `Harness::modes()` is a hard-coded per-harness list — six for Claude, three for Codex, empty
for the rest — rather than a probe, so no cache is involved. It is still the harness's own
vocabulary and never a shared enum; "Plan / Edit / Ask" is a label set the UI invented and does not
use.

**ACP reached the same conclusion and settled it as one generic mechanism**: a session advertises
its config options, each an id, a name, an optional description, a category, and either a current
value with choices or a boolean. The dedicated mode methods are deprecated and gone from the v2
draft; models never had methods of their own. So the model, the mode, the thinking level and
whatever a harness invents next are one shape — which is what P3 builds, and why the composer's
pickers are generated from a list rather than enumerated in code.

## The protocol

### Library side

**The event vocabulary landed with P1.** `AgentEvent` has sixteen variants (`io/model.rs:553`) —
turn framing so `RunState` is driven rather than guessed, chunks that are unambiguously appends,
`ToolCall`/`ToolCallUpdate` in ACP's shape (a stable id, a title, a `kind` from a fixed set which
is the block's coloured verb, a status, the paths touched, and content that is either text or a
diff carrying old and new), and a `PermissionRequest` that embeds the whole tool call because the
dialog has to show *what* is being authorised. Mapping a harness's raw tool JSON into that shape is
*harness knowledge*, which is why it lives in the library rather than in `crates/ubiq-host`.

`AgentParams` never became real, and does not need to: the config-option mechanism (Decision 2)
subsumed the model, mode and thinking level into one list, and `AgentInput::SetConfigOption` is the
single input that carries a change. What is still missing is on the *other* side of that trait —
nothing produces `ConfigOptionUpdate` and every bridge refuses `SetConfigOption`, which is P3's
work. `Harness` also still lacks a capability query, so the composer cannot yet be told that a
one-shot harness takes no second prompt.

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
- **A bug that starts in a document.** The Claude runtime contract in
  `crates/agent-manager/_docs/harness/claude-code.md` writes `modelUsage` with snake_case keys; the
  real event uses camelCase. `extract_usage` followed the document, so its per-model branch matches
  nothing and survives only through its fallback to the top-level `usage` object, which *is*
  snake_case — dropping per-model attribution and every cache-token field. Fix the document first:
  the code and its test both faithfully implement it.
- **The persistent, stdin-driven session this probe did not test never echoes the prompt back.**
  This table was built against a one-shot `claude -p "prompt" --output-format stream-json`
  invocation; Ubiq drives a different shape — `-p --input-format stream-json`, prompts delivered as
  NDJSON `{"type":"user",...}` lines on stdin (`io/jsonl.rs`'s `write_input`), the same process kept
  alive across turns. A live two-turn check against that exact shape
  (`io::jsonl::tests::live_two_turn_structured_session_when_claude_available`) found stdout carries
  no `"type":"user"` line at all for either turn — only `system`/`assistant`/`result`. So
  `write_input` now synthesizes `AgentEvent::UserMessageChunk` itself, locally, the moment a prompt
  line actually reaches the child's stdin, rather than mapping an echo that mode never sends. The
  same check found no `"type":"thinking"` content block either — expected, since nothing in the
  structured argv (`harness/claude.rs`) requests extended thinking, so there is nothing for the
  existing `map_content_blocks` thinking arm to map; turning thinking on is unbuilt follow-up work,
  not a mapping defect. And no `ai-title`/title-shaped event of any kind appeared on stdout across
  six turns of two separate live runs — the `{"type":"ai-title",...}` line this doc once expected
  to add a `Mapper::map_event` arm for is a shape observed only in Claude Code's on-disk **interactive**
  session transcript (`~/.claude/projects/**/*.jsonl`), a file headless `-p` runs do not even write;
  nothing confirms it is part of the headless stdout protocol at all, so no arm was added. The title
  plumbing (`AgentEvent::SessionInfoUpdate` → `ConvUpdate::Title` → `Conversation::title` →
  `refresh_agent_record` renaming the `WorkAgent`) is still built end to end, since it costs nothing
  idle and is what any future title producer — this harness or another — needs to reach a reader.

**The `init` event is mapped**, and generously: `io/jsonl.rs:481` fills `SessionStarted` with the
session id, the model, `permissionMode` as the mode, the tools and the subagent types. Its
`slash_commands` is read by nothing, and `AvailableCommandsUpdate` exists as a variant with no
producer. Three smaller drops, each a feature the UI would otherwise invent: **per-message `usage`
and `model`** on every `assistant` event go unread, so live per-turn accounting is invisible; a
**`tool_result` carrying `status: "async_launched"`** is the only progress signal between a tool
starting and finishing and passes through as opaque content; and every `system` event whose subtype
is not `init` falls through the mapper. `rate_limit_event` is no longer one of them: it is mapped
(`io/jsonl.rs`'s `map_rate_limit`) to `AgentEvent::RateLimitUpdate` → `ConvUpdate::RateLimit` →
`Conversation::rate_limit`, drawn in the footer next to cost and the context ring.

### Ubiq side

A **conversation family** on the wire, beside the work family, obeying four rules that already
exist as rules elsewhere. **The host is the only writer** — nothing writes into a transcript, and a
screen that drew its own half would be inventing the other half too, so a delta arrives and is
applied and the UI never synthesises a reply. **Deltas, not records** — a delta names the
workspace, carries a sequence number and one update; a late joiner asks for the whole conversation
once and follows deltas after. **The pump is never blocked by a slow UI**, the same rule that
governs a pseudo-terminal's reader. **Coalescing is the UI's business.**

`WorkAgent.thread` and `Turn` are replaced by the richer conversation, not extended: a two-field
`Turn` beside a block model would leave two half-truths about one conversation.

## Packages, in order

The order is deliberately a **thin vertical slice first, fully observable, then iterate**. One
harness, no isolation, no composition, nothing configurable — and both halves of the traffic written
down, so the features after it are discovered from real frames rather than designed from documents.

### P1 — One Claude conversation, end to end, with everything logged — **landed**

The whole flow at its narrowest: `SpawnWorkspace` with a conversation face starts `claude` through
the existing stream-json bridge — **no sandbox, no skills, no MCP servers, no account selection, no
model or mode** — the host pumps its events onto the bus, and the agents column renders them.

**Both layers are logged, and that is the point of the package**: the harness frames raw and
unmapped, exactly as the child wrote them and as we write back; and the bus messages after mapping,
as the UI receives them. That pair makes a disagreement between "what Claude said" and "what the UI
drew" a two-line diff rather than a debugging session. Raw frames carry prompts and file contents,
so the raw stream is opt-in and off by default (`G99`) — a log the user might paste into an issue
must not carry their code. `Subsystem::Harness` was the console slot nothing had ever filled
(`G25`). Isolation stays off, and cannot be otherwise: `--isolate` with structured I/O is refused.

What it settled beyond the slice: the vocabulary is `D53`, the family is in the transport contract,
and the conversation view is one component rather than one per screen. What it left open is `G92`
through `G99` — the permission request, the config options and the plan are on the wire and
unimplemented, and P3 and P7 fill them.

### P2 — The debug sink, and a second variation

A `SinkSection` with the transcript and composer on one side and the log console on the other, over
one conversation. P1 makes it possible; this makes it comfortable, and it is where every later
package gets looked at.

Then the second harness variation, which proves the seam is a seam: either **Claude over ACP**
through Zed's adapter, or **Codex** over its JSON-RPC bridge. Claude-over-ACP is the more
interesting — it makes ACP an *input* protocol here for the first time, and one harness then has
two variations to diff: one conversation, two wire formats, one rendered result. It needs an ACP
client bridge, which does not exist; `io/acp.rs` is an output mapper only.

**Watch for:** a one-shot harness (opencode, Copilot) accepts no second prompt — the composer must
be told by the capability query, not discover it by sending into a void.

**Done when** the same prompt, run as both variations, produces the same transcript, and the two
raw logs show why any difference exists.

### P2b — Status from what the stream already carries — **landed**

Cheap, and the visible payoff of P1's logging: the run pill driven by real activity, the token count
and context ring from real usage, the cost of a turn, and the rate-limit window. Every field is in
the table above and every render target exists. This is also where the two token bugs get fixed:
the camelCase mismatch in the bridge, and the hard-coded context window in the interface.

**Done when** a column's footer reports the real model, the real tokens and a ring computed from
that model's real context window.

### P2c — The session record, made portable, harness by harness — **landed**

A harness writes its own transcript to disk, and it is richer than anything it streams: Claude's
session file carries the sidechain flag, per-message uuids, the parent tool-use id, timestamps and
full tool payloads — the whole record, not the projection the wire carries. Claude keeps it *inside*
its configuration directory, which the library relocates into the throwaway run directory the pane
deletes on close, so the record has to be copied out before that deletion. It is, and the whole
mechanism — `Harness::transcripts` as the harness's own answer to which files are the record,
`session::save` for the metadata at composition, `Agents::archive` for the files at teardown, and
the three ownership answers that are Ubiq's rather than the library's — is stated once in
[`../tech/agent-manager.md`](../tech/agent-manager.md).

Nothing on the wire changed — no message, no `ubiq-proto` type, no coordinator or conversation
code. The record is a host-side file, and the surfaces that would read it are what comes next.

**What this unblocks and does not deliver.** A conversation's record survives closing its pane,
which is the precondition for per-session statistics and for resuming a stopped agent — but nothing
replays it (`G120`), and nothing deletes one either, so the store grows one directory per
conversation holding prompts, file contents and tool output (`G164`).

### P2d — The host stops building its own `RunSpec`, and one view draws every conversation — **landed**

Two changes that together are what made an identity reachable at all.

**`compose_run` calls `resolve`.** `crates/ubiq-host/src/agent.rs` no longer hand-sets five fields
of a `RunSpec`; it calls `agent_manager::resolve::resolve` and overrides only the three answers that
are Ubiq's — see [`../tech/agent-manager.md`](../tech/agent-manager.md), which owns that division and
the stores behind it. Everything else comes from the profile, so an account reaches a pane without
`agent.rs` learning what an account is. An unknown id fails the spawn with the fuzzy suggestions
`resolve` already produces, because a misconfigured account must say so rather than starting
unauthenticated.

**One conversation view, two surfaces.** `crates/ubiq/src/ui/conversation` was already the shared
transcript, composer and footer for an agents column; the chat panel draws it too, and its own
transcript, composer and fixture status strip are gone — the shared footer already reports the real
run state, context ring and cost. The panel lists the project's conversations, the same set the
agents sidebar lists: **one registry, two views**, so a conversation started in either is visible
in both, and selecting another or closing the panel ends nothing.

`WorkAgent.account` and `Conversation.account` carry the identity, **reported rather than
requested** — taken from what the run actually resolved, so a conversation cannot claim an account
it is not using.

**Removed, not left:** `state/chat.rs`'s four constant-backed pickers went with the panel's fixture
composer as dead code, rather than staying until P3 could fill them from a real list.

### P3 — The model, chosen before the harness starts — **landed**

**The ordering is forced, and it is cheaper than the obvious one.** A model reaches a harness only
as a launch flag, so it must be answered before the process exists. `discover_models` needs no
running agent — it spawns its own probe — so discovery happens *instead of* starting the agent, not
after it. `crates/ubiq-host/src/coordinator.rs` keeps that ordering as two stages: a conversation
the window asked for sits in `pending_conversations` until its harness actually launches, moving
into `conversations` — where a live pump exists — only then.

1. `StartConversation` carries an `agent_id` the window mints, the `SessionId` precedent, and the
   host adopts it: it registers the `WorkAgent` and answers `ConversationStarted` at once, before
   any harness exists, then discovers that harness's models on a thread of its own.
2. The list arrives as one `ConvUpdate::ConfigOptions`, addressed to that `agent_id` at `seq: 1` —
   a single `model` option, filled from what the harness answered, or a lone "Default" choice when
   discovery could not read anything, per the doc's own rule below.
3. `SetAgentConfig{config_id: "model", ..}` records the pick on the pending agent; nothing answers
   it, since the picker's own list already told the window what "current" means.
4. The window's first `PromptAgent` is what actually launches the harness, carrying the chosen
   model — or none, letting the harness fall back to its own default — on `RunFlags.model`, then
   forwards that same prompt as the harness's first turn.

**The window mints the `agent_id` and draws the pending conversation.**
`StartConversation`'s only caller, `AppState::pick_new_agent_menu`
(`crates/ubiq/src/app/agents.rs`), generates it with `AgentId::generate()` before sending. `Conversation`
carries a `launched` flag, `false` from `Conversation::new` until `ConvUpdate::Started` sets it, and
a `chosen: BTreeMap<String, String>` (keyed by `config_id`) the composer's own pickers write to,
since the host does not echo a `SetAgentConfig` sent before launch. While `launched` is false,
`composer()` (`crates/ubiq/src/ui/conversation/mod.rs`) draws one `Picker` chip per option present
in `conversation.config` — or a "Discovering models…" note before any has arrived — instead of the
footer's read-only pills, and a pick sends `AppState::pick_agent_config`. P3 generalised this from a
single hard-coded `model` picker to any of the (up to) three ids the host may mint — see
[`../tech/decisions.md`](../tech/decisions.md) and
[`../features/workbench.md`](../features/workbench.md).

**Where the vocabulary comes from differs per harness, and none of it is a protocol answer.** Claude
scrapes `/model`'s free text from a one-shot session; Codex answers `codex debug models --bundled`;
Copilot's ids come out of `copilot help config`; opencode prints one per line; Grok reads a cache
file it only writes once authenticated. Three of the five need the harness authenticated, so one
that cannot answer must offer "whatever it defaults to" rather than an empty picker. `ModelInfo` is
not `Serialize`; the host maps it into a `ConfigOption` itself
(`crates/ubiq-host/src/coordinator.rs`'s `model_config_option`) rather than adding a `ubiq-proto`
counterpart.

**Thinking effort is not in this package** — there is no catalog to read. The mode is, and nearly
free: Claude's `init` already reports `permissionMode`.

**Done when** a conversation started from either surface offers that harness's real models before
its first turn, and the harness launches with the one picked.

### P4 — Agent definitions — **landed**

**The identity half.** The settings overlay's Harnesses section lists the accounts Ubiq holds, each
showing which harnesses it can start; `+ Add harness` signs a new one in. Starting a conversation
offers one row per harness *and identity* — `HarnessChoice`, a flat list because the kit has no
submenu and a pick is an index, read by both New agent and New chat so one question has one answer.
The identity is chosen once and read-only in the footer after, because a turn already taken was
taken as somebody.

**The definition half.** A `Profile` carries a `mode` beside its `isolate`, because a permission
mode is a policy axis rather than a composition input — it lands in `spec.policy.permission_mode`,
not in the overlay `ProfileDefaults` describes, so `ProfileDefaults` is the wrong home for it.
`resolve` reads it as `flags.permission_mode.or_else(profile.mode)`, which makes the precedence flag,
then profile, then nothing at all — no `Policy` is minted when neither answers. Nothing else in the
library was needed: `FsProfileStore::save` and `ProfileStore::profiles()` already existed, and the
mode's vocabulary was already `Harness::modes()`, a fixed per-harness list — six for Claude, three
for Codex, empty for opencode, Copilot and Grok, which is how a harness says it has no such concept.

`ProfileInfo { id, agent_type, account, model, mode }` crosses the bus, `AgentTypeInfo` gained the
`modes` a picker draws from, and the profile family is `ListProfiles` / `Profiles` / `SaveProfile`
— documented in [`../tech/transport-contract.md`](../tech/transport-contract.md). `account` stays
beside `profile` on `StartConversation` rather than being folded into it: a bare harness row starts
with no profile at all, and `resolve` puts `flags.account` above the profile's, which is the "the
user picked this one" rule. Host-side, `crates/ubiq-host/src/agent.rs` grows `profiles()` and
`save_profile()` over an `FsProfileStore` rooted at `<ubiq root>/profiles` — a profile that pins no
harness is skipped, since a row with no `agent_type` names nothing that can be started — and
`compose_run` and `converse` thread the id through to `RunFlags.profile`. The pane path passes
`None`: P4 is conversation-only.

**The trap, and it is P3's ordering coming back.** A launch passes the picked model as
`flags.model`, which outranks the profile inside `resolve` — so if the pending conversation's
pickers started empty, a profile's model would be silently launched over. `start_conversation`
therefore seeds `chosen_model` and `chosen_mode` from the profile's record before discovery runs,
the mode picker's `current` reads that seed, and the discovery thread hands the seeded model to
`advertised_model` rather than a `None` that would have redrawn the picker as unset. A profile whose
picks launch correctly while displaying as empty is the failure this shape avoids.

Interface-side, `SettingsState` holds `profiles` and a `profile_form`; `harness_choices` grows a
third group, `Defined`, omitted whole when there are none exactly as `Configured` already is, whose
rows are `HarnessChoice::Profile(usize)`. A row reads `reviewer — Codex · syn · gpt-5 · Plan` and is
drawn faint when that harness is not installed. The form is built like the login modal — a name
field, `choice_pill`s for harness, account and mode, and a free-text model field — and switching
harness clears both mode and account, because both are scoped to a harness and neither survives the
switch meaning anything.

**Cut deliberately, and each is a row in [`../backlog.md`](../backlog.md) rather than an oversight.**
No delete or rename, because `FsProfileStore` has neither and saving over an id is the correction
(`G165`) — a `remove_dir_all` in `agent.rs` would be rule 1 of the boundary crossed. No `extends` in
the form: the library supports chains, and a first offering of one is a tree editor nobody asked
for. No mcps, skills, instructions or isolate fields, which is `G78` from the catalogue's side. The
model is free text because there is no `ListModels` message and discovery happens only inside
`start_conversation`'s own thread (`G166`) — the conversation-start picker still shows the harness's
real list before the first turn. No `thinking`, because `resolve` has no profile leg for one
(`G167`). And no `suggest()` for an unknown profile id: it fails the compose, which
`refuse_conversation` already reports.

### P5 — Credentials, through a login modal — **landed**

The insight that made it cheap: **a login is an interactive subprocess that needs a real terminal,
and Ubiq already owns terminals.** `Harness::login` answers a `LoginPlan` — a launch plus the
credential files to capture — and `ubiq_host::pty::Program` mirrors `Launch` field for field, so
Ubiq spawns it exactly as it spawns a harness. The browser OAuth flow is the harness's own,
unmodified. It runs in a modal rather than a tab because an OAuth flow wants the whole of the
user's attention for the half-minute it takes, and a login that scrolled away behind a pane is a
login nobody finishes.

**The trap that made it more than a pane, and the reason a new library function exists.** A
relocated `$HOME` alone no longer forces the plaintext credential: for Claude Code 2.1.218+ the OS
keychain is merely *unreachable*, which that version reports as an **error** rather than falling
back to a file. Denying it at the policy layer does take the clean fallback path — and isol8 would
have undone that quietly, because `auto_profiles` matches `agents/claude-code` on the command name
and that layer `requires = ["integrations/keychain"]`. So `isolate::login_confined` turns
auto-selection **off** and names its layers (`macos/system-runtime`,
`integrations/launch-services`, `integrations/browser-native-messaging`, and deliberately not the
keychain). Its test asserts on the *resolved* stack, which is what catches the transitive route.

**The outcome is decided by the credential, not the exit code**, because a harness can exit cleanly
having done nothing. The host stamps the credential's mtime before the launch; at the pane's end
there are three answers — fresh (an account exists), untouched (nobody was logged in), absent (the
flow was abandoned, which is what Abort does and is always safe). Only the first records anything,
so a half-finished login leaves nothing to clean up, and creating an account *is* logging one in.
`capture_login` persists a **reference** only, pointing `Account.home` at the capture dir: no
credential bytes are stored and none cross the bus. The family is in
`_docs/tech/transport-contract.md`.

**A `Shell` button beside `Sign in` runs a plain shell under the login's rendered policy instead
of the harness**, so a trap like the `mise` one below can be *seen*, not just reasoned about:
`isolate::login_confined` is called with the harness's own `LoginPlan` exactly as a real login
would, and only the argv rendered after `-p <policy>` is swapped — the policy itself never changes.

**Still open here:** the pasted-key form (a reference by construction — `Account.api_key_env` holds
an env-var *name*, so no secret need cross the bus); deleting or renaming an account, since a
mistyped one is currently permanent from the UI; and the **copy-back gap**, where a token the
harness refreshes inside the run directory is discarded on close so the next run re-seeds the older
one.

### P7 — Real permissions

Gated on library work, and worth stating plainly: **until the bridges stop auto-approving, Ubiq must
not draw a permission prompt.** A dialog that appears after the tool ran is worse than no dialog,
because it teaches the user that the dialog means something. So P7 is: make each bridge hold the
approval, surface it with its options, and answer it from the input side — then, and only then, the
UI asks. Honour ACP's four option kinds (allow once, allow always, reject once, reject always);
"always" is what makes the feature bearable, and where it is remembered is open question 4.

**Done when** a Claude run's file write waits for a click, and denying it visibly stops the tool.

### P6 — The default agent home — **done**

**A defined agent gets a persistent home keyed by its definition, an ad-hoc run gets an ephemeral
one**, under Ubiq's config root and never in a project (`D30`). The mechanism is `isolate::HomeMode`:
`Ephemeral` sets `ephemeral_home`, `Managed(id)` sets `home` to `@managed/<id>`, which isol8 resolves
to `<ubiq root>/isol8/homes/<id>` and creates at spawn time, since `agent.rs` hands `IsolateOptions`
a state directory of `<root>/isol8`.

`HomeMode` is a *sandboxed*-run feature — `isolate::plan` answers `None` for `Isolation::None`, so an
unconfined run materialises no home and replaces no `$HOME`. That is why this waited on `G92`'s
confinement half: `compose_run` used to set `Isolation::None` unconditionally for a conversation, and
a conversation is the only thing carrying a profile after P4. With confinement applied to both faces,
`compose_run` reads `RunFlags.profile` and sets `Managed(<sanitised id>)` when one is there,
`Ephemeral` when it is not — so a pane, which deliberately names no identity, still starts clean.
The one awkwardness, `sanitize_segment` being private to the feature-gated `cli` module, is four
lines copied as `Agents::home_id` rather than a name exported across that boundary. Teardown needed
no change: everything it deletes is under `<root>/runs/`, and a managed home is not.

**A managed home is not P5's login capture.** `Account.home` is `<root>/accounts/<id>`, a
HOME-shaped capture tree used as the login's `$HOME` and afterwards a read-only source copied into
each run; a managed home is the run's own live writable `$HOME`. Pointing one at the other would
close P5's copy-back gap and let a bad run corrupt the stored credential.

**Rejected:** keying `ConfigStrategy::Fixed` by profile id rather than by run and never deleting it.
Two conversations on one profile would share a config directory concurrently — two Claude processes
writing one `.claude.json` — and it is a second persistence mechanism real P6 would have to unwind.

**Done when** a defined agent's second run finds its own cache warm, and an ad-hoc run still starts
clean. The rendered policy's home path is what the test asserts for both
(`crates/ubiq-host/src/agent.rs`).

## Deferred, deliberately

Skills and MCP composition on the wire (`G78`, `G89`); Ubiq's own MCP surface so a hosted agent can
call back into the window (`G7`, with the library's in-process MCP as the mechanism); agents on
remote hosts. None block P1 to P7. Resuming a conversation after a restart has its record — P2c
wrote it — and waits only on the replay that hands it to a fresh harness (`G120`).

## Traps

- **Auto-approval is a security trap, not a feature gap.** See P7.
- **A model is a launch flag, not a setting.** Every bridge refuses `SetConfigOption`, so anything
  that lets a user pick a model after the harness started is a control that cannot work. Ask before
  spawning.
- **A relocated `$HOME` does not force a plaintext credential any more.** Only denying the keychain
  at the policy layer does — and isol8's `auto_profiles` will hand it back by matching the harness's
  own layer on the command name. See P5.
- **A replaced home grants nothing back from the real one.** isol8 auto-grants nothing from the real
  `$HOME` once a login's `$HOME` points at the capture directory, so a harness that is not a
  self-contained binary already named by this policy cannot even start — Codex's Node interpreter,
  reached through a shim that resolves outside the home entirely (`mise`), is the case that found
  this. The trap is not only the keychain; `login_confined`'s `login_runtime_grants` is the fix.
- **A pane with no project silently skips the close path.** `close_pane` returns early when a pane
  belongs to no project, which is every login pane — so a login that exited *successfully* never
  reached `CloseWorkspace`, `pane_gone` never ran, and the credential sat on disk with no account
  recorded. Found and fixed while wiring P5; the shape of the bug is worth remembering, because it
  hit only the happy path and said nothing.
- **An empty string still draws a pill.** A pill is a box with a border, so a record field nobody
  filled renders as a small empty box that reads as a value the interface failed to show. Every
  footer pill is now guarded; new ones must be.
- **A surviving transcript is not a resumable conversation.** P2c leaves the harness's own record
  under `<ubiq root>/sessions/<id>/harness/`, but nothing replays it: a restart, and a resume under
  the same `agent_id`, both start a harness that remembers nothing. The record is the input a
  replay needs, not the replay — `G120`.
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
- **A pending agent's `WorkAgent.account` is what was asked for, not what a run resolves.** Before
  P3, `composed.account()` filled it, taken from the actual run; a pending agent has no run yet, so
  it carries the requested account (or empty) until launch, and nothing corrects it afterwards. The
  two differ whenever a profile supplies an account nobody named on the row — a `Defined` pick with
  an account, or a profile called `default` — and the settings form can now write both, so the gap
  is real rather than hypothetical. It is accepted: the footer reads the resolved account once the
  run exists, and only the pending line can be wrong.
- **A profile's pick outranks nothing — a launch flag outranks *it*.** `resolve` reads
  `flags.model` above `profile.defaults.model` and `flags.permission_mode` above `profile.mode`, and
  a launch always passes what the pickers hold. So a picker that failed to display the profile's
  choice does not merely look wrong, it launches wrong; seeding the pending conversation from the
  profile before discovery is what keeps the two the same answer. See P4.

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
5. **Who mints an `AgentId`?** Settled by P3: the window mints it — `pick_new_agent_menu()` with
   `AgentId::generate()`, the `SessionId` precedent — and the host adopts it in
   `Coordinator::start_conversation` rather than generating its own.
6. **Should `discover_models` take the composed run?** It takes no account and no directory, so a
   list cannot be scoped to an identity and Claude's probe reads the ambient login. A per-account
   list needs the trait signature to change, and nothing yet says a user would notice.
7. **Does the agent process outlive its view, and for how long?** A view is a perspective on a
   conversation the host owns, so closing a panel ends nothing — which means a project accumulates
   live harnesses. Partly answered: a process can now be stopped without ending the conversation —
   `UnloadConversation`/`ConversationUnloaded`, and `ResumeConversation` to start it again under the
   same `agent_id` — so a harness someone is not actively driving no longer has to be killed outright
   to reclaim it. What is still open is *when* — nothing today unloads one on the user's behalf after
   a while of disuse, and a resumed harness starts with no memory of the transcript above it. P2c
   gives it a record to replay from; nothing replays it (`G120`).

## Related docs

- [`../tech/agent-manager.md`](../tech/agent-manager.md) — the boundary every package here crosses
- [`../features/chat.md`](../features/chat.md) — the render model a conversation fills
- [`../features/workbench.md`](../features/workbench.md) — the agents screen, its columns and the settings overlay
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the conversation family is documented once it exists
- [`../../refs/isol8-pty-seam-update.md`](../../refs/isol8-pty-seam-update.md) — the isol8 seam, and what a confined structured run still needs
