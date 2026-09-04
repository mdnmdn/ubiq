---
id: inbox-indexing
title: Proposal — indexing and filesystem watch
kind: proposal
status: proposal
summary: A host-owned watcher and three per-project indexes — filename, full text, symbol — so the knowledge base and the local web export answer from a store instead of a fresh walk, and the tree, the git state, and open diffs stop going stale between asks.
read_when: you are deciding how Ubiq watches a project's folder for change, or how it indexes a project's files for something other than a live content search
updated: 2026-09-04
depends_on: [tech-architecture, tech-structure, tech-transport, inbox-omni]
---

# Proposal — indexing and filesystem watch

Four gaps already say the same thing from four directions: `G34` — nothing watches a project's
folder, so a file created, deleted or renamed outside Ubiq is invisible until a folder is collapsed
and expanded again; `G71` — nothing watches the git directory, so `HEAD`, `MERGE_HEAD` and the index
go stale until a save or a project open asks again; `G30` — health is only probed at load, on open
and on request; `G77` — autorefresh of git and the tree on filesystem events is wanted outright. None
of the four needs an index. All four need a watcher, which is why this proposal builds the watcher
first and treats indexing as what rides on top of it once something asks.

The indexing half exists because [`inbox-omni`](./omni-search-proposal.md) already named the shape of
the gap it leaves: content search across a project's files stays a live walk, decided and shipped in
that proposal's design, and is explicitly not reopened here. What that proposal could not answer is
the knowledge base's own search, because the knowledge base "will be Ubiq's own store rather than the
user's folder" — its words — and a store is the one thing a live walk of the user's folder can never
be. This proposes that store, and the watcher that keeps it honest.

## 0. Where the build has got to — 2026-09-04

**Phase 1 has landed: the watcher and its unsolicited push, with no index of any kind built.** No
filename index, no symbol index, no `tantivy`, no host-owned cache directory — phases 2 to 4 are
untouched, and §1 below is the state before the watcher landed, kept for its reasoning rather than
as a report.

In the tree: `crates/ubiq-host/src/watch/mod.rs` (one recursive `notify` watch and one debounce
thread per open project, `QUIET` 150ms, `BOUND` 64), keyed per window and per project in
`coordinator.rs` and started from `Message::OpenedProject`, with `crates/ubiq-host/tests/watch.rs`
over the classification and the batching. `notify` is a direct dependency of `ubiq-host` and
resolves to the version already in the lock file, so no package entered the graph.
`Message::ProjectFilesChanged` — which existed on the wire and had no producer and no consumer — now
has both; the contract for it is
[`../tech/transport-contract.md`](../tech/transport-contract.md)'s file family, and what the window
does with it is [`../features/workbench.md`](../features/workbench.md)'s.

Where the tree differs from this document:

- **The two scopes are one recursive watch, filtered per event**, rather than a content scope plus a
  fixed `.git` scope: a selective watch would have to be re-registered every time a directory
  appears. `.git/HEAD`, `.git/MERGE_HEAD`, `.git/index` and `.git/refs/**` set `repository` on the
  next flush and everything else under `.git/` is dropped, which is the behaviour §3 asks for.
- **The watcher's ignore reading is its own, not the walk's.** §3 has both halves agreeing on one
  reading of a project's `.gitignore`; the watcher builds a single `Gitignore` from the root's
  `.gitignore` plus the merged excludes, because `ignore::WalkBuilder` cannot answer a question
  about one path. `G110` is that gap, and it over-reports rather than under-reporting.
- **The push is `ProjectFilesChanged`, not a `ProjectChanged` record**, and it carries a
  `repository` flag beside `changed` and `truncated`. It rides no new primitive, so `G47` is
  untouched: this is one unsolicited variant in the file family and nothing generalised.
- **`G30` is not closed.** The watcher reports what changed inside a project's folder and never
  re-probes the record's health, so a folder that goes away is still noticed only when somebody
  asks.
- **There is no `CloseProject`,** which §3's "stops when it closes" assumes. A watch is stopped by
  the same window opening another project or by the window leaving — `G113`.

## 1. Where it stands

**No index of any kind exists.** `⌘K`'s navigator matches names and paths from what is already in
memory — the explorer's tree, recents, bookmarks — and is owned by
[`ui-routing-proposal.md`](./ui-routing-proposal.md); it is not a persisted index and this proposal
does not change it. `RailMode::Kb` draws an empty page today (`G11`). `tantivy` is in no `Cargo.lock`
anywhere in the workspace. `tree-sitter` is real and vendored — `crates/ubiq/Cargo.toml` depends on
grammars for markdown, rust, python, javascript, java, toml, tsx, typescript and yaml — but only for
the editor's syntax highlighting; nothing walks a parse tree for structure.

**Nothing watches a project's folder.** `notify` is already in the dependency graph, pulled in
transitively through `gpui-component`, and nothing in Ubiq calls it. `ExplorerState::merge` in
`crates/ubiq/src/state/explorer/tree.rs` was written with this in mind — its own doc comment says a
re-listing folds into what is already known "rather than destructive," which is "what makes a
re-listing — a restore, or one day a filesystem watch — idempotent." The watcher this proposes is
that one day, and it can reuse the merge it already describes.

## 2. What this proposes, in one line

A single host-owned watch-and-index worker per open project: a `notify` watcher feeds a debounced
change queue, the queue drives three independent indexes — filename, full text, symbol — and a coarse
"something changed here" message reaches the interface without being asked. The watcher runs for
every open project and is cheap; the indexes are built lazily, the first time something needs one, and
kept warm afterward by the same feed.

## 3. The watcher

**Two scopes, one `notify` instance per project.** The content scope walks and watches everything the
project's own ignore rules allow — reusing the `ignore`-crate-based rule reading `inbox-omni` §6
already decided for search, so a project's `.gitignore` is read once and agreed on by both — minus
`.git` itself. A second, fixed scope watches `.git/HEAD`, `.git/MERGE_HEAD`, `.git/index` and
`.git/refs/**` unconditionally, because those are exactly the paths an ignore-aware walk would skip
and exactly the ones `G71` needs.

**Events are debounced and coalesced by path**, the same 150ms window `refs/markdown-web`'s watcher
already proved sufficient for — a directory saved by an editor as a burst of creates and renames
collapses to one change per path rather than one event per syscall.

**The watcher starts when a project opens and stops when it closes.** It answers `G34`, `G71`, `G30`
and `G77` on its own, before a single index exists — a project nobody has ever marked for the
knowledge base still gets a live tree and live git state.

## 4. The indexes

Three independent indexes share the watcher's feed and serve different readers. None blocks another,
and none is a source of truth — each is rebuilt from the files on disk, and none is asked to survive
being wrong.

### 4.1 Filename index

An in-memory tree of paths and titles, refreshed incrementally from the watcher's changes rather than
rescanned whole — the same idea as `refs/markdown-web`'s `docstore`, made incremental instead of
rebuilt. It backs the knowledge-base browser's tree and the web export's directory listings
([`inbox-kb-web`](./kb-web-export-proposal.md)), and it is available later as a faster backing store
for `⌘K`'s in-memory scan, without that navigator changing what it means to its user.

### 4.2 Full-text index

One `tantivy` index per project, built lazily and scoped to whatever asked for it — the
knowledge-base's marked files by default, the whole project if a full export ever needs project-wide
search. It persists under a new, host-owned cache directory beside the project's existing files — a
sibling to the interface's own `ui/` workarea under `projects/<ulid>/`, but the other way round: this
one belongs to the host, the interface never reads it directly, and it is exactly as disposable as
`ui/` is — safe to delete, rebuilt from the project's files, never consulted to decide what those files
are. A changed file's document is deleted and re-added inside one commit per debounce window, not one
commit per file; `tantivy` commits are not free.

This is the store `inbox-omni`'s `Source::Kb` was written to expect and cannot yet query — the phase
that lands this index is the phase that finally answers it (§9).

### 4.3 Symbol index

Per-file definitions — functions, types, methods, and headings treated as symbols for markdown —
extracted with the grammars already vendored for syntax highlighting, so v1's language coverage is
whatever that list already is, at no new grammar cost. Two readers: the knowledge-base's per-document
outline and its auto-generated titles (`refs/markdown-web`'s heading-walk TOC, done here over a
`tree-sitter-markdown` tree instead of a goldmark AST), and the web export's "code exploration" mode,
where a definition becomes a link target.

**Stated ceiling, not a silent gap:** v1 extracts definitions only. No cross-file reference
resolution, no call graph, no *find all references* — a definition site is a link, a use site is not,
yet. It is in-memory and rebuilt like the filename index; a symbol table is cheap enough that
persisting it is not worth the complexity.

## 5. The push

A coarse invalidation, host to interface, sent without being asked — the same class of thing `G47`
already wants for the work family ("the host answers work messages and never pushes one... a live
agent needs an unsolicited variant the work family does not have"). This proposal does not invent a
second unsolicited-push mechanism; whatever plumbing resolves `G47` is what this reuses.

```rust
pub struct ProjectChanged { pub project_id: ProjectId, pub changed: Vec<RelPath>, pub truncated: bool }
```

Batched on the same debounce window as the watcher itself, bounded the way search's own batches are
(64 files / 512 hits / 100ms, whichever first) rather than one message per file event. No content
crosses on this message, only relative paths — architecture rule 2 applied here the same way it is
applied to a search hit. A reader that wants the new content asks for it the normal way: a fresh
`ProjectTree` listing, a knowledge-base query, a diff.

Four readers of one message: the explorer tree, folded in through `ExplorerState::merge` exactly as
its own comment anticipates; git state, refreshed on any change inside the fixed `.git` scope; the
knowledge-base browser panel; and the web export's live-reload endpoint
(`inbox-kb-web` §5).

## 6. Where it runs, and what it costs

**Owns a process-local resource on every count — belongs in `crates/ubiq-host/`.** `tech-structure`'s
own placement rule is exact: "Does it own a process, a file descriptor or a path on disk? →
`crates/ubiq-host/`." A `notify` handle, a `tantivy` index directory and a parsed syntax tree are all
three.

**One worker thread per project**, its own queue — the shape `crates/ubiq-host/src/search/` already
proved for exactly this reason: putting indexing work on the file family's single thread would stall
every folder listing behind it, the same cost `G36` already names.

**One real new dependency.** `notify` is already in the build graph transitively through
`gpui-component`; a direct edge from `ubiq-host` is a manifest line, not a new crate to vet. The
tree-sitter grammars are already direct dependencies of the interface crate; a direct edge from
`ubiq-host` costs the same, a line and nothing else. `tantivy` is the one dependency this proposal
actually adds — no existing edge, transitive or otherwise, reaches it today.

## 7. Failure

| When | What happens |
|---|---|
| The watcher can't watch a subtree (permission denied) | Logged; that subtree gets no live updates, nothing else is affected |
| A `tantivy` commit fails, or the index directory is corrupt | Dropped and rebuilt from the project's files — the cache is disposable by design |
| A file fails to parse (mid-edit syntax error) | Its symbols stay stale until it parses again; no other file is affected |
| A burst overwhelms coalescing (a checkout touching thousands of paths) | Treated as a full rescan, the same fallback `refs/markdown-web`'s watcher effectively takes by calling `store.Rescan()` on any change |
| A project closes while indexing is in flight | The worker's queue is dropped with the project, the same as the file and search workers already do |

## 8. Rules this adds

- Indexing is always host-side, always lazy — built the first time something asks for it, kept warm
  afterward by the watcher's feed.
- The watcher and the indexes are separate: the watcher runs for every open project and fixes
  staleness on its own; an index is opt-in on top of it.
- An index is a disposable cache, never a source of truth — safe to delete, always rebuildable from
  the project's own files.
- Content search over an arbitrary project stays the live walk `inbox-omni` already decided. These
  indexes only ever answer for the knowledge base and the web export.

## 9. Phases

1. **The watcher alone.** `notify`, the two scopes, the debounce, `ProjectChanged`, wired to
   `ExplorerState::merge` and to git-state refresh. Closes `G34`, `G71`, `G30` and `G77` with no index
   built yet.
2. **The filename index.** Feeds the knowledge-base browser and the web export's directory listings.
3. **The symbol index.** Per-file outlines, document titles, code-exploration link targets.
4. **The full-text index.** Knowledge-base and web-export search — the phase that finally gives
   `inbox-omni`'s `Source::Kb` a store to answer from.

Phase 1 stands alone and is worth building even if nothing after it ever lands.

## 10. What this asks to be decided

- One `notify` watcher per open project, split into an ignore-aware content scope and a fixed
  git-plumbing scope, debounced 150ms and coalesced by path.
- The watcher runs for every open project regardless of indexing; indexing is lazy and separate.
- Three independent indexes — filename, full text, symbol — none a source of truth, none blocking
  another, the full-text one alone persisted, and only to a host-owned disposable cache directory.
- The invalidation push is coarse (paths, not content) and unsolicited, riding whatever primitive
  resolves `G47` rather than a second one.
- This does not touch how `inbox-omni`'s `Scope::Files` search runs — that stays a live walk.

Backlog rows this leaves open: whether the full-text index ever covers a whole project rather than
only the knowledge base, and at what project size that stops paying for itself; cross-file reference
resolution for the symbol index; whether a missing or corrupt `tantivy` index rebuilds eagerly on host
start or waits for the first query that needs it; index warm-up cost on a large project's first
activation.

## Related docs

- [`omni-search-proposal.md`](./omni-search-proposal.md) — the live-walk search this does not
  reopen, and `Source::Kb`, which phase 4 here answers
- [`kb-web-export-proposal.md`](./kb-web-export-proposal.md) — the feature this indexing serves
- [`../tech/architecture.md`](../tech/architecture.md) — the placement rule and the no-absolute-path
  rule this follows
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the message-family shape this
  imitates
- [`../tech/project-structure.md`](../tech/project-structure.md) — the workarea this new cache
  directory sits beside, on the other side of the boundary
