---
id: inbox-omni
title: Proposal — project search
kind: proposal
status: proposal
summary: Content search across a whole project — a shared query with its four options, a second host worker that walks the tree through the project's own ignore rules, a streamed and cancellable answer carrying the first search id on the wire, and a dock tab that draws the hits as places to go.
read_when: you are deciding how Ubiq searches the contents of a project, where that work runs, or how a long streamed answer is correlated and cancelled
updated: 2026-09-04
depends_on: [tech-architecture, tech-transport, feat-workbench, inbox-find, inbox-routing]
---

# Proposal — project search

Ubiq can filter the file tree by name and can search inside the one file that is open. It cannot
answer *where is this string in this project*, which is the question a user asks between those two
and the one an agent's output invites constantly — a symbol named in a pane, a path in a diff, an
error in a log. This proposes that search: one query, run by the host over the project's files,
streamed back as it is found, and drawn in the dock as a list of places to go.

It is the wider half of [`find-in-file-proposal.md`](./find-in-file-proposal.md), and the two are
written to share one thing — the query and its four options — and nothing else. That one is a buffer
already in the interface and crosses no bus; this one is a filesystem the interface may not touch.

The second mode the title implies — searching tasks, chats and the knowledge base beside the files —
is designed here and built later. §4 is why the shape is settled now even though only files answer
in v1: a source added afterwards must not change a message, a record or a row.

## 0. Where the build has got to — 2026-09-04

**Phases 1 to 3 have landed: a query typed into the panel runs over the project, streams back, and
its rows open files.** Everything below is the design; this is the gap between it and the tree, so a
later session does not re-derive it. §1 is the state before any of this landed, kept for its
reasoning rather than as a report.

In the tree: the contract (`crates/ubiq-proto/src/search.rs`, the six message variants); the worker
(`crates/ubiq-host/src/search/`, answered in `coordinator.rs`, with `tests/search.rs` covering §8's
ceilings, §5's interrupt and the filter refusals); the panel (`PanelKind::Search`, with a home
region, an availability rule and a saved key, drawn by `crates/ubiq/src/ui/search.rs`); and the
trigger — `AppState::run_project_search` mints the `SearchId`, cancels whatever it supersedes, and
sends `SearchProject`, fired by Enter on the query field. The three options are `toggle_pill`s that
apply to the next search rather than re-running the current one, a `SearchError` is drawn in the
status bar with the results it had left standing, both the group header and each hit row open the
file, and closing a project cancels its search. `crates/ubiq/tests/search.rs` drives the field's
Enter rather than the mutator, so the subscription is what is under test.

Where the tree is ahead of this document:

- **The titlebar's command field's Enter is now a second entry point into project search**,
  switching to the IDE, handing its text to the panel's query field, revealing it and running the
  search — see `crates/ubiq/src/app/git.rs`'s `submit_header_search`. §9 reserves that field for the
  `⌘K` navigator and says "`⌘K` is not an entry point" for this search; nothing currently binds
  `⌘K` on it, so in practice the field had no contract yet and this is what a direct product
  decision put there instead. Reconciling the two — a fast navigator whose Enter also happens to
  start a slow content search — is this document's to settle, not the tree's.
- **The walk takes a filter and an exclude list.** `Filter { patterns, subdir }` (§7) is honoured by
  `search::walk`, per-project and global excludes are merged in `coordinator.rs`, and the subdir
  resolves through `files::path` — the same boundary the file family uses. The settings dialog's
  Search section edits the two global lists; the panel still sends `Filter::default()`, so no
  search names a pattern or a subdirectory of its own.
- **A failed matcher can fall back to an external tool.** `search::fallback` runs a configured
  `grep` or `ag` when `RegexMatcherBuilder` refuses the pattern, with a watchdog kill; `find` and
  `fd` are refused by name. This is not designed anywhere below.

Where the tree is behind it:

- **Revealing is not the same as being drawn.** `PanelKind::Search::is_drawn` is
  `is_ide && has_project`, so revealing the panel from any other rail mode adds it to the dock and
  the next settling pass sets it invisible. Search is reachable only from IDE mode with a project.
- **A row opens the file, not the line.** §9's *go to this file at this line* needs the routing
  proposal's `Destination`; until then the row calls `select_file()`, which is what §9 says to do in
  the meantime.
- **The interface does not validate the query.** §6's added `regex` dependency was not taken: a bad
  pattern travels, the host answers `SearchError::BadQuery`, and the panel draws it. One place
  compiles the pattern instead of two agreeing.
- **Phases 4 and 5 are unbuilt** — no handoff with the find bar, no `Batch::Tasks`.

## 1. Where it stands

**The titlebar has two search affordances and neither does anything.** `command_field` in
`crates/ubiq/src/ui/titlebar.rs:94-120` draws an `Input` over `AppState.command_input`
(`crates/ubiq/src/app/mod.rs`) with the placeholder *Search files, or run a
command…* and a `⌘K` hint; nothing subscribes to it — the field is declared, constructed, assigned
and never read again. Beside it, `titlebar.rs:73` draws a search icon whose handler is `|_, _, _| {}`.
That is `G16`.

**The one filter that works matches names, not contents.** `ExplorerState::rows(&filter)`
(`crates/ubiq/src/state/explorer/filter.rs`) does a case-insensitive substring test against a row's
whole path (`:353`), fed from `workbench.file_filter`. It is a *go to file* field, and it is the
thing users mistake for search.

**The host reads one file at a time, and never looks inside more than one.** The file family is
`ProjectTree`, `ReadProjectFile` and `WriteProjectFile` (`crates/ubiq-proto/src/messages.rs:159-215`);
`crates/ubiq-host/src/files/mod.rs` answers all three on one worker thread. Nothing walks a project
for content, and nothing in the workspace could: no `grep-*` crate is in `Cargo.lock` at all, and
`ignore` and `regex` are present only transitively — `regex 1.12.4` and `aho-corasick 1.1.4` through
`gpui`, `ignore 0.4.32` through `rust-i18n`'s `globwalk`. No Ubiq crate declares any of them.

**The walk that exists cannot be pointed at a whole project.** `files::listing()` is breadth-first
and clamped — `MAX_DEPTH = 3`, `MAX_ENTRIES = 2_000` per directory, `MAX_REPLY_ENTRIES = 10_000` per
reply (`files/mod.rs:50-60`) — and it skips a fixed `NOT_DESCENDED` list (`files/mod.rs:34-47`)
rather than reading the project's ignore rules. That is `G33`, and its row already names the
conclusion this proposal reaches: reading `.gitignore` "is the one thing that would justify the
`ignore` crate."

## 2. Two searches, one word

Three things in this interface are called search and only one of them is this. Saying which is which
once is cheaper than three documents each assuming its own.

| | Matches | Scope | Runs in | Owned by |
|---|---|---|---|---|
| `⌘K` navigator | Names and paths | Everything addressable | The interface | [`ui-routing-proposal.md`](./completed/ui-routing-proposal.md) |
| Find in file | Contents | One open buffer | The interface | [`find-in-file-proposal.md`](./find-in-file-proposal.md) |
| Project search | Contents | Every file in the project | The host | This document |

**`⌘K` stays a navigator and never grows content matching.** It answers on the first keystroke from
things already in memory — recents, bookmarks, the explorer's tree, task titles — and a content
search cannot: it is a walk of the disk, it takes long enough to need a progress state, and its
result is a list worth keeping on screen rather than a menu that closes on a click. Mixing them
would make the fast thing slow and give the slow thing nowhere to live.

**Project search is the same question as find-in-file, asked of the disk instead of a buffer.** That
is the whole of what the two share, and §3 is that sharing made concrete.

## 3. What a query is

**One record, in a `search` module beside the others in `crates/ubiq-proto/src/`, used by both
searches.**

```rust
pub struct Query {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}
```

Four options, and they compose: `regex` off means the text is a literal, `whole_word` puts a word
boundary on either side of whatever the other two produced, `case_sensitive` off folds case. There is
no fifth, and in particular no *search in selection* and no *preserve case on replace* — both are
find-in-file's business if they ever exist, and neither belongs on the wire.

**The record lives in `ubiq-proto` even though find-in-file never sends it.** Not because the bus
needs it there, but because the four options must mean the same thing in both places: a user who
ticks `Aa` and `.*` in the editor's find bar and then presses *Search in project* must get the same
matches from the host as they were getting from the buffer. One record, one meaning, and the
interface hands the same value to two different engines.

**A query that cannot compile is not a request.** `regex` on with an unclosed group is caught in the
interface before anything is sent — the field marks itself and the search does not start. This is
safe to do in the interface *only* because both engines are the same engine: the host matches through
`grep-regex`, which is built on the `regex` crate the interface validates with, so the two agree on
what parses by construction rather than by care. The host still refuses a bad pattern with
`SearchError::BadQuery`, because a contract that trusts its caller is not a contract.

**An empty query is not a search.** No request, no results, no error — the same as the explorer's
filter, where empty means *no filter* rather than *match nothing*.

## 4. The two modes, and the source that answers

The request names a scope, and every batch of results names the source it came from.

```rust
pub enum Scope { Files, Project }

pub enum Source { File, Task, Chat, Kb }
```

**`Scope::Files` searches the project's files. `Scope::Project` searches everything the host can
search, which in v1 is the files.** That is the second mode, shipped honest: the interface asks for
everything and is told what was actually looked at, in `SearchFinished { searched: Vec<Source>, .. }`.
A source added later is a new arm, a new batch variant and a new group header — no message changes,
and no window has to be taught what it already draws.

**Each source's hits have their own shape, and nothing generalises them.** A file hit is a path, a
line number, the line's text and the ranges inside it. A task hit is a `TaskId`, which field matched
— title, description, a step — and the matched text. They are not the same row and pretending they
are would put a line number on a task. So the batch is an enum:

```rust
pub enum Batch {
    Files(Vec<FileHit>),
    Tasks(Vec<TaskHit>),   // later
}
```

This is the navigation proposal's locus problem in miniature, and it
takes the same answer: a shared envelope, a private shape, and no reader that matches on a kind it
does not own.

**The later sources are cheap for a reason worth stating.** A task search is a scan of
`Work`'s in-memory records, which the host already holds and which are small; the knowledge base is
`RailMode::Kb`, which draws an empty page today (`G11`), and it will be Ubiq's own store rather than
the user's folder. Neither is a walk of a disk, so neither needs the worker in §5 — they answer on
the coordinator's thread the way the work family already does. **Only the files are expensive**, and
that is why the files are what the design is shaped around.

## 5. Where the work runs

**A second worker thread, not the file worker's.** `D36` put the file family on one thread with one
FIFO queue, and `G36` is the cost: a request behind a slow one waits. A search is seconds where a
listing is milliseconds, so putting a search in that queue would stall every folder expand and every
file open behind it — a user who searches while a file loads would see the file arrive when the
search finished. So `crates/ubiq-host/src/search/`, one thread, and the shape `files::Files` proved
copied rather than reinvented: a `Job { project_id, search_id, root, query, scope, reply_to: Mailbox }`,
an unbounded `flume` queue, a coordinator that looks the project's root up in memory and submits.

Three things differ from the file worker, and each is forced by the work being long.

**The parallelism is inside the worker, not in the queue.** The walk is `ignore`'s
`build_parallel` over a bounded number of threads, and the worker thread is what owns it. From the
coordinator's side there is still one submitter, one queue and one mailbox — the host's structure is
unchanged — and the concurrency lands where it pays instead of in a pool of workers that would
reorder unrelated replies and reopen `G36`'s sequence-number problem.

**A search is interruptible, mid-file.** Each job carries an `Arc<AtomicBool>`; the walker checks it
between files and the sink checks it between matched lines — `grep-searcher` stops the moment a sink
answers `false`. Cancellation is therefore prompt rather than eventual, which matters because the
common case is not the user pressing a button: it is the user typing another character.

**One live search per project, and a new one supersedes the old.** The coordinator keeps the flag
against the `SearchId`, sets it when a second search arrives for the same project, and forgets it on
finish. Nobody reads two searches at once, and a search-as-you-type that queued would spend its life
answering prefixes.

## 6. The dependency

**The ripgrep libraries, in `ubiq-host`, and nothing hand-rolled.** Three crates, each doing the part
that is genuinely hard:

`ignore 0.4` walks the tree and applies the project's own rules — `.gitignore`, `.ignore`, nested
ignore files, the global one — which is what makes a search of a Rust project return source files
instead of forty thousand paths under `target/`. Hidden files are skipped, and the ignore rules are
the reason this crate is worth its size: reimplementing gitignore precedence correctly is a project,
not a function.

`grep-regex 0.1` builds the matcher, and its builder is the option matrix in §3 exactly:
`case_insensitive`, `fixed_strings` and `word` are three flags on `RegexMatcherBuilder`, so all four
of Ubiq's options are one construction with no special cases and no pattern rewriting of Ubiq's own.

`grep-searcher 0.1` reads the files — line numbers, memory maps for large ones, a heap limit, and
`BinaryDetection::quit` so a search does not print an object file's bytes into the interface.

**Ubiq gains `regex` as a direct dependency of the interface too**, for the validation in §3. It is
already in the build graph through `gpui`, so this costs a line in a manifest and no compile time.

**This does not change the file tree's walk.** `NOT_DESCENDED` stays where it is, and `G33` is
retired only for search. Teaching the explorer's listing the same ignore rules is the obvious next
step and a separate change, because a tree that hides rows and a search that skips files are
different promises to the user — the tree's own header already says a tree with rows missing is a
tree that lies.

## 7. The message family

A fifth family, alongside pane, session, project, file and work. Every variant names a project,
because a search is a question about one.

| Message | Direction | Payload |
|---|---|---|
| `SearchProject` | UI → host | `project_id`, `search_id`, `query`, `scope` |
| `CancelSearch` | UI → host | `project_id`, `search_id` |
| `SearchMatches` | host → UI | `project_id`, `search_id`, `batch` |
| `SearchProgress` | host → UI | `project_id`, `search_id`, `files_seen` |
| `SearchFinished` | host → UI | `project_id`, `search_id`, `searched`, `counts`, `truncated` |
| `SearchError` | host → UI | `project_id`, `search_id`, `error: SearchError` |

```rust
pub struct FileHit { pub rel_path: String, pub lines: Vec<LineHit>, pub truncated: bool }
pub struct LineHit { pub line: u32, pub text: String, pub ranges: Vec<(u32, u32)> }
```

**This is the first family to carry a request id, and the reason is worth stating** because the
contract's other four deliberately do not. The file family correlates by `project_id` and `rel_path`
and needs nothing more: one request, one answer, one FIFO queue. A search breaks all three
assumptions — one request produces many answers, an answer can arrive after the user has moved on,
and `(project, query)` is not an identity because the same query re-run over a changed tree is a
different search whose old batches must not be drawn into the new list. So the interface mints a
`SearchId` with `ulid_id!` (`crates/ubiq-proto/src/ids.rs:43-92`) and **the interface discards every
message naming a search it is not holding.** That is the same discipline the generation counter buys
in [`../tech/version-control.md`](../tech/version-control.md), reached from the other direction.

**Batches are flushed on whichever comes first: 64 files, 512 hits, or 100ms.** Not one message per
match, which would be twenty thousand messages for a common word, and not one message at the end,
which would make the interface look broken for four seconds. The precedent is `TerminalOutput`,
chunked as read; the difference is that a batch is a whole unit of meaning and never splits a line.

**`SearchProgress` exists so an empty result can be trusted.** A search that finds nothing and a
search that has not started look identical, and the fix is not a spinner — it is a count of files
seen, which also tells the user their ignore rules are working.

**Every reply is addressed to the asker, not broadcast.** A project is open in one window, so the
rule the file family already follows applies unchanged (`Reply::Asker`,
`crates/ubiq-host/src/reply.rs:9-13`).

**No absolute path crosses.** Every hit is a project-relative path, which is architecture rule 2
applied to search: nothing in a result says which machine answered.

## 8. Ceilings, and refusal

A search is the one operation whose result size the user does not control, so the ceilings are part
of the contract rather than a defensive detail. Proposed for v1: **100 hits per file**, **1,000 files
with hits**, **10,000 hits total** — the last matching `MAX_REPLY_ENTRIES`, because the same
reasoning bounds both — and files above the file family's own read ceiling are skipped rather than
truncated.

**Every ceiling that bites is drawn.** `FileHit::truncated` and `SearchFinished::truncated` are on
the wire so the interface can say *first 100 of many in this file* and *stopped at 10,000*. `G35`
is the lesson: `Row::truncated` has been on the tree's rows all along and is drawn nowhere, so a
listing cut short looks complete. A search that silently stops is worse, because the user concludes
the string is not there.

**A file that cannot be read is counted, not reported.** A permission refusal on one file in a
node_modules the ignore rules missed is not an error the user should dismiss; `SearchFinished` carries
the count of files skipped, and the log gets the detail. `SearchError` is for the search failing —
the root gone, the query bad, the walk refused at the top.

## 9. Where it appears

**A third dock tab.** `DockTab` is `Pane | Logs` (`crates/ubiq/src/app/mod.rs`), and search is the
third: a query bar with the four option toggles, then results grouped by file, each group collapsible,
each line one row. The dock is right because a result list is a place the user returns to while
working through it — a modal closes on the first click, and the explorer panel is too narrow for a
line of code. `ui/logs.rs` is the precedent for a dock tab that is not a pane, and its filter row is
the shape the query bar takes.

**A row is a destination.** Opening a hit is *go to this file at this line*, which is exactly what
[`ui-routing-proposal.md`](./completed/ui-routing-proposal.md)'s `Destination` with an `L`-locus already names.
Until that lands the row calls `select_file()` and the caret follows on the next frame, and when it
lands this becomes a one-line change and the results list also becomes linkable.

**Rows append in arrival order and never re-sort.** The walk is parallel, so files arrive in no
particular order; sorting a list that is still growing moves rows under the pointer and loses the
click. Groups are ordered by when their first hit arrived, and that is stable for the rest of the
search.

**Two entry points, and the third stays closed.** `⌘⇧F` from anywhere opens the tab and focuses the
query bar; the titlebar's search icon has the same job, which is the one thing it can mean. Both
call `reveal_search`, and focusing the field is part of it — a panel that opens with the caret left
where it was reads as nothing having happened. `⌘K` is not an entry point — §2. The binding is registered with `cx.on_action` in
a key context an element actually declares, which `G51` records as the mistake `⌘S` already made.

**`⌘⇧F` was taken, and it has been taken back.** The component library binds it to *replace in this
file* at the `Input` context, which is deeper in the tree than `Workbench` and so won every tie the
moment any field held focus — the shortcut meant two things depending on where the caret was, which
is the worst of the three available outcomes. Project search gets it, because that is what it means
in every editor a user arrives from, and replace-in-file moves to `⌘⌥F`. Both are bound again at the
field's own depth in `install_key_bindings`, which runs after `gpui_component::init`: same
predicate, registered later, wins. It is the device `ui::file_picker::key_bindings` documents.

**The two searches hand each other their query.** *Search in project* on the editor's find bar opens
the tab with the query and its four options carried over; *find in this file* on a result group does
the reverse. The shared `Query` in §3 is what makes both one line.

## 10. Failure

| When | What happens |
|---|---|
| The query is an invalid regex | Nothing is sent; the field marks itself and says why |
| The query is empty | No search, no result list, no error |
| The project's folder has gone | `SearchError::Root`; the tab says so and the results it had stay on screen |
| A search is superseded | The old flag is set, its remaining batches are discarded on arrival by id |
| The window closes mid-search | The mailbox has gone and the worker's send is a no-op, as the file worker's already is |
| A ceiling is hit | The results stand and are marked truncated, per file and overall |
| A file cannot be read | Counted in `SearchFinished`, logged, not raised |
| A file is binary | Skipped at the first NUL, counted with the skips |
| The project is closed in this window | The search is cancelled with the project's other state |

## 11. Rules this adds

**Content search runs in the host, always.** The interface never walks a directory and never reads a
file to match it. The one search that runs in the interface is over a buffer the interface already
holds, and that is find-in-file's rule, not an exception to this one.

**A search has an identity and the interface holds exactly one per project.** Any message naming a
search the interface is not holding is discarded without drawing.

**A result is a project-relative path, a line and ranges within it.** No absolute path, no file
handle, no byte offset into a file the interface cannot open itself.

**Search obeys the project's ignore rules, and the file tree does not — yet.** The two walks answer
different promises, and neither may be changed to match the other by accident.

## 12. Phases

1. **The query and the contract** — *done.* The `search` module in `crates/ubiq-proto/src/` —
   `Query`, `Scope`, `Source`, `Batch`, `FileHit`, `LineHit`, `SearchError` — the six message
   variants, the `SearchId`, and the
   contract document's fifth family. Nothing runs; find-in-file can already use `Query`.
2. **The worker** — *done.* `crates/ubiq-host/src/search/`, the three dependencies, the
   parallel walk, the matcher, the batching, the ceilings, the interrupt flag and supersession,
   tested against a fixture tree in `crates/ubiq-host/tests/search.rs`. The filter and the external
   fallback landed with it and are §0's first list.
3. **The panel** — *done, bar the one thing §0 names.* The query bar and its option toggles, the
   grouped results, the progress and truncation readouts, the trigger, `⌘⇧F` and the titlebar icon,
   the row as a destination and cancel. This is the phase that retires half of `G16`. A row opens
   the file rather than the line.
4. **The two-way handoff.** *Search in project* on the find bar and *find in this file* on a group.
   Waits on the find bar's own phase 1.
5. **The other sources.** `Batch::Tasks` over the work records, on the coordinator's thread, and the
   group header for it. `Scope::Project` starts answering with two sources instead of one, and the
   interface changes by one arm.

Phases 1–3 are the feature, and they are in the tree. Phase 5 is why the shape in §4 exists, and it
can wait indefinitely without leaving anything half-built. §0 says where the tree and this document
disagree in either direction.

## 13. What this asks to be decided

Seven decision rows:

- A query is one record with four options — literal or regex, case, whole word — shared by both
  searches so that the same ticks mean the same matches in a buffer and on disk.
- Content search runs in the host on a worker of its own, never on the file worker's queue and never
  in the interface.
- The walk obeys the project's ignore rules through the `ignore` crate, and matching goes through
  the ripgrep libraries rather than anything hand-rolled.
- A search carries an id minted by the interface — the first request id on the wire — and the
  interface discards any message naming a search it is not holding.
- One live search per project; a new search supersedes the old and is interrupted mid-file rather
  than queued.
- Results stream in bounded batches, are drawn in arrival order and never re-sorted, and every
  ceiling that bites is visible in the interface.
- The scope names what to search and the answer names what was searched, so a second source is an
  arm rather than a new message.

Backlog rows this leaves open: replace across a project, which needs a write per file with the
version the read came with, a preview, and an answer for the file an agent changed underneath it —
deliberately out of this proposal, which is shaped to accept it; a search-as-you-type debounce, and
whether it is worth it once supersession is prompt; teaching the explorer's listing the same ignore
rules, which is the rest of `G33`; searching only a subtree, only certain globs, or only the open
files, all of which are one field on the request and no new mechanism; whether a result list should
survive a project switch; and the knowledge base, which cannot be searched until it exists.

## Related docs

- [`find-in-file-proposal.md`](./find-in-file-proposal.md) — the other half, and where the `Query` is second used
- [`ui-routing-proposal.md`](./completed/ui-routing-proposal.md) — the `⌘K` navigator this stays out of, and the destination a result row becomes
- [`../tech/architecture.md`](../tech/architecture.md) — the rules §5 and §7 obey
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the four families this is a fifth beside
- [`../features/workbench.md`](../features/workbench.md) — the dock the results tab joins
