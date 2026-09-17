---
id: wip-kb
title: The knowledge base — sources, the write half, and what is next
kind: wip
status: current
summary: A project's knowledge base as it stands — a per-project list of sources persisted as one TOML file, a folder read where it lies, a git repository cloned and refreshed, an internal wiki, a host-side write half (`kb/ops.rs`) behind six new messages, the `ubiq-kb` MCP server that reaches it, and the explorer's right-click menu that reaches it from the interface — and the one piece the interface has not caught up to, a save path from the document on screen.
read_when: you are touching the knowledge base's sources, its git sync worker, its write half, its `ubiq-kb` MCP server, or its explorer panel or centre
updated: 2026-09-17
verified: 2026-09-17
code_anchors: [crates/ubiq-proto/src/kb.rs, crates/ubiq-host/src/kb/mod.rs, crates/ubiq-host/src/kb/ops.rs, crates/ubiq-host/src/kb/store.rs, crates/ubiq-host/src/kb/sync.rs, crates/ubiq-host/src/mcp/kb.rs, crates/ubiq/src/state/kb.rs, crates/ubiq/src/app/kb.rs, crates/ubiq/src/ui/kb/mod.rs, crates/ubiq/src/ui/kb/source_form.rs, crates/ubiq/src/ui/file_dialog.rs, crates/ubiq/src/ui/sink/project.rs, crates/ubiq-proto/src/messages.rs, crates/ubiq/tests/kb.rs]
depends_on: [tech-architecture, tech-transport, feat-workbench, tech-decisions]
---

# The knowledge base — sources, the write half, and what is next

## What is built

The knowledge base is the documents half of a project: markdown, plain text and — once a viewer is
wired behind it — diagrams and images, read from one or more sources that need not be inside the
project at all. `crates/ubiq-proto/src/kb.rs`'s own header states the difference from the project
explorer plainly: a knowledge base has **several sources**, and the project's own folder is the
common case rather than a special one.

A source ([`KbSource`]) is a name, an id, a [`KbOrigin`], a filter of space-separated globs matched
against a file's name at any depth, and a [`KbAccess`]. `KbOrigin` has three arms: `Folder { path }`,
read where it lies; `Git { url, branch?, store }`, fetched onto a directory of its own, where
[`KbStore`] says whether that directory is the machine's temporary area (`Cache`), the project's own
area of the config root (`Internal`, the default, `D30`'s guarantee) or a `.ubiq/kb` folder inside
the project itself (`Project`, the one arm that writes there on purpose); and `Internal`, a wiki
Ubiq keeps for the project itself, always writable and with nothing to fetch. The whole list for a
project rides one message rather than one row per edit — `SetKbSources` — because it is a short
list edited in a settings form, and a reorder, a rename and a removal are one fact together rather
than three.

**The host.** `crates/ubiq-host/src/kb/mod.rs`'s `Kb` is a concrete service, the same shape
`store::usage::Usage` takes: one per coordinator, holding the config root and an `overrides` map for
the two states disk cannot answer — `Syncing` while a thread is fetching, `Failed` after an attempt
that left nothing behind. Every other state is derived: a folder source reads `Ready` when the path
is a directory and `Failed` otherwise; a git source reads `Ready` when its clone directory looks like
a repository and `Pending` otherwise; an internal wiki is `Ready` the moment it is asked about, since
`Kb::base_path` creates its directory on demand. `Kb::set_sources` writes the list to disk and starts
a fetch for any git source that is new or was left `Pending`/`Failed`; `Kb::sync` is the manual retry
a settings row's own control or the explorer's row asks for, and is a no-op for a folder or a wiki.
`Kb::base_path` resolves once where a source's documents actually are, and nothing else in the module
composes that path a second time.

`crates/ubiq-host/src/kb/store.rs` is the list itself: one TOML file at
`<config root>/projects/<project ulid>/kb.toml`, `version` at the top, written atomically, missing
read as an empty list rather than an error — `store::file::FileTaskStore`'s own convention.
`crates/ubiq-host/src/kb/sync.rs` is the git fetch, on a thread of its own, on the shape
`repos::clone` set: a first clone requires `https://` and removes what it wrote if it fails, a
refresh fetches and hard-resets whatever branch is actually checked out — read off `HEAD`, not off
the source's own `branch` field, since a source edited to name a different branch is a new checkout
once its directory is gone. Progress is throttled the same 250ms `repos::clone::THROTTLE` uses, and
there is no cancel: a fetch already running is left to finish rather than torn down under a working
tree it might be resetting.

**The write half.** `crates/ubiq-host/src/kb/ops.rs` is six free functions — `write_file`, `create`,
`rename`, `delete`, `absolute`, `reveal` — rather than methods on `Kb`, so a caller with no bus in
reach (the `ubiq-kb` MCP server, below) can use them too. Every mutating one opens with
`KbSource::is_writable` — `ReadWrite` access, or any `KbOrigin::Internal` source whatever `access`
says — and refuses before it touches disk; containment is `files/path.rs`'s, the same resolver the
file family uses, so a second implementation never has the chance to drift from the first. The
filter (`KbSource::admits`) is never consulted here: it decides what a source's tree *shows*, and
gating a write on it would refuse a file the user just named for the crime of not matching the glob
about to hide it. The coordinator answers six new UI → host messages through `ops` —
`WriteKbFile`, `CreateKbEntry`, `RenameKbEntry`, `DeleteKbEntry`, `RevealKbPath`, `AskKbPath` — with
`KbChanged` (naming the directory to re-list) or `KbFileError` on failure, `KbPath` for
`AskKbPath`'s answer, and nothing on a successful `RevealKbPath`. `tech/transport-contract.md`'s Kb
table has the full shape of all eight.

**The `ubiq-kb` MCP server.** `crates/ubiq-host/src/mcp/kb.rs` is the fifth built-in server, beside
`test`, `project-info`, `manage-ubiq-tasks` and `use-task`, so an agent Ubiq started can read and
write the project's documents without being told where they are on disk. Eight tools:
`list_kb_sources`, `list_kb_documents`, `read_kb_document`, `write_kb_document`, `create_kb_entry`,
`rename_kb_entry`, `delete_kb_entry` and `sync_kb_source`.

Every document is addressed by one string, `<source>/path/to/file`, where `<source>` is the
source's **name** as the user set it — the thing a model reads in one listing and types back in the
next, rather than 26 characters of ULID it will transpose. A name that matches nothing is retried
as a `KbSourceId`, so an id works too; a bare `<source>` is that source's own top level, the same
meaning an empty `rel_path` carries on the wire; and two sources sharing a name is answered as an
in-band error listing both candidates by id rather than by picking one. The parse and the lookup
are one function, `mcp::kb::resolve`, so the eight tools agree about what an address means by
construction.

Reads go through `files::listing` and `files::contents` — the same two functions `files::kb_answer`
uses for `KbTree` and `ReadKbFile`, against the same `Kb::base_path`, so the panel and the agent
cannot read a source differently; `list_kb_documents` honours the source's filter for the same
reason. Every mutation goes through `kb/ops.rs` and is refused there, never twice: a write to a
read-only source comes back as MCP's in-band `isError` carrying `ops`' own `FileError`. After one
succeeds, `Message::KbChanged` naming the parent directory goes to every window, exactly as the
task server posts its work-family messages, so an open panel re-lists without the coordinator being
asked a question. The listener holds the coordinator's own `Kb` through `mcp::KbReach` — an `Arc`,
and named for the reach because `KbAccess` already means a source's writability — since the
`Syncing`/`Failed` overrides live in that one object's memory and a second `Kb` would answer a
state the first has never heard of. The project and its folder come from `AgentFacts.project`: the
URL says which agent is calling, and that agent's project is the knowledge base it reaches. A git
source's URL and branch are answered as the facts they are; nothing that would authenticate to it
is in this family to answer with.

**The interface.** `crates/ubiq/src/state/kb.rs`'s `KbState` holds one `KbSourceView` per source —
the host's status plus a tree that is deliberately smaller than `ExplorerState`'s: no git marks, no
filter walk and no drag, just what is inside a folder, whether it is open and whether its listing is
in flight, plus the one thing a documents explorer cannot do without, a right-click menu. A selected
document is a `KbDocKey`, the pair of source id and path, held
beside its body so a re-selection of the same document redraws from what is already there without
asking again. `crates/ubiq/src/app/kb.rs` is the wire between a click and a message: opening a
folder sends `KbTree`, opening a document sends `ReadKbFile`, a press on a source that is not ready
sends `SyncKbSource`. The settings mutators — `confirm_kb_source`, `remove_kb_source`,
`set_kb_source_filter` — all read the configured list back out of `KbState`, edit it in memory and
funnel through the one sender to `SetKbSources`; the host's `KbSourcesListed` answer is what actually
moves `KbState`, on `KbState::accept`'s own merge, which keeps an already-open source's tree across a
rename and drops only what a filter edit invalidates.

**A source's folder is chosen on the host, through Ubiq's own picker.** `browse_kb_source_folder`
opens a `PickKind::Folders` file picker owned by `PickerOwner::KbFolder` over a `host_browse` session
begun against `Bus::host_of_project` — the same two steps `open_remote_project_picker` takes, with a
different owner on the end — so the path the form holds is one the *host* resolved. The session
starts at the project's own root, passed as `begin_host_browse`'s `start`, rather than the host's
default: a source is almost always found inside or beside the project it is being added to. The
platform dialog `cx.prompt_for_paths` is gone from this family; `G32` still names it for a project's
Add and Locate, which have not moved.

`crates/ubiq/src/ui/kb/mod.rs` draws two things: the left panel, a multi-source explorer that
borrows `ui::kit::files`'s row, twisty and kind-icon exactly as `ui/explorer.rs` does, with a
source's own row carrying its origin as a tooltip and a state word — `pending`, a syncing detail, or
`failed` with a retry control — that `Ready` never draws; and the centre, which hands markdown to
the one markdown renderer the window has, falls back to plain text for anything `ViewerKind::Editor`
claims, and says "opens in the IDE" for a diagram or an image, because reaching those from here
means wiring a web tenant to a document that is not an open file (`G11`).
`crates/ubiq/src/ui/sink/project.rs`'s `kb` function is the third surface: an inline section in the
project settings dialog, drawn only for a project with a live record — the sink's fixture page and
the create form have no project for a source to belong to — listing each source with its filter
field, a read-only/read-write marker and a remove control, under one **Add source** button, every
edit sent as it is made.

`crates/ubiq/src/ui/kb/source_form.rs` is the fourth: the modal that button raises, painted at the
window root over the settings page for the reason every other dialog is — a modal drawn inside the
page it overlays is one the page can clip. Its state is `state::kb::KbSourceForm` on
`WorkbenchState::kb_source`, and it is the New agent modal's shape throughout: a `kit::modal` with a
footer, a first full-width `Picker` and every row under it drawn inert rather than hidden. The rows
are the name (seeded from `KbOrigin::default_name` until the user types their own, which
`KbSourceForm::named` is the whole of), the kind, the folder field with its browse control for a
folder, the URL/Check/branch/store block for a repository, and access. Check sends
`ListRepoBranches { source: RepoSource::Url(..) }` and its answer lands in `app/clone.rs`'s
`receive_repo`, which offers `RepoBranches` and `RepoError` to the clone modal *and* to this form —
each takes it only if it named the query id it is still waiting on, so neither surface has to know
the other exists. Confirm appends the source and writes the whole list; Cancel discards, and both
take the browse session down with them.

**Three layers, and one gesture peels one of them.** The settings page, this modal over it and the
folder picker over that are all painted at the window root, so each is *outside* the bounds of the
one under it and `on_mouse_down_out` is capture-phase: without a guard, a click inside the modal
closes the page beneath it and a click inside the picker closes both. Each dismissal therefore
ignores the click while a layer it raised is up — `ui::sink::project::overlay` while
`kb_source` or `file_picker` is up, and the modal's own closure while its picker list or that
dialog is. `kit::overlay`'s module doc is where the rule is stated. The KB nav row follows the same
gate Tools and Remote do, in `nav` *and* in `AppState::set_sink_project_nav`: an edit dialog answers
to it, a create one does not.

**Every row of the explorer has a right-click menu**, and it is the project explorer's menu rather
than a second one: `state/kb.rs`'s `KbMenu` carries the epoch, the source, the path and the row kind
copied in as it opens, `kb_menu_entries` is the pure function that says what the row offers, and
`ui/kb/mod.rs` draws it through `kit::context_menu` with a pick that is an index into that list —
`ExplorerState`'s discipline throughout, including the separator occupying a slot of its own.

What a row offers depends on its kind and on `KbSource::is_writable`. A **source row** offers
open-in-Finder (`RevealKbPath`), copy-path (`AskKbPath`), *Get latest version* (`SyncKbSource`) only
for a `KbOrigin::Git` source, New file and New folder only where the source is writable, and *Rename
source* always — a source's name is Ubiq's own label for where the documents come from, so it is
renameable even where the material behind it is read-only, and it commits through the whole-list
`SetKbSources` write rather than `RenameKbEntry`. A **folder row** offers open-in-Finder and
copy-path, plus New file, New folder, Rename and Delete where the source is writable; a **file row**
the same two, plus Rename and Delete. Nothing is ever drawn greyed: the explorer greys Paste because
a user can *make* there be something to paste, and nothing on this menu has that shape — a folder
source has no latest version to fetch and no state a user could reach to give it one.

The three questions a menu pick raises are the project explorer's own modals, not new ones:
`FileDialog::KbNew`, `KbRename`, `KbRenameSource` and `KbRemove` are drawn by
`ui/file_dialog.rs`'s `kit::prompt_modal` and `kit::confirm_modal`, typed into the window's one
`file_name` field, and answered by `confirm_file_dialog` handing them to `app/kb.rs`'s
`confirm_kb_dialog`. They are the KB's own variants rather than `New`/`Rename`/`Remove` reused
because every path in this family is relative to a source, and a dialog that forgot which source it
was raised on would write into whichever one happened to be first. Delete asks first, and a folder's
confirmation says what is inside it goes too.

**Copy path is the one place the interface learns an absolute host path**, and it is asked for
rather than volunteered: no listing carries one, `AskKbPath` is sent on the pick, and `KbPath`'s
answer goes straight to the clipboard and nowhere else. Open-in-Finder is `RevealKbPath` for the same
reason — the file manager that opens is the *host's*, so a source on a drone shows where it actually
is instead of nowhere.

**`KbChanged` re-lists rather than guesses.** `receive_kb` marks the named directory unlisted, marks
it loading and sends `KbTree` for it, so the tree redraws from what the host says is there; a source's
own top level is handled as the directory it is, though it has no node of its own, and a `KbChanged`
for a folder the tree has never opened asks for nothing. **`KbFileError` always lands somewhere
visible**: on the document on screen when it names that document, and otherwise on the source's own
row, because a create, a rename or a delete that failed has no document to fail in and a gesture that
silently did nothing is the one failure mode the panel must not have.

**The centre has not caught up to the write half yet.** A document is drawn, never edited: there is
no save path through `WriteKbFile` from the KB panel, so the write half is reached from the explorer's
menu and from the `ubiq-kb` MCP server, and not from the document itself.

The wire is the Kb family in `crates/ubiq-proto/src/messages.rs`: UI → host is `KbSources`,
`SetKbSources`, `KbTree`, `ReadKbFile`, `SyncKbSource`, `WriteKbFile`, `CreateKbEntry`,
`RenameKbEntry`, `DeleteKbEntry`, `RevealKbPath`, `AskKbPath`; host → UI is `KbSourcesListed`,
`KbSourceChanged`, `KbTreeListing`, `KbFileContents`, `KbFileError`, `KbChanged`, `KbPath`. Every
variant carries `project_id`, and every one naming a path carries a `KbSourceId` beside it, because a
path in this family is relative to its source rather than to the project.

## The `KbStore::Project` tension with `D30`

`D30` states a rule with no exception today: nothing Ubiq writes lands inside a user's project
folder. A git source's clone already respects this — `Kb::base_path` puts it under
`<config root>/projects/<ulid>/kb/`, never inside the project — and that placement is why `D30`'s own
prose names the knowledge base as one of the things it covers.

`KbStore::Project` breaks that rule on purpose: a `.ubiq/kb/<source id>` checkout **inside** the
project, for a team that wants the checkout shared with everyone who has the project, or committed
alongside it, rather than living only in one person's config root. This is a real, user-chosen
exception rather than an oversight, and it needs its own decision row before it is built — what a
reasonable person could later reverse about writing into a project folder for the first time since
`D30`, and what it costs, most obviously an entry a project's own `.gitignore` may or may not carry
for a checkout Ubiq did not ask permission to commit. Nothing in the tree builds `KbStore::Project`
yet; the gap is `G269`.

## What is being built now

- **The centre's save path.** `WriteKbFile` is answered by the host and reached by the MCP server,
  and by nothing the user can press: a document opens read-only whatever `KbSource::is_writable`
  says. The explorer's write affordances that stood beside this in the list are built and are
  described under **What is built**, as are the settings modal and the `ubiq-kb` MCP server.

## Next steps

1. **Search scoped to the knowledge base.** The search panel searches a project's files only, and a
   KB source is not under the project — it needs a scope selector, and a search answer addressed by
   source rather than only by path, on the same reasoning the file family already carries a
   `KbSourceId` beside every path.
2. **Updating remotes.** A git source is refreshed only when a row or a settings control asks for
   it — there is no schedule, no "refresh everything" action, and no notice when a remote has moved
   on since the last fetch.
3. **Drag and drop of folders into the knowledge base**, for a source local to the machine the host
   runs on. The architecture rule that the UI never assumes the pseudo-terminal or the filesystem is
   local applies here exactly as it does to a pane: a path dropped onto the panel is only meaningful
   when the host answering it is the same machine the drop happened on, which a remote host is not.
4. **A document assistant** — an agent scoped to the knowledge base, reaching it through the
   `ubiq-kb` MCP server above rather than through the project's own file tools.
5. **Marking a document as context or instruction for that assistant** — a per-document flag the
   assistant reads before answering. It cannot live inside the document's own folder for a read-only
   source, since nothing may write there; where it lives instead — a sidecar under the source's own
   record, most likely — is undecided.

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the rule nothing Ubiq fetches writes inside
  a project folder, and the coordinator/UI split every KB message crosses
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the Kb message family in full
- [`../tech/decisions.md`](../tech/decisions.md) — `D30`, which `KbStore::Project` needs a row of its
  own against
- [`../features/workbench.md`](../features/workbench.md) — the rail mode the knowledge base panel
  and centre draw inside
- [`../backlog.md`](../backlog.md) — `G11` (the KB centre is read-only and markdown-only), `G32`
  (the platform folder dialog assumes a local host) and `G269` (`KbStore::Project` has no decision
  row)
