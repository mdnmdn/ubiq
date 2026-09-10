---
id: inbox-plugins
title: Proposal — plugins, and the broker that is the whole of it
kind: proposal
status: proposal
summary: A plugin is a third client on the bus with the message set filtered by a grant — an embedded QuickJS runtime with no ambient authority, a manifest that declares which families, paths, hosts and commands it needs, a broker that checks every call against an exhaustive classification of the message enum, tools injected into agents through the in-process MCP server the harness library already ships, and a UI surface that is declarative contributions plus the A2UI renderer that is already built and has no producer.
read_when: you are deciding whether third-party code may run inside Ubiq, what it may reach, how it asks for more, how it reaches the agents, or how it draws anything
updated: 2026-09-10
depends_on: [tech-architecture, tech-transport, tech-agent-manager, tech-decisions, inbox-a2ui, inbox-isolation, inbox-config, inbox-web-panel, ref-a2ui-catalog]
---

# Proposal — plugins, and the broker that is the whole of it

A plugin system is usually two hard problems: inventing an API wide enough to be useful, and
building a sandbox narrow enough to be safe. **Ubiq has already solved the first one and does not
know it.** The message set in `crates/ubiq-proto/src/messages.rs` is not a transport detail — it is
the complete, closed, serialisable statement of everything the application can do, written down in
one enum because the two halves are not allowed to talk any other way.

So this proposes no plugin API.

> **A plugin is a third client on the bus, with the message set filtered by a grant.**

Everything else in this document follows from that sentence: the manifest is the filter, the broker
is the thing that applies it, the runtime is whatever can be given no authority of its own, and the
UI is whatever a plugin can describe rather than draw.

## 1. Where it stands

**The capability surface exists and is already the right shape.** Eighteen families, one
`#[serde(tag = "type", content = "payload")]` enum, no ad-hoc side channels, and two properties the
transport contract calls load-bearing: serialisable by construction, and inspectable — "a log of the
bus is a complete account of what happened". `crates/ubiq-proto/src/bus.rs` already records every
message in both directions onto a `Tape`. An audit trail for plugin activity is not a feature to
build; it is a filter over one that runs today.

**The families cover the ask almost exactly.** Project files are `ReadProjectFile`,
`WriteProjectFile`, `EditProjectPath`, `ProjectTree`, `DiffProjectFile`. Tasks are the whole work
family — `CreateTask`, `UpdateTask`, `MoveTask`, `AssignTask`, `AddStep`, `ToggleStep` and the rest,
durable in a per-project `tasks.toml`. Agents are the conversation family. Search, git, notification
and project are all there. Three things a plugin needs have no family and are named in §2.

**Sandboxing does not extend to embedded code, and is macOS-only where it does apply.**
`crates/agent-manager/src/isolate.rs` confines a *spawned child process*, and `confined_launch`
shells out to `/usr/bin/sandbox-exec`; on Linux and Windows it returns a hard error, because isol8's
Landlock backend applies itself inside the target between `fork` and `exec` and has no form a caller
that already owns the pseudo-terminal can use. It also refuses to nest. Two consequences decide this
proposal: **an interpreter running inside the host process gets nothing from isol8**, so its own lack
of authority has to be the boundary; and an out-of-process plugin sandboxed by the operating system
would work on one of the three platforms Ubiq targets.

**Reaching an agent is already built.** `crates/agent-manager/src/spec.rs` carries `McpRef::InProcess`
over an `Arc<dyn McpService>` — `tools()` and `call()` — which `provision()` turns into a loopback
MCP endpoint and hands to the harness as an ordinary entry. Every harness's config writer then does
its own thing with it: `--mcp-config` and `--strict-mcp-config` for Claude Code, a marker-delimited
`[mcp_servers.*]` block for Codex, `mcp-config.json` for Copilot. A plugin that wants to give an
agent a tool needs no server, no port of its own and no subprocess.

**The renderer for dynamic UI is built and has no producer.** `crates/ubiq/src/ui/a2ui.rs` draws all
eighteen components of the A2UI basic catalog out of the kit, on the kitchen sink's own page, and
`G206` records that nothing produces one. Every input is inert: no data model, no action dispatch,
no surface lifetime.

**Nothing scriptable exists.** No lua, wasm, quickjs, deno or extism anywhere in the workspace, and
every extension axis today is a closed Rust enum matched exhaustively at several sites —
`PanelKind`, `RailMode`, `ToolContent`, `SettingsSection`. That is a feature of the current tree,
not a defect, and §6 keeps it.

## 2. The shape

```
   plugin.js  ─ ubiq.* ─►  broker  ─ Message ─►  Hub  ─►  Coordinator
   (QuickJS,               (grants,             (a ClientId
    no globals)             budgets,             like a window's)
                            audit)
        ◄─ result ─────────────◄── reply routed to the plugin, never broadcast
```

Four rules, and they are the whole security model.

**R1 — A plugin sends messages; it does not call functions.** `ubiq.work.createTask(…)` becomes
`Message::CreateTask` and goes through `Coordinator::dispatch` exactly as a window's would. There is
no second code path, so there is no second thing to audit, and a capability the UI does not have is
not one a plugin can be given by accident.

**R2 — The grant is checked against an exhaustive classification of the enum.** In `ubiq-proto`,
beside `Message::project_id()`:

```rust
pub enum Access { Read, Write, Never }
pub struct Capability { pub family: Family, pub access: Access }
impl Message { pub fn capability(&self) -> Capability { /* exhaustive match */ } }
```

The match is what makes this hold over time. **A new message cannot be added without someone saying
what it costs a plugin** — the compiler asks — and `Access::Never` is a real answer, which is what
`BrowseHostDir`, `TerminalInput`, every account and connector mutation, and `SetSettings` get. The
transport contract's *Adding a variant* procedure gains one step: classify it.

**R3 — The plugin and the interface never touch.** A plugin contributes *data* that the interface
draws; it never gets an element, an entity or a frame. This is `D3` applied a third time, and it is
enforced the same way: `crates/ubiq-plugin` depends on `ubiq-proto` and nothing else, so the broker
*cannot* express anything but a message, and `just plugin` checks that nothing reintroduced it.

**Which half runs it is not a preference.** `crates/ubiq-plugin` is embedded by the host and runs on
a plugin worker thread beside the other five, because everything a plugin reaches is on that side —
files, tasks, git, conversations, the config root, the outbound HTTP client, the ability to spawn a
process. A plugin in the interface crate would have to break R2's own rule to reach any of it, and
four smaller things decide it independently: there is one host and N windows, so a plugin drawn into
the interface would sync twice and register its tools twice; a background activation has to tick
with no window focused; `McpService` is provisioned where the `RunSpec` is composed, so a tool
implemented in the interface could not be called at all; and the day the host is on another machine,
a plugin that ran beside the window would be looking for a project that is not there. Host-side, a
remote host costs the plugin nothing.

The interface's job is what it always is: draw the contributions, send the interactions back. The
price is that a plugin cannot react within a frame — a click on its surface round-trips over the bus
— which is what every panel already pays.

**R4 — A plugin holds no credential material and no path outside its grant.** It says
`auth: "connector:azure-devops"` and the broker attaches the token, the same way an account crosses
the wire as an id and a label. The domain rule already reads "accounts carry credential references,
never credential material"; a plugin is the first thing that would have tested it.

**Three capabilities have no family, and they are the new surface.** Everything else in this
document reuses something.

| Capability | Why it is new | What it is |
|---|---|---|
| `net` | Ubiq makes outbound requests for connectors and AI providers, never on a document's say-so | A `fetch` brokered through `ureq` with the host allowlist checked **after** DNS resolution, redirects re-checked, no raw sockets, no listening |
| `exec` | `SpawnWorkspace` runs a program *in a visible pane*, which is not what a background sync wants | A child process, `argv[0]` against the allowlist, never a shell, cwd inside a path grant, confined by isol8 where isol8 works |
| `store` | Preferences are the interface's opaque blob and settings are the host's; neither is a plugin's | A per-plugin key-value file under the config root, size-capped |

Why filter the existing set rather than write a plugin API: there is no second surface to keep in
sync, no second thing to review, adding a capability is a row in a table rather than new code, and
the day a plugin should run out-of-process the transport is already framed for a socket and already
carries a remote host.

## 3. The manifest, and what it is not

```toml
id = "azure-devops-sync"
entry = "plugin.js"
api = 1
activation = ["project.open", "command:azdo.sync", "work.task-changed"]

[grants.ubiq]                 # families, by access
work = "rw"
project = "r"
notification = "w"

[grants.files]
project = "r"                 # the open project's tree
paths = ["docs/**"]           # writable subtree
ask = ["**"]                  # anything else prompts once, and is remembered

[grants.net]
hosts = ["dev.azure.com", "*.dev.azure.com"]

[grants.secrets]
connectors = ["azure-devops"]
```

A `[budget]` table sits beside them — `memory`, `cpu` per call, `calls` per minute — and a
`[[contributes]]` array carries §6's declarations.

Three grant states and no fourth: **granted**, **ask**, and absent, which means no. Deny by default
everywhere — an unlisted family, host, path or command does not exist for that plugin. A grant is
consented to at install, shown as a whole rather than a scrolling list, and **an upgrade that widens
the manifest re-consents**, because the interesting attack on every plugin ecosystem that has ever
shipped is version 1.4 of something already trusted.

**The manifest is a declaration, not the enforcement.** It is resolved once at load into a grant
set, and the broker checks every call against that set at the moment of the call. Four checks are
worth writing down because they are the ones that get skipped:

- **Paths are canonicalised** before comparison, and a symlink out of the grant is out of the grant.
- **Hosts are checked against the resolved address**, and every redirect is re-checked. A wildcard
  covers one label, never a suffix match that `dev.azure.com.evil.test` satisfies.
- **`argv[0]` is the whole of an exec grant.** No shell, ever — not `sh -c`, not a string parsed
  into words.
- **Budgets are grants.** Memory, CPU per call, wall clock per call, calls per minute, bytes
  fetched. A plugin that exceeds one is stopped and reported, not throttled silently.

## 4. The runtime

**QuickJS, embedded, through `rquickjs`.** One runtime per plugin, all of them on a plugin worker
thread shaped like the four the host already runs — one thread, an unbounded queue, ends when the
coordinator drops the handle. `AsyncRuntime` and `AsyncContext` mean a host capability is a Rust
future the script awaits, which is what every call in §2 is.

It is chosen for one property above the others: **it has no ambient authority to remove.** A bare
QuickJS context has ECMAScript builtins and nothing else — no `fetch`, no `require`, no `process`,
no file system, no sockets, no threads. The `ubiq.*` object the broker installs is the entire reach
of the plugin, which means the sandbox is not a fence around a capable runtime but the absence of
any capability the broker did not hand over. That is the only posture that survives §1's finding
that an in-process interpreter gets nothing from the operating system. It also brings the two limits
the budget table needs: an interrupt handler for CPU, and `set_memory_limit` for the heap.

Authors write TypeScript against a `.d.ts` generated from the message set, and bundle to one ESM
file at package time. **npm is a build-time fact, never a runtime one** — a plugin that needs an
Azure DevOps client bundles it, and Ubiq ships no Node, downloads nothing, and resolves no modules.
A module resolver is itself ambient authority, which is the second reason not to have one. This is
what `D95` protects for the harness path, applied here.

What was weighed and why it lost:

| Alternative | Why not |
|---|---|
| Wasm components — `wasmtime` + WIT, as Zed does | The one option where deny-by-default is a property of the substrate rather than a configuration the host can get wrong, and the embed itself is cheap. The cost falls on authors: a component means Rust and a compile step, and getting *JavaScript* into one means `componentize-js`, which embeds a second JS engine to do it. It buys a structural guarantee for a boundary QuickJS already gives by having nothing on the other side |
| `deno_core` embedded, or a Deno/Node child process | The only candidates with real TypeScript and `npm:` resolution at runtime, which is the best author story on offer. Against it: V8's weight and build, an API that iterates faster than Ubiq wants to migrate, and — for the child-process form — a runtime Ubiq would have to find or ship, an IPC surface to design, and an OS sandbox that works on macOS only (§1). Reconsider when isol8 has a Linux seam |
| Lua via `mlua` | Cheapest to embed, and the one candidate that is **not** deny-by-default: stock Lua ships `os`, `io`, `debug` and a `package` that can `loadlib` native code, so the sandbox is "the host stripped the standard library correctly, and kept doing so across upgrades". The deciding factor is elsewhere: an integration plugin is mostly HTTP and JSON, the libraries for that are JavaScript, and so is what an agent writes best |
| `rhai`, `starlark-rust` | Turnkey-sandboxed, trivially cross-compiled, and neither can await a host capability — rhai's host functions are synchronous and starlark has no I/O by design. Fine languages with no ecosystem for the thing plugins are for |

How the applications this would be compared against answered the same question: **Zed** compiles
extensions to Wasm and wires them through WIT — real isolation, Rust-only authors. **VS Code** runs
an extension host process that is a crash boundary and explicitly not a security one, and leans on
marketplace review. **Neovim** and **Obsidian** run plugins with the editor's full authority, and
Obsidian's ecosystem has already carried a remote-access trojan. **Figma** moved plugins into
QuickJS after same-realm JavaScript isolation let sandboxed code forge host objects — a failure mode
Ubiq does not have, because the host is Rust and the guest is a different engine by construction.

**A plugin never blocks a frame.** Calls are async, the interface draws last-known contributions,
and a handler that takes two seconds delays a plugin's own work and nothing else. The coordinator's
own rule — its reader is never blocked by a slow UI — is the same rule pointed the other way.

## 5. Reaching the agents

Three forms, in order of what they cost.

**A plugin's tools become MCP tools, with Ubiq as the server.** `ubiq.tools.define({ name, schema,
handler })` registers into an `McpService` implementation the host already knows how to provision,
namespaced `plugin.<id>.<tool>` so provenance is legible in the transcript. No second process, no
port a plugin controls, no config path Ubiq names — the boundary in `tech/agent-manager.md` holds,
because what a run is composed of is still the library's answer and Ubiq is only handing it a
service. This is the same phase 0 the [A2UI proposal](./a2ui-proposal.md) needs, and building it
once serves both.

**A plugin may contribute instructions and skills**, which the library already injects, and
`crates/agent-manager/_docs/mcp-as-skill.md` is the latent form of the same thing: a one-line
description that costs no context until the agent reaches for it. Company-specific guidance riding a
plugin is exactly the case this shape was designed for.

**A plugin may declare an external MCP server it does not implement.** Cheap to support — it is one
more catalog entry — and it is the one contribution that leaves the sandbox entirely, so it gets its
own louder consent and its own line on the install screen. A plugin whose whole content is "run this
binary with my agents" is a plugin that is not sandboxed, and the user must be told in those words.

**The privilege rule, which is the part that is easy to get wrong:**

> A tool call from an agent runs with **the plugin's** grants, not the agent's.

That makes a plugin tool a privilege boundary an agent can call *across*, and two things follow. An
`ask` grant reached through a tool call becomes a permission prompt in the transcript, answered by
`AnswerPermission` like any other. And a plugin tool that writes anything is reachable by prompt
injection, so the consent screen names which tools a plugin exposes to agents and what those tools
can reach, as its own section rather than a line in a list.

## 6. Drawing something

The genuinely hard part, and the only one where the answer is a ladder rather than a rule. Four
tiers; the proposal is the first two, the third is conditional, and the fourth is refused.

**Tier 1 — declarative contributions, resolved at load.** Commands in the palette, a settings
schema, status-bar items, menu entries, an icon from the registry, notifications. Settings are the
interesting one: `crates/ubiq/src/ui/settings.rs` is four thousand hand-drawn lines over the row
furniture in `ui/kit/settings.rs`, and a schema renderer — a field list of typed rows drawn by the
existing builders — is new but small, and it pays for itself the first time a Ubiq setting is
described rather than written. This tier covers most of what plugins actually want.

**Tier 2 — A2UI surfaces, for anything dynamic.** A plugin emits a component tree; Ubiq draws it in
native widgets; the user's answer comes back as data. The renderer exists (§1). What is missing is
precisely what the A2UI proposal defers to its phases 3 and 4 — the data model behind a JSON Pointer
binding, the runtime id-to-`Entity<InputState>` map, action dispatch, and a surface that outlives one
call — and a plugin needs all four, so **this proposal owns that cost rather than inheriting it.**

The reason this is the right language and not merely the available one: the basic catalog carries
`justify`, `align` and per-child `weight`, and **no padding, no margin, no size and no colour**. A
plugin therefore *cannot* draw something that looks foreign, cannot escape the theme, and cannot
break the rule that no literal colour leaves `theme.rs`. A constraint the interface already enforces
against itself is one a third party gets for free.

One new panel variant carries every plugin: `PanelKind::Plugin(PluginId, SurfaceId)`, whose body is
a surface. Not one variant per plugin, and no new rail mode — the rail is Ubiq's own furniture.

What A2UI cannot do, stated so nobody discovers it later: a canvas, a graph, a diff, a code editor,
a virtualised table of a hundred thousand rows, and drag-and-drop. A plugin that needs one of those
is asking for tier 3.

**Tier 3 — a tenant of the web panel. Out of scope here.** The [web panel
proposal](./web-panel-proposal.md) designs a webview over the interface's own loopback origin with a
typed bridge, for exactly one tenant, and calls the general case "a door held open". A plugin tenant
would need a per-plugin origin, a CSP per plugin, and a second sandbox to get right — the bridge's
own rule is that it carries documents and never commands, and a plugin's bridge would have to carry
commands. The conditions for opening it are: the web panel has shipped for Excalidraw, a plugin
exists whose UI genuinely cannot be described, and the origin-per-plugin question has an answer.
Until all three, no.

**Tier 4 — native plugins, dynamic libraries, plugin-drawn GPUI elements. Refused, permanently.**
An ABI to keep stable, a crash that takes the window with it, no sandbox at any price, and a plugin
that can paint whatever colour it likes. There is no version of this worth having.

## 7. The example, end to end

An Azure DevOps plugin that keeps the board in sync and gives agents a lookup tool. The manifest is
§3. The plugin is roughly this:

```js
ubiq.on("project.open", async () => {
  const wi = await ubiq.net.fetch(
    `https://dev.azure.com/${ubiq.settings.organisation}/_apis/wit/wiql?api-version=7.0`,
    { method: "POST", auth: "connector:azure-devops", json: { query: WIQL } });
  const tasks = await ubiq.work.listTasks();
  for (const item of wi.workItems) await reconcile(item, tasks);
});

ubiq.tools.define({
  name: "find_work_item",
  description: "Look up an Azure DevOps work item by id or title",
  schema: { id: "number?", title: "string?" },
  handler: async (a) => search(a),
});
```

What the pieces are: `net` is a grant to one host and a token the plugin never sees; `work` is the
family that already exists, writing the same `tasks.toml` the board reads, so the cards move without
a line of interface code; `settings.organisation` came from the schema the manifest declared and a
form Ubiq drew; `tools.define` puts one tool in front of every agent in the project, running with
the plugin's grants and appearing in the transcript as `plugin.azure-devops-sync.find_work_item`.
The board panel, if it wants one, is an A2UI surface of `Card` and `Text` and a `Button`.

Nothing in that list is a mechanism this document invents except `net`, the settings schema, and the
A2UI data model.

## 8. What this proposes not to do

- **No plugin API.** If a capability is worth giving a plugin it is worth being a message, and if it
  is not worth being a message a plugin should not have it.
- **No npm, no Node, no runtime download.** Bundling is the author's job and happens before Ubiq
  sees the plugin.
- **No registry or marketplace.** A folder under the config root and a git URL. A registry is a
  distribution problem, and distribution is not what is unsolved here.
- **No plugin code in `crates/ubiq`.** The interface draws contributions; it does not host them.
- **No new rail mode, no arbitrary panel, no HTML, no native code.**
- **No dynamic capability registration.** The set of things a plugin can ask for is a table in the
  source, reviewed when it changes.
- **Plugins are not how Ubiq gets features.** The failure mode of every extensible application is a
  thin core and an ecosystem that carries it. A plugin is for what is specific to one company, one
  service or one person — not for what Ubiq should do itself.

## 9. What the sandbox does not buy

Worth a section of its own, because a security model that oversells itself is worse than none.

**A sandbox limits reach, not intent.** A plugin granted the project tree and one network host is,
by construction, able to send the project's source to that host. That is not a bug in the design; it
is what the user consented to. It follows that the consent screen must show the *combination* rather
than the list — "reads your project files" beside "reaches dev.azure.com" is the sentence that
matters, and neither line alone says it.

**A bundled dependency is the plugin's own code.** There is no supply-chain protection here beyond
the grant: an npm package the author bundled runs with exactly the plugin's authority. Pinning a
hash of the artefact at install and refusing to run a changed one without re-consent is the whole
mitigation.

**In-process means a runtime bug is a host bug.** QuickJS is small and well-exercised, and it is C.
Budgets and the absence of authority are the defence; there is no second wall behind them until a
plugin runs out-of-process, which is why the transport being framed for a socket is worth keeping
true.

**An agent can be talked into calling a plugin tool.** §5's privilege rule is what contains it, and
the containment is only as good as the grant the plugin asked for.

## 10. Phasing

| Phase | What lands | Rough cost |
|---|---|---|
| 0 | `Message::capability()` and its exhaustive match; `crates/ubiq-plugin` with the grant model and the broker; a plugin client on the hub; one in-tree test plugin driven by fixtures, no interpreter | 1 week |
| 1 | `rquickjs`, the `ubiq.*` shim, the manifest, the install and consent screens, commands, `store`, `net`, notifications, budgets and the audit view | 2–3 weeks |
| 2 | The settings schema and its renderer; `exec`; `ask` grants prompting through the permission machinery | 1 week |
| 3 | The host as `McpService`; `tools.define`; skill and instruction contributions; the privilege rule in the transcript | 1–2 weeks |
| 4 | A2UI surfaces: the data model, the id-to-entity map, action dispatch, surface lifetime, `PanelKind::Plugin` | 2–3 weeks |
| 5 | Out-of-process plugins over the framed transport | Not proposed |

Phases 0 to 2 are a plugin system that is useful without drawing anything, and phase 3 is where the
example in §7 earns its place. Phase 4 is the expensive one and the one that also unblocks `G206`.
Phase 5 is what to build if the in-process boundary ever proves insufficient, and isol8's Linux seam
is its precondition.

## 11. Decisions this would add

- **D97** — A plugin is a bus client with the message set filtered by a grant, rather than an API of
  its own. **Cost:** every new message variant now owes a capability classification, and a message
  set designed for one trusted peer is being asked to hold a trust boundary it was not drawn for.
- **D98** — Embedded QuickJS over Wasm components, an out-of-process Node, and Lua. **Cost:** a C
  dependency in the host process with no wall behind it, and no isolation from the operating system
  on any platform.
- **D99** — A plugin holds credential references and never credential material; the broker attaches
  the token. **Cost:** every capability that authenticates has to be brokered, so a plugin can never
  reach a service Ubiq has no connector for.
- **D100** — Plugin UI is declarative contributions plus A2UI, and never HTML or native drawing.
  **Cost:** a class of plugin — anything wanting a canvas, a graph or a large table — cannot be
  written, and Ubiq owns the A2UI data model it had been deferring.

## 12. Open questions

- Whether a plugin's grant is per-plugin or per-plugin-per-project. A sync plugin wants the second;
  every extra axis is another thing the consent screen has to explain.
- Whether `activation` events are worth having in phase 1, or whether every plugin simply loads with
  the project until one is slow enough to notice.
- What a plugin sees of a project it was not granted: the catalogue lists every project, and
  `work` and `files` grants are already project-scoped, so `project = "r"` may be too coarse.
- Whether the audit trail is a filter over the existing `Tape` or a record of its own, given the
  tape is in-memory and a plugin's history is worth keeping.
- Where a plugin's *definition* lives, given
  [`config-persistence-proposal.md`](./config-persistence-proposal.md) reserves definitions for
  agent-manager and names plugins in that list — the answer is probably that the library's plugin is
  a harness-facing thing and Ubiq's is not, and that two words are colliding the way "session" does.
- Whether `exec` earns its place in phase 2 at all, given that it is the one capability with no
  ceiling and its OS confinement works on one platform.
- Whether a plugin may raise a permission prompt against a *pane*, or only against a conversation.

## Related docs

- [Architecture](../tech/architecture.md) — the two halves, the bus, and the crate boundary R3 copies
- [Transport contract](../tech/transport-contract.md) — the families a grant filters, and where the
  classification step is added
- [The agent-manager boundary](../tech/agent-manager.md) — who owns MCP injection, skills and
  confinement
- [Proposal — A2UI](./a2ui-proposal.md) — the renderer, its phase 0, and the data model §6 adopts
- [Proposal — isolated runs](./isolation-proposal.md) — what isol8 enforces and where
- [Proposal — web panels](./web-panel-proposal.md) — the door tier 3 would open
- [Decision register](../tech/decisions.md) — where D97 to D100 land if this is built
