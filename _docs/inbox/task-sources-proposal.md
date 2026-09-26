---
id: inbox-task-sources
title: Proposal — remote task sources, and Trello as the first one
kind: proposal
status: proposal
summary: A provider-agnostic layer that binds a project's tasks board to a board somewhere else — a neutral remote item, a provider trait behind it, a per-project link table, a poll that writes through the shared work handle as an MCP tool does, and one wire family for setup, import and sync — with Trello as the first provider and the whole thing in the base, because a trait with one implementation is a guess. Conflict is a per-binding authority switch rather than a merge, a sync indicator reports drift on the board and on each card, and a force button pushes or pulls one card or all of them. Every fork is a numbered decision with a recommendation and a cost.
read_when: you are designing how Ubiq talks to an external issue tracker, adding a second task provider, or deciding what a remote-backed task means on the board
updated: 2026-09-26
code_anchors: [crates/ubiq-proto/src/tasksrc.rs, crates/ubiq-host/src/tasksrc/mod.rs, crates/ubiq-host/src/tasksrc/store.rs, crates/ubiq-host/src/tasksrc/sync.rs, crates/ubiq-host/src/tasksrc/outbound.rs, crates/ubiq-host/src/tasksrc/service.rs, crates/ubiq-host/src/tasksrc/trello.rs, crates/ubiq/src/state/tasksrc.rs, crates/ubiq/src/app/tasksrc.rs, crates/ubiq/src/ui/tasksrc.rs, crates/ubiq/tests/tasksrc.rs]
depends_on: [feat-workbench, feat-connectors, tech-transport, tech-architecture, tech-decisions, inbox-editions, wip-kb]
---

# Proposal — remote task sources, and Trello as the first one

**Every piece carries a cost, every fork is a numbered decision R1–R16 with a recommendation, and
§9 stages the work so the expensive halves come last.** Slices 1–6 have landed.

**Slices 1–3 landed on 2026-09-26** — the neutral model, the provider trait and registry, the
binding and link table, the `tasksrc.toml` sidecar, the Trello client on recorded fixtures, and
Trello as the base's seventh connector provider (`D181`, `D182`, `D183`).

**Slice 4's inbound half landed the same day, as `D185`** — §3.5's worker, with `R9`'s two rules
living there rather than in any provider, each pinned by a test that runs the Trello filter over the
fixtures and asserts its *effect* (`crates/ubiq-host/tests/tasksrc_sync.rs`).

**Slice 5's interface half landed the same day, as `D187`.** §3.6's wire family exists and
`crates/ubiq/src/ui/tasksrc.rs` draws it: **one renderer over `ConfigField`, naming no provider and
matching no capability** — `ProviderCaps` greys through a table of `fn(&ProviderCaps) -> bool` rows
— plus the settings section, the import dialog, the card badge including the `Parked` one M6 filed
and nothing drew, the command and the status item. Test runs the filter and counts.

**Slice 6 landed the same day, as `D188`** — the outbound write, §4's conflict table over two
per-field hash maps, drift on the link row with its overview under the authority switch, and the
host's arms for the whole of §3.6 (`G364`, closed). The layer is complete but for slice 7.

Two things are being proposed at once, and the order between them is the point:

1. **A task-source layer** — provider-agnostic: a neutral remote item, a trait, a link table, a poll,
   one wire family, one settings surface, one import dialog, one badge.
2. **Trello as its first provider** — a real consumer, in the base, free, shipping with the layer.

They ship together because **a trait with one implementation is a guess and a trait with two is a
seam**. Building the layer against Trello and then fitting a second provider to it is the only order
in which the generality gets tested by something other than an argument. The alternative order —
design the layer for provider *n* and take Trello later — produces a trait shaped like provider *n*
with a Trello-sized hole in it.

## 1. Why the base

`features/connectors.md` already ships six authenticated identities and closes with the sentence this
proposal answers: *"A second consumer beyond cloning: a task source, or a document picker."* `G145`
says the same thing from the other side — **no connection is read by anything**, and a task source is
the named candidate.

Against editions' own test (`inbox/editions-proposal.md` §9 — a feature belongs to the paid edition
when it serves somebody other than the person at the keyboard), a Trello board is the developer's
own: personal boards, a two-person side project, a freelance client's list. It is the shape of
tracker somebody runs *for themselves*. That is a base feature, and §9's rider applies — no base
feature is degraded to make room for a paid one.

## 2. What exists, and two things the old reading got wrong

**The record already has the fields an imported task wants.** `TaskRecord` carries `key` — the
user's own id, `UBQ-123` or `#4711` — and `link`, one URL to the issue the task stands for, both
settable through `SetTaskField`. The key minter raises its floor past any `T-<n>` and ignores keys in
any other shape, so a remote key never collides with it. `steps` is the ordered checklist the MCP
surface already calls *todos*, each with a stable `StepId`; `comments` are records with a
host-stamped author (`D121`); `labels`, `assigned_to`, `kind`, `priority`, `level` and `parent` are
all there.

**The write path a sync worker needs already exists, and `D120` is it.** The coordinator wraps `Work`
in a cloneable `Handle(Arc<Mutex<Work>>)`; the MCP listener holds a clone; a tool takes the lock for
one method, releases it before posting, and the change goes out as `TaskCreated`/`TaskChanged`/
`TaskDeleted` to **every** window, because there is no asking client. A sync worker is the third
holder of that handle and needs no new mechanism. The host's metadata sampler is the periodic half's
template: a named thread, a sleep, a snapshot behind a mutex, a coordinator that never blocks on it.

**The HTTP is already here.** The connector family's `http` module gives `get_json` and `post_form`
over a blocking `ureq` agent with the certificate-pin verifier wired in, its `providers` module joins
an instance's base URL, `Store::token(provider, ConnectionId)` resolves a token out of the OS secret
store, and the repository family's `identity` does the connection-id → `(provider, instance, token,
pin)` resolution — **for one consumer, privately**. R3 is about that.

Two claims that get repeated and are **not true**:

- **"Nothing watches `tasks.toml` for a change made outside the process."** It does.
  `Coordinator::sync_tasks_due` re-reads every loaded project's `tasks.toml` every two seconds
  (`TASK_SYNC_EVERY`), compares the deserialised list, and broadcasts a whole `WorkList` to every
  window when it differs. That is also why a second window sees another window's edit at all —
  window-originated `TaskChanged` is `Reply::Asker`, not `Everyone`.
- **"A store decorator is the shape."** `inbox/editions-proposal.md` §7 sketches this row as *"a
  `Box<dyn TaskStore>` that reads through it"*. **The store trait rules that out.** `TaskStore` is
  whole-list — `load` hands back every task, `save` takes every task — so a decorator would have to
  re-derive on every save which records are remote, and a task the user deleted is indistinguishable
  from a task the remote has not yet returned. `Work` is memory-authoritative above it and only
  writes through, so the decorator sits below the layer where every interesting event happens.
  **R2** says the seam is one layer up.

**What does not exist is any notion of a task that came from somewhere.** No external id, no
revision, no last-synced marker, no importer. `link` is display-only and the base's own words say so.

## 3. The shape

Five parts. Nothing in the first four is Trello-shaped.

| Part | Where | What it is |
|---|---|---|
| The neutral model | a `tasksrc` module in `ubiq-proto` | `RemoteItem`, `RemoteLane`, `RemoteContainer`, `ProviderCaps`, and the ids |
| The provider trait | a `tasksrc` module in `ubiq-host` | `TaskProvider`, and the registry a boot fills |
| The binding and link table | the same module | Per-project config and the row per linked task |
| The sync worker | its `sync` submodule | The thread, the tick, the drift set, the writes |
| The provider | its `trello` submodule | Trello REST, and nothing that names a `TaskRecord` |

### 3.1 The neutral item

The layer's whole job is to make "a card on a board somewhere" one type. It carries what every
tracker has and nothing any one of them is proud of:

```rust
pub struct RemoteItem {
    pub id: RemoteItemId,          // opaque, provider-scoped
    pub key: Option<String>,       // what a human says out loud: "#42", "4711"
    pub url: String,
    pub lane: RemoteLaneId,        // a Trello list, an ADO state, a GitHub column
    pub title: String,
    pub body: String,              // markdown as the provider gives it, never parsed here
    pub labels: Vec<String>,
    pub assignees: Vec<String>,    // display names; `assigned_to` is free text (D-none, it always was)
    pub kind_hint: Option<String>, // the provider's own word for the type, unmapped
    pub priority_hint: Option<String>,
    pub parent: Option<RemoteItemId>,
    pub checklist: Vec<RemoteCheckItem>,
    pub comments: Vec<RemoteComment>,
    pub revision: Revision,        // opaque version token — see R8
    pub updated_at: DateTime<Utc>,
}
```

**Every mapping is a string keyed onto a closed Ubiq set**, and the provider supplies the candidate
strings by listing them (`lanes()`, `facets()`). That one decision — R5 — is what lets a Trello list,
an ADO state and a GitHub column all be "the lane" without the layer knowing which it is holding.

### 3.2 Capabilities, not conditionals

Providers differ in what they *can* do, not only in how: Trello has checklists and a work-item
tracker generally does not; Trello has no conditional write and a revisioned tracker does; a tracker
may have a query language and Trello has none.

`ProviderCaps` is a flag set the provider answers with — `write_lane`, `write_title`, `write_body`,
`write_labels`, `write_assignee`, `comments_read`, `comments_write`, `checklist_read`,
`checklist_write`, `conditional_write`, `server_query`, `hierarchy` — and **the interface greys out
what the bound provider cannot do rather than failing when it is asked**, through a table of
`fn(&ProviderCaps) -> bool` rows rather than a branch per flag (`D187`). That is the mechanism
keeping provider specifics out of the core, and why §7's second-provider exercise is cheap.

### 3.3 The trait

Blocking, on the sync thread, because every call is already on a thread of its own and an async
runtime would be a second scheduler for nobody — `ubiq-host`'s existing reasoning for `ureq`.

```rust
pub trait TaskProvider: Send + Sync {
    fn id(&self) -> &'static str;              // "trello"
    fn caps(&self) -> ProviderCaps;
    fn config_schema(&self) -> &'static [ConfigField];   // R6

    fn containers(&self, who: &Identity) -> Result<Vec<RemoteContainer>>;   // boards
    fn lanes(&self, b: &Binding) -> Result<Vec<RemoteLane>>;                // lists
    fn facets(&self, b: &Binding) -> Result<Facets>;                        // one call: labels, members, lists

    fn query(&self, b: &Binding) -> Result<Vec<RemoteItem>>;                // everything under the filter
    fn fetch(&self, b: &Binding, ids: &[RemoteItemId]) -> Result<Vec<RemoteItem>>;
    fn create(&self, b: &Binding, draft: &RemoteDraft) -> Result<RemoteItem>;
    fn update(&self, b: &Binding, id: &RemoteItemId, patch: &RemotePatch,
              expect: &Revision) -> Result<RemoteItem>;
    fn comment(&self, b: &Binding, id: &RemoteItemId, text: &str) -> Result<RemoteComment>;
    fn set_check(&self, b: &Binding, id: &RemoteItemId, item: &RemoteCheckId,
                 done: bool) -> Result<()>;
}
```

`RemotePatch` is one `Option` per writable field: absent means *leave alone*, present means *set* —
`SetTaskField`'s own discipline, for the same reason it exists there.

### 3.4 The binding, and where it lives

One binding per project to start with (R4). It holds the connection id, the container, the lane map,
the kind/priority maps, the filter, the direction, the authority switch and the interval.

Storage is the knowledge base's convention exactly: **one versioned TOML file per project**, written
atomically, missing read as empty rather than as an error —
`<config root>/projects/<project ulid>/tasksrc.toml`. Never in `tasks.toml`: `TaskRecord` has no
unknown-field bag and the file is rewritten whole on every save, so a field added beside the record
is dropped by the first writer that does not know it (R1).

The **link table** is the same file's second half, one row per linked task:

| Column | Why |
|---|---|
| `task` (`TaskId`) | Ubiq's side |
| `item` (`RemoteItemId`), `container` | the remote's side |
| `revision` | what the last successful read saw |
| `hash` | **per field**, not per record — the dirty test in §4 needs field granularity |
| `synced_at`, `state` | `Linked` / `Drifted` / `Conflict` / `Unlinked` |

### 3.5 The worker

One named thread for the host, not one per binding: it walks the bound projects, ticks each on its
own interval, and does the whole of one binding's pass before starting another. It holds a `Handle`
clone and writes through `Work` exactly as an MCP tool does (`D120`), so every window redraws and an
imported task is an ordinary task from the moment it lands. **The lock is taken for one method and
never across a network call** — that rule is the whole mitigation for `D120`'s stated cost.

**Built, as `tasksrc/sync.rs` (`D185`).** The thread is `ubiq-tasksrc`, it holds no catalogue, and
it pulls **only the fields the remote changed**, by the link row's per-field hash — which is what
lets a local edit survive a pass, and the ground §4 is on.

### 3.6 The wire

A new family, because the transport contract's own picking rule sends it there: this names a
connection *and* a project *and* a piece of remote work, and the `work` family is project-local
tasks while the `connector` family is identity with no binding. It follows the `repository` family's
shape — the asker mints a query id, a reply naming an id the UI no longer holds is discarded.

| UI → host | Answers with |
|---|---|
| `ListTaskProviders` | `TaskProviders { providers: Vec<ProviderInfo> }` — id, label, caps, config schema, connector family (`D189`) |
| `ListRemoteContainers { query, connection }` | `RemoteContainers` |
| `ListRemoteLanes { query, binding }` | `RemoteLanes`, `Facets` |
| `TestTaskSource { query, binding }` | `TaskSourceTest { count, sample: Vec<RemoteItem> }` — the preview, R7 |
| `GetTaskSource { project_id }` / `SetTaskSource { project_id, binding }` | `TaskSource { project_id, binding, state }` |
| `ListRemoteItems { query, binding }` | `RemoteItems` — the import dialog's list, linked ones marked |
| `ImportRemoteItems { project_id, items }` | `TaskCreated` ×n to everyone |
| `SyncTaskSource { project_id, scope }` | `TaskSourceState`, then `TaskChanged` ×n |
| `ResolveTaskDrift { project_id, task_id, side }` | `TaskChanged`, `TaskLinkChanged` |
| `UnlinkTask { project_id, task_id }` | `TaskChanged` |

| Host → UI, unsolicited | What it says |
|---|---|
| `TaskSourceState { project_id, state, last_sync, drifted: Vec<TaskId>, error }` | the indicator |
| `TaskLinkChanged { project_id, task_id, link: Option<TaskLink> }` | the per-card state |
| `TaskSourceError { query, error }` | a sentence, per `WorkError`'s reasoning |

`scope` is `All` or `Task(TaskId)` — the same message serves the board's force button and a card's.
No variant carries a pane id, so `pane_id_of` is untouched.

**Built** — the interface half as `D187`, the host's arms and `ResolveTaskDrift` as `D188`.
[`tech/transport-contract.md`](../tech/transport-contract.md) owns the facts from here.
`ListRemoteItems` grew a `project_id`: only a project has link rows.

## 4. Conflict — a switch, not a merge

**Built, as `D188`**, which owns the settled table and the five rules that qualify it.

**The rule is one switch per binding: _Ubiq wins_ or _the remote wins_.** It is consulted **only when
both sides changed**, which is the case the switch exists for; in every other case there is nothing
to decide.

Each field of each linked task is in one of four states, derived by comparing the task, the fresh
remote item and the link row's stored per-field hash:

| Local changed | Remote changed | What happens |
|---|---|---|
| no | no | nothing |
| no | yes | **pull** — the remote value is written to the task |
| yes | no | **push** — the local value is written to the remote |
| yes | yes | the switch decides, the loser's value is kept in a comment, the task is marked `Drifted` |

Three rules make that safe:

- **Before any write, fetch.** A push is always preceded by a read of the item it is about to change,
  so the switch is being applied to the remote's current state and not to a cached one.
- **Where the provider has a conditional write, use it.** The stored `revision` goes as the
  precondition; a rejection means somebody wrote between the fetch and the push, and the pass
  restarts for that item rather than overwriting. `ProviderCaps::conditional_write` says whether this
  is available — see R8 for what it costs when it is not.
- **A refused write is never retried by overwriting.** It becomes `Conflict`, it says so on the card,
  and a human decides.

The user may override the switch per task — a card whose local edit must not be lost is pinned to
*Ubiq wins* whatever the binding says.

### The indicator, and the force buttons

**On the board**: one status item — the state, the last sync time, a count of drifted tasks, the
failure when there is one, and a click that runs a pass (built, `D187`). Clicking it will open **the
drift overview**: one row per task that differs, naming the field, the two values and which way the
switch would settle it, with **Push all** / **Pull all** and a per-row choice. That overview is the
answer to "what changed" and it is what makes a force button safe to press.

**On a card**: a badge beside the provider chip — parked, drifted, conflicted, unlinked, and nothing
at all when the card is merely in step (`D187`) — with the fields that differ on its hover and a
**Sync now** that runs one item's pass through `fetch` (`D188`).

## 5. Trello, concretely

Trello is chosen as the first provider because it is free, personal, ubiquitous, and **weak in
exactly the places a good abstraction must survive**: no query language, no work-item type, no
priority, no conditional write. A layer that fits Trello fits a richer tracker by relaxing.

| Ubiq | Trello | Notes |
|---|---|---|
| `status` (7 lanes) | a list on the board | Many-to-one map; an unmapped list parks the task (R9). A fresh binding is seeded `To Do → Backlog`, `Doing → InProgress`, `Done → Done` by list *name* — a board nobody has bound has no ids to match against — and the stored map is id-keyed from then on. The other four Ubiq lanes start unbound |
| `key` | `#{idShort}` | |
| `link` | `shortUrl` | `issue_provider` does not recognise `trello.com` today — it falls through to the generic chip. One arm and one glyph closes that |
| `title` / `description` | `name` / `desc` | Trello's `desc` is markdown; the host parses neither |
| `labels` | card labels | Trello labels are board-scoped and coloured; `Label.colour` is a swatch index, so the colour is mapped, never carried |
| `kind` | a label, by convention | Trello has no type field. The kind map is label → `Kind`, and the default is empty |
| `priority` | nothing | No native field. Unmapped unless the user maps labels to it |
| `assigned_to` | first member's full name | Read only: `write_assignee` is **off**, because a member is an account and free text is not one (`D188`) |
| `steps` | **the card's first checklist** | R10 |
| `comments` | `commentCard` actions | Author is stamped by the host, never carried from Trello (`D121`) |
| `level`/`parent` | nothing | Trello has no hierarchy. `hierarchy` cap is off |

What the API gives, and what each fact costs:

- **Auth is an API key plus a token**, sent as `Authorization: OAuth oauth_consumer_key="…",
  oauth_token="…"`, pasted from a `trello.com/1/authorize` page rather than a callback. R3 places it.
- **No query language.** `GET /1/boards/{id}/cards` returns the board's open cards; **filtering by
  label, member and list is client-side**. Fine for a board, wrong for a tracker with 100 000 items
  — which is why `ProviderCaps::server_query` exists and why R7's preview is a real count.
- **No ETag and no `If-Match`.** `dateLastActivity` is the only version token, it moves on any
  action, and comparing it is a check in *our* client, not a precondition on theirs. R8.
- **Rate limits are 300 requests per 10 seconds per key and 100 per token.** A pass is one card
  list plus, per reconciled card, a checklist and an actions call — so the budget is spent per
  *linked* card. `GET /1/batch?urls=` takes ten at a time and is the lever when it matters.
- **Webhooks need a publicly reachable callback URL**, which a desktop application does not have, so
  the layer polls for every provider — worth stating once rather than rediscovering per provider.

## 6. The interface

All of it is base code in existing containers. **Built as `D187`** in `crates/ubiq/src/ui/tasksrc.rs`,
bar the detail-panel section and the drift overview, which wait on the drift work they show.

| Surface | Where | What |
|---|---|---|
| Settings section | the project dialog, contributed through `D180`'s container | Provider, connection, board, lane map, kind/priority maps, the rendered filter, direction, authority switch, interval, **Test** |
| Import dialog | an overlay over the board or the page | Runs the filter, lists what comes back, marks what is already linked, imports the picks |
| Card badge | `board/mod.rs::shape_line` | Parked, drifted, conflicted or unlinked, and nothing when in step |
| Detail panel section | `board/detail.rs` | The link, its state, its differences, **Sync now**, **Push**, **Pull**, **Unlink** |
| Status item | the board's header | Last sync, spinner, drift count, failure; opens the drift overview |
| Command | the command line | *Import tasks…*, *Sync tasks now* |

The detail panel is beside the columns rather than a modal, so the per-card half needs no overlay.

## 7. What a second provider costs

The trait is only worth its weight if fitting a second provider is small. What each would force:

| Provider | What it needs that Trello does not | Does the layer already hold it? |
|---|---|---|
| A work-item tracker (Azure DevOps, Jira) | A query language in the filter; a real type field; a revision precondition; an epic/story hierarchy | `ConfigField::Text` + `TestTaskSource` for the query (R6); `kind_hint` is already the type; `conditional_write` is already the cap; `parent` + `Level::Mission` already exist, depth capped at one |
| GitHub / GitLab issues | Milestones and a project column that are two different lane candidates; issue state closed/open orthogonal to the column | `lanes()` chooses which dimension is the lane; the second one maps onto labels |
| Anything with attachments | Files | Not held. `Attachment` is a reference to a project-relative path or a `kb:` target, and a remote file is neither. Out of scope, and named so |

**The registry entry point a provider shipping outside this repository needs is built** (`D189`):
`Boot.contributions.task_providers` is the `tasksrc::Registry` itself, seeded with Trello, handed to
`coordinator::start` before the first window. A single seam with a base-side user from its first
commit — `inbox/editions-proposal.md` §13's fourth rule — and a far better test of §9's exit
criterion than a one-off feature: **two implementations prove the shape; one never can**, and with
Studio's Azure DevOps and ClickUp registered through it there are three.

## 8. Decisions

**R1 — Sync state lives in a per-project sidecar, never in `tasks.toml`.** *Recommended:*
`projects/<ulid>/tasksrc.toml`, versioned, atomic, missing-is-empty — `kb.toml`'s convention.
`TaskRecord` has no unknown-field bag and the task file is rewritten whole, so anything added beside
a record is dropped by the next writer. **Cost:** a task deleted outside the layer leaves an orphan
row, so the worker prunes on load.

**R2 — The worker writes through the shared work handle, as an MCP tool does. No store decorator.**
*Recommended*, for §2's reasons. **Cost:** the worker takes the lock the coordinator wants, so a slow
write stalls a click — `D120`'s own accepted cost, bounded by never holding the lock across a
network call.

**R3 — Trello becomes a seventh connector provider, with a pasted key-and-token secret.**
`features/connectors.md` says the provider list is closed at six and *"a user cannot add a seventh"*
— which is a rule about users, not about the base. Every consumer takes a `ConnectionId` rather than
a provider name, the token belongs in the secret store under `connector:trello`, and connectors' own
next step asks for exactly this consumer. *Recommended,* with two riders: the secret is a **pair**
(key and token), which no existing provider's `secret_prompt` shape has, and Trello's auth is a
`OAuth oauth_consumer_key=…` header rather than a bearer, so `connectors/http.rs` grows a
per-provider auth renderer. *Also recommended:* lift `repos/mod.rs::identity` into the connector
family, since this makes it the second caller. **Cost:** three small edits to a settled, working
family, and a user who pastes two strings instead of one.

*The alternative* — the layer holds Trello's credentials itself — is rejected: it puts a secret
outside the store that exists for secrets and gives the task source a second, private notion of
identity.

**R4 — One binding per project to start.** *Recommended.* Two boards on one Ubiq board is a real
want and a real cost: every message grows a binding id, the link table grows a second key, and the
lane map stops being a map. **Cost:** the table and the messages should be written binding-keyed
from the first commit even while the setup surface allows only one, so that the second is a UI
change and not a file migration.

**R5 — Every mapping is a string keyed onto a closed Ubiq set, with the provider supplying the
candidate strings.** *Recommended.* Lanes, kinds, priorities and labels are all the same mechanism,
and the layer never learns what a Trello list is. **Cost:** a remote value renamed on the remote
falls out of the map and the task parks (R9) rather than following the rename; matching by id where
a provider has stable lane ids avoids it, and Trello's list ids are stable.

**R6 — Provider-specific configuration is a declared field schema the base renders, not a view the
provider draws.** `&'static [ConfigField]` — `Text`, `Secret`, `Choice(from a `facets()` call)`,
`MultiChoice`, `Bool` — with an optional **Test**. A query language is a `Text` field with a test; an
iteration picker is a `Choice`. *Recommended,* because it is the single decision that keeps every
provider's UI out of the interface crate. **Cost:** a provider whose configuration does not fit
cannot ship until the schema grows — bounded, and visible as a missing field type rather than a
leak. **Held so far** (`D187`): nine field declarations across Trello and ADO, including ADO's area
tree flattened to a flat `Choice`, and no renderer branch on a provider's name.

**R7 — The filter preview runs the filter; no client-side query parser, ever.** *Recommended:* a
**Test** button that runs the query read-only and reports the count and the first handful of titles.
Where a provider has a query language it validates it server-side and returns a usable error; a
second grammar in our tree would be a language we do not own, kept in step by hand. **Cost:** a test
costs a round trip and a rate-limit slot. It is also strictly more truthful than a syntax check,
because it proves the query returns what was meant and not merely that it parses.

**R8 — A conditional write is used where it exists and emulated where it does not, and the gap is
stated rather than hidden.** *Recommended:* with `conditional_write`, the stored revision is the
precondition and a rejection restarts the item. Without it — Trello — the worker re-reads
immediately before the write and compares `dateLastActivity`, which closes the window to the
round-trip but does not close it. **Cost:** on Trello a write inside that window silently wins.
Say so in the settings section next to the switch; do not pretend the two providers are equally safe.

**R9 — An unmapped lane parks the task; a filter losing an item unlinks it; nothing deletes
anything.** *Recommended.* A task whose remote lane maps to nothing stays where it is with a badge
saying so. An item that leaves the filter keeps its task, its content and its history, loses its link
and says so. Deleting a task in Ubiq never deletes the remote item, and deleting the remote item
unlinks rather than deletes. **Cost:** a board whose filter drifts accumulates unlinked tasks, so the
overview needs a bulk act on them.

**R10 — The task's steps map to the card's first checklist.** *Recommended.* Ubiq has one ordered
checklist and Trello has many; the first is the only unambiguous choice, and a second on the card is
left alone rather than merged or deleted. Steps are matched by text, not by position,
because `StepId` and Trello's check-item id are two different things and the link row holds the pair.
**Cost:** two steps with the same text cannot be told apart, and the pass leaves both alone.

**R11 — `key` and `link` are written from the remote and are not the sync identity.** *Strongly
recommended.* They are the user's fields, free text and clearable; the identity is the link table.
**Cost:** two records of one fact, which can disagree — correctly: clearing `link` is not unlinking.

**R12 — Import is explicit; sync is what happens afterwards.** *Recommended:* nothing appears on the
board that a person did not pick, and the filter governs what is *offered*, not what is *created*.
The alternative — a filter that materialises every match — is a per-binding *Auto import* switch,
default off. **Cost:** a user who wants the whole board on it turns a switch on, one time.

**R13 — Outbound writes are off until the user turns them on.** *Recommended:* the direction control
is `Pull only` / `Two-way`, defaulting to `Pull only`. A first connection that silently starts
writing to somebody's shared board is the failure mode that loses trust in the whole feature.
**Cost:** one more control, and one more state to explain.

**R14 — The poll interval is per binding, with a floor.** *Recommended:* default five minutes, floor
one minute, and a pass never overlaps itself. **Cost:** a change made elsewhere takes up to an
interval to show, which the force button is for.

**R15 — The MCP surface grows two tools on the existing task server, not a new server.**
*Recommended:* `sync_task` and `import_tasks` on `manage-ubiq-tasks`, so a hosted agent can pull a
card before it starts work. The catalogue is a static table (`G7`, `G229`) and this is inside it.
**Cost:** none worth naming; still deferred.

**R16 — Build the layer and Trello together; fit the second provider afterwards.** *Recommended*, for
the reason in the preamble. **Cost:** Trello's weakness shapes the first cut of the trait, so the
second provider will move something — slice 7's job, and cheaper than guessing it right first.

## 9. Staging

| # | Slice | Worth having on its own? |
|---|---|---|
| 1 | **Landed.** The neutral model, the trait, the registry, the link table, the sidecar — with a fake provider and no network | No, but it is the one slice that decides everything |
| 2 | **Landed.** The Trello client: containers, lanes, facets, query, fetch — headless, tested against recorded responses | No |
| 3 | **Landed.** The connector provider row and the key/token flow (R3) | No |
| 4 | **The worker landed.** Inbound sync end to end, driven by a test rather than a button. The wire family beside it has not | **Yes — this is the first slice that is worth having** |
| 5 | **The interface half landed** (`D187`): the `ConfigField` renderer, the settings section, the import dialog, the card badge, the command and the status item. The host's arms for the family are slice 6's | Yes |
| 6 | **Landed** (`D188`). Outbound write, the authority switch, drift detection, the overview, the force buttons, and the host's arms for §3.6 | Yes |
| 7 | Comments and the checklist (R10); then a second provider against the frozen trait | Yes, and slice 7 is what proves slice 1 |

A client that can only be tested against a live board will not be tested — slice 2's recorded
responses are not optional, and the same fixture set is what a second provider's tests copy.

## 10. Related docs

- [`features/connectors.md`](../features/connectors.md) — the identity this authenticates through,
  why it names no board, and the second consumer it asks for
- [`features/workbench.md`](../features/workbench.md) — the board, its seven lanes, the card, the
  detail panel and the `link` chip this extends
- [`tech/transport-contract.md`](../tech/transport-contract.md) — the family-picking rule §3.6
  follows and the query-id discipline it copies
- [`tech/decisions.md`](../tech/decisions.md) — `D120`, the write path §3.5 is; `D121`, the comment
  author rule; `D113`, why a label has no registry
- [`inbox/editions-proposal.md`](./editions-proposal.md) — §7's row this reshapes, §9's test §1 runs,
  and §13's fourth rule §7's registry answers

## Next steps

- Verify the Trello endpoints, the auth header and the rate limits against a live board. The client
  and its fixtures are written against the documented shapes and nothing has met a real board
- Slice 7: comments and the checklist (`R10`), then a second provider against the frozen trait.
  These are the two fields the conflict table deliberately leaves out, because a provider may fill
  them on one endpoint and not another (`T-237`)
