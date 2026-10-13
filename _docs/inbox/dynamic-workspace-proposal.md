---
id: inbox-dynamic-workspace
title: Proposal — dynamic workspaces, a scriptable workbench
kind: proposal
status: proposal
summary: A new rail mode whose centre is a dockable workbench built and driven by a TypeScript program on a persistent, host-side QuickJS runtime — docks holding A2UI surfaces, web panels, terminals, chats, editors or database tabs; a capability-granted `ubiq.*` API over layout, menus, state, a local SQLite, files, processes, terminals, HTTP, a loopback server, database connections, secrets, connectors and agents; a two-way agent link through the host's MCP server; Run and Author modes on one live instance; a `space.toml` manifest with a settings and permissions panel; zip bundles, a manager, and a signed registry later.
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

## 7. The `ubiq` API

A space imports `ubiq` (an ES module, typed by a generated `ubiq.d.ts`). Each namespace is a
**permission** (§9), and an ungranted namespace is absent, not merely refused. Every call is
asynchronous unless it is marked as synchronous below.

| Namespace | What it offers | Built on |
|---|---|---|
| `space` | Identity, instance id, mode (run/author), launch args, `onActivate`/`onDeactivate`/`onReload`, `close()` | New |
| `layout` | `open`, `close`, `focus`, `move`, `update(title, icon, badge)`, `get()`, `reset()`, `on("change")` | New `Space*` messages |
| `ui` | `surface` (A2UI), `web`, dialogs, `toast`, `notify`, status items | A2UI, web tenant, notification family |
| `commands` / `menu` / `keys` | Palette commands, titlebar menu, dock context items, scoped keybindings | New descriptors |
| `state` | `get/set/delete/keys/clear`, synchronous over an in-memory copy, persisted as JSON. Scopes: `instance`, `space` (shared by instances) and `session` (memory only) | New, `<data>/state.json` |
| `sqlite` | `open(name)` returning `query`, `exec`, `tx` over a private SQLite file in the instance's data folder | `rusqlite` |
| `db` | List the project's saved connections, `query`, `page`, `cancel` | DB family, read-only guard kept |
| `fs` | `read`, `write`, `list`, `stat`, `watch`, scoped to granted roots (`$project`, `$data`, `$bundle` read-only, explicit paths) | File family, watch worker |
| `proc` | `exec(cmd, args)` with captured output, for allowlisted programs only | Host worker |
| `term` | `spawn(cmd?, cwd, env)` returning a pane; `write`, `onData` (raw bytes), `onExit`, `kill` | Pane family |
| `http` | `fetch` with no CORS, to allowlisted hosts; `auth` takes a secret or connector reference | Host HTTP |
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

## 15. Possibilities — a catalogue of first spaces

These are illustrations, each buildable on §7 alone:

- **PR cockpit** (§1): connector, A2UI list, web diff, a reviewing agent and a terminal.
- **Database dashboard:** saved connection, scheduled queries, chart web dock, alerts as
  notifications.
- **Incident runbook:** an A2UI checklist whose steps open terminals with prepared commands, an
  agent that reads the output, and a timeline in SQLite.
- **Agent pipeline:** planner, then N implementers, then a reviewer. Each is a chat dock, with a
  board surface showing the stage of each agent, retry and stop buttons, and spend from `Usage`.
- **Local dev cockpit:** `term` running the dev server, a `web` dock previewing it, a log tail
  through `fs.watch`, and an agent with `space_inspect` for "why is this page blank".
- **API playground:** requests in SQLite, secrets by reference, responses in a code view.
- **Release manager:** git tags, changelog draft by an agent, CI status through a connector, and a
  publish button behind `ask`.
- **Personal kanban or notes:** SQLite and A2UI, with no permissions at all.
- **MCP App viewer:** open any MCP server's UI resources side by side with the agent using them.

## 16. Wire changes

A new **space family** in `crates/ubiq-proto/src/messages.rs`. Its ids are `SpaceId` (the
manifest's reverse-DNS string), `SpaceInstanceId` (ULID) and `DockId` (a script-chosen string,
unique within an instance). Instance messages route to the window that owns the instance, the way
pane messages do, and catalogue messages reach every window.

| Direction | Variants |
|---|---|
| UI → host | `ListSpaces`, `InstallSpace`, `UninstallSpace`, `ExportSpace`, `OpenSpace{space, project?, args}`, `CloseSpace`, `DetachSpace`, `SetSpaceMode`, `ReloadSpace`, `SpaceAction{instance, dock, envelope}`, `SpaceInput{instance, dock, pointer, value}`, `SpaceWebCall{instance, dock, call}`, `SpaceLayoutChanged`, `SpaceCommand`, `SpaceDialogAnswer`, `SaveSpaceManifest`, `SetSpaceGrants`, `SpaceEval` (author REPL) |
| Host → UI | `SpacesListed`, `SpaceOpened{instance, manifest, layout}`, `SpaceLayout{instance, ops}`, `SpaceSurface{instance, dock, envelope}`, `SpaceWebEmit`, `SpaceContributions` (commands, menus, status items, keys), `SpaceDialog`, `SpaceConsole`, `SpaceConsent`, `SpaceClosed{reason}`, `SpaceError` |

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
| 4 | Web docks: the `space` tenant, manifest CSP, bridge RPC. `serve`. `events` | 2 weeks |
| 5 | Bundles: export, import, verify, manager, fork-to-edit, upgrade consent. Background spaces (the plugin case) | 1–2 weeks |
| 6 | MCP Apps hosting and `mcp.connect` | 1–2 weeks |
| 7 | Registry: signed index, publish, browse | Not proposed yet |
| 8 | Out-of-process runtime behind isol8 | Not proposed yet |

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

What it leaves open, for `backlog.md` once accepted:

- whether a grant is per space or per space per project for global spaces;
- whether `events` may cross spaces;
- whether `serve` may bind beyond loopback at all;
- how a remote UI shows a `web` dock when the host is on another machine (the origin is the
  interface's, the bundle is the host's);
- whether `agents.answer` should exist, or whether a permission prompt should always reach a human;
- how much of `ubiq.d.ts` is generated from the capability table rather than written by hand.

## Related docs

- [`inbox/plugin-system-proposal.md`](./plugin-system-proposal.md) — the grant model this builds on
- [`inbox/a2ui-proposal.md`](./a2ui-proposal.md) — the conversation-side sibling of the A2UI docks
- [`inbox/web-panel-proposal.md`](./web-panel-proposal.md) — the origin, bridge and webview reused here
- [`references/a2ui-protocol.md`](../references/a2ui-protocol.md) — the envelopes §5 completes
- [`features/workbench-sink.md`](../features/workbench-sink.md) — the scratchpad and A2UI page this grows from
- [`features/workbench.md`](../features/workbench.md) — the rail, the dock and the modes
- [`tech/transport-contract.md`](../tech/transport-contract.md) — where the space family would be written
