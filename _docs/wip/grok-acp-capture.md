---
id: wip-grok-acp-capture
title: Grok ACP — captured from the live binary
kind: wip
status: current
summary: What `grok agent stdio` (grok 1.0.13) actually speaks, captured frame by frame, and the gaps between it and `io/acp_client.rs`.
read_when: you are fixing Grok's ACP bridge — modes, models, authentication or subagents
updated: 2026-09-10
verified: 2026-09-10
---

# Grok ACP — captured from the live binary

`crates/agent-manager/_docs/harness/grok.md` describes the **npm `@vibe-kit/grok-cli`**. The
installed harness is a different program: xAI's official **`grok` 1.0.13** (Rust). Everything below
was captured from `grok agent stdio` on 2026-09-10 with a real account, not inferred.

## 1. `authenticate` is mandatory

`initialize` answers `authMethods: [{id:"cached_token"},{id:"grok.com"}]` and
`_meta.defaultAuthMethodId: "cached_token"`. Without a preceding `authenticate`, **`session/new`
fails**:

```json
{"code":-32000,"message":"Authentication required","data":"no auth method id provided"}
```

`{"method":"authenticate","params":{"methodId":"cached_token"}}` succeeds against the cached
`~/.grok/auth.json` and requires no interaction. `acp_client.rs:445-455` deliberately never calls
`authenticate`, so **every Grok ACP conversation dies at `session/new` today**.

## 2. Models and modes come back in the vendor shapes, not `configOptions`

`session/new` result (abridged, verbatim field names):

```json
{"sessionId":"01a08a54-…",
 "models":{"currentModelId":"grok-4.6",
   "availableModels":[{"modelId":"grok-4.6","name":"Grok 4.6","description":"…",
     "_meta":{"totalContextTokens":500000,"supportsReasoningEffort":true,"reasoningEffort":"high",
       "reasoningEfforts":[{"id":"xhigh","label":"Extra High Effort","default":false},
                           {"id":"high","label":"High Effort","default":true},…]}},
     {"modelId":"grok-4.5",…}]},
 "_meta":{"x.ai/sessionConfig":{"options":[
     {"id":"grok-4.6","category":"model","label":"Grok 4.6","selected":true},
     {"id":"grok-4.5","category":"model","label":"Grok 4.5","selected":false},
     {"id":"high","category":"mode","label":"High Effort","selected":true},…]},
   "x.ai/sessionDetail":{"kind":"build","cwd":"…","currentModelId":"grok-4.6"}}}
```

The `x.ai/sessionConfig` entries are flat — one row per choice, `category` saying which picker it
belongs to and `selected` marking the current one — where the `models` block is nested. The mode
rows repeat the selected model's `reasoningEfforts`.

There is **no `configOptions` and no `modes` block**. `session_events`
(`acp_client.rs:709-741`) reads only those two, so the interface is handed no choices at all —
that is the "no thinking mode, no harness mode" report.

Note what Grok calls things: **`category: "mode"` is the reasoning effort**, not a permission mode.
Grok exposes no permission mode over ACP at all (see §4).

The same `modelState` block is also on the `initialize` result's `_meta`, so the model list is
known before a session exists.

### Setting them — probed method by method

| Method | Result |
|---|---|
| `session/set_model` `{sessionId, modelId}` | **exists** |
| `session/set_mode` `{sessionId, modeId}` | **exists** (`modeId` = a reasoning-effort id) |
| `session/set_config_option` `{sessionId, configId, value}` | exists |
| `session/select_model`, `x.ai/set_model`, `session/select_mode` | method not found |

Ubiq only ever sends `session/set_config_option` (`acp_client.rs:881`), which Grok accepts but
which names no option Grok advertises.

Agent-initiated change arrives as `_x.ai/session_notification` with
`{"update":{"sessionUpdate":"model_changed","model_id":"grok-4.6","reasoning_effort":"high"}}`.

## 3. The model at launch

`grok agent stdio` accepts `-m/--model`, and the agent options go *between* `agent` and the
transport name: `grok agent --model grok-4.6 --always-approve stdio`. **It accepts the flag and
ignores it** — see §7: a session started with `--model grok-4.5` comes back on `grok-4.6`.
`--reasoning-effort` on the same command line *is* honoured.

`Grok::launch` used to drop the model on the structured path altogether, on the belief that it was
a config option, and `AcpBridge::new` was never given one either — so the resolved model reached
the process by no route at all. It now goes out both ways: on the command line, where it costs
nothing, and over the wire as a `session/set_model` after `session/new`, which is the one that
actually takes. `Provisioned::model` carries it to the bridge, the way `Provisioned::resume`
already carried a resume id.

## 4. Permissions: `--always-approve`, or `_meta.yoloMode` per session

From the guide bundled under the Grok home, in `docs/user-guide/` — the agent-mode chapter and the
permissions-and-safety one:

- `grok agent --always-approve stdio` (alias `--yolo`) — process-wide.
- `session/new` `_meta`: `{"yoloMode": true}` — **per session**, and `{"autoMode": true}` for
  Grok's `auto` mode. Also `rules`, `systemPromptOverride`, `agentProfile`.
- The TUI's own `--permission-mode` values are `default | acceptEdits | auto | dontAsk |
  bypassPermissions | plan`.

`_meta.yoloMode` is exactly the wire for a per-conversation "accept all".

## 5. Subagents — Grok streams them on a second `sessionId`

Captured from a run that spawned one:

- The spawn is an ordinary `tool_call` titled **`spawn_subagent`**, with
  `_meta["x.ai/tool"] = {"name":"spawn_subagent","kind":"task","label":"Subagent",…}` and
  `_meta.subagentBackground: true`. `rawInput` carries `description`, `prompt`, `subagent_type`
  (e.g. `explore`) and `background`.
- Its completion content names the child: `subagent_id: 01a08a55-9099-…`.
- **The child's whole transcript then arrives as normal `session/update` notifications whose
  envelope `sessionId` is that `subagent_id`** — `agent_thought_chunk`, `agent_message_chunk`,
  `tool_call`, `tool_call_update`, and a `user_message_chunk` carrying the child's prompt.
- Grok sends **no** `subagent_spawned` / `subagent_state_update` (the ACP draft extension
  `track_subagent` implements), so `state.subagents` stays empty and every child chunk falls to the
  open-delegate heuristic — or, for `user_message_chunk`, to nothing at all, because that variant
  carries no origin. That is why a delegate's prompt and its
  `<system-reminder>Background subagent … completed</system-reminder>` notice render as **user
  turns**.
- Polling a finished background child is a `get_command_or_subagent_output` tool call
  (`_meta["x.ai/tool"].kind == "background_task_action"`).

`is_delegate` (`acp_client.rs:1945`) matches only `task`/`agent`, so `spawn_subagent` is not
recognised either.

## 6. Other Grok extensions seen on the wire

`_x.ai/mcp/servers_updated`, `_x.ai/models/update`, `_x.ai/settings/update`,
`_x.ai/announcements/update`, `_x.ai/mcp_initialized`, `_x.ai/sessions/changed`,
`_x.ai/queue/changed`, `_x.ai/session_notification`, and standard
`session/update: available_commands_update | session_info_update`. None of them are required.

## 7. What the bridge does now, run end to end

`am grok --account mdn --io structured --model … --thinking …` against the live binary, after the
fixes in `acp_client.rs` and `grok.rs`:

The argv is `grok agent --model grok-4.5 --reasoning-effort low stdio`, and the run answers with a
`session_started`, then a `config_option_update` carrying **both** pickers — a `model` option
offering `grok-4.6` and `grok-4.5`, and a `mode` option offering `xhigh`, `high`, `medium` and
`low` — then the ordinary thought, message, usage and `turn_ended` events.

Both pickers being populated is what was missing. Two things this run pins down:

- **`--reasoning-effort` is honoured** by `grok agent stdio`: `--reasoning-effort low` came back as
  `mode: "low"`.
- **`--model` is not.** `--model grok-4.5` came back as `model: "grok-4.6"`, and the model option's
  `current_value` agreed. The flag is accepted and ignored for an `agent stdio` session, so a
  requested model has to be applied with `session/set_model` after `session/new`.

Re-run once the bridge does that, on the same command line: `session_started` reports
`"model":"grok-4.5","mode":"low"`, the model option's `current_value` is `grok-4.5`, and the turn
answers and ends normally. The requested model is what the session runs on, and it is what both
the `SessionStarted` event and the picker report.

## 8. `grok models`

```text
Default model: grok-4.6

Available models:
  * grok-4.6 (default)
  - grok-4.5
```

The starred entry is the default. `Grok::discover_models` reads the bulleted list under
`Available models:` and falls back to a `grok-`-prefixed token scan if that heading is not there.
