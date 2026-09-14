---
id: wip-opencode-acp-capture
title: opencode ACP — captured from a live session
kind: wip
status: current
summary: What `opencode acp` (opencode 1.18.28) actually speaks over ACP, captured frame by frame from a real session, and the four gaps it exposed in `io/acp_client.rs` — the turn with no user on the wire, the silent wait, the `task` spawn's shape, and a todo list with no `plan` update — and how the bridge reads them.
read_when: you are fixing opencode's ACP bridge — the user echo, a spawn, or the todo list
updated: 2026-09-13
verified: 2026-09-13
---

# opencode ACP — captured from a live session

Everything below was captured from a real `opencode acp` session on 2026-09-12 — opencode
**1.18.28**, the ubiq-agent session `ses_f63e56491ffePLNaX3yhGLaZ61`, agent id
`01M2E1J73WAHT2GBBHJBVYB5XN` — over most of a two-turn task. The frames are the ubiq bus stream
(session/update notifications mapped onto `ConvUpdate`), with the original wire JSON preserved on
every frame, in `_data/opencode-acp-2.jsonl` (21,388 lines). Counts below are exact.

## 1. A turn with no user on the wire

The variant census:

| `ConvUpdate` | count |
|---|---|
| `ThoughtChunk` | 19,734 |
| `AgentChunk` | 1,286 |
| `ToolCallUpdate` | 283 |
| `ToolCall` | 76 |
| `Usage` | 4 |
| `TurnEnded` | 2 |
| `Started` | 1 |
| `ConfigOptions` | 1 |
| **`UserMessageChunk`** | **0** |

**An opencode live turn never echoes the user.** The binary's ACP surface knows
`user_message_chunk` — the string is in `session/update`'s vocabulary — but it is only ever sent
while *replaying* a loaded or forked session; a live `session/prompt` is never answered by one. The
capture is a live session and carries none, at any point of any turn. The capture began mid-turn
(`Started` is seq 2), so even the prompt that opened it is absent.

The interface draws nothing when the user sends — the user line appears only when a
`UserMessageChunk` lands — so a whole live conversation over this bridge read as the agent muttering
to itself. And because a received `UserChunk` is what sets the turn's running mark, the missing echo
was also the missing "the agent is working": opencode sends no heartbeat and no progress frame of any
kind, so silence was indistinguishable from idleness.

The bridge now synthesises the echo itself: `write_input` answers `AgentInput::Prompt` by emitting
`AgentEvent::UserMessageChunk`, one per text part, the moment the prompt line is written — the exact
call `io/jsonl.rs` makes for Claude Code, which verified the same silence live. This is what puts the
user's turn on the transcript *and* flips the writing mark.

## 2. Three spawns

The session delegated three times (Scaffold, Author _docs, Verify). Each spawn in the raw wire:

- **call frame** — `sessionUpdate: "tool_call"`, `kind: "think"`, `title: "task"`,
  `rawInput: {}` (empty). opencode's `task` tool is a `think`-kind call precisely so the reading
  harness does not block it on a permission prompt.
- **patch frame** — `sessionUpdate: "tool_call_update"`, `title: <the human operation>` — e.g.
  `"Scaffold ubiq-pro workspace"` — and `rawInput: {"description": <operation>, "subagent_type":
  "general", "prompt": <the delegate's brief>}`.
- **completion** — `kind: "think"`, status `completed`, content text starting
  `<task id="ses_…" state="completed">` and closing with `<task_result>`.

`is_delegate` already caught the call by its title, `delegate_calls` kept every later patch a
delegate, and the patch's title is what the transcript shows as the operation. The addition is the
third test in `is_delegate`: a `rawInput.subagent_type` value anywhere marks a delegate — covering a
framing where the call's title is not `task` any more, and a permission ask that re-embeds the tool
call.

## 3. A todo list with no plan update

**opencode never sends ACP's `plan` update.** The kind string exists in its server and nothing emits
it. The whole todo list travels as a **`todowrite` tool call**:

- call frame — `kind: "other"`, `title: "todowrite"`, `rawInput: {}`.
- completion — `tool_call_update`, status `completed`, `title` a size readout (`"4 todos"`,
  `"3 todos"`, `"0 todos"` across the three writes — ignored), and the list itself written *twice*:
  `rawOutput.metadata.todos`, the structured `[{content, status, priority}]`, and the result's own
  text content (`content` blocks) holding the same JSON alone.

`todo_update` now republishes a completion's list as `AgentEvent::Plan { entries }` beside the tool
block it still is, reading `raw_output.metadata.todos` first and reparsing the result text when the
metadata copy is missing. The statuses and priorities map one to one — `pending` / `in_progress` /
`completed`, `low` / `medium` / `high` are exactly `PlanStatus` and `PlanPriority` serde spellings. A
result with no `todos` — any other tool's — passes through untouched, and so does an empty list.

Two limits on the wire, both honest rather than papered over: the list is only ever the *result* of a
completed write, so between two todowrite completions the in-flight todo state is not observable; and
a call's `rawInput` is always `{}`, so the earlier idea of reading the list out of `rawInput.todos`
(read nothing) is what the metadata/content read replaced.

## 4. What the bridge does now

The four changes, all in `crates/agent-manager/src/io/acp_client.rs`:

1. `write_input`'s `Prompt` arm synthesises the user echo (`io/jsonl.rs`'s pattern, §1).
2. `todo_update` + `plan_from_todo_update` translate a `todowrite` completion into a `Plan` (§3).
3. `is_delegate` recognises a spawn by `rawInput.subagent_type` as well as by name and title (§2).
4. `session_update` runs the todo pass last, after `user_chunk`'s filtering.

The interface side renders the plan above the footer: `plan_strip` in `crates/ubiq/src/ui/conversation/
mod.rs` draws up to five entries with a status glyph, when the conversation has one. Tests cover the
echo, the structured read, the text fallback and the no-todos pass-through.