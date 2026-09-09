---
id: inbox-chat-resume
title: Proposal — resume and fork a chat through the harness's own session
kind: proposal
status: proposal
summary: A conversation survives a restart and forks onto a second agent by handing the harness back its own session id, not by replaying a Ubiq-owned record — the resume token is already on disk in `SessionMeta.harness_session_id` keyed by the `AgentId`, five of six structured bridges already replay the whole transcript on resume, the remaining costs are a window that remembers its tab attachments and a closing window that stops deleting them, and compaction enters as a boundary the transcript has to draw rather than as a mechanism Ubiq has to build.
read_when: you are building restart survival, a fork control or a compaction control for a chat tab, deciding whether a conversation record has to exist first, or asking why the resume plumbing that is already built resumes nothing
updated: 2026-09-09
depends_on: [backlog, tech-agent-manager, tech-architecture, tech-transport, feat-chat, feat-sessions, ref-acp-protocol, inbox-conversation-record]
---

# Proposal — resume and fork a chat through the harness's own session

Two capabilities are asked for: a chat that comes back after Ubiq quits, and a chat cloned onto a
second agent **of the same harness**. This proposes both, and proposes them without building a
conversation record first.

[`inbox/conversation-record.md`](./conversation-record.md) reaches the opposite conclusion from the
same gap, and reaches it honestly: it is answering a wider question — reattachment across windows,
and teleport to another machine — where a harness-neutral record is the only possible carrier. For
those two, its argument stands. **Restricted to one harness on one host, the harness itself is the
carrier**, and the build is roughly a tenth the size. §7 states the divergence in full.

## 1. Most of this is already built, and resumes nothing

Four facts, none of them obvious from the backlog rows:

- **The `AgentId` is minted by the interface**, not the host — `crates/ubiq/src/app/new_agent.rs:372`
  and `crates/ubiq/src/state/layout.rs:808`. The window chooses the identity; the host adopts it.
- **That same id names the durable record.** `Agents::archive` writes
  `<ubiq root>/sessions/<agent id>/meta.json` with `meta.id` set from the key
  (`crates/ubiq-host/src/agent.rs:937`).
- **The resume token is already in it.** `SessionMeta.harness_session_id`
  (`crates/agent-manager/src/session.rs:32`) holds the harness's own session id, captured off
  `AgentEvent::SessionStarted`. It is written at teardown *and* by `Agents::sweep` at boot after a
  crash, so it survives both a quit and a kill.
- **The resume path from token to harness is complete.** `RunSpec::resume` reaches `--resume <id>`
  for Claude Code (`crates/agent-manager/src/harness/claude.rs:377`) and `session/load` for every
  ACP harness (`crates/agent-manager/src/io/acp_client.rs:439`), and
  `crates/agent-manager/src/cli/session.rs` already drives the whole thing from a `SessionMeta` as
  `am session resume`.

So a resume token, keyed by exactly the id the window would ask for, is written to disk today, and a
working resume already consumes one. What is missing is the read: nothing loads that record at
launch, which is what `G97` means by "nothing reads that store back".

The in-memory half is the part everybody sees. `PendingConversation.resume`
(`crates/ubiq-host/src/coordinator.rs:266`) holds the token for the process's lifetime, fed from
`ConvUpdate::Started.session_id` (`crates/ubiq-proto/src/conversation.rs:430`), whose own comment
already calls it "the one a resume needs". It dies with the process — `G95`.

## 2. What each harness can actually do

| Harness | Native resume | Replays the transcript on resume | Native fork |
|---|---|---|---|
| `claude-code` (jsonl) | `--resume <id>` — built | no | `--fork-session` in the CLI, unwired |
| `claude-code-acp` | `session/load` — built | **yes, in full** | `session/fork`, draft only |
| `codex-acp` | `session/load` — built | **yes, in full** | `session/fork`, draft only |
| `opencode`, `copilot`, `grok` | `session/load` — built | **yes, in full** | `session/fork`, draft only |
| `codex` (native app-server) | absent | absent | absent |

The second column is the one that changes the design. ACP's `session/load` "replays the whole
conversation as `session/update` notifications *before* it responds", as
`crates/agent-manager/src/io/acp_client.rs:506` records — so for five of six structured bridges a
resume returns the harness's memory **and** the drawn transcript, through the pump that already
exists, as ordinary `ConvUpdate`s. No record, no format, no compaction.

Two exceptions, each with an answer:

- **Claude Code's native bridge** restores memory silently. Its transcript is on disk in a format
  the tree already parses: `Harness::transcripts` names `<config dir>/projects/*/*.jsonl`
  (`crates/agent-manager/src/harness/claude.rs:155`, the only implementation), `Agents::archive`
  copies those files to `<ubiq root>/sessions/<id>/harness/` (`crates/ubiq-host/src/agent.rs:1004`),
  and `crates/agent-manager/src/io/jsonl.rs` is a parser for exactly that wire. Replay is that
  parser over a file instead of a pipe.
- **Codex's native app-server** cannot resume at all — `CodexBridge::new` takes `(child, cwd)` and
  no token (`crates/agent-manager/src/io/codex.rs:172`), and `harness/codex.rs:264` says outright
  not to invent a flag. `codex-acp` is the resumable Codex, and `D95`'s sibling-id pattern is
  already the shape for saying so.

## 3. Restart resume — three changes

**One. Stop destroying it.** `Coordinator::client_gone` calls `end_conversation` for every agent the
departing client owned (`crates/ubiq-host/src/coordinator.rs:852`), which retires the run directory;
the user-driven `UnloadConversation` keeps all of it. A closing window has to take the unload path,
not the end path. This is `G207` unchanged, and it is the only prerequisite this proposal shares
with the record proposal.

**Two. The window remembers its attachments.** `ChatTab.attached: Option<AgentId>`
(`crates/ubiq/src/state/chat.rs:16`) is already the whole binding, and `ViewPrefs`
(`crates/ubiq/src/state/prefs.rs:130`) is already a per-project blob written through
`crates/ubiq-host/src/atomic.rs`. Chat tabs and their attached ids go in it. `features/chat.md`'s
own next-step asks for this at `_docs/features/chat.md:490`, and the comment presently declining it
gives the reason it can be reversed: a chat id is reminted every run *because* an attachment points
at nothing. Once an attachment resolves, the tab is worth writing down.

**Three. `StartConversation` accepts the token.** The message
(`crates/ubiq-proto/src/messages.rs:1077`) carries every launch pick and no resume field. Adding one
— populated by the host from `sessions/<agent id>/meta.json` when the requested id already has a
record — is what turns the existing plumbing on. The host reads the record; the window supplies only
the id it remembers.

Nothing here enumerates conversations, so **`G208` is not a prerequisite.** The window asks for its
own tabs by id. A surface listing *every* past conversation is a separate feature, and it is the one
that needs enumeration.

## 4. Fork

A fork is a resume with a fresh `AgentId` and no claim on the source. On the same harness, that is a
harness operation:

- **Claude Code**: `claude --resume <src> --fork-session` continues the source's context under a new
  session id, leaving the original untouched — exact, immediate, no replay. The cost is a `fork`
  flag on `RunSpec` and one argv branch beside the `--resume` at `harness/claude.rs:377`. The CLI
  also offers `--session-id <uuid>`, which would let Ubiq assign the token rather than capture it;
  worth noting, not needed.
- **Every ACP harness**: `session/fork` exists only in the unstable draft schema
  (`_docs/references/acp-protocol.md:1585`) and `AcpBridge` does not implement it. `session/load`
  is not a substitute — it loads *into* the named session, so two agents would share one id and
  collide on write. **ACP forks when the draft lands**, gated on the capability the way
  `loadSession` already is.

Until then the honest fallback is the one the record proposal's §6 already identifies as free: the window holds the transcript, and `agent_preambles` already folds a
composed preamble into a new agent's first turn. A summary-seeded fork works today, for every
harness, and is a different and weaker thing than a fork — the control should say which one it gave.

In the interface, this is `Target` in `crates/ubiq/src/state/new_agent.rs:27` gaining a third
variant naming a source `AgentId`, with the source's harness fixed rather than chosen. §6 of
The record proposal works out that dialog, and none of it changes
here beyond one constraint being liftable: a native fork **can** promise continuation, where a
compacted replay cannot.

## 5. Compaction — a boundary to draw, not a mechanism to build

Compaction is three different things wearing one word, and separating them is most of the work.

**Shrinking the harness's context** is the harness's own job, and it already does it. Claude Code
takes `--autocompact <auto|tokens>` at launch, so the threshold is a provision-time flag Ubiq could
set from a profile with no wire work at all. A native resume then inherits whatever the harness
compacted, which is the whole reason §1's design needs no seed.

**A Ubiq-side automatic trigger is redundant, and provably so.** Firing at a threshold needs
occupancy; `G96` records that only Claude reports a context window, and `UsageRecord::context_pct`
(`crates/ubiq-proto/src/conversation.rs:376`) answers `None` for everyone else. So for the one
harness where Ubiq could decide, the harness already decides — better, because it knows its own
model — and for every harness where Ubiq would add something, it has no denominator to decide with.
There is no configuration of this feature that is worth its code.

**A manual control is the part that adds something**, and its discovery is already built.
`AgentEvent::AvailableCommandsUpdate` (`crates/agent-manager/src/io/model.rs:742`) carries "every
slash command the session has", parsed for every ACP harness by
`crates/agent-manager/src/io/acp.rs:502` and routed through `crates/agent-manager/src/io/acp_client.rs:1745`.
The harness states whether it has a `/compact`; Ubiq draws the control where one is offered and
draws nothing where none is. Sending it needs no new input variant either —
`crates/agent-manager/src/io/jsonl.rs:264` writes prompt text verbatim and no part of the crate
inspects a leading slash. This is cheaper than the `compact_command` on `Harness` that the record
proposal's §4 assumes, and it keeps that document's own rule — Ubiq never spells the command —
by construction rather than by discipline. Claude Code's native bridge is the one that would need
work: it reports `slash_commands` on `init` and `jsonl.rs` does not read the field.

**What actually matters for persistence is the boundary.** A compaction is invisible to Ubiq: ACP
has no notification for it, no capability, and no `StopReason` for a context that ran out
(`_docs/references/acp-protocol.md:724`). The only evidence is `used` falling, which a fresh turn
does too — `crates/agent-manager/src/io/model.rs:776` says as much. The interface is already honest
about this at the ring, where the tooltip reads "A level, not a total: it falls when the
conversation is compacted", and `crates/ubiq/src/ui/kit/controls.rs:230` redraws smaller with no
high-water mark.

The transcript is not honest about it, and persistence is what turns that from cosmetic into wrong.
After a compaction the window still shows every block while the harness has forgotten most of them.
Today a chat dies with its window and the discrepancy is short-lived. Once a tab comes back after a
restart, a user reads several hundred restored blocks and reasonably concludes the agent remembers
them. It does not.

So the one compaction change this proposal asks for is a marker. Claude Code already emits a
`system` event with subtype `compact_boundary`, and `crates/agent-manager/src/io/jsonl.rs:612`
special-cases only `init` before dropping every other subtype into a `trace!`. Mapping it to a
`ConvUpdate` the transcript draws as a divider — memory starts here — costs one match arm and one
renderer. Where a harness states no boundary, none is drawn: inferring one from a `used` drop would
be a guess, and the same rule that keeps a ring off an unreported window keeps a divider off an
unreported compaction.

This has a dependency the order of work has to respect. §8's step 4 replays Claude's archived
transcript to redraw a resumed tab, and that file holds pre-compaction history the harness no longer
carries — so the replay is only honest once the boundary is mapped. The boundary comes first.

**`G116` is not this problem despite sharing the word.** `Conversation::blocks`
(`crates/ubiq/src/state/conversation.rs:197`) grows unbounded in the window's memory, holding whole
diff bodies, with no eviction anywhere. That is a client memory bound answered by paging, and it is
independent of what the model carries. Persistence makes it worse — longer-lived conversations, and
a restored tab that starts full rather than empty — so it is worth naming here, and it is not a
prerequisite.

## 6. What is deliberately not built

- **No host-owned `ConvUpdate` record.** The harness holds the transcript; on ACP it hands it back.
- **No compaction Ubiq drives on its own.** §5: the harness compacts itself, a threshold Ubiq could
  fire on exists for one harness only, and a manual control draws from the command list the harness
  already reports. Record compaction, the record proposal's §4, has nothing to fold and stays unbuilt.
- **No cross-harness fork.** Forking Claude onto Codex needs the record, and the record is what §7
  defers rather than deletes.
- **No reattachment, no teleport.** Both need `G208` and a harness-neutral carrier.
- **No retention.** `G164` is untouched and gets worse the moment tabs start coming back: a session
  record is prompts, file contents and tool output, nothing deletes one, and this proposal gives the
  user a reason to accumulate them. A delete belongs in step 3.

## 7. Where this diverges, and why both can be true

[`conversation-record.md`](./conversation-record.md) argues that one primitive serves four features,
and that harness-native resume is "an optimisation behind a capability" rather than the mechanism.
Its reasons are sound and remain so: `Harness::transcripts` is Claude-only
(`crates/agent-manager/src/harness/mod.rs:635` defaults to empty), opencode and Copilot keep history
in a database, Grok writes under the unrelocated home, and none of that travels between machines.

The divergence is about scope, not about facts. That document's four features share a primitive
because they are stated harness-neutrally and host-neutrally. **This request is neither.** Same
harness, same host, same machine — and under that restriction the native path wins on every axis
that matters: it is exact rather than approximate, it needs no new format, no compaction and no
store, and on ACP it is already written.

So the two documents are a build order, not a contradiction:

| | Native session (this document) | `ConvUpdate` record |
|---|---|---|
| Restart, same harness | ✔ | ✔ |
| Fork, same harness | ✔ (Claude today, ACP on the draft) | ✔ |
| Fork onto a different harness | ✗ | ✔ |
| Reattach from a second window | ✗ | ✔ |
| Teleport to another host | ✗ | ✔ |

The record is the general answer and stays on the roadmap. This is the answer to the question asked,
and shipping it first buys the record's design a real user of restart survival to be measured
against. Nothing here forecloses it: the record would sit beside the native path as the fallback for
the rows this table marks `✗`, which is the arrangement `G120` describes from the other direction.

## 8. Order of work

Each step is testable alone, and the first two are the ones that make a tab come back at all.

1. **Disown rather than end** on `client_gone` — `G207`. No wire change, no store. On its own this
   makes a conversation outlive its window within one host process.
2. **Persist chat tabs and their attachments** in `ViewPrefs`, and **read `SessionMeta` back** at
   launch behind a resume field on `StartConversation`. This closes restart survival for the five
   ACP bridges outright, transcript included, and for Claude Code restores the harness's memory with
   an empty transcript above it.
3. **Retention** — a delete for a session record, `G164`. Before, not after, tabs start accumulating.
4. **The compaction boundary** — §5: `compact_boundary` mapped out of `io/jsonl.rs`'s catch-all and
   drawn as a divider. Small on its own, and step 5 is dishonest without it.
5. **Claude's transcript replay**: `io/jsonl.rs`'s mapper over an archived
   `sessions/<id>/harness/*.jsonl`, emitted ahead of the fresh `Started`. Closes the one gap step 2
   leaves, and is the only step that touches `crates/agent-manager`'s reading of a file rather than
   a pipe.
6. **Fork, Claude-native**: a `fork` flag on `RunSpec`, the argv branch, and the `Target` variant.
7. **Fork, ACP**: `session/fork` behind its capability, when the draft lands — `G214`'s versioning
   caution applies to it exactly as to the subagent extension.

Steps 1-3 are host and interface only. Steps 4 and 5 are `crates/agent-manager` changes that add no
harness knowledge. Step 6 is one flag.

Two compaction items sit outside this line because nothing here depends on them: surfacing
`AvailableCommandsUpdate` as a manual control, and `--autocompact` as a profile knob. Both are
cheap, both are independently useful, and neither blocks a tab from coming back.

## 9. Open questions

- **Does a resumed tab redraw or start clean?** On ACP the replay arrives whether or not the tab
  wants it, so the sequence numbering has to decide between continuing the old count and restarting.
  `UnloadConversation` already carries `next_seq` forward, which argues for continuing.
- **What does a resume do when the harness has forgotten?** A token whose session the harness has
  deleted fails at `session/load` or at `--resume`. Falling back to a fresh launch silently is wrong;
  the tab should say the harness no longer has it.
- **Does the record survive a config directory that is gone?** `am session resume` errors when
  `SessionMeta.config_dir` no longer exists (`crates/agent-manager/src/cli/session.rs`), and
  `G182` records that the home mode is not on the record at all. A Ubiq resume rebuilds the run
  directory from the recipe, so it may not need the old one — unverified.
- **Is a fork's source shown in the child?** A forked conversation whose transcript starts mid-thought
  is confusing without a marker naming what it came from.
- **Does a fork inherit the source's boundary, or start clean above it?** `--fork-session` carries
  the compacted context across, so the child's first drawn block sits after a divider it did not
  earn. Showing the source's history below the fork point is honest and long; showing none is short
  and mysterious.
- **Should a resumed tab restore every block, or only those after the last boundary?** Restoring
  everything is truthful about what was said and misleading about what is remembered; the divider is
  what makes the long version defensible, which is an argument for step 4 preceding step 5 rather
  than for trimming.
- **Should `--session-id` replace token capture for Claude Code?** Assigning the id at launch makes
  the mapping deterministic and removes the dependency on `SessionStarted` arriving. It also means
  Ubiq naming something inside the harness's own store, which no other harness would accept.

## Related docs

- [`inbox/conversation-record.md`](./conversation-record.md) — the general primitive, and the four
  features this document takes two of
- [`backlog.md`](../backlog.md) — `G95`, `G97`, `G120`, `G164`, `G182`, `G207`, `G208`, `G214`
- [`features/chat.md`](../features/chat.md) — a tab as a view onto a host-owned conversation, and
  the persistence its next steps ask for
- [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — unload against
  delete, and the record that outlives the run directory
- [`tech/agent-manager.md`](../tech/agent-manager.md) — the library's session against Ubiq's
- [`references/acp-protocol.md`](../references/acp-protocol.md) — `session/load`, and the draft
  methods `session/fork` sits among
