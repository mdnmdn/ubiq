---
id: inbox-indexing
title: Proposal — code navigation over a cached tree-sitter index
kind: proposal
status: proposal
summary: One index instead of three — a per-project table of definitions extracted with the tags queries the vendored grammars already ship, cached on disk per file and refreshed incrementally by the watcher that landed, answering the editor's outline, goto-definition and symbol picker and giving the web export its link targets, by name match rather than by a language server.
read_when: you are deciding how Ubiq makes a codebase navigable — an outline, a jump to a definition, a symbol picker, or a source page whose identifiers are links
updated: 2026-09-05
depends_on: [tech-architecture, tech-structure, tech-transport, inbox-omni, inbox-kb-web, inbox-find]
---

# Proposal — code navigation over a cached tree-sitter index

This document replaces the watcher-and-three-indexes proposal that stood here. The watcher shipped
and is recorded below; the indexing half named three indexes — filename, full text, symbol — and this
keeps one of them, on the argument that the other two answer questions the tree already answers.

The goal is a codebase you can move around in: the editor jumps from a use to a definition and shows
a file's shape, and the web export serves source whose identifiers are links. Both wants are the
same want, and both are answered by one table of names.

## 0. Where the build has got to — 2026-09-05

**The watcher has landed. No index of any kind is built.**

`crates/ubiq-host/src/watch/mod.rs` runs one recursive `notify` watch and one debounce thread per
open project — `QUIET` 150ms, `BOUND` 64 paths — keyed per window and per project in the
coordinator and started when a project opens, with `crates/ubiq-host/tests/watch.rs` over the
classification and the batching. `.git/HEAD`, `.git/MERGE_HEAD`, `.git/index` and `.git/refs/**` set
the `repository` flag on the next flush; everything else under `.git/` is dropped; everything else
is gitignore-filtered. The push is `ProjectFilesChanged`, one variant in the file family, carrying
relative paths and never contents, and the interface folds it into the explorer's merge and
re-issues `RefreshProjectGit` when the flag is set.

Three things the watcher did not close, kept as rows rather than as hedges: its own reading of the
ignore rules diverges from the walk's and over-reports (`G110`); a project's health is still only
probed when somebody asks (`G30`); there is no `CloseProject`, so a watch stops when the window
opens another project or leaves (`G113`).

**Nothing else from the old document exists.** No `tantivy` in any lock file, no host-owned cache
directory, no filename index, no symbol extraction. Tree-sitter is in the tree but only for colour:
`crates/ubiq/src/ui/editor.rs` registers Swift and C# with vendored highlight queries and maps
Ubiq's `FileLanguage` onto the component library's language enum; every other grammar arrives as a
`gpui-component` feature flag, and no Ubiq code has ever constructed a `Parser`, a `Query` or a
`Tree`. The web export is real and lives in `crates/ubiq/src/web_export/` — a process-wide
`tiny_http` listener in the interface crate, serving markdown, directory listings and source, with
syntax colour done by highlight.js in the browser and no notion of a symbol.

## 1. What this proposes, in one line

One host-owned table of definitions per project — every definition in every file the walk allows,
extracted with each grammar's own `tags.scm` query, cached on disk per file and refreshed
incrementally by the watcher's feed — and navigation resolved by matching a name against it, not by
resolving a scope.

## 2. Why one index, and why tree-sitter tags

**Tree-sitter already ships the query.** Nine grammars in the lock file expose a `TAGS_QUERY`
constant beside the highlight query the editor already uses — rust, python, javascript, typescript,
go, java, c, c-sharp and swift — capturing both halves of navigation: `@definition.function`,
`@definition.class`, `@definition.method`, `@definition.type`, `@definition.module`,
`@definition.constant`, `@definition.macro`, `@definition.interface`, and `@reference.call`,
`@reference.type`, `@reference.class`, `@reference.implementation`, each with a `@name` capture.
Extracting a project's symbols is running a query the grammar authors already wrote. Only the
definition half is ever stored, for the reason §3 measures.

**Name matching is the whole navigation model, and its ceiling is stated up front.** A jump from a
use site to a definition is: take the identifier under the cursor, look up every definition with
that name, offer them. One candidate is a jump; several are a picker. This is search-based code
navigation — the model GitHub shipped before stack graphs — and it is wrong exactly where scopes
matter: two methods called `run` on different types are two candidates, a local shadowing an import
is not resolved, and a name defined outside the project has no answer. **No scope resolution, no
type inference, no call graph, no rename.** Those need a language server, and a language server is a
separate proposal with a process lifecycle attached to it.

**The other two indexes answer questions already answered.** A filename index duplicates the
explorer's tree and the walk that builds it. A full-text index means `tantivy`, the one genuinely new
package in the old proposal, to answer a search the host already answers by walking with
`grep-searcher` under the ceilings in `crates/ubiq-host/src/search/ceiling.rs` — and the knowledge
base is a marked subset of the same files, so the same walk with a narrower root answers it too. If
a project ever outgrows that, it is reopened with a measurement rather than an assumption.

**So: one index, and no new third-party package.** Every grammar this needs already resolves in
`Cargo.lock`, as does `bincode` for the cache; what changes is that `crates/ubiq-host/` gains direct
edges to them and to `tree-sitter`, which it has never depended on. No `tantivy`, and no
`tree-sitter-tags` — that crate adds doc extraction and syntax-type resolution over a plain `Query`
run, and a plain `Query` run is forty lines.

## 3. What is stored, and what is not

**Measured on this workspace: 5 637 definitions against 54 008 call sites across 107 000 lines of
Rust.** One definition per nineteen lines, one reference per two, ten references for every
definition. That ratio is the whole storage argument, and it holds roughly across languages because
it is a property of how code is written rather than of a grammar.

**So the table holds definitions and nothing else.** One entry per definition capture:

```rust
pub struct Def {
    pub name: String,   // the lookup key
    pub file: FileId,   // u32 into the project's path table
    pub kind: DefKind,  // u8
    pub line: u32, pub col: u32, pub end_col: u32,
}
```

Beside it, the project's path table and one map from name to the entries carrying it — the only
structure navigation queries. A definition costs about sixty-four bytes with its share of the map.

**Extrapolated to a million-line repository: roughly fifty thousand definitions, near six megabytes,
and it does not grow with how much the code calls itself.** Storing references too would be five
hundred thousand entries and near forty megabytes, to answer one question the host answers another
way. Keeping the source line beside each entry, so a picker could show a signature without reading
the file, would add thirty megabytes at that scale for a string displayed a few dozen at a time; the
picker reads those lines from disk as it draws them, the same metadata-in-memory, content-on-demand
split the markdown reference server settled on.

**References are read per file, on demand, from a single parse — never stored.** This is what the
readers actually want: an export page links the identifiers *on that page*, the editor resolves the
identifier *under the cursor*. Both are one file, and one file is one parse of a few milliseconds.
Nothing needs every reference in a project held in memory except *find all references*, and that one
is answered by the content search that already exists — a whole-word query through
`crates/ubiq-host/src/search/worker.rs` is textual where the tag lookup is syntactic, one notch
coarser than a model that was already approximate, and it costs no memory, no new code and no new
message.

**Ceilings, each with a flag rather than a silent stop:** 2 000 definitions per file, 50 000 files,
500 000 definitions per project, and files above one megabyte are skipped unparsed.

**Coverage is what the grammars give, and a file with no tags is not an error.** The nine languages
above come free. Markdown gets headings as definitions from a hand-written query beside the two
highlight queries already vendored under `crates/ubiq/src/ui/languages/` — three patterns, because
the markdown grammar ships no tags query. Kotlin ships a `tags.scm` without a Rust constant, so it
is vendored the same way if wanted. YAML, TOML, CSS, HTML, bash, SQL and diff have no symbols and
never will here: opening one is fine, navigating inside it does nothing.

## 4. The cache

**One file per project, and the store layer already predicted it** — `crates/ubiq-host/src/store/file.rs`
says in its own words that where volume eventually arrives is the per-project cache, a different
store behind a different trait. This is that store, and it stays a file rather than becoming a
database for the same reason the catalogue did.

**It lives beside the interface's workarea, on the other side of the boundary** — a host-owned
sibling of the `ui/` directory under `projects/<ulid>/`, which the host makes and the interface
never reads. Nothing Ubiq remembers is written into the user's project folder.

**Keyed per file by the cheapest identity the walk already has.** The walk yields metadata for every
entry it visits, so a file is identified by its modification time and its length — no hashing, no
second stat, no content read. A cached entry whose pair still matches is reused; one that differs is
re-parsed; one whose file is gone is dropped; a file the cache has never seen is parsed. **A warm
start therefore costs a walk plus a parse of only what changed**, against a cold start's parse of
everything, which is the entire point: parsing is one to two orders of magnitude dearer per byte
than reading back a compact record.

```rust
struct Cached { version: u16, grammars: u64, files: Vec<(RelPath, Mtime, u64, Vec<Def>)> }
```

**Written back atomically, debounced, and only when something changed** — through
`crates/ubiq-host/src/atomic.rs`, on the same write-temp-and-rename path every other store uses,
after the queue has been idle a few seconds. A crash therefore loses at most the last few seconds of
parsing, and a torn file is impossible.

**Wrong is always cheap, because the cache is disposable by construction.** `version` bumps when the
record layout changes and `grammars` stamps the set of grammar versions compiled in; either
mismatching throws the file away. A corrupt or unreadable cache is deleted and the project is built
cold. It is never consulted to decide what a project's files *are* — only to avoid re-parsing ones
the walk has already found.

**It does not hog disk either.** Fifty thousand definitions is one to two megabytes of `bincode` —
smaller than the source it describes by two orders of magnitude. The directory goes when the project
is forgotten, collected by `crates/ubiq-host/src/gc.rs` with the rest of the project's workarea, and
a cache untouched for thirty days is deleted on the next sweep: a project nobody opens pays nothing
but the seconds of its next cold start.

## 5. Where the work runs

**The index is the host's; the open buffer's outline is the interface's.** This is the split
`inbox-find` already drew for search — inside one buffer crosses no bus, across the project does —
and it saves the entire second half of this feature. The editor is `gpui-component`'s, its
`Highlighter` already owns a parse tree per buffer, already re-parses incrementally on every edit,
already exposes that tree, and already walks it for fold ranges. An outline and a breadcrumb are one
more walk of a tree that is already there, updated as the user types, with no message, no worker and
no dependency added to `crates/ubiq/`. Asking the host for the outline of a file the user is editing
would be asking about a version of it that is not on disk.

**Everything across the project is the host's**, in a module beside `search/` and `watch/` under
`crates/ubiq-host/src/`, on the shape those two proved: a job on a queue, one thread per project,
parallelism inside the worker via the walk's `build_parallel`. It owns parse trees, a path on disk
and file descriptors, which is the placement rule in `tech/project-structure.md` answered three
times over, and it must never sit on the file family's single thread — that is `G36`'s cost paid
twice.

**A file opened outside any project is navigable for free, and this is the split paying for
itself.** The component library parses whatever buffer it is given and does not know whether a
project is open. A loose file dragged onto a window gets its outline, its breadcrumb and a jump to
any definition inside itself, with no host involvement, no index and no message. Only crossing a
file boundary needs a project, which is what a project *is*. The interface therefore never gates an
outline on a project being open, and the host is never asked about a file it cannot see.

## 6. What crosses the bus

**Two variants in the file family, not a new family and not a new search.** A symbol request names a
project and resolves to paths inside it, and the file family is where the transport contract sends
anything that names a project and a path. One request with an op on it, rather than three that
differ only in their argument:

```rust
ProjectSymbols { project_id, of: SymbolsOf }                    // UI → host
SymbolsListed  { project_id, of, defs: Vec<Def>, truncated }    // host → UI

enum SymbolsOf { File(RelPath), Name(String), Prefix(String) }
```

`File` is a document's outline for anything that is not the buffer in front of the user — a web
export page, a knowledge-base document. `Name` is goto-definition: the reply is the candidate set.
`Prefix` is the symbol picker. Failures reuse the file family's `ProjectFileError`, with an empty
path for the two queries that name no file.

**Symbol search in the omni panel is a `Source`, not a message** — a `Batch` arm and a group header,
exactly as `inbox-omni` says a new source should arrive. **Goto-definition deliberately does not go
there**, because that family allows one live search per project and a supersede, and an editor
gesture must not cancel the search the user is reading. That is the whole reason the pair above
exists.

**The index never crosses the bus. Only answers do.** Handing the interface the whole table so it
could query locally is the tempting shape and it is wrong on three counts. It is large: fifty
thousand definitions serialised is megabytes in one message, and the reference table nobody should
store would be tens. It is on a shared channel: the same bus carries every pane's terminal bytes,
and a multi-megabyte frame head-of-lines all of them — the coordinator's reader may not be blocked
by a slow interface, because that stalls the harness itself. And it assumes locality: the
architecture already forbids the interface assuming the pseudo-terminal is local, and a bus treated
as a network link is one where you ship the question, not the corpus. Every reader wants a slice —
one file's definitions, one name's candidates, one prefix's matches — so **every navigation reply is
bounded**: two hundred candidates, four hundred prefix matches, one file's definitions, each with a
`truncated` flag, and none of them streams.

## 7. What the editor gets

**A breadcrumb and an outline** from the buffer's own tree, following the cursor, costing one walk
per edit and nothing on the wire.

**Goto-definition on the identifier under the cursor** — the name comes from the buffer's tree, the
candidates from the host; one opens, several raise the picker the search panel already draws rows
in. **Find-references** is a whole-word content search under a symbol heading rather than an index
lookup, which is the trade §3 makes: textual, so a comment or a string can appear among the hits,
and free. Both are blocked on the same thing: `G107`, that a result can open a file but not a
position, because `ReadProjectFile` carries no line and no editor takes one. Until a destination
with a line locus lands, a definition on line 900 opens at line 1, and this is not worth shipping.
It is phase 0 and it is not optional.

**A symbol picker over the project**, `Prefix` against the table, which is the first thing `G16`'s
unbuilt half — find a file, find a symbol — can be built on.

## 8. What the web export gets

The export exists, in the interface crate, and this changes two things about it and nothing else.

**Identifiers become links.** Rendering a source page, the export parses that one file and runs the
same tags query over it, which gives every definition and every reference on the page it is about to
draw. Definitions become anchors; references become links to a lookup route that resolves the name
against the host's table — one candidate redirects, several list. The per-page parse costs what the
highlighting already costs, and the export asks the host only the small question, once per followed
link, instead of holding a project's worth of symbols for as long as it is up.

**Colour stays in the browser.** `inbox-kb-web` says source is rendered with the same tree-sitter
grammars the editor highlights with; this supersedes that. highlight.js already colours these pages
at no cost to the export, and the index buys the thing highlight.js cannot do. Rendering colour
twice in two engines to arrive at the same page is work for its own sake.

Everything else that document decided stands — one process and one port, slugs derived at start,
path safety re-checked at the HTTP boundary, loopback unauthenticated, LAN behind an ephemeral share
slug.

## 9. Failure

| When | What happens |
|---|---|
| A file fails to parse (mid-edit, or a dialect the grammar rejects) | Its entries stay as they were until it parses; no other file is affected |
| A language has no tags query | Its files carry no definitions; opening them works, navigating inside them does nothing |
| A name resolves to several definitions | A picker, ordered by same-file, then same-directory, then path; never a silent choice |
| A name resolves to none | Nothing happens and the status bar says so — an unresolved use site is the normal case for anything defined outside the project |
| The cache is corrupt, truncated, or stamped for other grammars | Deleted, and the project is built cold |
| A file's modification time is unchanged but its content is not | Its definitions stay stale until it is touched again or the project is rebuilt — the cost of not hashing, and the watcher covers every change made while the project is open |
| The watcher reports a truncated burst | The table is dropped and rebuilt on the next request, the same fallback the explorer takes |
| A project is closed while a build is in flight | The queue is dropped with the project, as the file and search workers already do |
| A ceiling is hit | The reply carries `truncated`; the interface says the project is indexed up to the ceiling |
| A file is open outside any project | Outline and inside-file jumps work off the buffer's own tree; anything crossing a file has nowhere to go and says so |
| Find-references matches a comment or a string | It does, and the heading says the source is a text search |

## 10. Rules this adds

- Navigation resolves names, never scopes. A definition is a candidate, not an answer, and the
  interface never hides that behind a jump it cannot justify.
- Definitions are stored, references are parsed on demand. Anything wanting every reference in a
  project is a content search, not an index read.
- The index never crosses the bus; only bounded answers do. The interface holds no copy of it, and
  no navigation message streams.
- The cache is disposable by construction — versioned, stamped, atomically written, deleted rather
  than repaired, and never consulted to decide what a project's files are.
- Inside the open buffer the interface answers from the tree it already has; across the project the
  host answers. Neither asks the other for what it owns, and nothing about a buffer's own shape
  requires a project to be open.
- Content search stays the live walk. The index answers about names and never about content.
- A language's coverage is whatever its grammar's own query gives. Ubiq does not write tags queries
  for languages upstream has not, beyond the markdown headings case.

## 11. Phases

0. **A destination with a line.** `G107`. Nothing here is usable without it, and it is owned outside
   this document.
1. **The table and the two messages.** The host module, the parallel build, the watcher-fed refresh,
   `ProjectSymbols` and its reply, the nine grammars plus markdown headings. In memory only.
2. **The cache.** Per-file records keyed by modification time and length, atomic debounced write-back,
   version and grammar stamps, eviction on the existing sweep.
3. **The editor.** Breadcrumb and outline from the buffer's own tree; goto-definition and the project
   symbol picker from the table; find-references from the content search.
4. **The web export.** Definition anchors, reference links, the name-lookup route.
5. **Symbol search in the omni panel.** A `Source` arm and a group header.

Phase 1 is worth building alone: a symbol picker over a project is the whole of `G16`'s missing half.
Phase 2 is worth building only once phase 1 is slow enough to notice, and the honest trigger is a
cold start somebody complains about.

## 12. What this asks to be decided

- One index, not three: the filename and full-text indexes are dropped and `tantivy` does not enter
  the build. Knowledge-base and export search ride the live walk that already exists.
- Navigation is name matching over tree-sitter tags, with the ceiling stated rather than implied, and
  a language server is a later and separate question.
- Definitions are stored and references are not: ten references per definition measured here, near
  six megabytes per million lines instead of forty, and find-references falls back to the content
  search.
- The index is cached on disk per project, keyed per file by modification time and length, written
  atomically and thrown away on any doubt — a warm start re-parses only what changed.
- `crates/ubiq-host/` takes its first tree-sitter dependency; the grammars and `bincode` are already
  in the lock file, so this is a set of manifest lines and no new package.
- The open buffer's outline is the interface's, off the parse tree the component library already
  keeps; everything project-wide is the host's, and the buffer half works with no project open.
- The interface never receives the table, only bounded answers, because the bus is shared with every
  pane's terminal bytes and is to be treated as a network link.
- Two file-family variants for navigation plus a search-family `Source` for symbol search, with
  goto-definition deliberately not routed through the search family.
- The export keeps browser-side highlighting, superseding `inbox-kb-web`'s decision to re-render
  source with the editor's grammars.

Backlog rows this leaves open: whether find-references ever earns the stored reference half, priced
at the measured ten-to-one and decided on a complaint rather than a guess; whether the cache's
modification-time identity ever needs a content hash behind it, and what it would cost; the sibling
proposal still cites a full-text index and a symbol section that no longer exist here, and owes a
reconciliation pass; at what project size the live walk stops being enough for knowledge-base
search; whether a language server ever supplies what name matching cannot, and what owns its process
if it does; nested-`.gitignore` reading, so the index's walk and the watcher's filter agree
(`G110`).

## Related docs

- [`omni-search-proposal.md`](./omni-search-proposal.md) — the live walk this leans on instead of a
  full-text index, and the `Source` arm phase 5 adds
- [`kb-web-export-proposal.md`](./kb-web-export-proposal.md) — the export this gives link targets to,
  and the highlighting decision this supersedes
- [`find-in-file-proposal.md`](./find-in-file-proposal.md) — the buffer-versus-project split this
  reuses for outlines
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the family this adds two
  variants to, and the procedure for adding them
- [`../tech/project-structure.md`](../tech/project-structure.md) — the placement rule and the
  workarea this cache sits beside
