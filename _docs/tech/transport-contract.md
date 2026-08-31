---
id: tech-transport
title: Transport contract
kind: tech
status: draft
summary: The complete message set the UI and the coordinator exchange — the pane, session, project and file families, the framing rules, and the procedure for adding a variant.
read_when: you are adding, changing or removing a message, or wiring either half to the bus
updated: 2026-09-01
verified: 2026-09-01
code_anchors: [crates/ubiq-proto/src/messages.rs, crates/ubiq-proto/src/ids.rs, crates/ubiq-proto/src/projects.rs, crates/ubiq-proto/src/files.rs]
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
`WorkspaceId`, `ProjectId` in `crates/ubiq-proto/src/ids.rs` — so a pane's id cannot be passed
where a session's belongs. Each serialises as its bare 26-character string, and all four come from
one monotonic generator, because sorting by creation time is most of why a ULID is worth having.
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
| `PaneExited` | coordinator → UI | `pane_id`, `code` | The harness ended. The pane stays visible and stops accepting input |
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
| `AgentTypes` | coordinator → UI | `types[]` | — |
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
| `ProjectList` | host → UI | `projects[]` | — |
| `ProjectAdded` | host → UI | `project` | — |
| `ProjectChanged` | host → UI | `project` | — |
| `ProjectForgotten` | host → UI | `project_id` | — |
| `ProjectError` | host → UI | `project_id?`, `error` | — |
| `Preferences` | host → UI | `scope`, `value?` | — |
| `HostInfo` | host → UI | `config_root`, `is_default` | — |

**`ProjectChanged`, `ProjectAdded` and `ProjectForgotten` are broadcast** to every attached window,
so every picker agrees by construction rather than by each window asking again. A `ProjectList` and
a `Preferences` go only to the window that asked.

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

**`AddProject` never creates a folder.** A path that does not exist is a `ProjectError`. A folder
already in the catalogue answers with the project that is there, so no duplicate appears.

**`ForgetProject` is not deleting.** It removes the record and the project's own directory in
Ubiq's config, and touches nothing inside the project's folder.

## The file family

The fourth family. Every variant names a project by id **and a path by `rel_path`**, because an
answer arrives after the click that asked for it and the window may have changed project since.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ProjectTree` | UI → host | `project_id`, `rel_path`, `depth` | `ProjectTreeListing` or `ProjectFileError` |
| `ReadProjectFile` | UI → host | `project_id`, `rel_path`, `max_bytes?` | `ProjectFileContents` or `ProjectFileError` |
| `WriteProjectFile` | UI → host | `project_id`, `rel_path`, `bytes`, `expected?` | `ProjectFileWritten` or `ProjectFileError` |
| `ProjectTreeListing` | host → UI | `project_id`, `rel_path`, `listings[]` | — |
| `ProjectFileContents` | host → UI | `project_id`, `rel_path`, `contents` | — |
| `ProjectFileWritten` | host → UI | `project_id`, `rel_path`, `version` | — |
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
costs one row rather than a walk. A directory over the host's entry ceiling comes back `truncated`
rather than quietly short.

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

**`ProjectFileError` is per path**, not per project, for the reason `PaneError` is per pane: the
interface can only mark the row or the tab the user is looking at if the message says which one. Its
`error` is a `FileError` — `Refused`, `Missing`, `WrongKind`, `Denied`, `Conflict` or `Failed` — and
each arm is a different thing for the interface to do rather than a sentence to match on. **The host
does not re-probe a project's health for a file failure**; a `Missing` or a `Denied` is the
interface's cue to send `RefreshProject`, which is the project family's job.

## The payload records

Nine records travel inside payloads.

| Record | Fields |
|---|---|
| `SessionInfo` | `id`, `name`, `home_folder`, `created_at` |
| `WorkspaceInfo` | `id`, `session_id`, `project_id`, `rel_path?`, `agent_type`, `cols`, `rows`, `running` |
| `AgentTypeInfo` | `name`, `command`, `description`, `default_args` |
| `ProjectRecord` | `id`, `name`, `path`, `colour`, `created_at`, `last_opened_at?` |
| `ProjectSnapshot` | a `ProjectRecord`, flattened, plus `health` and `open_panes` |
| `DirEntry` | `name`, `rel_path`, `kind`, `size?`, `symlink` |
| `DirListing` | `rel_path`, `entries[]`, `truncated` |
| `FileContents` | `bytes`, `len`, `truncated`, `is_binary`, `version?` |
| `FileVersion` | `len`, `modified?` |

**The record is what the store holds; the snapshot is what crosses the bus.** Keeping them apart is
what stops a stale health flag or a pane count from being written down and believed at the next
boot.

Four enums travel inside those records. `ProjectHealth` is `Ok`, `Missing`, `NotADirectory`, or
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

`WorkspaceInfo` carries no handle to a process, a writer or a pseudo-terminal. Those live in the
coordinator and stay there — a record that crosses the bus must survive serialisation, which is the
mechanical form of the rule that the UI never assumes the pseudo-terminal is local.

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
   it**, the file family. If it names a project alone, the project family. Otherwise the session
   family.
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
