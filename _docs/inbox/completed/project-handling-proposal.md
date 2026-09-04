---
id: inbox-projects
title: Proposal — project handling
kind: proposal
status: proposal
summary: Projects owned by a headless host — the durable record, the message family that creates, forgets and describes one, how a missing directory is reported rather than deleted, and the crate split that makes the UI's lack of disk access a compile error.
read_when: you are deciding how projects are created, remembered or described, or where the line between the host and the interface falls
updated: 2026-08-31
depends_on: [tech-architecture, tech-transport, feat-workbench]
---

# Proposal — project handling

A project is the unit a window is opened on: a name, a folder, a colour, and what Ubiq remembers
about it between runs. Today it lives entirely in the UI as invented fixtures. This proposes moving
it to the host, behind the same message set every other capability crosses, with a storage seam that
can be swapped and a crate boundary that makes the UI's lack of filesystem access mechanical rather
than voluntary.

## 1. Where it stands

`crates/ubiq/src/state/windows.rs` holds `Project` — name, path, colour, terminals, and a `when`
string — and `WindowRegistry`, a GPUI global owning both the catalogue and which window holds which
project. The catalogue is seeded from `crates/ubiq/src/state/sample.rs` and dies with the process.

Three things follow, and each is a reason to move. **The UI owns durable state**, so nothing
survives a restart and nothing can without the UI half writing to disk. **A project is a `usize` into
a `Vec`**, so every window slot, picker row and action names a project by its index, and removing one
renumbers the rest. **The runtime facts are fabricated**: `terminals` is a fixture that only the half
owning the panes can know, and `when` is a pre-rendered string the UI cannot re-render as time
passes.

The window-to-project map is not the problem and stays where it is. Windows are a UI concept.

## 2. The headless host

The frame this proposal assumes, and which it asks to be made explicit:

**The host owns everything that is not drawing.** Disk, the project catalogue, the file tree and
file contents, version control, harness configuration and launch, processes and pseudo-terminals,
sessions and workspaces. It has no window, no palette, no layout, and no dependency that draws.

**The UI is an interface to the host and nothing else.** It draws, routes input, and holds a
projection of what the host has told it. It opens no file, resolves no path, spawns no process, and
reads no configuration. Every fact it shows arrived in a message.

That is the architecture's existing rules taken to their conclusion: the UI cannot assume the
pseudo-terminal is local, so it must not assume the *filesystem* is local either. Two futures depend
on it — a UI on a different host from the one running the agents, and a remote drone supplying
filesystem and process access for one project. Neither is in this version; what this version owes
them is that they stay transport changes.

**Make it a crate boundary, not a convention.** The rule is written down today and the two halves
share a crate, which makes cheating a one-line import. Three crates make it a compile error:

| Crate | Holds | Depends on |
|---|---|---|
| `crates/ubiq-proto/` | The message set, the bus, the log sink | Nothing that draws, nothing that touches disk |
| `crates/ubiq-host/` | Projects, files, git, sessions, harnesses, processes, PTYs | The protocol crate, agent-manager, gitoxide |
| `crates/ubiq/` | Windows, panes, chrome, theme, the projection | The protocol crate — **not** the host crate |

Only the binary names both, and it names the host once: to start it and hand the UI the other end of
the bus. The UI crate cannot reach around the bus because the host's types are not in its dependency
graph. This is the device `crates/agent-manager/` already uses — no UI dependency, checked
mechanically — and it earns a matching recipe: the host crate builds and tests with no drawing crate
anywhere in its tree. When the host detaches, the binary becomes two binaries and the bus grows
framing; nothing else moves, because nothing else was allowed to know.

Two consequences worth stating before they surprise someone. **Harness definitions move host-side**:
`crates/ubiq-host/src/agent.rs` becomes the host's, and the UI learns agent types from the `AgentTypes`
message the contract already has. And **the log sink does not cross** — it is a `tracing` layer both
halves write to, installed by the binary, so it goes to the protocol crate; a detached host's records
would need a diagnostics message family, which is D24's known cost and stays a backlog row.

## 3. The model

Split the record along the line of what survives a restart.

**Durable — the record.** Written to the store, read back at boot.

| Field | Meaning |
|---|---|
| `id` | A ULID. Stable across rename, recolour and a move on disk |
| `name` | Display name. Defaults to the folder's leaf; a rename never touches the filesystem |
| `path` | The canonical absolute path, as the **host** resolves it |
| `colour` | Index into the theme's project swatches |
| `created_at` | When the project entered the catalogue |
| `last_opened_at` | Stamped by the host when a window opens it. Absent until first opened |

**Derived — the snapshot.** Computed on demand, never written. The record plus:

| Field | Meaning |
|---|---|
| `health` | `Ok`, `Missing`, `NotADirectory`, or `Unreadable` with the reason |
| `open_panes` | How many panes the host has running in this project |

The snapshot crosses the bus; the record is what the store holds. Keeping them apart is what stops a
stale health flag or a pane count from being persisted and believed at the next boot.

**Identity is the ULID, not the path.** The canonical path is a *uniqueness key* — adding a folder
already in the catalogue resolves to the project that is there rather than making a second — but it
is not the identity. A project that moves on disk keeps its id, colour, bindings and history; only
its `path` changes, which is what makes a Locate action possible at all. **`name` is display, `path`
is truth**: two projects may share a name, and nothing keys off it.

**A ULID rather than a UUID**, because it sorts by creation time, prints as 26 case-insensitive
characters with no hyphens, and so gives a readable directory name and a stable ordering for free —
none of which a v4 UUID does. It costs one new dependency; `uuid` stays in the tree, because pane
and session ids are `Uuid` in the contract today. Whether those follow is a separate question, and
not one this proposal needs answered: the two kinds of id never meet.

### Ids everywhere else

If a project is a ULID, the rest of the contract should be one too. Pane and session ids are `Uuid`
in `crates/ubiq-proto/src/messages.rs`, which would leave the message set carrying two id schemes for no
reason anyone could state a year from now.

**The argument is not really ULID versus UUID — it is that a contract-wide id change is the only
moment newtypes are free.** `pane_id` and `session_id` are both `Uuid` today, so nothing but care
stops one being passed where the other belongs. Doing the swap means touching every id site anyway,
and `PaneId`, `SessionId`, `WorkspaceId` and `ProjectId` as newtypes over `Ulid` cost nothing extra
while they are being touched. Later they cost a second sweep.

**The wire does not change shape.** Both types serialise as a string; a ULID is 26 characters against
36. Old and new cannot interoperate, which costs nothing while the bus is an in-memory channel and no
message log is kept — and is the reason to do it before a socket exists rather than after.

The surface is small: 39 mentions across six files, four generation sites, and `uuid` leaves Ubiq's
manifest. Three things to get right, none of them large:

- **Use a monotonic generator.** Two ids minted in the same millisecond sort arbitrarily otherwise,
  and sorting by creation time is most of why this is worth doing.
- **A ULID carries its creation time.** That is a feature in a config directory and a fact worth
  knowing before ids travel to a host the user does not own. It is not a secret here.
- **GPUI's `WindowId` is not ours** and stays exactly as it is.

`crates/agent-manager/` has no UUID at all — its session ids are `<unix-millis>-<pid>` strings, which
a ULID would improve on both counts. That is a change in that crate's own documentation, proposed
there, not decided here.

Reserved for later, named so their absence is deliberate: `pinned`, `tags`, and the field a remote
drone would need to say *where* the folder is. A per-project default harness is **not** on that
list — that is agent-manager's, and §5 says why.

## 4. The message family

A third family beside the pane and session families. Every variant names a project by id.

| Message | Direction | Payload | Answers with |
|---|---|---|---|
| `ListProjects` | UI → host | — | `ProjectList` |
| `AddProject` | UI → host | `path`, `name?`, `colour?` | `ProjectAdded` or `ProjectError` |
| `ForgetProject` | UI → host | `project_id` | `ProjectForgotten` |
| `UpdateProject` | UI → host | `project_id`, `name?`, `colour?` | `ProjectChanged` |
| `OpenedProject` | UI → host | `project_id` | `ProjectChanged` |
| `RefreshProject` | UI → host | `project_id` | `ProjectChanged` |
| `ProjectList` | host → UI | `projects[]` | — |
| `ProjectAdded` | host → UI | `project` | — |
| `ProjectChanged` | host → UI | `project` | — |
| `ProjectForgotten` | host → UI | `project_id` | — |
| `ProjectError` | host → UI | `project_id?`, `error` | — |

**Adding is not creating a directory.** `AddProject` on a path that does not exist is a
`ProjectError`, not an empty folder. Making new folders is a separate flow that does not exist.

**Forgetting is not deleting.** `ForgetProject` removes the record, and the project's own directory
in Ubiq's config — its view state and its cache — and touches nothing inside the project's folder.
The word in the UI is "Forget" for that reason.

**`OpenedProject` is how `last_opened_at` gets stamped.** The UI reports that a window pointed at a
project; the host decides what that means and persists it.

### Choosing a folder is a host question

`AddProject` carries a path, and it is the only message that does. That path has to come from
somewhere, and the obvious source — the operating system's native folder dialog — is wrong under a
headless host: it browses the *interface's* filesystem, which is not the one the project lives on
the moment the two are separated.

So the host browses its own filesystem and the UI draws the result:

| Message | Direction | Payload | Answers with |
|---|---|---|---|
| `BrowseHost` | UI → host | `path?` | `HostListing` |
| `HostListing` | host → UI | `path`, `parent?`, `entries[]` | — |

An absent `path` means the host's home. Entries carry a name, whether they are a directory, and
whether they are readable — enough to draw a chooser, and nothing more. The native dialog stays as a
local-only shortcut, marked as such: it is the first thing to break when the host moves, and the
host-side browser is what makes that not matter.

### Files, later

| Message | Direction | Payload | Answers with |
|---|---|---|---|
| `ProjectTree` | UI → host | `project_id`, `rel_path`, `depth` | `ProjectTreeListing` |
| `ReadProjectFile` | UI → host | `project_id`, `rel_path`, `max_bytes` | `ProjectFileContents` |

**The UI holds project-relative paths only.** A `rel_path` is resolved against the record's root by
the host, and one escaping the root after normalisation is refused with `ProjectError`. This is the
file-level form of the rule that the UI never assumes the pseudo-terminal is local, and it is the
seam a remote drone would slot into without the UI noticing: a project id and a relative path do not
say which machine answered.

The walk is bounded — a depth and an entry ceiling — never follows a symlink out of the root, and
skips a default ignore set. Contents cross as bytes with `truncated` and `is_binary` flags the host
sets: opaque, like terminal bytes.

**One existing message changes.** `SpawnWorkspace` carries `folder: Option<String>`, an absolute
path the UI sends. It becomes `project_id` plus an optional `rel_path`, which removes the last
absolute path from the UI half and makes a workspace something that belongs to a project rather than
to a string.

## 5. Persistence
## 5. Persistence

Five classes of durable state, one movable config root, two store traits, and the rule that Ubiq
holds bindings while agent-manager holds definitions — all of it in
[`config-persistence-proposal.md`](./config-persistence-proposal.md), which is where the catalogue's
own store is specified.

The two facts this document leans on: **the catalogue is one TOML file behind a `ProjectStore`
trait**, and **everything Ubiq remembers about a project lives in Ubiq's config root keyed by the
project's ULID**, never inside the project's own folder.

## 6. Failure

The directory is the part of a project the host does not control, and every interesting failure is a
variation on it going away.

| What happens | Result |
|---|---|
| The folder is deleted while the project is open | The next probe reports `Missing`. The record stays. Open panes keep their last screen |
| A project is opened from history and its folder is gone | The window opens; the explorer is empty with the reason stated; the picker row is marked |
| A workspace is spawned in a `Missing` project | Refused with `ProjectError` before a pseudo-terminal exists, not a failed spawn |
| The path is a file, or a broken symlink | `NotADirectory`. Refused at `AddProject`; marked on an existing record |
| The folder exists but cannot be read | `Unreadable` with the OS reason. The tree comes back empty rather than partial |
| A folder already in the catalogue is added again | Resolves to the existing project. The picker points at it; no duplicate appears |
| The folder comes back — a volume remounts | The next probe reports `Ok`. Nothing was lost, because nothing was removed |
| The store file is corrupt or unreadable | The catalogue starts empty; the file is preserved; one `ProjectError` says so |
| The store cannot be written | Mutations hold in memory for the session; one `ProjectError` says they are not durable |
| A project's view state is corrupt or from an older schema | Discarded; the window opens on defaults. The host never read it, so it cannot say more |
| A view-state write fails | A log line. Where a splitter sat is not worth an error the user has to read |

**A record is never removed because its folder went away.** An unplugged drive, a network mount that
has not come up, a worktree mid-rebase — all temporary, and a catalogue that forgets on the user's
behalf is one the user stops trusting. Forgetting is a user action, always.

A `Missing` project offers two: **Locate**, which re-points the record at a folder chosen through
`BrowseHost` and keeps the id, the colour and the history; and **Forget**.

Health is probed at load, on `OpenedProject`, and on `RefreshProject`. One `symlink_metadata` per
record makes the boot probe cheap enough to be unconditional. Watching the filesystem is a backlog
row.

## 7. One host, many windows

**A catalogue is process-wide; the coordinator is per window.** Each `AppState` opens its own bus in
`crates/ubiq/src/app/boot.rs` and starts its own coordinator thread. Two hosts with a catalogue each
would race the store file and disagree about what exists — and "headless host" is singular by
definition.

So: **one host per process**, with the bus in `crates/ubiq-proto/src/bus.rs` growing a hub — a client id
per window, pane-family messages routed to the window owning the pane, project-family messages
broadcast to every window. A daemon with two attached UIs is the same routing problem, so the work
is not spent twice, and it closes the existing gap where two windows cannot see each other's panes.
Broadcast also makes the projection idempotent by construction: every window replaces the same
snapshot by id, and the existing `observe_global` redraw keeps every picker in step.

The alternative — per-window hosts sharing the catalogue behind a lock — is shared mutable state
reached around the bus, the shape rule 1 exists to forbid, and it does not solve the two writers.

## 8. What the UI half becomes

`WindowRegistry` keeps its job — which window holds which project, the picker's three groups, the
one-window-per-project rule — and loses the catalogue. Its `projects` vector becomes a projection
keyed by `ProjectId`, `WindowSlot` holds ids instead of indices, and the project fixtures in
`crates/ubiq/src/state/sample.rs` go. `terminals` reads from the snapshot; `when` is rendered from
`last_opened_at` at draw time, because how long ago something was is a fact about the moment it is
drawn. The registry stays pure logic, so `crates/ubiq/tests/windows.rs` keeps testing it without a
frame, and both stores get the same treatment over their memory implementations.

**An empty catalogue is a new state.** On first run there are no projects, and the current rule — a
window with no project closes — would quit the application at boot. The proposal is a window opening
on no project, showing the picker with an "Add a project…" affordance over `BrowseHost`, and the
rule amended to except it. Boot order: start the host, ask `ListProjects`, open the first window on
the most-recently-opened project, or on nothing if there are none — no UI-side disk read in any of
it.

## 9. Phases

1. **The ids.** ULID behind newtypes across the contract. Independent of everything below, small,
   and cheapest before a second id scheme exists.
2. **The config root.** Resolution order, `ubiq.toml`, and the redirect that keeps a development
   run out of the user's real accounts and credentials. Cheap, and everything else lands in it.
3. **The catalogue.** Record, snapshot, both store traits, their file and memory implementations,
   the project family, `BrowseHost`, the UI projection. Add, forget, rename, recolour and open
   survive a restart, and so do panel sizes and the palette.
4. **The crate split.** Protocol, host and interface crates, and the recipe that proves the host
   draws nothing. Harness definitions and the log sink move.
5. **One host per process.** The bus hub, pane routing, project broadcast.
6. **Files.** `ProjectTree` and `ReadProjectFile`; explorer and editor stop being fixtures;
   `SpawnWorkspace` takes a `project_id`.
7. **Catalogue projection and bindings.** `ListCatalog`, the composer's constants replaced,
   `project.toml`, and the embedder layer agent-manager needs to accept.
8. **Version control.** gitoxide in the host, behind a summary on the snapshot and a per-path status
   map feeding the explorer's existing status enum and the status bar's counts.

Phase 4 can precede phase 3: nothing in the catalogue work depends on the split beyond discipline.

## 10. What this asks to be decided

Ten decision rows, if this is taken:

- The core is a headless host owning disk, files, version control, harnesses and processes; the UI
  is an interface over the bus and nothing else.
- That boundary is three crates, not a written rule, and the host crate's freedom from drawing
  dependencies is checked mechanically.
- The project catalogue lives in the host; the UI holds a projection.
- A project is identified by a ULID; its canonical path is a uniqueness key, not its identity.
- Every id in the contract is a ULID behind a per-kind newtype, replacing `Uuid` for panes,
  sessions and workspaces as well.
- Projects persist as one TOML file behind a store trait. SQLite is a later swap, and it lands on
  the per-project cache rather than the catalogue.
- View state is persisted by the host as an opaque value it never parses, keyed by interface or by
  project, and Ubiq writes nothing inside a project's own folder.
- One config root resolves every store, movable by flag, environment or a bootstrap `ubiq.toml`, and
  moving it moves the embedded library's roots with it.
- Ubiq stores bindings, never definitions: profiles, harnesses, accounts, skills and plugins stay
  agent-manager's, projected over the bus rather than copied into a store of Ubiq's own.
- One host per process, with a routing hub in the bus, replacing one coordinator per window.

Two rule amendments: a window may hold no project when the catalogue is empty, and the UI holds
project-relative paths, never absolute ones.

Backlog rows left open: an advisory lock for two hosts on one store; a filesystem watch behind
health and the tree; ignore rules read from `.gitignore` rather than a fixed set; a diagnostics
message family for a detached host's log records; what a bounded transport does with a large tree or
file payload; and the embedder-supplied settings layer this asks of agent-manager, which is that
crate's row to file, not this one's.

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the two halves and the rules this obeys
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the message families land
- [`../features/workbench.md`](../features/workbench.md) — the picker and the rules projects follow today
- [`config-persistence-proposal.md`](./config-persistence-proposal.md) — what is written down, and where
- [`../tech/decisions.md`](../tech/decisions.md) — where the rows above would be appended
