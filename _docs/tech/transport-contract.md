---
id: tech-transport
title: Transport contract
kind: tech
status: draft
summary: The complete message set the UI and the coordinator exchange — the pane, session, project, file, git, work, conversation, search and account families, the framing rules, and the procedure for adding a variant.
read_when: you are adding, changing or removing a message, or wiring either half to the bus
updated: 2026-09-03
verified: 2026-09-03
code_anchors: [crates/ubiq-proto/src/messages.rs, crates/ubiq-proto/src/ids.rs, crates/ubiq-proto/src/projects.rs, crates/ubiq-proto/src/settings.rs, crates/ubiq-proto/src/files.rs, crates/ubiq-proto/src/git.rs, crates/ubiq-proto/src/work.rs, crates/ubiq-proto/src/conversation.rs]
depends_on: [tech-architecture]
review_cycle: monthly
---

# Transport contract

The contract is the one piece of Ubiq that is expensive to change, because both halves are written
against it and every future topology preserves it. `crates/ubiq-proto/src/messages.rs` is its home; the
rules that make the split worth having are in [`architecture.md`](./architecture.md).

**This document owns every message fact.** Variant names, payload fields, direction and response
behaviour are stated here and linked from everywhere else.

## The shape of a message

One tagged enum, serialised with the variant name in `type` and the body in `payload`. Variants
without a body omit `payload` entirely.

```json
{ "type": "SpawnWorkspace",
  "payload": { "session_id": "…", "project_id": "…", "rel_path": null,
               "agent_type": "claude", "args": [] } }
```

Every id in the contract is a ULID behind a per-kind newtype — `PaneId`, `SessionId`,
`WorkspaceId`, `ProjectId`, `TaskId`, `StepId` in `crates/ubiq-proto/src/ids.rs` — so a pane's id
cannot be passed where a session's belongs. Each serialises as its bare 26-character string, and all
six come from one monotonic generator, because sorting by creation time is most of why a ULID is
worth having. `WorkspaceId` is also an agent's id: the work family calls it `AgentId`, an alias of
the same type, because a workspace and an agent are one thing until a workspace outlives its pane.
`gpui::WindowId` is the framework's and is not one of these.

In Rust that is `#[serde(tag = "type", content = "payload")]` over a single enum. Two properties
follow from that choice and both are load-bearing: the message set is **serialisable by
construction**, so moving the bus onto a socket needs no new types; and a message is **inspectable**,
so a log of the bus is a complete account of what happened.

## The pane family

The hot path. Every variant carries a pane ID, and terminal bytes are opaque — neither half parses
them.

| Message | Direction | Payload | Meaning |
|---|---|---|---|
| `TerminalOutput` | coordinator → UI | `pane_id`, `bytes` | Raw pseudo-terminal output. Continuous while the harness runs. Written straight into the pane's emulator |
| `TerminalInput` | UI → coordinator | `pane_id`, `bytes` | Raw keystrokes from the focused pane. No response; effects arrive as `TerminalOutput` |
| `TerminalResize` | UI → coordinator | `pane_id`, `cols`, `rows` | New geometry. The coordinator sets the pseudo-terminal size; the kernel signals the harness |
| `Focus` | UI → coordinator | `pane_id` | The pane that receives input. Exactly one at a time |
| `PaneExited` | coordinator → UI | `pane_id`, `code` | The harness ended |
| `PaneError` | coordinator → UI | `pane_id`, `error` | The pane could not be spawned or its stream failed |

`bytes` is a byte sequence, never a string. Harness output is not guaranteed to be valid UTF-8 at a
message boundary, and a partial multi-byte sequence must survive the trip intact.

## The session family

The control path. Lower volume, request-and-response.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ListSessions` | UI → coordinator | — | `SessionList` |
| `CreateSession` | UI → coordinator | `name`, `agent_type`, `home_folder?` | `SessionCreated` |
| `AttachToSession` | UI → coordinator | `session_id` | `SessionAttached` |
| `DetachFromSession` | UI → coordinator | `session_id` | — |
| `SpawnWorkspace` | UI → coordinator | `session_id`, `project_id`, `rel_path?`, `agent_type?`, `args` | `WorkspaceSpawned` or `ProjectError` |
| `CloseWorkspace` | UI → coordinator | `pane_id` | — |
| `ListAgentTypes` | UI → coordinator | — | `AgentTypes` |
| `SessionList` | coordinator → UI | `sessions[]` | — |
| `SessionCreated` | coordinator → UI | `session` | — |
| `SessionAttached` | coordinator → UI | `session`, `workspaces[]` | — |
| `WorkspaceSpawned` | coordinator → UI | `workspace` | — |
| `AgentTypes` | coordinator → UI | `agent_types[]` | — |
| `Status` | coordinator → UI | `message` | — |
| `Error` | coordinator → UI | `message` | — |

An optional field marked `?` falls back to a default: `home_folder` to the session home, `rel_path`
to the project's own root, and `agent_type` to the agent type the session starts when it is told
nothing. `args` is the argument list the harness is launched with, empty for a plain start.

**`SpawnWorkspace` names a project, not a folder.** `project_id` is not optional, because a pane's
working directory is the project's folder and nothing else: the host resolves it from the record and
the interface never holds the path. A spawn into a project whose folder is missing, is not a
directory or cannot be read is refused with a `ProjectError` **before a pseudo-terminal exists**, and
the fresh snapshot is broadcast so every picker marks the row from the probe that just happened. A
`rel_path` that escapes the root is refused the same way.

`CloseWorkspace` names a pane rather than a workspace ID because the two are the same ID, and the
pane is what the user closed. It kills and reaps the harness; it is the only variant that ends one.

`Status` and `Error` are unaddressed — they concern the application, not a pane. Anything that
concerns one pane uses `PaneError`, so the UI can put the message where the user is looking.

## The project family

The third family. Every variant names a project by id, and a project's id is stable across rename,
recolour and a move on disk.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ListProjects` | UI → host | — | `ProjectList` |
| `AddProject` | UI → host | `path`, `name?`, `colour?` | `ProjectAdded` or `ProjectError` |
| `ForgetProject` | UI → host | `project_id` | `ProjectForgotten` |
| `UpdateProject` | UI → host | `project_id`, `name?`, `colour?` | `ProjectChanged` |
| `LocateProject` | UI → host | `project_id`, `path` | `ProjectChanged` or `ProjectError` |
| `OpenedProject` | UI → host | `project_id` | `ProjectChanged` |
| `RefreshProject` | UI → host | `project_id` | `ProjectChanged` |
| `GetPreferences` | UI → host | `scope` | `Preferences` |
| `SetPreferences` | UI → host | `scope`, `value` | — |
| `GetSettings` | UI → host | `layer` | `Settings` |
| `SetSettings` | UI → host | `layer`, `value` | — (Ui) or `SettingsError` (Host) |
| `ProjectList` | host → UI | `projects[]` | — |
| `ProjectAdded` | host → UI | `project` | — |
| `ProjectChanged` | host → UI | `project` | — |
| `ProjectForgotten` | host → UI | `project_id` | — |
| `ProjectError` | host → UI | `project_id?`, `error` | — |
| `Preferences` | host → UI | `scope`, `value?` | — |
| `Settings` | host → UI | `layer`, `value?` | — |
| `SettingsError` | host → UI | `layer`, `error` | — |
| `HostInfo` | host → UI | `config_root`, `is_default` | — |
| `ListShells` | UI → host | — | `ShellList` |
| `ShellList` | host → UI | `shells` | — |

**`ProjectChanged`, `ProjectAdded` and `ProjectForgotten` are broadcast** to every attached window,
so every picker agrees by construction rather than by each window asking again. A `ProjectList`, a
`Preferences` and a `Settings` go only to the window that asked.

**Settings are not preferences.** View state — theme, dock, open tabs — is `GetPreferences` /
`SetPreferences` with a `Scope`, opaque, debounced. How the application behaves is
`GetSettings` / `SetSettings` with a `SettingsLayer`. `Ui` is opaque: the host writes the string
and never looks inside, a failed write is a log line, and a blob whose schema this build does not
know is discarded. `Host` is parsed: a blob the host cannot read answers `SettingsError` and a
corrupt file is preserved, like the catalogue. Harness definitions are neither layer — they belong
to agent-manager.

A `SettingsLayer` is `Ui` or `Host`. The Host record on the wire is JSON with a `schema` field;
on disk it is TOML of that same record. The Ui record's schema lives in the interface.

**`LocateProject` is separate from `UpdateProject`** because the two differ in kind. A rename or a
recolour is display only: it touches no filesystem and cannot fail. Locate changes truth — it
canonicalises, re-probes the folder, and is refused when another record already owns it.

**No message browses a filesystem to find a project.** A project's folder is chosen in the platform's
own dialog and reaches the host as the `path` of an `AddProject` or a `LocateProject` — which makes
the interface's filesystem the one being browsed, and is the one place the two halves are assumed to
share a machine (`D32`). Once a project exists, browsing *inside* it is the file family's, and every
path in it is relative.

**`HostInfo` is unsolicited**, sent once to each window as it attaches. The interface reads no
disk, so it is the only way the status bar can say that a run is not writing to the usual place.

**`ListShells` and `ListAgentTypes` are asked repeatedly and answered from a fresh probe.** Which
programs are on the machine is another fact the interface cannot read, and unlike a config root it
can change while a window is open, so both are requests rather than something stamped at attach —
the new-pane menu asks every time it opens. A pane is then started with the `program` a `ShellInfo`
carried or the `id` an `AgentTypeInfo` carried, both on `SpawnWorkspace`'s existing `agent_type`:
one field, and the coordinator's answer to whether the harness library knows that name is what
decides whether the pane is a composed agent or a program. An `AgentTypeInfo` whose `available` is
false is offered and not pickable, so the interface never has to decide what a missing binary means.

**`AddProject` never creates a folder.** A path that does not exist is a `ProjectError`. A folder
already in the catalogue answers with the project that is there, so no duplicate appears.

**`ForgetProject` is not deleting.** It removes the record and the project's own directory in
Ubiq's config, and touches nothing inside the project's folder.

**Every `ProjectSnapshot` carries a `workarea`** — an absolute path to the directory that project's
*interface* may keep its own files in. It travels on the snapshot, so it arrives on `ProjectList`,
`ProjectAdded` and `ProjectChanged` with everything else about a project, and four things hold of
it.

- **The host reserves the name and creates it, and never reads or writes inside it.** What is in
  there is the interface's business alone. The host makes the directory before it names it, so the
  interface is told a path rather than a maybe.
- **It is disposable.** Deleting it loses a cache and nothing else. Nothing the user would miss goes
  there — that is what a `Scope::Project` preference blob is for, and that still crosses the bus.
- **It is not the project's folder.** Nothing the interface writes there lands in the user's
  repository, which is the whole reason it sits under Ubiq's config root rather than beside the
  user's code.
- **The interface never composes it.** It uses the string it was handed and never builds one out of
  `HostInfo.config_root`, which is what makes a host on another machine a change of value rather
  than a change of code.

This is the one path in the contract the interface acts on directly rather than over the bus, and
`architecture.md` states the rule that keeps it honest.

## The file family

The fourth family. Every variant names a project by id **and a path by `rel_path`**, because an
answer arrives after the click that asked for it and the window may have changed project since.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ProjectTree` | UI → host | `project_id`, `rel_path`, `depth` | `ProjectTreeListing` or `ProjectFileError` |
| `ReadProjectFile` | UI → host | `project_id`, `rel_path`, `max_bytes?` | `ProjectFileContents` or `ProjectFileError` |
| `WriteProjectFile` | UI → host | `project_id`, `rel_path`, `bytes`, `expected?` | `ProjectFileWritten` or `ProjectFileError` |
| `DiffProjectFile` | UI → host | `project_id`, `rel_path`, `base` | `ProjectFileDiffed` or `ProjectFileError` |
| `ProjectTreeListing` | host → UI | `project_id`, `rel_path`, `listings[]` | — |
| `ProjectFileContents` | host → UI | `project_id`, `rel_path`, `contents` | — |
| `ProjectFileWritten` | host → UI | `project_id`, `rel_path`, `version` | — |
| `ProjectFileDiffed` | host → UI | `project_id`, `rel_path`, `diff` | — |
| `ProjectFileError` | host → UI | `project_id`, `rel_path`, `error` | — |

Every one of these answers only the window that asked. Nothing in this family is broadcast: what one
window is looking at is not a fact about the catalogue.

**The interface holds project-relative paths only.** A `rel_path` is forward-slashed, has no leading
slash and no `..`, and is empty for the project's root. The host resolves it against the record's
root, and one that escapes the root after every symlink is resolved is refused. This is the
file-level form of the rule that the UI never assumes the pseudo-terminal is local, and it is the
seam a remote drone slots into: a project id and a relative path do not say which machine answered.

**A listing is one directory.** `depth` asks the host to descend, and it is clamped; the reply is a
flat list of one-level listings whatever was asked for, so a depth change never changes a type. The
interface asks for `depth: 1` when a folder is expanded, which is why a repository's `node_modules`
costs one row rather than a walk, and for more than one in the background when a project opens, so
the window's file cache is full enough to search without waiting. `WALK_SKIP` in `files.rs` is the
names a depth walk does not descend into; an explicit listing of one of them is still answered in
full. `LIST_HIDE` is the leaf names omitted from every listing, including an explicit one — junk
files that are never user content, `.DS_Store` today. A directory over the host's entry ceiling
comes back `truncated` rather than quietly short.

**Contents cross as bytes**, on the same discipline that keeps terminal bytes uninterpreted: a read
cut short at the ceiling can sever a multi-byte sequence, a binary file has no text at all, and which
encoding to draw is the interface's decision. `is_binary` is the host reporting a NUL byte near the
start, not a verdict on encoding.

**A save names the version it read.** `expected` is the `FileVersion` that came back with the
contents, and a mismatch is refused as `Conflict` with the file untouched — which is what stops a
save landing on a change an agent made in a pane. `expected` absent means creating a file, and is
refused if anything is already there. No folder is ever created, the mirror of `AddProject` never
creating one, and the write is atomic and keeps the file's permissions.

**A truncated read cannot be saved**, and mechanically rather than by the interface remembering:
`FileContents.version` is absent when `truncated`, so there is no version to name, and a write naming
none is refused on a file that exists.

**A diff is the file family's because it names a path.** `DiffProjectFile` compares the working
tree with a base — `Head` for the commit that is checked out, `Index` for what has not been staged
— and the host computes the hunks, so no diff library reaches the interface, on the discipline that
keeps a VT parser out of the host. A `FileDiff` carries rows with the line numbers already worked
out, because a gutter that counts them itself gets it wrong the first time a hunk is cut short. A
file with no change against its base answers with no hunks; one the host would not diff comes back
`binary`, and one it stopped at a ceiling comes back `truncated`, the way a listing and a read do.
There is no new error variant: a failure is a `ProjectFileError`, and **a project with no version
control in it is `Refused`** rather than a missing file or an empty diff.

**`ProjectFileError` is per path**, not per project, for the reason `PaneError` is per pane: the
interface can only mark the row or the tab the user is looking at if the message says which one. Its
`error` is a `FileError` — `Refused`, `Missing`, `WrongKind`, `Denied`, `Conflict` or `Failed` — and
each arm is a different thing for the interface to do rather than a sentence to match on. **The host
does not re-probe a project's health for a file failure**; a `Missing` or a `Denied` is the
interface's cue to send `RefreshProject`, which is the project family's job.

## The git family

The fifth family. Every variant names a project, because the interface holds no repository identity
of its own — a repository is a fact about a project, discovered by the host. Nothing in this family
is broadcast: a project is open in exactly one window, so the window that asked is the only one
drawing it. No absolute path crosses; a repository root above the project, or a prefix inside one,
is a relative string.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ProjectGit` | UI → host | `project_id` | `GitOverview` or `GitError` |
| `RefreshProjectGit` | UI → host | `project_id`, `full` | `GitOverview`, and `GitWorkingTree` when `full`; or `GitError` |
| `GitOverview` | host → UI | `project_id`, `overview?` | — |
| `GitWorkingTree` | host → UI | `project_id`, `generation`, `entries[]`, `rollups[]`, `truncated` | — |
| `GitError` | host → UI | `project_id`, `error` | — |

**`overview` absent is an ordinary answer**, not a failure: the project is not in a repository, and
the interface draws no branch and no badges. `GitError` is for a repository that exists and could
not be read — `NotFound`, `Corrupt`, `Denied`, `Interrupted` or `Failed`.

**The overview is cheap.** It is refs and a handful of files in the git directory: `HEAD` as a
branch name, a detached short id or an unborn name; the upstream and ahead/behind when there is
one, capped at 99; an in-progress operation; whether the repository is bare. Working-tree counts
ride with a full refresh, and are absent rather than zero on a bare or unborn repository, and
absent until a walk has run.

**The working-tree map carries only paths that have something to say.** A row not in the map is
clean, once a map has arrived. An entry is a pair — how the index differs from HEAD, how the
worktree differs from the index — plus whether the path is conflicted or ignored. The interface's
single status is a projection of that pair, stated on `GitEntry::mark` so two windows cannot
disagree: conflicted, else worktree untracked, else a worktree change, else an index change, else
ignored. A file both staged and modified draws as modified. Directories get a rollup of the
children's worst case, sent by the host because the explorer expands one level at a time and cannot
derive a folder's badge from children it has not asked for. Past the entry ceiling the map is
`truncated`. `.DS_Store` is omitted from the map the way it is omitted from a listing.

**A reply carries a generation**, bumped when a full refresh starts. The interface discards an older
one. A second full refresh for a project still walking replaces the queued one rather than lining
up behind it.

## The work family

The sixth family. **Every variant names a project by id**, because the work belongs to a project:
its tasks are written down under that project's own directory in Ubiq's config root, and its
sessions and agents are minted per project. A task id alone would not say which store to write.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ListWork` | UI → host | `project_id` | `WorkList` or `WorkError` |
| `CreateTask` | UI → host | `project_id`, `title`, `session?` | `TaskCreated` or `WorkError` |
| `UpdateTask` | UI → host | `project_id`, `task_id`, `title?`, `description?`, `priority?`, `shape?` | `TaskChanged` or `WorkError` |
| `MoveTask` | UI → host | `project_id`, `task_id`, `status` | `TaskChanged` or `WorkError` |
| `AssignTask` | UI → host | `project_id`, `task_id`, `session?` | `TaskChanged` or `WorkError` |
| `DeleteTask` | UI → host | `project_id`, `task_id` | `TaskDeleted` or `WorkError` |
| `AddStep` | UI → host | `project_id`, `task_id`, `title` | `TaskChanged` or `WorkError` |
| `RenameStep` | UI → host | `project_id`, `task_id`, `step_id`, `title` | `TaskChanged` or `WorkError` |
| `RemoveStep` | UI → host | `project_id`, `task_id`, `step_id` | `TaskChanged` or `WorkError` |
| `MoveStep` | UI → host | `project_id`, `task_id`, `step_id`, `to` | `TaskChanged` or `WorkError` |
| `ToggleStep` | UI → host | `project_id`, `task_id`, `step_id` | `TaskChanged` or `WorkError` |
| `AssignAgent` | UI → host | `project_id`, `agent_id`, `task_id?` | `AgentChanged` or `WorkError` |
| `SendToAgent` | UI → host | `project_id`, `agent_id`, `text` | `AgentChanged` or `WorkError` |
| `WorkList` | host → UI | `project_id`, `sessions[]`, `agents[]`, `tasks[]` | — |
| `TaskCreated` | host → UI | `project_id`, `task` | — |
| `TaskChanged` | host → UI | `project_id`, `task` | — |
| `TaskDeleted` | host → UI | `project_id`, `task_id` | — |
| `AgentChanged` | host → UI | `project_id`, `agent` | — |
| `WorkError` | host → UI | `project_id`, `task_id?`, `error` | — |

**Nothing in this family is broadcast.** Every reply goes to the window that asked, on the file
family's rule for the file family's reason: a project is open in exactly one window at a time, so
the window that asked is the only one drawing that project's work, and what one window is looking at
is not a fact about the catalogue.

**`project_id` is echoed on every reply**, and `task_id` on a `TaskDeleted`, because an answer
arrives after the click that asked for it and the window may have moved on.

**A move and an assignment are their own messages rather than fields on `UpdateTask`**, by the same
test `D31` applies to the project family. `UpdateTask` is display only: it renames, re-describes,
reprioritises and reshapes, touches nothing outside the record, and can be refused for nothing but a
task that is not there. `MoveTask` carries the one field the board reserves for a drag — a column is
a stage and a card only ever changes column, which [`workbench.md`](../features/workbench.md)
prescribes — so folding `status` into an update would offer a second way to do the one thing the drag
exists for. `AssignTask` names another entity and is
refused for a session the host does not hold, which makes it fallible where an update is not; it
also spares the wire an `Option<Option<SessionId>>` inside an update, which is a type nobody should
have to read.

**`WorkList` is one message, not three.** Sessions, agents and tasks arrive in the same frame,
because two round trips would let the board draw a card naming a session it has not heard of.

**A mutation echoes the whole record, not a diff** — `ProjectChanged`'s discipline, and what makes
the interface's projection idempotent: applying a record replaces on id, so the same answer twice
changes nothing.

**`TaskCreated` is separate from `TaskChanged`** because the asker cannot know an id it did not mint,
and the board selects the card it just made. It is the shape `ProjectAdded` has.

**A step is addressed by a `StepId`, never by its place in the list.** Two clicks in one frame — a
remove and a tick — would otherwise arrive as two indices into two different lists, and the second
would land on the wrong step. `MoveStep` reorders by naming the step and the place it should end up
in; its `to` is clamped by the host, because a list that shortened under a drag is not an error the
user can do anything about.

**`ToggleStep` carries no target state.** Unticking lands on idle, because nothing can know what a
step's owner would go back to doing — a rule about the work, and so the host's to keep rather than a
value the interface works out and sends.

**`AssignAgent` and `SendToAgent` change the host's mock agents.** Which task an agent serves is the
host's fact even while the agent is invented; where its card sits is the interface's and never
crosses. `SendToAgent` answers with the agent record carrying one more `Turn`, and **nothing answers
the thread**: a fabricated reply is the one thing a screen with no live agent must not draw.

**A `DeleteTask` can answer with more than a `TaskDeleted`.** Every agent pointing at the task is
taken off it and reported as an `AgentChanged`, because a card pointing at a task that has gone would
be drawn in no container and counted in one, and the repair is the interface's to hear rather than to
work out.

**Nothing in this family is unsolicited.** The host never pushes a change nobody asked for, so there
is no variant for an agent making progress of its own — a gap in [`../backlog.md`](../backlog.md).

**`WorkError.error` is a sentence rather than an enum**, and deliberately the opposite of `D34`. An
enum earns its keep when each arm is a different thing for the interface to do; every failure here —
no such project, no such task, no such step, no such session, a store that will not write — comes
down to saying so once, where the user is looking. `task_id` is present when one task is at fault and
absent when a project's work as a whole is.

**`AgentChanged` boxes its payload, and is the only variant in the set that does.** A `WorkAgent` is
272 bytes, the widest record in the contract by some way, and an enum is as wide as its widest
variant — so an unboxed one makes every message on the bus that wide, including the terminal chunks
on the hot path. `Message` is 192 bytes with the box and 288 without it. The wire form is the same
either way, because a `Box` serialises as what is inside it.

## The conversation family

The seventh family, and the only one whose vocabulary was borrowed rather than invented. **Every
variant names an agent**, because an agent is what a conversation belongs to and because that name
is what multiplexes several of them down one channel.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `StartConversation` | UI → host | `agent_id`, `project_id`, `session_id`, `rel_path?`, `agent_type`, `account?`, `name?` | `ConversationStarted` or `ConversationError` |
| `PromptAgent` | UI → host | `agent_id`, `text` | — |
| `CancelTurn` | UI → host | `agent_id` | — |
| `AnswerPermission` | UI → host | `agent_id`, `request_id`, `option_id` | — |
| `SetAgentConfig` | UI → host | `agent_id`, `config_id`, `value` | — |
| `EndConversation` | UI → host | `agent_id` | `ConversationEnded` |
| `ConversationStarted` | host → UI | `project_id`, `agent`, `session`, `accepts_input` | — |
| `ConversationUpdate` | host → UI | `agent_id`, `seq`, `update` | — |
| `ConversationEnded` | host → UI | `agent_id`, `stop_reason` | — |
| `ConversationError` | host → UI | `agent_id`, `error` | — |

**The vocabulary is the Agent Client Protocol's; the transport is the bus.** `D53` states why, and
[`../../refs/acp-protocol.md`](../../refs/acp-protocol.md) is the wire reference every name here
comes from. What that buys is that the library's own event model, this family and the mapper between
them are one vocabulary rather than three, and that a harness which speaks ACP natively is read
rather than translated.

**A conversation is a workspace's other face.** `SpawnWorkspace` makes a terminal one and
`StartConversation` makes a conversation one; a harness cannot be both at once, because a child's
standard output is either a pseudo-terminal or a pipe. They are two messages rather than one with a
flag because they answer with different things: a pane has a size and a conversation does not, and a
`WorkspaceInfo` full of geometry nobody set would be a record that lies.

**`agent_id` is the multiplexing key**, and it is the same role a `sessionId` plays in ACP — where
one connection hosts many sessions and every session-scoped message names its own. Here one bus
hosts many conversations and every variant names its own. Two agents streaming at once need no
second channel, no fan-out and no per-agent subscription.

**`agent_id` is minted by the window, not the host** — the same precedent `SessionId` already sets
— because a conversation starts *pending*: `ConversationStarted` answers at once, before any
harness exists, so the window can draw the record and a loader while the host discovers that
harness's models in the background and reports them as a single `ConvUpdate::ConfigOptions`
addressed to that `agent_id` (always the first update it receives, at `seq: 1`, whether or not
discovery actually found anything). Only the window's first `PromptAgent` launches the harness,
carrying whatever `SetAgentConfig` last chose on `RunFlags.model` — a model reaches a harness only
as a launch flag, so changing the pick before that first prompt costs nothing. See
`_docs/wip/agent-setup.md`'s P3.

**`name` is what the user typed, not what the harness is called.** It sets `WorkAgent.name` —
the sidebar row, the column header, the chat panel row all draw that field, never `harness`
directly — and an absent `name` falls back host-side to the harness's own label, the way every
conversation's name worked before this field existed. Chosen once, at the naming prompt between
picking a harness and the first turn, and never after: renaming mid-conversation is not this
field's job.

**`seq` is per agent, monotonic, and starts at one.** Order is promised per agent and not across
them, on exactly the terms the pane family already sets for terminal output. A window that receives
a `seq` which does not follow the last one has lost a message; it says so and applies the update
anyway, because half a transcript is worth more than none.

**Deltas, not records.** `AgentChanged` re-sends a whole `WorkAgent` and that is right for a record
that changes rarely; a token stream cannot. So an update carries one thing — a chunk of prose, a
tool call, a patch to one already announced — and the window folds it in. **An absent field in a
patch means unchanged**, and `content` and `locations` replace rather than append; a window that
applies them the other way silently loses half of an edit.

**The host is the only writer.** The composer appends nothing when it sends: the user's own line
appears when the harness echoes it back as a `ConvUpdate::UserChunk`, which is what the harness
actually received rather than what was typed at it. That is the same rule
[`../features/workbench.md`](../features/workbench.md) states, applied to a conversation that now
has something behind it.

**Coalescing is the window's.** The host forwards what the harness said, on an unbounded mailbox, so
a window that cannot draw two hundred chunks a second is a window that has fallen behind rather than
a harness that has stopped.

**What a window derives rather than asks for.** A conversation's activity badge, its run pill and
its context ring are read off the stream it already holds. Those are renderings of a delta, not
content, and asking the host for them would be a round trip per token.

**`ConversationUpdate` boxes its payload**, and is the second variant in the set to do so, for
`AgentChanged`'s reason: an enum is as wide as its widest variant, and the terminal chunks on the hot
path share it.

**The session travels with the agent.** The sidebar lists agents *under* a session, and a window's
own session is not one of the work's, so an agent whose session nothing names is an agent nothing
draws. The host holds the sessions its live agents belong to beside the agents themselves, and a
`WorkList` carries both.

**`accepts_input` travels with the agent rather than being discovered.** Two of the four bridges are
one-shot: their prompt goes in through the launch and they take nothing after it. A composer that
learned that from a refused turn would have offered the user something that was never there, so the
capability is on the message that says the agent exists.

**A conversation outlives its harness.** `ConversationEnded` says the process is gone; the transcript
stays on screen, and the agent stops accepting turns. Only closing it discards what was said.

**`AnswerPermission` is on the wire and answered with a refusal.** Nothing emits a permission
request yet — every bridge auto-approves, and P7 is what changes that. It is named here because the
family was designed whole rather than grown one variant at a time, and because a client that sends
one deserves an error rather than silence.

**`SetAgentConfig` is real before a harness exists, and refused after.** While a conversation is
still pending (above), `SetAgentConfig{config_id: "model", ..}` is what records the model its first
prompt will launch with — the only config option a pending agent offers. Once a bridge is running,
the same message is refused: every bridge rejects `SetConfigOption`, because a model, once chosen,
cannot change mid-conversation.

## The payload records

Twenty-eight records travel inside payloads.

| Record | Fields |
|---|---|
| `SessionInfo` | `id`, `name`, `home_folder`, `created_at` |
| `WorkspaceInfo` | `id`, `session_id`, `project_id`, `rel_path?`, `agent_type`, `cols`, `rows`, `running` |
| `ShellInfo` | `label`, `program`, `is_default` |
| `AgentTypeInfo` | `id`, `label`, `available` |
| `ProjectRecord` | `id`, `name`, `path`, `colour`, `created_at`, `last_opened_at?` |
| `ProjectSnapshot` | a `ProjectRecord`, flattened, plus `health`, `open_panes` and `workarea` |
| `DirEntry` | `name`, `rel_path`, `kind`, `size?`, `symlink` |
| `DirListing` | `rel_path`, `entries[]`, `truncated` |
| `FileContents` | `bytes`, `len`, `truncated`, `is_binary`, `version?` |
| `FileVersion` | `len`, `modified?` |
| `DiffRow` | `kind`, `old_line?`, `new_line?`, `text` |
| `DiffHunk` | `old_start`, `old_lines`, `new_start`, `new_lines`, `rows[]` |
| `FileDiff` | `base`, `hunks[]`, `binary`, `truncated` |
| `TaskRecord` | `id`, `session?`, `status`, `priority`, `shape`, `title`, `description`, `steps[]`, `created_at`, `updated_at` |
| `Step` | `id`, `title`, `state`, `owner?` |
| `WorkSession` | `id`, `name`, `branch`, `worktree` |
| `WorkAgent` | `id`, `session`, `task?`, `parent?`, `name`, `role`, `activity`, `note`, `branch`, `tokens`, `harness`, `model`, `context_pct`, `thread[]` |
| `Turn` | `from`, `text` |

| `ConvUpdate` | one of: `Started`, `UserChunk`, `AgentChunk`, `ThoughtChunk`, `ToolCall`, `ToolCallUpdate`, `Plan`, `ConfigOptions`, `ModeChanged`, `Title`, `Usage`, `RateLimit`, `PermissionRequest`, `TurnEnded` |
| `ToolCallRecord` | `id`, `title`, `kind`, `status`, `content[]`, `locations[]` |
| `ToolCallPatch` | `id`, and `title?`, `kind?`, `status?`, `content?`, `locations?` — absent is unchanged |
| `ToolLocation` | `path`, `line?` |
| `UsageRecord` | `used`, `size`, `cost_usd?`, `model?` |
| `RateLimitRecord` | `five_hour_pct?`, `five_hour_resets_at?`, `seven_day_pct?`, `seven_day_resets_at?`, `status` |
| `ConfigOption` | `id`, `name`, `description?`, `category?`, `value` |
| `ConfigChoice` | `value`, `name`, `description?`, `group?` |
| `PermissionOption` | `option_id`, `name`, `kind` |
| `PlanEntry` | `content`, `priority`, `status` |

**The record is what the store holds; the snapshot is what crosses the bus.** Keeping them apart is
what stops a stale health flag or a pane count from being written down and believed at the next
boot.

**The work's names carry the same distinction without a second type.** A `TaskRecord` is written
down, and `tasks.toml` holds exactly what crosses the bus, so there is nothing to keep apart: no
field on a task is like `health` or `open_panes`, which can only be known at the moment they are
asked for. A `WorkSession`, a `WorkAgent` and a `Turn` are the other way round — per-request payloads
with no store behind them, in the class `DirEntry` and `DirListing` are in.

Twelve enums travel inside those records. `ProjectHealth` is `Ok`, `Missing`, `NotADirectory`, or
`Unreadable` with the reason. `FileError` is `Refused`, `Missing`, `WrongKind`, `Denied`, `Conflict`
or `Failed`, and the file family's section says what each one asks the interface to do.

`EntryKind` is `Dir`, `File`, or `Other` — a symlink leading out of the project or nowhere, a socket,
a device, a pipe. `Other` is **drawn and refused**: the row appears, because a tree with rows missing
is a tree that lies, and a `ProjectTree` or a `ReadProjectFile` naming it comes back `WrongKind`.
`size` is present only for a regular file, and it is the only way the interface can know how large
something is before it asks for it.

A `Scope` — `Interface` or `Project(ProjectId)` — says what a stored preference belongs to. Its
`value` is **opaque**: a string the host writes down and hands back and never parses, on the same
discipline that keeps terminal bytes uninterpreted. The interface owns that schema and versions
it.

`SettingsLayer` — `Ui` or `Host` — says which half owns a settings blob. The Ui layer is opaque
the same way a preference is. The Host layer is JSON on the wire of a `HostSettings` record the
host parses; a schema this build does not understand is `SettingsError`, not a discarded default.
`HostSettings` carries a `schema` and `isolate_agents`, which is whether an agent runs confined —
the one setting the host acts on rather than stores, read again at every spawn. A record written by
an older build still parses, because every field added since carries a default; only a newer schema
is refused.

The conversation family's own enums are the Agent Client Protocol's and are named after it rather
than after anything here, so a reader can check them against
[`../../refs/acp-protocol.md`](../../refs/acp-protocol.md) directly. `ToolKind` is ACP's ten —
`Read`, `Edit`, `Delete`, `Move`, `Search`, `Execute`, `Think`, `Fetch`, `SwitchMode`, `Other` —
and carries the verb its block's header leads with. `ToolStatus` is `Pending`, `InProgress`,
`Completed` or `Failed`. `PermissionKind` is `AllowOnce`, `AllowAlways`, `RejectOnce` or
`RejectAlways`; nothing remembers an "always" yet, and where it should be remembered is an open
question in [`../backlog.md`](../backlog.md). `StopReason` is ACP's five plus `Failed`, which is
ours and means the run broke rather than the model declining. `ConfigCategory` — `Mode`, `Model`,
`ModelConfig`, `ThoughtLevel`, or an `Other` carrying whatever a harness invented — is a hint about
which picker draws an option and must never change what an id means.

`DiffBase` is `Head` or `Index`, and `DiffRowKind` is `Context`, `Added` or `Removed` — the marker a
textual diff puts at the front of a line, kept as a thing to draw rather than a character to strip.

Six of the twelve are the work's, and all but `Speaker` carry the words they answer to — a `label()`,
plus a `note()`, an `all()` or a `bucket()` where there is one — because the host needs those as much
as the interface does: it seeds the columns, it writes a `Status` down, and it classifies its own
agents. `Status` is
`Backlog`, `Ready`, `InProgress`, `InReview` or `Done`, in the order the board draws and work moves
along. `Priority` is `Low`, `Normal` or `High`, where `Normal` is the absence of a claim rather than
a middle value and so has no word. `Shape` is `Direct`, `Chain` or `Coordinated`, and says whether
the agents on a task run in order. `StepState` is `Idle`, `Working`, `NeedsYou`, `Failed` or `Done`.
`Activity` is `Thinking`, `Writing`, `Tools`, `NeedsYou`, `Ended` or `Failed`, and buckets into the
four coarse states — `Running`, `Waiting`, `Ended`, `Error` — a filter asks about. `Speaker` is `You`
or `Agent`. Which token any of them reads in stays the interface's alone.

`WorkspaceInfo` carries no handle to a process, a writer or a pseudo-terminal. Those live in the
coordinator and stay there — a record that crosses the bus must survive serialisation, which is the
mechanical form of the rule that the UI never assumes the pseudo-terminal is local.

## The search family

The eighth family. **Every variant names a project and a search**, because a search is scoped to a
project and identified by the UI-created `SearchId` that rides on every message.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `SearchProject` | UI → host | `search_id`, `project_id`, `query` | `SearchMatches`, `SearchProgress`, `SearchFinished`, or `SearchError` |
| `CancelSearch` | UI → host | `search_id`, `project_id` | — |
| `SearchMatches` | host → UI | `search_id`, `project_id`, `batch` | — |
| `SearchProgress` | host → UI | `search_id`, `project_id`, `files_seen` | — |
| `SearchFinished` | host → UI | `search_id`, `project_id`, `searched`, `truncated` | — |
| `SearchError` | host → UI | `search_id`, `project_id`, `error` | — |

**One live search per project.** A new `SearchProject` for a project that already has one in flight
supersedes it: the host cancels the old walk and starts the new one. The UI creates a `SearchId`
before the first request hits the wire, so the first `SearchProject` message carries the id.

**Batching.** The worker flushes on 64 files or 512 hits, whichever comes first. There is no timer.
A batch carries zero or more `FileHit` records, each with a `rel_path`, a list of `LineHit`s and a
`truncated` flag. `SearchFinished` carries the total count of files with hits and a global
`truncated` flag.

**Ceilings.** `HITS_PER_FILE` is 100, `FILES_WITH_HITS` is 1 000, `TOTAL_HITS` is 10 000. A ceiling
that bites sets the relevant `truncated` flag.

**Progress.** `SearchProgress` is sent every 100 files the walker sees, so the UI can show a spinner
that advances.

## The account family

The ninth family. An **account** is one authentication a harness runs as, and this family is how
one comes into being and how the interface learns which exist.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ListAccounts` | UI → host | — | `Accounts` |
| `Accounts` | host → UI | `accounts` | — |
| `BeginHarnessLogin` | UI → host | `agent_type`, `account` | `HarnessLoginStarted`, or `HarnessLoginFailed` |
| `HarnessLoginStarted` | host → UI | `pane_id`, `agent_type`, `account`, `cols`, `rows` | — |
| `HarnessLoginCaptured` | host → UI | `agent_type`, `account` | — |
| `HarnessLoginFailed` | host → UI | `agent_type`, `account`, `error` | — |

**References only, never material.** `AccountInfo` is an id and the harness ids it has a captured
login for. No credential and no path cross this family — that is the domain rule about accounts
carrying credential references, and this family is where it is kept or lost. The log sink listens
to the same bus, so a secret here would be a secret in a log the user might paste into an issue.

**Which harnesses an account covers is derived, not recorded.** An account is a home; a harness is
logged in there when the files its own `login_seed` names are present. So `logged_in` is computed
per request, one account can serve several harnesses without saying so anywhere, and an empty list
means the account references an environment variable rather than a captured session.

**A login runs in a pane, and that pane belongs to no project.** `HarnessLoginStarted` names a
`PaneId` that behaves like any other — it carries `TerminalOutput`, takes `TerminalInput`, resizes
by `TerminalResize` — but it joins no project's pane count and gets no dock panel. The window draws
it in a modal instead. Ending it is an ordinary `CloseWorkspace`.

**The outcome is decided by the credential, not the exit code.** The host records the credential's
timestamp before the login starts, and on the pane's end there are exactly three answers: the file
appeared and is newer, so an account exists; it is there but untouched, so the harness exited
without logging anyone in; or it is absent, so the flow was abandoned. Only the first sends
`HarnessLoginCaptured`, and `Accounts` follows it so no window has to ask again. This is what makes
abandoning a login safe, and it is why an exit code alone would not do: a harness can exit cleanly
having done nothing.

**Creating an account is logging one in.** There is no `AddAccount`. `BeginHarnessLogin` with an
unknown id creates that identity if and only if the login captures something, so a half-finished
flow leaves nothing behind to clean up.

## Framing

- **Message boundaries are explicit.** The in-memory channel carries whole values; a socket
  transport frames them. Neither half may rely on a read returning exactly one message.
- **Output is chunked, not lined.** A `TerminalOutput` is whatever the reader got from one read.
  The UI reassembles nothing; the emulator handles partial sequences.
- **Order is preserved per pane.** Two messages for the same pane arrive in the order they were
  sent. Across panes, no ordering is promised.
- **The bus never blocks the coordinator's reader.** In process, that is two unbounded channels, so
  a send never waits on a receiver. A queue that fills is a UI that has fallen behind, not a
  harness that has stopped. What a bounded transport would drop instead is open — see
  [`../backlog.md`](../backlog.md).
- **The file family is answered in the order it was asked.** One worker and one queue, so two
  expands of the same folder cannot leave the older answer on screen. A pool would reorder, and
  fixing that would cost a sequence number on the wire.

## Adding a variant

1. Decide the family. If it names a pane, the pane family. If it names a project **and a path inside
   it**, the file family. If it names a project **and a piece of work inside it** — a task, a step or
   an agent — the work family. If it names a project **and a search inside it**, the search family.
   If it names a project alone, the project family. Otherwise the
   session family.
   If it names an **agent** and carries something that agent said, the conversation family.
2. Add the variant to the enum in `crates/ubiq-proto/src/messages.rs`, with an owned payload — no
   borrowed data, no handles, nothing that fails to serialise.
3. Add a row to the table above, in the same commit.
4. Handle it in the coordinator's dispatch. A message the coordinator receives but ignores is worse
   than one that does not exist.
5. If the variant makes a structural choice, append a row to [`decisions.md`](./decisions.md).

Response-direction variants are never received by the coordinator; its dispatch rejects them rather
than falling through silently.

## Rationale

**Why one enum instead of separate request and event types?** One enum means one wire format, one
dispatch, and one place to look. The direction column is documentation, not a type-level
distinction — and buying that distinction would cost the single serialisable channel that makes the
process split cheap.

**Why raw bytes rather than parsed terminal events?** Parsing would put a VT engine in the
coordinator, duplicate the emulator the UI already has, and make the contract depend on how a
harness draws. Opaque bytes keep both halves ignorant of terminal semantics.

**Why is `Focus` a message at all, when focus looks like pure UI state?** Because the coordinator
decides what to do with output for an unfocused pane, and because a detached coordinator with two
attached UIs needs to know which one is typing.

## Related docs

- [`architecture.md`](./architecture.md) — the two halves and the rules the contract enforces
- [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — what the session family is for
- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — what the pane family is for
- [`../features/workbench.md`](../features/workbench.md) — what the work family is drawn as
