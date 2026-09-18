---
id: inbox-help
title: Proposal — In-app help, a bundled documentation panel that is contextual and agent-readable
kind: proposal
status: proposal
summary: A `?` in the titlebar opens a Help panel in the right region that draws versioned markdown from a bundle built outside the Rust build, unpacked once into the config root; links are `ubiq://` destinations so a page can send the reader to a mode, a manifest maps the current view to the page that explains it, an MCP server hands the same pages to agents, and everything downgrades to a "not built yet" page when the bundle is absent.
read_when: you are building the help panel, writing help content, packaging the help bundle, or wiring contextual help from a screen
updated: 2026-09-18
depends_on: [feat-workbench, tech-ui, tech-transport, tech-architecture, tech-operations, wip-kb, inbox-web-panel, inbox-plugin-system]
---

# Proposal — In-app help

Ubiq has documentation for the people who build it and nothing at all for the people who run it.
This proposes the smallest complete path to in-app help: a panel that draws markdown, content that
ships as one prebuilt file, a manifest that says which page explains where you are, and an MCP
server so the agent in the next pane can read the same manual you are reading.

The argument of the document is that **almost none of this is new machinery**. Ubiq already has a
markdown renderer, a dockable panel model, an internal link scheme with anchors, a bundle format
with a reader, a built-in MCP server family, and a rail/view value that names the current context.
Help is the assembly of those six things plus one packaging script.

---

## 1. What exists, and what help borrows from it

| Need | Already in the tree |
|---|---|
| Draw markdown | `crates/ubiq/src/ui/viewer/markdown.rs::render` — GFM, tables, images, selectable, scrollable, mermaid/excalidraw fences |
| A movable panel | `state/dock.rs::PanelKind` + `ui/dock/mod.rs::body` + `app/panels.rs::panel` |
| Internal links | `state/nav/text.rs` — `ubiq://<project>/<view>[/<item>][#<anchor>]`, parsed, with `Locus::Anchor` |
| Link clicks from markdown | `ui/mod.rs::on_link(app, base)` — resolves relative, then `ubiq://`, then http |
| A single-file archive | `UBIQBND1` — writer `ubiq-host/src/web_assets/archive.rs`, reader `ubiq/src/web_export/archive.rs` |
| Missing-asset degradation | `state/web_panel.rs::BundleState::Unavailable { reason }` |
| Built-in MCP servers | `crates/ubiq-host/src/mcp/` — five today, `kb.rs` is the closest sibling |
| "Where am I" | `state/nav.rs::View`, `state/workbench.rs::RailMode`, `PanelKind::name()` |
| A `?` can hang here | `ui/titlebar.rs::render` right cluster, and `ui/overflow_menu.rs::OverflowRow` |

Help adds: one panel kind, one view arm, one host store, one MCP server, one content tree, one
Python packer, and three `just` recipes.

**The relationship to the knowledge base** (`wip-kb`) is worth stating once, because they look alike
and are not. The KB is *per project, user-owned, writable, host-catalogued*. Help is *per install,
app-owned, read-only, shipped with the binary*. Making help a fourth `KbOrigin` would put product
documentation inside a user's project catalogue, and would make "delete this source" a question the
user can be asked about the manual. They stay separate and **share the renderer**, which is the part
that matters. Where the KB solves a problem first — scoped search, in particular — help adopts the
solution rather than a second one.

## 2. The shape

### 2.1 Content

Help content is markdown in the repository, under `help/` at the workspace root — versioned today,
a git submodule pointing at its own repository tomorrow, with no code change when that happens
because nothing reads `help/` except the packer.

```
help/
  manifest.toml            # version, nav order, context map
  index.md                 # the landing page
  getting-started.md
  concepts/
    panes.md  sessions.md  projects.md  glossary.md
  agents/
    starting.md  accounts.md  permissions.md  chat.md
  workbench/
    modes.md  panels.md  explorer.md  search.md  editor.md
  git/
    overview.md  changes.md  history.md  diff.md
  kb/
    overview.md
  troubleshooting.md
  img/
    *.png                  # screenshots, referenced doc-relative
```

A page is GFM under a frontmatter block (§2.2). Headings give anchors through the existing
`Locus::Anchor { slug }`, and the frontmatter is stripped on render by the machinery already in
`markdown.rs::split_frontmatter`.

### 2.2 Page frontmatter

Every help page carries frontmatter, and it is **the source of truth for indexing, linking and
contextual mapping**. The packer reads it once and writes the derived tables into the bundle's
index; nothing at runtime parses YAML.

```yaml
---
id: git-changes                          # stable forever, unique in the namespace
title: Reviewing and staging changes
summary: What the Changes panel shows, how to stage a hunk, and what the badges mean.
keywords: [stage, unstage, hunk, diff, revert]
context: [panel.ubiq.git-changes]        # this page explains that place
order: 20                                # position among its siblings
status: current                          # current | draft | planned
related: [git-overview, git-history]     # ids, not paths
redirects: [git/staging.md]              # paths this page used to live at
since: "2026.09"                         # the Ubiq version the page describes
---
```

`id` is the field that earns the block. Help links are written `[the Changes panel](git-changes)` —
an id, not a path — so moving a page between folders breaks nothing, and `redirects` keeps any
`ubiq://` link a user has already pasted working. The packer resolves ids to paths at build time and
ships the map, so `View::Help { page }` accepts either and the id wins.

The rest each does one job, and none is decoration:

| Field | Who reads it |
|---|---|
| `title` | tree, panel header, back/forward history, `list_help_pages` |
| `summary` | tree tooltip, search result line, `list_help_pages` — the agent's whole basis for choosing a page to read |
| `keywords` | search, weighted above body text; the synonyms a user types that the prose never says |
| `context` | the contextual map (§5), inverted by the packer |
| `order`, `status` | nav tree position; `planned` pages are listed greyed and open on the stub body (§7) |
| `related` | a "see also" strip at the foot of the page, drawn from ids so it cannot rot |
| `since` | shown when the page describes something newer than the running build |

The `context` field is deliberately **on the page rather than in the manifest**. One editing
location beats two, a page that explains a panel says so next to the prose that explains it, and a
deleted page takes its context binding with it instead of leaving a dangling manifest row. The
manifest keeps only what is genuinely global: the version, the nav order, and deliberate overrides
(§5).

### 2.3 The bundle

`just help-bundle` runs `_tools/helpbundle.py`, which walks `help/`, validates it (§6), and writes
**one file** in the existing `UBIQBND1` format — deflated entries, a JSON index, a `u64` LE index
offset, the magic. Python writes it with `zlib` in thirty lines; the reader in
`web_export/archive.rs` opens it unchanged.

It lands at `target/help/help.bundle`, which `.gitignore` already covers through `target/`. Nothing
in `build.rs` or `cargo` knows it exists — that is the point of the separate script.

Resolution at runtime, first hit wins:

1. `UBIQ_HELP_BUNDLE` — an explicit path, for tests and for content authors iterating
2. beside the binary — `Contents/Resources/help.bundle` in the `.app`, `help.bundle` next to
   `ubiq.exe`; `just bundle` and `just bundle-win` copy it in when it exists
3. `target/help/help.bundle` — the development case, found by walking up from the executable
4. none — the panel opens on the unavailable page (§7)

### 2.4 Unpacked, not read in place

On first open the host **extracts the bundle once** into `<config root>/ui/help/<version>/` and from
then on reads plain files. The version in the path is the content version from the manifest, so a
new bundle unpacks beside the old one and the old one is removed on success.

This is the decision that makes everything else cheap, so it deserves its cost stated plainly: it
spends disk (a few megabytes) and one extraction pass to buy three things that are otherwise
separately hard. Images become real paths that `img()` can load — the gpui-component image source
resolves a filesystem path but not a doc-relative one and not an archive entry, which is exactly gap
**G11** for the KB. Doc-relative links resolve with `resolve_relative` and no new resolver. And the
MCP server reads files rather than holding an archive handle. The alternative — serving help through
a web tenant (`inbox-web-panel`) — buys nothing here, because help is markdown and Ubiq draws
markdown natively.

## 3. The panel

`PanelKind::Help`, `PanelClass::Free`, `home()` → `Region::Right`. Free means the user can drag it
anywhere like any other panel; right is where it starts. It is `closable()`, not `is_mode_owned()`,
so it survives a rail-mode change — reading the git page while switching to git mode is the normal
gesture, not an edge case.

Opening it is the recipe `app/shell.rs` already uses four times:

```rust
pub fn reveal_help(&mut self, page: Option<HelpPage>, window: &mut Window, cx: &mut Context<Self>) {
    // set the target page, then:
    let panel = self.panel(PanelKind::Help, cx);
    dock::reveal(&self.dock.clone(), &panel, PanelKind::Help.home(), window, cx);
    cx.notify();
}
```

The body is a header (title, back/forward, a contents toggle, a search field), a collapsible tree
built from the manifest's nav order, and `viewer::markdown::render` for the page. Back/forward is a
`Vec<HelpPage>` with a cursor — help navigation is its own history, not the window's.

Three ways in, all of them opening the same panel:

- the **`?` button** in the titlebar's right cluster, beside `feedback` and `overflow-menu`
- **F1**, which opens help *for the current context* (§5)
- a **Help** row in the overflow menu, for discoverability

The `?` needs an icon the registry may not have; `ubiq-icons` owns that and the draw-review loop
runs before the panel lands.

## 4. Links that do something

A help page's links are `ubiq://` destinations, so documentation can move the reader instead of
describing the move. "Open the git mode" becomes a link that opens the git mode.

Two additions to `state/nav`:

- **`View::Help { page }`** — a destination inside the manual, where `page` is an id (§2.2) or a
  path, id first. `[panes](panes)` and `[panes](../concepts/panes.md)` both land, and
  `ubiq://./help/git-changes#staging` is a deep link anyone can paste. Prose should use the id
  form; the path form exists so a page copied in from elsewhere is not broken on arrival.
- **a project-less form** — help content ships before it knows any project ULID, so it cannot write
  one into a link. `ubiq://./git` means *this view, in the current project*, and is rejected as
  today's `NotALink` if there is no current project. This is the one genuinely new piece of link
  syntax, and it is worth it: without it no shipped page can link to anything.

Link handling is `ui::on_link(root, Some(base))` with `base` set to the page's folder — the
conversation transcript passes `None` today and that is why its relative links do nothing.

## 5. Contextual help

The map is **built from the pages**. Each page's `context:` list names the places it explains, and
the packer inverts those lists into one key → id table in the bundle index. Keys are the strings the
UI already computes.

`manifest.toml` holds only what no single page can state:

```toml
[help]
version = "2026.09.1"
namespace = "ubiq"

[nav]                       # the tree, in order; a folder without a row is hidden
order = ["index.md", "getting-started.md", "concepts/", "agents/", "workbench/", "git/"]

[context]                   # overrides only — wins over any page's own claim
"rail.kb" = "kb-overview"
```

Resolution is most-specific-first, and stops at the first hit:

1. an **explicit page** the screen named for itself
2. `panel.<PanelKind::name()>` of the focused panel — those names are already stable strings
   (`"ubiq.git-changes"`) chosen for saved layouts, and reusing them means a context key cannot
   drift from a renamed enum variant without the lint (§6) noticing
3. `view.<View>` of the current destination
4. `rail.<RailMode>`
5. `index.md`

The "explicit page" rung is how a specific place gets a specific page: a modal, a dialog or a
settings section declares `fn help_page() -> Option<&'static str>` and F1 there lands on it. Nothing
else in the UI needs to know help exists.

Two keys claimed by two pages is an error the packer refuses, not a last-write-wins race — and the
manifest's `[context]` table is the deliberate way to settle one. Both halves are data rather than
code, so the content repository can retarget contextual help without a release, which is the whole
reason this is a table and not a `match`.

## 6. Tooling, and Windows

Three recipes, all of them a single `uv run` so they behave identically on both platforms:

| Recipe | Does |
|---|---|
| `just help-bundle` | validate, then write `target/help/help.bundle` |
| `just help-check` | validate only — every page has an `id`, ids are unique, every link and `related` id resolves, no `redirects` path collides with a live page, every image exists, every `context` key is claimed once and is a real `PanelKind`/`View`/`RailMode` name, no page orphaned from `nav.order` |
| `just help-clean` | remove `target/help/` |

`help-check` joins `just verify`, and passes trivially when `help/` is absent.

`_tools/` is already all Python under `uv run` with PEP-723 inline metadata, so `helpbundle.py`
follows `webassets.py` exactly and introduces no new toolchain. The recipes themselves contain no
`rm`, no `cp`, no shell loop — the script does that work through `pathlib`.

This does **not** make the Justfile run on Windows. `Justfile:1` calls
`_devops/scripts/bundle-version.sh` at parse time, so every recipe needs bash before any recipe
runs; `bundle-win` is Windows-targeting but Unix-shell-only. That is a pre-existing blocker with its
own home in `wip-windows-build`, and the help work's obligation is to add nothing to it. Fixing it
is a backlog row, not this proposal's scope.

## 7. When the bundle is not there

The bundle is optional at every layer, because a contributor who clones and runs `just dev` has not
built it and should not be stopped.

- **Host**: no bundle → `HelpState::Unavailable { reason }`, exactly the shape
  `BundleState::Unavailable` already has. No error dialog, no log at warn level.
- **Panel**: opens on a built-in page compiled into the binary — one `include_str!` of a short
  markdown file that says help content is not built and names `just help-bundle`. The `?` button is
  never disabled and never hidden; a disabled `?` is a worse answer than a page that explains
  itself.
- **A missing page** inside a present bundle renders a "this page has not been written yet" body
  with the path, and the link that led there still highlights. Half-written help is the expected
  state for a long time, and it has to look deliberate.
- **MCP**: `list_help_pages` returns an empty list with a `note`, rather than failing the call. An
  agent that asks for help and gets an error will report a broken tool; one that gets an empty list
  will move on.

## 8. The MCP server

A sixth built-in server in `crates/ubiq-host/src/mcp/help.rs`, modelled on `kb.rs`, reaching the
agent through the in-process path that already exists — `McpRef::InProcess` on the `RunSpec`,
rewritten by `provision::host_inproc_mcps` into a loopback HTTP server before the harness sees it.

Four tools:

| Tool | Returns |
|---|---|
| `list_help_pages` | the nav tree — `id`, `title`, `summary` per page |
| `read_help_page` | one page's markdown, by id, frontmatter stripped |
| `search_help` | pages matching a query over `keywords`, `title`, `summary` and body, with the matching lines |
| `help_for_context` | the page bound to a context key |

Every tool addresses a page by `id`, never by path — the same handle a help link uses, so a page the
agent cites can be pasted back to the user as `ubiq://./help/<id>` and will open.

Why it earns its place: the agent sitting in the next pane is the one the user asks "how do I do X
in Ubiq". Without this it answers from a training cutoff and invents menus. With it, it answers from
the same manual the `?` button opens, and `help_for_context` lets it answer about where the user
actually is.

## 9. Ubiq now, Studio later

The resolver takes a **list** of bundle roots, not one, and each root declares a namespace in its
manifest. Ubiq ships `ubiq`; Studio adds `studio` and its own bundle beside the first.

Merge rules, stated now so the second bundle is not a rewrite: page paths are namespaced
(`ubiq/git/overview.md`), nav trees concatenate in root order, and a later root **may add** context
keys and **may override** one it declares deliberately — but the override is in the later manifest,
never a patch of the earlier one. One bundle, one file, no cross-references that assume the other is
present.

## 10. Phases

Each phase is shippable and leaves the tree working.

1. **The panel, empty.** `PanelKind::Help`, the `?` button, F1, the overflow row, the built-in
   unavailable page. No bundle, no content. Proves the panel docks, moves and closes.
2. **Content and packaging.** `help/` with the base pages of §2.1, `_tools/helpbundle.py`, the three
   recipes, resolution and extraction, the tree and the renderer. Help works end to end.
3. **Navigation.** `View::Help`, `ubiq://./`, relative links, anchors, back/forward, search.
4. **Context.** The manifest `[context]` table, the five-rung resolution, `help_page()` on the first
   two or three screens that want it.
5. **MCP.** The four tools.
6. **Studio.** The root list and namespacing, when there is a second bundle to prove it.

Phases 1 and 2 are the useful minimum; a reader with a rendered manual and no contextual mapping is
far better served than today.

## 11. What this owes the library when it lands

- A feature document, `features/help.md` — deleting help from Ubiq would delete it, so it is a
  feature by the placement rule, and it becomes the owner of every fact here.
- A fact-ownership row in `INDEX.md` §3, which has no row for help or for the KB today.
- Decision rows, next free number **D146** (D97–D100 are reserved unwritten by
  `inbox-plugin-system`): the bundle built outside the Rust build; unpack-on-open rather than
  read-in-archive; help kept separate from the KB while sharing the renderer; pages addressed by a
  frontmatter `id` rather than by path, with the context map inverted out of the pages themselves;
  contextual help keyed on existing panel and view names; `ubiq://./` as the project-less link form.
- Backlog rows from **G295**: the Justfile's parse-time bash dependency, which help does not fix;
  help search scoping, which waits on the KB's; the screens that have no `help_page()` yet.
- `tech/operations.md` gains the three recipes.

## 12. Later, deliberately not now

**An agent inside the help panel.** A composer at the foot of the panel, a conversation scoped to
the manual — the user asks a question in prose, and an agent answers with the MCP server of §8
pointed at the pages, citing the ones it used as `ubiq://` links the user can follow. The panel is
already a chat surface's shape and the tools already exist by phase 5; what is missing is the policy
question of which account and which profile answers a question about the product, and whether that
conversation is recorded like any other. Worth doing, and worth doing after the manual is written,
because an assistant over three pages is a demo.

**Interactive on-screen help.** Help that points at the running interface rather than describing
it — a coach-mark overlay that highlights the rail and names its modes on first run, a "show me"
link in a page that opens the panel it is about and outlines it for a moment, a guided first-agent
walkthrough. This wants an overlay layer the window does not have and an anchor vocabulary that can
name a live element, which is a larger piece of UI work than everything above. The groundwork it
needs from this proposal is already here: stable panel names, `ubiq://` destinations, and a manifest
that maps context to content.

---

## Related docs

- [`wip/kb.md`](../wip/kb.md) — the knowledge base, the renderer help shares, and gap G11
- [`features/workbench.md`](../features/workbench.md) — rail modes, panels, regions
- [`tech/ui-and-design.md`](../tech/ui-and-design.md) — the render model and the kit
- [`inbox/web-panel-proposal.md`](./web-panel-proposal.md) — the tenant model help does not need
- [`inbox/plugin-system-proposal.md`](./plugin-system-proposal.md) — contributed content, and D97–D100
- [`wip/windows-build.md`](../wip/windows-build.md) — the Justfile's Windows blocker
