---
id: inbox-config
title: Proposal — configuration and persistence
kind: proposal
status: proposal
summary: What Ubiq writes down and where — the five classes of durable state, the movable config root that keeps a development run out of the user's real accounts, the two store traits, and the rule that Ubiq holds bindings while agent-manager holds definitions.
read_when: you are deciding where a piece of durable state lives, adding a store, or wiring the embedded library's roots
updated: 2026-08-31
depends_on: [tech-architecture, inbox-projects]
---

# Proposal — configuration and persistence

Everything Ubiq remembers between runs, and which half of it writes each thing down. The companion
to [`project-handling-proposal.md`](./project-handling-proposal.md), which proposes the headless host
this assumes: the host owns disk, the interface owns nothing, and every store below is the host's.

## 1. Five classes, and who owns each

"Configuration" is five different things with five different durability rules, and the mistake to
avoid is one store for all of them.

| Class | Example | Home | Lost if deleted |
|---|---|---|---|
| The catalogue | Which projects exist, name, path, colour, `last_opened_at` | Ubiq's config root, one file | The list of projects |
| Bindings | Which profile, harness and account this project opens on | Ubiq's config root, per project | A choice, remade in one click |
| View state | Panel sizes, expanded folders, open tabs, palette, terminal layout | Ubiq's config root, per project | Where you left off |
| Definitions | Profiles, harnesses, accounts, skills, plugins, MCP servers, policies | **agent-manager's**, never Ubiq's | Nothing of Ubiq's |
| Derived cache | File-tree index, git history, transcripts | Ubiq's config root, per project, deletable | Nothing — it rebuilds |

The line that matters most is between definitions and bindings. **A definition is agent-manager's; a binding to
it is Ubiq's.** What a "developer" profile *is*, which accounts a harness has, what a skill contains
— none of that is Ubiq's to store, and §5 says what that means as the set grows.

## 2. Where the config root is

Every store above resolves against one directory, and that directory has to be movable — a
development run must not touch the accounts, catalogue and credentials a user works with all day.

| Source | Precedence |
|---|---|
| A `--config-root` flag | Highest |
| `UBIQ_CONFIG_DIR` | |
| The nearest `ubiq.toml` walking up from the working directory | |
| `~/.config/ubiq` | The default |

`ubiq.toml` is a **bootstrap file, not a settings file** — it exists to say where the settings are,
and the discipline that keeps it useful is that it never grows a second answer to a question a store
already answers. In this repository it sets `config_root = "_data/config"`, and `_data/` is ignored
by git.

**Local mode redirects agent-manager too.** A config root that moves Ubiq's stores but leaves the
embedded library pointed at `~/.config/agent-manager` is a trap: a test login writes a real account,
and the credentials engine reaches the real keychain. So the host derives that library's roots from
its own config root — catalogue, accounts, and a file-backed credentials engine in place of the OS
store — and a development run is self-contained by construction. The library already takes each of
those as an override; the host supplies them instead of letting them default.

Because a config root you cannot see is a foot-gun, the status bar says when it is not the default.

## 3. The catalogue store

```rust
pub trait ProjectStore: Send + Sync {
    fn load(&self) -> Result<Vec<ProjectRecord>, StoreError>;
    fn upsert(&self, record: &ProjectRecord) -> Result<(), StoreError>;
    fn remove(&self, id: ProjectId) -> Result<(), StoreError>;
}
```

Three methods, because that is every mutation the catalogue makes. A file store rewrites the whole
file for each; a SQL store maps each to a statement. Neither shape leaks into the caller. The trait
and the directory it writes to live in the host crate; the UI never learns where.

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
matching benefit. Where it earns its place is the cache — real volume, behind its own trait, one
database per project, deletable by definition.

Four rules the file store holds to:

- **A `version` key at the top of the file**, so a future migration has a hook to read.
- **Atomic writes.** Serialise, write a sibling temp file, fsync, rename over. A crash mid-write
  leaves the previous catalogue, never half of one.
- **A corrupt file is preserved, not truncated.** A parse failure renames it aside with a timestamp,
  starts empty, and sends `ProjectError`. Losing a catalogue silently is worse than starting without
  one loudly.
- **An unwritable store does not stop the session.** Mutations apply in memory and answer normally,
  with one `ProjectError` saying the catalogue is not durable.

Two hosts writing one file is last-writer-wins — a backlog row, closed by an advisory lock around
the read-modify-write, not a design question.

## 4. The preference store

A headless host means the UI writes nothing, which means the UI cannot remember its own panel sizes.
Under the old shape that was a UI file; here it is the host's to store — and tmux is the precedent,
where the layout lives in the server and the client is a view that comes and goes.

But the host has no opinion about layout or colour, and must not acquire one to hold this. So it
stores the value **opaque**: a string it writes down and hands back, never parses, on the same
discipline that keeps terminal bytes uninterpreted.

```rust
pub trait PreferenceStore: Send + Sync {
    fn get(&self, scope: &Scope) -> Result<Option<String>, StoreError>;
    fn set(&self, scope: &Scope, value: &str) -> Result<(), StoreError>;
    fn clear(&self, scope: &Scope) -> Result<(), StoreError>;
}
```

`Scope` is `Interface` or `Project(ProjectId)` — the palette and window bounds belong to the
interface, the expanded folders and open tabs to the project. Two messages carry it:
`GetPreferences { scope }` answering `Preferences { scope, value }`, and `SetPreferences { scope,
value }`, which answers nothing.

Three rules that differ from the catalogue's, and the differences are the point:

- **The UI owns the schema, so the UI versions it.** A blob that fails to parse is discarded and the
  window opens on defaults. The host cannot validate what it will not read.
- **Writes are debounced and best-effort.** A panel drag fires continuously, and a preference that
  fails to save is a log line, not a `ProjectError`. Losing where a splitter sat is not an event.
- **One file per project, not a section in the catalogue.** Otherwise every drag rewrites the list
  the user may be hand-editing.

## 5. What arrives next, and where it goes

Profiles with roles — director, developer, QA — harness definitions, several accounts per harness,
skills and plugins are all coming, and all of them exist in agent-manager today. The rule that keeps
that merge cheap is the one already written: **Ubiq never names a harness config path.** It follows
that Ubiq stores no definition of any of them, and grows no registry of its own.

What Ubiq's host does instead is two things.

**It projects the library's catalogue over the bus.** A `ListCatalog` answered by a `Catalog` message
carrying profiles, harnesses, accounts, skills and plugins — identifiers and display names, resolved
by the library, held by the UI as a projection like everything else. The composer's `HARNESSES`,
`MODELS` and `MODES` constants in `crates/ubiq/src/state/chat.rs` stop being constants and become
that projection. **An account crosses as an id and a label, never as credential material** — the
domain rule holds at the transport, not just in the library.

**It stores the bindings.** Which profile a project opens on, which harness, which account: a
per-project `project.toml` beside its `view.toml`, holding identifiers into the library's catalogue
and nothing else. A binding whose target has gone is reported like a missing folder — named, marked,
and never silently repaired.

The host reads `project.toml` because it acts on it, and never reads `view.toml` because it has no
opinion about layout. That is the same rule in both directions, and it is why they are two files.

**A binding is a personal layer, not a replacement for the project's own configuration.** A team's
committed `am.toml` is discovered by walking up from the project root, as it already is; Ubiq's
binding is one more layer over it, resolved by the library's existing merge. That needs agent-manager
to accept a layer from its embedder — a change in that crate, with its own documentation, not here.

## 6. What Ubiq does not persist

**Nothing is written inside the project folder.** Everything Ubiq remembers about a project lives in
Ubiq's own config directory, keyed by the project's ULID. That buys four things: forgetting a project
cleans up completely, a read-only or missing folder still has its view state, no repository acquires
a file to gitignore, and no team has to agree on one. A committed `.ubiq/` for deliberately shared
view state stays available later, as an opt-in that changes none of this.

## 7. On disk

```
<config root>/
  projects.toml                 the catalogue
  preferences.toml              interface-scoped view state
  projects/<ulid>/project.toml  bindings — the host reads these
  projects/<ulid>/view.toml     view state — opaque, the host never reads it
  projects/<ulid>/cache/        derived, deletable, the SQLite candidate
  agent-manager/                the embedded library's roots, in local mode
```

Keying by ULID is what makes Forget a clean operation: drop the record, then drop the directory —
and the directory listing comes out in the order the projects were added, at no cost.
The order matters — the catalogue is authoritative, so it goes first, and a directory left behind by
a crash between the two is collected at the next load, where any `projects/<ulid>/` with no matching
record is garbage. Copying the one directory moves every project, every preference and every colour
to another machine.

## Related docs

- [`project-handling-proposal.md`](./project-handling-proposal.md) — the host, the catalogue and the message families these stores sit behind
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — the boundary that decides which side owns a definition
- [`../tech/architecture.md`](../tech/architecture.md) — the rule that keeps the host out of the interface's business
