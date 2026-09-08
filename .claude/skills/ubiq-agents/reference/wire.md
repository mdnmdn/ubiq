# The conversation wire, and the library behind it

## `crates/agent-manager` — module map

`crates/agent-manager/_docs/README.md` is the library's own entry point; read in the order it
names. This table is only so you know which module answers which question.

| Module | Answers |
|---|---|
| `harness/` | How to identify, provision and launch each supported harness — `claude.rs`, `codex.rs`, `copilot.rs`, `grok.rs`, `opencode.rs`, `shared.rs`. `Harness::modes` is a harness's permission modes; `Harness::transcripts(config_dir)` is the files it wrote as its own record (defaulted empty = not portable yet; overridden today only by `Claude`) |
| `spec.rs` | `RunSpec` — the fully-resolved, harness-agnostic plan |
| `resolve.rs` | Merge flags + settings + catalog into a `RunSpec`. **Ubiq calls this rather than building a spec itself** |
| `profile.rs` | `Profile` — a saved setup: harness, account, model, and `mode` beside `isolate`. The profile named `default` is what an unqualified run resolves to |
| `account.rs`, `credentials/` | Accounts as credential *references*; pluggable secret stores (memory / file / keychain / OS) |
| `provision.rs`, `overlay.rs`, `config.rs`, `source.rs` | Turn a `RunSpec` into an ephemeral config dir plus launch argv/env |
| `registry/` | The catalog of skills and MCP servers (trait + filesystem impl) |
| `io/` | Bridging harness I/O into a neutral event model — `model.rs`, `structured.rs`, `acp.rs`, `agui.rs`, `jsonl.rs`, plus per-harness `codex.rs`, `copilot.rs`, `opencode.rs`; `passthrough.rs` is behind the `pty` feature |
| `isolate.rs` | Wrap a `Launch` in an isol8 sandbox invocation. A pure transform over `Launch` |
| `session.rs` | Session history — `SessionMeta` and transcripts, under the library's own state dir |
| `mcp/` | The embedder-facing `McpService` trait (core) and the loopback server that hosts it (`inproc-mcp`) |
| `run.rs`, `cli/`, `tui.rs` | The library's own front ends — **not built into Ubiq** |

**Feature flags Ubiq uses**: `default-features = false` (no `clap`, no `ratatui`), plus
`inproc-mcp` when Ubiq exposes its own tools back to a hosted agent. `cli` and `pty` are absent
from this build, so the host builds its stores itself.

The stores `resolve` reads are filesystem defaults rooted under **Ubiq's own** config root —
`<root>/accounts`, `<root>/profiles`, `<root>/catalog` — so a development run never touches what
the `am` CLI manages. A missing directory is an empty store, not an error. The library's own
settings file is deliberately **not** read: Ubiq's settings are the settings surface, so
precedence is flags, then the profile.

## `crates/ubiq-host/src/agent.rs` — `Agents`

| Method | Does |
|---|---|
| `new(root, isolate)`, `isolate()`, `set_isolate()` | The store root and the confinement toggle |
| `set_policy(home, extra_grants)` | Pushed on every `SetSettings`: becomes the home mode and `extra_ro`/`extra_rw` of the `IsolateOptions` handed to `isolate::plan` |
| `set_commands(map)`, `command_of(agent_type)` | Custom launch commands per harness |
| `types() -> Vec<AgentTypeInfo>` | `harness::all()` projected, each marked with whether its binary is on this machine |
| `is_agent_type(id)` | A spawn naming one of those ids composes a run; anything else is a program name, which is what a shell is |
| `discover_models(agent_type)` | The harness's own model list |
| `accounts()`, `rename_account()`, `delete_account()`, `delete_harness_login()` | The account store |
| `profiles()`, `save_profile(ProfileInfo)` | Read/write through `FsProfileStore` at `<root>/profiles`; a profile pinning no harness is skipped. **There is no delete, because the library offers none** |
| `check_login(agent_type, account, now_ms) -> LoginStatus` | A credential that is absent is an answer |
| `begin_login(...) -> PendingLogin`, `finish_login(&PendingLogin)` | An interactive harness login, run in a real terminal inside a modal |
| `compose(...) -> Composed` | The terminal face: `IoModes::Passthrough`, `Composed::exec() -> Launch` |
| `converse(...)` | The conversation face: `IoModes::Structured` and a `structured_bridge` |
| `agent_dir(agent)`, `retire(pane)`, `retire_agent(agent)`, `sweep()` | Run directories — `ConfigStrategy::Fixed` under Ubiq's root, named by the pane id or agent id (both ULIDs, so neither reads as the other's), deleted on close, swept at startup |
| `sessions_dir()` | `<root>/sessions`, passed explicitly rather than via `session::sessions_root`, so `AM_SESSIONS` cannot redirect Ubiq transcripts into the `am` CLI's store |
| `archive(...)` | Called by every path that deletes a run directory, the startup sweep included |

**Confinement is Ubiq's answer, the policy is the library's.** An agent in a pane is confined
unless the host settings say otherwise; the policy grants the project folder and the run directory
and denies the rest. `HostSettings.agent_home` and `extra_grants` decide the `$HOME` and the extra
paths. The default home is the user's real one and that is **not** a soft default: a layer's
`~`-relative grant expands against the run's *effective* home, so a replaced home aims every
toolchain grant (`~/.cargo`, `~/.npm`, `~/.dotnet`) at a directory nothing populated.
`compose_run` calls `IsolateOptions::grant_toolchains_from_env()` before applying the settings'
grants, so a `CARGO_HOME` or `GOPATH` outside `~` needs no host setting; the user's own grant is
applied last and stays the last word. Confining a run in a terminal Ubiq owns is **macOS-only**
(`sandbox-exec`); the replacing seam is `refs/isol8-pty-seam-update.md`.

Tool approval is the bridge's alone — it answers each request itself, because it holds its child's
descriptors. **Today only the Claude bridge actually asks a human**: Codex auto-accepts every
approval RPC, opencode runs `--dangerously-skip-permissions` and Copilot `--allow-all
--no-ask-user`, so an `AnswerPermission` naming one of those three is a documented no-op. `G92`.

## `crates/ubiq-host/src/conversation.rs` — `Conversation`

`start(...)`, `ended()`, `accepts_input()`, `prompt(text)`, `cancel()`,
`answer_permission(request_id, option_id)`, `set_config(config_id, value)`, `stop(quiet)`, plus
`UsageMeter`. `map_event(AgentEvent) -> Option<ConvUpdate>` is the single translation point.

## Message family — UI → host

| Message | Carries |
|---|---|
| `StartConversation` | `agent_id` (**minted client-side**, so a surface attaches with no round trip), the folder, `agent_type` from `AgentTypeInfo`, optional `account` from `AccountInfo`, optional `profile` from `ProfileInfo` |
| `PromptAgent { agent_id, text }` | One turn. Attachments are composed into this text as `@path` mentions — nothing new crosses the bus for them |
| `CancelTurn { agent_id }` | The turn ends; the conversation and harness stay |
| `AnswerPermission { agent_id, request_id, option_id }` | Answers one `PermissionRequest`. The option id is echoed back unchanged |
| `SetAgentConfig { agent_id, config_id, value }` | A model, a mode, a thinking level — one message for all of them, because upstream has one mechanism for all of them |
| `UnloadConversation { agent_id }` | Kills the harness, keeps the conversation: the transcript stays, the run directory stays (seeded credentials included), and the same id restarts |
| `ResumeConversation { agent_id }` | Starts an unloaded conversation's harness again under the same id, with no prompt |
| `EndConversation { agent_id }` | Stops the agent and cleans up after it |

Adjacent families: `SpawnWorkspace` / `WorkspaceSpawned` / `CloseWorkspace` (the *terminal* face of
the same thing), `ListAgentTypes` / `AgentTypes`, `CheckAgentCommand`, `ListAccounts` / `Accounts`
/ `RenameAccount` / `DeleteAccount` / `DeleteHarnessLogin` / `AccountError`, the harness-login
family (`BeginHarnessLogin` → `HarnessLoginStarted` → zero or more `HarnessLoginLink` →
`HarnessLoginCaptured` or `HarnessLoginFailed`; `CheckHarnessLogin` → `HarnessLoginStatus`), and
`ListProfiles` / `Profiles` / `SaveProfile`.

## Message family — host → UI

| Message | Carries |
|---|---|
| `ConversationStarted` | `project_id`, a boxed `WorkAgent`, the `WorkSession` it belongs to (**the session travels with the agent** — the sidebar lists agents *under* a session, so an agent whose session nothing names is an agent nothing draws), and `accepts_input` — a one-shot harness answers `false`, so the capability travels rather than being discovered by a refusal |
| `ConversationUpdate` | `agent_id`, `seq` (per agent, monotonic, from one — order is promised per agent and not across them), a boxed `ConvUpdate`, and optional `raw`: the harness's own line, drawn by the debug viewer and read by nothing in the transcript |
| `ConversationEnded { agent_id, stop_reason }` | The harness is gone; the transcript stays |
| `ConversationUnloaded { agent_id }` | The harness is gone and the conversation is not — back to the state before its first turn |
| `ConversationError { agent_id, error }` | A sentence |

## `ConvUpdate` — the vocabulary

ACP `session/update` (`D53`). Three variants have no ACP equivalent because ACP carries them at
the protocol level: `Started` is `session/new`'s result, `PermissionRequest` is a request back to
the client, `TurnEnded` is `session/prompt`'s response. They are updates here because the bus is
one stream, not a JSON-RPC peer.

| Variant | Is |
|---|---|
| `Started { session_id, model, mode, tools, agents }` | The harness is up. `session_id` is the *harness's* own id, the one a resume needs — not the agent's |
| `UserChunk { content, message_id }` | What the user said, as the harness received it. The composer appends nothing of its own |
| `AgentChunk { content, message_id, subagent }` | Assistant prose. Chunks sharing a `message_id` are one message; a change of id starts a new one |
| `ThoughtChunk { content, message_id, subagent }` | Reasoning |
| `ToolCall(ToolCallRecord)` / `ToolCallUpdate(ToolCallPatch)` | **Absent patch fields are unchanged** — a patch naming only a status must not claim to clear a title |
| `Plan(Vec<PlanEntry>)` | The todo list, complete each time |
| `ConfigOptions(Vec<ConfigOption>)` | Every option, complete each time — setting one can change another, so a partial set would leave a stale picker |
| `ModeChanged { mode_id }` | The harness changed mode by itself |
| `Title(String)` | Where the harness names the conversation |
| `Usage(UsageRecord)` / `RateLimit(RateLimitRecord)` | `context_pct()` is used-over-size; a window nobody reported draws no ring rather than one computed from a constant |
| `PermissionRequest { request_id, tool_call, options }` | `tool_call` is a **patch whose id is the only guaranteed field** — the prompt reads the rest off the call the transcript already holds |
| `TurnEnded { stop_reason, error }` | The turn is over; the agent is still alive |

Supporting types in `crates/ubiq-proto/src/conversation.rs`: `ConvContent`, `ToolKind` (`Read`,
`Edit`, `Delete`, `Move`, `Search`, `Execute`, `Think`, `Fetch`, `SwitchMode`, `Delegate`,
`Other`), `ToolStatus`, `ToolContent`, `DiffKind`, `ToolLocation`, `Subagent`, `ToolCallRecord`,
`ToolCallPatch`, `PlanEntry`, `ConfigCategory` (`Mode`, `Model`, `ModelConfig`, `ThoughtLevel`),
`ConfigOption`, `ConfigChoice`, `ConfigValue`, `PermissionKind` (`AllowOnce`, `AllowAlways`,
`RejectOnce`, `RejectAlways`), `PermissionOption`, `TokenSpend`, `UsageRecord`, `RateLimitRecord`,
`StopReason`.

## The records that cross

| Record | Fields, and the rule behind them |
|---|---|
| `AgentTypeInfo` | `id` (library harness id, e.g. `claude-code`), `label`, `command` (what the library *would* run — a placeholder a custom command is typed over, **never** something the interface composes a launch from), `available` (binary found, or a custom command configured — a row that cannot start says so before it is picked), `modes` (this harness's advertised permission modes; empty where it has no such axis, because a mode is not a universal concept) |
| `AccountInfo` | `id`, `logged_in` (harness ids with a captured login — **derived**, not recorded: an account is a home, and a harness is logged in there when the files it names are present; an empty list means an env-var reference rather than a captured session). No credential and no path ever appears here |
| `ProfileInfo` | `id`, `agent_type`, `account`, `model`, `mode` — every field a *reference*, `None` meaning the profile does not mention that axis and a lower layer decides |
| `WorkspaceInfo` | `id` (also its pane's), `session_id`, `agent_type` (what the coordinator actually started), `project_id`, `rel_path`, `cols`, `rows`, `running`. No process, no writer, no pseudo-terminal |
| `LoginStatus` | `Valid { expires_at_ms }`, `Expired { expires_at_ms }`, `Unknown` (stored but names no expiry — an API key looks like this and is usually fine), `Missing` |
| `Secret` | The only way material crosses. `Debug` prints `Secret(***)`; no `Display`, no `Deref`, no `AsRef<str>` — so `tracing::info!(token = %secret)` does not compile. `expose()` is the one way out, which makes every escape one grep |
