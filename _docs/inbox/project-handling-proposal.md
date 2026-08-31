---
id: inbox-projects
title: Proposal — project handling
kind: proposal
status: proposal
summary: Projects owned by a headless host — the durable record, the message family that creates, forgets and describes one, the storage trait behind it, how a missing directory is reported rather than deleted, and the crate split that makes the UI's lack of disk access a compile error.
read_when: you are deciding how projects are created, remembered, described or persisted, or where the line between the host and the interface falls
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

Three things follow, and each is a reason to move:

- **The UI owns durable state.** Nothing survives a restart, and nothing can without the UI half
  writing to disk.
- **A project is a `usize` into a `Vec`.** Every window slot, picker row and action names a project
  by its index. Removing one renumbers the rest.
- **The runtime facts are fabricated.** `terminals` is a fixture; only the half that owns the panes
  can know it. `when` is a pre-rendered string, so the UI cannot re-render it as time passes.

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
on it holding — a UI on a different host from the one running the agents, and a remote drone
supplying filesystem and process access for one project — and neither is in this version. What this
version owes them is that they stay transport changes.

**Make it a crate boundary, not a convention.** The rule is currently written down, and the two
halves share a crate, which makes cheating a one-line import. Three crates make it a compile error:

| Crate | Holds | Depends on |
|---|---|---|
| `crates/ubiq-proto/` | The message set, the bus, the log sink | Nothing that draws, nothing that touches disk |
| `crates/ubiq-host/` | Projects, files, git, sessions, harnesses, processes, PTYs | The protocol crate, agent-manager, gitoxide |
| `crates/ubiq/` | Windows, panes, chrome, theme, the projection | The protocol crate — **not** the host crate |

Only the binary names both, and it names the host once: to start it and hand the UI the other end of
the bus. The UI crate cannot reach around the bus because the host's types are not in its dependency
graph. This is the same device `crates/agent-manager/` already uses — no UI dependency, checked
mechanically — and it earns a matching recipe: the host crate must build and test with no drawing
crate anywhere in its tree.

When the host detaches, the binary becomes two binaries and the bus grows framing. Nothing else
moves, because nothing else was allowed to know.

Two consequences worth stating before they surprise someone. **Harness definitions move host-side**;
`crates/ubiq/src/agent.rs` becomes the host's, and the UI learns agent types from the `AgentTypes`
message the contract already has. And **the log sink is the one thing that does not cross** — it is
a `tracing` layer both halves write to, installed by the binary, and it goes to the protocol crate.
A detached host's records would need a diagnostics message family; that is the known cost of D24 and
stays a backlog row.

## 3. The model

Split the record along the line of what survives a restart.

**Durable — the record.** Written to the store, read back at boot.

| Field | Meaning |
|---|---|
| `id` | A v4 UUID. Stable across rename, recolour and a move on disk |
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

**Identity is the UUID, not the path.** The canonical path is a *uniqueness key* — adding a folder
already in the catalogue resolves to the project that is there rather than making a second — but it
is not the identity. A project that moves on disk keeps its id, colour and history; only its `path`
changes. That is what makes a Locate action possible at all.

**`name` is display, `path` is truth.** Two projects may share a name. Nothing keys off it.

Reserved for later, named so their absence is deliberate: `pinned`, `tags`, a per-project default
harness, and the field a remote drone would need to say *where* the folder is.

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

**Forgetting is not deleting.** `ForgetProject` removes the record from the catalogue and the store
and touches nothing on disk. The word in the UI is "Forget" for that reason.

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
whether they are readable — enough to draw a chooser, and nothing more. The native dialog stays
available as a local-only shortcut, marked as such, and is the first thing to break when the host
moves; the host-side browser is what makes it not matter.

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
sets, on the same terms as terminal bytes: the host does not interpret them.

**One existing message changes.** `SpawnWorkspace` carries `folder: Option<String>`, an absolute
path the UI sends. It becomes `project_id` plus an optional `rel_path`, which removes the last
absolute path from the UI half and makes a workspace something that belongs to a project rather than
to a string.

## 5. Persistence

The store is a trait, so the format is a swap rather than a rewrite:

```rust
pub trait ProjectStore: Send + Sync {
    fn load(&self) -> Result<Vec<ProjectRecord>, StoreError>;
    fn upsert(&self, record: &ProjectRecord) -> Result<(), StoreError>;
    fn remove(&self, id: ProjectId) -> Result<(), StoreError>;
}
```

Three methods, because that is every mutation the catalogue makes. A file store rewrites the whole
file for each; a SQL store maps each to a statement. Neither shape leaks into the caller. It lives
in the host crate, and so does the configuration directory it writes to — the UI never learns where.

**The recommendation is one TOML file, not SQLite.** At `~/.config/ubiq/projects.toml`, overridable
by `UBIQ_CONFIG_DIR`, on the convention `crates/agent-manager/src/settings.rs` already sets.

| | One TOML file | SQLite |
|---|---|---|
| Records | Tens. A whole-file rewrite is microseconds | Same, with a page cache in front |
| Repair | The user opens it and fixes a path | A shell and a schema |
| Dependencies | `toml` and `directories`, both already in the workspace | A bundled C library and a build step |
| Concurrent writers | Last writer wins | Handled |
| Partial and incremental reads | No | Yes |

Nothing the catalogue does needs a query, an index or a partial read, so SQLite is a cost with no
matching benefit. It earns its place when the host starts caching per-project data with real volume
— a file-tree index, git history, chat transcripts — which is when partial reads and incremental
writes stop being optional. The trait is that seam, and the record set does not change at it.

Four rules the file store holds to:

- **A `version` key at the top of the file**, so a future migration has a hook to read.
- **Atomic writes.** Serialise, write a sibling temp file, fsync, rename over. A crash mid-write
  leaves the previous catalogue, never half of one.
- **A corrupt file is preserved, not truncated.** A parse failure renames it aside with a timestamp,
  starts empty, and sends `ProjectError`. Losing a catalogue silently is worse than starting without
  one loudly.
- **An unwritable store does not stop the session.** Mutations apply in memory and answer normally,
  with one `ProjectError` saying the catalogue is not durable.

Two hosts writing one file is last-writer-wins. That is a backlog row, not a design question — an
advisory lock around the read-modify-write closes it.

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
`crates/ubiq/src/app.rs` and starts its own coordinator thread. Two hosts with a catalogue each
would race the store file and disagree about what exists — and "headless host" is singular by
definition.

So: **one host per process**, with the bus in `crates/ubiq/src/bus.rs` growing a hub — a client id
per window, pane-family messages routed to the window owning the pane, project-family messages
broadcast to every window. A daemon with two attached UIs is the same routing problem, so the work
is not spent twice, and it closes the existing gap where two windows cannot see each other's panes.

The alternative — per-window hosts sharing the catalogue behind a lock — is shared mutable state
reached around the bus, the shape rule 1 exists to forbid, and it does not solve the two writers.

Broadcast makes the projection idempotent by construction: every window replaces the same snapshot
by id, and each window's existing `observe_global` redraw keeps every picker in step.

## 8. What the UI half becomes

`WindowRegistry` keeps its job — which window holds which project, the picker's three groups, the
one-window-per-project rule — and loses the catalogue. Its `projects` vector becomes a projection
keyed by `ProjectId`, `WindowSlot` holds ids instead of indices, and the project fixtures in
`crates/ubiq/src/state/sample.rs` go. `terminals` reads from the snapshot; `when` is rendered from
`last_opened_at` at draw time, because how long ago something was is a fact about the moment it is
drawn, not one to transmit. The registry stays pure logic, so `crates/ubiq/tests/windows.rs` keeps
testing it without a frame, and the catalogue gets the same treatment over a memory store.

**An empty catalogue is a new state.** On first run there are no projects, and the current rule — a
window with no project closes — would quit the application at boot. The proposal is a window opening
on no project, showing the picker with an "Add a project…" affordance over `BrowseHost`, and the
rule amended to except it.

Boot order: start the host, ask `ListProjects`, open the first window on the most-recently-opened
project, or on nothing if there are none. No UI-side disk read anywhere in it.

## 9. Phases

1. **The catalogue.** Record, snapshot, store trait, file store, memory store, the project family,
   `BrowseHost`, the UI projection. Add, forget, rename, recolour and open survive a restart.
2. **The crate split.** Protocol, host and interface crates, and the recipe that proves the host
   draws nothing. Harness definitions and the log sink move.
3. **One host per process.** The bus hub, pane routing, project broadcast.
4. **Files.** `ProjectTree` and `ReadProjectFile`; explorer and editor stop being fixtures;
   `SpawnWorkspace` takes a `project_id`.
5. **Version control.** gitoxide in the host, behind a summary on the snapshot and a per-path status
   map feeding the explorer's existing status enum and the status bar's counts.

Phase 2 can precede phase 1 if the boundary is worth having before there is much to put behind it;
nothing in phase 1 depends on the split beyond discipline.

## 10. What this asks to be decided

Six decision rows, if this is taken:

- The core is a headless host owning disk, files, version control, harnesses and processes; the UI
  is an interface over the bus and nothing else.
- That boundary is three crates, not a written rule, and the host crate's freedom from drawing
  dependencies is checked mechanically.
- The project catalogue lives in the host; the UI holds a projection.
- A project is identified by a UUID; its canonical path is a uniqueness key, not its identity.
- Projects persist as one TOML file behind a store trait. SQLite is a later swap, not a start.
- One host per process, with a routing hub in the bus, replacing one coordinator per window.

Two rule amendments: a window may hold no project when the catalogue is empty, and the UI holds
project-relative paths, never absolute ones.

Backlog rows left open: an advisory lock for two hosts on one store; a filesystem watch behind
health and the tree; ignore rules read from `.gitignore` rather than a fixed set; a diagnostics
message family for a detached host's log records; and what a bounded transport does with a large
tree or file payload.

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the two halves and the rules this obeys
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the message families land
- [`../features/workbench.md`](../features/workbench.md) — the picker and the rules projects follow today
- [`../tech/decisions.md`](../tech/decisions.md) — where the six rows above would be appended
