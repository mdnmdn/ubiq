---
id: ref-a2ui-protocol
title: A2UI protocol — the wire
kind: reference
status: current
summary: The Agent-to-UI protocol as an external contract — the six agent-to-renderer envelopes and the four renderer-to-agent ones, the surface lifecycle, the flat adjacency-list component model, JSON-Pointer data binding and two-way input, actions and function calls, catalog negotiation, the version landscape, and the SDKs that exist.
read_when: you are deciding whether an agent-authored user interface can reach a Ubiq surface, or you need the exact shape of an A2UI message before writing a parser
updated: 2026-09-09
verified: 2026-09-09
depends_on: [ref-a2ui-catalog]
---

# A2UI protocol — the wire

**A2UI (Agent-to-UI)** is an open protocol that lets an agent describe a user interface as
declarative JSON rather than as HTML, code or plain text. The agent sends the *intent* of an
interface — a component tree plus a data model — and the receiving application draws it with its
own native widgets. The project's phrase for the property this buys is that agent-generated
interfaces become **"safe like data, but expressive like code"**: nothing executable crosses the
boundary, and the renderer only ever draws components it implements.

The project is open source under **Apache 2.0**, backed by Google with contributions from
CopilotKit and the wider community, and lives at
[a2ui-project/a2ui](https://github.com/a2ui-project/a2ui) with documentation at
[a2ui.org](https://a2ui.org/). Its own status line reads **early stage public preview** —
"functional but still evolving. Expect changes."

This document covers the wire. The component vocabulary an agent draws from is a separate document:
[A2UI catalogs and components](./a2ui-catalog.md).

## The version landscape

| Version | Standing |
|---|---|
| v0.8 | Legacy |
| v0.9 / **v0.9.1** | The stable family; v0.9.1 is the production release |
| **v1.0** | Release candidate — "a candidate for becoming stable", with a high bar for further breaking changes |

Everything below describes **v1.0**, whose specification directory is
[`specification/v1_0`](https://github.com/a2ui-project/a2ui/tree/main/specification/v1_0). v1.0 was
drafted under the name v0.10.

Five things separate v1.0 from v0.9, per the protocol document's own summary:

- **Bidirectional function calls.** Typed invocation messages in both directions, verified against
  the catalog at runtime.
- **Single-message instantiation.** `createSurface` carries the initial `components` and
  `dataModel` inline, so a whole interface fits in one payload.
- **Decoupled branding.** The rigid theme properties and hardcoded brand colours are gone; visual
  styling defers entirely to the target framework's native theme.
- **Enhanced catalog schemas.** Function definitions become object maps for O(1) lookup, and inline
  catalogs carry standard `$schema` and `$id`.
- **Strict identifiers.** Every catalog entity name obeys Unicode UAX #31, and the `@` namespace is
  reserved for system evaluations such as `@index`.

The schema files are renamed to match the vocabulary: v0.9's `server_to_client.json` and
`client_to_server.json` become
[`agent_to_renderer.json`](https://raw.githubusercontent.com/a2ui-project/a2ui/main/specification/v1_0/json/agent_to_renderer.json)
and
[`renderer_to_agent.json`](https://raw.githubusercontent.com/a2ui-project/a2ui/main/specification/v1_0/json/renderer_to_agent.json).
The two sides of a conversation are the **agent** and the **renderer**.

## Transport

A2UI mandates no transport. It defines the JSON message structure and the semantic contract, and
asks any carrier to meet four conditions, set out under *Transport decoupling* in the
[protocol specification](https://github.com/a2ui-project/a2ui/blob/main/specification/v1_0/docs/a2ui_protocol.md):

1. **Reliable, ordered delivery.** Updates are stateful — a surface is created before it is updated
   — so out-of-order delivery corrupts the interface.
2. **Message framing.** Individual JSON envelopes must be delimited: newlines in JSONL, WebSocket
   frames, SSE events.
3. **Metadata support.** Needed for data-model synchronisation and for the capability exchange that
   tells each side which catalogs the other knows.
4. **A return channel** (optional in principle, required for anything interactive).

Named bindings: **AG-UI** as the standard binding for agent-to-user interaction, **A2A** through a
dedicated extension that maps envelopes into A2A messages, and **MCP**, where A2UI rides over tool
calls, tool outputs or resource subscriptions. SSE with JSON-RPC, WebSockets and plain REST are
listed as workable carriers, REST with the caveat that it cannot stream.

## Agent to renderer: six envelopes

Every message is a JSON object carrying `version` plus **exactly one** other key. The schema is a
`oneOf` over six shapes, and `additionalProperties` is false throughout — an envelope with two
payload keys is invalid.

| Envelope | What it does |
|---|---|
| `createSurface` | Opens a surface. Only `surfaceId` is required; `components`, `dataModel`, `catalogId`, `sendDataModel` and `metadata` are optional |
| `updateComponents` | Upserts component definitions into an existing surface, by `id` |
| `updateDataModel` | Replaces the value at a JSON Pointer `path` in the surface's data model |
| `deleteSurface` | Removes the surface and everything in it |
| `callRendererFunction` | Asks the renderer to execute a catalog function locally, under a `functionCallId` |
| `agentFunctionResponse` | The agent's answer to a `callAgentFunction`, keyed by the same `functionCallId` |

A `surfaceId` must be globally unique for the renderer's lifetime, and creating one that exists
is an error — the old surface has to be deleted first.

```json
{
  "version": "v1.0",
  "createSurface": {
    "surfaceId": "user_profile_card",
    "catalogId": "https://a2ui.org/specification/v1_0/catalogs/basic/catalog.json",
    "sendDataModel": true,
    "components": [{"id": "root", "component": "Column", "children": ["user_name"]}],
    "dataModel": {"name": "John Doe"}
  }
}
```

`updateDataModel` is a replace at a path, never a diff. An omitted `path`, or `/`, replaces the
whole model; an explicit `null` value deletes the key.

```json
{"version": "v1.0", "updateDataModel": {"surfaceId": "user_profile_card", "path": "/name", "value": "Jane Doe"}}
```

## Renderer to agent: four keys

The return schema is the mirror image — `version` plus exactly one of `action`, `callAgentFunction`,
`rendererFunctionResponse` or `error`.

An **action** is the only way user input reaches the agent. It carries `name`, `surfaceId`,
`sourceComponentId`, an ISO 8601 `timestamp` and a `context` object, all required, plus an optional
human-readable `userMessage` and optional `metadata`.

```json
{
  "version": "v1.0",
  "action": {
    "name": "submit_form",
    "surfaceId": "user_profile_card",
    "sourceComponentId": "submit_button",
    "timestamp": "2026-09-09T10:00:00Z",
    "context": {"email": "jane@example.com"}
  }
}
```

An **error** reports a renderer-side failure. Three codes are reserved and structured —
`VALIDATION_FAILED`, `UNALLOWED_PARENT`, `UNALLOWED_CHILD` — each carrying the JSON Pointer to the
offending field and a one-or-two-sentence `message`. Any other code takes the generic shape and
names either a `surfaceId` or a `functionCallId`, never both.

## The surface lifecycle

A surface is the unit of interface. `createSurface` implicitly instantiates a reserved `Surface`
container whose `child` is fixed to `root` — it is immutable and cannot be addressed by
`updateComponents`. Catalogs are forbidden from defining a component named `Surface`.

The ordinary sequence: create the surface, stream component definitions into it, stream data into
it, let the user interact, push further updates as the conversation moves, and delete the surface
when the region is done with. The steps need not arrive in that order.

## The component model: a flat adjacency list

The interface is **a flat list of components; the tree is implicit**. Containers name their children
by `id`, and the renderer keeps every component in a map and rebuilds the tree at draw time.

Two consequences follow, and they are the reason the model exists:

- **Definitions may arrive in any order.** An agent can emit a parent before its children.
- **Rendering starts the moment `root` exists.** Exactly one component carries the id `root`. Until
  it arrives, updates are buffered and nothing is visible; after it arrives, the renderer draws the
  best tree it can from what it holds, **skipping invalid references** rather than failing.

Catalogs may constrain nesting with `allowedParents` and `allowedChildren` on a component
definition; the renderer evaluates them against the live tree, with `Surface` acting as the
top-level parent, and reports a breach as `UNALLOWED_PARENT` or `UNALLOWED_CHILD`.

## Data binding

Any bindable property is typed `DynamicString`, `DynamicNumber`, `DynamicBoolean`,
`DynamicStringList` or the open `DynamicValue`, defined in
[`common_types.json`](https://raw.githubusercontent.com/a2ui-project/a2ui/main/specification/v1_0/json/common_types.json).
Each accepts three forms: a **literal**, a **`{"path": "..."}` binding**, or a
**`{"call": "...", "args": {...}}` function call**.

Paths are **JSON Pointers** (RFC 6901) resolved against an evaluation scope:

- A path starting with `/` is **absolute** and resolves from the data-model root, wherever the
  component sits in the tree.
- Inside a templated list, a path **without** a leading slash is **relative** to the current item —
  `firstName` under a template iterating `/users` resolves to `/users/0/firstName`, then
  `/users/1/firstName`, and so on. Absolute paths still reach the root from inside a scope.

During streaming a path may resolve to nothing, because the `updateDataModel` carrying it has not
arrived. Renderers are asked to handle that gracefully — an empty string, or a loading indicator —
rather than treat it as an error. Non-string values interpolate by standard string conversion, with
null and undefined becoming `""` and objects and arrays becoming JSON.

### Two-way binding is local

Input components — `TextField`, `CheckBox`, `Slider`, `ChoicePicker`, `DateTimeInput` — bind both
ways, but **only against the renderer's own copy of the data model**. A keystroke writes to the
bound path immediately, and every other component bound to that path updates in real time. None of
it reaches the agent.

State crosses to the agent at exactly one moment: an action fires, and the renderer resolves the
paths named in that action's `context` and sends the resolved values. If the surface was created
with `sendDataModel: true`, the renderer additionally attaches the full data model to every message
it sends to the creating agent, which is what makes the two copies converge.

## Actions and function calls

A component's `action` property is a `oneOf`: either an **event** dispatched to the agent — a
required `name`, an optional `userMessage`, and a `context` map whose values are literals or
bindings — or a **local function call** the renderer executes itself, which is how `openUrl` works
without a round trip. The specification's own guidance on `context` is to use literal values unless
a value must genuinely track the data model.

Beyond that, v1.0 has symmetric RPC. `callRendererFunction` and `callAgentFunction` each carry a
`functionCallId` that the answering side **must** copy into its `rendererFunctionResponse` or
`agentFunctionResponse`; a response carries either a `value` or an `error`, never both.

## Catalogs and negotiation

A **catalog** is a JSON Schema document listing the components and functions an agent may use. It
carries a `catalogId` — conventionally a URI to avoid collisions, but treated as an arbitrary
string, not something the renderer resolves — plus optional `instructions` (Markdown design rules
that steer the model generating the interface), a `components` map and a `functions` map.

The envelope schema is deliberately catalog-agnostic: it references components through the
placeholder `catalog.json#/$defs/anyComponent`, which a validator maps to whichever catalog is in
play. A surface names one catalog through `catalogId` at creation, individual components and
function calls may override it, and everything the agent sends is validated against it. Renderer and
agent have to agree on well-known catalog ids for their systems to interoperate.

The security argument rests here: an application that ships its own catalog **restricts the agent to
exactly the components and visual language that exist in it**.

## Relation to neighbouring protocols

- **AG-UI** — the standard transport binding for agent-to-user interaction.
- **A2A** — a dedicated extension maps A2UI envelopes into A2A messages, standardising metadata
  placement, capability negotiation and data-model synchronisation. Ubiq's own harness transport is
  a different protocol; see [the ACP wire reference](./acp-protocol.md).
- **MCP** — A2UI travels over tool calls, tool outputs or resource subscriptions, and the repository
  carries a `catalogs/mcp` directory for it.

## Implementations

The repository holds a Python agent SDK under `agent_sdks/python`, renderers for Angular, Flutter,
Lit, React, Markdown and a shared `web_core`, and top-level `swift/`, `dart/` and `kotlin/`
directories, alongside `conformance`, `blueprints`, `samples` and `eval`.

**No Rust implementation exists** — not a renderer, not an agent SDK. Anything Ubiq does with A2UI
starts from the JSON schemas. The separate A2A protocol has third-party Rust SDKs, but they carry
no A2UI layer.

## Related docs

- [A2UI catalogs and components](./a2ui-catalog.md) — the widget vocabulary and the basic catalog.
- [Agent Client Protocol (ACP) — wire reference](./acp-protocol.md) — the other agent protocol this
  library tracks.
- [Proposal — A2UI surfaces in Ubiq](../inbox/a2ui-proposal.md) — what adopting this would cost.
