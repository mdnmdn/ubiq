---
id: wip-planning-system
title: The planning system — missions, plans and their annotations
kind: wip
status: draft
summary: A proposal for the planning flow — a mission/epic/user-story task type whose term is a setting, parent/child and task-to-task references, task attachments that cross the bus, a plan document stored beside the tasks carrying human and agent annotations, the editor that has to support selection, annotation and `/` commands, the MCP surface agents answer annotations through, the new-mission dialog that launches a planning assistant, a `mission assistant` profile flag and project-scoped profiles — each piece costed, with the decisions to take and a staging order.
read_when: you are deciding how the planning flow is shaped, or picking the first slice of it to build
updated: 2026-09-21
code_anchors: [crates/ubiq-proto/src/work.rs, crates/ubiq-proto/src/messages.rs, crates/ubiq-proto/src/projects.rs, crates/ubiq-proto/src/settings.rs, crates/ubiq-host/src/store/file.rs, crates/ubiq-host/src/work/mod.rs, crates/ubiq-host/src/mcp/catalogue.rs, crates/ubiq-host/src/mcp/tasks.rs, crates/ubiq-host/src/mcp/server.rs, crates/agent-manager/src/profile.rs, crates/ubiq-host/src/agent.rs, crates/ubiq/src/state/new_agent.rs, crates/ubiq/src/state/conversation.rs, crates/ubiq/src/ui/board/mod.rs, crates/ubiq/src/ui/board/form.rs, crates/ubiq/src/ui/viewer/markdown.rs, crates/ubiq/src/ui/viewer/mod.rs, crates/ubiq/src/app/nav.rs, crates/ubiq/src/app/web_panel.rs, crates/ubiq/src/state/dock.rs, crates/ubiq-host/src/plan/mod.rs, crates/ubiq-host/src/plan/blocks.rs, crates/ubiq-host/src/plan/lines.rs, crates/ubiq-host/src/plan/provenance.rs, crates/ubiq-proto/src/plan.rs, crates/ubiq-host/src/store/plan.rs, crates/ubiq-host/src/mcp/plan.rs, crates/ubiq/src/app/plan.rs, crates/ubiq/src/state/plan.rs, crates/ubiq/src/state/document.rs, crates/ubiq/src/ui/plan.rs, crates/ubiq/src/ui/editor.rs]
depends_on: [feat-workbench, tech-transport, tech-decisions, wip-kb, wip-web-panel-phase6]
---

# The planning system — missions, plans and their annotations

A proposal, not a settled design. It exists so the cost of the ask is visible before any of it is
built: every piece ends with a complexity verdict, every fork is a numbered decision with a
recommendation, and the staging section offers a first slice small enough to land alone.

## What is being asked

The user presses **New mission** on the tasks board. A dialog asks for a title, a description, a
`require plan` flag and an **assistant** — a profile marked as fit for planning. Pressing Start
creates the task and launches that assistant with the mission's information already in hand and the
right MCP servers enabled. The assistant and the user work the mission together: the assistant reads
the board, writes a **plan** — a markdown document stored beside the tasks and exportable as plain
markdown — and the user annotates it, selecting passages and leaving comments the assistant answers
through an MCP tool. A mission is the parent of the ordinary tasks that come out of the plan. Files
can be attached to the task, from the project, from the knowledge base or pasted. Tasks can
reference each other.

The word "mission" is itself a setting: a team that says *epic*, or *user story*, gets its own term,
app-wide and overridable per project.

## What this builds on

The ground this lands on, stated as it is today.

**A task is one record.** `TaskRecord` in `crates/ubiq-proto/src/work.rs` is both the wire type and
the stored type — `status`, `priority`, `shape`, `kind` (`Bug`, `Feature`, `Chore`, `Docs`),
`complexity`, `assigned_to`, `key`, `link`, `labels`, `colour`, `title`, `description` (markdown,
stored never parsed), `steps` (one flat level), `comments` (append-only, author host-stamped, `D121`)
and the two timestamps. There is no parent, no task-to-task link, no attachment and no document. The
only `parent` in the work model is `WorkAgent::parent`, which is agent spawn hierarchy and unrelated.
Labels carry a name and a colour index with no registry behind them (`D113`).

**Tasks are one file per project**, `tasks.toml` under the project's directory in the config root,
written by `crates/ubiq-host/src/store/file.rs` as a versioned envelope and rewritten whole and
atomically on every mutation. There is no file lock; serialisation is in-process only, through
`work::Handle`, which the coordinator and the MCP listener share. An absent file is not an empty
board — the first `ListWork` writes one (`crates/ubiq-host/src/work/mod.rs`). A corrupt file is
preserved aside and an unknown version is never overwritten.

**The work message family** lives in `crates/ubiq-proto/src/messages.rs`: `ListWork`, `CreateTask`,
`UpdateTask`, `SetTaskField` over a `TaskField` enum, `MoveTask`, `AssignTask`, `DeleteTask`, the
step verbs, `AddComment`, `AssignAgent`, `SendToAgent`; back come `WorkList`, `TaskCreated`,
`TaskChanged` (the whole record, replaced by id), `TaskDeleted`, `AgentChanged` and `WorkError`.

**The board** is `crates/ubiq/src/ui/board/mod.rs` for columns, cards and drag, with `board/detail.rs`
(read-only report) and `board/form.rs` (edit controls) sharing one panel slot, over
`crates/ubiq/src/state/board.rs` and the `WorkProjection` in `crates/ubiq/src/state/work.rs`;
handlers in `crates/ubiq/src/app/board.rs`. A drag only ever sends `MoveTask`.

**MCP** is one loopback HTTP listener, `POST /mcps/<agent-or-pane-id>/<mcp-name>`
(`crates/ubiq-host/src/mcp/server.rs`), stateless per request, identity read from the URL and looked
up in the registry. Handlers reach state through small `*Reach` / `*Access` structs holding Arcs and
a mailbox — `WorkAccess` carries the `work::Handle` and the broadcast mailbox. The catalogue is a
const table in `crates/ubiq-host/src/mcp/catalogue.rs` with three readers: the settings checklist,
`tools/list`, and the catalog-name check in `compose_run`. `manage-ubiq-tasks` and `use-task`
(`crates/ubiq-host/src/mcp/tasks.rs`) are the task servers; `ubiq-kb` is the eight-tool knowledge
base server; `ubiq-ask` parks a tool call until the user answers (`D138`) and is the precedent for
an agent waiting on a human inside a turn.

**A profile** is `agent_manager::profile::Profile` — `id`, `extends` (folded by `flatten()`),
`account`, `harness`, `defaults` (mcps, skills, model, hooks, instructions, thinking, prompt),
`isolate`, `mode`, `max_subagents`. It carries **no boolean flags at all**. It is stored as a
directory per profile, `<root>/<id>/profile.toml` plus a per-harness `base/`, and in Ubiq the root is
`<config root>/profiles` (`crates/ubiq-host/src/agent.rs`) — **global, with no project id anywhere in
the path or the record.** There is no separate profile editor: one shared form, `NewAgentForm`
(`crates/ubiq/src/state/new_agent.rs`, drawn by `crates/ubiq/src/ui/new_agent.rs`, reused inline by
the settings screen) serves both `Purpose::Start` and `Purpose::Profile` and round-trips through
`from_profile()` / `as_profile()`. Its one boolean, `persistent`, is always false and disabled.

**A launch** is `StartConversation` — agent id, project, session, path, agent type, account, profile,
model, thinking, mode and an `mcps` list — resolved by `compose_run` in
`crates/ubiq-host/src/agent.rs`, which splits the requested MCP names into Ubiq built-ins and on-disk
catalog entries and injects the built-ins as inline HTTP servers. The launch form already draws the
MCP checklist, seeded from the profile and overridable per launch; `StartConversation.mcps` is a
standalone pick rather than a diff. A profile's `defaults.prompt` is already an opening turn sent as
the first message — the mission briefing has a place to go without inventing one.

**Chat attachments never cross the bus.** `Conversation::attached` holds project-relative paths in
the interface; on send they are folded into the single `PromptAgent { text }` as `@path` mentions.
The entry points are the composer's `+` over the window file picker, `⌘V` (a copied path becomes a
tag; a pasted image is written to `.ubiq/pasted/` through `WriteProjectFile`) and drag-and-drop.
Picking from the knowledge base is not wired into that picker — it reads the project explorer only.

**The knowledge base** addresses a document as `<source-name>/path`, with an interface key
serialised `kb:{source}:{path}`, and a KB tab is an `OpenFile` with a source set.

**Settings** come in two shapes: app-wide `HostSettings` in `crates/ubiq-proto/src/settings.rs`, and
per-project overrides that live on `ProjectRecord` in `crates/ubiq-proto/src/projects.rs`. Two
patterns there are worth copying by name: `ProjectRecord::index` is `Option<IndexLevel>` where `None`
means *follow the app-wide value* — an override rather than a value, because "follow the default" and
"happens to equal the default today" are different answers — and `ProjectRecord::search_excludes` is
additive, unioned on top of the app-wide set.

**The editor edits; the preview only renders.** `crates/ubiq/src/ui/viewer/mod.rs:178` draws every
text buffer with `gpui_component::input::Editor` over an `EditorState` — a code editor with a
readable and writable selection range, `range_to_bounds`, a Monaco-shaped `TextDecorationCollection`,
soft wrap and line numbers — and Ubiq already drives it for vim motions, the outline's jump and
bookmark marks, which are `TextDecoration`s (`crates/ubiq/src/app/nav.rs:453`). The preview half,
`crates/ubiq/src/ui/viewer/markdown.rs`, wraps `gpui_component::text::TextView`: `selectable(true)`
yields a string, not a document range, and a block has no identity unless the markdown block hook
gives it one. Absent are a comment-thread record — nothing outside `ui/viewer/image_edit.rs`, which
annotates pixels — and a `/` surface: no `CompletionProvider` is implemented, though the trait and
its menu are compiled in.

## The pieces

### The item type axis, and the term

A mission is a task that is allowed to have children and to carry a plan. The board draws it, filters
by it and offers a different creation path to it. The term for it — *mission*, *epic*, *user story*
— is display text, set app-wide and overridable per project, and nothing but the interface reads it.

The record needs one discriminator the board and the host can branch on, and the host needs one
string it hands back for display. Neither is hard; what makes this more than trivial is that `kind`
already exists and is a different axis (what sort of work), so the choice of where the mission-ness
lives decides how every later piece reads. See decision 1 and decision 2.

**Complexity: small.** One optional field on `TaskRecord`, one `TaskField` arm, one card affordance,
one settings row and one project override. `tasks.toml` gains an optional key, which the versioned
envelope absorbs without a version bump.

### Parent and child

A mission is the parent of the tasks that come out of its plan. The board has to draw the relation
(a child count on the mission card, a parent breadcrumb on the child), filter by it, and refuse a
cycle.

The whole-file rewrite makes the storage question easy and the consistency question the real one: a
single `parent: Option<TaskId>` on the child is one field, rewritten with everything else, and the
parent's child list is derived on read. Deleting a parent has to do something defined — orphan the
children or refuse. Two levels is the ask; nothing here needs arbitrary depth, and allowing it buys a
cycle check nobody asked for.

**Complexity: medium.** `crates/ubiq-proto` (field, `TaskField` arm), `crates/ubiq-host` (validation
on create, update and delete), `crates/ubiq` (card, detail, form, filter). No file format change
beyond the field. `messages.rs` gains one `TaskField` arm and nothing else.

### Task-to-task references

"Connect other tasks as references" is a symmetric, untyped list of task ids on the record, drawn as
a list of chips on the detail panel that navigate to the other card. It is strictly less work than
parent/child: no cycle rule, no deletion semantics beyond dropping dangling ids on read, no
board-level drawing.

**Complexity: small.** One `Vec<TaskId>` field, one `TaskField` arm, one chip list and one navigate
action.

### Task attachments

A task's attachments differ from the chat's in the one way that matters: **the chat's never cross the
bus.** They are interface state folded into the prompt text at send. A task's attachment is stored,
so it has to cross, live on the record, survive a restart and be readable by an agent that never saw
the interface.

The record needs a reference, not content — a project-relative path, or a KB address in the
`kb:{source}:{path}` form the interface already serialises, plus an optional label. A pasted image
reuses the chat's own path: written through `WriteProjectFile` into `.ubiq/pasted/`, then attached by
path. The one genuinely new interface work is the picker: attaching from the knowledge base needs the
file picker to offer KB sources, which it does not do today. Agents adding further references is the
same `SetTaskField` from the MCP side, free once the field exists.

**Complexity: medium.** The field and its `TaskField` arm are small; the KB-aware picker is the cost,
and it is interface-only work in `crates/ubiq/src/app/picker.rs` and the file picker state.

### The plan document

A plan is a markdown document that belongs to any task carrying a `level` — not only to a mission
subtype, and refused for a task with none — exports as plain markdown, on request rather than as a
continuous mirror, and carries annotations — ranges of the document with a thread of comments from
humans and agents attached.

The card is explicit that the plan is stored **near the tasks, not inside the tasks file**, and that
is right for a reason beyond taste: `tasks.toml` is rewritten whole on every mutation, so putting a
long document in it makes every checkbox tick rewrite the plan. `<config root>/projects/<id>/plans/<TaskId>.<ext>`
is the obvious reading, one file per plan, and it gives the export for free if the body is markdown.

The hard part is not the storage, it is the **anchor**. An annotation must survive the document being
edited — by a human in the editor, by an agent through an MCP tool, by a wholesale rewrite. Character
offsets lose: any insertion above a range silently moves every annotation below it, and an agent
rewriting a section invalidates all of them at once with no way to tell. Three candidates, costed in
decision 4.

**Complexity: medium.** A new proto type, a new store module beside `store/file.rs`, a new message
family (load, save, add annotation, reply, resolve), and one broadcast. No interface work in this
piece — that is the next one, and it is where the money goes.

### The plan editor

The interface already holds most of it. The editor the file viewer draws is `EditorState`, and it
carries a selection range the application reads and writes (`selected_range`, `set_selected_range`,
`selected_text`), `range_to_bounds` — a range turned into screen pixels, which is exactly what
anchors a popover to a passage — a decoration layer modelled on Monaco's (`TextDecoration { range,
style }`, `TextDecorationCollection::{set, append, clear, get_ranges}`), soft wrap, line numbers,
folding, and LSP-shaped provider traits (`CompletionProvider` with `is_completion_trigger`,
`HoverProvider`, `CodeActionProvider`) each with its popover already drawn. Ubiq drives the first
group today — vim (`crates/ubiq/src/app/vim.rs:167`), the outline's jump
(`crates/ubiq/src/ui/outline.rs:376`), bookmarks (`crates/ubiq/src/app/nav.rs:453`, collections
cached per buffer in `crates/ubiq/src/app/mod.rs:635`) — and implements none of the providers.

What is absent is narrow: the `/` menu is a `CompletionProvider` impl against an existing popover
rather than a menu written from nothing; the preview hands back a selected *string* with no document
offsets and no per-block identity, so annotating from it needs the block hook rather than the
selection; and there is no comment-thread record or panel.

**Complexity: medium.** One `CompletionProvider` impl for `/`, one decoration collection for the
annotated ranges, one anchored thread popover over `range_to_bounds`, one thread panel, and the
markdown block hook if annotation is to work from the preview too. The routes are decision 6.

### The annotation MCP surface

An agent needs to list the plan's open annotations, read the passage each one points at, reply, apply
an edit the annotation asks for, and resolve it. That is five or six tools over the same plan store
the interface uses, reached the way every other server is reached: a `*Reach` struct holding the
store's handle and the mailbox, a handler module, a `ServerSpec` with its `ToolSpec`s in the
catalogue, and routing in `server.rs`. Built-ins bypass the on-disk MCP catalog entirely, so there is
no packaging step.

The one design point worth flagging: `ubiq-ask` parks a tool call until the user answers (`D138`).
The same mechanism would let an agent post an annotation and wait for the human's reply inside one
turn, which is a much better planning loop than polling. It is also the only place in the proposal
that needs no new mechanism at all.

**Complexity: small to medium.** One handler module and a catalogue entry, on top of the plan store
from the previous piece. Entirely `crates/ubiq-host`. It is small *because* the plan store is medium.

### The new-mission dialog and the assistant launch

A dialog with four controls — title, description, `require plan`, assistant — that on Start creates
the task and immediately launches a conversation. The board already has the precedent: `New agent`
sits beside `New task` in the toolbar and goes straight to the launch form, skipping the board
entirely.

The launch itself needs nothing new. `StartConversation` already carries a profile, a model and an
explicit `mcps` list, so the dialog picks the assistant's profile and ticks the planning servers. The
briefing — the mission's title, description, attachments and the board's own state — rides
`defaults.prompt`'s slot or the first `PromptAgent`, which is the existing mechanism for an opening
turn. "Further assistants can be spawned later" is the ordinary `New agent` path with the mission
preselected.

**Complexity: medium.** A new modal in `crates/ubiq`, its state, and one composed action that creates
a task and then starts a conversation. No proto change if the briefing goes through the existing
prompt path.

### The `mission assistant` profile flag

`Profile` has no boolean fields today, so this adds the first one and the pattern for the next. It is
an optional bool in `profile.toml`, threaded through `ProfileInfo` so the interface can filter the
assistant list, with one checkbox in the shared `NewAgentForm` under `Purpose::Profile` — beside
`persistent`, the disabled boolean that already proves the form can draw one.

**Complexity: small.** One field in `crates/agent-manager/src/profile.rs`, one in `ProfileInfo`, one
checkbox, one filter. `crates/agent-manager` gains no UI dependency, so `just core` stays green.

### Project-scoped profiles

"Profiles can also be set up per project, visible only in the project they were created in." Nothing
supports this today: the store root is `<config root>/profiles`, and neither `Profile` nor
`ProfileInfo` carries a project id. Two routes in decision 8 — a second store rooted under the
project's own directory, or a `project: Option<ProjectId>` field on the record with one flat store.
Either way the consumers multiply: profile listing, the settings screen, `compose_run`'s resolution,
and `extends` — a project profile extending a global one is the obvious want, and a global one
extending a project one has to be refused.

**Complexity: medium.** Touches `crates/agent-manager` (store, resolution, `flatten()`),
`crates/ubiq-host` (two stores or one filtered store) and `crates/ubiq` (the lists). No message
reshape if `ProfileInfo` simply gains a field.

## The decisions

### 1. How the item type is modelled

`kind` (`Bug`, `Feature`, `Chore`, `Docs`) answers *what sort of work*. Mission answers *what level*.
Three options: add `Mission` to `Kind`; add a new optional `level` axis beside it; or make a mission a
separate record with its own store.

Folding it into `Kind` is cheapest and wrong — it makes "a mission that is a bug fix" unsayable and
puts a structural flag in a cosmetic enum. A separate record doubles the store, the message family
and the board's projection for something that is a task in every respect that matters: it has a
status, it moves between columns, it takes comments.

**Recommendation: a new optional axis on `TaskRecord`** — `level: Option<Level>` with a `Mission` arm
(name it for the concept, not for the user's word; the user's word is decision 2). `None` is an
ordinary task. It leaves `kind` alone, it costs one `TaskField` arm, and the parent rule reads
naturally: only a record with a `level` may be a parent.

### 2. Where the term is set

The display term is app-wide with a per-project override, and `ProjectRecord::index` is exactly that
shape already.

**Recommendation: `HostSettings::mission_term: String` defaulting to `"Mission"`, and
`ProjectRecord::mission_term: Option<String>` where `None` means follow the app-wide value.** Copy
`index`'s optionality rather than `search_excludes`'s union — a term is a value, not a set. The board
reads it through the projection and nothing else in the tree knows the word exists.

### 3. How parent and child are stored

`tasks.toml` is rewritten whole, so either direction of the link persists identically. The question is
which is authoritative.

**Recommendation: `parent: Option<TaskId>` on the child, and nothing on the parent.** One writer per
fact, no list to keep in sync, and a dangling parent id is dropped on read the way an unknown label
would be. Deleting a parent **orphans its children** — clears their `parent` — rather than refusing
or cascading: refusing makes a board unclearable, and cascading deletes work a user did not select.
Depth is capped at one: a task with a `parent` may not itself be a parent, which removes the cycle
check entirely.

### 4. The annotated-plan file format

The anchoring problem is the real question, and character offsets lose — an insertion anywhere above a
range moves every annotation below it, and an agent that rewrites a section invalidates all of them
with no signal.

- **Quoted-context anchors** — store the annotated text plus a few words either side, and re-find it
  on load. Cheap, format-free, survives edits elsewhere in the document, and fails silently when the
  quoted text is itself edited. Fuzzy re-finding is a tuning problem with no end.
- **Stable block ids** — every block-level element carries an id; an annotation names a block id and,
  optionally, a quoted span inside it. An edit inside a block keeps the anchor; a deleted block
  orphans its annotations explicitly, which is a reportable state rather than a silent loss.
- **A CRDT-ish id per inline run** — precise, and it makes the document no longer markdown: every
  writer has to go through a library, and the export becomes a render.

**Recommendation: stable block ids, in a sidecar.** The plan body stays plain markdown on disk —
which is the export, for free, and the thing an agent can read and rewrite with ordinary file tools —
and a sibling file (JSON, one object per annotation: block id, quoted span, thread, state) holds the
annotations. Block ids are assigned on save by walking the parsed document and matching against the
previous version; an unmatched block is new, a vanished block orphans its annotations and the
interface says so. Two files per plan, `<TaskId>.md` and `<TaskId>.annotations.json`, which also
means a plan opened in any other editor is undamaged.

### 5. Where plans live

**Recommendation: `<config root>/projects/<ProjectId>/plans/<TaskId>.md` and `.annotations.json`.**
It is the plain reading of "near the tasks", it sits beside `tasks.toml` and `kb.toml` in a directory
that already exists, and it keeps plans out of the user's repository — which matters, because a plan
is workspace state and `D30` already puts that kind of thing in the config root. Export writes a copy
wherever the user asks.

### 6. Which surface the plan editor is

**A. A web-panel tenant.** The editor rides the phase-6 machinery Excalidraw and draw.io ride —
embedded webview, chrome page, vendor mirror, bridge, mark-and-sweep clipping — and a mature editor
with comment threads and slash commands is a component rather than a thing to write. The second
tenant (draw.io, `web-panel-phase6.md`) cost a `ViewerKind` arm, a mirror entry, one bridge frame, a
cache tier, an explorer row and a file seed, so the plumbing is a known quantity. It forfeits Linux:
`gpui-wry` is compiled on macOS and Windows only, so there the plan editor is a browser window beside
Ubiq — tolerable for a diagram, not for the surface the planning loop lives in. The bridge also
carries `document: String` and nothing else, so annotations need their own frames and block ids have
to be assigned inside the page.

**B. Native, on the editor that is already there.** A `.md` plan is a text tab: `EditorState` gives
the buffer, the selection and the undo history. Annotated ranges are a `TextDecorationCollection`,
which `app/nav.rs` already builds for bookmarks. The thread popover is `kit::popover` positioned by
`range_to_bounds`. The `/` menu is one `CompletionProvider` impl — `is_completion_trigger` fires on
the typed character and the library's menu draws itself. The thread list is an ordinary panel. Block
ids are assigned in Rust on save, which decision 4 and `ubiq-plan` need anyway. It forfeits WYSIWYG:
the writing surface is markdown source with a preview, not a Notion page. Nobody asked for WYSIWYG.

**C. Web first, then native.** Cheap exactly when the format and the MCP surface are the contract and
the editor is a leaf — and the Excalidraw precedent says that much holds: `FromWeb::Changed` is
`EditorState::set_value` on the buffer the tab already holds and `Save` is the tab's own `⌘S`, so a
web tenant's edits go **through** the normal save path, not around it
(`crates/ubiq/src/app/web_panel.rs:14`). A trap all the same. The switch throws away six pieces and
keeps two: gone are the vendor mirror, the chrome page and its CSP, the annotation bridge frames, the
in-page block-id assignment and its Rust twin, the `ViewerKind`/layout arm and the Linux fallback
copy; kept are decision 4's two files and the `ubiq-plan` tools — which B gets to first anyway. And
building web first pays A's Linux forfeit for a release, then asks the users who learned that editor
to learn another.

**Recommendation: B, native on the existing editor.** This reverses the earlier reading, on a fact
rather than an argument: the claim that the tree has no selection model and no decoration layer was
wrong — it has both, ships them in the file viewer, and already uses them for vim and bookmarks. That
turns the native route from "write a rich text editor", which is a product, into "implement one
provider trait and one popover against an engine already compiled in", which is a slice. It is also
the only option that works on every platform Ubiq builds for, and the plan is not a diagram: it is
where the planning loop happens. A wins only if the plan must be WYSIWYG rich text, and that is the
question worth re-asking before slice 6.

A middle position remains in view: **ship the plan as a read-only rendered markdown tab first**, with
annotations at block granularity from the preview's block hook and answered through MCP. That is most
of the planning loop without an editing surface, and it is why slice 3 is shaped the way it is.

### 7. A new MCP server, or tools on `manage-ubiq-tasks`

`manage-ubiq-tasks` has twelve tools and `use-task` has seven. Adding six plan tools to the first
makes an eighteen-tool server whose name no longer describes it, and forces every agent that wants
task management to carry plan tools it will not use.

**Recommendation: a new `ubiq-plan` server.** The catalogue is a const table and adding an entry is
cheap; the tick boxes in the launch form are the composition mechanism, and a planning assistant
ticks `manage-ubiq-tasks` and `ubiq-plan` together. It also keeps the `*Reach` clean: `PlanReach`
holds the plan store, not the work handle.

### 8. How a profile gets a project scope

Path scoping (`<config root>/projects/<id>/profiles/<profile-id>/`) keeps the record untouched and
makes the scope a fact about where it was found — which means `ProfileInfo` still has to gain a field
so the interface can label it, and `compose_run` has to be handed two stores.

A `project: Option<ProjectId>` field on `Profile` keeps one store and one root, and makes filtering a
predicate instead of a second traversal.

**Recommendation: path scoping, with the scope surfaced on `ProfileInfo`.** A project profile is
deleted when the project is, it is copied when the project is, and a file whose location is its scope
cannot contradict itself. A field in the record can say `project = X` while sitting in the global
root, and someone will eventually put it there. The extra store is a second `FsProfileStore` with a
different root, which is the cheapest half of the work either way.

## A staging proposal

Each slice stands alone and is useful on the day it lands.

1. **Built. The mission task type and its term.** `level: Option<Level>` on `TaskRecord`,
   `TaskField::Level` promoting and demoting a task, the board card's leading accent chip and the
   task panel's own Level switch, `HostSettings::mission_term` for the app-wide word and
   `ProjectRecord::mission_term` for a project's own override, drawn as a Default/Custom pill pair
   in the project's Tasks tab. Decisions 1 and 2, taken as recommended. The board draws a mission
   that reads as one, and every later slice has something to hang off. *Unlocks: everything.*
2. **Built. Parent, children and references.** `parent: Option<TaskId>` on the child and nothing on
   the parent, `TaskField::Parent` enforcing the one-level rule host-side and refusing with
   `WorkError` where it does not hold, `references: Vec<TaskId>` and `TaskField::References`
   replacing the whole set, the board card's child-count chip beside the mission chip, and the task
   panel's Parent picker and References chip list. Deleting a mission orphans its children rather
   than cascading or refusing the delete. Decision 3, taken as recommended. The board is a two-level
   board and a mission is worth creating. *Unlocks: the mission dialog's reason to exist.*
3. **Built. The plan as a stored markdown document, read-only in the interface.** `FilePlanStore`
   (`crates/ubiq-host/src/store/plan.rs`) writes one file per plan at
   `<config root>/projects/<ProjectId>/plans/<TaskId>.md`; `crate::plan::Plans` holds the level
   check and refuses the family for a task carrying no `level`, on `Work::parent_refusal`'s own
   posture. `LoadPlan`, `SavePlan`, `DeletePlan` and `ExportPlan` cross the bus, answered by `Plan`,
   `PlanDeleted`, `PlanExported`, `PlanChanged` and `PlanError`. A mission's task panel opens the
   plan in a read-only modal over the tree's own Markdown renderer, rather than the rendered tab
   first sketched — with an Export action writing an explicit one-shot copy into the project's
   tree, never a continuous mirror. The `ubiq-plan` MCP server carries `read_plan` and `write_plan`.
   Decisions 4 (body half), 5 and 7 as recommended, and decision 6's read-only middle position in
   place of an editor. *Unlocks: the annotation layer, and it proved the storage choice first.*
4. **Built. Annotations without an editor.** `PlanBlock`/`Annotation`/`AnnotationState`
   (`crates/ubiq-proto/src/plan.rs`); `crates/ubiq-host/src/plan/blocks.rs` matches blocks across a
   save (identical text, then word-Dice similarity, nearest in document order breaking ties) and
   orphans, never deletes, a vanished block's annotations. The sidecar carries the index and the
   threads; `ListPlanAnnotations`, `AnnotatePlan`, `ReplyToAnnotation` and `ResolveAnnotation` cross
   the bus. `ubiq-plan` gains `list_annotations`, `reply_annotation` and `resolve_annotation` — no
   `annotate_plan`, and the `ubiq-ask` parking trick below was **not built**: an agent answers a
   thread a human opened and does not wait inside its turn. The plan modal gains a thread panel
   beside clickable block cards, whole-block selection only. Decision 4 short that trick.
5. **Built. The new-mission dialog and the assistant.** `Profile::mission_assistant: Option<bool>`
   folds in `flatten()` beside `max_subagents`, mirrored on `ProfileInfo` and a `Mission assistant`
   checkbox under `Purpose::Profile`; `state/new_mission.rs::assistants()` filters the picker to
   profiles ticked true, and `mission_briefing()` is the one place the opening turn's text lives.
   `start_new_mission()` composes the launch from existing messages — `CreateTask`, then on
   `TaskCreated` a `SetTaskField(Level::Mission)`, the description's `UpdateTask`, a
   `StartConversation` ticking `manage-ubiq-tasks` and `ubiq-plan`, and the briefing's `PromptAgent`
   — parked on `BoardState::pending_mission` the way `PendingTask` parks an ordinary draft, needing
   no wait between the two steps because the window mints the agent id and `launch_pending` launches
   on the first prompt. Built on the reading that `require plan` is a reminder the briefing states,
   not a gate. *Unlocks: the ask as written.*
6. **Built. The plan editor.** Decision 6 taken as recommended, native on the existing editor, and
   built **generic over a document rather than over a `TaskId`**: `crate::state::document`'s
   `DocumentEditor` and `DocumentHandle` are the surface, the plan is the handle's one variant, and
   `app/plan.rs`'s `impl DocumentHandle` is the only place a handle becomes a message. The window's
   `plan_editor` is an ordinary `EditorState` with Source/Split/Preview on the file viewer's own
   `ViewLayout`; `SavePlan` has its first caller (⌘S and Save), and an unsaved edit is never lost to
   a concurrent write — the surface reports the other copy moved and keeps what was typed.
   Annotated passages are one `TextDecorationCollection` painted from `block_ranges()`, which joins
   the host's block ids to buffer offsets by a forward scan; a click inside one opens its thread
   anchored by `range_to_bounds`; a selection in Source annotates with the passage as its quote, and
   the preview keeps whole-block granularity. `/` is `ui::editor::SlashCommands`, the tree's only
   `CompletionProvider`. **Not built: a second document.** Annotating an arbitrary project `.md`
   needs a store for its body and a sidecar for its threads, and a sidecar inside a user's git
   repository is a decision nobody has taken — the seam is the handle's second variant and nothing
   else. *Unlocks: the planning loop with a writing surface in it.*
   **Its proto and host half is built — edit provenance** (`D157`): a monotonic
   `revision`, a `SaveOrigin` fixed at the entry point, a per-line stamp rebased on every save into
   the same sidecar, `ListPlanChanges`/`PlanChanges`, and `ubiq-plan`'s `plan_changes`. Counted in
   lines, because a reworded block keeps its id; nothing in `crates/ubiq` reads it yet.
7. **Built, the profiles half.** A store per project under `<config root>/projects/<id>/profiles`,
   read with the global one as a `ScopedProfileStore`; written from the project's Integrations tab,
   deleted with the project. Decision 8 as recommended; `D158` rules `extends`. Attachments: the
   other half.

Slice 1 is one field, one enum, one setting and one card affordance. Slice 3 is the first one that
costs a week.

## Open items

Each of these moves on a short answer.

- **Does `require plan` do anything mechanical**, or is it a reminder? It could gate the mission out
  of `Done` until a plan exists, or it could be a flag the assistant reads in its briefing.
- **Does a child inherit anything from its parent** — labels, colour, session, assignee?
- ~~Where a project-scoped profile appears, and which way `extends` may point~~ — answered by
  `D158`: only inside its project, and outwards only.

## Related docs

- [`../features/workbench.md`](../features/workbench.md) — the tasks board, its cards, its task panel
  and the launch form this proposal extends
- [`kb.md`](./kb.md) — the knowledge base a task attachment would point into
- [`web-panel-phase6.md`](./web-panel-phase6.md) — the tenant machinery decision 6 would ride
- [`../tech/decisions.md`](../tech/decisions.md) — `D30`, `D113`, `D121`, `D138`
