---
id: wip-indexing
title: Indexing a project
kind: wip
status: current
summary: What Ubiq keeps about a project so a search need not re-read it — a per-project level defaulting from an application setting, and a full-text index that selects candidate files for the existing content search rather than answering it. The full-text half is built; the symbol half the `full` level names is not, which is the gap this document exists to record.
read_when: you are changing what Ubiq indexes, how a content search is answered, or what the indexing level means
updated: 2026-09-06
verified: 2026-09-06
code_anchors: [crates/ubiq-host/src/index/mod.rs, crates/ubiq-host/src/index/text.rs, crates/ubiq-host/src/index/ceiling.rs, crates/ubiq-host/src/search/worker.rs, crates/ubiq-host/src/search/hits.rs, crates/ubiq-host/src/watch/mod.rs, crates/ubiq-host/src/coordinator.rs, crates/ubiq-proto/src/projects.rs, crates/ubiq-proto/src/settings.rs, crates/ubiq/src/ui/settings.rs, crates/ubiq/src/ui/sink/project.rs]
depends_on: [tech-architecture, tech-transport, tech-structure, feat-workbench]
---

# Indexing a project

This replaces the proposal that stood in `inbox/`. That document argued for one index — a table of
definitions extracted with each grammar's `tags.scm` — and against a full-text index. **Both halves
of that argument were reversed**: the full-text index is built and the symbol table is not.

## What is built, and what is not

**Built:** the level, and the full-text index behind `light`. A content search on an indexed
project reads only the files that could match, refreshed as the watcher reports changes.

**Not built:** the symbol table. `full` is selectable and behaves exactly like `light` — it keeps
no symbols, because nothing extracts any. There is no `ProjectSymbols` message, no tree-sitter
dependency in `crates/ubiq-host/`, no outline fed from a project-wide table. **A user who picks
"Full text + symbols" today gets full text and no symbols, and nothing says so** (`G156`).

The buffer's own outline is a separate thing and does exist: `crates/ubiq/src/ui/outline.rs` parses
the file in front of the user with tree-sitter in the interface crate, needs no host, no index and
no project, and works at every level including `none`.

## The level

Indexing is a per-project choice, defaulting from an application-wide setting. The levels are
**cumulative**, not alternative — `full` is `light` plus symbols:

| Level | Content search | Symbols |
|---|---|---|
| `none` | the live walk, as before any index existed | — |
| `light` (the default) | the full-text index | — |
| `full` | the full-text index | *not built* |

Tantivy carries the content half at both `light` and `full`, for every kind of file — including
the ones a grammar exists for. A symbol table answers questions about *names* and never about
content, so it can never stand in for the other half. That is why the levels are cumulative and
why the full-text index is not optional above `none` (`D77`).

**What a level decides is what is kept, not what is noticed.** The filesystem watch runs at every
level, `none` included, so an unindexed project still refreshes its explorer and its Git state.
Nothing is written into the user's project folder at any level.

### How it resolves

`HostSettings::index_level` (schema 5) is the application-wide value, `Light` when unset.
`ProjectRecord::index` is an `Option<IndexLevel>` — **an override, not a value**, so absent means
"follow the setting" and moving the default moves every project that never said otherwise.
`coordinator.rs::index_level` is the one place the two are resolved, and `settle_index` acts on the
answer whenever either could have moved: the project's own `UpdateProject`, or a `SetSettings` that
moved the default under every open project.

`UpdateProject` carries the override as an `IndexChange` — `Inherit` or `Set(level)` — because
serde reads an absent field and an explicit `null` into the same `Option::None`, and "clear the
override" has to be distinguishable from "say nothing about it".

This is a widening of `ProjectRecord::no_local_index`, a boolean that had been carried, stored and
never read. A record written before the widening parses and falls back to the default; the field
was never consulted, so nothing is lost by ignoring its old value.

### What the user sees

Application settings' Search section offers Off / Full text / Full text + symbols
(`ui/settings.rs::index_level_choice`). A project's own settings dialog offers the same three plus
a Default row naming whichever the application says (`ui/sink/project.rs::index_row`), and is drawn
only for a project that already has a record — the Sink form and Create mode are about a project
with no record to override.

A change is sent on the click rather than held until the dialog closes, and the host acts on it at
once: turning indexing off frees the disk while the dialog is still up.

## The full-text index selects candidates; it never produces hits

This is the load-bearing decision (`D75`). Each document is a file's path and its trigrams — the
content is not stored. A query resolves to a set of paths, and `search::hits::scan_file` reads
those files back through the same `grep-searcher` sink the walk uses.

So there is **one** search implementation with a shortcut into it, not two kept in step. Cancel,
supersede, streaming, the batching ceilings, `case_sensitive`, `whole_word` and the highlight
ranges are untouched code. A `LineHit` is byte-identical whichever path chose the file, and the
interface is never told which ran — `SearchProject` gained no field and the search family gained
no variant.

Three things fall out of it:

- **A stale entry costs a read, never a wrong hit.** A candidate that no longer matches contributes
  nothing, so staleness degrades speed and not correctness.
- **`case_sensitive` and `whole_word` need no index support**, because the read re-checks them.
- **A regex query never consults the index**, because a term index cannot bound an arbitrary
  pattern. It walks.

### Trigrams, not words

`ngram(3,3)` with case folding, queried as the `AND` of the query's trigrams (`D76`). A word
tokenizer is the obvious choice and is wrong: it makes the candidate set a **subset** of what the
walk finds — `needl` would never return the file holding `needles` — and no re-reading recovers a
file the index did not name. Trigrams make it a **superset**, and the re-read discards the false
positives for free. The stemmer is disabled for the same reason.

Trigrams are computed over `chars`, not bytes: a byte window splits a multi-byte character and
emits terms the tokenizer never produced, which would silently answer "no candidates" for any
query containing a non-ASCII character.

Candidates are collected with `DocSetCollector` rather than a ranked `TopDocs`. The body is indexed
without positions or frequencies, so there is no score to rank by and a "top N" would be an
arbitrary order presented as a relevant one; sorting by path instead makes truncation deterministic.

### The filter applies to candidates too

An exclude naming a directory — `vendor` — matches the *directory*. The walk gets pruning free from
`ignore`, so no file inside is ever visited. The candidate path has no traversal to prune, and the
glob does not match the leaf `vendor/skip.txt`, so `worker::allowed` tests every ancestor directory
as well as the file. Missing this returns an excluded folder's files the moment a project is
indexed, and only then.

## Where the work runs

`crates/ubiq-host/src/index/` — one thread named `ubiq-index`, one unbounded queue, one open index
per project, on the shape `search/` and `watch/` already proved. The thread owns every index, so a
writer is never shared and never locked; what leaves it is a read handle carrying no writer, so a
search can never block indexing. A `readers` map beside the queue is what lets the coordinator take
a handle on its own thread rather than queueing a request behind a build in flight.

The index lives in a host-owned `index/` directory under `projects/<ulid>/`, the mirror of the
interface's `ui/` workarea: the interface is never told it exists. Both sit under the project's own
directory, so Forget and the orphan collector remove them without knowing what either holds.

**The cold build runs `search::walk::builder`** — the same walk content search runs, so the two can
never disagree about what a project's files are. It is triggered when a project opens, not at boot:
an index for a project nobody has open is work nobody asked for.

**A build empties the index first**, even when a stamped one is on disk. The walk overwrites every
file it finds, but nothing visits a file *deleted* while the project was closed, and its document
would survive as a candidate that cannot exist. Nothing is lost by this today, because there is no
per-file reuse and a build re-reads the whole tree either way (`G154`).

### The watcher feeds it directly

The watch thread pushes `ProjectFilesChanged` straight to its client and the coordinator never sees
it, so there is nothing for the coordinator to relay. A `watch::Job` therefore carries an optional
sender into `index::Job::Changed` beside its mailbox — one extra send per flush, downstream of a
debounce that coalesced the burst.

Per path it is delete-then-add, which is one code path for create, modify and delete; a rename
arrives as two paths that each do the right thing. A `truncated` burst — the watcher dropped the
names — clears the index and rebuilds from the tree, and searches walk while that runs.

Commits are debounced: the queue going quiet for `COMMIT_QUIET`, or `COMMIT_PENDING` paths piling
up, whichever comes first. A commit is the expensive part of an incremental update and an editor
saving a file produces a burst rather than one event.

### Disposal

One `STAMP`. A stamp that is missing, unreadable or from another build, or a directory tantivy
refuses to open, deletes the directory and builds cold. It is never repaired and never migrated:
repairing derived data is a second implementation of building it, and building it costs a walk.

## Ceilings

`crates/ubiq-host/src/index/ceiling.rs`. Files over `MAX_FILE_BYTES` and files with a NUL in their
first `BINARY_SNIFF_BYTES` are never indexed — they are still found by the walk, and by nothing
else. `FILES` bounds how many one project holds. `CANDIDATES` bounds one query's answer and sets
the same `truncated` flag the walk's own ceilings set, meaning the same thing. `MIN_QUERY_CHARS` is
three, below which there is no trigram to look up.

The search family's own ceilings in `search/ceiling.rs` are untouched, because both paths run the
same sink and the same accumulator.

## Failure

| When | What happens |
|---|---|
| The level is `none` | No index directory; the walk answers; the behaviour before any index existed |
| The index is cold, building, or rebuilding after a truncated burst | The walk answers, and nothing says which ran |
| The query is a regex, or shorter than three characters | The walk answers |
| The index errors, or its thread has gone | Logged once, and the walk answers |
| A file is over the size ceiling, or binary | Never indexed; found by the walk alone |
| `CANDIDATES` is hit | The reply carries `truncated`, drawn the way the walk's own truncation is |
| The stamp mismatches, or tantivy refuses the directory | Deleted, and the project builds cold |
| A commit fails | The batch is dropped and logged; a stale index cannot invent a hit |
| The index is stale | A candidate that no longer matches contributes nothing; a file that *became* a match is missed until it is touched or the project reopens |
| A project is closed or forgotten | The handle is dropped; the directory goes with the project's own |
| Two windows hold one project | Two watches, so every change is reported twice. Delete-then-add is idempotent, so the cost is a wasted read (`G155`) |
| The level is `full` | The same as `light`. The symbol half is not built (`G156`) |

## Rules this adds

- **The index selects files; it never produces hits.** Anything that would have the index answer a
  query rather than narrow it is a second search implementation.
- **The candidate set is a superset, never a subset.** Any tokenizer change has to keep that, which
  is what rules out stemming and word tokenizing.
- **The walk is always the answer when the index cannot be one.** Every refusal falls through
  rather than erroring, and the interface is never told which path ran.
- **A level decides what is kept, not what is noticed.** The watch runs at every level.
- **The index is disposable by construction** — stamped, deleted rather than repaired, and never
  consulted to decide what a project's files are.

## What remains

The symbol half, which is what `full` names and does not yet keep. In the order it would be built:

1. **The table.** A host `index/symbols` module with direct tree-sitter dependencies pinned to the
   versions already in `Cargo.lock`, extraction through each grammar's own `TAGS_QUERY`, and a
   vendored markdown headings query. Nine grammars ship a `TAGS_QUERY`; markdown and Kotlin do not.
   A version skew would compile every grammar twice and fail at *runtime* with a `LanguageError`
   rather than at compile time, so the check is that `Cargo.lock`'s tree-sitter entries do not grow.
2. **Two file-family messages** — `ProjectSymbols` and `SymbolsListed` — with bounded replies and
   an `indexed` flag, so a project below `full` answers rather than errors. Deliberately not the
   search family, whose one-live-search-per-project supersede would let an editor gesture cancel
   the search the user is reading.
3. **Goto-definition and a symbol picker**, both routed through the `navigate` path that already
   opens a file at a line. Find-references stays a whole-word content search at every level.
4. **Definition anchors in the web export.** The export's page is highlighted by highlight.js,
   which replaces `innerHTML` and destroys any anchor injected into the `<code>` block, so the
   anchors go in the existing `{{TOC}}` slot and the line ids are added after highlighting runs.

Until (1) lands, the `full` option is a promise the build does not keep (`G156`).

## Related docs

- [`../tech/decisions.md`](../tech/decisions.md) — `D75`, `D76` and `D77`, the three choices here
  that a reasonable person might reverse
- [`../tech/architecture.md`](../tech/architecture.md) — where the index thread sits among the
  host's workers, and how the watch feeds it
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — `IndexLevel`, `IndexChange`
  and the settings schema
- [`../features/workbench.md`](../features/workbench.md) — what the setting looks like and what a
  change does
