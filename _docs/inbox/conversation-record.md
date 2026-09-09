---
id: inbox-conversation-record
title: Proposal — one conversation record, four features
kind: proposal
status: proposal
summary: A conversation's transcript lives in exactly one place, the requesting window's memory, so surviving a restart, reattaching from a second window, forking an agent from an existing one and moving a conversation to another host are four names for the same missing primitive — a host-owned, replayable ConvUpdate record; teleport is what settles its format, and compaction is the step that makes replay affordable rather than a sibling feature.
read_when: you are building restart survival, reattachment, a fork control in the New agent dialog, a compact action, or asking why a resumed harness remembers nothing
updated: 2026-09-09
depends_on: [backlog, tech-agent-manager, tech-architecture, tech-transport, feat-chat, feat-sessions, inbox-agents-definitions]
---

# Proposal — one conversation record, four features

Four capabilities read as separate features and are the same one. **Restart survival** (`G97`),
**reattachment** from a window that did not start the conversation (`G190`), **fork** — a new agent
seeded from an existing one's conversation — and **teleport**, moving a conversation to a host on
another machine. Each is a replay of a conversation into a harness; they differ only in which
`AgentId` receives it and on which host.

Nothing in the tree replays anything, and `G120` names why.

## 1. The gap, stated once

`Conversation` in `crates/ubiq/src/state/conversation.rs` holds `blocks`, the drawn transcript. It
has no host-side counterpart. The pump in `crates/ubiq-host/src/conversation.rs` maps each
`AgentEvent` to a `ConvUpdate`, sends it to the owning window and keeps nothing; when no window is
listening the send fails and the pump ends. `Coordinator` keeps `Conversation` and
`PendingConversation` — a launch recipe and a sequence counter, bookkeeping that makes the *next*
launch behave — and no record of what was said.

So a transcript exists in one place, the requesting window's process memory, for as long as that
window and that host process are both alive. Everything else follows from that.

What is durable is a different thing: `Agents::archive` copies the harness's own transcript files,
named by `Harness::transcripts`, into `<ubiq root>/sessions/<id>/harness/` beside a `SessionMeta`.
`features/sessions-and-workspaces.md` states the limit plainly — the record is data, not a resume.
Nothing parses it back, and only Claude Code implements `transcripts` at all.

Three existing rows describe the consequences from three angles: `G97` (a restart starts every
conversation empty), `G120` (a resumed harness starts as blank as a first launch), `G183` (a
generated name lives in one window, "the same store gap `G97` names for a transcript, and the same
fix would carry both").

## 2. One primitive, four consumers

A host-owned record of a conversation's `ConvUpdate` stream, keyed by `AgentId`, written as the pump
forwards each update. Every one of the four is then that record replayed:

| Feature | The record, replayed into |
|---|---|
| Restart survival (`G97`, `G120`) | a fresh harness under the same `AgentId` |
| Reattachment (`G190`) | a second window viewing the same `AgentId` |
| Fork | a new `AgentId`, the source untouched |
| Teleport | a new `AgentId` on another host |

Fork is the cheapest of the four once the record exists: it is a resume with a fresh id and no claim
on the original. It is also the one with a UI already shaped for it — see §6.

## 3. Teleport settles the format

The record's format is the one decision that cannot be deferred, and moving a conversation between
hosts is what answers it.

Everything host-resident about a conversation is bound to its machine. The run directory under
`runs/<id>/`, the harness's relocated configuration directory, its native session file, the
project's folder, the `workarea` string, and the credentials — `D65` holds that no path and no
credential crosses. Ids do not relocate either: `HostId` in `crates/ubiq/src/app/hosts.rs` never
serialises, `D81` holds that no host can learn another exists, and `Bus` keys its routing maps by
bare `PaneId` and `ProjectId`, so the same id arriving from two hosts overwrites rather than
disambiguates.

A harness-native transcript therefore cannot be the carrier. `Harness::transcripts` is implemented
by Claude Code alone; opencode and Copilot keep their history in a database; Grok writes its
sessions under the real home directory even when the library relocates `HOME`. A machine-bound,
harness-specific, largely unimplemented artifact cannot be what a conversation travels as.

**The record is Ubiq's own `ConvUpdate` log.** It already serialises — it crosses the bus as
MessagePack — it is already sequenced by `ConversationUpdate`'s `seq`, and it is harness-neutral by
construction. It teleports because it is only frames. That single choice makes all four features
draw on one store and keeps `crates/agent-manager` out of the mechanism entirely.

The harness's own transcript keeps the role it has: a forensic copy under `sessions/<id>/harness/`,
written on the way to deleting a run directory.

## 4. Compaction is the replay's compression step, not a fifth feature

Replay cannot be a literal re-send. `G116` records that `Conversation::blocks` grows unbounded and
holds whole diff bodies; handing a long conversation to a fresh harness spends its whole context
window on the first turn. Something has to reduce the record before it is replayed, and that is what
compaction is.

Two layers, which a surface must not conflate:

- **Harness compaction** shrinks the *harness's* context. Every harness that offers it offers it as
  a slash command typed into a turn — `/compact` for Claude Code, Codex and opencode, `/compress`
  for Gemini CLI, nothing for Grok or Copilot. No structured wire in the library exposes it: not
  Claude's stream-json, not Codex's app-server, not the two one-shot NDJSON bridges, not ACP. Codex
  and Copilot document `PreCompact` and `PostCompact` hooks, which observe compaction rather than
  ask for it. Driving it means a command string the library owns — a `compact_command` beside
  `io_support` on `Harness` — and a capability on `AgentTypeInfo` so a surface can hide the control
  for a harness that has none. Ubiq never spells the command itself.
- **Record compaction** folds Ubiq's own log into a seed. It is harness-agnostic, so it serves Grok
  and Copilot too, and it is what makes fork and teleport affordable rather than merely correct.

Occupancy cannot drive either one generally: `G96` records that only Claude Code reports a context
window, so only Claude can offer compaction at a threshold. Every other harness gets a manual
control and nothing else. `UsageRecord`'s `used` is already defined to tolerate a harness compacting
underneath it.

## 5. Two gaps the record needs closed under it first

**A closing window ends its conversations rather than releasing them.** `Coordinator::client_gone`
calls `end_conversation` for every agent the departing client owned — which removes the
`PendingConversation`, drops the ownership entry and calls `Agents::retire_agent`, deleting the run
directory. `Agents::sweep` at boot does the same to whatever a killed process left. So a record
would be built and then destroyed by the two paths that most need it. What is missing is a disowned
state that behaves as `unload_conversation` does — harness stopped, recipe and record kept, no owner
— reached when a client goes rather than when the user asks.

**A conversation is undiscoverable to a window that did not start it.** Conversation-family messages
route `To::Client(owner)`, and ownership is single-client, checked by `drives` on every drive,
unload, abort and resume. There is no message that enumerates conversations and none that transfers
or shares ownership, so a second window cannot learn a conversation exists, let alone draw it.
`features/chat.md` already treats a chat tab as a view onto a host-owned conversation that outlives
it; that contract is only half-reachable while the host answers no one but the starter.

## 6. Fork in the New agent dialog

`crates/ubiq/src/state/new_agent.rs`'s `Target` is already the dialog's first, gating answer — a
harness signed into an account, or a saved profile. A fork is a third variant naming a source
`AgentId`, with its rows drawn from the project's live agents rather than from
`WorkbenchState::harness_choices`, and with harness, account, model and mode prefilled from the
source the way a profile prefills them.

Two constraints the panel imposes:

- **A profile cannot remember a fork source.** A profile is a reusable setup; a fork source points
  at one past conversation. `as_profile` drops it, and the row is not drawn when the form's purpose
  is a profile — the settings page renders the same form.
- **The dialog must not promise byte-identical continuation.** What a forked agent receives is a
  compacted replay, and no harness guarantees that its own resume and a replay agree.

There is a cheap intermediate worth naming, because it needs no host or wire change at all: the
window already holds the transcript, and `start_new_agent` already stashes a preamble in
`agent_preambles` that `send_prompt` folds into the first turn and strips from the echo. A fork that
seeds a new agent with a summary the *window* composes works today, for every harness. It is a
smaller feature than fork-as-replay and it is honest about being one.

## 7. Order of work

Dependency order, and the first three touch no harness code.

1. **Disown rather than end** when a client goes, and stop `sweep` deleting what it has just
   archived. No store yet; this alone lets a conversation outlive its window.
2. **The record**: `ConvUpdate` per `AgentId`, written as the pump forwards. Memory first, then
   disk under `sessions/<id>/` beside the `SessionMeta` already there. Retention arrives with it —
   `G164` holds that nothing deletes a session record, and the record is prompts, file contents and
   tool output.
3. **Enumeration and replay to a viewer**: a message that lists conversations and one that hands
   back a record, so a window draws a conversation it never received live. This is what closes
   reattachment and what makes a restart able to list what it had.
4. **Record compaction**: fold a record into a seed. Fork, teleport and `G120`'s replay all consume
   this one piece.
5. **Replay into a harness**: hand the seed to a fresh process, closing `G120`. Textual injection
   serves every harness; a harness's own native resume stays an optimisation behind a capability.
6. **Fork**: steps 4 and 5 against a new `AgentId`, plus the `Target` variant of §6.
7. **Teleport**: steps 4 and 5 against a new `AgentId` on another host, plus an answer for project
   identity on the destination and for re-provisioning credentials there. `Q10` and `G191` sit in
   front of it.

## 8. Open questions

- **Is the record opt-in per conversation?** `inbox/agents-definitions.md` proposes exactly this as
  a flag settable while an agent is open, and is right that it is a framing over an unbuilt gap.
  Given `G116` and `G164`, opt-in is the safer default.
- **Does a replayed record keep its sequence numbers?** An unload already carries `next_seq`
  forward, so a resumed conversation continues its numbering. A replay has to decide between
  reusing the original numbers and arriving as one bulk answer ahead of a fresh `Started`.
- **Does fork branch a conversation or prime a new one?** The two read identically in the dialog and
  are different builds. A compacted replay is the honest answer to both, which is an argument for
  saying so in the control's own words rather than in a document.
- **`agents-definitions.md`'s retask** — a new goal against an accumulated transcript — is this same
  primitive against the *same* id. Naming it here keeps it from being built twice.
- **Does a record replace `WorkAgent.name`'s store gap too?** `G183` says the same fix would carry
  both. Whether the record holds the name and summary, or a separate row does, is unsettled.

## Related docs

- [`backlog.md`](../backlog.md) — `G97`, `G116`, `G120`, `G164`, `G183`, `G190`, `G96`, `Q10`
- [`tech/agent-manager.md`](../tech/agent-manager.md) — where a run's record is kept, and the
  library's session against Ubiq's
- [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — unload against
  delete, and the record that outlives the run directory
- [`features/chat.md`](../features/chat.md) — a tab as a view onto a host-owned conversation
- [`tech/architecture.md`](../tech/architecture.md) — `D65`, `D81`, `D85`
- [`inbox/agents-definitions.md`](./agents-definitions.md) — resumability and persistence as
  dimensions of an agent
