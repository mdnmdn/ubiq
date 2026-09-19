---
id: wip-help
title: How in-app help works
kind: wip
status: draft
summary: The mechanics of Ubiq's help system end to end — the content tree and its page frontmatter, the packer and the bundle it writes, how the app finds and unpacks it, how the panel renders and navigates a page, how a context key becomes a page, what the MCP server exposes, and what happens at every point where something is missing.
read_when: you are writing a help page, changing the packer or the bundle, wiring a screen to contextual help, or debugging why the help panel shows the wrong thing
updated: 2026-09-19
verified: 2026-09-19
code_anchors: [crates/ubiq/src/app/help.rs, crates/ubiq/src/state/help.rs, crates/ubiq/src/ui/help/mod.rs, crates/ubiq-host/src/mcp/help.rs, _tools/helpbundle.py]
depends_on: [inbox-help, feat-workbench, tech-ui, tech-transport, tech-operations, wip-kb]
---

# How in-app help works

The design this describes is settled in [`inbox/help-proposal.md`](../inbox/help-proposal.md), which
argues *why*. This document is the *how*, for someone who has to write a page or change the
machinery. Where the two disagree, the proposal is the older document and this one is the record of
what was actually built.

Help is four artifacts and one rule connecting them.

| Artifact | Lives | Written by | Versioned |
|---|---|---|---|
| The content tree | `help/` | a human, in markdown | yes — in git, later a submodule |
| The bundle | `target/help/help.bundle` | `_tools/helpbundle.py` | no — `target/` is ignored |
| The unpacked root | `<config root>/ui/help/<version>/` | the host, on first open | no — a cache |
| The built-in stub | compiled into `crates/ubiq` | `include_str!` | yes |

The rule: **the content tree is the only thing anyone edits, and every derived table in the bundle
comes out of the pages themselves.** There is no second place to register a page.

---

## 1. The lifecycle of a page

A page moves through six steps, and a human is present for exactly the first one.

1. **Authored** — `help/git/changes.md`, markdown under frontmatter.
2. **Packed** — `just help-bundle` runs `_tools/helpbundle.py`, which validates the tree, resolves
   ids, inverts the context map and deflates everything into `target/help/help.bundle`, one
   `UBIQBND1` file. `dev`, `verbose`, `build`, `bundle` and `bundle-win` all depend on `help-bundle`,
   so packing is not a step anyone has to remember — it runs before any of them.
3. **Shipped** — `just bundle` and `just bundle-win` copy that file beside the binary. Development
   runs out of `target/` too, since `dev` and `build` pack it there before `cargo run`/`cargo build`.
4. **Resolved** — the host looks in `UBIQ_HELP_BUNDLE`, then beside the binary, then `target/`, then
   gives up (§4).
5. **Unpacked** — once, into `<config root>/ui/help/<version>/`. Plain files from here on.
6. **Drawn** — `HelpReady` reaches the UI, and the panel renders through
   `viewer::markdown::render` with links handled by `ui::on_link`.

Nothing in `cargo build` touches any step of this — packing is a Justfile dependency, not a Cargo
one. A tree with no `help/` folder and no bundle still compiles, runs, and opens a help panel that
says so.

## 2. The page

### 2.1 Frontmatter

Required on every page. It is read once, by the packer; no YAML is parsed at runtime.

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | The page's handle, stable forever, unique within the namespace. Links, MCP tools and the context map all address this, never a path |
| `title` | yes | Shown in the tree, the panel header and the history |
| `summary` | yes | One sentence. The tree tooltip, the search result line, and the only thing an agent sees before choosing to read the page |
| `keywords` | no | Search terms weighted above body text — the words a user types that the prose never uses |
| `context` | no | Context keys this page explains (§5). Inverted by the packer into the map |
| `order` | no | Position among siblings. Absent sorts last, alphabetically |
| `status` | no | `current` (default), `draft` (renders with a work-in-progress note), `planned` (listed greyed, opens the stub body) |
| `related` | no | Ids drawn as a "see also" strip at the foot of the page |
| `redirects` | no | Paths or ids this page used to answer to, so old links keep working |
| `since` | no | The Ubiq version the page describes, surfaced when it is newer than the running build |

```yaml
---
id: git-changes
title: Reviewing and staging changes
summary: What the Changes panel shows, how to stage a hunk, and what the badges mean.
keywords: [stage, unstage, hunk, diff, revert]
context: [panel.ubiq.git-changes]
order: 20
status: current
related: [git-overview, git-history]
---
```

### 2.2 Body

Plain GFM, rendered by the window's one markdown renderer,
`crates/ubiq/src/ui/viewer/markdown.rs::render` — tables, task lists, code fences, and the
`ubiq-diagram-fence` extension, so a ```` ```mermaid ```` block in a help page draws as a diagram
like anywhere else.

Two conventions the renderer does not enforce:

- **No `# h1`.** The `title` is the heading; the panel draws it. Start the body at `##`.
- **Headings are link targets.** They slug into `Locus::Anchor`, so any `## Staging a hunk` is
  reachable as `#staging-a-hunk` from anywhere, forever. Renaming one breaks inbound links the same
  way renaming a file would, which is what `redirects` is for at page level and nothing is at
  heading level — so rename headings sparingly.

### 2.3 Links

Three forms, all handled by `ui::on_link(root, Some(base))` with `base` set to the page's folder.

| Written | Goes to |
|---|---|
| `[the Changes panel](git-changes)` | another help page, by id — **the form to use** |
| `[the Changes panel](../git/changes.md)` | the same page, by path; works, but breaks if the file moves |
| `[git mode](ubiq://./git)` | somewhere in the running interface |
| `[staging](git-changes#staging-a-hunk)` | a heading in another page |
| `[the ACP spec](https://…)` | the system browser, through `cx.open_url` |

`ubiq://./<view>` is the project-less form: *this view, in the current project*. Help content ships
before it knows any project ULID, so it can never write one; a page that links to `ubiq://./git`
opens git mode in whatever project is open, and does nothing if none is. The full
`ubiq://<project>/<view>` form still parses, and is what the user gets if they copy a link out of
the panel to send to someone.

### 2.4 Images

Doc-relative paths into `help/img/`:

```markdown
![The Changes panel](../img/git-changes.png)
```

They work because the bundle is unpacked to real files before anything is rendered (§4) — the image
source underneath resolves a filesystem path, not an archive entry. Screenshots are PNG, taken at 2×
on the dark theme unless the page is about theming.

## 3. The packer

`_tools/helpbundle.py`, run through `uv` like every other tool in `_tools/`. Three recipes, all
depended on by `dev`, `verbose`, `build`, `bundle` and `bundle-win` (except `help-check`, which only
`verify` pulls in):

| Recipe | Does |
|---|---|
| `just help-bundle` | validate, then write `target/help/help.bundle` |
| `just help-check` | validate only. Part of `just verify`; passes trivially when `help/` is absent |
| `just help-clean` | remove `target/help/` |

`bundle`/`clean` take `--out-dir DIR`, default `target/help`. Ubiq Studio compiles both `ubiq` and
`ubiq-studio` into its own `target/`, one level up from this checkout, so its Justfile calls this
same script with `--out-dir target/help` pointed there instead — the packer itself knows nothing
about Studio (`_docs/tech/operations.md` in `ubiq-studio`).

Validation is the whole value of the step, and it fails the build rather than warning:

- every page has `id`, `title`, `summary`; ids are unique
- every link and every `related` id resolves; every `redirects` entry collides with nothing live
- every image path exists
- every `context` key is claimed exactly once, and names a real `PanelKind::name()`, `View` or
  `RailMode` — the packer reads those out of the Rust source, so a renamed variant fails here
  rather than silently unbinding a page
- every page is reachable from `nav.order`

What it writes is the existing `UBIQBND1` format — deflated entries, a JSON index, a `u64` LE index
offset, the magic — so the reader in `crates/ubiq/src/web_export/archive.rs` opens it with no
change. Alongside the pages it writes one derived entry, `catalog.json`:

```json
{
  "version": "2026.09.1",
  "namespace": "ubiq",
  "pages": [{"id": "git-changes", "path": "git/changes.md", "title": "…", "summary": "…",
             "order": 20, "status": "current", "related": ["git-overview"]}],
  "nav": ["index", "getting-started", "…"],
  "context": {"panel.ubiq.git-changes": "git-changes", "rail.git": "git-overview"},
  "redirects": {"git/staging.md": "git-changes"}
}
```

That file is the runtime's entire index. Nothing at runtime walks the tree or parses frontmatter.

## 4. Finding it, and unpacking it

The host resolves a bundle at first open, first hit wins:

1. `UBIQ_HELP_BUNDLE` — an explicit path, for tests and for content authors iterating
2. beside the binary — `Contents/Resources/help.bundle` in the `.app`, `help.bundle` next to
   `ubiq.exe`
3. `target/help/help.bundle`, walking up from the executable — the development case
4. nothing → `HelpState::Unavailable`

On a hit it extracts into `<config root>/ui/help/<version>/`, writing to `<version>.part` and
renaming — the same "the rename is the only proof of done" discipline as the web asset bundle. A
version already present is used as is; older versions are removed after a successful extract.

The config root is `ubiq_host::config::resolve`, and `ui/` under it is the shared workarea
(`projects::shared_workarea`), so help sits beside the web bundles rather than inventing a location.

## 5. From "where am I" to a page

The UI computes a context key and asks for the page bound to it. Resolution stops at the first hit:

| Rung | Key | Source |
|---|---|---|
| 1 | an explicit page | a screen's own `help_page() -> Option<&'static str>` |
| 2 | `panel.<name>` | `PanelKind::name()` of the focused panel — `"ubiq.git-changes"` |
| 3 | `view.<name>` | `state::nav::View` of the current destination |
| 4 | `rail.<name>` | `state::workbench::RailMode` |
| 5 | `index` | the landing page |

Rungs 2–4 cost the UI nothing: those values exist and are already computed for the dock and the
navigator. Rung 1 is how a modal or a settings section claims a page for itself, and is the only
place a Rust file names a help id.

**To bind a page to a place, edit the page** — add the key to its `context:` list. The manifest's
`[context]` table exists only to settle a key two pages both want, and the packer refuses the
ambiguity rather than picking.

### `manifest.toml`

Everything that is not a property of one page:

```toml
[help]
version = "2026.09.1"
namespace = "ubiq"

[nav]
order = ["index.md", "getting-started.md", "concepts/", "agents/", "workbench/", "git/"]

[context]
"rail.kb" = "kb-overview"    # overrides only
```

## 6. The panel

`PanelKind::Help` — `PanelClass::Free`, `home()` → `Region::Right`, closable, not mode-owned, so it
survives a rail-mode change. It docks, moves and closes like any other panel, and its layout
persists under the stable name `"ubiq.help"`.

Three ways in, all the same panel: the `?` in the titlebar's right cluster, **F1** (which opens the
page for the current context, §5), and a **Help** row in the overflow menu.

The body is a header (title, back, forward, contents toggle, search field), the nav tree from
`catalog.json`, and `viewer::markdown::render` for the page. Back and forward walk a `Vec<HelpPage>`
with a cursor — help history is the panel's own, not the window's, so reading three pages does not
disturb the editor's navigation.

Wire vocabulary, UI → host and back:

| Message | Carries |
|---|---|
| `HelpEnsure` | nothing — "make help available" |
| `HelpReady` | `version`, `root`, the parsed catalog |
| `HelpUnavailable` | `reason`, a sentence fit to show a user |

The panel reads page files under `root` once the host says they are there, the same way the web
panel reads the bundle path the host hands it. That makes local help work and leaves help against a
remote host unsolved — a backlog row, not a silent assumption.

### 6.1 Follow mode

`Help::follow` (`state/help.rs`) is a checkbox in the panel header, off by default, drawn through
`kit::check_box` and toggled by `AppState::toggle_help_follow`. Off is every behaviour above: F1 and
**?** bind a page once, and the reader is free to browse away from it without the panel following
them back.

On, the panel tracks §5's ladder itself. `AppState::bound_help_page` is the ladder read without the
`index` fallback rung 5 adds — rungs 1 to 4 only, so a context that binds nothing answers `None`
rather than the landing page. `help_target` (F1, **?**) still falls back to `index`, because those
always have to open something; `sync_help_follow` reads `bound_help_page` instead, and does nothing
when it answers `None` — **follow only ever carries the reader forward onto a page that exists, it
never blanks the one already open.** When it answers a page different from the one on screen,
`sync_help_follow` visits it exactly as a nav click would.

`sync_help_follow` runs from the two places a context changes without the reader clicking anything:
`AppState::set_rail_mode` (`app/shell.rs`), for rung 4, and `AppState::note_active_panel`
(`app/help.rs`), pushed from the dock whenever the displayed panel changes, for rung 2. It is **not**
called from `enter_project` or the `Scope::Project` config-load path in `app/projects.rs`, which
assign `rail_mode` while restoring a project rather than through an active mode change — so follow
does not yet react to a project switch that lands on a different mode than the one the reader left.
That gap is `G303` in `backlog.md`.

## 7. When something is missing

Every layer degrades rather than failing, because a half-written manual is the expected state for a
long time and a contributor who has never run `just help-bundle` must not be stopped.

| Missing | What happens |
|---|---|
| The `help/` tree | `just help-check` passes, `just verify` passes, nothing is built |
| The bundle | `HelpUnavailable`; the panel opens on the built-in stub naming `just help-bundle`; the `?` is never disabled or hidden |
| A page the nav lists | A "this page has not been written yet" body with the id; the link that led there still highlights |
| A context binding | Falls through to the next rung, ending at `index` — F1 always opens something |
| An image | The renderer draws the alt text; `help-check` would have caught it before the bundle existed |
| Help, from an agent's view | `list_help_pages` returns an empty list with a note; no tool call ever errors |

## 8. What an agent sees

`crates/ubiq-host/src/mcp/help.rs`, a built-in server reaching the harness through
`McpRef::InProcess`, rewritten by `provision::host_inproc_mcps` into a loopback HTTP server before
the harness is launched.

| Tool | Returns |
|---|---|
| `list_help_pages` | `id`, `title`, `summary` per page, in nav order |
| `read_help_page` | one page's markdown by id, frontmatter stripped |
| `search_help` | matches over `keywords`, `title`, `summary`, body — with the matching lines |
| `help_for_context` | the page bound to a context key |

Everything is addressed by `id`, which is why ids are worth their cost: a page the agent cites can
be pasted back to the user as `ubiq://./help/<id>` and will open.

## 9. Adding to help

**A new page.** Write it under the right folder with full frontmatter, add its id to
`nav.order`'s folder if the folder is not already listed, link it from at least one existing page,
run `just help-check`.

**A new contextual binding.** Add the key to the page's `context:` list. If the place has no key
because it is a modal or a dialog, give it `fn help_page() -> Option<&'static str>` returning the id
instead — that is rung 1 and needs no manifest change.

**A new namespace (Studio).** The resolver takes a list of bundle roots. A second bundle declares
its own `namespace`, ships its own `catalog.json`, and its nav tree concatenates after the first.
A later root may add context keys and may deliberately override one it declares; it never patches
the earlier manifest. Page ids are namespaced when they collide (`studio/git-changes`).

## Related docs

- [`inbox/help-proposal.md`](../inbox/help-proposal.md) — the argument, the phases, and what is deferred
- [`kb.md`](./kb.md) — the knowledge base, whose renderer help shares and whose search it inherits
- [`features/workbench.md`](../features/workbench.md) — panels, regions and rail modes
- [`tech/ui-and-design.md`](../tech/ui-and-design.md) — the render model and the kit
- [`tech/operations.md`](../tech/operations.md) — the recipe list
