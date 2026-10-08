---
id: inbox-codex-app-server
title: Problem — Codex converses, but tells Ubiq nothing about tokens, context, delegates or limits
kind: proposal
status: proposal
summary: Codex already runs as a conversation over `codex app-server` (`io/codex.rs`), but the bridge was written from prose rather than the wire — it reads `itemType` where the protocol says `type`, ends the turn on any thread's completion, auto-approves everything and drops every usage figure. Generated against codex-cli 0.161.0's own schema, this maps the app-server's v2 vocabulary onto the existing accounting contract (`UsageUpdate`, `RateLimitUpdate`, `Origin`) so Codex reaches Claude's parity on context ring, spend, delegate split, the 5h and weekly windows and per-account quota — all inside agent-manager, with no new message on the bus — and stages it as six board-sized phases. §12 says what has landed, in the library and (T-361) in the host and the interface — fork, persistence, steering, identity, device login.
read_when: you are changing the Codex bridge, adding Codex telemetry, quota or subagents, or asking why a Codex conversation draws no ring and no limits
updated: 2026-10-08
depends_on: [tech-agent-manager, tech-transport, feat-stats, feat-chat, inbox-harness-quota, inbox-message-queue-steering]
---

# Problem — Codex converses, but tells Ubiq nothing about tokens, context, delegates or limits

The ask: bring Codex to Claude Code's level inside agent-manager — telemetry (tokens, context,
subagents), the five-hour and weekly limits, and several accounts. The integration path is the one
Codex's own IDE extension uses, `codex app-server`: newline-delimited JSON-RPC 2.0 over stdio.

**Every method, notification and field named below was read from the protocol codex-cli 0.161.0
generates for itself** (`codex app-server generate-json-schema --out` / `generate-ts --out`), or
from the `openai/codex` source at `codex-rs/app-server`, or observed on a live app-server. None is
from memory. Regenerating the schema is the first step of every task below, because the protocol
is marked experimental and moves.

## 1. What exists today

| Piece | Where | State |
|---|---|---|
| Native app-server bridge | `crates/agent-manager/src/io/codex.rs` (`CodexBridge`) | Landed: `initialize` → `initialized` → `thread/start`, `turn/start`, `turn/interrupt`, id-correlated `request`, reader thread |
| ACP variant | `harness/codex.rs` `Codex::new_acp()` → `codex-acp` | Landed, a sibling; not the subject here |
| Launch, `CODEX_HOME`, accounts | `harness/codex.rs::launch`, `config_anchor` (`Relocate::All`), `shares_home`, `login_home` (`codex login`), `login_files` (`auth.json`) | Landed — multi-account already works through `harness-homes/<account>/codex` (`D194`) |
| Quota | `io_support().quota = QuotaSource::None`; `Harness::quota` default error | Not wired (`G246`) |
| Accounting | `map_turn_completed` drops all usage; `SessionStarted` sends `model: None` | Nothing (`G96`, `G161`) |
| Approvals | reader auto-accepts every server request | No human in the loop (`G92`) |
| Resume | `spec.resume` is a documented no-op | Not wired |

Claude's reference shape, which Codex must meet without a second path anywhere above the library:
`AgentEvent::UsageUpdate { used, size, spend, origin, model }` under the five accounting rules in
`io/model.rs`; `AgentEvent::RateLimitUpdate { five_hour, seven_day, status, … }`, which
`ubiq-host/src/conversation.rs` already files as a per-account `QuotaChanged`; `Origin` stamping a
delegate's lines; `Harness::quota` + `QuotaSource::Probe` for an account with nothing running
(`quota::claude`). Every one of those is a slot Codex can fill — **no `ConvUpdate`, no `Message`
and no interface change is required**; `map_event` already carries them all.

## 2. The bridge is wrong against the real wire

Read against the generated schema, the landed bridge has defects that must be fixed before any
telemetry is worth adding:

1. **`ThreadItem` is tagged `type`, not `itemType`.** `map_item` reads `item.itemType`, so every v2
   `item/started` / `item/completed` maps to nothing; only the legacy `codex/event` dialect draws.
2. **`commandExecution` output is `aggregatedOutput`** (plus `exitCode`, `status`:
   `inProgress|completed|failed|declined`, `durationMs`); the bridge reads `output`/`result` and
   always reports `Completed`. `fileChange.changes` is `[{path, kind, diff}]`, so a real diff is
   available for `ToolContent::Diff`.
3. **Subagent threads stream on the same connection.** `app-server/src/lib.rs` (~1289–1304)
   attaches a listener for every newly created thread to every initialized connection, so a
   spawned agent's `turn/completed`, `item/*` and `thread/tokenUsage/updated` arrive tagged with
   *its* `threadId`. The bridge filters nothing, so **a delegate finishing ends the parent's turn**.
4. **`turn/completed` carries `turn.status` (`completed|interrupted|failed|inProgress`) and
   `turn.error { message, codexErrorInfo }`**; the bridge always says `EndTurn`.
   `codexErrorInfo` names `usageLimitExceeded`, `rateLimitExceeded`, `contextWindowExceeded` —
   the limit-hit signal.
5. **`thread/start`'s response states `model`, `reasoningEffort`, `approvalPolicy`, `sandbox`**;
   `SessionStarted` sends `None` for all of them.
6. Responses arrive **without a `"jsonrpc"` field** (observed live); the reader already routes on
   `id`/`method`, which is correct — tests must not assume it.

## 3. Protocol summary (0.161.0)

| Direction | Names used by this proposal |
|---|---|
| Client → server requests | `initialize`, `thread/start`, `thread/resume`, `thread/fork`, `turn/start`, `turn/steer`, `turn/interrupt`, `thread/compact/start`, `model/list`, `account/read`, `account/login/start`, `account/login/cancel`, `account/logout`, `account/rateLimits/read`, `account/usage/read` |
| Client notification | `initialized` |
| Server notifications | `thread/started`, `thread/status/changed`, `thread/tokenUsage/updated`, `turn/started`, `turn/completed`, `turn/plan/updated`, `turn/diff/updated`, `item/started`, `item/completed`, `item/agentMessage/delta`, `item/reasoning/summaryTextDelta`, `item/reasoning/textDelta`, `item/commandExecution/outputDelta`, `item/mcpToolCall/progress`, `account/rateLimits/updated`, `account/updated`, `account/login/completed`, `thread/compacted` (deprecated for the `contextCompaction` item), `model/rerouted`, `error`, `warning` |
| Server → client requests | `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval`, `item/tool/requestUserInput`, `mcpServer/elicitation/request`, `item/tool/call`, `account/chatgptAuthTokens/refresh`, legacy `execCommandApproval`, `applyPatchApproval` |

Key payloads:

- `ThreadTokenUsageUpdatedNotification { threadId, turnId, tokenUsage: { total, last, modelContextWindow } }`,
  each breakdown `{ totalTokens, inputTokens, cachedInputTokens, cacheWriteInputTokens, outputTokens, reasoningOutputTokens }`.
  Emitted from core's `TokenCount` event (`bespoke_event_handling.rs::handle_token_count_event`),
  which **also** emits `account/rateLimits/updated` when the same event carries rate limits.
- `RateLimitSnapshot { limitId, limitName, primary, secondary, credits, planType, rateLimitReachedType, spendControlReached, … }`,
  window `{ usedPercent, windowDurationMins, resetsAt }`. `account/rateLimits/read` returns
  `{ rateLimits, rateLimitsByLimitId, ordinaryUsageAllowed, accountId, … }`; the push is sparse and
  is to be *merged* into the last read.
- `GetAccountResponse { account: {type:"apiKey"} | {type:"chatgpt", email, planType} | {type:"amazonBedrock",…} | null, requiresOpenaiAuth }`.
- `LoginAccountParams` `type`: `apiKey`, `chatgpt` (→ `authUrl`), `chatgptDeviceCode`
  (→ `verificationUrl`, `userCode`), `chatgptAuthTokens`, `amazonBedrock*`; completion is
  `account/login/completed { loginId, success, error }`.
- `ThreadItem` `type`s: `userMessage`, `agentMessage`, `reasoning`, `plan`, `commandExecution`,
  `fileChange`, `mcpToolCall`, `dynamicToolCall`, `webSearch`, `imageView`, `imageGeneration`,
  `collabAgentToolCall`, `subAgentActivity`, `contextCompaction`, `enteredReviewMode`,
  `exitedReviewMode`, `hookPrompt`, `functionCallOutput`, `sleep`.
- `collabAgentToolCall { id, tool, status, senderThreadId, receiverThreadIds, prompt, model, reasoningEffort, agentsStates }`,
  `tool` ∈ `spawnAgent|sendInput|resumeAgent|wait|closeAgent|sendMessage|followupTask|interruptAgent|listAgents`.
  `Thread.parentThreadId` is set on a subagent's thread; `subAgentActivity { agentThreadId, agentPath, kind: started|interacted|interrupted|completed }`.

Observed live with an empty `CODEX_HOME`: `initialize` answers `{ userAgent, codexHome, platformFamily, platformOs }`
in well under a second; `account/read` → `{ account: null, requiresOpenaiAuth: true }`;
`account/rateLimits/read` → error `-32600 "codex account authentication required to read rate limits"`.
No model call is made by any of the three — an account probe is cheap.

## 4. Mapping — Codex → `AgentEvent` (→ `ConvUpdate` by the existing `map_event`)

All rows live in `io/codex.rs`. *Root* = the thread `thread/start` returned; *child* = any thread
whose `thread/started` carried `parentThreadId` (transitively) pointing at root.

| Codex | `AgentEvent` | Notes |
|---|---|---|
| `thread/start` response | `SessionStarted { session_id, model, mode }` | `model`, `reasoningEffort`, `approvalPolicy` from the response |
| `item/agentMessage/delta` (root) | `AgentMessageChunk { message_id: itemId }` | Streaming; `item/completed agentMessage` then only reconciles |
| `item/reasoning/summaryTextDelta`, `…/textDelta` | `AgentThoughtChunk` | |
| `turn/plan/updated` | `Plan` | `plan[]` steps + status |
| `item/started commandExecution` | `ToolCall { kind: Execute, title: command }` | `cwd`, `commandActions` → kind refinement (Read/Search) |
| `item/commandExecution/outputDelta` | `ToolCallUpdate` (content append) | |
| `item/completed commandExecution` | `ToolCallUpdate { status }` | `status`/`exitCode` → `Completed`/`Failed`; `aggregatedOutput` |
| `fileChange` | `ToolCall { kind: Edit, locations }` + `ToolContent::Diff` | `changes[].path/diff` |
| `mcpToolCall` / `dynamicToolCall` / `webSearch` | `ToolCall` (`Other` / `Fetch`) | `result`/`error` |
| `contextCompaction` item | `Compacted` | |
| `collabAgentToolCall` `spawnAgent` | `ToolCall { kind: Delegate, id }`, remember `receiverThreadIds → id` | `model`/`reasoningEffort` → `Origin.model`/`thinking` |
| any child-thread item / delta | same event, `origin: Origin { parent_tool_use_id: <spawn call id>, subagent_type: agentPath/role, model }` | Rule 4: never moves occupancy |
| child `turn/completed` / `subAgentActivity completed` | `ToolCallUpdate { id: spawn call, status }` | **never** `TurnEnded` |
| root `thread/tokenUsage/updated` | `UsageUpdate { used, size: modelContextWindow, spend, model }` | see §5 |
| child `thread/tokenUsage/updated` | `UsageUpdate { used: unchanged, spend, origin }` | delegate spend split |
| `account/rateLimits/updated` | `RateLimitUpdate { five_hour, seven_day, status }` | see §6; host already files it per account |
| root `turn/completed` | `TurnEnded` | `completed→EndTurn`, `interrupted→Cancelled`, `failed→Failed` (+ `codexErrorInfo`; `contextWindowExceeded→MaxTokens`) |
| server approval requests | `PermissionRequest` (parked) | §8 phase 5 |
| `model/rerouted` | `Log` (+ `SessionInfoUpdate` model) | |
| `error { willRetry:false }`, `warning` | `TurnEnded{Failed}` / `Log` | unchanged semantics |

## 5. Telemetry — the accounting contract, filled

- **Occupancy (a level):** `used = last.totalTokens` of the root thread, `size = modelContextWindow`.
  `modelContextWindow` is stated on every report, so `G96` closes for Codex with no constant. The
  model catalogue agrees (`codex debug models --bundled`: `context_window` 272 000 on current
  models), but the live figure wins. *Verify on a capture* that `last.totalTokens` is what Codex's
  own TUI calls context-in-use; if not, `last.inputTokens + last.outputTokens`.
- **Spend (a flow):** `total` is thread-cumulative, so each report's spend is
  `total − previous total` **per thread** (the same `Spend::saturating_sub` Claude's split uses).
  OpenAI counts cached input *inside* `inputTokens` and reasoning *inside* `outputTokens`, so
  `Spend { input: input − cached, cache_read: cached, cache_creation: cacheWrite, output: output − reasoning, thinking: reasoning }`.
  *Verify the inclusion on a capture* before landing — the schema names the fields, not their overlap.
- **Delegates:** child-thread reports carry `Origin` and spend only; usage.db then records Codex
  rows (`G161`) with no host change.
- **Cost:** none — Codex states no money for ChatGPT plans. `cost: None`.

## 6. Limits — five-hour and weekly

Two routes, both already provided for by the quota design (`inbox-harness-quota`):

1. **Push, while a turn runs.** `account/rateLimits/updated` → `RateLimitUpdate`. Windows are
   assigned **by `windowDurationMins`, not by position**: `300 → five_hour`, `10080 → seven_day`;
   any other duration is not forced into either slot (it reaches the gauges through route 2).
   `utilization_pct = round(usedPercent)`, `resets_at = resetsAt`. `status` is `"allowed"` unless
   `rateLimitReachedType` is set (its value verbatim). Merge sparse pushes over the last value
   per account, as the schema instructs.
2. **Probe, with nothing running.** `quota::codex(account, harness, home)`: spawn
   `codex app-server --listen stdio://` with `CODEX_HOME = home`, `initialize`/`initialized`,
   `account/read`, `account/rateLimits/read`, close stdin. Produces one `QuotaGauge` per window
   of every snapshot in `rateLimitsByLimitId` (label from `windowDurationMins` — "5 hours", "Week" —
   prefixed by `limitName` when there is more than one limit id), a `Credit` gauge when
   `credits.hasCredits && !unlimited` and `balance` parses, `plan` from `planType`. No model call,
   sub-second, so `io_support().quota` becomes **`QuotaSource::Probe`** (not `Bridge`); the host's
   spaced-out worker, cache and panel apply unchanged. An `apiKey` account answers the auth error
   above → the probe returns the error sentence and the panel draws an em dash.

A turn that hits the wall additionally ends with `codexErrorInfo: usageLimitExceeded` —
`TurnEnded { Failed, error }` carries it, and the push in route 1 normally arrives first.

## 7. Multi-account

Already structurally done: an account's Codex home is `harness-homes/<account>/codex`, reached
through `CODEX_HOME` (`Relocate::All`), signed in by `login_home`. Three improvements:

- **`G379` corrected.** Codex's keyring entry *is* keyed by home: service `Codex Auth`, account key
  computed from the canonicalised `codex_home` (`codex-rs/login/src/auth/storage.rs`,
  `KEYRING_SERVICE` / `compute_store_key`). Two homes do not share one login even with
  `cli_auth_credentials_store = "keyring"`. A macOS run still confirms it.
- **Identity.** `account/read` returns `email` and `planType` — the probe in §6 carries them, so
  an account row can say *who* it is signed in as and on which plan, and Ubiq can warn when two
  accounts resolve to the same email.
- **Login without a terminal (optional).** `account/login/start { type: "chatgptDeviceCode" }`
  returns `verificationUrl` + `userCode` and completes with `account/login/completed`; this lets
  `BeginHarnessLogin` sign a Codex home in from a dialog rather than a `codex login` pane. The
  pane path keeps working; this is an alternative, not a replacement.

## 8. Where it lands

| Crate | Change |
|---|---|
| `crates/agent-manager/src/io/codex.rs` | All of §2, §4, §5, route 1 of §6; thread registry (root / child → spawn id); parked approvals; `thread/resume`; `turn/steer` |
| `crates/agent-manager/src/harness/codex.rs` | `quota: Probe`, `fn quota`, resume plumbing into the bridge, optional device-code login |
| `crates/agent-manager/src/quota.rs` | `pub fn codex(...)` beside `claude(...)` |
| `crates/agent-manager/_docs/harness/codex.md` | Rewritten "Output stream protocol" from the generated schema; fixtures under `test-runs/` |
| `crates/ubiq-host` | **Nothing** expected — `map_event`, the `RateLimitUpdate → QuotaChanged` filing and the quota worker are generic. Only `accept_all` must keep working once Codex really asks |
| `crates/ubiq-proto`, `crates/ubiq` | Nothing. The ring, the spend meter, the delegate rows and the quota panel light up from data |

There is no "ubiq-host io/ module": the one `AgentEvent → ConvUpdate` translation stays in
`ubiq-host/src/conversation.rs::map_event`, and Codex-specific parsing stays in the library
(rule 4 of the agents skill).

## 9. Gaps and risks

- **Experimental protocol.** `generate-*` and most of v2 are marked experimental and change per
  release (0.161.0 has 104 client methods). Mitigation: a captured-frames fixture per release and a
  `just` recipe that regenerates the schema into `/tmp` and diffs the method lists.
- **Child-thread routing assumption** rests on reading `lib.rs`, not on a capture; Phase 0 captures
  a multi-agent turn to confirm. Multi-agent v2 children (`is_v2_child`) may behave differently.
- **Token-field overlap** (cached ⊂ input, reasoning ⊂ output) is OpenAI convention, not stated by
  the schema; double counting is the failure mode. Capture first.
- **Approvals change behaviour.** Today Codex never asks; parking makes columns block where they
  did not. The default must stay what `Profile.mode` / `unattended_mode` says, and `accept_all`
  must cover Codex too.
- **`G24`** (`codex_bridge_round_trips_events_and_terminates` timing out under the workspace run)
  will get worse with more traffic; fix it in Phase 0.
- **Analytics.** The app-server leaves analytics off unless `--analytics-default-enabled`; keep it
  off. `remoteControl/status/changed` arrives unasked — ignore.
- **apiKey accounts** have no plan windows; they read as "not said", never zero.

## 10. Phases and tasks

Sizes: S ≈ half a day, M ≈ 1–2 days, L ≈ 3+ days. One area per task.

| # | Phase | Task | Area | Size |
|---|---|---|---|---|
| 0.1 | Correct the wire | Capture real frames (single turn, tool calls, file edit, multi-agent spawn, limit push) into `crates/agent-manager/_docs/test-runs/`; add the schema-diff recipe | agent-manager docs/tests | M |
| 0.2 | | Fix `map_item` (`type`, `aggregatedOutput`, `status`/`exitCode`, `changes[].diff`), stream `item/agentMessage/delta` + reasoning deltas, `turn/plan/updated`, `mcpToolCall`/`webSearch`/`contextCompaction` | `io/codex.rs` | M |
| 0.3 | | Thread registry: only root `turn/completed` ends a turn; `turn.status`/`codexErrorInfo` → `StopReason`; `SessionStarted` from `thread/start` response | `io/codex.rs` | S |
| 0.4 | | Fix `G24` | `io/codex.rs` tests | S |
| 1.1 | Telemetry | `thread/tokenUsage/updated` → `UsageUpdate` (occupancy, `modelContextWindow`, per-thread spend delta); close `G96`/`G161` for Codex | `io/codex.rs` | M |
| 2.1 | Delegates | `collabAgentToolCall` → `Delegate` tool call; child threads → `Origin`; child completion → tool-call update; child spend split | `io/codex.rs` | M |
| 3.1 | Limits | `account/rateLimits/updated` → `RateLimitUpdate` by window duration, sparse merge | `io/codex.rs` | S |
| 3.2 | | `quota::codex` probe + `QuotaSource::Probe` + `Codex::quota` (both `codex` and `codex-acp` ids); close `G246` | `quota.rs`, `harness/codex.rs` | M |
| 4.1 | Accounts | Identity (`email`, `planType`) on the probe; correct `G379`'s keychain sentence after a macOS check | `quota.rs`, backlog | S |
| 4.2 | | *(optional)* Device-code login through `account/login/start` behind `BeginHarnessLogin` | `harness/codex.rs` + host login flow | L |
| 5.1 | Interaction parity | Park approval requests as `PermissionRequest`; `AnswerPermission` → `decision` (`accept`/`acceptForSession`/`decline`/`cancel`); `item/tool/requestUserInput` → ask; honour `unattended_mode`; close `G92` for Codex | `io/codex.rs` | L |
| 5.2 | | Resume via `thread/resume { threadId }` from `spec.resume`; `thread/fork` for fork | `io/codex.rs`, `harness/codex.rs` | M |
| 5.3 | | `turn/steer` for mid-turn input; `SetConfigOption` model/effort as `turn/start` overrides | `io/codex.rs` | M |
| 6.1 | Docs | Rewrite `_docs/harness/codex.md` protocol sections from the schema; update `tech/agent-manager.md`, `features/stats.md`, backlog rows; file `Dnn` for "Codex windows by duration, probe not bridge" | docs | S |

Phase 0 is a prerequisite for everything; 1, 2 and 3 are independent of each other after it; 5 is
independent of 1–4.

## 11. Decisions taken (`D209`)

1. **Approvals follow the profile's permission mode.** `danger-full-access` (the unattended mode)
   launches `approval_policy = "never"`; every other mode launches `on-request`, and the bridge
   parks each request as a permission prompt — it never answers one itself.
2. **`codex-acp` stays a sibling** (as `D95` keeps `claude-code-acp`); the native app-server bridge
   is Codex's default, and the ACP variant is offered only behind its settings switch.
3. **Only the `codex` limit is read** — the `5 hours` and `Week` gauges; other limit ids are ignored.
4. Device-code login is library-only for now (see §12).

## 12. Status — 2026-10-08

Landed in `crates/agent-manager` (no proto, host or interface change):

| Phase | What is in the tree |
|---|---|
| 0 | `io/codex.rs` `Mapper`: items by `type`, `aggregatedOutput`/`exitCode`/`status`, diffs, MCP/dynamic/web-search calls, streamed message and reasoning deltas, plan, `contextCompaction`; one end per turn from the root's `turn/completed` with its status and `codexErrorInfo`; `SessionStarted` with model and mode. Fixtures and `just codex-schema-diff` under `tests/fixtures/codex-app-server/`. `G24` closed (passes under `serde_json/preserve_order`) |
| 1 | `thread/tokenUsage/updated` → `UsageUpdate` (ring from `last.totalTokens` / `modelContextWindow`, spend as the change in `total`) |
| 2 | Subagent threads → one `Delegate` call per thread id, every line stamped with its `Origin`, its turns as the call's status, its spend split out under the parent's ring; `collabAgentToolCall` titles the delegate with its prompt and `agentsStates` moves its status |
| 3 | `account/rateLimits/updated` → `RateLimitUpdate` (windows by duration, sparse merge, `codex` only); `quota::codex` probe on a short-lived app-server, `QuotaSource::Probe` for `codex` and `codex-acp` |
| 4.1 | The probe reads `account/read` (`email`, `planType`); `planType` is the snapshot's `plan`. `G379` corrected: the keychain entry is keyed by the canonical `CODEX_HOME` |
| 5 | Parked approvals with per-method answers; `thread/resume` from `Provisioned::resume`; `ThreadOpen::Fork` (`thread/fork`); `turn/steer` for a prompt into a live turn; `model/list` pickers (`model`, `reasoning_effort`) and their picks riding on `turn/start` |
| 4.2 | `harness::begin_codex_device_login(home)` → `CodexDeviceLogin { verification_url, user_code }`, `.wait()` |

Token semantics are now read from Codex's own source (`codex-rs/protocol` `TokenUsage`,
`codex-rs/codex-api` Responses parser): cached and cache-written input are inside `inputTokens`,
reasoning inside `outputTokens`, `totalTokens = input + output`, context in use = `last.totalTokens`.
Still unconfirmed on an authenticated capture.

**Outside the library — landed (T-361):**

| Item | What is in the tree |
|---|---|
| Fork | `RunSpec::fork` → `Provisioned::fork`; the Codex bridge opens `ThreadOpen::Fork` for it. The host's fork (`revive_conversation`, `source != agent_id`) sets it, and clears it once the fork's harness names its own thread, so a later re-attach resumes the fork. Claude ignores it |
| Persistence | Already worked for Codex (`keeps_sessions` from the `CODEX_HOME` lever; the thread id recorded as `harness_session_id`; boot revival resumes it by `thread/resume`). The New agent dialog's *Persistent* tick is now live for any `keeps_sessions` harness and sends `SetConversationPersistent` after the start. No harness-session picker exists to list Codex threads in |
| Steering | `IoSupport::steer` (native Codex) → `AgentTypeInfo::steers`; the composer sends mid-turn (*Steer*) instead of queueing |
| Identity | `QuotaSnapshot::email` (library and wire) from `account/read`; drawn in the settings quota footer; the host cache keeps it across pushed readings (`G415` for the duplicate warning) |
| Device-code login | `Harness::device_login` / `begin_device_login`; `AgentTypeInfo::device_login`; `BeginDeviceLogin` → `HarnessDeviceCode` → `HarnessHomeSignedIn` / `HarnessLoginFailed`; *Sign in with a code* beside *Sign in* in the login modal (`G414`) |

**What needed a change outside the library** (as filed before T-361):

1. **Fork (host).** `Agents::fork_run` copies the run directory and resumes the source's id; for a
   Codex run on a shared account home that now resumes the *same* thread, so both conversations
   write to one. The host needs to ask for a fork rather than a resume — a `RunSpec`/`Provisioned`
   flag the Codex bridge reads as `ThreadOpen::Fork`.
2. **Steering (interface + proto).** The composer only sends while idle; `turn/steer` needs the
   interface to send a prompt mid-turn for a harness that says it can — an `AgentTypeInfo` flag.
3. **Identity (proto + host + interface).** `email` has no field to travel in: `AccountInfo` or the
   quota snapshot would carry it, and the account row would draw it (and warn on two accounts with
   one email).
4. **Device-code login (host + proto + interface).** `BeginHarnessLogin` for Codex would call
   `begin_codex_device_login`, send the URL and code to the window, and wait on another thread.
