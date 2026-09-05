---
id: inbox-editions
title: Proposal — two editions, one architecture
kind: proposal
status: proposal
summary: Ubiq split into a source-available base under the Sustainable Use License and a closed Pro edition in a second repository — the composition root moved out of the binary so a second binary can compose the same halves with more in them, three registration points that each already have a base-side user, one opaque message family for everything a Pro feature says, and the rule that keeps the base working with nothing registered.
read_when: you are deciding what belongs in the base repository and what belongs in Pro, how a closed feature reaches into the interface or the host, or how the two are developed and built together
updated: 2026-09-04
depends_on: [tech-architecture, tech-transport, tech-structure, tech-agent-manager, feat-workbench, inbox-routing]
---

# Proposal — two editions, one architecture

Ubiq is one workspace, five crates and one binary, licensed under the Sustainable Use License with a
contributor agreement already in place. This proposes the second half of that arrangement: a **Pro
edition** in a second, closed repository, which composes the same crates and adds features the base
does not have — integrations with issue trackers, single sign-on, central account registration,
encryption at rest, team features — and later hosts a runtime plugin registry the base never learns
about.

The load-bearing claim is that **this needs almost no new architecture**, because the split the base
already draws for other reasons is the split an edition needs. `crates/ubiq-app` is the only crate
that names both halves; make it a library with a three-line `main.rs` and a second binary can name
both halves with more in each. The bus already carries an opaque blob the host does not parse, and
one more family of those is all a closed feature needs to say. The stores are already boxed traits
with two implementations each, which makes encryption at rest a decorator rather than a feature.

What is genuinely missing is three registration points, one message family, and a discipline:

> **An extension point with no base-side user is not built.** A trait with one implementation, all
> of it in the closed repository, is a plugin framework nobody uses — and it is exactly the shape
> this document is at risk of becoming. Every seam below either exists today, or is introduced by
> moving something the base already does through it.

There is one deliberate exception, named in §5.

## 1. Where it stands

**The workspace is flat and unversioned.** Six members, no `[workspace.dependencies]`, no `[patch]`,
no `[workspace.package]`; every crate `0.1.0`, `publish = false` — except `crates/agent-manager`,
which has no `publish` key at all.

**The licence is settled.** `LICENSE` is the Sustainable Use License 1.0, the four Ubiq crates point
at it with `license-file = "../../LICENSE"`, and `CONTRIBUTOR_LICENSE_AGREEMENT.md` gives the project
permission to license contributions on any terms — the clause the whole of §12 rests on. Two things
sit outside it: `vendor/gpui-terminal` keeps its upstream `MIT OR Apache-2.0`, and
`crates/agent-manager` is `MIT` with a licence file of its own, because it is a standalone library
Ubiq embeds rather than a part of Ubiq.

**Four seams exist and are proven.** The store traits in `crates/ubiq-host/src/store/mod.rs` —
`ProjectStore:70`, `TaskStore:77`, `PreferenceStore:86`, `SettingsStore:98`, each `Send + Sync`, each
boxed into its subsystem by `Stores::files` in `crates/ubiq-app/src/lib.rs`, each with a file and a
memory implementation. `agent_manager::harness::Harness` (`harness/mod.rs:456`), resolved by string.
`IoBridge` and `AgentInputSink` (`io/model.rs:758,769`), already substituted by
`tests/conversation.rs:72`. And `agent_manager::registry`, a trait precisely so an embedder can back
the skill and MCP catalogue with a database or a remote service, with `source::Source` making the two
indistinguishable to the provisioner.

**The interface has two of its own — the prior art §4.2 copies.** `ui/kit/mod.rs:12`'s
`Action = Rc<dyn Fn(&mut Window, &mut App)>`, and `ui/dock/skin.rs:71`'s `NewPane`, a struct of five
such closures handed *across* a seam to the component library. Nothing else in `crates/ubiq` is
dynamic: no handler registry, no plugin list, no trait-object screen table.

**Two blobs already cross the bus unparsed.** `SetPreferences`/`Preferences` carry a `String` under
`Scope::Interface` or `Scope::Project`, and `SettingsLayer::Ui` is the same arrangement: the schema
is the interface's, the host stores and returns it. And rule 6's workarea is a directory the host
names and never looks inside.

**Four closed sets are what a Pro feature would otherwise have to fork.**

| Closed set | Where | Shape today | Cost of a new member |
|---|---|---|---|
| `Message` | `crates/ubiq-proto/src/messages.rs:23` | One flat `#[serde(tag, content)]` enum, 88 variants, 10 families that are comment banners | One arm in `coordinator.rs:383`'s single `match`, which runs to `:816` |
| `RailMode` | `crates/ubiq/src/state/workbench.rs:29` | `Copy` enum, 8 variants, `Serialize`/`Deserialize`, with `label`, `note`, `groups`, `is_ide` and `ui/rail.rs:22`'s icon | Six or seven edits across five files. One screen-dispatch match, `ui/dock/mod.rs:451`; three `==` comparisons in `ui/status_bar.rs`; and it is a `HashMap` key in `ViewPrefs::modes` |
| `PanelKind` | `crates/ubiq/src/state/dock.rs:81` | Enum with `name() -> &'static str` (`"ubiq.logs"`, …), `from_name`, `class`, `home`, `is_drawn` | Five small matches, and a saved layout that cannot rebuild an unknown name already drops it |
| `Palette` | `crates/ubiq/src/theme.rs:138` | Seven `Copy` structs of colour fields, behind a thread-local `Theme::current()`, with ~30 zero-argument accessors and 30 `pub const f32` sizes beside them | A field, an accessor, and both of `dark()`/`light()`. `ThemeId` is a closed pair and `palette_for` is a match, so a third palette cannot be registered either |

The dock is already almost open — a panel is a namespaced string plus a payload, and `from_name`
returning `None` for a name this build cannot rebuild is behaviour the base needs for its own
terminals. That is the model the other two should copy.

**And `AppState` is one struct of about eighty fields in a 7083-line file** (`app.rs:283-471`),
thirty of them component-library entities initialised in `for_project` at `:479`. This decides
§4.2's shape, because **a closed screen cannot add a field to `AppState`** — so a registered screen
owns its state, which is an improvement the base wants for its own reasons.

**The coordinator names an unhandled message rather than dropping it** (`coordinator.rs:811-816`
warns and does not reply), and `mcp_server.rs` is an eight-line stub whose doc comment is a TODO.
Both matter below.

## 2. The shape

Two repositories, three tiers of extension, and one binary per edition.

| | Base — `ubiq` | Pro — `ubiq-pro` |
|---|---|---|
| Licence, visibility | Sustainable Use License 1.0, public | Proprietary and private, naming the base's licence |
| Holds | Everything today: four crates, `agent-manager`, the vendored terminal | Three crates — a host half, an interface half, a binary |
| Names the other | Never, and nothing in it is conditional on Pro | Always: it depends on the base's crates |
| Ships | `ubiq` | `ubiq` — a different bundle, the same config root (§11) |

The tiers say how much of this is work.

**Tier 0 — the seams that exist.** Encryption at rest is a `Box<dyn PreferenceStore>` wrapping
another; central account registration is an `agent_manager::registry` implementation and a
`credentials::SecretStore`; a remote task source is a `Box<dyn TaskStore>`. None needs a line of new
base code — the composition root chooses the implementation, and §3 is all that has to change.

**Tier 1 — three registration points, this document's work.** A Pro feature that needs to *appear* —
a screen, a panel, a command — or to *run in the host* — a thread, a socket, a poll of a remote API
— has nowhere to attach today. §4 is those points.

**Tier 2 — runtime plugins, later and entirely Pro's.** A tier-1 extension is a name, an opaque
payload and a rendered surface, so a runtime plugin is one whose implementation is loaded rather than
linked. The base gains no loader, no ABI and no sandbox for one. §8.

## 3. The composition root

**Landed on 2026-09-04 (`3c51380`), as a move with no behaviour change.** `main()` was 246 lines
doing six things — config root, stores, host, component library and palette, quit, first window — all
six the same in both editions. So the file became two:

```rust
// crates/ubiq-app/src/lib.rs — the boot, and the only crate that names both halves.
pub struct Stores { projects, preferences, tasks, settings }  // the four boxed store traits
impl Stores { pub fn files(root: &Path) -> Stores { /* the Box::new the boot made inline */ } }
pub struct Boot { pub stores: Box<dyn FnOnce(&Path) -> Stores> }  // an edition may wrap them
impl Default for Boot { /* Stores::files */ }
pub fn run(boot: Boot) { /* the former main(), verbatim, and its private helpers */ }

// crates/ubiq-app/src/main.rs, entire.                    // pro: crates/ubiq-pro-app/src/main.rs.
fn main() { ubiq_app::run(ubiq_app::Boot::default()) }     // ubiq_app::run(ubiq_pro::boot())
```

`Cargo.toml` gained `[lib] name = "ubiq_app"` beside `[[bin]] name = "ubiq"`; `stores` is a
`FnOnce(&Path)` because the config root is resolved inside `run`. **`Boot` has one field, not
three:** `features` and `contributions` await `Feature` and `Contribution` — the rule above, that an
extension point with no base-side user is not built. §14's phases 4 and 2 add them.

Three properties follow, and each is a requirement. **The base works out of the box because
`Boot::default()` is the base** — not a reduced configuration, not a mode; a base feature that only
works because something was registered is a bug `just verify` catches by never registering anything.
**Pro is a second startup project that invokes the base one**: it cannot re-implement or skip a step
of the boot, because `run` is the whole sequence and Pro's only input is the value it hands in.

**And Pro mirrors the base's crate split, so the base's boundary checks keep their meaning.**
`ubiq-pro-host` may name `ubiq-host` and never `ubiq`; `ubiq-pro` may name `ubiq` and never
`ubiq-host`; `ubiq-pro-app` names both, exactly once, in the line above — and Pro's own `just host`
and `just ui` are the two `cargo tree` greps from `Justfile:48-57`. A closed edition that broke
architecture rule 1 is one nobody could later split into two processes, which is the reason the rule
exists.

`Stores` is tier 0 made reachable: the four boxes the boot built inline, lifted into a struct so Pro
can hand in a wrapped `PreferenceStore`. No new trait, no indirection — and the boot is testable, by
`an_edition_can_hand_in_stores_of_its_own`, which hands in the memory stores.

## 4. What an extension is

Two traits, one per half. Neither names the other's.

### 4.1 A host feature

```rust
// crates/ubiq-host/src/feature.rs
pub trait Feature: Send {
    /// `vendor.name`. Every message name, directory and preference key this feature owns is
    /// under it, and the coordinator checks that prefix rather than trusting it.
    fn id(&self) -> &'static str;
    /// Once, before the first window: where a feature spawns its thread or opens its socket.
    fn start(&mut self, ctx: &Ctx) {}
    /// One `Extension` message addressed to this feature.
    fn said(&mut self, name: &str, payload: &str, ctx: &Ctx) -> Vec<Reply>;
}

impl Ctx {
    /// `extensions/<id>/`, made before `start`. Nothing else in the host reads inside it.
    pub fn dir(&self) -> &Path;
    /// What the catalogue says about one project — the wire record, cloned, not a handle.
    pub fn project(&self, id: ProjectId) -> Option<ProjectSnapshot>;
    /// For what a feature says unasked. Clonable, so its thread keeps one.
    pub fn everyone(&self) -> Mailbox;
}
```

Three methods and three accessors, and it is that small because **a Pro host feature is compiled
in**: it needs no API to spawn a thread or open a connection, only its messages, a way to answer, and
somewhere to write. Anything richer has grown into a subsystem, and a subsystem in Pro is a module of
`ubiq-pro-host` it owns outright. `Reply` is `reply.rs`'s `{ Asker, Everyone }`; dispatch gains one
arm before the fallback at `coordinator.rs:811`, which keeps its job of warning about a message
naming a feature this build lacks.

### 4.2 An interface contribution

A rail mode's state is per window, because `AppState` is per window — so what is registered is a
descriptor with a **factory**, not a screen, and `Box<dyn Screen>` is the only way a closed screen
can hold state at all.

```rust
// crates/ubiq/src/ext.rs
pub trait Contribution {
    fn id(&self) -> &'static str;                 // `vendor.name`, the same namespace
    fn screens(&self) -> Vec<ScreenSpec> { vec![] }   // and panels(), and commands()
}

pub struct ScreenSpec {
    pub id: &'static str, pub label: &'static str, pub icon: IconName,
    pub group: RailGroup,                         // APP or PROJECT, `RailMode::groups()`'s two
    pub build: fn() -> Box<dyn Screen>,           // once per window
}

pub trait Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement;
    /// The pair `inbox-routing` §2 asks of every screen: answer both and you are linkable,
    /// bookmarkable and in the history; answer neither and you are a page.
    fn locate(&self) -> Option<Destination> { None }
    fn reveal(&mut self, item: &str, locus: Option<&Locus>) {}
}
```

`PanelSpec` is the same idea over `PanelKind`'s five functions — `name`, `class`, `home`, `is_drawn`
and a `build` — and is the cheapest of the three, because `dock.rs` is already written this way. The
three closed sets each gain one variant, and none loses a derive it has today — `RailMode` and
`Destination::view` keep `Copy`, and `PanelKind` never had it, because it holds a `File(String)`:

| Set | New variant | Why it stays cheap |
|---|---|---|
| `RailMode` | `Extension(&'static str)` | The id is `'static` because it comes from a registered spec, so `Copy`, `Eq` and `Hash` all survive. `label`, `note`, `icon` and `groups` consult the registry for this arm and are unchanged for the other eight |
| `PanelKind` | `Extension(&'static str)` | `name()` returns the id, and `from_name` returning `None` for an unknown one is behaviour the base already relies on for its terminals |
| `Destination::view` | `Extension { id, item }` | `inbox-routing` §2's table gains a row and its router gains no knowledge, because it already dispatches by calling the screen's own pair |

**One fiddly detail, of the kind found late.** `RailMode` is a `HashMap` key in `ViewPrefs::modes`,
so it must serialise as a bare string — a newtype variant serialises as `{"Extension":"…"}`, not a
legal JSON map key — and `&'static str` cannot be deserialised from an owned document at all. One
hand-written serde pair answers both: out as `"ide"` or `"ext:vendor.name"`, in through the registry,
which turns a string into the `'static` id it registered or into nothing for a mode this build lacks.
The eight built modes keep their present text, so no existing blob moves.

**This is why `inbox-routing` phase 1 should land before the screen registry.** Its `where`/`reveal`
pair *is* the screen contract; defining `Screen` first means defining it twice, and the second
definition has to be reconciled with a router that already exists.

Two rules keep the interface honest. **Pro invents no theme token, no size constant and no locus
kind** — all three sets are closed on purpose; a colour the palette lacks is added to the base in
both palettes, because a colour is not a trade secret, and a place inside a Pro screen is a
`Node { key }` or an `Anchor { slug }`. And **a registry with nothing in it changes no pixel**: no
placeholder, no greyed row, no "available in Pro" anywhere. §13 makes that non-negotiable.

## 5. The one new message family

A Pro feature has to talk to its own half, and must not fork `messages.rs`: the contract is the one
piece of Ubiq expensive to change, and a second edition with a second message set is two products. So
one family, four variants, and the payload is a string the base never parses:

| Message | Direction | Payload | Meaning |
|---|---|---|---|
| `AskExtension` | UI → host | `feature`, `name`, `payload` | Something the interface's half of a feature says to the host's half. `feature` is the `vendor.name` id; `name` is the feature's own verb; `payload` is opaque |
| `ExtensionSaid` | host → UI | `feature`, `name`, `payload` | The answer, or anything the feature says unasked |
| `ExtensionError` | host → UI | `feature`, `error` | The feature refused or failed, in the words the family's own schema fixes |
| `ExtensionAbsent` | host → UI | `feature` | This build has no such feature. The reason a base host can answer a Pro interface at all, and what makes a downgrade a message rather than a hang |

Five rules stop this from being a hole in the contract:

1. **The payload is a `String` the host does not parse**, exactly as `SetPreferences`' value is. Both
   halves of one feature share a schema; nothing else may depend on it.
2. **The `feature` field is checked, not trusted** — a feature only ever receives names under its own
   id, and cannot address another's messages.
3. **No pane, no path, no handle.** Every architecture rule the rest of the contract obeys applies,
   rule 2 in particular: a feature that wants a file asks the file family for it.
4. **Nothing in the base emits or handles one.** With an empty registry `AskExtension` is unreachable
   and every arrival answers `ExtensionAbsent`.
5. **A feature that grows a real family stops using this one** — it gets a family in `messages.rs`
   under the transport contract's procedure, and moves to the base. The envelope is for what is
   genuinely Pro's, not a way around the contract.

**This family is the one exception to the opening rule, deliberately.** It has no base-side user and
is still worth building, because the alternative is a forked `messages.rs` — a second wire format, a
second dispatch, and the end of the "in-process → two processes → distributed by only changing the
transport" property architecture rule 1 buys. Cheap to audit: four variants, one arm, one grep.

## 6. Where a Pro feature keeps things

Three places, each an existing arrangement with one more namespace.

| What | Where | Who reads it |
|---|---|---|
| A host feature's own files — a cache, a token, a sync journal | `extensions/<id>/` under the config root, made by the host before `start` | The feature only. The host names it and looks no further: architecture rule 6, applied a second time |
| A feature's interface state | The preference blob under `Scope::Interface` or `Scope::Project`, in an object keyed by the feature's id | The interface's half of that feature |
| A feature's host settings | Its own directory — a fifth store would be a new trait for one user | The feature |

**`ViewPrefs` must round-trip keys it does not know**, or a user who opens their catalogue in the
base once loses every Pro setting they had. **Landed 2026-09-04 (`3db7b4a`)** as a general catch-all,
not the named `extensions` map sketched here: both it and `InterfacePrefs` gained
`#[serde(flatten, default)] rest: BTreeMap<String, Value>`, keeping every key the struct does not
name, feature ids among them — identical mechanism, and only the general one has a base user today.
`remember_interface` rebuilds `InterfacePrefs`, so its keys park on `interface_rest`. Likewise **a
persisted `RailMode::Extension` no registered spec claims resolves to the default mode**, silently,
and is written back unchanged, so returning to Pro returns to the screen.

## 7. Five hypothetical Pro features, and the seam each uses

The list tests the design, it does not plan the product: a row needing something not in §§3-6 is a
hole.

| Feature | Host | Interface | Seam |
|---|---|---|---|
| **GitHub / Jira / Azure DevOps as a task source** | A feature that polls the API, maps issues to `TaskRecord`, and a `Box<dyn TaskStore>` that reads through it | A settings section, a badge on a card, an `Import` command | Tier 0 for the store, `Feature` for the poll, `Extension` messages for the connect flow, one `CommandSpec` |
| **Single sign-on** | A feature whose `start` blocks the boot until an identity is established, and which owns the token in its own directory | A sign-in surface, drawn as a screen | `Feature::start`, one `ScreenSpec`. Nothing else — the base has no notion of an identity, and must not gain one |
| **Central agent account registration** | An `agent_manager::registry` implementation over the company's service, plus a `credentials::SecretStore` that resolves references rather than holding material | The existing account list in the settings overlay, unchanged | Tier 0 entirely, and it belongs in `agent-manager` because `AGENTS.md` says Ubiq never names a harness config path |
| **Encryption at rest** | Four decorators over the file stores, and the key in the OS keychain through `credentials` | Nothing | `Boot::stores`. No new code in the base at all |
| **Team features — shared sessions, presence** | A feature holding a connection, pushing through `Ctx::everyone()` | A presence panel, a teammate's cursor on the graph | `Feature`, `Extension` messages, one `PanelSpec` |

Four observations, and they are the design review. **Every row is covered, and two need nothing** —
encryption at rest and central accounts are tier 0, the strongest evidence the base's existing seams
were drawn in the right places. **Nothing wanted a new store trait, theme token or locus kind**: the
three things §4 forbids are the three no hypothetical feature asked for. **Only SSO wants something
structural**, and it is a scheduling fact rather than an API — `start` runs before the first window,
so a feature that must gate the boot can, by not returning. **And `mcp_server.rs` is a fifth surface
that should be a base concern**: tools Ubiq exposes to the agents it hosts are the product, not an
edition. Named here so it is not later mistaken for a Pro extension point.

## 8. The runtime plugin registry

A tier-1 extension is a namespaced id, an opaque payload, and a rendered surface. A runtime plugin —
the k8s control panel fetched from an online registry — is the same three things with the
implementation loaded instead of linked. So:

**Pro is the plugin host, and the base never learns that plugins exist.** Pro registers one
`Feature` and one `Contribution` whose id is `pro.plugins`, and behind them is a WASM runtime, a
manifest format, a signature check, a permission prompt and a fetch from the registry. Every plugin
screen is a `pro.plugins` screen; every plugin message is a `pro.plugins` message. Three reasons, in
order of how much they save:

**The base gains no ABI.** A dynamic-library plugin interface across a Rust ABI is a support burden
that outlives every decision here, and a WASM one is a runtime, a host-call surface and a sandbox.
Neither belongs in a source-available application whose extension story is "fork it, it is there".

**A plugin's blast radius is Pro's to define.** The base's rules cannot be enforced against code the
base never sees; enforcing them against code *Pro* loads is Pro's problem, and Pro picks a runtime
that can.

**And it proves the tiers are drawn correctly**: if tier 2 needs a base change, tier 1 was drawn in
the wrong place, which is why §14 puts it last.

## 9. What stays in the base

The base has to be a product on its own, or the licence is a marketing device and contributors know
it. One test, applied to a feature and not to a user:

> **A feature belongs to Pro when it serves somebody other than the person running Ubiq** — an
> organisation, a team, a fleet, a purchasing department. A feature that makes one developer's own
> work better belongs to the base, whatever it costs to build.

SSO, central accounts, team features and the plugin registry all serve an organisation. Encryption at
rest is the interesting case: it serves the person at the keyboard, so by this test it belongs in the
base. It sits in Pro above because that is where it was named — a §16 row, not something to resolve
quietly.

Two things the test does not license: **no base feature is degraded to make room for a Pro one**, and
**no Pro feature is stubbed in the base**, so the base repository contains no evidence of what Pro
has — which is also what keeps the two documentation libraries separable.

## 10. Working on both at once

**Sibling checkouts, a git dependency, and one committed `[patch]`** — the least machinery that
survives CI. `works/ubiq/` is the base; `works/ubiq-pro/` is Pro, its own workspace and lock, naming
the base by revision and patching it next door:

```toml
[patch."https://github.com/mdnmdn/ubiq"]
ubiq       = { path = "../ubiq/crates/ubiq" }
ubiq-app   = { path = "../ubiq/crates/ubiq-app" }
ubiq-host  = { path = "../ubiq/crates/ubiq-host" }
ubiq-proto = { path = "../ubiq/crates/ubiq-proto" }
```

So `cargo build` in Pro compiles the base's working tree — edit either side, build either side, one
editor over both — while the recorded dependency stays a pinned revision, which CI and a release
resolve when the sibling is absent. Pro's `Justfile` adds two recipes: one running the base's
`just verify` in `../ubiq`, one building against the pinned revision with the patch off.

**The rejected alternative is a nested workspace member** at a gitignored `pro/` inside the base,
picked up by a `members` glob. Less machinery — no patch, no pinning, one `target/` — and it fails on
one thing: one workspace is one `Cargo.lock`, and the base's is public, so Pro's entire dependency
graph would be committed to the open repository. Two workspaces, two locks.

**The base's CI never knows Pro exists**, and that asymmetry is the right way round: the open
repository must never be blocked by a build nobody outside can run.

## 11. One config root, two editions

A user moves between editions — trials Pro, lets it lapse, opens the base build to check something.

**An edition is not a data format.** Both read and write the same `projects.toml`, task files,
settings and preference blobs; Pro adds `extensions/<id>/` directories and namespaced keys, changes
no schema and bumps no `schema` field. **A downgrade loses nothing it can avoid losing**: the base
ignores directories it does not know, round-trips preference keys it does not know (§6), and resolves
an unknown rail mode to the default. What it cannot do is decrypt a store Pro encrypted, which is why
encryption at rest needs an export before it is offered — a §16 row.

**Both binaries are called `ubiq`** and differ in bundle identifier, so neither overwrites the other
and either can open the same root — though not simultaneously, which the catalogue already answers
with a no: one host per process exists because two would race the store.

## 12. Licensing, and why this needs no exception

**Pro linking the base needs no grant, because it is the same copyright holder.** The Sustainable
Use License limits what *licensees* may do, not the licensor, who may license their own code under
any additional terms. Pro's binary distributes SUL-covered code under its own licence's terms.

**The contributor agreement is what keeps that true.** `CONTRIBUTOR_LICENSE_AGREEMENT.md` gives the
project permission to license contributions on any terms, so a base contribution can be relicensed
into a Pro build; without it the first outside contribution would make Pro's use of the base an
unresolved question. It exists — what changes is that signing it becomes a gate on merging.

**Pro's `LICENSE` is proprietary and points at the base's** — naming the components under the
Sustainable Use License and the MIT one, where each licence sits, and preserving every notice the
SUL requires be preserved, in the binary and the about surface alike.

## 13. What the base must never gain

Four, and they erode a commit at a time rather than in a decision.

1. **No licence check, no entitlement, no key** — whether a Pro feature is licensed is asked inside
   Pro, by code the base does not contain. **No telemetry, no phoning home** either.
2. **No upsell** — no placeholder screen, no disabled control, no "learn about Pro" (§9), and **no
   dynamic loading**: no `libloading`, no plugin ABI, no WASM runtime (§8).
3. **No conditional compilation for editions** — no `#[cfg(feature = "pro")]` anywhere, because an
   edition is a different `Boot` value, which is what makes `Boot::default()` a real test of the base
   rather than one configuration of three.
4. **No seam without a base user** — the opening rule, with §5's single named exception.

## 14. Phases

1. **The composition root — landed 2026-09-04 (`3c51380`).** `ubiq-app` is a library with a
   three-line `main.rs`; `Boot` (its `stores` alone) and `Stores` exist; `Boot::default()` is present
   behaviour, nothing is registrable, the boot is testable. §6's round-trip landed with it.
2. **The interface registries.** `ScreenSpec`, `PanelSpec`, `CommandSpec`, the three `Extension`
   variants and `RailMode`'s serde pair — and the part that earns its keep, the base's own kitchen
   sink registered through the screen registry rather than matched in five files: the registry is
   proven by the mode that exists to prove things. *After `inbox-routing` phase 1.*
3. **The second repository.** Three crates, the `[patch]` layout, the two `just` recipes, Pro's own
   boundary greps, a proprietary `LICENSE`, and a binary that registers nothing and runs. Ships no
   feature; proves the composition.
4. **The host feature.** `Feature`, `Ctx`, `extensions/<id>/`, the `Extension` family and its one
   dispatch arm. Still no Pro feature. Its unknown-key round-trip in `ViewPrefs` landed with phase 1.
5. **One real feature end to end.** The issue-tracker task source, the §7 row touching the most
   seams at once — a store decorator, a host thread, the message family, a settings surface, a
   command and a card badge. Whatever is wrong with §§3-6 is found here, with one feature's worth of
   code to change rather than five.
6. **The runtime plugin registry, in Pro.** The success criterion is that it needs no base change.
   If it does, phase 2 or 4 was drawn wrong, and the change belongs there.

Phases 1 and 2 are worth having on their own merits. Phase 4 is the one to defer if Pro slips.

## 15. What this costs

**Three trait objects and one message family in the base**, none dispatched through by the base
except the screen registry, which the kitchen sink uses; plus `RailMode`'s registry lookup in six or
seven places and its hand-written serde pair, the only part needing care. **Two repositories** means
the base cannot see Pro's breakage, mitigated by Pro's daily build against base `main`. **The risk
the phases are arranged around** is that the base becomes a plugin framework whose only plugin is
closed — defended against by the opening rule, phase 2's base-side user, and phase 5 coming before
any second feature.

## 16. What this asks to be decided

Eight rows.

- Pro composes the base's crates through one boot function: `crates/ubiq-app` becomes a library with
  a three-line binary, and `Boot::default()` is what ships.
- Pro mirrors the base's crate split, so architecture rule 1 and the two `cargo tree` guards survive
  into the closed edition.
- Extension has three tiers: the seams that exist, three compile-time registration points, and a
  runtime plugin registry entirely in Pro. The base gains no dynamic loading and no ABI, ever.
- An extension point with no base-side user is not built. The `Extension` message family is the one
  named exception, taken because a forked `messages.rs` costs more than four unused variants.
- Pro invents no theme token, no size constant, no locus kind and no store trait.
- The base contains no licence check, no telemetry, no upsell and no stub of a Pro feature, and no
  base feature acquires a limit that Pro lifts.
- An edition is not a data format: both read one config root, Pro only adds namespaces, and the base
  round-trips what it does not understand rather than dropping it.
- A feature belongs to Pro when it serves an organisation rather than the person at the keyboard —
  **and by that test encryption at rest belongs in the base.** This row is where that is decided.

Backlog rows this leaves open: whether MIT `agent-manager` is actually published, since it is the one
crate that now could be; `just verify` omitting `core` and `fmt`, which matters more once a second
workspace consumes these crates; an export path for an encrypted store, without which encryption at rest is a
one-way door; whether the `mcp_server.rs` surface is a base concern (this proposal says yes); and how
a Pro release pins the base — a tag, a submodule or a vendored copy — which phase 3 answers and this
document does not.

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the two halves, the six rules, and the crate boundary this proposal composes rather than crosses
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the message set §5 adds a family to, and the procedure it follows
- [`../tech/project-structure.md`](../tech/project-structure.md) — the workspace §3 and §10 reshape
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — the boundary owning accounts, credentials and the skill catalogue, and therefore §7's third row
- [`../features/workbench.md`](../features/workbench.md) — the rail modes and panels §4 registers
- [`./completed/ui-routing-proposal.md`](./completed/ui-routing-proposal.md) — the `where`/`reveal` pair that is the screen contract, and why its phase 1 comes first
- [`../backlog.md`](../backlog.md) — where §16's open rows go
