---
id: inbox-subagent-status-dictionaries
title: Shared agent and subagent status dictionaries
kind: proposal
status: proposal
summary: A precise lifecycle/activity model shared by agents and subagents, with a hexagonal UI mark and reliable Claude completion readings.
read_when: you are changing agent or subagent status vocabulary, status indicators, Teams blocks, or delegate completion handling
updated: 2026-09-21
verified: 2026-09-21
code_anchors: [crates/ubiq/src/state/conversation.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/state/teams.rs, crates/ubiq/src/ui/teams/graph.rs, crates/ubiq/src/ui/agents/column.rs, crates/ubiq-host/src/conversation.rs, crates/agent-manager/src/io/jsonl.rs]
depends_on: [feat-chat, tech-transport-contract, tech-ui-and-design]
review_cycle: monthly
---

# Shared agent and subagent status dictionaries

## Purpose

Agents and subagents expose related status information, but the current surfaces mix lifecycle,
activity, coarse buckets and tool-call results. This proposal gives both agents and subagents two
precise dictionaries: one for execution lifecycle and one for activity or terminal result.

The model also gives the Teams block a single combined mark. Its outer hexagonal border represents
lifecycle, its inner fill represents activity or result, and its existing text chip remains visible.
The same vocabulary reaches the agent chat activity bar, the agent column, the Teams graph and any
future subagent surface.

The design also closes the Claude-specific stale reading in which a subagent remains `running` after
its work has ended. A subagent is ended when its delegate completion is explicit, and its lifecycle
must no longer be presented as active.

## Behaviour

### Two dictionaries

The dictionaries are independent. A lifecycle answers whether an execution can continue; an
activity/result answers what it is doing or what happened to the delegated work.

#### Lifecycle

The shared lifecycle dictionary is:

| Value | Meaning | Typical presentation |
|---|---|---|
| `Starting` | The execution is being prepared and has no usable runtime state yet. | Muted, non-pulsing |
| `Ready` | Configuration is available and the execution can accept its first turn. | Success, non-pulsing |
| `Idle` | The execution is alive and can accept another turn, but no turn is running. | Success, non-pulsing |
| `Working` | A turn or delegated execution is in progress. | Info, pulsing |
| `Waiting` | A human response or permission is required before execution can proceed. | Warning, pulsing |
| `Unloaded` | The conversation is preserved but its harness is not running. | Muted, non-pulsing |
| `Ended` | The execution cannot accept further work. | Muted or terminal, non-pulsing |

`Done` is not a lifecycle value. When a subagent reports `Done`, its lifecycle is `Ended`: its
delegated work has completed and it is no longer running.

#### Activity and result

The shared activity/result dictionary is:

| Value | Meaning | Typical presentation |
|---|---|---|
| `Queued` | Work is accepted but has not started. | Muted |
| `Thinking` | The model is reasoning without an attributed tool call. | Info |
| `Writing` | The model is producing agent-facing prose. | Info |
| `Tools` | The execution is handling a tool call. | Info |
| `NeedsYou` | The execution is blocked on a human response. | Warning |
| `Done` | A delegated execution completed successfully. | Success |
| `Failed` | A delegated execution completed with an error. | Danger |
| `Unknown` | The available events do not identify the current activity or result. | Muted |

`Running` is not a precise activity value. It is a compact bucket or label derived from an active
combination such as `Working` plus `Thinking`, `Writing` or `Tools`. A surface may retain the word
`running` as a compact chip, but it must retain the precise dictionary values in state.

The same pair can therefore describe both kinds of record:

| Subject | Lifecycle | Activity/result | Chip |
|---|---|---|---|
| Main agent using a tool | `Working` | `Tools` | `tools` |
| Main agent awaiting permission | `Waiting` | `NeedsYou` | `needs you` |
| Claude subagent executing | `Working` | `Unknown` or a reported activity | `running` or the precise activity |
| Completed subagent | `Ended` | `Done` | `done` |
| Failed subagent | `Ended` | `Failed` | `error` |
| Preserved conversation with no harness | `Unloaded` | `Unknown` | `unloaded` |

### Hexagonal status mark

The Teams agent block combines the dictionaries without replacing the existing chip:

- The **outer hexagonal border** represents lifecycle.
- The **outer border is transparent** as a fill; it carries only the lifecycle stroke, so the block
  interior remains readable and the status does not become a second card background.
- The **inner fill** represents activity or terminal result.
- The existing **status chip remains visible** and continues to say `working`, `done`, `thinking`,
  `tools`, `needs you`, `error` or the applicable compact reading.
- A lifecycle transition changes the border without destroying the activity/result reading.
- A result transition changes the fill without falsely implying that the execution is still active.

The visual combinations are deliberately compositional:

| Lifecycle border | Activity fill | Reading |
|---|---|---|
| `Working` | `Thinking` | An active execution reasoning |
| `Working` | `Tools` | An active execution handling tools |
| `Waiting` | `NeedsYou` | An execution blocked on a human |
| `Ended` | `Done` | A completed subagent |
| `Ended` | `Failed` | A failed subagent |
| `Idle` | `Unknown` | A live agent with no current activity detail |

The agent chat activity bar uses the same pair. Its main-agent row uses the conversation lifecycle
and activity; each subagent row uses the delegate lifecycle and activity/result. The agent column
title and tab indicators continue to show the lifecycle mark, with the activity/result available in
the row chip and tooltip.

### Claude subagent completion

The delegate status is driven by the spawning `Task` tool call. A Claude asynchronous launch can
produce an `InProgress` update while the subagent is running and defer the terminal update until a
later result reports aggregate subagent statistics. The bridge must emit a terminal update when the
Claude result establishes that the delegate completed or failed.

The completion rules are:

1. An explicit delegate completion maps to activity/result `Done` and lifecycle `Ended`.
2. An explicit delegate failure maps to activity/result `Failed` and lifecycle `Ended`.
3. A permission request maps to lifecycle `Waiting` and activity/result `NeedsYou`, regardless of
   the last execution activity.
4. An `InProgress` launch remains `Working` only while no terminal completion or failure is known.
5. Silence alone never proves `Done`; when no terminal event exists, the state remains `Unknown` or
   the last reported active state and may expose a stale/awaiting-completion tooltip.
6. A subagent with `Done` or `Failed` is never included in an active/running count or active pulse.

This preserves the distinction between an ended delegate and an unloaded parent conversation. The
parent can remain `Working` while a delegate is active, or return to `Idle` after the delegate
result is folded into the parent turn.

## Contract

The proposal does not add a transport message. Existing conversation updates carry tool-call
creation and patch events, including `ToolStatus::Pending`, `InProgress`, `Completed` and `Failed`.
The host maps neutral agent-manager events to those updates in
`crates/ubiq-host/src/conversation.rs`, and the Claude native JSONL bridge derives delegate
completion updates in `crates/agent-manager/src/io/jsonl.rs`.

The state layer owns the normalized dictionaries. The UI consumes normalized state and does not
interpret Claude event shapes, aggregate counters or harness-specific completion fields.

## Implementation

### State vocabulary and derivation

`crates/ubiq/src/state/conversation.rs` owns `SubagentTab`, the delegate status derivation and the
conversation lifecycle. The existing `DelegateStatus` and `delegate_status()` are the closest
boundary for the subagent result dictionary; they should become or feed the shared lifecycle and
activity/result types rather than remain a second vocabulary.

`crates/ubiq-proto/src/conversation.rs` owns the transport-level `ToolStatus` values. It remains a
wire status, not the UI lifecycle dictionary. `ToolStatus::Completed` is normalized to subagent
activity/result `Done`, and its lifecycle becomes `Ended`.

`crates/ubiq-proto/src/work.rs` owns the work projection's `Activity` and coarse `Bucket`. The
coarse bucket remains useful for filters and aggregate edges, but it must not replace the precise
pair used by agent and subagent indicators.

`crates/ubiq/src/state/teams.rs` owns `DelegateStatus`, `delegate_status()`, and the Teams filtering
rules. Its `Done` filtering remains valid, but active filtering and pulse decisions must use the
normalized lifecycle rather than treating every non-error delegate as running.

### Shared presentation dictionary

`crates/ubiq/src/ui/work.rs` is the natural home for shared activity/result presentation because it
already maps `Activity` and `Bucket` to colors. It should expose presentation records or functions
for labels, colors, icons, pulse behaviour and compact buckets.

`crates/ubiq/src/ui/conversation/mod.rs` owns the existing lifecycle presentation functions:
`lifecycle()`, `lifecycle_colour()`, `lifecycle_pulses()`, `lifecycle_mark()` and
`lifecycle_menu()`. The lifecycle dictionary should remain centralized there or move to a shared
status module consumed by this file, the Teams graph and the agent column.

The shared presentation layer must accept normalized lifecycle and activity/result values. It must
not accept a Claude-specific event, a `ToolStatus` directly, or a guessed `running` boolean.

### Agent chat and activity bar

`crates/ubiq/src/ui/conversation/mod.rs` draws the main and subagent rows in `activity_bar()`.
The main row currently uses conversation lifecycle presentation while delegate rows use
`status_label()` and `status_colour()` over `ToolStatus`. Replace that split with the shared pair:
the main row reads the conversation pair, and each delegate row reads the normalized delegate pair.

The delegate row keeps its status chip. The chip may use the compact `running` bucket while its
tooltip and accessible label expose the precise pair, such as `Working · Tools` or `Ended · Done`.

### Agent columns and Teams blocks

`crates/ubiq/src/ui/agents/column.rs` draws the lifecycle mark beside an agent title and on its
tabs. It should continue to give the lifecycle mark priority while adding the shared activity/result
reading to the row or tooltip without duplicating the title text.

`crates/ubiq/src/ui/teams/graph.rs` draws the Teams agent blocks and is the primary home for the
hexagonal status mark. The block receives both normalized values, draws a transparent outer
hexagonal border for lifecycle, draws the activity/result fill inside it, and preserves the existing
chip. The geometry should be a reusable UI primitive rather than a Teams-only interpretation so the
same mark can reach another agent surface.

`crates/ubiq/src/state/layout.rs` owns Teams block geometry and placement. If the hexagon requires
an inset or a fixed status-mark size, those measurements belong with the existing card and subagent
layout constants rather than in the renderer as untracked literals.

`crates/ubiq/src/theme.rs` remains the only source of concrete colors. The presentation dictionary
uses existing theme tokens or adds paired tokens for lifecycle strokes and activity fills in both
palettes. No hex color is placed in a Teams or conversation renderer.

### Claude bridge and host mapping

`crates/agent-manager/src/io/jsonl.rs` owns Claude native JSONL interpretation. Its mapper tracks
launched asynchronous delegate calls and emits `ToolCallUpdate::finished()` when a result's
`subagent_stats` establishes completion or failure. The completion path must cover the case where
the delegate's own transcript has ended but the parent result arrives later.

`crates/ubiq-host/src/conversation.rs` remains the only mapping between
`agent_manager::io::AgentEvent` and `ubiq_proto::conversation::ConvUpdate`. It maps the terminal
tool update without inventing another subagent status vocabulary.

Tests in `crates/agent-manager/src/io/jsonl.rs`, `crates/ubiq-host/src/conversation.rs` and
`crates/ubiq/src/state/conversation.rs` should cover launch, delayed completion, failure,
permission waiting, duplicate completion and a delegate whose parent call never receives a
terminal event. UI tests in `crates/ubiq/tests/agents.rs` and `crates/ubiq/tests/teams.rs` should
assert that `Done` has an ended lifecycle, no active pulse and a visible chip.

## Failure

If a harness omits a delegate terminal event, the UI does not infer success from elapsed time,
missing output or the parent turn ending. It keeps the delegate's last reported state and marks the
activity/result as `Unknown` where the precise state is no longer trustworthy. The tooltip can say
that completion is unreported, but the block does not claim `Done`.

If a completion event arrives after a stale `InProgress` reading, the terminal update wins, the
outer border changes to `Ended`, the inner fill changes to `Done` or `Failed`, and the pulse stops.
Duplicate terminal updates are idempotent.

If a permission request arrives after a delegate completion, the completion remains terminal and the
late request is surfaced as a protocol inconsistency rather than changing `Done` back to `Waiting`.
If a delegate is unloaded with its parent conversation, the parent reads `Unloaded`; the delegate's
last result remains available in the transcript and is not relabelled as a successful completion.

## Related docs

- [The chat panel](../features/chat.md) — shared conversation view, lifecycle mark and activity bar.
- [The workbench](../features/workbench.md) — agent columns, Teams blocks and work projection.
- [UI and design](../tech/ui-and-design.md) — theme tokens, status colors and reusable shape rules.
- [Agent Client Protocol](../references/acp-protocol.md) — upstream tool-call and update vocabulary.

## Next steps

- Add the normalized lifecycle/activity-result types and presentation records.
- Correct Claude delayed delegate completion and add regression fixtures.
- Implement the reusable transparent-border hexagon and preserve status chips.
- Add state and UI coverage for ended, failed, waiting and unreported delegate states.
