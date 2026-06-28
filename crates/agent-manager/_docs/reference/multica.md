# Reference: multica — how it orchestrates agent harnesses

> **Scope.** This is a language-agnostic architectural read of the **multica**
> project, vendored as a submodule at
> [`../../refs/multica/`](../../refs/multica/). It exists to capture *how a
> real, shipping system* hosts and drives many interactive agent harnesses
> (Claude Code, Codex, Copilot, Cursor, opencode, and others) so that
> `agent-manager` can borrow the proven techniques. The runtime techniques
> themselves are folded, vendor-neutral, into the per-harness docs under
> [`../harness/`](../harness/) — this file is the system-level companion that
> explains the moving parts and where they live in the source.
>
> **Citations.** All `path:symbol` references are relative to
> `crates/agent-manager/refs/multica/`. multica's daemon/server is written in
> Go under `server/`; this doc deliberately describes *behaviour and
> contracts*, not Go specifics, so the patterns transfer to a Rust
> implementation. Pinned to the submodule commit recorded by the parent repo;
> line numbers drift, symbol names are the stable anchors.

## 1. What multica is

multica is a **task-oriented agent orchestrator**. A user (or an automation
such as an issue assignment, a comment mention, or an "autopilot" trigger)
creates a *task*; multica routes that task to a machine running a *daemon*, the
daemon spins up an isolated working directory, launches the appropriate agent
harness as a child process, streams the harness's output back as structured
events, and records the result. The agent itself talks back to the platform
through a thin CLI (`multica …`) using a task-scoped credential.

Three design choices dominate the architecture and are the most relevant to
`agent-manager`:

1. **A single harness-agnostic adapter contract** (`server/pkg/agent`) so every
   harness — regardless of its wire protocol — presents the same typed event
   stream to the rest of the system.
2. **A split between a stateless backend and a fleet of daemons**, connected by
   an HTTP control plane plus a best-effort WebSocket wakeup channel, designed
   so daemons can live on remote/distributed hosts.
3. **Per-run, on-disk isolation**: each task gets a fresh working directory and
   provider-scoped config (model, MCP, skills, always-on context) materialised
   just before launch and garbage-collected after.

## 2. Topology

```
┌──────────────────────────────────────────────────────────────────────┐
│                          BACKEND SERVER (Go)                          │
│  REST API (Chi)        realtime hub            daemonws hub           │
│  /api/*                (web UI WS)             /api/daemon/ws          │
│       │                  scope: task/ws         scope: daemon_runtime  │
│       └──────────── PostgreSQL (sqlc) ───── Redis relay (multi-node) ──│
└───▲───────────────────────────────────────────────▲──────────────────┘
    │ HTTP REST (Bearer PAT)                          │ WebSocket (JSON frames)
    │ register / heartbeat / claim / start /          │ wss://…/api/daemon/ws
    │ progress / messages / complete / fail           │ ?runtime_ids=<csv>
    │                                                  │ wakeup + heartbeat
┌───┴──────────────────────────────────────────────────────────────────┐
│                       DAEMON PROCESS (multica CLI)                    │
│  workspaceSyncLoop   heartbeatLoop   taskWakeupLoop (WS client)       │
│  (30s)               (15s)           → per-runtime pollLoop (3s)      │
│                          │                                            │
│                  execenv.Prepare()  →  isolated workdir + config      │
│                          │ spawn child process                       │
│             ┌────────────▼─────────────────────────────┐             │
│             │   AGENT HARNESS (child: claude/codex/…)   │             │
│             │   CWD=workdir   ENV=MULTICA_TOKEN(task)   │             │
│             └────────────┬─────────────────────────────┘             │
│        stdout events ────┘     agent → backend via `multica` CLI     │
└──────────────────────────────────────────────────────────────────────┘
                                       │ REST (task-scoped token)
                                       ▼
                         ┌────────────────────────────┐
                         │   WEB / DESKTOP UI          │
                         │   WS client, scope: task    │
                         │   (Next.js / Electron)      │
                         └────────────────────────────┘
```

**Four roles** (`server/cmd/server/router.go` wires the server; the daemon is
`server/internal/daemon/daemon.go:Daemon`):

| Role | Where | Responsibility |
|------|-------|----------------|
| **Backend server** | `server/cmd/server`, `server/internal/handler` | REST control plane, two WebSocket hubs, Postgres persistence, Redis fan-out. Stateless across nodes. |
| **Daemon / runner** | `server/internal/daemon` | Runs on the user's (or a cloud) host. Registers its detected harnesses as *runtimes*, polls for tasks, prepares the sandbox, spawns the agent, streams results back. |
| **Agent harness** | child process | One of `claude`, `codex`, `copilot`, `opencode`, `cursor-agent`, `kiro-cli`, `qodercli`, … Talks to the backend only through the `multica` CLI with a task-scoped token. |
| **Web / desktop UI** | `apps/web`, `apps/desktop` | Subscribes to a *separate* WebSocket scope (`task` / `workspace`). Never touches a daemon or agent directly. |

The UI and the daemon are deliberately on different channels: the daemon channel
is a control plane; the UI channel is a read-mostly event feed.

## 3. Daemon ↔ backend communication

Two channels run in parallel; the HTTP plane is authoritative and the WebSocket
plane is an optimisation.

### 3.1 HTTP control plane (always on)

Every call carries `Authorization: Bearer <PAT>` plus identity headers
`X-Client-Platform: daemon`, `X-Client-Version`, `X-Client-OS`, and a
capability advertisement `X-Client-Capabilities: skill-bundles-v1`
(`server/internal/daemon/client.go:setIdentityHeaders`;
capability constants in `server/pkg/protocol/messages.go`, e.g.
`DaemonCapabilitySkillBundlesV1`).

| Concern | Endpoint / call | Notes |
|---------|-----------------|-------|
| Register | `POST /api/daemon/register` (`client.go:Register`) | Sends daemon id, device name, CLI version, and detected runtimes (type/version/status). Response returns **stable runtime UUIDs**, workspace repos, settings. |
| Heartbeat | `POST /api/daemon/heartbeat` (`client.go:SendHeartbeat`) | Body `{runtime_id, supports_batch_import}`. ~15 s cadence. Response piggybacks pending actions (see §3.3). |
| Claim | `POST /api/daemon/runtimes/:id/tasks/claim` | Atomically transitions the oldest `queued` task for that runtime → `dispatched`. |
| Lifecycle | `POST /api/daemon/tasks/:id/{start,progress,messages,session,usage,complete,fail}` | Streams progress and batched agent messages; terminal calls (`complete`/`fail`) use bounded retry (`client.go:postJSONWithRetry`). |
| Cancellation poll | `GET /api/daemon/tasks/:id/status` | Daemon polls every ~5 s for a server-side cancel signal. |
| Orphan recovery | `POST /api/daemon/runtimes/:id/recover-orphans` | Fails tasks left behind by a crashed daemon. |
| Deregister | `POST /api/daemon/deregister` | Marks runtimes offline on clean shutdown. |
| Token renewal | `POST /api/tokens/current/renew` (`client.go:RenewToken`, `daemon.go:tokenRenewalLoop`) | Auto-rotates the PAT (~every 3 days). |

**Registration is identity-by-runtime, not by daemon.** A *runtime* is
`(workspace, agent provider, device)`; the server assigns it a UUID at register
time and that UUID is how tasks are addressed. If a heartbeat returns HTTP 404
`runtime not found`, the daemon treats the runtime as gone
(`client.go:isRuntimeNotFoundError`) and re-registers with fresh UUIDs
(`daemon.go:handleRuntimeGone`).

### 3.2 WebSocket wakeup channel (best effort)

`wss://<server>/api/daemon/ws?runtime_ids=<csv>` (URL built in
`server/internal/daemon/wakeup.go:taskWakeupURL`; connection loop
`runTaskWakeupConnection`, reader `readTaskWakeupMessages`). Same Bearer header
on the HTTP upgrade. On disconnect it reconnects with exponential backoff
(1 s → 30 s, jittered) while HTTP polling continues as the fallback — the
WebSocket only ever *reduces latency to first claim*, it is never required for
correctness.

Heartbeats migrate onto this channel when it is healthy: the daemon sends
`daemon:heartbeat` frames per runtime, the server replies `daemon:heartbeat_ack`,
and the HTTP heartbeat for that runtime is suppressed while acks are recent
(`daemon.go:wsHeartbeatRecentlyAcked`); on WS loss `clearWSHeartbeatAcks`
resumes the HTTP heartbeat.

### 3.3 Message contract

All WebSocket frames are a JSON envelope `{"type": "<string>", "payload": <json>}`
(`server/pkg/protocol/messages.go:Message`).

**Downstream — server → daemon (WS):**

| `type` | Payload | Meaning |
|--------|---------|---------|
| `daemon:task_available` | `TaskAvailablePayload{RuntimeID, TaskID}` | "A task is queued for this runtime — go claim it." A hint, not the task itself. |
| `daemon:heartbeat_ack` | `DaemonHeartbeatAckPayload` | Heartbeat confirmation + piggybacked actions. |
| `daemon:runtime_profiles_changed` | `RuntimeProfilesChangedPayload` | "Refresh your runtime profile list." |

`DaemonHeartbeatAckPayload` is the catch-all action carrier — its optional slots
turn the heartbeat into a control bus: `RuntimeGone`, `PendingUpdate` (CLI
auto-update), `PendingModelList` (enumerate models, see §7), `PendingLocalSkills`
/ `PendingLocalSkillImport(s)` (skill inventory/import), and `FeatureFlags` (a
server-evaluated flag snapshot).

**Upstream — daemon → server:**

- Over WS: `daemon:heartbeat` (`DaemonHeartbeatRequestPayload{RuntimeID, SupportsBatchImport}`).
- Over HTTP: everything in the §3.1 table, plus result reports for the
  piggybacked requests (`POST /api/daemon/runtimes/:id/models/:req-id/result`,
  `…/local-skills/:req-id/result`, `…/update/:update-id/result`), each with
  retry/backoff.

**Backend → web UI (the other hub):** task lifecycle events
`task:{queued,dispatch,running,progress,completed,failed,cancelled,message}`
plus issue/comment/agent/chat/autopilot events
(`server/pkg/protocol/events.go`), fanned out through the realtime hub
(`server/internal/realtime/broadcaster.go:ScopeTask` / `ScopeDaemonRuntime`) and,
in multi-node deployments, relayed over Redis streams
(`server/internal/realtime/sharded_stream_relay.go`).

## 4. Task delegation — from creation to agent output

The full path, end to end:

1. **Enqueue.** A user action inserts a task row with status `queued`, bound to
   a specific `runtime_id` (the agent's assigned runtime in that workspace). The
   server emits `task:queued` to the UI.
2. **Wakeup hint (fast path).** The backend calls
   `NotifyTaskAvailable(runtimeID, taskID)`
   (`server/internal/daemonws/notifier.go`), which pushes a
   `daemon:task_available` frame to the hub's per-runtime index — and, with
   Redis configured, publishes it to a relay shard so any API node can deliver
   it locally. The daemon enqueues a wakeup on the per-runtime channel.
3. **Poll / claim.** The daemon runs one poller goroutine per runtime
   (`daemon.go:pollLoop`), woken by the wakeup channel *or* a ~3 s timer. It
   first acquires a slot from a concurrency semaphore
   (`newTaskSlotSemaphore`, default 20 concurrent tasks) held for the whole run,
   then `POST …/tasks/claim`. **There is no central scheduler picking daemons —
   each daemon self-selects by polling its own runtime UUID.** Dispatch is by
   `runtime_id` matching, set when the task was enqueued.
4. **Prepare the sandbox.** On a non-nil claim, `handleTask` → `runTask` calls
   `execenv.Prepare()` (`server/internal/daemon/execenv/execenv.go`), creating
   `~/multica_workspaces/<workspace_id>/<short_task_id>/workdir/` plus `output/`
   and `logs/`. The workdir starts **empty** — repos are checked out on demand
   by the agent via `multica repo checkout <url>`, backed by a shared bare-clone
   cache at `<workspacesRoot>/.repos/`.
5. **Materialise per-run config.** Skills, the always-on context brief, and
   provider-specific config (Codex `CODEX_HOME`, Cursor MCP + data dir, OpenClaw
   config) are written into the workdir/env root (see §8).
6. **Spawn the agent.** The daemon `exec`s the harness with `CWD=workdir` and a
   task-scoped environment: `MULTICA_TOKEN` (a `mat_…` credential scoped to this
   task), `MULTICA_TASK_ID`, `MULTICA_TASK_SLOT`, `MULTICA_WORKSPACE_ID`. The
   agent uses the same `multica` CLI subcommands (`multica issue get`,
   `multica issue comment add`, …) to read context and post results.
7. **Stream output.** The daemon reads the harness's stdout line by line,
   normalises it to the common event model (§6), batches the events, and
   `POST …/tasks/:id/messages`. The server stores them and broadcasts
   `task:message` to the UI.
8. **Complete.** On process exit the daemon calls `CompleteTask` or `FailTask`
   (with exponential retry); the server transitions the task and notifies the UI.

A 5 s cancellation-poll goroutine runs alongside the agent
(`GET …/tasks/:id/status`); a server-side cancel tears the run down through the
normal cancellation path (§6/§5).

## 5. Where agents run (execution environments)

### Local execenv

`execenv.Prepare` / `Reuse` (`server/internal/daemon/execenv/execenv.go`). Each
task gets an isolated tree; isolation is layered on with provider-specific
redirections so a task never mutates the user's real config:

- **Codex** — a per-task `codex-home/` synthesises a `~/.codex/` (skills,
  settings); `CODEX_HOME` redirects the CLI (`execenv/codex_home.go`,
  `execenv.go:hydrateCodexSkills`).
- **Cursor** — managed MCP at `<workDir>/.cursor/mcp.json` plus a per-task
  `CURSOR_DATA_DIR` carrying pre-seeded `mcp-approvals.json` (approval keys
  computed as `sha256(path+server)[:16]`) and a `.workspace-trusted` marker, so
  interactive trust/approval prompts are bypassed without touching global state
  (`execenv/cursor_mcp.go:prepareCursorMcpConfig`).
- **OpenClaw** — a synthesised per-task `openclaw.json` pins `workspaceDir`;
  `OPENCLAW_CONFIG_PATH` / `OPENCLAW_INCLUDE_ROOTS` point at it
  (`execenv/openclaw_config.go`).
- **local_directory resources** — when a project pins an existing local
  directory, the agent's CWD is that directory (not a fresh workdir); a
  `LocalPathLocker` enforces one task at a time per path.

**Garbage collection** (`daemon.go:gcLoop`, `execenv.go:GCMeta`,
`server/internal/daemon/gc.go`): a periodic loop reads `.gc_meta.json` in each
task dir, checks the parent issue/chat/run status, and reclaims directories by
three policies — full removal (done/cancelled + TTL), orphan removal (missing
`.gc_meta.json`), and artifact-only cleanup (`node_modules`, etc.).

### Cloud runtime

`server/internal/cloudruntime/client.go` is a thin HTTP proxy to a separate
"cloud runtime fleet" service (`provision` / `terminate` / `status` / `gateway`
/ `billing`, with `X-User-ID` / `X-Request-ID` passthrough). This path is
entirely server-side — the local daemon is not involved — and is the seam for
running harnesses on managed remote hosts.

## 6. The agent-adapter layer (the harness-agnostic contract)

Every harness adapter implements one interface,
`server/pkg/agent/agent.go:Backend`:

```
Execute(ctx, prompt, opts ExecOptions) (*Session, error)
```

- **`ExecOptions`** carries every per-run knob: `Cwd`, `Model`, `SystemPrompt`,
  `ThreadName`, `MaxTurns`, `Timeout`, `SemanticInactivityTimeout`,
  `ResumeSessionID`, `ExtraArgs`, `CustomArgs`, `McpConfig`, `ThinkingLevel`,
  `OpenclawMode`.
- **`Session`** exposes two channels: `Messages` (a stream of typed events) and
  `Result` (exactly one terminal value).
- **`Message.Type`** is one of seven canonical categories
  (`MessageText`, `MessageThinking`, `MessageToolUse`, `MessageToolResult`,
  `MessageStatus`, `MessageError`, `MessageLog`) with side fields `Tool`,
  `CallID`, `Input`, `Output`, `Status`, `SessionID`.
- **`Result`** = `Status` (`completed|failed|aborted|timeout|cancelled`),
  `Output`, `Error`, `DurationMs`, `SessionID`, `Usage` (a per-model
  `map[string]TokenUsage`).

The supported set is enumerated in `agent.go:SupportedTypes`: `claude`,
`codebuddy`, `codex`, `copilot`, `opencode`, `openclaw`, `hermes`, `pi`,
`cursor`, `kimi`, `kiro`, `antigravity` (`qoder` exists but is excluded from
custom-profile types). **Each adapter's whole job is to translate its harness's
wire protocol into that seven-type event stream**, so the daemon, the message
batcher, and the UI never see a vendor-specific frame.

Shared spawn mechanics (all adapters): pipes, never a PTY; a bounded
stderr ring-buffer (`stderr_tail.go:stderrTail`, 2 KiB) appended to
`Result.Error` on failure; a unified cancellation context
(`agent.go:runContext`) honouring `Timeout` and `SemanticInactivityTimeout`;
process groups on Unix for tree-kill (`proc_other.go:configureProcessGroup` /
`signalProcessGroup`).

## 7. Wire protocols to harnesses

multica deals with **four protocol families** behind the single adapter
contract. The vendor-neutral details (exact flags, event shapes) are in the
per-harness docs' *Orchestration / headless invocation* sections; here is the
map of which harness uses which, and the cross-cutting machinery (the full
per-adapter profile, including the nine harnesses `agent-manager` does not yet
document, is in §12).

| Family | Harnesses | Launch shape | Transport |
|--------|-----------|--------------|-----------|
| **NDJSON `stream-json`** | claude, codebuddy, copilot, opencode, cursor, pi, openclaw | print/run mode + output-format flag | one JSON object per stdout line; prompt on stdin (claude) or argv (others) |
| **ACP (Agent Client Protocol)** | hermes, kimi, kiro, qoder | `<bin> acp` (+ trust flags) | JSON-RPC 2.0 over stdio: `initialize` → `session/new` → `session/prompt`, `session/update` notifications |
| **Vendor app-server JSON-RPC** | codex | `codex app-server --listen stdio://` | JSON-RPC 2.0 over stdio: `initialize`/`initialized` → `thread/start` → `turn/start` |
| **Plain text + log scrape** | antigravity | `agy -p … --log-file <tmp>` | stdout lines as text; structured data scraped from a glog-format log |

**NDJSON stream-json** (e.g. `server/pkg/agent/claude.go`): events are
`{"type":"assistant"|"user"|"system"|"result"|"log"|"control_request", …}`.
Claude's tool-approval is an in-band stdin handshake — a `control_request` is
answered with a `control_response` (`claude.go:handleControlRequest`); multica
auto-allows and rewrites `run_in_background:true → false`
(`forceClaudeToolInputForeground`). Per-model token usage is read from the
`result` event's `modelUsage` map.

**ACP** (`server/pkg/agent/hermes.go`, shared by kimi/kiro/qoder via
`hermesClient`): MCP servers are passed *in* `session/new` as an array built by
`buildACPMcpServers` and filtered to the runtime's advertised transports by
`filterACPMcpServersByCapability` (so an stdio-only ACP runtime never receives an
HTTP MCP entry). Incoming `session/update` notifications are canonicalised by
`normalizeACPUpdate` (handling both snake_case and camelCase variants);
`session/request_permission` is auto-approved
(`handleAgentRequest`). A stderr sniffer (`acpProviderErrorSniffer`) promotes a
"success" turn to "failed" when a terminal provider error (rate limit, auth) is
seen on stderr. Per-harness deviations are real: Kiro resumes via `session/load`
and signals completion with a `goal_complete` sentinel tool; Qoder needs a
bounded drain because it holds pipes open after the response.

**Codex app-server** (`server/pkg/agent/codex.go:codexClient`): structurally like
ACP but with codex's own methods, and it **auto-detects the notification
dialect** — `legacy` (`codex/event` with `msg.type`) vs `raw`/v2
(`turn/started`, `item/*`, …) — on the first notification
(`handleNotification`), handling both. Server→client approval requests
(`item/commandExecution/requestApproval`, `applyPatchApproval`,
`mcpServer/elicitation/request`) are auto-accepted (`handleServerRequest`).

**Plain text** (`server/pkg/agent/antigravity.go`): every stdout line becomes
`MessageText`; the session id and a "print timed out" signal are scraped from
the `--log-file` (needed because the binary exits 0 even on print timeout).

**Capability & version gating** (`server/pkg/agent/version.go`): registration
enforces minimum versions (`claude ≥ 2.0.0`, `codex ≥ 0.100.0`,
`copilot ≥ 1.0.0`) via `CheckMinVersion` over `DetectVersion` output; git-describe
dev builds are exempt. OpenClaw is re-checked before *every* run
(`openclaw.go:checkOpenclawVersion`) because older builds wrote JSON to stderr.

### Model & reasoning selection (cross-cutting)

`server/pkg/agent/models.go:ListModels` is the single entry point; it discovers a
model catalog by one of three strategies — a **static list annotated at call
time** (claude, codex, codebuddy), a **dynamic CLI query** (`opencode models
--verbose`, `cursor-agent --list-models`, …), or an **ACP handshake** that reads
`session/new`'s `availableModels` — caching results with short TTLs
(`cachedDiscovery`). The chosen model reaches each harness through its protocol's
natural channel: a `--model` flag (claude/opencode/copilot/cursor/codebuddy), an
RPC field (`thread/start` for codex; `session/set_model` for ACP harnesses), a
`--provider`/`--model` split (pi), or an agent-id (`--agent`) for openclaw.
Reasoning effort is likewise per-protocol: `--effort` (claude/codebuddy),
`--variant` (opencode), `config.model_reasoning_effort`/`effort` (codex). The
*level vocabulary is never normalised* — `ExecOptions.ThinkingLevel` carries the
runtime-native token verbatim, validated cheaply server-side
(`thinking.go:IsKnownThinkingValue`, `providerThinkingEnums`) and per-model at the
daemon before launch (`thinking.go:ValidateThinkingLevel`, fail-open). When the
backend asks "what models do you have?" (the `PendingModelList` heartbeat slot),
the daemon answers with `daemon.go:handleModelList` →
`POST …/runtimes/:id/models/:req-id/result`, serialising the catalog including
the nested per-model thinking block.

## 8. Per-run injection (how a task is shaped before launch)

This is the part most directly reusable by `agent-manager`: how one coherent
"agent definition" is projected onto a harness at launch.

- **Skills as files.** Skills live as a bundle (`server/pkg/skillbundle/hash.go`:
  `Skill` + `BuildManifest`, a content-addressed `sha256:…` over name +
  description + every supporting file), cached on disk
  (`server/internal/daemon/skill_cache.go:SkillBundleCache`). Before launch,
  `execenv/context.go:writeContextFiles` → `writeSkillFiles` materialises each
  skill into the harness's *native discovery directory* under the workdir —
  `.claude/skills/`, `.github/skills/`, `.opencode/skills/`, `.agents/skills/`
  (codex), etc. — synthesising frontmatter if absent
  (`ensureSkillFrontmatter`). **No harness receives skills as slash commands or
  prompt injection** — they are always real files in the expected path.
- **Builtin skills** are embedded into the binary
  (`server/internal/service/builtin_skills.go`, `//go:embed builtin_skills`),
  prefixed to avoid collision with user skills, and appended to the per-task
  bundle list (`server/internal/service/task.go:LoadAgentSkillBundles`).
- **MCP, per harness.** There is no single MCP mechanism — each harness gets MCP
  through its own seam: a written temp file + `--mcp-config`/`--strict-mcp-config`
  (claude, `claude.go:writeMcpConfigToTemp`), an env var carrying inline JSON
  (`OPENCODE_CONFIG_CONTENT`, `opencode_mcp.go:buildOpenCodeMCPConfigContent`,
  which also *translates* Claude-style `mcpServers` to opencode's `mcp` shape),
  a managed block in `CODEX_HOME/config.toml` (`codex.go:ensureCodexMcpConfig`),
  a written `.cursor/mcp.json` (`execenv/cursor_mcp.go`), or the ACP
  `session/new` server array. A common idea recurs: **a managed, marker-delimited
  region** so hand-authored config survives, plus a **strict mode** that
  suppresses inherited servers.
- **Always-on context brief.** `execenv/runtime_config.go:InjectRuntimeConfig`
  writes a structured brief into the harness's memory file — `CLAUDE.md` for
  claude/codebuddy, `AGENTS.md` for everything else — assembled by
  `buildMetaSkillContent` (agent identity, requesting user, workspace/repo
  context, instruction precedence, workflow, skills index, output rules) and
  fenced with HTML-comment markers (`<!-- BEGIN MULTICA-RUNTIME -->` …) so
  user-authored content in the same file is preserved
  (`writeRuntimeConfigFile`). A supplementary `.agent_context/issue_context.md`
  carries task-assignment details (`context.go:renderIssueContext`).
- **Per-turn prompt.** `server/internal/daemon/prompt.go:BuildPrompt` assembles
  the actual prompt, branching by task type (chat / comment-triggered /
  autopilot / quick-create / standard). For stream-json claude it is sent as a
  `user` message on stdin; for opencode it is a positional argv.
- **Tool approval is the daemon's job, not a harness hook file.** multica does
  **not** write `settings.json` hooks. Instead it answers each harness's native
  approval channel inline (claude `control_response`, ACP
  `session/request_permission`, codex `requestApproval`) with a blanket allow,
  and launches with bypass flags (`--permission-mode bypassPermissions`,
  `--dangerously-skip-permissions`, `--yolo`, …). The only non-trivial mutation
  is forcing background tools to the foreground. This is the functional
  equivalent of a "PreToolUse hook with an allow policy", implemented at the
  protocol layer.

## 9. Multi-agent: delegation, handoff, resume

multica has **no agent-to-agent socket**. All collaboration flows through the
platform (issues, comments, the task queue), which keeps the topology uniform
and auditable. Three distinct mechanisms exist
(`server/internal/daemon/types.go:Task`, `prompt.go`):

1. **Handoff note (human → agent).** A free-text `Task.HandoffNote` is injected
   into the opening prompt as a scoping instruction the agent must honour before
   anything broader (`prompt.go:BuildPrompt`,
   `execenv.go:TaskContextForEnv.HandoffNote`). One-way, human to agent.
2. **Squad leader delegation (agent → agent, via the platform).** A *squad* is a
   named group of agents with a designated leader. When an issue is assigned to a
   squad, the backend dispatches to the leader; the daemon detects a
   `## Squad Operating Protocol` marker in the leader's instructions and sets
   `IsSquadLeader` in the exec env (`daemon.go` ~`:3459`,
   `execenv.go:TaskContextForEnv.IsSquadLeader`). The leader then **delegates by
   using the `multica` CLI** — reassigning the issue or sub-issues to squad
   members, or emitting `multica squad activity <issue> no_action`. Each
   reassignment enqueues a *new* task that the target agent's daemon claims in
   turn. `Task.SquadID` / `Task.SquadName` tell the agent it acts on behalf of
   the squad, not itself.
3. **Session resume (same agent, across runs).** `Task.PriorSessionID` /
   `Task.PriorWorkDir` let the daemon call `execenv.Reuse()` instead of
   `Prepare()` and pass the prior session id to the harness (e.g.
   `claude --resume <id>`, codex `thread/resume`, ACP `session/resume` /
   `session/load`). Intra-agent continuity, not delegation.

> Note: the `Co-authored-by` / "coauthor" feature
> (`daemon.go:workspaceCoAuthoredByEnabled`) is a Git commit-trailer integration,
> **not** agent-to-agent delegation — don't conflate the two.

## 10. What `agent-manager` should take from this

- **One adapter contract, many protocols.** A `Backend`-style trait that yields a
  small, fixed set of typed events (text / thinking / tool-use / tool-result /
  status / error / log + a terminal result with per-model usage) is what lets a
  multiplexer stay harness-agnostic. (Cross-reference [`../architecture.md`](../architecture.md).)
- **Project per-run config onto native locations.** Skills as real files in the
  harness's discovery dir; MCP through the harness's own seam (flag/env/managed
  block) with a strict mode and marker-delimited managed regions; always-on
  context into the harness's memory file behind preserve-the-user markers. This
  is exactly the projection `agent-manager sync` performs — multica confirms the
  per-harness target paths now documented in [`../harness/`](../harness/).
- **Headless invocation is a first-class contract, not an afterthought.** The
  print/run flags, the output-format flag, the auto-approve/bypass flags, and the
  in-band approval handshake together are *the* interface an orchestrator needs;
  they are now captured per harness under *Orchestration / headless invocation*.
- **Control plane vs. event plane.** An authoritative request/response control
  plane (here: HTTP) with a best-effort low-latency wakeup channel (here:
  WebSocket) is a clean split that survives the in-process → two-process →
  distributed evolution `agent-manager` anticipates.

## 11. Source map (quick index)

| Concern | Primary files |
|---------|---------------|
| Adapter contract & event model | `server/pkg/agent/agent.go` (`Backend`, `ExecOptions`, `Session`, `Message`, `Result`, `runContext`, `SupportedTypes`) |
| Per-harness adapters | `server/pkg/agent/{claude,codex,copilot,opencode,cursor,pi,hermes,kimi,kiro,qoder,antigravity,openclaw,codebuddy}.go` |
| Process/stderr/cancel helpers | `server/pkg/agent/{stderr_tail,proc_other,proc_windows,version}.go` |
| Model & reasoning | `server/pkg/agent/{models,thinking}.go` |
| Daemon core | `server/internal/daemon/daemon.go` (`Daemon`, `pollLoop`, `handleTask`, `runTask`, `heartbeatLoop`, `taskWakeupLoop`, `handleModelList`, `handleRuntimeGone`, `gcLoop`, `tokenRenewalLoop`) |
| Daemon HTTP client | `server/internal/daemon/client.go` (`Register`, `SendHeartbeat`, `CompleteTask`, `FailTask`, `RenewToken`, `setIdentityHeaders`) |
| WS wakeup client | `server/internal/daemon/wakeup.go` |
| Task type & prompt | `server/internal/daemon/{types.go,prompt.go,slash_skill.go}` |
| Exec environment | `server/internal/daemon/execenv/{execenv,context,runtime_config,cursor_mcp,openclaw_config,codex_home}.go` |
| Skill bundling | `server/pkg/skillbundle/hash.go`, `server/internal/skill/frontmatter.go`, `server/internal/service/{builtin_skills.go,task.go}`, `server/internal/service/builtin_skills/` |
| Wire protocol & events | `server/pkg/protocol/{messages.go,events.go}` |
| daemon↔backend WS hub | `server/internal/daemonws/` (`notifier.go`) |
| Realtime fan-out | `server/internal/realtime/{broadcaster,sharded_stream_relay}.go` |
| Cloud runtime | `server/internal/cloudruntime/client.go` |
| Server routing | `server/cmd/server/router.go`, `server/internal/handler/` |
| Narrative docs (upstream) | `CLI_AND_DAEMON.md`, `CLAUDE.md`, `SELF_HOSTING*.md` (repo root) |

## 12. Appendix: full harness adapter catalogue

multica ships adapters for **thirteen** harness types
(`server/pkg/agent/agent.go:SupportedTypes`,
`launchHeaders`). Four of them — `claude`, `codex`, `copilot`,
`opencode` — overlap with the harnesses `agent-manager` documents, and their
vendor-neutral runtime contracts live in
[`../harness/`](../harness/). The other nine are catalogued here because they
exercise the *same* injection patterns against different protocols, which is
useful when deciding how far `agent-manager`'s harness abstraction must stretch.

Each adapter is one file `server/pkg/agent/<type>.go`. Reasoning-effort support
exists only for `claude`, `codebuddy`, `codex`, `opencode`; all others ignore
`ExecOptions.ThinkingLevel`. Managed per-run MCP injection exists for `claude`,
`codebuddy`, `codex`, `opencode`, `cursor`, the four ACP harnesses (via the
`session/new` server array), and `openclaw`; `copilot`, `pi`, and `antigravity`
fall back to the harness's own config.

| Type | Launch (after which `custom_args` are appended) | Protocol | Model channel | Reasoning | MCP seam | Skills dir |
|------|--------------------------------------------------|----------|---------------|-----------|----------|------------|
| `claude` | `claude -p --output-format stream-json --input-format stream-json …` | NDJSON | `--model` | `--effort` | temp file `--mcp-config` (+`--strict-mcp-config`) | `.claude/skills/` |
| `codebuddy` | `codebuddy --output-format stream-json …` | NDJSON (Claude-shaped) | `--model` | `--effort` | `--mcp-config` | `.claude/skills/` |
| `codex` | `codex app-server --listen stdio://` | JSON-RPC app-server | RPC `thread/start.model` | `config.model_reasoning_effort` / `effort` | `CODEX_HOME/config.toml` managed block | `.agents/skills/` (in `CODEX_HOME`) |
| `copilot` | `copilot -p "…" --output-format json --allow-all --no-ask-user` | NDJSON | `--model` | — | own config files | `.github/skills/` |
| `cursor` | `cursor-agent -p "…" --output-format stream-json --yolo` | NDJSON (`stdout:`/`stderr:` prefixes stripped) | `--model` | — | `.cursor/mcp.json` + `CURSOR_DATA_DIR` pre-seeded approvals | `.cursor/skills/` |
| `opencode` | `opencode run --format json --dangerously-skip-permissions` | NDJSON | `--model <provider/id>` | `--variant` | `OPENCODE_CONFIG_CONTENT` env (inline JSON) | `.opencode/skills/` |
| `pi` | `pi -p --mode json --session <file.jsonl>` | NDJSON | `--provider` + `--model` (slug split) | — | — | `.pi/skills/` |
| `hermes` | `hermes acp` (env `HERMES_YOLO_MODE=1`) | ACP JSON-RPC | `session/new.model` + `session/set_model` | — | ACP `session/new` server array (capability-filtered) | `.agent_context/skills/` |
| `kimi` | `kimi acp` | ACP (reuses `hermesClient`) | `session/set_model` | — | ACP server array | `.kimi/skills/` |
| `kiro` | `kiro-cli acp --trust-all-tools` | ACP | `session/set_model` | — | ACP server array | `.kiro/skills/` |
| `qoder` | `qodercli --yolo --acp` | ACP | `session/set_model` | — | ACP server array | `.qoder/skills/` |
| `antigravity` | `agy -p "…" --dangerously-skip-permissions --print-timeout <d> --log-file <tmp>` | plain text + log scrape | `--model "<display name>"` | — | — | `.agents/skills/` |
| `openclaw` | `openclaw agent [--local] --json --session-id <id> --message "…"` | JSON blob or NDJSON | `--agent <id>` (agent registration, *not* a model slug) | — | synthesised `openclaw.json` via `OPENCLAW_CONFIG_PATH` | `skills/` |

### The nine non-overlapping adapters

- **`codebuddy`** (`codebuddy.go`) — a Claude-compatible fork: identical
  `stream-json` NDJSON contract and the same in-band `control_request` approval,
  so it reuses Claude's event handling and writes skills into `.claude/skills/`.
  Effort levels are `low|medium|high|xhigh` (no `max`), discovered from
  `codebuddy --help` (a slow call — cached ~60 s).
- **`cursor`** (`cursor.go`) — `cursor-agent` speaks `stream-json` but prefixes
  each line with `stdout:` / `stderr:`, stripped before parsing
  (`normalizeCursorStreamLine`). Its background worker lingers after the run, so
  the adapter cancels the context the instant a `result` event arrives and uses a
  short 500 ms `WaitDelay`. MCP and trust are pre-seeded into a per-task
  `CURSOR_DATA_DIR` so no interactive approval is needed
  (`execenv/cursor_mcp.go`).
- **`pi`** (`pi.go`) — `pi --mode json`; the model identifier is a `provider/model`
  slug split into `--provider` + `--model` (`splitPiModel`). The session is a
  filesystem **path to a JSONL file** under `~/.multica/pi-sessions/`, not an
  opaque id. stdin is opened then immediately closed (the binary blocks on a
  stdin read otherwise). Text output is heavily filtered to strip embedded
  tool-call markup (`drainPiTextBuffer`).
- **`hermes` / `kimi` / `kiro` / `qoder`** (`hermes.go` + the three thin
  wrappers) — all four speak the **Agent Client Protocol** and share one
  transport (`hermesClient`). MCP servers are handed over inside `session/new`
  and filtered to the transports the runtime advertised in its `initialize`
  reply (`filterACPMcpServersByCapability`) — an stdio-only ACP runtime never
  receives an HTTP MCP entry. Auto-approval is `session/request_permission` →
  `approve_for_session`. Deviations: **kimi** streams tool-call arguments
  token-by-token and title-cases tool names (re-normalised via
  `kimiToolNameFromTitle`); **kiro** resumes with `session/load` and treats a
  `goal_complete` tool call as success even when the prompt RPC then errors;
  **qoder** runs in permanent bypass mode and needs a 2 s bounded drain because
  `qodercli` keeps pipes open after the response.
- **`antigravity`** (`antigravity.go`) — `agy` emits **no structured output**;
  every stdout line is treated as assistant text and all structured data
  (session id, "print timed out" signal, provider errors) is scraped from the
  glog-format `--log-file`. This scrape is mandatory because `agy` exits `0` even
  when the print mode times out — without it a timeout would look like success.
  The model is the exact human-readable display string (e.g.
  `"Claude Opus 4.6 (Thinking)"`), validated up front against `agy models`
  (fail-closed — the only adapter that rejects an unknown model before launch).
- **`openclaw`** (`openclaw.go`) — `openclaw agent` does not take `--model`; the
  model is bound when the agent is registered (`openclaw agents add/update
  --model`), and `ExecOptions.Model` carries an **agent id** passed as `--agent`.
  Output is either a single pretty-printed JSON blob or NDJSON — the adapter
  tries bulk-JSON first, then falls back to line scanning. `--local` is dropped
  when `OpenclawMode == "gateway"`. Unusually, the version floor (`≥ 2026.5.5`)
  is enforced **before every run**, not just at registration, because older
  builds wrote JSON to stderr and broke the parser
  (`openclaw.go:checkOpenclawVersion`).

**Takeaway for `agent-manager`.** Nine very different CLIs collapse onto one
adapter contract by routing the same five per-run knobs (model, reasoning, MCP,
skills, approval) through whatever channel each protocol exposes — a flag, an env
var, an RPC field, a written config file, or a pre-seeded sidecar — and by
scraping a side log when a harness gives no structured output at all. The harness
abstraction's hard requirement is therefore *not* a common wire format but a
common **set of injection seams** plus a normalising **event mapper** per
protocol family.

## Sources

- multica source tree — [`../../refs/multica/`](../../refs/multica/) (submodule,
  `git@github.com:multica-ai/multica.git`).
- `CLI_AND_DAEMON.md` (repo root) — the upstream narrative on the CLI/daemon
  split, polling, heartbeats, and task lifecycle.
- All other claims are anchored to the `server/…` symbols cited inline.
