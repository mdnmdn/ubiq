---
id: inbox-web-interface
title: Proposal — a web interface served by the host, read-only observers first
kind: proposal
status: proposal
summary: The host serves a browser interface and a websocket beside its existing remote listener, and the bus gains a second client role — observer — so one host with its desktop interface (local or remote) as the only writer can carry several web clients that explore the project's code, agents, teams and tasks without being able to change anything; reconnection resumes from a per-stream cursor, and the first release covers the IDE, Agents, Teams and Tasks workbenches read-only.
read_when: you are deciding whether and how Ubiq exposes a workspace to a browser, adding a client role or a read-only gate to the bus, or building a viewer that attaches to a host it does not drive
updated: 2026-10-11
depends_on: [tech-architecture, tech-transport, tech-decisions, feat-workbench-ide, feat-workbench-agents, feat-workbench-teams, feat-workbench-tasks, inbox-kb-web, inbox-multi-attach, inbox-conversation-record]
---

# Proposal — a web interface served by the host, read-only observers first

Ubiq already has a remote host: `ubiq-app --serve` opens a listener (`crates/ubiq-host/src/remote.rs`)
that upgrades `GET /attach?token=` to raw length-prefixed MessagePack frames, and a desktop
interface on another machine attaches to it as an ordinary bus client. This proposal adds a second
kind of interface on the same listener, a browser. It also adds the one thing the bus lacks for it:
a client that can look but not touch.

The case it is built for is **one host, one driver, many watchers**. A host runs with its desktop
interface, either in-process or remote. That interface is the only one that writes. Several people
open the host's URL in a browser and explore the same workspace: the code, the agents and their live
transcripts, the teams canvas and the task boards. They see what the driver does as it happens, and
nothing they do can change it.

## 1. Where it stands

| Fact | Today | Consequence for a web client |
|---|---|---|
| Clients per host | Unbounded; a remote connection is a `Client` like a window (`bus.rs`) | Attaching N browsers needs no new routing primitive |
| Client identity | `ClientId`, a process-local counter; no role, no permission | Any client holding the token can spawn, write, kill (`G168`) |
| Auth | One 256-bit token printed at startup; optional TLS (`serve_tls`) | One token means one privilege level |
| Wire | 4-byte length + MessagePack, `to_vec_named` (`wire.rs`, `D79`) | A browser can decode it with a stock msgpack library |
| HTTP | The listener answers `GET /` with `200` and `/attach` with `101`; no websocket | Static pages and a websocket fit on the same port |
| Pane bytes | `To::Client(owner)` only; emulator and scrollback live in the interface | A watcher gets no terminal stream and no history |
| Conversations | `To::Client(owner)`, delta stream, host keeps nothing (`G208`) | A watcher cannot list or replay a transcript |
| Work (tasks, agents, boards) | Host-owned; mostly answered to the asker, a few broadcasts (`G47`) | A watcher can read a snapshot, but not every change reaches it |
| Files | Read requests answered to the asker; `ProjectFilesChanged` broadcast | IDE exploration works as-is |
| Keepalive | None (`G189`); a dropped client's panes are killed on `Gone` | Harmless for a watcher, which owns nothing |

So the IDE and Tasks reads work over the existing messages today. Teams and Agents need two things
the host does not have: a way to find conversations it did not start, and a history to replay them
from.

## 2. Topology

```
                         ┌──────────────── host process ────────────────┐
 desktop UI (local) ─────┤ Hub ── coordinator ── workers, stores, ptys   │
                         │  │                                            │
 desktop UI (remote) ────┤  ├─ remote.rs listener  :7420                 │
   /attach  (writer)     │  │    GET /attach   → native frames          │
                         │  │    GET /ws       → websocket frames        │
 browser × N ────────────┤  │    GET /, /app/* → embedded web bundle     │
   /ws      (observer)   │  │                                            │
                         └──┴────────────────────────────────────────────┘
```

- **The host is the web server.** The bundle is embedded in the binary and served from the listener
  that already exists. Nothing new is bound and no second process is needed. The interface-side
  `web_export` server stays what it is: a loopback server for one window's web panels.
- **The writer is unchanged.** A local window keeps its in-process `Client`. A remote desktop keeps
  `/attach`.
- **Every browser is an observer** in this proposal. A browser writer is a later phase (§10) and is
  not designed here beyond not ruling it out.

The supported shapes, all on one host:

| Shape | Writer | Observers |
|---|---|---|
| Desktop with watchers | Local desktop window(s) | N browsers on the LAN or over TLS |
| Headless host, remote driver | One remote desktop via `/attach` | N browsers |
| Headless host, watch-only | None | N browsers on an otherwise idle host, e.g. a CI box running agents from a mission |

## 3. Client roles

`FromClient::Connected` gains a role, and the coordinator keeps it next to the client:

```rust
pub enum ClientRole { Writer, Observer }

FromClient::Connected { client: ClientId, role: ClientRole }   // bus.rs
roles: HashMap<ClientId, ClientRole>                            // coordinator
```

- `Hub::connect()` keeps minting a `Writer`. A new `Hub::connect_as(role)` exists for the listener.
- **The role comes from the token, not from the transport.** The host prints two tokens at startup,
  `--writer-token` and `--observer-token`, each settable from a flag or generated. A desktop
  interface could attach with the observer token too, which makes the gate testable without a
  browser.
- **The gate is in the coordinator, before dispatch.** It is a single function in `ubiq-proto`:

  ```rust
  impl Message { pub fn observer_may_send(&self) -> bool { matches!(self, /* allowlist */) } }
  ```

  It is an **allowlist**, so a message added later is denied to observers until someone decides
  otherwise. A refused message is answered with a new `Refused { kind: &'static str, reason }` and
  logged under the `transport` subsystem. Naming conventions (`List*`, `Read*`) are not trusted;
  the list is explicit.
- **The web server never filters.** The host is the authority, and a browser that hand-crafts
  frames gets the same answer as one that does not.

## 4. Fan-out to observers

Today the routing decision is "the asker" or "everyone". An observer needs a third answer: **every
client watching this project**.

- New messages: `Watch { project }` and `Unwatch { project }`, observer-allowed.
- The coordinator keeps `watchers: HashMap<ProjectId, HashSet<ClientId>>`. A single helper,
  `mirror(project, &msg)`, sends a copy to each watcher. It is called at the points where
  project-scoped state changes and is today sent only to an owner:

| Stream | Today | Mirrored to watchers |
|---|---|---|
| `ConversationStarted` / `Update` / `Ended` | owner | yes |
| `AgentChanged`, `TaskCreated` / `Changed` / `Deleted`, `MissionChanged` | asker (mostly) | yes (also closes `G47` for second windows) |
| `ProjectFilesChanged`, `GitChanged`, `Boards`, `WorkList` push | everyone | already reaches them |
| `TerminalOutput` | owner | phase 4 only, opt-in per pane (§8) |

- The writer's hot path is untouched. `Mailbox` stays pre-resolved, and the mirror is skipped when
  the watcher set is empty.
- Each mirrored copy goes into an unbounded flume channel, as everything does. A slow browser grows
  its own queue and never the reader's. The websocket pump drops a client whose queue passes a cap
  (say 64 MiB), and that client resynchronises via §6.

## 5. What an observer can see — the first release

These are the four workbenches, read-only. The allowlist is everything in the "Read" column.

| Workbench | Read (allowed) | Never allowed |
|---|---|---|
| **IDE** | `ProjectTree`, `ReadProjectFile`, `DiffProjectFile`, `RelatedProjectFiles`, `ProjectGit`, `ProjectGitLog`, `ProjectGitRefs`, `SearchProject` / `CancelSearch` | `WriteProjectFile`, `EditProjectPath`, `ImportIntoProject`, `WriteProjectGit`, every `*HostFile` / `BrowseHostDir` (outside the project) |
| **Agents** | `ListConversations` (new), `ConversationHistory` (new), `ListAgentTypes` | `StartConversation`, `PromptAgent`, `CancelTurn`, `AnswerPermission`, `AnswerAsk`, every `Set*`/`End*`/`Resume*`/`Revive*` |
| **Teams** | `ListWork`, `ListMissions`, `LoadJournal`, plus the Agents reads | `AssignAgent`, `SendToAgent`, `AnswerSpawn`, mission mutations |
| **Tasks** | `ListWork`, `ListBoards`, `OnBoard` wrapping a read, `LoadPlan`, `ListPlanAnnotations` | every task, step, comment, board, plan and task-source mutation |
| Always | `ListProjects`, `Watch` / `Unwatch`, `Hello` | panes (`SpawnWorkspace`, `TerminalInput`, `TerminalResize`, `StopPane`), accounts, connectors, settings, DB, KB writes, `RunTool` |

Two notes:

- **Permission prompts are shown, not answered.** An observer sees that the agent is waiting on the
  driver, which is useful, and has no button to answer it.
- **The Teams canvas arrangement is interface-only state.** The browser lays the canvas out itself,
  using the same fence-by-task rule. It does not mirror the driver's card positions.

## 6. Snapshots, history and reconnection

A watcher arrives late and drops out. Both cases come down to the same rule: **every stream an
observer reads has a cursor, and the host can answer "everything after cursor X" or "here is a
fresh snapshot".**

| Stream | Cursor | Resume |
|---|---|---|
| Conversation | `seq`, already per agent and monotonic from 1 | `ConversationHistory { agent_id, after_seq }` replays the kept `ConvUpdate`s as ordinary `ConversationUpdate` frames, then `HistoryEnd { agent_id, seq }` |
| Work (tasks, agents, missions) | none | re-ask `ListWork` / `ListMissions`; a work epoch is not worth adding |
| Files, git, search | none | re-read what is open; search is re-run |
| Terminal (phase 4) | byte offset | ring replay from offset, or the ring's start (§8) |

**Conversation history is the only real new host work.** It is the primitive
[`conversation-record.md`](./conversation-record.md) already argues for: a host-owned, replayable
`ConvUpdate` record. This proposal needs only its in-memory half. Each live conversation keeps a
bounded `Vec<(seq, ConvUpdate)>` in the coordinator, and the cap is a setting. When the cap is hit,
the oldest turns are dropped and the replay starts with a `HistoryTruncated` marker. The on-disk
half, compaction and restart survival stay in that proposal. The web viewer becomes its first
consumer, not a competing design.

`ListConversations { project }` returns each live conversation's `WorkAgent`, harness, title, owner
role and last `seq`. That is the discovery `G208` says is missing, and the desktop interface's second
window can use it too.

**Reconnection, browser side:**

- On first load the page mints a `ViewerId` (a ULID) and keeps it in `sessionStorage`. Each tab is
  its own viewer.
- It connects with backoff: 0.5 s doubling to 15 s, with ±20 % jitter, and no attempt cap while the
  page is open.
- The first frame after the upgrade is `Hello { token, viewer, resume: Vec<(AgentId, seq)> }`. The
  token travels in the frame, not the URL, so it stays out of access logs and history. The host
  answers `Welcome { role, host_info }`. On a bad token it closes with websocket code 4401.
- On `Welcome` the client re-sends `Watch` for its projects, then one `ConversationHistory` per
  transcript it was showing, from the last `seq` it applied. It also re-asks `ListWork` and re-reads
  open files.
- A gap inside a live stream (a `seq` jump) triggers the same `ConversationHistory` call. A replay
  that comes back truncated makes the client discard that transcript and rebuild it.

**Reconnection, host side:**

- An observer owns nothing, so `Gone` only drops its watcher entries and searches. The kill-on-`Gone`
  rule for writers is untouched.
- Websocket ping every 15 s, and close after 2 missed pongs. This settles `G189` for `/ws`, and the
  same timer can be added to `/attach` later.

## 7. The websocket and the bundle

- **Framing.** One websocket binary message is one MessagePack-encoded `Message`, with the same
  encoding as `wire.rs` minus the length prefix, which websocket framing replaces. No JSON twin
  exists, so there is one codec and one contract.
- **Library.** The listener is synchronous std I/O, and the websocket should match. `tungstenite`
  is synchronous, works over the `TcpStream` or the rustls stream `remote.rs` already holds, and
  adds no runtime. Hand-rolling RFC 6455 is not worth the review cost.
- **Upgrade checks.** `GET /ws` requires a websocket upgrade. An `Origin` header must match the
  host's own origin, or appear in `--web-origin`. The existing 8 KiB header cap and 10 s handshake
  timeout apply.
- **Static routes.** `GET /` and `GET /app/*` serve the embedded bundle with a strict CSP:
  `default-src 'self'; connect-src 'self'`. There is no token on static routes, because they carry
  no data.
- **Feature gate.** `ubiq-host` gains a `web` feature, on in `ubiq-app`. `just host` builds without
  it.
- **The bundle.** It lives in a new source folder `web/` at the root. It is TypeScript with Preact,
  CodeMirror 6 read-only for code, and `@msgpack/msgpack`, built by esbuild into one JS file and one
  CSS file. The built output is committed under `crates/ubiq-host/web-dist/` and embedded with
  `include_bytes!`, so `cargo build` needs no Node. A `just web` recipe rebuilds the bundle.
  Markdown files render through the same markdown library the bundle already pulls in. Using
  `ubiq-md` via wasm is a later option, not a first-release need.
- **Types.** The TypeScript types for the allowlisted messages are generated from `ubiq-proto` with
  `ts-rs` behind a dev feature, and checked in. `just web-types` regenerates them, and the
  allowlist test fails if a generated type is missing.
- **Theme.** The bundle reads a CSS-variable export of the `theme.rs` tokens, generated by the same
  recipe. No literal colour is written in the bundle. The rule in `AGENTS.md` extends to it.

## 8. Terminal output, later

The user asked for an eventual console stream. It is phase 4 because nothing replays a terminal
today:

- The emulator lives in the interface, and Ubiq writes no VT parser. The browser therefore renders
  with xterm.js, which is someone else's emulator, so the rule holds.
- The host keeps a per-pane byte ring, 1 MiB by default, opt-in per pane through a `Share` toggle
  the driver controls. `WatchPane { pane, from_offset }` replays the ring and then mirrors live
  `TerminalOutput`.
- A replay that starts mid-escape-sequence or after the alternate screen was entered can draw
  garbage until the harness repaints. The viewer offers a "request repaint" action, which the host
  turns into a no-op resize nudge to the same geometry. Whether that nudge is acceptable is
  decision 4 in §11.
- **Geometry stays the writer's.** The browser scales or scrolls the writer's grid. It never sends
  `TerminalResize`. A pane has one geometry (see the multi-attach proposal).

## 9. Security

- There are two tokens. The observer token can be shared. The writer token is never shown in the web
  interface.
- Without TLS the host warns at startup that observers will be readable on the wire. When
  `--serve` binds a non-loopback address with a web observer token but no TLS, it prints the warning
  once per start.
- **Project-relative paths only.** Every IDE read already resolves inside the project root, and
  host-browse messages are excluded. One open question is a per-project `observer_hide` glob list
  (default: `.env*`, `*.pem`, `*.key`) that the files worker applies when the asker is an observer.
- Observers never see account, connector or credential messages, because they are not on the
  allowlist.
- Rate limits: at most 4 concurrent searches per observer, and `ProjectTree` depth is capped as for
  any client.

## 10. Phases

| Phase | Delivers | Host work | Web work |
|---|---|---|---|
| 0 | Roles and the gate | `ClientRole`, two tokens, `observer_may_send`, `Refused`; a desktop interface can attach read-only | — |
| 1 | IDE read-only in a browser | `/ws` with tungstenite, static routes, `Hello`/`Welcome`, ping, `Watch` | Shell, project list, tree, CodeMirror viewer, git log and diff, search |
| 2 | Tasks and Teams read-only | `mirror()` on the work and mission streams | Board, task detail, plan reader, teams canvas |
| 3 | Agents read-only, live | `ListConversations`, in-memory `ConvUpdate` history, `ConversationHistory`, `HistoryEnd`, `HistoryTruncated` | Transcript renderer for `ConvUpdate`, gap detection, resume |
| 4 | Console streams | Per-pane byte ring, `WatchPane`, `Share` toggle in the desktop interface | xterm.js viewer |
| later | A browser writer | Write lease from the multi-attach proposal, grace period instead of kill-on-`Gone` | Composer, permission answers, editing |

Phases 0 to 2 need no new persistence. Phase 3 is the first that stores anything, and it stores it
in memory.

## 11. What this asks to be decided

1. **Role by token.** Is a second token right, or should observer access be a separate port that
   cannot carry writes at all? A separate port is simpler to audit, and the token is simpler to run.
   The recommendation is the token, with the gate in the coordinator.
2. **History cap.** What is the in-memory `ConvUpdate` cap per conversation, by turns or by bytes?
   The recommendation is bytes (8 MiB) with a turn-aligned cut.
3. **Bundle build.** Is committing the built bundle acceptable, or should `build.rs` require Node?
   The recommendation is to commit it.
4. **Repaint nudge.** Is a same-size resize an acceptable way for an observer to make a harness
   repaint, given it is a side effect on the writer's process? The alternative is a viewer that
   waits for the next natural repaint.
5. **`observer_hide`.** Should it ship in phase 1 or wait for a request?

## Related docs

- [`../tech/transport-contract.md`](../tech/transport-contract.md): owns every message; phases 0 to 3
  add `Hello`, `Welcome`, `Refused`, `Watch`, `Unwatch`, `ListConversations`, `Conversations`,
  `ConversationHistory`, `HistoryEnd` and `HistoryTruncated`.
- [`../tech/architecture.md`](../tech/architecture.md): the one-host-per-process and UI/host boundary
  this keeps.
- [`./tui/multi-attach-proposal.md`](./tui/multi-attach-proposal.md): write leases and the joiner's
  first frame, which is the browser writer's prerequisite.
- [`./conversation-record.md`](./conversation-record.md): the replayable `ConvUpdate` record. Phase
  3 builds its in-memory half.
- [`./kb-web-export-proposal.md`](./kb-web-export-proposal.md): the interface-side read-only HTTP
  export. It is a different server for a different audience, and the two do not merge.
- Backlog rows this touches: `G47`, `G168`, `G189`, `G208`.
