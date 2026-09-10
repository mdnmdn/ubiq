---
name: ubiq-agents
description: Reference for agents in Ubiq — the agent-manager library boundary, how the host composes and drives a harness, the conversation wire vocabulary, and the chat panel / agents columns that draw one. Use when starting or launching a harness, touching accounts, profiles or permission modes, adding a ConvUpdate, or changing a chat tab, transcript, tool block, permission prompt or composer.
---

# Agents in Ubiq

An agent reaches the user through three layers, and each owns a strictly different fact:

```
agent-manager           ubiq-host                     ubiq (UI)
"how a harness runs"    "this harness, running"       "what the reader sees"
harness/, resolve,      agent.rs composes a run;      state/conversation.rs holds
spec, profile,          conversation.rs pumps its     the transcript; ui/conversation
account, isolate, io    bridge and maps its events    draws it; chat + agents columns
                        onto the bus                  are two surfaces onto one view
```

## Read first

| You are | Read |
|---|---|
| Launching a harness, or touching accounts / profiles / skills / MCP servers | `_docs/tech/agent-manager.md`, then `crates/agent-manager/_docs/` |
| Changing a chat tab, its attachment, or the start-or-attach control | `_docs/features/chat.md` |
| Changing the agents screen — columns, tabs, the bench | `_docs/features/workbench.md` |
| Adding or changing a message | `_docs/tech/transport-contract.md` |
| Session / workspace lifecycle | `_docs/features/sessions-and-workspaces.md` |

**"Session" means two things.** The library's session is a resumable *harness conversation*.
Ubiq's session is a named grouping of panes with a folder. Say which one you mean.

## The five rules

1. **Ubiq never names a harness configuration path.** Not `~/.claude`, not `CLAUDE_CONFIG_DIR`,
   not a settings filename. If Ubiq needs one, the library grows an accessor. A path literal in
   `crates/ubiq/src/` is the clearest sign the boundary has been crossed.
2. **Ubiq never hard-codes how to launch a harness.** Which binary, which arguments, which
   environment — the library answers all three.
3. **Nothing about windows, panes or terminals goes into the library.** It builds with no UI
   dependency; `just core` is that check.
4. **New harness support is a library change** — a new impl in `crates/agent-manager/src/harness/`
   against the runtime contract in `crates/agent-manager/_docs/harness/`. Ubiq gains it with no
   change of its own.
5. **A fact stated in the library's documentation is linked, never copied.**

Plus the standing architecture rule: `crates/ubiq` does not depend on `crates/ubiq-host` *or* on
`agent-manager`. Everything crossing is a `Message`. `just ui` and `just host` enforce it.

## Who owns what

| Fact | Owner |
|---|---|
| Where a harness stores config, in what format; how to launch it | the library |
| What a run is composed of — skills, MCPs, account, instructions, hooks | the library |
| A harness's permission modes (`Harness::modes`), which one means "ask nothing" (`Harness::unattended_mode`), and how `Profile.mode` reaches a policy | the library |
| Which accounts exist; how credentials are referenced | the library |
| Which files are a harness's own record, and its on-disk shape | the library |
| How harness I/O becomes structured events, and what they are called | the library |
| What a policy grants; which layers a confined run stacks | the library |
| Which definition a conversation starts from, and the form that writes one | Ubiq |
| Where a run's record is kept, under whose id, when it is written | Ubiq |
| The one translation from `AgentEvent` onto the bus | Ubiq |
| Whether an agent is confined at all, and where its run directory lives | Ubiq |
| Which `$HOME` a confined agent runs with, and every extra grant | Ubiq |
| That a harness runs under a pseudo-terminal in a pane; which panes exist | Ubiq |

## The host side, in one paragraph each

**`crates/ubiq-host/src/agent.rs` is the whole of the consumption, and is deliberately thin.**
It calls `agent_manager::resolve::resolve` with a `RunFlags` naming only the harness and folder,
and overrides exactly **four** fields of what comes back — the configuration directory, the I/O
mode, the isolation, and, when a run is isolated, the permission mode: the sandbox contains it, so
it asks nothing, and `Harness::unattended_mode` is the library's word for which mode that is (an
explicit mode picked for the run outranks it; a profile's does not). Everything else (account, model, skills, MCP servers, config overlays) is
the library's answer read from the profile. So an account reaches a pane without `agent.rs`
learning what an account is.

**A workspace has two faces, both composed here.** `Agents::compose` is the terminal one:
`IoModes::Passthrough` plus a `Launch` to exec under a pseudo-terminal. `Agents::converse` is the
other: `IoModes::Structured` plus a `structured_bridge` over the harness's own JSON — a
conversation's harness writes frames on a pipe rather than drawing a screen.

**`crates/ubiq-host/src/conversation.rs` is the pump thread that owns the bridge.**
`IoBridge::next_event` blocks and both its methods take `&mut self`, so whoever reads the bridge
cannot also be handed a prompt: the reader owns it, and a turn reaches the harness through the
detached `AgentInputSink`. A harness answering `None` there takes no second turn —
`Conversation::accepts_input` is how the interface asks. Events use the same unbounded mailbox a
pseudo-terminal's reader does, so a window behind on drawing never stalls the harness.

**`map_event()` in that file is the only place that names `agent_manager::io::AgentEvent` and
`ubiq_proto::conversation::ConvUpdate` together.** Both are ACP `session/update` vocabulary
(`D53`), so the translation is a rename. **A second mapping anywhere else is the boundary being
crossed.**

## The wire

The UI speaks only `crates/ubiq-proto`. See
[`reference/wire.md`](reference/wire.md) for the full conversation family, every `ConvUpdate`
variant, and the `AgentTypeInfo` / `AccountInfo` / `ProfileInfo` / `WorkspaceInfo` records.

The shape to remember: `StartConversation` (UI mints the `AgentId` client-side, so a surface
attaches with no round trip) → `ConversationStarted` → many `ConversationUpdate { seq, update }` →
`ConversationEnded`. Turns go out as `PromptAgent`, are stopped by `CancelTurn`, and a
`ConvUpdate::PermissionRequest` is answered by exactly one `AnswerPermission { request_id,
option_id }`. **`UnloadConversation` kills the harness and keeps the conversation**, and
`AbortConversation` does the same by killing the process outright rather than asking it to shut
down and waiting; only `EndConversation` takes everything.

## The UI side

Two surfaces host a live agent — the **chat panel** (`ui/chat/`) and an **agents column**
(`ui/agents/column.rs`) — and both draw the *same* shared view, `ui/conversation/mod.rs`, over the
same state, `state/conversation.rs`. A rule read in two places is a rule that drifts: every
lifecycle question (`lifecycle`, `lifecycle_colour`, `lifecycle_menu_enabled`, and the
`lifecycle_mark` / `lifecycle_menu` fragments) is answered in that one module, and a surface that
wants the controls elsewhere flips `ConversationView::header` rather than forking them.

See [`reference/ui.md`](reference/ui.md) for the transcript, tool folding, permission prompts,
subagents, attachments, the footer's readings, the composer pool, and the chat tab / column
lifecycles.

The load-bearing points:

- **A view is never the workspace.** A chat tab is a perspective on a host-owned conversation;
  closing one ends nothing, and the host is never told which surfaces are looking. It holds in one
  direction only: `ConversationDeleted` **closes** every chat tab attached to that conversation —
  `AppState::close_chat_tab_in(project, id, cx)`, by project id because the delete can arrive while
  the window is looking elsewhere. A delete is the one host event that ends a view, because a
  detached tab would be an empty panel left where a conversation used to be.
- **Exclusivity is per chat tab, not per conversation** (`D61`) — the agents screen may show the
  same conversation in a column at the same moment a chat tab is attached to it.
- **Closing a column tab benches the agent; it does not end it.** Nothing on the agents screen
  kills an agent, `Close all` included.
- **The bench is computed, not stored** — every agent the host reports that no column shows.
- **A permission ask is drawn on the tool call it authorises**, joined by tool call id, not in a
  dialog. Several may be up at once, all block, and there are no timeouts.
- **`option_id` is opaque.** `kind` decides only how a button *reads*; nothing on this side
  interprets an id or remembers a choice.
- **Rows and the actions behind them are matched by position** — one `ChatPick` / `StartOffer`
  list that the frame draws and the click resolves against, never two readings of the harness list.

## Adding things

**A new `ConvUpdate` variant**: add it in `crates/ubiq-proto/src/conversation.rs`, map it in
`conversation.rs::map_event` (and nowhere else), fold it into `Conversation::apply` in
`crates/ubiq/src/state/conversation.rs`, and draw it in `ui/conversation/mod.rs`. Update
`_docs/tech/transport-contract.md` in the same commit.

**A new harness**: nothing in this repo's Ubiq half. It is
`crates/agent-manager/src/harness/<name>.rs` plus its `io/` bridge, against
`crates/agent-manager/_docs/harness/`.

**A new composition knob** (a skill set, an MCP server, an instruction): it belongs on the
`Profile`, resolved by `resolve`. `agent.rs` needs no change — that is the point of the split.
What the *interface* can set today is four fields (harness, account, model, mode); the rest is
written by hand because nothing lists the catalog on the wire — a backlog row, not a hedge.

## Verifying

`just verify` = `check clippy test host ui docs-lint`. `just core` checks the library still builds
with `--no-default-features`. Conversation tests are `crates/ubiq/tests/conversation.rs` and
`chat.rs` — state-level, driving `AppState`.
