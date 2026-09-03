---
id: inbox-kb-web
title: Proposal — KB mode and local web export
kind: proposal
status: proposal
summary: Fills the empty KB rail with a per-project curation of marked docs, a native explorer/reader/assistant panel, and an on-demand local web server — one process, one port, projects and shares told apart by URL segment — that serves the curated knowledge base or the whole project as a read-only, searchable site.
read_when: you are building the KB rail mode, or deciding whether and how Ubiq exposes a project over HTTP
updated: 2026-09-03
depends_on: [tech-architecture, tech-structure, tech-transport, feat-workbench, feat-chat, inbox-indexing, inbox-omni, inbox-markdown]
---

# Proposal — KB mode and local web export

`refs/markdown-web` is the model this borrows from directly: a single Go binary pointed at a
directory, serving it live over HTTP with a filesystem-mirrored URL scheme, a `notify`-driven
live-reload, and no build step. It solves one project at a time. Ubiq's version has to solve one
process serving several — a user with three projects open should not get three ports — so the
routing scheme borrows the shape and drops the one-directory-per-process assumption.

`RailMode::Kb` already exists — `crates/ubiq/src/state/workbench.rs` gives it a label, "KB", a note,
"Notes and documents the agents can read," and a place in the `PROJECT` group — and draws as an empty
page today through `dock/mod.rs`'s catch-all fallback. That emptiness is half of `G11`. This proposes
what fills it, and a second capability that rides beside it because the rail is its natural home:
serving that same content, or the whole project, over the network.

## 1. Where it stands

**Nothing marks a file or folder as knowledge base content.** No curation model, no store for one,
exists anywhere in the tree.

**Ubiq renders markdown natively, not to HTML.** `TextView::markdown` from `gpui-component` draws
straight to the desktop canvas in three places today (`inbox-markdown`'s own accounting); there is no
markdown-to-HTML pipeline in the codebase. A web export needs one, and inherits `inbox-markdown`'s
frontmatter-stripping convention rather than reinventing it.

**A local, embeddable HTTP server already has a precedent in this workspace.** `crates/agent-manager`
runs one today — `tiny_http`, behind its `inproc-mcp` feature, serving MCP tools to a hosted harness
on a loopback port (`crates/agent-manager/src/mcp/server.rs`). Nothing named `axum`, `hyper` or `warp`
appears anywhere in `Cargo.lock`. The web export in this proposal reuses that same crate and pattern
rather than introducing an async web framework for a read-only file server.

## 2. Two things, one rail

**Knowledge base mode (native).** A per-project curation — folders and files marked "doc" — browsable
and searchable inside Ubiq, three regions: left an explorer scoped to what is marked, titled from the
symbol index's heading extraction ([`inbox-indexing`](./indexing-fswatch-proposal.md) §4.3) where
available; centre the same native markdown viewer `inbox-markdown` already establishes, not a second
renderer; right an assistant — a `ConversationView`
(`crates/ubiq/src/ui/conversation/mod.rs`) scoped to the knowledge base's content rather than the
fixture it draws today. The assistant panel is designed here; it is not buildable until the chat
transport family lands (`G10`, `feat-chat`), and this proposal says so rather than assuming it solved.

**Web export (on demand).** The same content, or the whole project, served read-only over HTTP —
independent of curation, since exporting an entire project needs no marking step at all.

| | Marked by | Served natively | Served over HTTP |
|---|---|---|---|
| Knowledge base | Yes — a curated set | The KB panel (§4) | KB-only export (§5) |
| Full project | No marking needed | The existing explorer and viewers | Full-project export, with code exploration (§5) |

## 3. Marking knowledge-base membership

Marking a folder or file "doc" is a set membership, not a copy — the knowledge base is a view over
the project's own tree, the same principle a bookmark follows, and unmarking never deletes anything.
Marking a folder means everything under it, the same recursive shape a `.gitignore` rule already has,
inverted.

**Storage:** a new host-owned file, `kb.toml`, a sibling to `tasks.toml` and `view.toml` under
`projects/<ulid>/`, behind the same store trait `crates/ubiq-host/src/store/` already gives those two
— not the interface's `ui/` workarea, which `tech-structure` reserves for what the user would not
miss, and a curation list is exactly what they would. It holds a set of project-relative paths and
nothing else; a title is never stored redundantly, it is read at display time from `inbox-indexing`'s
symbol index.

**Where the action lives:** the explorer's own context menu, once it exists (`G70`) — mark/unmark
rides that menu rather than growing a second one.

## 4. The native panel

Explorer sub-tree scoped to marked paths, an empty state pointing at "mark a folder to add it here,"
a centre pane reusing the existing markdown viewer unchanged, and the assistant on the right as
above. Refreshed by `inbox-indexing`'s `ProjectChanged` push, folded through the same merge-by-path
principle the main explorer already uses.

## 5. The web server

**One process for the whole application, one port — never one per project, never one per export.**
Started the first time any window activates an export, from any project; stopped when the last active
export is deactivated, so an idle Ubiq holds no listening socket nobody asked for. Implementation is
`tiny_http`, reused from its existing precedent rather than adding an async framework for a read-only
file server.

**Routing tells projects and shares apart by URL segment, not by port:**

```
/<project-slug>/path/to/file                    — free, loopback-only view
/<project-slug>/<share-slug>/path/to/file        — a scoped share (§6)
```

`project-slug` is derived from the project's name — kebab-cased, deduplicated against whatever else
is active by appending a short suffix from the project id on collision — computed when the export
starts, not stored. It is stable for the life of the server process and not guaranteed to survive a
restart; a bookmarked link is a link for this session, stated as a v1 ceiling rather than an
oversight. One export is active per project at a time — starting a second one for the same project
supersedes the first, the same supersession `inbox-omni` already uses for a re-run search, rather than
stacking two exports of one project under two slugs.

**Two content modes, chosen when an export starts:**

- **Knowledge-base only** — the marked subset, the direct analogue of pointing `refs/markdown-web` at
  a curated folder.
- **Full project** — everything the project's own ignore rules allow (never `target/`, never
  `node_modules/`), with source files rendered using the same `tree-sitter` grammars Ubiq's editor
  already highlights with, so a browsed source file's colours are the code Ubiq already knows, not a
  second highlighter learning the same languages again. Symbol definitions become link targets, per
  `inbox-indexing` §4.3's stated ceiling — a definition is a destination, a use site is not yet.

**Rendering:** `pulldown-cmark` becomes a direct dependency of `ubiq-host`. It is already in the
workspace's dependency graph — pulled in by the interface crate through `gpui-component`'s diagram
renderer — so this is a new edge from `ubiq-host`, not a new crate for the workspace to audit.

**Search:** the export's search box is answered server-side by `inbox-indexing`'s full-text index
(§4.2 there), scoped to whatever the export covers — never a client-side JSON dump rebuilt per
request the way `refs/markdown-web` does it, because Ubiq already keeps a persisted index and a JSON
dump would duplicate it.

**Live update:** one SSE endpoint per export, fed by `inbox-indexing`'s `ProjectChanged` push — a
connected browser tab is told to reload, the same mechanism `refs/markdown-web`'s watcher already
proves out, debounced and coalesced by the same upstream event rather than a second timer.

**Path safety:** every request resolves strictly under the export's root, `Clean` plus a prefix check
— the same discipline the file family's `rel_path` rules already state on the bus, applied a second
time at the HTTP boundary. Dotfiles and `.git` are excluded, matching the indexer's own walk.

## 6. Sharing on the network

The default bind is loopback only. A request from `127.0.0.1` never needs a token, which keeps the
common case — "expose this locally" — free of any new authentication code.

Sharing on the LAN is a separate, explicit action per export: it rebinds (or adds) a listener on the
LAN interface and mints a share — a random, unguessable slug bound to the export's root, or to a
narrower subtree if the user picks "share just this folder." A request arriving on a non-loopback
address must carry a valid, unrevoked share-slug for that project or is refused with no detail in the
body; a loopback request is never asked for one, share or no share.

**Shares are ephemeral by construction** — held in memory, gone when the export stops or the app
quits. No durable share store, no expiry timer, no audit log of what was fetched: a share-slug is a
capability, not an identity, and these are stated as v1 ceilings rather than gaps.

## 7. Where this appears

`RailMode::Kb`'s arm in `dock/mod.rs`'s `centre()`, replacing today's `not_built()` fallback, the
same way `RailMode::Git` and `RailMode::Agents` already have their own arms. Two new `PanelKind`
variants — the KB explorer sub-tree and the KB assistant — dispatched the same way `Explorer` and
`Chat` already are. An export control — start, open in browser, stop, share — lives inside the KB
panel for KB-only exports, and as a project-level action (the project switcher, or a context menu) for
full-project export, since that one has nothing to do with curation.

## 8. Failure

| When | What happens |
|---|---|
| The default port is taken | Falls back to an OS-assigned one, the same fallback `refs/markdown-web` already proves out |
| The export's project folder disappears mid-serve | Requests 404 from then on; nothing crashes, matching the file family's own "root gone" handling |
| A marked path no longer exists | Dropped from the KB view silently, the same merge-by-path principle as elsewhere |
| A share-slug is wrong or revoked | 403, no reason given in the body |
| No default browser, or the host is headless | The URL still reaches the interface, so a user can copy it manually |

## 9. Rules this adds

- Knowledge-base membership is a view over the project's files, never a copy.
- The web server is one process, one port, for the whole application.
- Loopback access is always free; every other address needs a share.
- A share is a capability, ephemeral by construction — never an account.

## 10. Phases

1. **Knowledge-base membership and the native panel** — explorer sub-tree, centre reuse, no assistant
   yet. The smallest thing that fills half of `G11` with something real.
2. **The assistant** — waits on `feat-chat`'s own transport family (`G10`).
3. **The web server, knowledge-base mode** — the direct `refs/markdown-web` analogue, loopback only.
4. **Full-project export**, with `tree-sitter` code highlighting and symbol links.
5. **LAN sharing** — share-slugs.
6. **Server-side search**, once `inbox-indexing`'s full-text phase lands.

## 11. What this asks to be decided

- Knowledge-base membership is a stored set of paths, a view rather than a copy, edited from the
  explorer's own context menu once it exists.
- One web server process for the whole application, `tiny_http`-based, started on demand and stopped
  when idle — never one per project.
- URL routing distinguishes projects and shares by path segment, under one port, not by port per
  project.
- Loopback access needs no token; LAN access needs an ephemeral, revocable share-slug.
- The export's search and live-reload both ride `inbox-indexing`'s index and push rather than
  reimplementing either.

Backlog rows this leaves open: whether a project-slug should be user-editable and stable across
restarts rather than derived fresh each session; whether knowledge-base-only and full-project export
can run for the same project at once, under different slugs, or must stay one at a time; scoping a
share to something narrower than a whole export's root; anything durable about shares beyond the
serving session; the assistant's context model once `feat-chat` exists; whether "code exploration"
ever grows past definition links into `find all references`, which waits on `inbox-indexing`'s own
symbol-index ceiling.

## Related docs

- [`indexing-fswatch-proposal.md`](./indexing-fswatch-proposal.md) — the watcher and indexes this
  serves from
- [`omni-search-proposal.md`](./omni-search-proposal.md) — `Source::Kb`, which this is what finally
  gives content to search
- [`markdown-rendering-proposal.md`](./markdown-rendering-proposal.md) — the native viewer the centre
  pane reuses
- [`../features/workbench.md`](../features/workbench.md) — rail modes and panel ownership
- [`../features/chat.md`](../features/chat.md) — the assistant panel's dependency
- [`../tech/architecture.md`](../tech/architecture.md) — crate placement, the no-absolute-path rule
