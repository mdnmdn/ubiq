---
id: inbox-dynamic-workspace
title: Proposal — dynamic workspaces, a scriptable workbench
kind: proposal
status: proposal
summary: A new rail mode whose centre is a dockable workbench built and driven by a TypeScript program on a persistent, host-side QuickJS runtime — docks holding A2UI surfaces, web panels, agent-written live documents (the magic whiteboard), a Playwright-style browser, terminals, chats, editors or database tabs; a capability-granted `ubiq.*` API over layout, menus, state, a local SQLite, files, processes, terminals, HTTP and WebSockets, a loopback server, schedules, model calls, database connections, secrets, connectors and agents; a two-way agent link through the host's MCP server; Run and Author modes on one live instance; a `space.toml` manifest with a settings and permissions panel; zip bundles, a manager, and a signed registry later; and two dozen example spaces graded against the model.
read_when: you are deciding how a user- or agent-authored tool runs inside Ubiq, what a script may reach, how a scripted panel is drawn in the dock, or how such a tool is packaged and shared
updated: 2026-10-13
depends_on: [inbox-plugin-system, inbox-a2ui, inbox-web-panel, ref-a2ui-protocol, ref-a2ui-catalog, feat-workbench, feat-workbench-sink, feat-workbench-db, feat-connectors, tech-architecture, tech-transport, tech-decisions]
---

# Proposal — dynamic workspaces, a scriptable workbench

Every rail mode Ubiq has is compiled in. IDE, DB, Git, Agents, Teams, KB and Tasks are each a
screen somebody wrote in Rust, and anything they do not cover is somebody else's application. The
parts needed to close that gap are already in the tree, but they are not connected:

- QuickJS with an oxc TypeScript front end, used by the Sink's script scratchpad (`D127`–`D129`);
- an A2UI renderer that draws in kit primitives (`state/a2ui.rs`, `ui/a2ui.rs`);
- a loopback web origin with a bridge and an embedded webview (`web_export/`, `ui/web_view.rs`);
- a dock that already hosts terminals, chats, editors and database tabs;
- a host that owns files, pseudo-terminals, database sessions, the keychain, connectors and
  conversations;
- a loopback MCP server the agents already reach (`D102`).

This proposal connects them into one new capability. **A dynamic workspace is a small program,
written in TypeScript, that builds its own workbench out of Ubiq's parts.** It decides what docks
exist and what each one shows. It reacts to what the user does in them, and it reaches the host's
features through a granted API. An agent can be wired to it in both directions. Ubiq can also edit
one while it is running, and it can be packed into a zip and given to someone else.

The inspirations are [MCP Apps](https://modelcontextprotocol.io/seps/1865-mcp-apps-interactive-user-interfaces-for-mcp)
(a UI resource beside a tool, a typed bridge, a declared CSP) and
[app-ly](https://github.com/mdnmdn/app-ly). From app-ly it takes three ideas: an app is a folder
with a TOML manifest and static contents, one injected host object replaces many bridges, and
everything the UI can do is also available headless.

## 0. The name

**"Workspace" is taken.** The glossary uses it for one running agent inside a session (see
`product/glossary.md`). "Workbench" is the window's shell, and "panel", "pane" and "dock" are also
in use. This document calls a dynamic workspace a **space**:

| Term | Meaning |
|---|---|
| **Space** | The authored thing: a manifest, a TypeScript entry point, and its UI and asset files |
| **Space instance** | One running copy: a live runtime, its docks, its state and its data |
| **Space bundle** | A space packed as one `.ubiqspace` zip |
| **Dock** (inside a space) | One tab the space's program placed in its workbench, holding one panel |
| **Run / Author** | The two modes of one instance: using it, and editing it |

The rail entry is **Spaces**. Choosing the word is the first decision this document asks for (§18).
Alternatives considered were *app* (collides with `ubiq-app` and `AppState`), *tool* (collides with
tool calls), *plugin* (that proposal is a narrower thing, see §14) and *canvas* (that is the Teams
screen).

## 1. What a space is, in one example

This is a PR review cockpit. It has a list of open pull requests on the left, a diff in the centre,
an agent in the right dock and a terminal at the bottom:

```ts
import { layout, ui, connectors, agents, term, state } from "ubiq";

const gh = await connectors.get("github");                 // a reference, never a token
const prs = await gh.fetch("/repos/{owner}/{repo}/pulls"); // the host attaches the credential
const list = ui.surface("prs", "./ui/pr-list.a2ui.json", { prs: await prs.json() });

layout.open({ id: "list", panel: list, region: "left", title: "Pull requests" });
layout.open({ id: "shell", panel: term.spawn({ cwd: "$project" }), region: "bottom" });
const reviewer = await agents.start({ type: "claude-code", tools: ["space"] });
layout.open({ id: "agent", panel: reviewer.chat(), region: "right" });

list.on("open", async ({ number }) => {
  state.set("last", number);
  layout.open({ id: `pr-${number}`, panel: ui.web("./web/diff.html", { number }), region: "centre" });
  reviewer.prompt(`Review PR #${number}. Use space_get_diff to read it.`);
});
```

Each line uses a feature that already exists in Ubiq. What is new is that a forty-line script can
combine them. The same pattern covers a database dashboard, an incident runbook, a release manager,
an API playground, an agent pipeline, or a small kanban kept in a local SQLite file (§15).

## 2. The rule

> **A space is a bus client with a grant, and nothing more.** Its program runs on the host, holds no
> ambient power, and reaches every feature as a message the grant admits. Its docks are panels the
> interface draws from data it was sent. No path, process handle or credential crosses into the
> interface or into the script that the grant did not name.

This is the plugin proposal's model (`inbox-plugin-system`, `D97`–`D100`) with a workbench added.
It keeps every architecture rule in `AGENTS.md`:

- The UI and the host still talk only through `ubiq-proto` messages.
- The coordinator still renders nothing.
- A terminal dock is still a pane id and an opaque byte stream.
- Credential material stays in the keychain.

## 3. Where the runtime lives — the host, not the interface

The scratchpad runs QuickJS in the interface (`crates/ubiq/src/state/script.rs`). It is stateless
by design: each run builds a fresh runtime, reads a fact snapshot and writes an intent log that
nothing performs (`D129`). A space needs the opposite on every count:

| A space needs | So the runtime must be |
|---|---|
| Files, processes, pseudo-terminals, SQLite, database sessions, the keychain, connectors | Next to them, in the host |
| A long-lived program with timers, promises and subscriptions | Persistent, with an event loop |
| To keep running when its window closes or the UI is remote (drone, `ubiq-proto` framing) | Independent of any window |
| An agent to call it while no window shows it | Reachable from the host's MCP server |
| To run headless from a CLI (`ubiq space run …`) | Free of GPUI |

**Proposal: a new crate, `crates/ubiq-space`, holds the runtime, and `crates/ubiq-host` hosts its
instances.**

- **`ubiq-space`** is a near-leaf. It depends on `ubiq-proto` for ids, and it carries `rquickjs`,
  `oxc`, `zip` and `rusqlite` behind a `runtime` feature. It owns:
  - the manifest schema and its validation;
  - bundle pack, unpack and hash verification;
  - the ES module loader (transpiling bundle-relative `.ts` and `.tsx` on load);
  - the event loop (timers, promises, host-call futures);
  - the `ubiq.*` bindings, written against a `SpaceHost` trait.

  Because the bindings depend only on that trait, the crate never names `ubiq-host`. With
  `--no-default-features` it is just the manifest and wire types, which the interface needs for the
  settings and permissions panels.
- **`ubiq-host/src/space/`** implements `SpaceHost`. Each instance runs on its own worker thread,
  holding one `rquickjs::AsyncRuntime` with a single-threaded executor. A binding call becomes a
  request to the coordinator or to the relevant worker, and its reply resolves the promise. The
  coordinator's reader never waits on a script, and a script never waits on the UI.
- **The scratchpad stays where it is.** It is a fixture page, and `D127`/`D129` remain its rules.
  The oxc front end (`front::transpile`, `validate`) moves into `ubiq-space`, with no default
  features, so both share one transpiler.

Limits come from `D127` and are made per instance. There is an interrupt handler for a synchronous
block (5 s by default) and a memory limit (64 MiB by default, which the manifest may raise up to a
cap). A runaway instance is stopped and its docks show a crash card offering Restart and Author.
None of these limits is a security boundary. §11 says where the boundary is.

## 4. The workbench — docks and what goes in them

### 4.1 The mode

A new rail mode, **Spaces**, is registered through `ext::rail::register` (`D184`). Its centre is the
active instance's dock area, and a strip at the top of the centre switches between open instances.
When no instance is open, the empty state lists the installed and dev spaces with **New space**.
A space may also ask, in its manifest, for a pinned rail entry of its own (`[rail] pin = true`).
The interface then registers a `RailModeSpec` at runtime for it. The registry is already data, so
this is a registration made later, not a new mechanism.

### 4.2 The dock

Each instance gets its own `DockArea`, using the same `WorkbenchPanel` wrapper as the other modes.
Every tab in it is a new `PanelKind::Space { instance, dock }`. The `dock` id is chosen by the
script and scoped to the instance. The tab's body is drawn from a **panel descriptor** the host
sent:

| Panel kind | Drawn by | Notes |
|---|---|---|
| `a2ui` | The existing A2UI renderer | Native, themed, no fetch (`D115`). Surfaces become keyed and incremental (§5) |
| `web` | The embedded webview on the loopback origin | A generic `space` tenant serving the bundle's HTML (§6). On Linux the external-browser fallback applies |
| `live` | The embedded webview, on a Ubiq-owned shell page | A page the runtime or an agent writes and patches in real time (§6.1) |
| `browser` | The embedded webview, or a screencast image for a host-driven Chromium | Navigates the open web within granted hosts, driven by script or agent (§6.2) |
| `terminal` | `terminal::pane` | A real `PaneId` spawned by the host. Resize, focus and `TabClose` behave as everywhere else |
| `chat` | The chat panel | A view onto a host-owned conversation the space started or attached to |
| `editor` / `markdown` / `viewer` | The IDE's editor and viewers | A project file, or a file in the instance's data folder |
| `db-sql` / `db-table` | The DB mode's tabs | A saved connection the grant names. The read-only guard keeps its three layers |
| `explorer` | The files tree | Rooted at a granted path |
| `log` | A console panel | The instance's `console.*` output and errors |

The last six rows reuse existing panel bodies. The new `PanelKind` arm delegates to them, which is
what lets a dock hold "any panel" without the closed enum growing once per kind.

### 4.3 Who owns the layout

**The script owns the logical layout, and the user is allowed to move things.**

- The script describes docks by id, region, order, title, icon, size hint and closability.
- The interface maps that onto `DockArea`.
- When the user drags, resizes or closes a tab, the interface reports `SpaceLayoutChanged`, and the
  script sees it as an event.
- On restore, the persisted `DockAreaState` blob (per instance, beside the mode layouts in
  `ViewPrefs`) wins over the script's first layout. A user's arrangement therefore survives a
  reload, and a script can call `layout.reset()` to throw it away.

A space's own `TabClose` setting applies to its docks: `hide` keeps the panel alive and lets the
script reopen it, and `close` disposes it. A terminal dock still obeys the user's terminal setting.

### 4.4 Chrome a space can add

A space can contribute the following:

- commands, which appear in the command palette, prefixed by the space name;
- a space menu in the titlebar;
- context-menu items on its own docks;
- status-bar items;
- keybindings scoped to the instance;
- toasts and notifications, through the existing notification family;
- modal dialogs: confirm, prompt, the file picker in its existing shapes, and an A2UI modal.

All of these are declarative descriptors. None of them draws anything outside the space's docks.

## 5. A2UI as the native panel

A2UI is the default UI for a space. It is native, it follows the theme and the size axes, and it
cannot fetch, navigate or introduce a colour. The renderer handles one `createSurface` today
(`G250`, `G206`). A space needs the rest of the protocol:

- surfaces keyed by `surfaceId`, one per `a2ui` dock;
- `updateComponents`, `updateDataModel` and `deleteSurface`, applied incrementally to `Live`
  without rebuilding it;
- `userAction` envelopes routed to the host as `SpaceAction`, delivered to `surface.on(name)`
  handlers with the bound data model when `sendDataModel` asks for it;
- two-way input, so a text field's model value reaches the script when it changes and not only when
  the user acts.

In the script, a surface is an object whose data model is a JSON value:
`surface.set("/prs/0/title", …)` sends an `updateDataModel` and `surface.render(tree)` sends an
`updateComponents`. Definitions live in `ui/*.a2ui.json` files or are built in code. Ubiq's own
catalog (`ubiq-catalog.json`) gains the components a workbench needs and the basic catalog lacks,
each of which goes through the catalog rules: `Table`, `Tree`, `CodeView`, `Chart`, `Badge`,
`KeyValue` and `Markdown` (through `ubiq-md`).

This is also where `inbox-a2ui` lands. Its `render_ui` tool draws one surface inside a conversation,
and a space's `space_render` tool (§8) draws one into a dock. Both use the same renderer. A2UI
surfaces outside a conversation were left to "a different proposal with a different risk profile"
(`inbox-a2ui` §7), and this is that proposal.

## 6. Web panels — the escape hatch

Some panels are not forms: a chart library, a diff viewer, a map, a canvas. For those, a `web` dock
serves the bundle's own HTML on the existing loopback origin under a generic **`space` tenant**:

- **Route.** `/_web/space/<token>/…` is served from the instance's bundle folder, read-only. The
  token is per dock.
- **CSP.** It is default-deny. It is built from the manifest's `[web.csp]`
  (`connect`, `resource`, `frame` domains, after MCP Apps' `_meta.ui.csp`), and the host may tighten
  it but never loosen it. With no declaration, the page reaches only its own origin and the bridge.
- **Bridge.** `bridge.js` gains a JSON-RPC channel next to its document frames:
  - `space.call(fn, args)` invokes a function the script exported to that dock;
  - `space.on(event)` receives what the script emits to it;
  - `ui/initialize` returns the theme tokens and palette, so a page can match the window.

  The frame vocabulary stays closed, and the page never receives `ubiq.*` itself. It reaches
  whatever its script chooses to export, and that script is bound by its grant.
- **Vendored libraries** ship inside the bundle, hash-listed like everything else. There is no CDN
  unless the CSP declares one.
- **Platforms.** The embedded webview covers macOS and Windows. Linux falls back to the external
  browser, as for every other web panel, until `gpui-wry`'s Unix path lands.

A `web` dock may also host an **MCP App**: a `ui://` resource read from an MCP server the space
connected to. The bridge then speaks MCP Apps' `ui/*` methods, proxying `tools/call` back to that
server. Ubiq thereby becomes an MCP Apps host for no more than a second protocol on a bridge it
already has (§14).

### 6.1 Live documents — the magic whiteboard

A **live document** is a `web` dock whose page the runtime writes, not the bundle. It is what an
agent needs to show *anything* — a diagram, a slide, a table, a game board, a mock-up — while it is
thinking, and to change it as the conversation moves:

```ts
const board = ui.live({ id: "board", trust: "generated" });   // a web dock with no bundle page
layout.open({ id: "board", panel: board, region: "centre", title: "Whiteboard" });
mcp.tool("board_write", { html: "string", target: "string?", mode: "replace|append|morph" },
  ({ html, target, mode }) => board.patch({ target: target ?? "#root", html, mode }));
board.on("action", (e) => agent.prompt(`The user clicked ${e.name}: ${JSON.stringify(e.data)}`));
```

The mechanics:

- **The shell.** The dock loads a small Ubiq-owned page, `live.html`, on the `space` tenant. It has
  an empty `#root`, the theme tokens as CSS variables, and a patch applier. Content arrives as
  bridge frames — `Replace{html}`, `Patch{target, html, mode}`, `Style{css}`, `Script{js}`,
  `Assets{map}` — and `morph` uses a DOM-morphing pass (idiomorph, vendored) so a re-render keeps
  scroll, focus and form input instead of flashing.
- **Round trip.** Any element carrying `data-action="name"` (and optional `data-*` payload) sends
  an `action` event to the script when clicked or submitted, and a form's values come with it. The
  whiteboard is therefore generative UI in both directions — the agent draws, the user answers by
  touching what was drawn, and the script decides what reaches the agent.
- **Real time, honestly.** Each `board_write` lands as soon as the tool call completes, so the
  board moves at the speed of the agent's tool calls. Finer than that — the page filling in while
  the model is still typing the HTML — needs the harness to stream a tool call's arguments. Ubiq
  already sees `ToolCall`/`ToolCallUpdate` in the `ConvUpdate` stream, so where a harness emits
  partial input, the host feeds the partial
  `html` argument to the board as it grows — the same trick MCP Apps calls
  `tool-input-partial`. Claude Code's native stream-json bridge (`D95`) can: with
  `--include-partial-messages` it emits `input_json_delta` events while the model writes a tool
  call's arguments, so the bridge needs to pass that flag and fold the deltas into a
  `ToolCallUpdate`. On harnesses that only report a finished call, the board updates per call.
- **History.** Every patch is appended to the instance's SQLite with a timestamp, so the board has
  undo, a scrubber, named snapshots, and **Export** to a standalone HTML file or a PNG (the same
  platform snapshot as §6.2, or `resvg` for an SVG-only board).
- **Pages.** A whiteboard is many live documents, not one: `ui.live` takes an id, and a tab strip
  surface lists them. An agent can open a new page for a new topic instead of overwriting.
- **Generated script.** `trust: "generated"` marks the page as written by a model. Its CSP stays
  default-deny for the network (`connect-src` is the bridge only, no remote images unless
  declared), and its bridge RPC is limited to `action` events — `space.call` is unavailable, so
  agent-written JavaScript can animate and compute inside the page but cannot reach a single
  `ubiq.*` capability or exfiltrate anything. Whether `Script{js}` is allowed at all is a manifest
  switch, `[web] generated_scripts = true`, off in the template.
- **Linux.** The live document works in the external-browser fallback unchanged, because the
  bridge's long-poll transport carries the same frames.

The whiteboard is a template space, not a built-in: about 150 lines of TypeScript plus the shell.
Its variants come cheap — a *slide deck* (one live page per slide, an agent narrating), a *design
review board* (an agent pins mock-ups, the user annotates with `data-action` stickies), a *teaching
canvas* (an agent explains a codebase with diagrams generated by `ubiq-archify`'s SVG export), a
*report* that rebuilds itself whenever a query's result changes.

### 6.2 The browser dock — automation, Playwright-style

A web dock is "a document, never a browser" (`inbox-web-panel`). A **browser dock** is the opposite,
deliberately: it navigates the open web, and the script or an agent drives it. It is a separate
panel kind with a separate permission, so the risk is never inherited by accident.

**The API** follows Playwright's shape so it reads familiar to authors and to agents:

| Call | Does |
|---|---|
| `browser.open({ profile?, backend?, hosts? })` | A browser handle and a panel descriptor for a dock |
| `page.goto(url)`, `back`, `forward`, `reload` | Navigate, resolving on load; refused outside the granted hosts |
| `page.click(sel)`, `fill(sel, text)`, `press(key)`, `select(sel, v)`, `hover(sel)` | Act on an element by CSS selector, text selector or snapshot ref |
| `page.waitFor(sel \| "load" \| "idle", { timeout })` | Wait on the page |
| `page.evaluate(fn, arg)` | Run a function in the page; the result comes back as JSON only |
| `page.snapshot()` | The accessibility tree with stable element refs — what an agent reads instead of pixels |
| `page.screenshot({ fullPage? })` | A PNG into the instance's data folder, and a handle a dock or an agent can show |
| `page.content()`, `url()`, `title()`, `cookies()` | Read state |
| `page.on("console" \| "navigate" \| "download" \| "dialog")` | Events |
| `page.route(pattern, handler)` | Intercept requests (CDP backend only) |

**Two backends, one API:**

- **`webview`** — the embedded `wry` view, visible and interactive in a dock, on macOS and Windows.
  Driving it is script injection: `with_initialization_script` installs a small automation agent
  in each page (query, scroll into view, synthesised events, the accessibility walk), and
  `evaluate_script_with_callback` carries each command and its answer. `load_url`, `go_back`,
  `with_navigation_handler`, `with_new_window_req_handler`, the download handlers, `cookies` and
  `with_permission_handler` cover the rest. Synthesised events are untrusted (`isTrusted` is
  false), which most sites accept and a few reject. `wry` has no screenshot call, so the screenshot
  is platform code: WebView2's DevTools-protocol channel (`CallDevToolsProtocolMethod`) on Windows,
  which also gives trusted input, and WKWebView's `takeSnapshot` on macOS. The webview runs in the
  interface process, so its commands travel host → UI as `SpaceBrowserOp` messages and back. With
  a remote UI the browsing happens on the UI's machine and its network, which the consent screen
  says.
- **`cdp`** — a Chromium the host launches (the user's installed Chrome or Chromium, never a
  download Ubiq performs silently) and drives over the DevTools protocol. It is Playwright's actual
  fidelity: trusted input, network interception, downloads, multiple tabs, headless by default. It
  runs on the host, so it works on Linux, with a remote UI, and with no window at all. To be watched,
  it streams `Page.startScreencast` frames into a dock that draws them as an image and forwards the
  user's clicks back, so the user can take over a session the agent got stuck in. Two Rust crates
  can carry it: `chromiumoxide` (async, tokio, pre-1.0) and `headless_chrome` (synchronous and
  thread-based, which matches the host's worker threads). Choosing one is a spike, not a decision
  this document makes.

**For agents**, the `ubiq-space` MCP server gains `browser_*` tools when the space grants them —
`browser_snapshot`, `browser_click`, `browser_type`, `browser_navigate`, `browser_screenshot`,
`browser_evaluate` — modelled on Playwright MCP's ref-based tools, so an agent that already knows
Playwright MCP needs no new vocabulary. The space's script can also wrap them in higher-level tools
("log in to staging", "collect the invoice PDFs") so the agent calls one intention instead of forty
clicks.

**Boundaries:**

- **Profile isolation.** Each instance gets its own browser profile under its data folder: a `wry`
  `WebContext` with its own data directory (macOS keys the store by an identifier instead, which
  the spike confirms), or a CDP `--user-data-dir`. It never sees the user's
  real browser cookies. A profile can be named in the manifest and shared by instances of the same
  space, so one login serves them all.
- **Hosts.** `[permissions.browser] hosts = ["localhost:*", "*.example.com"]`. `"*"` is allowed and
  flagged high risk. Navigation, redirects, frames and new windows outside the list are blocked and
  reported as a `navigate` event.
- **Page content is untrusted input.** Text an agent reads through `browser_snapshot` is marked as
  page content in the tool result, so a page that says "ignore your instructions" is data the
  privilege rule (§8.2) already contains: the agent can do nothing through the space that the grant
  does not allow, and every `ask` action still prompts the human.
- **Secrets.** `page.fill(sel, secrets.ref("password"))` attaches the value at the last hop — inside
  the host for `cdp`, inside the interface for `webview` — so the script never holds it. This needs
  `browser.fill_secrets`, high risk.
- **Downloads** land in `$data/downloads`; `file://` and `ubiq://` navigation is refused.

**The playground.** A *browser space* template opens a URL bar, the browser dock, a recorder (user
actions turned into TypeScript `page.*` lines in an editor dock), a test runner whose results are an
A2UI table, and an agent dock with the `browser_*` tools. That is a Playwright codegen and a test
runner and an agent-driven browser in one space, and it is ordinary space code on top of §6.2.

## 7. The `ubiq` API

A space imports `ubiq` (an ES module, typed by a generated `ubiq.d.ts`). Each namespace is a
**permission** (§9), and an ungranted namespace is absent, not merely refused. Every call is
asynchronous unless it is marked as synchronous below.

| Namespace | What it offers | Built on |
|---|---|---|
| `space` | Identity, instance id, mode (run/author), launch args, `onActivate`/`onDeactivate`/`onReload`, `close()` | New |
| `layout` | `open`, `close`, `focus`, `move`, `update(title, icon, badge)`, `get()`, `reset()`, `on("change")` | New `Space*` messages |
| `ui` | `surface` (A2UI), `web`, `live` (§6.1), dialogs, `toast`, `notify`, status items, `on("drop")` for files dragged onto a dock | A2UI, web tenant, notification family |
| `commands` / `menu` / `keys` | Palette commands, titlebar menu, dock context items, scoped keybindings | New descriptors |
| `state` | `get/set/delete/keys/clear`, synchronous over an in-memory copy, persisted as JSON. Scopes: `instance`, `space` (shared by instances) and `session` (memory only) | New, `<data>/state.json` |
| `sqlite` | `open(name)` returning `query`, `exec`, `tx` over a private SQLite file in the instance's data folder | `rusqlite` |
| `db` | List the project's saved connections, `query`, `page`, `cancel` | DB family, read-only guard kept |
| `fs` | `read`, `write`, `list`, `stat`, `watch`, scoped to granted roots (`$project`, `$data`, `$bundle` read-only, explicit paths) | File family, watch worker |
| `proc` | `exec(cmd, args)` with captured output, and `spawn` with streamed stdout/stderr lines, stdin and `kill`, for allowlisted programs only, with no pseudo-terminal | Host worker |
| `term` | `spawn(cmd?, cwd, env)` returning a pane; `write`, `onData` (raw bytes), `onExit`, `kill`; `snapshot()` for the visible screen as text, answered by the interface's emulator when a window holds the pane | Pane family |
| `http` | `fetch` with no CORS, to allowlisted hosts; `auth` takes a secret or connector reference | Host HTTP |
| `net` | WebSocket and server-sent-event clients to allowlisted hosts, with the same `auth` references. No raw TCP or UDP | Host worker |
| `browser` | A navigable, scriptable browser dock with a Playwright-shaped API (§6.2) | `wry` webview, or a host-driven Chromium |
| `ai` | One-shot model calls — `complete`, `structured` (JSON-schema output), `embed` — through the user's configured providers, with no harness and no conversation | The assist family and `ai-provider` keys (`D86`) |
| `schedule` | `every(cron, fn)` and `at(time, fn)`, persisted, firing while the host runs and catching up once at start | New, in-host |
| `serve` | Register routes on the loopback origin (`/_space/<instance>/…`) or on a declared port, handled in JS | `tiny_http` |
| `secrets` | `set(name)` (prompting the user), `has`, `ref(name)` to pass to `http`/`proc`; `reveal` only under its own permission | Keychain namespace `space/<id>` |
| `connectors` | List the user's connections and `fetch` through one, with the token brokered | Connector family |
| `agents` | `list`, `start`, `attach`, `prompt`, `cancel`, `status`, `history`, `on("update")`, `answer(permission)` | Conversation family |
| `mcp` | `tool(name, schema, handler)` to expose a tool to agents; `connect(server)` as an MCP client | Host MCP server (`D102`) |
| `projects` / `git` / `tasks` / `kb` / `search` | Read access to the families that exist, plus writes each under its own permission | Existing families |
| `events` | Publish and subscribe between instances of the same space, or across spaces that both grant it | New, in-host |
| `clipboard` / `open` | Write the clipboard; open a URL or file externally after a confirm | Existing |
| `console` / `log` | Goes to the instance's log dock and the `space` log subsystem | Log sink |

The runtime adds standard globals that rquickjs does not: `setTimeout`, `setInterval`,
`queueMicrotask`, `TextEncoder`/`TextDecoder`, `URL`, `structuredClone`, and `fetch` as an alias for
`http.fetch`. There is no npm at runtime. A dependency is vendored into the bundle as an ES module,
or bundled at author time by whatever tool the author likes. Ubiq does not install one.

### 7.1 The state API

`state` is the "local storage" every space gets with no permission. It is a JSON document per
scope, written with a debounce and an atomic rename, capped in size (1 MiB by default), and it
survives reloads and restarts. It is the right place for UI state, such as the last selection or
filters. Anything bigger or queried belongs in `sqlite`. A space's state and SQLite files live in
its data folder (§10), which an uninstall offers to keep.

### 7.2 Secrets without material

The domain rule is that accounts carry references, never material, and the same holds for a space:

- `secrets.set("api-key")` raises a native prompt. The value goes to the keychain under
  `space/<id>/api-key` and never passes through the script.
- `secrets.ref("api-key")` is an opaque handle. `http.fetch(url, { auth: { bearer: ref } })` and
  `proc.exec(cmd, { env: { TOKEN: ref } })` attach the material host-side.
- A space that needs the plain value, for example to sign a request itself, asks for
  `secrets.reveal`. That permission is flagged high-risk in the consent screen.

Connector tokens are never revealable.

## 8. Agents, in both directions

### 8.1 The space drives an agent

`agents.start({ type, profile, project, tools })` composes and launches a harness through the
existing conversation family, with agent-manager deciding how. `agents.attach(id)` binds to a
conversation that already exists, with the user's consent. A handle offers:

- `prompt(text)` and `cancel()`;
- `status()`, reporting idle, working, waiting for permission or ended;
- `history()`, the transcript as typed blocks;
- `on("update", fn)`, the `ConvUpdate` stream;
- `chat()`, a panel descriptor for a chat dock;
- `answer(permission, choice)`, which needs `agents.permissions`.

A space can therefore run pipelines, fan work out to several agents, gate on their results, or put
a custom dashboard over a conversation. `history` needs the history-list message the conversation
family lacks (`G208`).

### 8.2 An agent drives the space

The host's loopback MCP server (`D102`) gains a **`ubiq-space` server**. It is attached
automatically to agents a space starts with `tools: ["space"]`, and the user can attach it to any
other agent from the instance's agent menu. The agent's identity in the URL decides which instance
it reaches. The server offers:

| Tool | Does |
|---|---|
| `space_info` | Manifest, mode, docks, the exported functions and events |
| `space_call` | Invoke a function the script exported with `mcp.export(name, fn)` |
| `space_emit` / `space_wait` | Send an event to the script; block until the script emits one back |
| `space_state` | Read the instance's state (write needs the script's consent flag) |
| `space_render` | Draw an A2UI surface into a named dock, which is the dock-hosted `render_ui` |
| *custom tools* | Every `mcp.tool(...)` the script defined, with its own schema |

The privilege rule from the plugin proposal applies. An agent's call reaches no further than the
space's grant, and an action the grant marks `ask` prompts the user on the agent's behalf, so a
prompt-injected agent cannot use a space to escalate. The transcript shows each call as an ordinary
tool block.

### 8.3 The message channel

For anything that is not a tool call, `agents.on("message")` and the `space_emit` tool form a plain
event channel. An agent can report progress to a dashboard, and a dashboard can push a nudge into
the agent's queue through the steering work in `inbox/message-queue-and-steering.md`.

## 9. Permissions

The grant model is the plugin proposal's, made concrete for a space:

- **`Message::capability()`** classifies every message variant, and the match is exhaustive, so a
  new variant cannot be added without a classification. A space's host calls pass through it.
- **The manifest declares and the user grants.** `[permissions]` lists namespaces with their
  scopes:
  - `fs` roots;
  - `http` hosts;
  - `proc` programs;
  - `db` connection names or `"*"`;
  - `agents` types;
  - `serve` ports;
  - `secrets.reveal` as a flag.

  The grant is stored per space and, for a project-scoped space, per space per project.
- **Three levels per permission:** `allow`, `ask` (prompt on first use, remember for the instance
  or for always) and `deny`. A capability that is absent from the manifest is denied and cannot be
  granted without an edit, so a space cannot widen itself at runtime.
- **Risk is shown, not hidden.** The consent screen groups permissions by risk. Writes,
  `proc`, `term`, `secrets.reveal`, `serve` on a non-loopback interface and `agents.permissions` are
  high. An upgrade that asks for more shows the difference and asks again.
- **Audit.** Each granted call is a line in the `space` log subsystem, and the instance's log dock
  filters to it.

**The permissions panel** in Author mode is the editing side of the same model. It shows one
checkbox per namespace with its scope editor (path list, host list, program list), the level, and a
risk badge, and it writes `[permissions]` into `space.toml`. The consent screen at install or first
run is the same component, read-only on the manifest side and editable on the grant side.

## 10. Where things live

| What | Where |
|---|---|
| Installed bundles | `<config root>/spaces/<space-id>/<version>/`, unpacked and verified, read-only to the runtime |
| Dev spaces | Any folder, registered in `<config root>/spaces/dev.toml`. Often a project, so the IDE edits it |
| Instance registry | `<config root>/spaces/instances.toml`: open instances, owning window, mode, restored at start |
| Instance data | `<config root>/spaces-data/<space-id>/<instance-id>/`: `state.json`, `*.sqlite`, `files/` |
| Project-scoped instance data | `projects/<ulid>/spaces/<space-id>/<instance-id>/`, or `.ubiq/spaces/…` for a project-managed project |
| Space-wide shared data | `…/<space-id>/_shared/` |
| Grants | Host settings, keyed by space id (and project) |
| Secrets | Keychain, a seventh namespace: `space/<space-id>/<name>` |
| Dock arrangement | `ViewPrefs`, per instance, as an opaque `DockAreaState` |

The manifest decides **instancing**:

- `singleton`: one instance at a time, and opening it again focuses that instance;
- `multi`: each open creates a new instance;
- `per-project`: one instance per project, with `$project` bound.

It also decides **scope**: `global` instances see no project unless the user picks one, and
`project` instances live inside one. An instance outlives its window. Closing the dock area
detaches it, and a `background = true` space keeps running with no docks at all (§14).

## 11. Security, stated plainly

**QuickJS in-process is not a sandbox.** `D127` said so and it holds. The boundary is the API:

- The runtime has no ambient capability. There is no `std`/`os` module, no filesystem, no network
  and no process; every power is a host function that checks the grant before acting.
- Module loading is confined to the bundle folder. Paths are canonicalised, symlinks out of the
  bundle are refused, and there is no `eval` of remote code because there is no network the grant
  did not allow.
- Web docks get MCP Apps' model: a separate per-dock origin token, a default-deny CSP that only the
  manifest can widen, and no `ubiq.*` in the page.
- A2UI cannot fetch, navigate or perform (`D115`), and a function call in a payload is still only
  an intent, which the script decides whether to honour.
- Agent calls obey the privilege rule (§8.2).
- Resource exhaustion is bounded by the interrupt, the memory cap, the state and SQLite size caps,
  and a per-instance limit on open docks and terminals.

What this does not defend against is a malicious space that was granted `proc` or `fs` write. That
space can do what those grants say, the same as a VS Code extension can. The remedy is the consent
screen, signing (§13), and later an out-of-process runtime behind isol8's seam
(`inbox/isolation-proposal.md`), which the `SpaceHost` trait allows without touching any script.

## 12. Run and Author — one instance, two modes

**Run** is the space as its author intended. Only its own docks are visible, it reads its bundle
read-only, and it is reached from the Spaces rail, the palette, a pinned rail entry or a
`ubiq://space/<id>` link.

**Author** is the same live instance with an editing kit around it. A toggle (in the space strip,
and `⌘⇧E`) switches modes without restarting anything:

- **The run docks stay in the centre**, so the space is its own live preview.
- **Left: the bundle's file tree.** Opening a file adds an editor dock using the IDE's editor, with
  syntax highlighting for TS, JSON, HTML and CSS, and the A2UI JSON with a side-by-side surface
  preview.
- **Right: Settings and Permissions.**
  - Settings covers name, description, id, version, icon, author, licence, instancing, scope,
    background, pinned rail entry and the memory limit.
  - Permissions is §9's panel.

  Both are A2UI forms writing `space.toml`, and both validate as the user types.
- **Bottom: Console**, holding `console.*` output, errors with source-mapped TS positions, the audit
  log and a REPL evaluated inside the running instance.
- **An agent dock**, if the user wants one, with authoring tools attached:
  - read and write the bundle;
  - `space_reload`;
  - `space_console`;
  - `space_inspect`, which returns the layout, every A2UI surface's tree and data model, and a
    screenshot of a dock — the WebDriver idea from app-ly, in tool form.

  "Build me a space that tails these logs and flags errors" then becomes an ordinary conversation.

**Hot reload.** On save, the module graph is re-transpiled, the runtime is rebuilt, and `onReload`
receives the previous instance's `state`. Docks whose ids survive keep their place, and their
surfaces are re-rendered. Terminals and conversations the script started are kept and handed back
to the new runtime by id. A pseudo-terminal is not killed because its script changed. A transpile
or load error leaves the old runtime running and shows the error in the console.

**Editing an installed space** forks it. Author on an installed bundle offers **Edit a copy**, which
unpacks into a dev folder with a bumped version and keeps the original immutable and verified.

**New space** offers templates:

- blank;
- A2UI form;
- web panel;
- agent pipeline;
- database dashboard;
- terminal runbook.

Each template is a working space with a README, the `ubiq.d.ts` types and an `AGENTS.md` telling an
authoring agent the API and the rules.

## 13. Bundles, the manager and the registry

**A bundle** is a zip with a fixed shape:

```
my-space.ubiqspace
├─ space.toml          manifest: identity, instancing, permissions, contributions, csp
├─ src/main.ts         entry (the manifest names it)
├─ ui/*.a2ui.json      A2UI definitions
├─ web/…               html, css, js, vendored libraries
├─ assets/…            icons, data
├─ README.md
└─ SHA256SUMS          every file's hash, checked on install and on load
```

**Export** in Author mode validates the manifest, checks every import resolves inside the bundle,
writes `SHA256SUMS` and zips. **Import** comes from a file, a drag onto the window, or a URL. It
unpacks into a staging folder, verifies, shows the consent screen, and renames into place, the same
`.part`-then-rename discipline as `web_assets`.

**The manager** lives in application settings. It lists installed and dev spaces with version,
source, permissions granted, data size and running instances. Its actions are Open, Author, Export,
Update, Disable, Revoke grants, and Uninstall (keep or delete data).

**The registry**, later, is a static signed index (JSON over HTTPS) of bundles with publisher ids,
versions, declared permissions and hashes. Bundles are signed with minisign, which `ubiq-host`
already verifies with (`minisign-verify`), and the index pins each publisher's key. The workflow is
`ubiq space publish` and a browse-and-install page in the manager. Nothing in the bundle format
changes when the registry arrives, because a local bundle is already a registry entry without the
index.

## 14. How this relates to what is already proposed

| Proposal | Relation |
|---|---|
| `inbox-plugin-system` | Shares its grant model, broker and capability classification. **A plugin is a space with `background = true` and no docks.** Its commands, store, net and tools are a subset of §7. Building spaces builds its phases 0, 1, 3 and 4 |
| `inbox-a2ui` | Same renderer. `render_ui` (in a conversation) and `space_render` (in a dock) are siblings. This proposal pays for `G250`/`G206` |
| `inbox-web-panel` | The `space` tenant generalises a tenant from "a viewer for one file type" to "a bundle's page". The bridge gains the RPC channel |
| `inbox-editions` / `D177`–`D184` | Editions contribute compiled-in modes, and spaces contribute runtime ones. Both register through the same `Registry<Spec>` |
| `inbox-ui-id` | A space's docks and surfaces get `UiId`s under `ubiq.space.<id>.<dock>`, so help, personalisation and `space_inspect` can address them |
| Sink script scratchpad | Unchanged. Its rules (`D127`–`D129`) keep governing it. It may gain **Promote to space**, which writes its buffer as a new dev space |
| Drone | Instances run on the host, so a space opened on a remote host runs there and its docks draw here |

**Integrations it opens up:**

- **MCP Apps host:** connect to any MCP server, list its `ui://` tools and open them in web docks.
- **MCP client:** `mcp.connect` makes a space a broker between an agent and a third-party server.
- **Task sources:** a space contributes a task source to the Tasks mode.
- **KB views:** a space renders a knowledge-base source its own way.
- **Notifications and missions:** a space reacts to mission events or agent status changes.
- **Remote A2UI agents:** an A2A or HTTP agent that speaks A2UI natively can stream envelopes into
  an `a2ui` dock through a small adapter space.

## 15. Validation — two dozen spaces against the model

The model is only as good as the spaces it can hold. Each row below was designed against §4–§11 as
first written, and graded by what it needed:

- **Fits** — buildable on the model as it stood;
- **Fits after amendment** — it exposed a gap, and the amendment is now part of this proposal
  (the section that carries it is named);
- **Fits under a high-risk grant** — buildable, but only with a permission the consent screen flags;
- **Does not fit** — outside what a space should be, with the reason and the nearest workaround.

The second half of the list is deliberately unexpected: goals nobody would build a rail mode for.

| # | Space | Built from | Verdict | What it exposed |
|---|---|---|---|---|
| 1 | PR cockpit (§1) | `connectors`, A2UI list, web diff, `agents`, `term` | Fits | — |
| 2 | Database dashboard | `db`, chart in a web dock, `notify` | Fits after amendment | Refreshing every five minutes had no home: **`schedule`** (§7) |
| 3 | Incident runbook | A2UI checklist, `term`, `agents`, `sqlite` timeline | Fits after amendment | An agent cannot read a terminal's raw VT bytes: **`term.snapshot()`** returns screen text from the interface's emulator (§7) |
| 4 | Agent pipeline | `agents`, chat docks, A2UI board, `Usage` spend | Fits | Parallel implementers want a worktree each; `agents.start({ worktree })` follows `inbox/worktree-proposal.md` |
| 5 | Local dev cockpit | dev server, preview, log tail, `agents` | Fits after amendment | A dev server is not interactive: **`proc.spawn`** with streamed output (§7). A `web` dock cannot navigate to `localhost:3000`: **the browser dock** (§6.2) |
| 6 | API playground | `http`, `secrets.ref`, `sqlite`, code view | Fits | — |
| 7 | Release manager | `git`, `connectors`, `agents`, `ask` on publish | Fits | — |
| 8 | Personal kanban | `sqlite`, A2UI | Fits | Needs no permission at all, which is the point of `state` and `sqlite` being free |
| 9 | MCP App viewer | `mcp.connect`, web dock in MCP Apps mode | Fits | — (phase 7) |
| 10 | **Magic whiteboard** | live document, `mcp.tool`, `agents` | Fits after amendment | A web page the runtime writes rather than the bundle: **live documents** (§6.1) |
| 11 | **Browser QA / Playwright playground** | browser dock, editor, A2UI results, `agents` | Fits after amendment | Navigation and automation: **the browser dock** (§6.2) |
| 12 | Tabletop game master | live map, A2UI dice and sheets, `sqlite` campaign, an agent as GM | Fits | Walkthrough in §15.1 |
| 13 | Price watcher and form filler | headless `cdp` browser, `schedule`, `notify`, `sqlite` | Fits after amendment | `schedule` and the `cdp` backend. It runs only while the host runs (row 22) |
| 14 | Home dashboard (Home Assistant, MQTT) | `http`, A2UI tiles, `notify` | Fits after amendment | Home Assistant pushes over a WebSocket: **`net`** (§7). MQTT over raw TCP does not fit; MQTT over WebSocket does |
| 15 | Expense importer | dropped CSV or PDF, extraction, `sqlite`, chart | Fits after amendment | Files dropped on a dock: **`ui.on("drop")`**. Turning a receipt into rows without a conversation: **`ai.structured`** (§7). PDF text through `proc` (`pdftotext`) or pdf.js in a web dock |
| 16 | Spoken-language tutor | web dock with microphone, `ai`, live transcript | Fits after amendment | Media access in a page: **`[web] permissions = ["microphone", "camera"]`**, MCP Apps' list, answered by `wry`'s permission handler and shown at consent |
| 17 | Data notebook / spreadsheet | vendored grid in a web dock, `sqlite`, `ai` | Fits | QuickJS has no JIT. Heavy computation belongs in the web dock (a JIT engine and Web Workers) or in `proc`, and the script orchestrates. The template says so |
| 18 | Slides from a repository | live pages, an agent narrating, full-screen | Fits after amendment | Full-screen or second-monitor docks need `layout.detach`, which waits on `inbox/detached-panes-proposal.md` |
| 19 | Phone remote / second screen | `serve` on the LAN, a live page for the phone | Fits under a high-risk grant | `serve` beyond loopback is an open question in §18. With a per-session token and a QR code it is reasonable; without the grant it does not run |
| 20 | Arduino / serial dashboard | serial port readings, charts | Fits under a high-risk grant | No serial API, by design. A `proc.spawn` of an allowlisted reader bridges it |
| 21 | Real-time 2D game | canvas in a web dock, `state` high scores | Fits | A2UI cannot draw it, and need not: the webview runs at frame rate |
| 22 | Nightly report while Ubiq is closed | `db`, `ai`, email through a connector | Fits after amendment | A space lives in the host process. The OS scheduler runs **`ubiq space run <id> --entry nightly`** headless, so a manifest may name extra entry points (`[entry] main`, `[entry.jobs] nightly`) |
| 23 | Team standup board shared live by colleagues | shared state, presence | Does not fit | Multi-user real-time sync is not a space's job. The workaround is a shared store the user already has: a Postgres through `db`, or a git repository through `git` |
| 24 | Clipboard history / global launcher | OS-wide hotkey, clipboard watching, tray | Does not fit | Hooks outside Ubiq's window belong to the OS. A space reaches the clipboard only on a user action |

### 15.1 Walkthrough — the tabletop game master

The goal is far from coding, which is why it is a good test. The user runs a role-playing campaign
with an agent as game master.

- **Docks:** a live document in the centre for the scene (map, illustrations as inline SVG, the
  current room's description); an A2UI party sheet on the right; an A2UI dice tray and initiative
  tracker at the bottom; the GM agent's chat on the left.
- **State:** the campaign in `sqlite` (characters, inventory, locations, session log), the current
  scene id in `state`.
- **Agent link:** the script defines `mcp.tool("roll", …)`, `mcp.tool("set_scene", …)` and
  `mcp.tool("update_character", …)`. The agent narrates in chat and calls those tools, and each
  call updates the board and the sheets. A player's click on a door in the map
  (`data-action="open"`) reaches the agent as an event, and the dice tray's result reaches it as a
  tool result, so the agent never invents a roll.
- **Permissions:** `agents` and `mcp`. No files, no network, no processes.

It fits with nothing added beyond §6.1, and it shows the two-way agent link (§8) doing real work:
the script is the rulebook and the agent is the storyteller, and neither can cheat the other.

### 15.2 Walkthrough — the price watcher

The user wants a set of product pages checked twice a day and a notification when a price drops.

- `schedule.every("0 9,18 * * *", check)` wakes the instance. `browser.open({ backend: "cdp" })`
  runs headless, so no window has to be open.
- `check` visits each URL kept in `sqlite`, and reads the price with `page.evaluate`. For pages
  that change their markup it asks `ai.structured` to extract `{ price, currency, in_stock }` from
  `page.snapshot()` instead.
- A drop raises `notify`. The A2UI list shows the history, and a chart in a web dock shows the
  trend.
- **Permissions:** `browser` with the shop hosts only, `ai` and `schedule`. The consent screen
  shows that the browser cannot leave those hosts.

It exposed three amendments (`schedule`, the `cdp` backend, `ai`) and one honest limit: it runs
while the host runs. Row 22's headless entry point is the answer for a user who wants it from cron.

### 15.3 Walkthrough — the home dashboard

The user has Home Assistant at home and wants lights, sensors and a "leaving home" routine beside
their agents.

- `net.ws("wss://ha.local/api/websocket", { auth: { bearer: secrets.ref("ha-token") } })`
  subscribes to state changes. The token never reaches the script.
- Live tiles sit in an A2UI grid, and a toggle sends a service call over the same socket.
- A "leaving home" command checks every sensor and asks an agent to summarise what is still on.
- **Permissions:** `net` with one host, `secrets` and `agents`.

Before `net` existed this did not fit, because a five-second `http` poll was the only option. MQTT
on its raw port still does not fit, and the template points at MQTT over WebSocket instead.

### 15.4 What the validation changed, and what it confirmed

**Amendments now in the proposal:** live documents (§6.1), the browser dock (§6.2), `proc.spawn`,
`term.snapshot`, `net`, `ai`, `schedule`, dock drop events, media permissions for web docks, extra
headless entry points, and `layout.detach` as a dependency on the detached-panes proposal.

**Confirmed by the exercise:**

- **The docks reuse what exists.** The live document and the browser dock are the only new panel
  kinds, and they carry eight rows between them (5, 10, 11, 12, 13, 16, 18, 19). The rest are built
  from terminals, chats, editors, A2UI and web docks.
- **The grant model scales down.** The kanban, the game and the whiteboard need no file, network or
  process access, and those are exactly the spaces most likely to be shared.
- **The agent link is the differentiator.** Rows 3, 4, 10, 11, 12 and 15 depend on the script
  defining tools for an agent, or on the agent reading the space's state.
- **QuickJS is the conductor, not the orchestra.** Wherever the work is heavy — rendering, charts,
  a grid, a game loop, PDF parsing — it moves to a web dock or a process, and the runtime only
  coordinates.

**Where the model stops, on purpose:** multi-user real-time collaboration, OS-wide automation,
raw sockets and devices, and running while Ubiq itself is not. Each has a workaround through a
user-held store, `proc` or the headless CLI, and none of them is a reason to widen the runtime.

## 16. Wire changes

A new **space family** in `crates/ubiq-proto/src/messages.rs`. Its ids are `SpaceId` (the
manifest's reverse-DNS string), `SpaceInstanceId` (ULID) and `DockId` (a script-chosen string,
unique within an instance). Instance messages route to the window that owns the instance, the way
pane messages do, and catalogue messages reach every window.

| Direction | Variants |
|---|---|
| UI → host | `ListSpaces`, `InstallSpace`, `UninstallSpace`, `ExportSpace`, `OpenSpace{space, project?, args}`, `CloseSpace`, `DetachSpace`, `SetSpaceMode`, `ReloadSpace`, `SpaceAction{instance, dock, envelope}`, `SpaceInput{instance, dock, pointer, value}`, `SpaceWebCall{instance, dock, call}`, `SpaceLayoutChanged`, `SpaceCommand`, `SpaceDialogAnswer`, `SaveSpaceManifest`, `SetSpaceGrants`, `SpaceEval` (author REPL), `SpaceDrop{instance, dock, files}`, `SpaceBrowserResult{instance, dock, op, result}`, `PaneSnapshot{pane, text}` |
| Host → UI | `SpacesListed`, `SpaceOpened{instance, manifest, layout}`, `SpaceLayout{instance, ops}`, `SpaceSurface{instance, dock, envelope}`, `SpaceWebEmit`, `SpaceLive{instance, dock, patch}`, `SpaceBrowserOp{instance, dock, op}` (webview backend only), `SpaceScreencast{instance, dock, frame}` (cdp backend), `RequestPaneSnapshot{pane}`, `SpaceContributions` (commands, menus, status items, keys), `SpaceDialog`, `SpaceConsole`, `SpaceConsent`, `SpaceClosed{reason}`, `SpaceError` |

Terminal, chat, editor and database docks need no new messages: the host spawns them through their
own families and tells the interface only the descriptor that names the resulting id. Every new
variant owes a `capability()` classification.

## 17. Phases

| Phase | What lands | Rough cost |
|---|---|---|
| 0 | Decisions (§18). `crates/ubiq-space` with manifest, module loader, oxc moved in, event loop, timers, `state`. `ubiq space run <folder>` headless with `console` and `state` only | 1–2 weeks |
| 1 | `Message::capability()` and the grant store. Host instance worker. Space wire family. Spaces rail mode, `PanelKind::Space`, `layout`. A2UI keyed and incremental (`G250`/`G206`) with actions and two-way input. Commands and menus. Run mode for dev folders | 3–4 weeks |
| 2 | Reused docks: `term`, `agents` with a chat dock, `fs`, `editor`, `db`. Then `sqlite`, `proc`, `http`, `secrets` with brokering, `connectors`. Consent screen and audit | 3 weeks |
| 3 | Author mode: file tree, settings and permissions panels, console with REPL, hot reload, `ubiq.d.ts`, templates. `ubiq-space` MCP server with `space_*` and custom tools, `space_inspect` | 2–3 weeks |
| 4 | Web docks: the `space` tenant, manifest CSP and media permissions, bridge RPC. Live documents and the whiteboard template, with partial tool input from the stream-json bridge. `serve`, `events`, `net`, `ai`, `schedule`, `proc.spawn`, `term.snapshot`, drop events | 2–3 weeks |
| 5 | The browser dock: the Playwright-shaped API, the `webview` backend on macOS and Windows, then the `cdp` backend after a crate spike. `browser_*` MCP tools. The browser playground template | 3 weeks |
| 6 | Bundles: export, import, verify, manager, fork-to-edit, upgrade consent. Background spaces (the plugin case). Headless entry points | 1–2 weeks |
| 7 | MCP Apps hosting and `mcp.connect` | 1–2 weeks |
| 8 | Registry: signed index, publish, browse | Not proposed yet |
| 9 | Out-of-process runtime behind isol8 | Not proposed yet |

Phases 0 and 1 already deliver something useful: a scripted A2UI tool with persistent state, with
no power beyond its own docks. Phase 2 makes it a workbench, and phase 3 makes it self-building.

## 18. What this asks to be decided

Each of these would become a row in `tech/decisions.md` when accepted:

1. **The name.** *Space* (proposed), or *app*, *bench* or *studio*. The glossary entry follows the
   choice.
2. **The runtime runs on the host,** in a new crate behind a `SpaceHost` trait, with one persistent
   QuickJS runtime per instance on its own thread. The scratchpad stays an interface-side fixture.
3. **A space is a granted bus client.** This adopts the plugin proposal's `D97`/`D99` for spaces.
   `Message::capability()` becomes mandatory for every variant.
4. **Plugins are background spaces.** One model, one manifest and one consent screen, rather than
   two systems.
5. **A2UI is the native panel and HTML is the escape hatch.** This amends the plugin proposal's
   `D100` ("never HTML") for spaces. A web dock is allowed because it is CSP-confined and gets no
   `ubiq.*`, and A2UI stays the default.
6. **The script owns the logical layout and the user's arrangement wins on restore.**
7. **Secrets are references.** `reveal` is a separate high-risk permission, and connector tokens are
   never revealed.
8. **Author mode is the same instance.** Hot reload keeps state, terminals and conversations, and an
   installed bundle is immutable and forked to edit.
9. **The bundle format** is a zip with `space.toml` and `SHA256SUMS`, minisign-signed when it comes
   from a registry.
10. **A live document may hold model-written HTML and, by manifest switch, script.** It is safe
    because its page has no network beyond the bridge and its bridge carries only `action` events.
11. **A browser dock is a separate panel kind with a separate permission.** It may navigate, unlike
    a web dock. Every instance gets an isolated profile, navigation is confined to granted hosts,
    and page content reaches an agent marked as untrusted.

What it leaves open, for `backlog.md` once accepted:

- whether a grant is per space or per space per project for global spaces;
- whether `events` may cross spaces;
- whether `serve` may bind beyond loopback at all;
- how a remote UI shows a `web` dock when the host is on another machine (the origin is the
  interface's, the bundle is the host's);
- whether `agents.answer` should exist, or whether a permission prompt should always reach a human;
- how much of `ubiq.d.ts` is generated from the capability table rather than written by hand;
- which crate drives the `cdp` backend, and whether Linux gets a `webview` backend when
  `gpui-wry`'s Unix path lands;
- whether `term.snapshot` should work for a pane no window is drawing, which would need a terminal
  state engine on the host that the domain rules currently exclude.

## Related docs

- [`inbox/plugin-system-proposal.md`](./plugin-system-proposal.md) — the grant model this builds on
- [`inbox/a2ui-proposal.md`](./a2ui-proposal.md) — the conversation-side sibling of the A2UI docks
- [`inbox/web-panel-proposal.md`](./web-panel-proposal.md) — the origin, bridge and webview reused here
- [`references/a2ui-protocol.md`](../references/a2ui-protocol.md) — the envelopes §5 completes
- [`features/workbench-sink.md`](../features/workbench-sink.md) — the scratchpad and A2UI page this grows from
- [`features/workbench.md`](../features/workbench.md) — the rail, the dock and the modes
- [`tech/transport-contract.md`](../tech/transport-contract.md) — where the space family would be written
