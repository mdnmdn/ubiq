---
id: tech-transport
title: Transport contract
kind: tech
status: draft
summary: The complete message set the UI and the coordinator exchange — the pane, session, project, file, git, work, conversation, search, account, profile, command-line, host browse, connector, repository, assist and notification families, the framing rules, and the procedure for adding a variant.
read_when: you are adding, changing or removing a message, or wiring either half to the bus
updated: 2026-09-09
verified: 2026-09-09
code_anchors: [crates/ubiq-proto/src/messages.rs, crates/ubiq-proto/src/connectors.rs, crates/ubiq-proto/src/ids.rs, crates/ubiq-proto/src/projects.rs, crates/ubiq-proto/src/settings.rs, crates/ubiq-proto/src/files.rs, crates/ubiq-proto/src/git.rs, crates/ubiq-proto/src/work.rs, crates/ubiq-proto/src/conversation.rs, crates/ubiq-proto/src/repos.rs, crates/ubiq-proto/src/stats.rs, crates/ubiq-proto/src/assist.rs, crates/ubiq-proto/src/notifications.rs, crates/ubiq-host/src/notifications/mod.rs, crates/ubiq-host/src/assist/mod.rs, crates/ubiq-host/src/assist/api.rs, crates/ubiq-host/src/assist/providers.rs, crates/ubiq-host/src/assist/subject.rs, crates/ubiq-host/src/assist/stub.rs, crates/ubiq-host/src/conversation.rs, crates/ubiq-proto/src/wire.rs]
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
| `CheckAgentCommand` | UI → coordinator | `agent_type`, `command` | `AgentCommandChecked` |
| `ListHarnessCatalogue` | UI → coordinator | `agent_type`, `account?` | `HarnessCatalogue` |
| `SessionList` | coordinator → UI | `sessions[]` | — |
| `SessionCreated` | coordinator → UI | `session` | — |
| `SessionAttached` | coordinator → UI | `session`, `workspaces[]` | — |
| `WorkspaceSpawned` | coordinator → UI | `workspace` | — |
| `AgentTypes` | coordinator → UI | `agent_types[]` | — |
| `AgentCommandChecked` | coordinator → UI, asking client only | `agent_type`, `ok`, `detail` | — |
| `HarnessCatalogue` | coordinator → UI, asking client only | `agent_type`, `account?`, `models[]`, `last_model`, `last_thinking` | — |
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
| `AddProject` | UI → host | `path`, `name?`, `colour?`, `custom_colour?`, `temporary` | `ProjectAdded` or `ProjectError` |
| `ForgetProject` | UI → host | `project_id` | `ProjectForgotten` |
| `UpdateProject` | UI → host | `project_id`, `name?`, `colour?`, `custom_colour?`, `search_excludes?`, `index?` | `ProjectChanged` |
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
| `ListStats` | UI → host | — | `Stats` |
| `Stats` | host → UI | `stats` | — |

**A `Stats` goes only to the window that asked, and `ListStats` is the one thing the interface
polls for.** Everything else in Ubiq is pushed because something happened; the host's memory and its
uptime change when nothing happens, so there is no event behind them to push. The window asks every
two seconds while the Control screen is the rail mode and stops as soon as it is not, which makes
two windows watching that screen two independent samplers rather than one broadcast —
[`../features/stats.md`](../features/stats.md) owns what the reading contains and what the screen
does with it.

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
`available` is true when the harness's own binary is found on this machine **or** an override is
configured for it in `HostSettings.agent_commands`; `command` is what the library would run —
`claude`, say — carried to be shown as the field's placeholder when a user types an override, never
composed into a launch. `chat` is a separate axis: whether the harness has a structured bridge at
all, so its output can become a `ConvUpdate` rather than terminal bytes. A harness with no bridge
(Grok) still gets a pane — `available` and `command` mean the same thing for it as for any other —
but `harness_choices` in `crates/ubiq/src/state/workbench.rs` filters every chat-start menu on
`chat`, since starting a conversation with one would compose a run nothing ever reads. `chat` says
nothing about how many turns one process takes; a one-shot harness (Copilot, opencode) is `chat:
true` the same as a multi-turn one (Claude Code, codex).

**`modes` is whatever the harness named, and `unattended_mode` says which of them asks nothing.**
A permission mode is not a universal concept — it is one harness's own vocabulary, carried as
`ConfigChoice` rows — so an interface that wanted to open a start form on "ask nothing" had no way
to tell which id meant it without a table of its own. `unattended_mode` is the library's
`Harness::unattended_mode` crossing the bus: the harness's own spelling, `None` where it has no
such mode or already asks nothing. Which id means "all permissions" stays the harness's business,
which is the same reason `modes` is a list rather than an enum.

**`CheckAgentCommand` tries a typed command line before it is saved.** The UI sends `agent_type` and
the candidate `command`; the coordinator runs it with `--version` on its own thread against a 5s
timeout and answers `AgentCommandChecked` — `ok` and a one-line `detail` — to the asking client
only, never broadcast, because trying a command is not a fact about the harness that every window
needs to hear.

**`ListHarnessCatalogue` asks what a harness offers before anything is started.** A start form has
to name a model and a reasoning level, and until this pair existed the only way to learn them was
to launch the harness and read the `ConfigOptions` that came back — which meant the form could
only be filled in for a conversation that already existed. The coordinator answers on a one-off
thread through the same `probe_catalogue` the discovery thread inside `start_conversation` uses,
so the probe shells out without blocking the run loop, and the answer is cached on the harness
binary's own version string: slow exactly once, cheap after. A probe that fails answers
`HarnessCatalogue` with an empty `models` list rather than nothing at all — an unanswerable
harness offers "whatever it defaults to" instead of leaving the asker waiting. `account` rides
along as the cache key's identity leg; the probe itself is per harness. `last_model` and
`last_thinking` are what this harness was last actually launched with, empty where no such flag
ever went out — the same convention the host's own `chosen_model` follows — and they are a
preselection, never a promise: a model gone since simply preselects nothing. The answer goes to
the asking client only, because a form is one window's question.

**`AddProject` never creates a folder.** A path that does not exist is a `ProjectError`. A folder
already in the catalogue answers with the project that is there, so no duplicate appears.

**The two colour fields travel together.** `colour` is an index into the theme's swatches and
`custom_colour` is a packed `0x00RRGGBB` that wins over it, which is what lets project settings
offer a colour outside the swatches and have it survive a restart. `custom_colour` is applied only
when `colour` is `Some`, and a `None` `custom_colour` beside a `Some` `colour` clears the custom
colour back to the swatch — so the pair needs no third state to mean "leave it alone". A name-only
`UpdateProject` carries neither and touches neither. A swatch index is stored, so the swatch list is
only ever appended to: reordering it would recolour every project in the catalogue.

**`AddProject.temporary` opens a folder without writing it to the catalogue.** The record it mints
carries `ProjectRecord.temporary: true` and lives only in the host's memory — nothing is written to
`projects.toml`, and it is gone at the next launch. Every other message treats it exactly like any
other project, because they all resolve a project through the host's in-memory record lookup. There
is deliberately no separate "promote" message: `UpdateProject` on a temporary record is what clears
the flag and writes it down, and adding the same folder again through `AddProject` (with
`temporary: false`) promotes it the same way. See `D54`.

**`ForgetProject` is not deleting.** It removes the record and the project's own directory in
Ubiq's config, and touches nothing inside the project's folder — with one exception, which is the
folder Ubiq created in the first place. An **ephemeral clone** is deleted with its record, and only
when both of two independent things hold: the record is `temporary`, and its canonicalised path is
strictly inside the ephemeral root. Neither alone is enough — a folder dragged onto the window is
`temporary` too, and the root is a setting a user could point at their home directory — which is why
the gate is a conjunction rather than a flag (`D74`).

**The snapshot's `ephemeral` says whether that exception applies**, and the interface reads it
rather than repeating the test. The host answers the question with the same conjunction it deletes
by, so the warning a window raises and the folder `ForgetProject` removes cannot come apart; and the
interface could not repeat it in any case, since the unset ephemeral root resolves to a default only
the host knows. It sits beside `health` and `open_panes` for that reason — a fact about a project
that only the half owning the filesystem can state.

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

## The host browse family

The fourteenth family, and the smallest: one request and two possible answers, about browsing the
host's own filesystem before any project exists. It names no project, no pane and no path relative
to anything — the file family's `rel_path` only makes sense once a project's root is known, and this
is what lets a picker find that root in the first place, including on a host the interface has never
seen the disk of.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `BrowseHostDir` | UI → host | `path?` | `HostDirListing` or `HostDirError` |
| `HostDirListing` | host → UI | `path`, `parent?`, `entries[]`, `truncated` | — |
| `HostDirError` | host → UI | `path?`, `error` | — |

**`path` absent asks for a sensible starting place, not a listing of one the interface named.** The
host decides — the user's home directory, the same source the CLI shortcut and the config root
already draw it from — because a remote host's home directory is a fact only that host can state;
the interface has no filesystem of its own to propose one from.

**`path` is absolute and resolved against nothing.** Unlike every `rel_path` in the file family,
there is no project root yet for it to be relative to — this family exists to find that root, which
is also why the file family's containment check (`crates/ubiq-host/src/files/path.rs`) plays no part
here: there is nothing yet to contain a path inside, and this family answers a different question
than that one guards. Neither weakens the other. `HostDirListing.path` is always canonicalised, so
the interface shows where the host actually landed rather than the string it asked with — and never
carries the verbatim `\\?\` prefix on Windows, which does not display and does not survive the
interface joining it with a separator. A request path carrying `/` separators is accepted all the
same: the host normalises it before listing.

**`parent` is `None` only at the filesystem root**, so a picker knows when to stop offering to walk
up.

**A listing is capped at 2,000 entries**, independently of the file family's own per-reply ceiling —
a browse listing is always exactly one level, so there is no reply spanning several listings to
share a budget across. `truncated` says whether that ceiling cut this one short.

**Hidden entries are marked, not omitted.** `HostDirEntry.hidden` is true for a dotfile; unlike
`LIST_HIDE`'s junk files, a dotfile is real content the user may want to see, so whether to draw it
is the picker's call.

**`HostDirEntry.readable` is a hint, not a promise.** It says whether the host could open or enter
the entry when it looked, for greying out a row before the click; the filesystem can still change
before the next request.

**`HostPathError` is smaller than `FileError`.** It has no `Refused` — there is no root here for a
path to escape — and no `Conflict` — nothing here is ever written; its `NotADirectory` stands in for
`WrongKind`, since listing is the only thing this family does, so the one kind mismatch it can hit is
being asked to list something that is not a folder.

**This family is a deliberate departure from `D32`'s plan**, which expected a future host-side
listing to extend `AddProject` and `LocateProject` rather than add a new message family — `D82`
records why it went the other way instead, and what that costs.

## The file family

The fourth family. Every variant names a project by id **and a path by `rel_path`**, because an
answer arrives after the click that asked for it and the window may have changed project since.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ProjectTree` | UI → host | `project_id`, `rel_path`, `depth` | `ProjectTreeListing` or `ProjectFileError` |
| `ReadProjectFile` | UI → host | `project_id`, `rel_path`, `max_bytes?` | `ProjectFileContents` or `ProjectFileError` |
| `WriteProjectFile` | UI → host | `project_id`, `rel_path`, `bytes`, `expected?` | `ProjectFileWritten` or `ProjectFileError` |
| `DiffProjectFile` | UI → host | `project_id`, `rel_path`, `base` | `ProjectFileDiffed` or `ProjectFileError` |
| `EditProjectPath` | UI → host | `project_id`, `rel_path`, `to?`, `op` | `ProjectPathEdited` or `ProjectFileError` |
| `ProjectTreeListing` | host → UI | `project_id`, `rel_path`, `listings[]` | — |
| `ProjectFileContents` | host → UI | `project_id`, `rel_path`, `contents` | — |
| `ProjectFileWritten` | host → UI | `project_id`, `rel_path`, `version` | — |
| `ProjectFileDiffed` | host → UI | `project_id`, `rel_path`, `diff` | — |
| `ProjectPathEdited` | host → UI | `project_id`, `rel_path`, `to?`, `op` | — |
| `ProjectFileError` | host → UI | `project_id`, `rel_path`, `error` | — |
| `ProjectFilesChanged` | host → UI | `project_id`, `changed[]`, `truncated`, `repository` | — |

Every one of these answers only the window that asked, except the last, which nobody asked for.
Nothing in this family is broadcast: what one window is looking at is not a fact about the
catalogue.

**`ProjectFilesChanged` is the one file-family message the host sends unasked.** It names paths and
never contents, so a reader that wants what changed asks for it the normal way — a fresh
`ProjectTree` listing, a `ReadProjectFile`, a `RefreshProjectGit`. It reaches the one window that
has the project open, because that window's own watch is what produced it. `changed` is
project-relative paths on this family's rule, coalesced by path over a 150ms quiet window;
`truncated` means the burst outgrew the batch, so `changed` is empty and the reader re-lists the
subtree instead of patching names; `repository` means the git directory moved — `HEAD`,
`MERGE_HEAD`, the index or a ref — and is independent of `changed`, so a message may carry only it.

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

**An edit to a path is one message with an op on it.** `EditProjectPath` carries a `PathOp` —
`Create { dir }`, `Move`, `Copy`, `Trash` or `Delete` — and `ProjectPathEdited` echoes the request
whole. One message rather than five because the interface's need is identical every time: a path, a
destination for the two ops that have one, and what to do with them; five variants would be five
coordinator arms handing work to the same worker. The reply echoes the op because an answer arrives
after the click that asked for it, and what the interface does next depends on which gesture
finished — a created file is opened, a moved one retargets its tab, a removed one closes it.

`to` is the destination, and it is present for `Move` and `Copy` only. It is **refused** where it does
not belong rather than ignored: a field the host silently drops is a wiring mistake the interface
cannot see. **Every op refuses a destination that already exists**, which is the same judgement an
absent `expected` on a write makes — the contract does not hand out a forced overwrite for free — and
`Move` and `Copy` also refuse a destination inside their own source, so a folder cannot be moved into
its own child. `Create` makes exactly one level and never a parent, on the rule a write already keeps.

**`Trash` and `Delete` are two ops because they are two promises.** `Trash` hands the path to the
platform's own trash, where the user can get it back without Ubiq; `Delete` removes it, and a folder
goes with everything under it. Which one the interface is about to do is something it says before it
asks, so the difference is never left for the user to infer — and keeping them apart on the wire is
what lets it. Neither is a `WriteProjectFile` with no bytes: a write creates and a removal destroys,
and the version guard that makes a write safe has nothing to say about either.

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
| `ProjectGitLog` | UI → host | `project_id`, `cursor?`, `count`, `rel_path?`, `first_parent` | `GitLogPage` or `GitError` |
| `ProjectGitRefs` | UI → host | `project_id`, `with_tracking` | `GitRefs` or `GitError` |
| `GitOverview` | host → UI | `project_id`, `overview?` | — |
| `GitWorkingTree` | host → UI | `project_id`, `generation`, `entries[]`, `rollups[]`, `truncated` | — |
| `GitError` | host → UI | `project_id`, `error` | — |
| `GitLogPage` | host → UI | `project_id`, `cursor?`, `commits[]`, `next_cursor?` | — |
| `GitRefs` | host → UI | `project_id`, `refs[]` | — |

**`overview` absent is an ordinary answer**, not a failure: the project is not in a repository, and
the interface draws no branch and no badges. `GitError` is for a repository that exists and could
not be read — `NotFound`, `Corrupt`, `Denied`, `Interrupted` or `Failed`.

**The overview is cheap.** It is refs and a handful of files in the git directory: `HEAD` as a
branch name, a detached short id or an unborn name; the upstream and ahead/behind when there is
one, capped at 99; an in-progress operation; whether the repository is bare; its remotes, and any
submodule below the project's scope. Working-tree counts ride with a full refresh, and are absent
rather than zero on a bare or unborn repository, and absent until a walk has run.

**A remote is not a submodule.** `GitRemote` is a name and a URL the overview's own repository
fetches from; `GitSubmodule` is a different repository, pinned at a commit, with remotes of its
own. The overview carries both lists and flattens neither into the other, and a submodule outside
the project's scope is omitted the way a file outside it never appears in a listing.

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

**The log is cursor-paged, not offset-paged.** A page is a bounded walk from a starting commit —
`cursor` absent starts at `HEAD` — and `next_cursor` is the commit after the last one the page
carried, absent at the end. An offset would re-walk from `HEAD` every page and be wrong the moment
the tree moved underneath. `count` is clamped to `MAX_LOG_PAGE`; `rel_path` narrows the walk to one
path's history; `first_parent` skips the merged side of a merge commit. An unborn `HEAD` answers
with an empty page, not a `GitError`. `GitLogPage` echoes the `cursor` its request carried, so a
reply that lands after a later request already advanced the cursor is told apart from the current
one rather than guessed at from whether the interface already holds a cursor.

**A commit's `parents` are ids, not a count**, because a lane algorithm matches a child to the lane
its parent occupies and a count cannot say which lane that is. `lane` and `merges` are computed
host-side over those ids — which column the commit's dot sits in, and which lanes its extra parents
draw from for the merge lines — so two windows cannot lay out the same history differently; the
interface carries them through rather than computing a topology of its own.

**Refs are one reply for four sections.** Local branches, remote-tracking branches, tags and
stashes come back together in one `GitRefs`, because a sidebar with five sections has no use for
five walks when the host has already opened the repository. `with_tracking` adds ahead/behind per
local branch, one merge-base walk each, so a caller that only wants names — a branch picker —
skips the cost.

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
| `StartConversation` | UI → host | `agent_id`, `project_id`, `session_id`, `rel_path?`, `agent_type`, `account?`, `profile?`, `model?`, `thinking?`, `mode?` | `ConversationStarted` or `ConversationError` |
| `PromptAgent` | UI → host | `agent_id`, `text` | — |
| `CancelTurn` | UI → host | `agent_id` | — |
| `AnswerPermission` | UI → host | `agent_id`, `request_id`, `option_id` | — |
| `SetAgentConfig` | UI → host | `agent_id`, `config_id`, `value` | — |
| `EndConversation` | UI → host | `agent_id` | `ConversationEnded` |
| `UnloadConversation` | UI → host | `agent_id` | `ConversationUnloaded` |
| `AbortConversation` | UI → host | `agent_id` | `ConversationUnloaded` |
| `ResumeConversation` | UI → host | `agent_id` | `ConvUpdate::Started`, or nothing if already live |
| `ConversationStarted` | host → UI | `project_id`, `agent`, `session`, `accepts_input` | — |
| `ConversationUpdate` | host → UI | `agent_id`, `seq`, `update` | — |
| `ConversationEnded` | host → UI | `agent_id`, `stop_reason` | — |
| `ConversationUnloaded` | host → UI | `agent_id` | — |
| `ConversationError` | host → UI | `agent_id`, `error` | — |
| `ConversationNamed` | host → UI | `agent_id`, `title`, `summary?` | — |

**The vocabulary is the Agent Client Protocol's; the transport is the bus.** `D53` states why, and
[`../references/acp-protocol.md`](../references/acp-protocol.md) is the wire reference every name here
comes from. What that buys is that the library's own event model, this family and the mapper between
them are one vocabulary rather than three, and that a harness which speaks ACP natively is read
rather than translated.

**`StartConversation` carries the picks the start form asked for**, and each of `model`,
`thinking` and `mode` outranks the profile's own record, the way a pick always does: the host
seeds `chosen_model`, `chosen_thinking` and `chosen_mode` from the field first and the profile
second. An absent or empty field says nothing, which is what leaves the profile — or, failing
that, the harness's own default — in charge. Empty rather than `None` alone because the interface
sends the form's answer whatever it is, and "the user did not choose" and "the field is not on
this message" have to read the same.

**There is deliberately no `max_subagents` here.** No harness has a flag for it, so there is
nothing for the host to pass; the interface says it to the agent instead, as a directive folded in
front of the conversation's first turn. `ProfileInfo` still carries the number, because a saved
setup has to remember what it asked for; *The workbench* says what a start does with it.

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
discovery actually found anything). That first `ConfigOptions` carries up to three options, all
built by the same mechanism: `model` (always), `thinking` (only when the chosen model has
reasoning levels), and `mode` (only when the harness offers one). Picking a model makes the host
re-send `ConfigOptions` at the next `seq`, with `thinking` recomputed for the newly chosen model —
a level the old model accepted may not exist under the new one, so the window drops any held pick
the fresh options no longer back. **The re-send only happens when those levels actually differ.**
The model and mode lists cannot change by picking a model, and `current` is the value the window
itself just sent, so a pick that recomputes to the same picker is answered with silence rather than
a message that redraws what is already on screen. Only the window's first `PromptAgent` launches the harness,
carrying whatever `SetAgentConfig` last chose for each of the three — they reach a harness only as
launch flags, so changing a pick before that first prompt costs nothing. See
`_docs/wip/agent-setup.md`'s P3 and P6.

**`model`'s `current` on that first send is the harness's own last launch, not its default (D63).**
`FileHarnessCache` remembers, per harness, the model and thinking level `Coordinator::launch` last
actually used, and `build_config_options` preselects those as `current` — falling back to the
harness's flagged default when nothing is remembered, the remembered model has left the discovered
catalogue, or the remembered thinking level does not belong to the resolved model. The preference
is written only at launch, never at a pick alone, so a pick opened and abandoned in the picker
never becomes "the last one used."

**And what it shows is what it launches.** Only `SetAgentConfig` fills `chosen_model`, so a session
that simply accepted the proposal would otherwise send no flag at all and run the harness's true
default rather than the id `current` was showing. `Coordinator::launch` resolves the remembered
value itself when nothing was picked (`launch_picks`), under the same two validity rules the option
builders follow: a remembered model absent from the discovered catalogue falls back to no flag
rather than naming a model that is gone, and a remembered level the resolved model does not accept
is dropped, because a level belongs to a model and never to a harness.

**`WorkAgent.name` is derived host-side, not typed by the user.** The host names a conversation
from its harness's command — `claude`, `codex`, `opencode`, not the display label a menu shows —
with a counter from the second occurrence onward, per project: `claude`, `claude 2`, `claude 3`.
The first free name is picked, so a closed `claude 2` is reused before a new `claude 4` would be
minted. The sidebar row, the column header and the chat panel row all draw that field, never
`harness` directly. **That name is a placeholder, and two things may replace it.**
`ConvUpdate::Title` is the harness naming the conversation itself, and `ConversationNamed` is Ubiq
naming it from the opening exchange; both write the same field, and whichever spoke last is the
name. Neither of them is the user — no rename message exists on the wire, so a name nothing else
writes is the name for the conversation's life (`G119`).

**`ConversationNamed` is Ubiq's own reading, which is why it is not a `ConvUpdate`.** It carries no
`seq` and takes no place in the sequence an interface checks for gaps: the naming is not something
the harness said, and the pump that owns that sequence is not what produced it — the coordinator
is, on a thread of its own, once the turn that carried the reply is over.
Folding it into the transcript's numbering would make one message's absence read as a lost delta.
It is sent at most once per conversation, and only where a provider is configured to write one. The
title is what every surface that draws `WorkAgent.name` says from there on, and the `summary` beside
it is the tooltip those same surfaces draw — the assist family below carries the wording behind
both, and `D90` is why a mechanical name is replaced at all.

**A naming that fails is not reported.** There is no error variant paired with it, and the host
sends nothing: a provider that will not answer leaves the conversation called `claude 2`, which is
the name it holds until something writes another. Nothing is renamed behind anybody's back and
nothing reaches a repository, so there is no state for an interface to unwind and no question of
the user's left hanging — unlike a `SuggestError`, which answers something a window asked for.

**It reaches the window that owns the conversation, and no other.** A naming is a fact about a tab,
and only one window draws that tab; a second window with the same project open has nothing to
redraw. So it appears in no arm of `Message::project_id()` — the same call `ConversationError`
makes, for the same reason — and in no `pane_id_of` arm.

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

**`accepts_input` travels with the agent rather than being discovered.** It answers whether the
harness can be conversed with at all — a fact of the harness, which the library knows without a
bridge — and a composer that learned it from a refused turn would have offered the user something
that was never there. **It is not the question of whether one process survives a second turn.**
Two of the four bridges are one-shot: their prompt goes in through the launch and the process ends
with its answer, but the *conversation* takes every turn it is given, because the coordinator
relaunches the process for the next one. So a one-shot harness answers `true` here; answering
`false` is what drew one as `Lifecycle::Ended` before it had spoken at all (`G95`).

**`CancelTurn` reaches the harness as its own turn abort, not as a closed pipe.**
`Conversation::cancel` sends the library's `AgentInput::Cancel`, and each bridge writes whatever
its harness documents for "stop this turn": Claude Code an `interrupt` `control_request`, codex a
`turn/interrupt`. Closing the harness's input is a *different* library input, `AgentInput::Shutdown`,
which is what an unload and an abort send — so a cancel leaves a process that takes the next
`PromptAgent`, which is exactly what the stop button promises. The two one-shot harnesses have no
turn to interrupt short of ending the run, so for them a cancel kills the process; the conversation
survives it, which is why `accepts_input` stays `true`. `crates/agent-manager/_docs/io-modes.md`
§"Permissions and cancellation" carries the per-harness table.

**A conversation outlives its harness, and can start another one.** `ConversationEnded` says the
process is gone for good; the transcript stays on screen, and the agent stops accepting turns.
`UnloadConversation` is the same ending without the finality: the harness is killed, but the
transcript, the run directory and the `WorkAgent` all stay, and `ConversationUnloaded` — not
`ConversationEnded` — says so. A conversation in that state reads exactly as a pending one that has
not launched yet: the pickers return, `launched` is false again, and either `ResumeConversation` or
the next `PromptAgent` starts a fresh process under the same `agent_id`, picking the sequence up
where the old one left off rather than restarting it at one. Only `EndConversation` discards what
was said.

**`AbortConversation` is an unload that does not ask.** An unload asks the harness to shut down
and waits for it to; a harness that does not act on the ask is what makes that wait long, and the
wait is the coordinator's own thread (`G121`). An abort kills the process first and reaps it
afterwards, so the answer does not depend on the harness cooperating.
Everything else is an unload — the transcript, the run directory and the `WorkAgent` stay, the same
`agent_id` resumes, and the reply is the same `ConversationUnloaded`, so an interface handles the
two identically. A turn in flight is lost rather than stopped: `CancelTurn` is what interrupts one
and leaves the harness running.

**`AnswerPermission` closes a loop the harness is blocked on.** A `ConvUpdate::PermissionRequest`
carries a `request_id`, the `ToolCallPatch` it is asking about and the `PermissionOption`s the
harness offered; the interface draws them, and the user's press is one `AnswerPermission` naming
that request and one `option_id`. The host resolves the `agent_id` and calls
`Conversation::answer_permission` in `crates/ubiq-host/src/conversation.rs`, which sends the
library's `AgentInput::AnswerPermission` with `PermissionOutcome::Selected` — nothing between the
button and the harness interprets the id.

**The answer is an option id and nothing else.** `AgentInput::AnswerPermission` also carries an
`updated_input`, and this transport always leaves it `None`: the message has no field for an edited
tool input and will not grow one, because a client that rewrites what it was asked to approve is
approving something the transcript does not show. Take it or leave it, per option.

**Several requests may be outstanding at once, and every one of them must be answered.** There is
no timeout anywhere in the protocol, so a request nobody answers stalls the turn indefinitely. The
host tracks the outstanding `request_id`s per conversation — recorded as requests arrive, cleared as
they are answered — and `CancelTurn` is what discharges the rest: every request still outstanding
for that agent is answered `PermissionOutcome::Cancelled` **before** the cancel goes down, which is
what the library asks of a caller that gives up on a question it raised.

**`SetAgentConfig` is real before a harness exists, and refused after.** While a conversation is
still pending (above), `SetAgentConfig{config_id: "model", ..}` is what records the model its first
prompt will launch with — the only config option a pending agent offers. Once a bridge is running,
the same message is refused: every bridge rejects `SetConfigOption`, because a model, once chosen,
cannot change mid-conversation.

## The payload records

Forty-three records travel inside payloads.

| Record | Fields |
|---|---|
| `SessionInfo` | `id`, `name`, `home_folder`, `created_at` |
| `WorkspaceInfo` | `id`, `session_id`, `project_id`, `rel_path?`, `agent_type`, `cols`, `rows`, `running` |
| `ShellInfo` | `label`, `program`, `is_default` |
| `AgentTypeInfo` | `id`, `label`, `command`, `available`, `chat`, `modes[]`, `unattended_mode?` |
| `ProjectRecord` | `id`, `name`, `path`, `colour`, `custom_colour?`, `temporary`, `created_at`, `last_opened_at?` |
| `ProjectSnapshot` | a `ProjectRecord`, flattened, plus `health`, `open_panes`, `workarea` and `ephemeral` |
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
| `WorkAgent` | `id`, `session`, `task?`, `parent?`, `name`, `summary?`, `role`, `activity`, `note`, `branch`, `tokens`, `harness`, `model`, `context_pct`, `thread[]` |
| `Turn` | `from`, `text` |

| `ConvUpdate` | one of: `Started`, `UserChunk`, `AgentChunk`, `ThoughtChunk`, `ToolCall`, `ToolCallUpdate`, `Plan`, `ConfigOptions`, `ModeChanged`, `Title`, `Usage`, `RateLimit`, `PermissionRequest`, `TurnEnded` |
| `ToolCallRecord` | `id`, `title`, `kind`, `status`, `content[]`, `locations[]` |
| `ToolCallPatch` | `id`, and `title?`, `kind?`, `status?`, `content?`, `locations?` — absent is unchanged |
| `ToolLocation` | `path`, `line?` |
| `UsageRecord` | `used`, `size`, `cost_usd?`, `model?` |
| `RateLimitRecord` | `five_hour_pct?`, `five_hour_resets_at?`, `seven_day_pct?`, `seven_day_resets_at?`, `status` |
| `ConfigOption` | `id`, `name`, `description?`, `category?`, `value` |
| `ConfigChoice` | `value`, `name`, `description?`, `group?` |
| `CatalogueModel` | `id`, `description?`, `default`, `levels[]`, `default_level?` |
| `Notification` | `id`, `level`, `family`, `actor?`, `category?`, `text`, `link?`, `at`, `muted`, `read` |
| `NotificationRequest` | `level`, `family`, `actor?`, `category?`, `text`, `link?`, `muted`, `os` — the last two default `false` |
| `UbiqLink` | one of: `Pane`, `Project`, `Agent`, `File{project,path}`, `Url` |
| `MuteScope` | one of: `All`, `Family`, `Actor{family,actor}`, `Category{family,category}` |
| `MuteRule` | `scope`, `max_level`, `until?` |
| `MuteFor` | one of: `Minutes5`, `Minutes15`, `Hour1`, `Hours8`, `Always` |
| `Notifications` | `items[]` newest first, `mutes[]` |
| `ProfileInfo` | `id`, `agent_type`, `account?`, `model?`, `mode?`, `thinking?`, `max_subagents?`, `prompt?` |
| `PermissionOption` | `option_id`, `name`, `kind` |
| `CliDir` | `path`, `exists`, `on_path` |
| `PlanEntry` | `content`, `priority`, `status` |
| `RemoteRepo` | `id`, `name`, `full_name`, `description?`, `default_branch?`, `private`, `clone_url`, `pushed_at?` |
| `CloneRequest` | `clone_id`, `source`, `branch?`, `shallow`, `parent`, `name`, `ephemeral` |
| `ParsedRepo` | `host`, `owner`, `name`, `clone_url` |
| `SuggestSubject` | one of: `CommitMessage { project_id }`, `ProviderCheck { provider_id, role }` |
| `AssistLimits` | `label`, `context_tokens` |

**The record is what the store holds; the snapshot is what crosses the bus.** Keeping them apart is
what stops a stale health flag or a pane count from being written down and believed at the next
boot.

**The work's names carry the same distinction without a second type.** A `TaskRecord` is written
down, and `tasks.toml` holds exactly what crosses the bus, so there is nothing to keep apart: no
field on a task is like `health` or `open_panes`, which can only be known at the moment they are
asked for. A `WorkSession`, a `WorkAgent` and a `Turn` are the other way round — per-request payloads
with no store behind them, in the class `DirEntry` and `DirListing` are in. `WorkAgent.summary` is
the one field on that record no host store and no harness fills: it arrives with a
`ConversationNamed` and the interface folds it onto the record beside the title, so a surface
reads one place for both. It is absent for every agent nothing has named, which includes every
mock.

Fifteen enums travel inside those records. `ProjectHealth` is `Ok`, `Missing`, `NotADirectory`, or
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
`HostSettings` carries a `schema` — at 12 — and `isolate_agents`, which is whether an agent runs
confined, the one setting the host acts on rather than stores, read again at every spawn.
`agent_home` and `extra_grants` are the confined run's other two answers: an `AgentHome` of
`Inherit`, `Ephemeral` or `Named(String)`, defaulting to `Inherit`, and a list of `Grant` — a
`path` the user typed, absolute or `~`-prefixed, and whether the agent may `write` there,
read-only when nothing says. `agent_commands` is a `BTreeMap<String, String>` from a harness id to
the command line this machine runs for it instead of the harness's own name — a bare word looked up
on the login shell's `PATH`, an absolute path, or a launcher and its arguments. All three are
written by the interface and pass through `Settings::set`
unchanged, unlike `connections`, `oauth_apps` and `trusted_certs` below, which the host owns and
overwrites with what is on disk. What either means for a run belongs to the agent-manager
boundary, not here. It also
carries `projects_root` and `ephemeral_root`, the two folders a clone lands in: an absent or blank
one means the host's own default under its config root, so the interface offers a placeholder rather
than inventing a path it cannot read. `index_level` is how much of a project is indexed for every
project that does not say otherwise, and is `light` when nothing says. `assist` is an
`AssistProvider`, the one setting the assist family reads, and `auto_name_conversations` is whether
a conversation names itself once its agent has answered its opening prompt — **on by default, and
that default changes nothing on its own**, because a naming runs through `assist`, which is `Off`
until a user picks a provider. Both pass through `Settings::set`
unchanged like the interface's own fields above — unlike `ai_providers`, the records it may point
at, which the host overwrites from disk for the reason the connector fields are overwritten and one
more of its own: a record names a key filed under its id. A record written by
an older build still parses, because every field added since carries a default; only a newer schema
is refused.

`IndexLevel` — `none`, `light`, `full` — is **cumulative**: `full` is `light` plus a symbol table,
and the full-text half carries content search at both. A project's own `ProjectRecord.index` is an
`Option<IndexLevel>`, absent meaning it follows the setting above, so moving the application
default moves every project that never overrode it. `UpdateProject` carries that override as an
`IndexChange` — `Inherit` or `Set(level)` — rather than an `Option<Option<IndexLevel>>`, because
serde reads an absent field and an explicit `null` into the same outer `None` and "clear the
override" would become indistinguishable from "say nothing about it".

The conversation family's own enums are the Agent Client Protocol's and are named after it rather
than after anything here, so a reader can check them against
[`../references/acp-protocol.md`](../references/acp-protocol.md) directly. `ToolKind` is ACP's ten —
`Read`, `Edit`, `Delete`, `Move`, `Search`, `Execute`, `Think`, `Fetch`, `SwitchMode`, `Other` —
and carries the verb its block's header leads with. `ToolStatus` is `Pending`, `InProgress`,
`Completed` or `Failed`. `PermissionKind` is `AllowOnce`, `AllowAlways`, `RejectOnce` or
`RejectAlways`, and it is a **display hint**: it says which button reads as going ahead and which
reads as lasting, and it never changes what an `option_id` means. Remembering an "always" is the
agent's job, not the client's — Ubiq keeps no allow-list of its own, and echoes the id back
unchanged. `StopReason` is ACP's five plus `Failed`, which is
ours and means the run broke rather than the model declining. `ConfigCategory` — `Mode`, `Model`,
`ModelConfig`, `ThoughtLevel`, or an `Other` carrying whatever a harness invented — is a hint about
which picker draws an option and must never change what an id means.

`CliShortcutAction` — `Query`, `Install` or `Remove` — is the whole of what the interface may ask
about the `ubiq` command, and a `CliDir` is one directory the host considered, with whether it
exists and whether the shell would find a command in it. A directory that does not exist is still
offered: the first candidate is created on install.

`DiffBase` is `Head` or `Index`, and `DiffRowKind` is `Context`, `Added` or `Removed` — the marker a
textual diff puts at the front of a line, kept as a thing to draw rather than a character to strip.

`AssistReason` and `AssistProvider` are the assist family's, in
`crates/ubiq-proto/src/assist.rs`. The first is the closed set an unavailable answer maps onto and
carries a `code()` giving its kebab-case wire string; the second is `Off`, `OnDevice` or
`Api { provider_id }`, defaults to `Off`, and is the whole of what a `HostSettings` says about which
backend runs. The same module holds the API provider's records — `AiProviderKind`, `ModelRole`,
`AiProvider`, `AiProviderDraft`, `AiProviderInfo`, `AiModel` and `AiModelList` — and the rule that
governs all of them is that none carries a key.

Six of the fifteen are the work's, and all but `Speaker` carry the words they answer to — a `label()`,
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
| `BeginHarnessLogin` | UI → host | `agent_type`, `account`, `probe` | `HarnessLoginStarted`, or `HarnessLoginFailed` |
| `HarnessLoginStarted` | host → UI | `pane_id`, `agent_type`, `account`, `cols`, `rows` | — |
| `HarnessLoginCaptured` | host → UI | `agent_type`, `account` | — |
| `HarnessLoginFailed` | host → UI | `agent_type`, `account`, `error` | — |
| `HarnessLoginLink` | host → UI | `pane_id`, `url` | — |
| `CheckHarnessLogin` | UI → host | `agent_type`, `account` | `HarnessLoginStatus` |
| `HarnessLoginStatus` | host → UI | `agent_type`, `account`, `status` | — |
| `RenameAccount` | UI → host | `account`, `new_account` | `Accounts`, or `AccountError` |
| `DeleteAccount` | UI → host | `account` | `Accounts`, or `AccountError` |
| `DeleteHarnessLogin` | UI → host | `agent_type`, `account` | `Accounts`, or `AccountError` |
| `AccountError` | host → UI | `error` | — |

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

**An account is a home, so renaming and deleting are account-wide.** Several harnesses log in to
one account by writing into one directory, which is why `logged_in` is a list. `RenameAccount`
renames that home and every login inside it keeps working under the new name; `DeleteAccount`
removes them all. Signing a single harness out is the narrower operation — `DeleteHarnessLogin`
deletes only the files that harness itself declared, and leaves the rest of the home untouched.

**Signing out is not the same as deleting.** An account with an empty `logged_in` still exists — it
is a name with no login, and the next `BeginHarnessLogin` naming it fills it back in.
`DeleteAccount` is the one that leaves nothing.

**Validity is what the credential says about itself.** `HarnessLoginStatus`'s `status` is read out
of the stored credential's own expiry field; nothing calls the provider. So `Valid` means "not
expired", not "will work" — a token the provider revoked early still reads as `Valid` here.

**A link is an affordance, not a filter.** The host forwards a URL it saw in the login's output; it
does not remove it from the stream. The pane still shows the harness's real output, and the
`HarnessLoginLink` button only saves the user selecting text in a terminal.

**Re-authentication is an ordinary login.** There is no separate message: `BeginHarnessLogin`
naming an account that already exists re-runs the harness's flow, and the mtime rule that decides
capture (above) already distinguishes a fresh credential from the old one.

**`probe` swaps what runs, never what it runs under.** The policy rendered for a login is the
harness's own — computed from its program's symlink and shebang chain, see `agent_manager::
isolate::login_confined` — and `probe: true` only replaces the argv exec'd *after* that policy is
resolved with the user's plain shell, so a human can inspect exactly what the login sandbox
permits. It answers with the same `HarnessLoginStarted`/`HarnessLoginFailed` pair, but a probe
pane's exit is never treated as a login outcome: nothing is written to the credential's mtime, so
the host records no account and sends neither `HarnessLoginCaptured` nor `HarnessLoginFailed` for
it — the pane simply closes, which the UI reads for itself from `PaneExited` rather than waiting on
a host answer that will not come.

## The profile family

The thirteenth family, and the account family's neighbour. A **profile** is a saved setup — which
harness, as whom, with which model, reasoning level and permission mode, how many subagents at
once, and what to open with — and this family is how one is listed and written. It is deliberately three messages: profiles are stored beside accounts by the
harness library, so they fail the same way and share `AccountError` rather than minting a second
error variant.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ListProfiles` | UI → host | — | `Profiles` |
| `Profiles` | host → UI | `profiles` | — |
| `SaveProfile` | UI → host | `profile` | `Profiles`, or `AccountError` |

**There is no delete.** A profile is a saved setup, and a stale one costs a row in a list — not a
credential on disk, which is what makes deleting an account worth a message and deleting a profile
not.

**References only, like the account family.** `ProfileInfo` names an account, a model, a reasoning
level and a mode by id; nothing here is credential material or a path. `None` on a field means the
profile does not mention that axis and a lower layer decides, which is the library's
replace-by-default rule crossing the bus intact.

**A profile answers the same questions a start does**, which is why the record grew `thinking`,
`max_subagents` and `prompt` when the start form did: one form asks both, so anything the form can
answer is something a profile can save. The last two are the interface's own — no harness has a
subagent flag and an opening prompt is a turn, not a launch — so the library records them and
never reads them, and it is the start that acts on them.

**A profile named on `StartConversation` seeds the picker, it does not bypass it.** The host reads
the profile's record and copies its model, level and mode into the pending conversation's picks, so the
`ConfigOptions` the window draws show the profile's choices and a launch that nobody touched
sends them. It has to work this way round: the host passes the picks as flags, and a flag outranks
the profile inside the library's `resolve`, so a profile left unseeded would be displayed wrong and
then launched over. `account`, `model`, `thinking` and `mode` on the same message stay separate and
win over the profile's, which is what "the user picked this one" means — a profile is the default
a form opened on, and every field the user then changed is the user saying otherwise.

## The command-line family

The tenth family by position, and the smallest: one request and one answer, about the `ubiq` script on the
shell's `PATH`. It names no project, no pane and no account, because what it is about is the
machine.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `CliShortcut` | UI → host | `action` | `CliShortcutState` |
| `CliShortcutState` | host → UI | `installed?`, `stale`, `target?`, `candidates[]`, `error?` | — |

**The request carries no directory.** `CliShortcutAction` is `Query`, `Install` or `Remove`, and
nothing else rides with it. Which directory the shortcut belongs in is a fact about the machine's
`PATH`, and the host is the half allowed to look — so every path in this exchange travels host to
UI, and the interface draws what it was told rather than proposing anywhere.

**One answer serves all three actions.** `Install` and `Remove` are answered with the same
`CliShortcutState` a `Query` is, so one path in the interface draws the section and a failed write
is a state with an `error` on it rather than a variant of its own. `error` is one sentence about the
last write or delete, never a stack trace.

**`installed` and `stale` are different questions.** `installed` is where a shortcut Ubiq wrote was
found, recognised by the marker line it carries; `stale` says that shortcut launches a build other
than the one running. A `ubiq` on the `PATH` that Ubiq did not write is reported as neither, because
it is not Ubiq's to name or to delete.

**`target` is where a write would land, and its absence is a refusal.** No candidate usable means no
`target`, which is what the interface draws its disabled button from. `candidates` is every
directory considered, in the order it was considered, each a `CliDir` — so a machine that fits none
of them shows why.

## The connector family

The eleventh family, and the account family one layer out. An **account** is one authentication a
*harness* runs as; a **connection** is one authentication an external *service* runs as — a GitHub
login, a GitLab identity on a company's own install, a Google Workspace grant. This family is how
one comes into being, where its token lives, and how the interface learns which exist.

Several connections per provider is the ordinary case rather than a feature: nothing in the record
is unique per provider, and every consumer takes a connection id rather than a provider name.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ListConnections` | UI → host | — | `Connections` |
| `Connections` | host → UI | `connections`, `bundled` | — |
| `BeginConnect` | UI → host | `connect_id`, `provider`, `instance?`, `label`, `auth`, `client_id?`, `oauth_app?` | `ConnectPending`, then `ConnectCaptured` or `ConnectFailed` |
| `ConnectPending` | host → UI | `connect_id`, `stage` | — |
| `ConnectCaptured` | host → UI | `connect_id`, `connection` | `Connections` follows |
| `ConnectFailed` | host → UI | `connect_id`, `error` | — |
| `CancelConnect` | UI → host | `connect_id` | — |
| `SubmitConnectSecret` | UI → host | `connect_id`, `secret` | `ConnectCaptured` or `ConnectFailed` |
| `RenameConnection` | UI → host | `connection`, `label` | `Connections` or `ConnectorError` |
| `DeleteConnection` | UI → host | `connection` | `Connections` or `ConnectorError` |
| `CheckConnection` | UI → host | `connection`, `probe` | `ConnectionStatus` |
| `ConnectionStatus` | host → UI | `connection`, `status` | — |
| `ConfirmCertificate` | host → UI | `connect_id`, `origin`, `cert` | `TrustCertificate`, or a cancel |
| `TrustCertificate` | UI → host | `connect_id`, `origin`, `sha256` | resumes the flow, or `ConnectorError` |
| `ForgetCertificate` | UI → host | `origin` | `Settings` or `ConnectorError` |
| `SaveOauthApp` | UI → host | `id?`, `provider`, `name`, `origin?`, `client_id` | `Settings` or `ConnectorError` |
| `DeleteOauthApp` | UI → host | `id` | `Settings` or `ConnectorError` |
| `SetAppSecret` | UI → host | `app`, `secret` | `Settings` or `ConnectorError` |
| `ClearAppSecret` | UI → host | `app` | `Settings` or `ConnectorError` |
| `ConnectorError` | host → UI | `error` | — |

**`ConnectionInfo` carries no material.** It is the stored record — id, provider, label, instance,
auth, scopes, the provider's own name for the identity, the client id it was made with and the
registration it was made under — plus the status the host read out of the token and whether the
instance is pinned. The log sink listens to the same bus, so the account
family's rule applies unchanged: a token here is a token in a log a user might paste into an issue.

**Two variants carry material, and the rule is the type rather than the list.**
`SubmitConnectSecret` takes a pasted access token and `SetAppSecret` a user-supplied client secret.
Both carry a `Secret`, whose `Debug` prints `Secret(***)` and which has no `Display`, no `Deref` and
no `AsRef<str>` — so a whole `Message` can be logged, as both halves do, without material reaching
the sink. The rule to hold is **material crosses only in a `Secret`, and a `Secret` is never
printed**; see [D65](./decisions.md). A client *id* is public and rides the settings blob like any
other setting, so `SaveOauthApp` carries one in the clear while the secret keeps its own variant.

**An application registration is keyed by an `OauthAppId` and by nothing else.** One provider and
one instance may carry several, so `SaveOauthApp`, `DeleteOauthApp`, `SetAppSecret`,
`ClearAppSecret`, `BeginConnect` and the `Connection` record all name the id. `SaveOauthApp` with
`id: None` creates and the host mints one — a registration exists once it is written, the discipline
`ConnectId` and `ConnectionId` already follow — and with `Some` rewrites that registration in place,
so a rename disturbs neither the secret filed under it nor the connections that reference it. A
blank name or a blank client id is refused with `ConnectorError`; so is an instance that is not an
absolute `http` or `https` URL. Deleting a registration clears its client secret with it.

**An `OauthApp` read from an older blob is filled deterministically.** Neither the id nor the name
existed before, so a record without them takes a name derived from its provider and origin and an
id derived from the same pair — derived rather than minted, because the host re-reads the settings
file on every question it asks of it and a minted fill would answer a different id each time.
`HOST_SETTINGS_SCHEMA` is 6 for the change, so a build that predates it refuses the file rather than
rewriting it without the ids the keychain is now keyed by.

**A saved remote host carries a name and an address, and never its token.** `HostSettings` grows a
`remote_hosts` of `SavedRemoteHost`, so a host reached once can be offered again after a restart —
but a token is credential material, and the rule that keeps it off a record is the same one
`connections` and `oauth_apps` follow. Those two have somewhere to put it, the secret store the
harness library owns; a remote host's token has no such home, so it is not written down at all and
a reconnect asks for it again. `HOST_SETTINGS_SCHEMA` is 7 for the field, which an older blob
parses by defaulting rather than being refused, because a record with no remote hosts in it is a
complete record.

Unlike `connections`, `oauth_apps` and `trusted_certs`, this field is the interface's to mutate and
rides `SetSettings` whole. Those three are re-read from disk on every write because a flow running
in the background can finish while a dialog holds a stale copy of them; nothing adds or forgets a
saved host except a person on that settings page, so there is no concurrent writer to clobber.

**`bundled` on `Connections` says which providers this build ships an application for.** It is a
compile-time fact of the host — every built-in client id is an `option_env!` — and the interface's
only way to know it, so the connect flow offers a "Default" exactly where one can be honoured. An
interface that guessed would open a browser at an authorization URL with no client id in it.

**The callback URL is a constant, not a payload.** `ubiq_proto::connectors::OAUTH_REDIRECT` and
`OAUTH_REDIRECT_PORT` sit on the contract because both halves need the same string for different
reasons: the host binds the port, the interface shows the URL to whoever is registering the
application. A second spelling of it in either half would be a string that drifts from the one
actually listened on.

**Creating a connection is completing a flow.** There is no `AddConnection`. `BeginConnect` mints
nothing but a flow — the `connect_id` is the interface's, on the search family's discipline — and a
connection exists only once a token is stored, so an abandoned flow leaves nothing behind. Exactly
one `ConnectCaptured` or `ConnectFailed` ends a flow, and the interface discards any stage naming an
id it no longer holds.

**`CheckConnection` has two modes, and only one touches the network.** `probe: false` is what the
list draws: the stored token's own expiry, no request and no latency, so the path that runs on every
render calls nobody. `probe: true` is the "Check" button, and it runs as a flow with a `connect_id`
like any other — a handshake must never happen on the thread that carries keystrokes. That is also
what makes it the one place an existing connection's certificate can be confirmed or replaced, and
why `ConfirmCertificate` needs only a `connect_id` to say what to resume.

**A pin is answered, never assumed.** `TrustCertificate` must carry the same `sha256` the host
offered; anything else is a `ConnectorError` and the flow stays stopped. That is what makes the
confirmation meaningful rather than a formality the interface can click through on the user's
behalf, and why the payload is a fingerprint rather than a boolean. `origin` is what gets pinned, so
the trust is instance-wide: two connections to one server find the same row, and a pin outlives the
flow that created it — including one the user then abandons, because their answer was about the
server and the server has not changed.

**Deleting a connection deletes the token, and only the token.** It does not touch the pin, which
belongs to the instance and may be why another connection still works; `ForgetCertificate` is that
operation, keyed by origin. There is no "sign out but keep the name": unlike a harness account,
which is a home directory that survives its login, a connection with no token is nothing.

**Status is what the token says about itself.** `ConnectionStatus` carries the same `LoginStatus` the
account family uses, read from the stored blob's own expiry. Nothing calls the provider, so `Valid`
means "not expired" rather than "will work" — a revoked token reads `Valid` until something uses it.

**The records live in the host settings blob**, as `connections`, `oauth_apps` and `trusted_certs` —
already persisted, versioned and round-tripped, so no new store and no new messages to read them.
The host owns those three fields: the interface mirrors the whole record and writes the whole of it
back, so a `SetSettings` carrying its older copy would otherwise lose a connection a flow had just
made. `ListConnections` exists for the refresh-after-a-flow case rather than as the primary path.

## The repository family

The twelfth family, and the first consumer of a connection. It answers two questions about a remote
— which repositories an identity has, and which branches one of them holds — and then clones one
into a folder. It sits between the connector family and the project family in
`crates/ubiq-proto/src/messages.rs` because that is where it sits in life: it starts at a connection
and ends at a project.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ListRepos` | UI → host | `query_id`, `connection`, `query` | `Repos` or `RepoError` |
| `ListRepoBranches` | UI → host | `query_id`, `source` | `RepoBranches` or `RepoError` |
| `CloneRepo` | UI → host | `request` | `ClonePending`, then `ProjectAdded` or `CloneFailed` |
| `CancelClone` | UI → host | `clone_id` | — |
| `Repos` | host → UI | `query_id`, `repos`, `truncated` | — |
| `RepoBranches` | host → UI | `query_id`, `branches`, `default` | — |
| `RepoError` | host → UI | `query_id`, `error` | — |
| `ClonePending` | host → UI | `clone_id`, `stage` | — |
| `CloneFailed` | host → UI | `clone_id`, `error` | — |

**There is deliberately no clone-success message.** A finished clone registers the project, so
`ProjectAdded` — already a broadcast — is the success signal, and every window's picker learns about
the clone rather than only the one that asked. A second variant saying the same thing would be a
second truth for the interface to reconcile, and the one that arrived first would win.

**A `RepoQueryId` and a `CloneId` are the interface's**, on `ConnectId`'s stale-answer discipline:
the asker mints the id, every reply carries it, and a reply naming an id the interface no longer
holds is discarded rather than drawn. That is what makes a filter typed faster than the network
answers safe — the answer to the query before last is thrown away on arrival.

**A `RepoSource` is a `Connection` or a `Url`, and the second needs no connection at all.** A public
repository is cloned from a pasted URL, which is why `ListRepoBranches` takes a source rather than a
connection: a connection's branches come from the provider's branch API, and a bare URL's come from
an anonymous ref listing against the remote. `parse_repo_url` in
`crates/ubiq-proto/src/repos.rs` is the single place a repository URL is sniffed, so the modal, the
navigator and the host all agree on what one is. It normalises whatever it is given to
`https://<host>/<owner>/<name>.git`, `git@host:owner/name` included, so an ssh remote is understood
well enough to be named as unsupported rather than read as "not a repository"; refusing it is the
caller's, which is why `ParsedRepo` carries no flag for it.

**A destination is a parent folder and a name, not a path.** `CloneRequest` carries the two apart
because the modal offers them as two fields, and joining them is the half that touches disk — which
is also where a `name` that is not a single path component is refused.

**Listing is bounded and says so.** `Repos` carries `truncated`, because the provider's own
membership listing is paged and the host stops after a fixed number of pages. The interface filters
what it holds in memory; only a filter that comes up empty against a truncated listing asks the
provider to search.

**A clone reports a stage, not a percentage.** `CloneStage` is `Resolving`, `Counting`, `Receiving`,
`CheckingOut` or `Registering`, and `ClonePending` is throttled host-side rather than sent per
object. `CloneError` is `Network`, `Auth`, `NotFound`, `Exists`, `Unsupported` or `Refused` —
`Unsupported` is what a provider with no repository listing and an `ssh` URL both answer, and
`Refused` is a request the host would not run at all. `RepoError` carries the same `CloneError`
rather than a kind of its own, because a listing fails for the same reasons and the interface writes
the same sentence. `CancelClone` and a failure are the same outcome on disk: the partial destination is
removed, so nothing half-cloned is ever registered.

## The assist family

The fifteenth family, and two things at once: asking for a sentence, and configuring who writes it.
**No variant carries prompt text**, because a request names a subject and the host owns every word
that reaches a model — the availability pair asks about the host, the suggestion variants ride a
`SuggestId` the interface mints before its first request hits the wire, and the provider variants
carry a record and, once, a key.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `GetAssist` | UI → host | — | `Assist` |
| `Suggest` | UI → host | `suggest_id`, `subject` | `SuggestChunk`\*, then `Suggestion` or `SuggestError` |
| `CancelSuggest` | UI → host | `suggest_id` | — |
| `GetAiProviders` | UI → host | — | `AiProviders` |
| `AddAiProvider` | UI → host | `draft`, `key` | `Settings` + `AiProviders`, or `AiProviderError` |
| `UpdateAiProvider` | UI → host | `provider_id`, `draft`, `key?` | `Settings` + `AiProviders`, or `AiProviderError` |
| `ForgetAiProvider` | UI → host | `provider_id` | `Settings` + `AiProviders`, or `AiProviderError` |
| `ListAiModels` | UI → host | `provider_id`, `refresh` | `AiModels` or `AiProviderError` |

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `Assist` | host → UI | `available`, `reason?`, `detail?`, `limits?` | — |
| `SuggestChunk` | host → UI | `suggest_id`, `text` | — |
| `Suggestion` | host → UI | `suggest_id`, `text` | — |
| `SuggestError` | host → UI | `suggest_id`, `error` | — |
| `AiProviders` | host → UI | `providers` | — |
| `AiModels` | host → UI | `list`, `refreshed` | — |
| `AiProviderError` | host → UI | `provider_id?`, `error` | — |

**The interface names a subject and never a prompt.** A `SuggestSubject` carries ids only —
`CommitMessage { project_id }` says *this project's commit message*, `ProviderCheck { provider_id,
role }` says *make sure this provider's fast model answers* — and nothing about how to ask for
either. Every prompt string, every instruction and every truncation budget lives in
`crates/ubiq-host/src/assist/subject.rs`, so the host is where a wording is tested, against a fake
backend and with no window. A family that accepted prompt text would be a generic model console
whatever it was called, and every later feature would reach for it (`D83`).

**A subject is not the only thing that reaches a wording, though it is the only thing an interface
may name.** `subject.rs` holds one prompt with no `SuggestSubject` in front of it:
`conversation_title` is asked for by the coordinator, not by a window, because naming a
conversation is not something anyone requests — it is something the host notices it can do once an
agent has answered its opening prompt. The wording still lives in that module with every other
wording, so it is tested the same way, and `naming` beside it is the reading of the two lines it
asks for: a title, and a summary that is `None` where the model answered one line. What the rule
protects is the direction — the host owns every word that reaches a model — and that is untouched;
what it does not promise is that every prompt in the module answers a `Suggest`.

**The naming's material is the opening exchange, split down the middle.** The asked half is the
first `PromptAgent`'s text, which never leaves the coordinator, and the answered half is the
agent's first message, accumulated by the pump in `crates/ubiq-host/src/conversation.rs` from the
chunks sharing a `message_id` — skipping any carrying a `parent_tool_use_id`, because a subagent's
prose is not what the conversation is about — and published whole at `TurnEnded` for
`Conversation::first_reply` to hand over. Each half gets half of `AssistLimits`'s budget, so a long
opening prompt cannot crowd out the reply that says what was done about it. It is the one subject
whose material is not ASCII by construction, so `subject.rs` clips it on a character boundary
rather than a byte one.

**`ProviderCheck` is the one subject that names its own backend.** Every other subject is answered
by the provider the setting points at; a user checking a key they have just typed is asking about
*that* provider, so the coordinator builds a backend for the named record, uses it for the one
request and drops it. The held backend is untouched, which is what lets a provider be tested
without being selected.

**A suggestion is advisory.** It fills an editable field the user was going to type in: it writes
nothing into a repository, so a suggestion that never arrives leaves the mechanical name exactly as
it was. That is why `SuggestError` is a sentence and never a state the interface has to unwind
(`D83`). **The half of that rule about renaming holds only for this family.** A
`ConversationNamed` does replace a name the user did not type, which is a departure `D90` records
and confines: what it replaces is a mechanical placeholder, the setting behind it is one checkbox,
and nothing outside the window is written.

**Availability is asked, never inferred.** `GetAssist` is the only way the interface learns whether
a suggestion can be produced — there is no OS-version comparison and no device allow-list on either
side. `AssistReason` is the closed set the answer maps onto — `UnsupportedPlatform`,
`DisabledBySetting`, `UnsupportedOs`, `DeviceNotEligible`, `NotEnabled`, `ModelNotReady`,
`Unavailable`, each with a kebab-case `code()` for the wire — and `detail` is a sentence the host
wrote. A vendor name appears in it only for a provider the user configured; a backend nobody asked
for describes itself as the on-device model (`D84`).

**`AssistLimits` is the backend's fact, not the host's.** Its `label` and `context_tokens` come from
whichever backend `assist::select` chose, and the truncation in `subject.rs` measures a subject's
material against that number rather than a constant — so the same subject is cut differently behind
a different model, and the interface reads the label rather than composing one.

**A suggestion streams, and an interface may ignore that it does.** Zero or more `SuggestChunk`
precede the `Suggestion` that ends an id, and every chunk concatenated is that message's text
(modulo the surrounding whitespace `Suggestion` trims). So an interface that draws only the final
message is correct, and one that draws chunks shows a first token instead of a spinner. A backend
that cannot stream sends exactly one chunk, because `Assist::stream` defaults to generating and
handing the answer over whole — which is why the coordinator forwards chunks without knowing who is
behind them (`D87`). Chunks already delivered are not a partial answer: a `SuggestError` for the same id
discards them.

**Cancellation is best effort, and now reaches further.** `CancelSuggest` sets the flag that stops
the reply from being sent; against a streaming backend it also stops the *reading*, because the
sink the coordinator hands down returns `false` and the backend drops the connection. It still does
not stop a model that has already been asked. A suggestion runs on a one-off named thread with a
deadline because generation blocks, and the coordinator must keep answering every other window
while it thinks.

**`Suggest` is the one variant whose project is inside its payload's payload.**
`Message::project_id()` has its own arm for it, reaching through the subject, because a project id
sits in `SuggestSubject` rather than beside it. Nothing in the family names a pane, so no variant
appears in `pane_id_of`.

**The setting is host-layer and off by default.** `HostSettings.assist` is an `AssistProvider` —
`Off`, `OnDevice` or `Api { provider_id }`, defaulting to `Off`. `Off` is what `DisabledBySetting`
reports, and it is distinct from every reason that describes the machine; an `Api` id no record
answers to is `Unavailable`, because a user who chose a provider that has since been deleted has
something to repair rather than a preference to re-read.

**A provider list says what is configured, never what is chosen.** `AiProviderInfo` is the record
plus `has_key`, and nothing more: which provider assistance runs on is `HostSettings.assist`, which
every window already mirrors, so a settings row derives lit-ness rather than being told it. That is
the same call `ConnectionInfo` makes about a pinned certificate, and it is what keeps a moved
setting from needing a second broadcast to keep a list honest.

**A provider record carries no key, and the records are the host's to write.** `AiProvider` is an
id, a kind (`openai-compatible`, `anthropic`, `gemini`), the user's name for it, an optional base
URL, a fast model and an optional smart model — and never material. The key crosses once, in a
`Secret`, on `AddAiProvider` or `UpdateAiProvider`, and goes straight to the OS secret store under
the record's id; every record afterwards says only whether one is filed, as
`AiProviderInfo.has_key`. `UpdateAiProvider` with `key: None` leaves the stored key alone, which is
the only thing an edit that renames can do — the interface is never told a key, so it cannot send
one back.

`HostSettings.ai_providers` is therefore host-mutated, joining `connections`, `oauth_apps` and
`trusted_certs`: what the interface sends for it in a `SetSettings` is discarded and what is on disk
is kept. The reason is stronger here than for the other three. A stale interface copy that dropped a
record would strand that record's key in the keychain — unreachable, unlistable and still there —
which is why every change is one of the three provider variants and none of them is a settings
write (`D86`). `HOST_SETTINGS_SCHEMA` is at 10 for the field and for `AssistProvider::Api`.

**`agent_commands` rides `SetSettings` whole, the same as `projects_root`.** There is no per-key
write for it: the Add-harness login modal reads and writes the map through the ordinary `Host`
settings blob, keyed by harness id. `HOST_SETTINGS_SCHEMA` is 11 for the field — an older build
drops every override on its next write, and a harness only reachable through one stops starting
until the override is set again.

**`auto_name_conversations` rides `SetSettings` whole too, and defaults to on.** It is the second
setting the assist family reads and the only one that is not about a backend: whether the host
names a conversation from its opening exchange at all. `HOST_SETTINGS_SCHEMA` is 12 for the field,
and this one's fallback runs the wrong way — an older build that drops it turns naming back **on**
for a user who had switched it off, so the setting reverts to *calling a model* rather than to not
calling one. It is separate from `assist` because the two questions are separate: a user may want a
commit message written on request and still not want every conversation to cost a call nobody
asked for.

**A provider is added before its models are known.** A draft's `fast_model` may be blank and
normally is: a model picker needs an id to name and a key to call with, and both exist only once
`AddAiProvider` has written them — so the host lists the new provider's models itself, immediately,
and the user picks from the answer rather than typing a model name from memory. A provider with a
key and no model is a real state, reported as unavailable with a detail saying which half is
missing. A name is the only field the host insists on.

**`ModelRole` is the whole of model selection.** A provider configures a fast model and optionally a
smart one, a subject asks for a role rather than for a model, and `AiProvider::model_for` falls back
to the fast one — so a subject that wants the capable model always has something to run. Two roles
rather than a model per call site, because the choice a user makes is about cost and latency and not
about a subject.

**A model list is cached by the host and refreshed only when asked.** `ListAiModels { refresh:
false }` is served from the host's cache and calls the provider only when there is nothing cached,
which is what makes a searchable model picker openable without a network; `refresh: true` is a
control the user pressed, and runs on a thread of its own. `AiModels.refreshed` says which of the
two happened and `AiModelList.fetched_at_ms` says when, so a stale list says so rather than looking
fresh. The cache is not in the settings blob: it is derived from what a provider said, the host is
its only writer, and several hundred model names have no business in a file a user opens to change
a preference.

**`AiProviderError` is the family's one refusal.** A name or a fast model that is blank, a key the
platform's secret store would not keep, a provider a model list could not be got from, an id no
record answers to. Its `provider_id` is present when the refusal is about one particular provider
and absent when it is about the list. Nothing was changed on the way to failing, so it is a line to
show on a settings page rather than a state to unwind.

## The notification family

The sixteenth family, and the only one whose messages name nothing in Ubiq at all: a notification
names an **origin** — a family, an optional actor inside it, an optional category of event — and
that origin is what a mute rule matches on. **The host decides whether a notification is silenced**,
because the rules live there and because the operating system must be told at most once however
many windows are open. Both host → UI variants are broadcast to `To::Everyone`: the bell is drawn
in every window, and two badges that disagree is the bug this rules out by construction.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `RaiseNotification` | UI → host | `request` | `NotificationRaised` |
| `ListNotifications` | UI → host | — | `NotificationsState` |
| `ReadNotifications` | UI → host | `id?` — `None` is all of them | `NotificationsState` |
| `DismissNotifications` | UI → host | `id?` — `None` is all of them | `NotificationsState` |
| `MuteNotifications` | UI → host | `scope`, `max_level`, `duration` | `NotificationsState` |
| `UnmuteNotifications` | UI → host | `scope` | `NotificationsState` |
| `NotificationRaised` | host → UI | `notification` (boxed) | — |
| `NotificationsState` | host → UI | `state` (boxed) | — |

**A raiser is on either side.** `RaiseNotification` is a UI → host message because the host is what
files a record, but the host raises its own the same way internally — a subsystem hands the
notification centre a `NotificationRequest` and the same rules apply. Nothing raises a notification
by drawing one.

**Muted is the host's verdict, not the raiser's wish.** A request carries `muted` (arrive quietly)
and the host ORs it with every rule in force; `Notification.muted` is the answer. A muted
notification is still filed and still counts against the badge — it only stops the bell flashing
and stops the operating system hearing about it.

**`os` is opt-in, always.** A request that does not ask for it never reaches the desktop, and a
request that does is still suppressed when the notification comes out muted.

**One rule per scope.** `MuteNotifications` on a scope that already has a rule replaces it, which is
what makes "mute this agent for an hour" idempotent from a menu. A rule whose `until` has passed
silences nothing and is dropped the next time the centre is touched.

**The state is sent whole.** `NotificationsState` carries the history — newest first, capped at
`HISTORY_CAP` (200) — and the rules in force. There is no patch protocol: the list is small, it
changes on a click, and a whole-state message cannot leave two windows disagreeing.

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
  fixing that would cost a sequence number on the wire. The host browse family shares that same
  worker and queue, on the same rule.

**The socket framing exists, in `crates/ubiq-proto/src/wire.rs`.** A frame is a 4-byte big-endian
length prefix followed by the message body, so a reader knows exactly how many bytes to read before
it decodes anything. A prefix claiming more than `MAX_FRAME` (64 MiB) is refused before any
allocation for the body is made — the terminal family already chunks as the pseudo-terminal hands
bytes back, so nothing this contract carries needs a body near that size, and a claim past it is a
corrupt or hostile header rather than a message running long. `encode`/`decode` turn a `Message`
into a body and back with no prefix, for callers that frame differently; `write_frame`/`read_frame`
add it. A peer that closes cleanly between frames is `WireError::Eof`, kept apart from a torn frame
or a real I/O failure, so a socket pump does not have to guess which one happened from an `io::Error`
alone.

**The body is MessagePack via `rmp-serde`, and self-describing is the reason, not a side effect.**
`ProjectSnapshot` flattens a `ProjectRecord` into itself with `#[serde(flatten)]`, and dozens of
optional fields across the message set carry `skip_serializing_if` — both require a format that
carries field names on the wire and can deserialise into a self-describing shape (`deserialize_any`),
which postcard and bincode do not provide. `wire.rs` encodes with `to_vec_named` rather than the
compact positional `to_vec` for the same reason: a positional encoding has no map for `flatten` to
merge into. Self-describing also means a remote host and a UI built at different revisions do not
have to agree on field order to decode each other's frames — a fact worth having before either half
can be on the other end of a socket.

**The three byte-vector fields on the hot path are `serde_bytes`.** `TerminalOutput.bytes`,
`TerminalInput.bytes` and `WriteProjectFile.bytes` carry `#[serde(with = "serde_bytes")]`, so
`rmp-serde` encodes each as one `bin` blob instead of one MessagePack integer per byte — the
difference between a terminal frame close to its payload size and one several times larger. A test
in `wire.rs` asserts a terminal frame stays close to its payload size, and fails if that attribute is
ever dropped.

## Adding a variant

1. Decide the family. If it names a pane, the pane family. If it names a project **and a path inside
   it**, the file family. If it names a project **and a piece of work inside it** — a task, a step or
   an agent — the work family. If it names a project **and a search inside it**, the search family.
   If it names a project alone, the project family. Otherwise the
   session family.
   If it names an **agent** and carries something that agent said, the conversation family.
   If it names nothing in Ubiq at all and asks about the machine, the command-line family.
   If it names an **absolute path on the host's own filesystem, with no project yet to be relative
   to**, the host browse family.
   If it names a **connection** at an external service, or a flow authenticating one, the connector
   family. If it names a **remote repository** — listing one, or cloning one into a project that
   does not exist yet — the repository family. If it names a **subject Ubiq wants a sentence for**
   and carries no prompt, or configures the provider that would write it, the assist family.
   If it names **nothing in Ubiq and reports that something happened**, the notification family.
2. Add the variant to the enum in `crates/ubiq-proto/src/messages.rs`, with an owned payload — no
   borrowed data, no handles, nothing that fails to serialise.
3. Add a row to the table above, in the same commit.
4. Handle it in the coordinator's dispatch. A message the coordinator receives but ignores is worse
   than one that does not exist.
5. If it carries a `pane_id`, add it to `pane_id_of` in `crates/ubiq/src/app/hosts.rs`. That match
   is how the interface decides which of its hosts a message belongs to, and its catch-all arm
   answers "no pane" — so a pane-carrying variant left out of it routes to whichever host is
   active rather than to the one that owns the pane. With one host attached nothing goes wrong,
   which is what makes the omission worth a step of its own here.
6. If the variant makes a structural choice, append a row to [`decisions.md`](./decisions.md).

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
