---
id: inbox-ui-id
title: Proposal — naming the interface, an identity scheme for areas, menus and elements
kind: proposal
status: proposal
summary: A second, independent design for UI element identity — the thing an in-place inspector points at, a personalisation rule keys off, and an extension anchors to. It argues that the hierarchy must be declared in a data catalogue rather than inferred from whatever the last frame drew, that repetition is a typed selector on top of a name and never part of it, that help targets need their own frontmatter field instead of borrowing the contextual `context` ladder, and that readable dotted paths beat opaque keys only once a rename contract with aliases exists to make them safe.
read_when: you are designing or reviewing how UI elements are named, wiring an inspector or in-place help, or deciding what a personalisation rule or an extension is allowed to anchor to
updated: 2026-09-20
depends_on: [tech-ui, feat-workbench, inbox-help, wip-help, inbox-personalisation, inbox-plugin-system, inbox-editions, tech-decisions]
---

# Proposal — naming the interface

Ubiq cannot name anything on its own screen. There are roughly 355 kebab-case strings scattered
through `crates/ubiq/src/ui/`, and every one of them exists to satisfy GPUI: `icon_button` in
`crates/ubiq/src/ui/kit/controls.rs` takes an id because a stateful element needs one for hover,
scroll and focus bookkeeping. They are not unique, not hierarchical, and not semantic.
`crates/ubiq/src/ui/rail.rs` — the most identifiable area in the product — carries none at all.
There is no registry, no enum, and no map from any of those strings to a meaning.

Three features want that map: an in-place, inspector-style help mode; targeted personalisation; and
extensions that contribute into places they do not own. This document is a **second** design for the
identity layer underneath all three, written against a first one already in flight (§8). It takes a
different position on four points, and the disagreements are stated rather than smoothed over.

The claim it argues: **identity is a published contract, not a rendering artifact.** Everything else
follows from that.

---

## 1. Three granularities, and why they are not one thing

The three things a user points at behave differently enough that a single rule for all of them is
what makes a scheme collapse.

| | **Area** | **Menu** | **Element** |
|---|---|---|---|
| Lifetime | the window's | one gesture | the frame's |
| Contains others | yes, and that is its job | its items | no |
| Position | fixed in the layout | over whatever it opens on | fixed or repeated |
| Named by | where it sits in the tree | its trigger | its parent plus its role |
| How many | dozens | dozens | hundreds, some unbounded |

An **area** is a region that persists and contains: the rail, the titlebar, the dock's right region,
a rail mode's body, a panel. It is the unit personalisation actually wants — "make this column
narrower", "hide this whole strip" — and the unit help usually wants too, because a paragraph
explains the explorer, not the chevron on row four. Areas are the only nodes allowed children, which
is what turns a dotted string into a real tree: an element whose parent path is not a declared area
is a lint failure, not a stylistic quibble.

A **menu** is transient and data-driven, and that combination is what breaks naive schemes. It is
not at a fixed place in the layout, so it cannot be named by where it is drawn; its items come from
a list, so they cannot be named by their position in it. A menu is named by **its trigger**, which
is a stable element that already has a name, and its items are named by **the action they invoke**,
which is a name the code already has. Row order never enters the name. A right-click tab menu whose
two rows are `Hide` and `Close` gives `…tab.menu.item[action:hide]`, and reordering the rows changes
nothing.

An **element** is a single control. Most are fixed and unproblematic. The interesting ones are the
repeated: a row per file in the explorer tree, a column per agent, a tab per pane, a badge per
project. These have no static name and never will, because their number is a property of the user's
data.

## 2. Repetition is a selector, not a name

This is the load-bearing decision, and getting it wrong is what would make the scheme fall over the
first time it met the explorer tree.

**An id names a kind of place. An instance is addressed by a typed selector appended to it.**

```
explorer.tree.row                      the kind of place — one id, declared once
explorer.tree.row[path:src/main.rs]    one instance — never declared, never stored
agents.column[ulid:01J…]               one instance, keyed by the entity it draws
```

Selectors are typed — `path:`, `ulid:`, `action:`, `kind:` — and the type says what the identity of
the instance actually rests on. The type matters because it decides what survives: a `ulid:` outlives
a restart, a `path:` outlives one until the file moves, and an **`index:` selector is forbidden
anywhere it would be persisted**. "The third row" is a position, positions move, and a saved
personalisation keyed on one is a bug with a delay fuse.

From that, one rule that keeps the whole scheme finite:

> **Help, personalisation and extensions bind to the path. Only the inspector reads the selector.**

The inspector needs the selector because it is telling the user what is under the cursor and wants to
say *which* row. Everything durable binds to `explorer.tree.row`, so there are hundreds of ids in the
catalogue rather than one per file on the user's disk. A per-instance override is a separate, opt-in
feature that a later proposal can argue for; identity does not need it and should not pay for it.

The consequence worth stating plainly: the set of declared ids is **bounded by the code**, not by the
user's data. That is what makes a catalogue, a lint, and a coverage number possible at all.

## 3. Two naming models, honestly compared

| | **Readable path** `workbench.rail.mode.tasks` | **Opaque key** `u7f3a2c`, label beside it |
|---|---|---|
| Authoring a help page | write the name you already know | look the key up every time |
| Reading a diff | the name says what moved | nothing |
| Rename | silently breaks every binding | free, the label changes and the key does not |
| Prefix match | `workbench.rail.*` works | impossible — keys carry no structure |
| Hierarchy | in the name | a separate parent field, which can disagree with the layout |
| Collision | possible, catchable by lint | impossible by construction |
| Grep for usage | trivial | trivial |
| First impression of a stranger | correct | none |

Prefix matching is what settles it. Personalisation's most valuable rule is the one that applies to a
region and everything in it, and an extension anchors "inside this area, after that element". Both
need to reason about containment from the id alone, against targets that may not be loaded. The
opaque model can only do that by carrying a parent pointer that is a second source of truth for the
same fact, and two sources of truth for the tree shape is exactly the failure the opaque model was
supposed to avoid.

So: readable paths — but the rename row in that table is real, and ignoring it is what makes people
reach for opaque keys in the first place. The fix is the third model.

### The hybrid, and what it borrows

**The id is a readable dotted path, and it is declared in a catalogue that also carries its aliases.**
A rename is an alias row, not an edit. This is not a new idea in this tree: help pages are addressed
by a frontmatter `id` rather than a path for exactly this reason, and `redirects` keeps a link a user
already pasted working after a page moves. Identity needs the same mechanism one level down.

The catalogue is a TOML file, `ui-targets.toml`, beside the crate whose interface it names. One row
per declared id:

```toml
[targets."workbench.rail.mode.tasks"]
kind = "element"                 # area | menu | element
label = "Tasks"                  # what the inspector calls it
blurb = "Opens the tasks mode, where a project's work items live."
help = "workbench-modes"         # optional: a help page id
since = "2026.10"
```

Four things that row buys and a bare string in Rust does not: a **label and a sentence** for the
inspector, which is the whole visible product of this work; a **declaration** the lint can check
usage against in both directions; a **kind**, which is what makes the area rule enforceable; and a
place to put `moved_from`, which is the rename contract.

## 4. Stability is the contract, not the id

The moment a personalisation file or an extension manifest names an id, that id is public API. The
rules have to be written down before the first one is written, not after.

- **A declared id with no `status = "experimental"` is stable.** Experimental ids are fair game to
  rename freely and are not offered to extensions.
- **Renaming is adding, never editing.** The new row carries `moved_from = "old.id"`; the old id
  keeps resolving. A resolver walks at most **one** hop — a chain of two is a lint failure and the
  fix is to collapse it — so the cost of an alias never compounds.
- **Who may rename: the change that renames the element.** No tidying passes over the catalogue. A
  rename with no behaviour change behind it is a breaking change for zero benefit.
- **Path is a namespace, not a location.** Moving the search field from the titlebar into the ribbon
  does not rename it, because its meaning did not change. This is the rule that makes readable paths
  survivable, and it is the one people get wrong: the path describes what a thing *is* within the
  product's structure, not which container currently draws it. A path that has to change every time
  the layout does is a layout description wearing an id's clothes.
- **Deletion is deletion.** The row goes, the lint fails any help page still naming it, and a stored
  personalisation keyed on it is dropped at load with one log line. Never a dialog: a user who
  upgrades is not the right person to ask about a removed button.

## 5. Binding to help — a new field, not the `context` ladder

The in-flight design binds help through the existing page frontmatter `context:` list under a new
`ui.*` namespace. That reuses machinery that already works, and it is still wrong. Three reasons,
and each is a constraint the packer enforces today:

1. **`context` answers a different question.** It is one rung of a five-rung fallback ladder in
   `crates/ubiq/src/app/help.rs` that resolves *where the reader is* to exactly one page. Its
   exactly-once validation exists because the ladder must return a single answer. A UI target is
   not a fallback and not exclusive — several pages may legitimately mention the same control, and
   one of them is primary.
2. **It would flood the manual.** Every `context` key must be claimed by a page, and every page must
   be reachable from `nav.order`. Bind three hundred elements through `context` and the nav tree
   grows three hundred stubs whose only reader is a tooltip.
3. **Most targets do not deserve a page.** What an inspector shows when the cursor is over a button
   is one sentence. That sentence belongs in the catalogue row next to the label, where whoever
   names the element writes it, and where it exists for every id rather than only for the ones
   someone wrote a page about.

So, two bindings with different shapes:

- **`blurb` in the catalogue** — the sentence, mandatory, one per id. This is what inspector mode
  draws, and it is why the catalogue is data and not a Rust constant list.
- **`targets:` on a help page** — a new frontmatter list, many-to-many, no exclusivity, no nav
  obligation. This is the "read more" link, and its inverse is what lets a page say which parts of
  the screen it explains.

`context:` keeps its job, unchanged and unextended. Packing both fields is the same mechanical
inversion, so the packer barely grows.

## 6. What the other two consumers need that help does not

Designing against help alone is what would make this help-shaped by accident. The other two ask for
three things help never would.

**Personalisation** ([`personalisation-proposal.md`](./personalisation-proposal.md)) needs an id to
**survive an upgrade**. Help is republished with the binary, so a stale `context` key is caught at
build time; a personalisation file is the user's and is read by a build that has never seen it. That
is the entire argument for `moved_from` and for the silent-drop rule. It also needs to address
**areas, with containment** — a size or density rule set on a region applies to what is inside it —
which is §3's prefix-matching requirement and the reason opaque keys lose.

**Extensions** ([`plugin-system-proposal.md`](./plugin-system-proposal.md)) need to name a target
that **does not exist here**: one that ships only in Studio, one a later release adds, one behind a
feature another plugin provides. So resolution is late and tolerant — an unresolved anchor is inert,
logged once, never an error — and an extension manifest is validated against the catalogue it was
authored against, not the one it runs on. They also need **a namespace they own**: `ext.<plugin-id>.*`
is reserved, the base declares nothing under `ext.`, and a plugin's own contributed elements are
addressable by the next plugin along.

**Editions** ([`editions-proposal.md`](./editions-proposal.md)) make the catalogue a list, exactly as
the help bundle root is a list: each edition ships its own file under its own root prefix, a later
one may add rows and may deliberately override one it declares, and never patches the earlier file.

## 7. Coverage, and not rotting

The scheme needs a denominator, and there is a good one available for free: **marking replaces the
existing `.id(…)` call rather than sitting beside it.** The marking API returns the `ElementId` GPUI
wanted anyway, so a marked element has one id and an unmarked one has the old bare string. That makes
coverage greppable: every `.id("…")` under `crates/ubiq/src/ui/` that is not a marking call is an
unmarked element, and there are 91 of them to work through today.

Coverage is not required to reach 100%, and pretending otherwise is how a lint gets suppressed.
The target is **every area and every menu, and elements on demand** — an element earns an id when
help, personalisation or an extension wants it, and the catalogue's `unused` report is the honest
list of ids nobody wanted.

Enforcement is a tool, specified separately in
[`ui-id-tooling-proposal.md`](./ui-id-tooling-proposal.md): declared-but-unused, used-but-undeclared,
naming violations, dangling help `targets:`, and per-area coverage. It joins `just verify` beside
`help-check`.

## 8. Where this disagrees with the design in flight

The first design is a dotted `UiId`, a per-frame registry of `(UiId, Bounds)` fed by an element
extension, deepest-hit lookup, help bound through `context:` under `ui.*`, and a static greppable
list of known ids in Rust. The registry and the deepest-hit lookup are right and this proposal keeps
them. Four things it should change:

1. **The catalogue must be data and authoritative.** A `const` list in Rust is a comment: nothing
   stops a marking call with an id absent from it, and it has nowhere to put a label, a sentence, an
   alias or a deprecation. Every consumer past the inspector needs those.
2. **The per-frame registry must not be the source of truth for what exists.** If it is, the answer
   to "which ids exist" depends on what happened to be on screen. Coverage, linting, an extension
   manifest validated at install, and a personalisation file read at startup all need a static
   answer. Registry for hit-testing; catalogue for identity.
3. **`context:` under `ui.*` is the wrong field** — §5.
4. **No instance model is the gap that matters.** `rail.mode.tasks` is a fine static name and the
   explorer tree has none. Without §2's typed selector the design has nothing to say about the first
   two screens anyone will point the inspector at.

One naming collision worth catching early: `crates/ubiq/src/ui/mark.rs` is the brand mark, and a
marking API called `ui_mark` reads as being about that. `ui_id` and a module called `ident` cost
nothing and confuse nobody.

## 9. Recommendation

**Take the hybrid: readable dotted paths, declared in `ui-targets.toml`, with typed instance
selectors and `moved_from` aliases.**

Opaque keys buy rename safety and lose prefix matching, which is the one operation personalisation
and extensions both need most. Bare readable paths — the in-flight design — buy readability and pay
for it the first time anything durable keys off them. The catalogue is what converts the second into
the first: it costs one TOML file and a lint, and it is also where the inspector's label and sentence
were always going to have to live, so most of it is not overhead at all.

Phase it so the expensive part is last:

1. The catalogue, the marking API, and **areas and menus only**. The tree is small, the lint is easy,
   and the inspector already has something worth showing.
2. The inspector over the per-frame registry, with instance selectors.
3. Help binding — `targets:` on pages, `blurb` in the catalogue, the "read more" link.
4. Personalisation keys, with `moved_from` proven by the first real rename.
5. Extension anchors and the `ext.*` namespace.

Phases 1 and 2 are the useful minimum, and they are shippable without help, personalisation or
extensions existing at all.

## 10. What this owes the library when it lands

- A `tech/` document owning the id grammar, the selector types and the stability contract. It is a
  rule, not a capability, so it is filed rather than created by the implementer.
- Decision rows: identity declared in data rather than in Rust; repetition as a selector rather than
  as part of the name; a `targets:` field kept separate from the `context` ladder; `ext.*` reserved.
- Backlog rows for the unmarked elements, and for the per-instance override this deliberately
  excludes.
- `tech/operations.md` gains the `ui-ids` recipes.

---

## Related docs

- [`ui-id-tooling-proposal.md`](./ui-id-tooling-proposal.md) — the lint and search tool this needs
- [`help-proposal.md`](./help-proposal.md) — the help pipeline, its page ids and its `redirects`
- [`../wip/help.md`](../wip/help.md) — how the context ladder and the packer work today
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the render model and the kit
- [`../features/workbench.md`](../features/workbench.md) — the areas this scheme has to name
- [`personalisation-proposal.md`](./personalisation-proposal.md) — the second consumer
- [`plugin-system-proposal.md`](./plugin-system-proposal.md) — the third
- [`editions-proposal.md`](./editions-proposal.md) — why the catalogue is a list
