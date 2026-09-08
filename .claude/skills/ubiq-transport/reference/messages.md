# The message contract

`crates/ubiq-proto/src/messages.rs` — one tagged enum, 1595 lines, serialisable by construction.
`#[serde(tag = "type", content = "payload")]`: the variant name travels in `type`, the body in
`payload`, and a body-less variant omits `payload` entirely.

```json
{ "type": "SpawnWorkspace",
  "payload": { "session_id": "…", "project_id": "…", "rel_path": null,
               "agent_type": "claude", "args": [] } }
```

Two properties follow from that one choice, and both are load-bearing: the set is **serialisable
by construction**, so moving the bus onto a socket needs no new types; and a message is
**inspectable**, so a log of the bus is a complete account of what happened.

**One enum, not separate request and event types.** One wire format, one dispatch, one place to
look. The direction column is documentation, not a type-level distinction — buying that
distinction would cost the single serialisable channel that makes the process split cheap.

`_docs/tech/transport-contract.md` **owns every variant, payload field and direction**. This file
organises them; it does not replace that table, and a change goes there.

## Picking a family

The test, in the order it is applied:

| It names | Family |
|---|---|
| A pane | pane |
| A project **and a path inside it** | file |
| A project **and a piece of work inside it** (task, step, agent) | work |
| A project **and a search inside it** | search |
| A project alone | project |
| An **agent**, carrying something that agent said | conversation |
| An **absolute path on the host's own filesystem**, no project yet | host browse |
| A **connection** at an external service, or a flow authenticating one | connector |
| A **remote repository** — listing or cloning one | repository |
| A **subject Ubiq wants a sentence for**, carrying no prompt | assist |
| An **account** a harness runs as | account |
| A **saved setup** for a conversation | profile |
| Nothing in Ubiq at all, asking about the machine | command-line |
| Otherwise | session |

## The families

Banners in `messages.rs` split most families into a `UI → host` block and a `host → UI` block.
They appear in the file in this order: pane, session, account, profile, connector, repository,
project, command-line, host browse, file, git, work, conversation, search, assist.

### 1. Pane — the hot path

`TerminalOutput`, `TerminalInput`, `TerminalResize`, `Focus`, `PaneExited`, `PaneError`.

Every variant carries a `PaneId`. `bytes` is a byte sequence and never a string — a partial
multi-byte sequence must survive the trip. Output is **chunked, not lined**: a `TerminalOutput` is
whatever one read returned, and the UI reassembles nothing. A resize is incomplete until the
kernel signals the harness. `PaneError` is per-pane so the interface can put the message where the
user is looking; `Status` and `Error` (session family) are unaddressed, about the application.

**Why `Focus` is a message at all**, when focus looks like pure UI state: the host decides what to
do with output for an unfocused pane, and a detached host with two attached UIs needs to know
which one is typing.

### 2. Session — the control path

`ListSessions`/`SessionList`, `CreateSession`/`SessionCreated`, `AttachToSession`/`SessionAttached`,
`DetachFromSession`, `SpawnWorkspace`/`WorkspaceSpawned`, `CloseWorkspace`, `ListAgentTypes`/
`AgentTypes`, `ListShells`/`ShellList`, `ListStats`/`Stats`, `HostInfo`, `Status`, `Error`.

- **`SpawnWorkspace` names a project, not a folder.** `project_id` is not optional: a pane's
  working directory is the project's folder and the interface never holds the path. A spawn into a
  missing or unreadable folder is refused with `ProjectError` **before a pseudo-terminal exists**.
- **`CloseWorkspace` names a pane** because a pane id *is* a workspace id, and the pane is what the
  user closed. It is the only variant that ends a harness.
- **`HostInfo` is unsolicited**, sent once to each client as it attaches — the only way the status
  bar can say the run is not writing to the usual place. The interface reads no disk.
- **`ListShells` / `ListAgentTypes` are asked repeatedly** and answered from a fresh probe: which
  programs are on the machine can change while a window is open. Both feed
  `SpawnWorkspace.agent_type`; whether the harness library knows that name is what decides between
  a composed agent and a plain program.
- **`ListStats` is the one thing the interface polls** (every two seconds, only while the Control
  screen is up). Everything else is pushed because something happened; memory and uptime change
  when nothing happens, so there is no event behind them.

### 3. Project — the catalogue

`ListProjects`/`ProjectList`, `AddProject`/`ProjectAdded`, `ForgetProject`/`ProjectForgotten`,
`UpdateProject`, `LocateProject`, `OpenedProject`, `RefreshProject` (→ `ProjectChanged`),
`ProjectError`, `GetPreferences`/`Preferences`, `SetPreferences`, `GetSettings`/`Settings`,
`SetSettings`/`SettingsError`.

- **The only broadcast family.** `ProjectAdded`, `ProjectChanged`, `ProjectForgotten` reach every
  attached window, so every picker agrees by construction. `ProjectList`, `Preferences` and
  `Settings` go only to the asker.
- **A project id is stable** across rename, recolour and a move on disk. The path is a uniqueness
  key, never the identity.
- **No message browses a filesystem to find a project** — a folder is chosen in the platform's own
  dialog and arrives as a `path` (`D32`). Once a project exists, browsing inside it is the file
  family's and every path is relative.
- **Settings are not preferences.** View state (theme, dock, open tabs) is
  `GetPreferences`/`SetPreferences` with a `Scope`, opaque and debounced. How the application
  behaves is `GetSettings`/`SetSettings` with a `SettingsLayer` — `Ui` is opaque, `Host` is a
  `HostSettings` the host parses, and a schema this build does not understand is a `SettingsError`
  rather than a discarded default.
- **Four `HostSettings` fields are the host's to write**, not the interface's: `connections`,
  `oauth_apps`, `trusted_certs`, `ai_providers`. A `SetSettings` carrying a stale copy of them is
  discarded, because a background flow can finish while a dialog holds an old copy (`D86`).
  `remote_hosts` is the interface's and rides `SetSettings` whole, because nothing but a person on
  that settings page writes it.
- **`workarea` on every `ProjectSnapshot`** is the one path the interface acts on directly rather
  than over the bus: the host reserves and creates it, never reads inside it, and the interface
  uses the string it was handed rather than composing one from `config_root` — which is what makes
  a host on another machine a change of value rather than a change of code.

### 4. Host browse — finding a root

`BrowseHostDir` → `HostDirListing` or `HostDirError`.

The smallest family, and the one that exists so a picker can find a project root **on a host whose
disk the interface has never seen**. `path` is absolute and resolved against nothing; absent asks
the host for a sensible starting place, because a remote's home directory is a fact only that host
can state. `parent` is `None` only at the filesystem root. Capped at 2,000 entries with a
`truncated` flag. Hidden entries are **marked, not omitted**. `readable` is a hint for greying a
row out, never a promise. A deliberate departure from `D32`'s plan; `D82` records why.

### 5. File — a project and a path

`ProjectTree`, `ReadProjectFile`, `WriteProjectFile`, `DiffProjectFile`, `EditProjectPath`, their
five echoing replies, `ProjectFileError`, and `ProjectFilesChanged`.

- **Every variant names a project *and* a `rel_path`**, because an answer arrives after the click
  that asked for it and the window may have changed project since.
- **The interface holds project-relative paths only** — forward-slashed, no leading slash, no
  `..`, empty for the root. A path escaping the root after symlinks are resolved is refused. This
  is the seam a remote host slots into: a project id and a relative path do not say which machine
  answered.
- **A listing is one directory.** `depth` is clamped and the reply is always a flat list of
  one-level listings, so a depth change never changes a type.
- **A save names the version it read.** `expected` mismatched is a `Conflict` with the file
  untouched. A truncated read has no `version`, so it mechanically cannot be saved.
- **`EditProjectPath` is one message with a `PathOp`** — `Create`, `Move`, `Copy`, `Trash`,
  `Delete` — and the reply echoes the op, because what the interface does next depends on which
  gesture finished. `Trash` and `Delete` are two ops because they are two promises.
- **`ProjectFilesChanged` is the one unasked message here**, naming paths and never contents.

### 6. Git — a project's repository

`ProjectGit`, `RefreshProjectGit`, `ProjectGitLog`, `ProjectGitRefs`; `GitOverview`,
`GitWorkingTree`, `GitLogPage`, `GitRefs`, `GitError`.

Nothing broadcast — a project is open in exactly one window. No absolute path crosses. `overview`
absent is an ordinary answer (not a repository), not a failure. A reply carries a **generation**,
bumped when a full refresh starts, so an older one is discarded. The log is **cursor-paged, not
offset-paged**: an offset would re-walk from `HEAD` and be wrong the moment the tree moved. `lane`
and `merges` are computed host-side so two windows cannot lay out one history differently.

### 7. Work — tasks, sessions, agents

`ListWork`, `CreateTask`, `UpdateTask`, `MoveTask`, `AssignTask`, `DeleteTask`, `AddStep`,
`RenameStep`, `RemoveStep`, `MoveStep`, `ToggleStep`, `AssignAgent`, `SendToAgent`; `WorkList`,
`TaskCreated`, `TaskChanged`, `TaskDeleted`, `AgentChanged`, `WorkError`.

Every variant names a project, because the store is per project. Nothing broadcast, nothing
unsolicited. **A mutation echoes the whole record, not a diff**, which makes the projection
idempotent. **A step is addressed by `StepId`, never by its place in a list.** `WorkList` is one
message rather than three so the board never draws a card naming a session it has not heard of.
`AgentChanged` **boxes its payload** — the only work variant that does.

### 8. Conversation — a live agent

`StartConversation`, `PromptAgent`, `CancelTurn`, `AnswerPermission`, `SetAgentConfig`,
`EndConversation`, `UnloadConversation`, `ResumeConversation`; `ConversationStarted`,
`ConversationUpdate`, `ConversationEnded`, `ConversationUnloaded`, `ConversationError`.

The one family whose vocabulary was borrowed rather than invented — the Agent Client Protocol's
(`D53`), with `_docs/inbox/acp-protocol.md` as the reference.

- **`agent_id` is the multiplexing key**, the role `sessionId` plays in ACP: one bus hosts many
  conversations and every variant names its own. **Minted by the window**, so a surface attaches
  with no round trip and `ConversationStarted` can answer before any harness exists.
- **Deltas, not records.** An absent field in a `ToolCallPatch` means unchanged; `content` and
  `locations` replace rather than append.
- **`seq` is per agent, monotonic, from 1.** A window that sees a gap says so and applies the
  update anyway — half a transcript is worth more than none.
- **The host is the only writer.** The composer appends nothing; the user's line appears as a
  `ConvUpdate::UserChunk` echoed back, which is what the harness actually received.
- **`UnloadConversation` kills the harness and keeps the conversation**; only `EndConversation`
  discards what was said. A resumed conversation picks the sequence up rather than restarting it.
- **`ConversationUpdate` boxes its payload**, the second variant in the set to do so.

`ubiq-agents`'s `reference/wire.md` covers every `ConvUpdate` variant in detail.

### 9. Search — one live search per project

`SearchProject`, `CancelSearch`; `SearchMatches`, `SearchProgress`, `SearchFinished`,
`SearchError`. The UI mints the `SearchId` before the first request hits the wire. A new
`SearchProject` for a project already searching supersedes it. Batches flush on 64 files or 512
hits, no timer; `SearchProgress` every 100 files. Ceilings: `HITS_PER_FILE` 100,
`FILES_WITH_HITS` 1 000, `TOTAL_HITS` 10 000.

### 10. Account — the identities a harness runs as

`ListAccounts`/`Accounts`, `BeginHarnessLogin`, `HarnessLoginStarted`, `HarnessLoginCaptured`,
`HarnessLoginFailed`, `HarnessLoginLink`, `CheckHarnessLogin`/`HarnessLoginStatus`,
`RenameAccount`, `DeleteAccount`, `DeleteHarnessLogin`, `AccountError`.

**References only, never material.** No credential and no path crosses. *The log sink listens to
the same bus, so a secret here would be a secret in a log the user might paste into an issue.*
A login runs in a real pane that belongs to no project. **The outcome is decided by the
credential's mtime, not the exit code** — a harness can exit cleanly having done nothing.
**Creating an account is logging one in**; there is no `AddAccount`. Status is what the stored
credential says about itself, so `Valid` means "not expired", not "will work".

### 11. Profile — saved setups

`ListProfiles`/`Profiles`, `SaveProfile`. Three messages, sharing `AccountError` because profiles
are stored beside accounts. **There is no delete** — a stale profile costs a row in a list, not a
credential on disk. A profile named on `StartConversation` **seeds the picker, it does not bypass
it**: the host copies its model and mode into the pending conversation's picks, because a flag
outranks a profile inside the library's `resolve`.

### 12. Command-line — the `ubiq` shortcut

`CliShortcut { action }` → `CliShortcutState`. **The request carries no directory**: which
directory belongs on `PATH` is a fact about the machine, and the host is the half allowed to look,
so every path travels host → UI. One answer serves `Query`, `Install` and `Remove`.

### 13. Connector — identities at external services

`ListConnections`/`Connections`, `BeginConnect`/`ConnectPending`/`ConnectCaptured`/`ConnectFailed`,
`CancelConnect`, `SubmitConnectSecret`, `RenameConnection`, `DeleteConnection`,
`CheckConnection`/`ConnectionStatus`, `ConfirmCertificate`/`TrustCertificate`,
`ForgetCertificate`, `SaveOauthApp`, `DeleteOauthApp`, `SetAppSecret`, `ClearAppSecret`,
`ConnectorError`.

**Two variants carry material** — `SubmitConnectSecret` and `SetAppSecret` — and the rule is the
*type*, not the list: material crosses only in a `Secret` (`D65`). **Creating a connection is
completing a flow**; an abandoned one leaves nothing. **A pin is answered, never assumed**:
`TrustCertificate` must carry the same `sha256` the host offered. `OAUTH_REDIRECT` and
`OAUTH_REDIRECT_PORT` are constants on the contract because the host binds the port and the
interface shows the URL — a second spelling would drift.

### 14. Repository — listing and cloning

`ListRepos`/`Repos`, `ListRepoBranches`/`RepoBranches`, `CloneRepo`/`ClonePending`/`CloneFailed`,
`CancelClone`, `RepoError`. **There is deliberately no clone-success message**: a finished clone
registers the project, so the already-broadcast `ProjectAdded` is the success signal. A clone
reports a **stage, not a percentage**. A cancel and a failure are the same outcome on disk — the
partial destination is removed, so nothing half-cloned is registered.

### 15. Assist — a sentence for a subject

`GetAssist`/`Assist`, `Suggest`/`SuggestChunk`/`Suggestion`/`SuggestError`, `CancelSuggest`,
`GetAiProviders`/`AiProviders`, `AddAiProvider`, `UpdateAiProvider`, `ForgetAiProvider`,
`ListAiModels`/`AiModels`, `AiProviderError`.

**No variant carries prompt text** (`D83`). A `SuggestSubject` carries ids only; every prompt
string and truncation budget lives in `crates/ubiq-host/src/assist/subject.rs`. *A family that
accepted prompt text would be a generic model console whatever it was called.* **Availability is
asked, never inferred** — `GetAssist` is the only way the interface learns whether a suggestion
can be produced. A suggestion **streams**, and an interface may ignore that it does: every chunk
concatenated is the final message's text (`D87`). **`Suggest` is the one variant whose project is
inside its payload's payload** — `Message::project_id()` has a dedicated arm reaching through the
subject.

## The records

Thirty-five records travel inside payloads; the transport contract lists every field. The
distinctions worth remembering:

- **The record is what the store holds; the snapshot is what crosses the bus.** `ProjectRecord` is
  written down; `ProjectSnapshot` flattens it and adds `health`, `open_panes`, `workarea` and
  `ephemeral` — facts only the half owning the filesystem can state, and which must never be
  written down and believed at the next boot.
- **`WorkspaceInfo` carries no handle** to a process, a writer or a pseudo-terminal. A record that
  crosses the bus must survive serialisation.
- **`Scope`'s and `SettingsLayer::Ui`'s `value` is opaque** — a string the host writes down and
  hands back and never parses, on the discipline that keeps terminal bytes uninterpreted.
- **`PermissionKind` and `ConfigCategory` are display hints.** They decide how a control *reads*
  and never what an `option_id` means.

## The ids — `crates/ubiq-proto/src/ids.rs`

Fourteen kinds, each a `#[serde(transparent)]` newtype over a `Ulid`, all minted by the
`ulid_id!` macro so none grows its own surface: `generate()`, `created_at()`, `as_ulid()`,
`Display` (the bare 26 characters), a named `Debug`, `FromStr`. **There is no `new()` and no
`Default`** — an id that was not minted is a nil id that looks real.

| Id | Minted by | Note |
|---|---|---|
| `PaneId` | host | One pane, and the byte stream it is |
| `SessionId` | interface | Ubiq's session: panes plus a folder |
| `WorkspaceId` | host | Also `work::AgentId`, an alias — one thing until a workspace outlives its pane |
| `TaskId`, `StepId` | host | Written down, so they survive a restart |
| `ProjectId` | host | Stable across rename, recolour and a move on disk |
| `SearchId`, `ConnectId`, `CloneId`, `RepoQueryId`, `SuggestId` | interface | Stale-answer discipline: a reply naming an id nobody holds is discarded |
| `ConnectionId`, `OauthAppId`, `AiProviderId` | host | Exist only once something is written — an abandoned flow leaves no id |

**Why newtypes**: a pane id and a session id are both 128 bits, and nothing but care would stop
one being passed where the other belongs — the compiler does it here instead.

**Why ULID, not UUID**: it sorts by creation time, prints as 26 case-insensitive characters with
no hyphens (a readable directory name), and carries its own timestamp. All of them come from **one
process-wide monotonic generator** (`static IDS: Mutex<ulid::Generator>`) — two ids minted by a
bare `Ulid::new()` in the same millisecond sort arbitrarily, and sorting by creation time is most
of why a ULID is worth having. Contention is nil: every call site is on a control path, never on
the byte stream.

`OauthAppId::derived(seed)` is the one exception to minting — FNV-1a over provider+origin, spelled
out rather than taken from `std::hash` whose output is not promised stable between compiler
releases, because the host re-reads the settings file on every question and a minted fill would
answer a different id each time.

`gpui::WindowId` is the framework's and is not one of these. Neither is `bus::ClientId` nor
`app::hosts::HostId` — those never serialise.

## The log sink — `crates/ubiq-proto/src/log.rs`

One process-wide ring the whole application writes to and every console reads. **Collection goes
through `tracing` and nothing else**: no registration, no sink to thread through a signature, so a
crate that has never heard of Ubiq is collected on the same terms as Ubiq's own modules.

`install()` puts two layers behind one `EnvFilter`: the ring, and a plain writer on standard error
so a terminal run reports without a console — which is a headless `--serve` run's only report. A
second call is a no-op by design.

- **`CAPACITY` is 5 000**, and the count of what fell off the front is kept and reported: *a
  console that silently loses its beginning is a console that lies.*
- **`DEFAULT_FILTER`** when `RUST_LOG` says nothing:
  `ubiq=debug,ubiq_app=debug,ubiq_host=debug,ubiq_proto=debug,agent_manager=debug,gpui_terminal=debug,warn`.
- **`Subsystem` is derived from the emitting module's target, not declared.** `Subsystem::ALL` has
  **seven**: `Ui`, `Coordinator`, `Pty`, `Harness`, `Mcp`, `Search`, `External`. The mapping is
  `Subsystem::of` — more specific prefixes first, because `ubiq_host::pty` is also `ubiq_host`,
  and the bare `ubiq` arm is last because every crate here starts with it. `ubiq_proto::bus` maps
  to `Coordinator`. Nothing falls through: an unrecognised target is `External`.
- **`LogLevel` is ordered** (`Trace` < `Debug` < `Info` < `Warn` < `Error`), so the console's
  filter is a **floor** rather than a set. `Filter { subsystem: Option<Subsystem>, min_level }`.
- **Records travel one way** — a producer writes and never reads, the console reads and writes
  nothing a producer can see, and nothing in a record is a pane's state, a path or a descriptor.
  That is what makes a sink shared by both halves something other than a way around the bus
  (`D24`). Its cost: diagnostics die with the process, and a detached coordinator does not carry
  them across for free.

Adding a subsystem is an arm in `Subsystem::of`, an entry in `ALL`, a `label()`, and a row in
`_docs/features/logs.md`.
