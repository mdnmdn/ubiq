---
id: inbox-a2ui
title: Proposal — A2UI, and the tool call that draws one
kind: proposal
status: proposal
summary: Rendering agent-authored A2UI component trees inside a conversation as the result of an MCP tool call, using the in-process MCP server the harness library already ships — display-only first, buttons second, and the text-input machinery the interface does not have deferred until something needs it.
read_when: you are deciding whether an agent may draw its own interface inside Ubiq, how a harness that speaks no A2UI would emit one anyway, or what a dynamic component tree costs in a GPUI transcript
updated: 2026-09-09
depends_on: [ref-a2ui-protocol, ref-a2ui-catalog, feat-chat, tech-transport, tech-architecture, tech-decisions]
---

# Proposal — A2UI, and the tool call that draws one

An agent that wants a choice from a human has one shape available to it: prose, and a permission
prompt it does not control. [A2UI](../references/a2ui-protocol.md) is a wire format for the missing
third shape — the agent sends a declarative component tree, the client draws it in native widgets,
and the user's answer comes back as data rather than as a sentence to be parsed.

This proposes the smallest path to that inside Ubiq, and argues that the smallest path is not a
renderer for the protocol but **a renderer for one message of it, delivered as a tool result**.

## 1. The blocker, stated first

**No harness Ubiq hosts emits A2UI.** Not Claude Code, not Codex, not Gemini CLI, not opencode, not
Copilot CLI. The protocol's shipped SDKs are Python, Swift, Dart, Kotlin and two web renderers;
there is no Rust implementation of either half, and no harness in the catalogue speaks it.

A renderer with no producer draws nothing. Any proposal that starts with "parse the envelope set"
is proposing to build the second half of a bridge whose first half nobody has poured. So the
question this document answers is not *how does Ubiq render A2UI* — that part is cheap — but **what
makes an agent send some**.

## 2. The shape — one MCP tool, not a protocol port

Every harness in the catalogue speaks MCP. None of them speak A2UI. So the producer is not the
harness: it is a tool the harness is handed.

**The proposal is one MCP tool, `render_ui`, taking an A2UI `createSurface` payload as its
argument and returning the user's action as its result.** The agent calls it the way it calls any
other tool. It needs no A2UI support, no protocol negotiation and no upstream change — it needs a
tool description that tells it the payload shape, which is what a tool description is for.

And the mechanism to host that tool exists in this workspace. `crates/agent-manager/src/mcp/mod.rs`
defines `McpService` — an embedder-provided tool set, `tools()` plus `call()` — and
`crates/agent-manager/src/mcp/server.rs` hosts one on a loopback HTTP MCP endpoint behind the
`inproc-mcp` feature. `provision()` turns an `McpRef::InProcess` into an ordinary inline http server
in the spec before the harness ever sees it. The module header names the Ubiq multiplexer as the
embedder it was written for.

**Nothing in `crates/ubiq-host` implements `McpService` today.** That is the first piece of work,
and it is the piece that pays for itself: once the host is an `McpService`, `render_ui` is one
`ToolDef` on it, and so is the next tool anyone wants.

What the agent sends:

```json
{
  "version": "v1.0",
  "createSurface": {
    "surfaceId": "pick-branch",
    "components": [
      {"id": "root", "component": "Column", "children": ["q", "row"]},
      {"id": "q", "component": "Text", "text": "Which branch should I rebase onto?"},
      {"id": "row", "component": "Row", "children": ["main", "dev"]},
      {"id": "main", "component": "Button", "child": "main_l", "action": {"name": "pick", "context": {"branch": "main"}}}
    ],
    "dataModel": {}
  }
}
```

What comes back as the tool's result:

```json
{"action": {"name": "pick", "sourceComponentId": "main", "context": {"branch": "main"}}}
```

That is the whole contract of a first cut. No surface lifetime, no `updateDataModel`, no
`deleteSurface`, no catalog negotiation — a tool call is already a request/response pair with a
lifetime, and it is the same lifetime a surface wants.

### The cost this shape carries

`McpService::call` is synchronous and the in-process server handles one request at a time — its own
header says every request is handled fully before the next is read. A `render_ui` that blocks until
a human clicks **blocks every other in-process tool call from that harness for as long as the
prompt is on screen.** That is acceptable for a tool whose entire purpose is to wait for a human,
and unacceptable as a general property of the server. Either the wait is bounded by a timeout that
returns a "no answer" action, or the server learns to handle requests concurrently. This is the one
piece of the proposal that is not free, and it belongs to the harness library rather than to Ubiq.

## 3. What it touches

**The wire.** `ConvUpdate` in `crates/ubiq-proto/src/conversation.rs` is a closed enum and holds no
variant for a component tree. `ConvBlock` in `crates/ubiq/src/state/conversation.rs` is closed the
same way — `User`, `Agent`, `Thought`, `Tool`. Each needs one variant, and the answer needs one
client-to-host message carrying the surface id and the action, shaped exactly like
`AnswerPermission` in `crates/ubiq-proto/src/messages.rs`.

There is a cheaper seam worth weighing against a new variant. `ToolCallRecord` already carries
`content: Vec<ToolContent>` — the structured-tool-output slot, today `Text` and `Diff`. A third
`ToolContent` variant would make a rendered surface part of the tool call that produced it, which
is what it *is*, and would inherit the tool block's existing open/closed disclosure for free. The
argument against is that `ToolContent` is ACP's vocabulary and a component tree is not in it. The
argument for is that a surface with no tool call to belong to has no way back to the agent anyway.
**This proposal takes the `ToolContent` seam** and files the disagreement rather than settling it.

**The drawing.** `crates/ubiq/src/ui/conversation/mod.rs` is the one view both the chat panel and
the agents columns share, so a render arm added there reaches every surface at once. It already
builds heterogeneous children from runtime-length data in several places, which is the only
question that matters about GPUI here: `permission()` maps a harness-supplied `Vec<PermissionOption>`
into per-item buttons, each getting an element id from the view and a listener closing over an
option id that **nothing in the interface reads** — it is the harness's token, echoed back exactly
as it arrived.

That is the pattern a component tree needs, one level deeper. The recursion is unremarkable:

```rust
fn node(id: &str, tree: &Surface, view: &ConversationView, cx: &mut Context<AppState>)
    -> AnyElement
{
    match tree.get(id) {
        Some(Node::Text { text }) => TextView::markdown(view.eid(id), text).into_any_element(),
        Some(Node::Row { children }) => div().flex().gap_2()
            .children(children.iter().map(|c| node(c, tree, view, cx)))
            .into_any_element(),
        // Button, Column, List, Card, Divider, Image, Icon, CheckBox, ChoicePicker…
        _ => placeholder(id).into_any_element(),
    }
}
```

`AnyElement` erases the type, `children()` takes an iterator, and A2UI's flat adjacency list means
the recursion is a map lookup rather than a tree walk — a component may be referenced before it
arrives, which the placeholder arm already handles.

**The theme.** A2UI v1.0 carries no colour, no padding, no margin and no positioning: it exposes
`justify`, `align`, `weight` and a discrete `variant` enum per component, and defers everything
visual to the renderer's own theme. **An A2UI payload therefore cannot violate the rule that no
literal colour escapes the theme module.** For a format arriving from an untrusted producer that is
not a minor convenience; it is the property that makes this safe to draw at all.

## 4. Component coverage

The basic catalogue is eighteen components. Most of them are a call to something that exists.

| Component | Cost |
|---|---|
| `Text` | Free — `TextView` with `markdown_ast` already draws the transcript's prose |
| `Row`, `Column` | Free — `div()` with flex, `weight` maps to flex-grow |
| `Divider` | Free — the transcript already draws separators |
| `Card` | Free — `card()` in the kit takes an id and an edge colour |
| `Button` | Free — `primary_button()` and `ghost_button()`, wired exactly as `permission()` wires them |
| `Icon` | Cheap — a name-to-`IconName` map, with a placeholder for unknown names |
| `Image` | Cheap — `img()` with an `ImageSource`, but a remote URL is a network fetch the interface does not do today |
| `CheckBox` | Cheap — `check_box()` exists; the state is the expensive part, not the widget |
| `ChoicePicker` | Cheap — `choice_pill()` and `toggle_pill()` cover both variants |
| `List` | Moderate — the templated form needs the binding scope of §5 |
| `Slider` | Moderate — `meter()` draws one but does not drag |
| `TextField` | Expensive — §5 |
| `DateTimeInput` | Expensive — a text field plus a calendar the interface does not have |
| `Modal`, `Tabs` | Out of scope — a surface inside a transcript is already inside a layout that owns those |
| `Video`, `AudioPlayer` | Out of scope — no player, and no reason to build one for this |

Anything unrecognised draws a placeholder naming the component. The protocol's own guidance is
graceful degradation, and a transcript that renders fourteen of eighteen components and labels the
rest is more useful than one that refuses the payload.

## 5. The expensive part

**Text inputs, and they are expensive for a structural reason rather than a drawing one.**

Every field in Ubiq is a named `Entity<InputState>` allocated once during startup in
`crates/ubiq/src/app/boot.rs` — the file picker's, the commit search's, the task title's, twenty of
them — each held as a named field and each with its subscription parked in the state's
`_subscriptions`. The convention is one field, one name, one allocation, for the lifetime of the
window.

An agent-authored tree has an unknown number of inputs with ids nobody chose in advance. It needs a
runtime map from component id to `Entity<InputState>`, created lazily the first time a component is
seen and torn down when its surface goes away. **No such lifecycle exists anywhere in the
codebase.** It is not large, but it is new, and it is the only part of this proposal that invents a
mechanism rather than reusing one.

Three smaller things travel with it:

- **Focus.** Exactly one thing takes keystrokes, and the composer wants them. A form inside a
  transcript competing with the composer for focus is unspecified territory in every screen Ubiq
  has today.
- **Scrolling.** A surface tall enough to scroll inside a transcript that already scrolls needs its
  own id and tracked scroll state, which is a known GPUI sharp edge rather than a discovery.
- **Trust.** A button in Ubiq today calls a hand-written closure. A button in an agent-authored
  tree calls whatever the tree says, and the tree came from a model. The answer is that it calls
  nothing — it produces an action that becomes a tool result and reaches no state mutator — but
  that has to stay true as the renderer grows, which makes it a rule rather than an implementation
  detail.

Data binding rides along here too: JSON Pointer resolution against a local data model, two-way for
inputs, plus the `checks` validators that gate whether an action may fire. None of it is hard; all
of it is only needed once an input exists.

## 6. Phasing

| Phase | What lands | Rough cost |
|---|---|---|
| 0 | The host implements `McpService`; `render_ui` accepts a payload and returns a fixed action | 1–2 days |
| 1 | Display-only: `Text`, `Row`, `Column`, `Card`, `Divider`, `Icon`, placeholders for the rest | 2 days |
| 2 | `Button` and the action round-trip — the tool result carries what the user clicked | 2–3 days |
| 3 | `CheckBox`, `ChoicePicker`, `Slider`, `TextField`, the id-to-entity map, JSON Pointer binding, `checks` | 1–2 weeks |
| 4 | Real surfaces: `updateComponents`, `updateDataModel`, `deleteSurface`, lifetimes outliving a tool call | Not proposed |

Phases 0 to 2 are the proposal. They are worth building on their own — an agent that can ask a
structured question and get a structured answer is useful before a single input exists, and it is
the same round trip a permission prompt already makes, opened up to a shape the agent chooses.

Phase 3 is worth building when something asks for it. Phase 4 is worth building when a producer
exists that needs it, which is to say not on the strength of this document.

## 7. What this proposes not to do

**No new crate.** Not a `ubiq-a2ui`. The payload types are `serde` structs that belong beside the
conversation vocabulary they arrive through, and the renderer is a module in the interface's
conversation view. A crate is a boundary, and there is no boundary here to draw.

**No catalog negotiation.** The protocol lets a client advertise supported catalogue ids and the
agent lock one per surface. With one catalogue, partially implemented, that machinery describes a
choice nobody is making. The tool description says what the tool accepts, which is the same
information delivered where the agent reads it.

**No custom catalogue authoring.** A chart component is a good idea for the day something needs a
chart.

**No chase of v1.0.** The specification is a release candidate that renames fields between
versions — `theme` became `surfaceProperties` on the way to it. Building against v0.9's component
shapes and treating the envelope as one message is what keeps the cost of being wrong small.

**No surfaces outside a conversation.** A panel whose content is agent-authored is a different
proposal with a different risk profile.

## 8. Open questions

- Whether a rendered surface is a `ToolContent` variant or a `ConvBlock` of its own — §3 takes the
  first and does not settle it.
- What a blocking `render_ui` does to the in-process MCP server's one-request-at-a-time loop:
  timeout with a "no answer" action, or teach the server concurrency.
- What happens to a surface when its turn ends, its conversation is cancelled, or its agent exits
  with the tool call unanswered.
- Whether an `Image` component may fetch a remote URL, given that no other part of the interface
  makes an outbound request on a document's say-so.
- Whether an action from a surface is replayable — a transcript redraws, and a button that has
  already been answered has to know it.
- Whether this earns a `Dnn` entry when built, on the grounds that "an agent may draw inside the
  transcript" is a door rather than a feature.

## Related docs

- [A2UI — the protocol](../references/a2ui-protocol.md) — envelopes, lifecycle, data binding, actions
- [A2UI — the component catalogue](../references/a2ui-catalog.md) — the eighteen components and the layout model
- [The chat panel](../features/chat.md) — the transcript and tool blocks a surface would sit among
- [Transport contract](../tech/transport-contract.md) — the conversation family, and how a variant is added
- [Architecture](../tech/architecture.md) — the rule that the two halves talk only through the message set
- [The agent-manager boundary](../tech/agent-manager.md) — who owns the in-process MCP server
- [Decision register](../tech/decisions.md) — where this lands if it is built
