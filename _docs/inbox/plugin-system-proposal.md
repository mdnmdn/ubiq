
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