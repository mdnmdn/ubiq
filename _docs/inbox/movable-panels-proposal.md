---
id: inbox-panels
title: Proposal — movable panels and file viewers
kind: proposal
status: proposal
summary: Every area of the workbench becomes a movable panel in a dock tree — terminals, the log console, the explorer, the chat, each open file — with the placement policy that keeps side panels on the border, the pane rules a move has to survive, and what it costs the one-view rule.
read_when: you are deciding how the window is arranged, or whether a panel can be dragged, split or docked
updated: 2026-08-31
depends_on: [feat-workbench, feat-panes, tech-ui, tech-architecture]
---

# Proposal — movable panels

Today the window's arrangement is a compile-time constant: three panels around a centre, the centre a
stack of two, each shown or hidden and dragged within its own bounds. This proposes replacing that
frame with a **dock** — a tree of tabbed groups the user rearranges by dragging — and making every
area of the workbench a **panel** inside it: the terminals, the log console, the explorer, the chat
and each open file.

What a panel *draws* when the file it holds is not plain text is
[`file-viewers-proposal.md`](./file-viewers-proposal.md), its companion. Neither blocks the other,
and each is worth less alone: a dock holding only terminals is a multiplexer Ubiq already nearly has,
and a Markdown preview that cannot be dragged beside its source is a tab the user switches away
from.

## 1. Where it stands

`crates/ubiq/src/ui/shell.rs` builds one `h_resizable` of three slots — explorer, centre, chat — and
makes the centre a `v_resizable` of two: the editor above, the dock below. The arrangement is
written in the function; what varies at runtime is each panel's `visible` flag and its size within a
constant range. Four consequences, and each is a reason to move.

**Only one pane is ever drawn.** `ui/terminal.rs` renders the *focused* pane's emulator and nothing
else, so the domain rule that unfocused panes keep drawing has nowhere to be true: the dock has one
body, and every pane but one is a tab label.

**The layout mode is an enum nobody reads.** `LayoutMode::{Single, Vsplit, Hsplit, Grid}` is stored
on `AppState`, returned by an accessor with no caller, and drawn by nothing. That is `G6`.

**The console borrows a strip built for panes.** `D25` put the log console in the dock's tab strip
and paid for it: one tab with no pane ID, a `+` and a close button that mean two things, and a pane
and the console that cannot be read at once.

**The editor is one buffer wearing several tabs.** A single `Entity<EditorState>` is re-seeded on
every tab click, and the tab strip is hand-rolled — both of which the dock replaces, and which its
companion takes further.

Panel sizes and visibility also die with the process, which is `G13`.

## 2. What "movable" has to mean here

A pane is not a document. Five rules from [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md)
constrain the design before any of it is drawn, and a dock that breaks one of them is worse than the
frame it replaces.

**Moving a pane must not restart its harness.** A drag rearranges the window and does not touch the
process: the emulator entity, its output stream and its `PaneInput` writer survive the move
untouched, and the pane ID does not change.

**Every move ends in a measurement that reaches the harness.** A pane that lands in a narrower
column and is not re-measured is the corruption bug the domain rules exist to prevent. The existing
mechanism already covers it — the emulator measures its own bounds and its resize callback posts
`TerminalResize` — but the invariant has to be stated for the dock: *a move is a resize*.

**A pane that is not the displayed tab is not laid out, and is not resized.** Its harness keeps the
last geometry it was told, which is correct — nothing changed for it — while its output keeps
arriving and its emulator keeps consuming. The stream must not stall on visibility, or a background
tab backpressures the harness.

**Exactly one pane holds the keyboard.** Under a dock this generalises: the keyboard belongs to the
focused *panel*, and when that panel is not a terminal, no pane holds it. That is today's
`DockTab::Logs` rule, stated for every non-terminal panel rather than for one.

**An exited harness's pane is still a panel.** It keeps its last screen, it moves, it splits, it
docks. Nothing about being dead makes it furniture.

## 3. The model

### Vocabulary

The word "panel" currently means one of the three regions around the centre, and has to be freed
because the movable unit needs it. The proposed set — a change to
[`../product/glossary.md`](../product/glossary.md), which owns vocabulary, and therefore something
this proposal asks for rather than does:

| Word | Means |
|---|---|
| **Panel** | The movable unit. It has a tab, a title, a focus handle, and it can be dragged, split, tabbed, zoomed and closed. A terminal, the console, the explorer, the chat, an open file, a viewer |
| **Pane** | Unchanged: the terminal view of one workspace. A pane is one *kind* of panel, and every rule already written about panes survives verbatim |
| **Group** | A tabbed stack of panels sharing one rectangle. Exactly one of its panels is displayed |
| **Split** | Groups arranged along an axis, with a draggable divider between them |
| **Region** | Centre, left, right or bottom. Each holds its own tree of splits and groups |
| **Dock** | All four regions together — the window's whole arrangement |

"The bottom dock" stops being a proper noun; it is the bottom region, and what is in it is a choice
rather than a definition.

### The tree

Each region is a tree whose interior nodes are splits and whose leaves are groups, and a panel lives
in exactly one group. There is no leaf that is a bare panel — a single panel is a group of one, which
is what makes "drop another panel beside it" the same operation everywhere.

Dropping a panel on the middle of a group adds it as a tab; dropping it on an edge splits that group
along the matching axis and puts it in the new half. Removing the last panel removes the group, and a
split left with one child collapses into it. That is the whole interaction: **tabs come from the
middle, rows and columns come from the edges.**

### Where a panel may go

Not everything should go everywhere: the explorer in the bottom region beside a terminal is a
sixty-pixel-tall tree, and a chat squeezed into a centre column stops being a conversation. So
placement is a property of the panel's kind:

| Class | May sit in | Kinds |
|---|---|---|
| **Edge** | The left or right region only | Explorer, Agents, Chat |
| **Free** | The centre or the bottom region | Terminal, Log console |
| **Centre** | The centre region only | Editor, every viewer |

An Edge panel moves between the left and the right border and nowhere else — that is what "the side
panel remains on the border" turns into. It stays movable, tabbed with its siblings and resizable; it
has two homes instead of four. The policy is a table, not a special case: one function from panel
kind to a set of regions, consulted in one place, and widening it later is a row.

### Top

**There is no top region.** The component library's dock places edge regions left, right and bottom
only, which was verified against the revision the workspace pins. "Docked on top" is therefore a
split at the top of the centre region — visually the same result, structurally a centre split rather
than an edge dock, which means it takes its width from the centre rather than spanning under the
explorer. That is the honest limit, and a real top region is a backlog row against the library.

## 4. Adopting the component library's dock

`gpui-component` at the revision the workspace pins ships a complete dock: a `DockArea` owning one
tree per region, tabbed groups, splits over the same `resizable` primitive `shell.rs` already uses,
drag-to-tab and drag-to-edge with drop indicators, zoom, a free-floating tiles canvas, and a
serialisable layout with a panel registry to rebuild leaves on load. Ubiq should use it rather than
write a second one. That is [`../tech/ui-and-design.md`](../tech/ui-and-design.md)'s "gpui-component
first" applied to the largest widget in the library.

Two things make it a fit rather than a compromise.

**The drag is not ours to write.** A dropped tab is re-parented by the dock itself, and re-parenting
moves a *panel ID* within the tree — the entity is never rebuilt. That is exactly the guarantee §2
demands: a dragged terminal is the same `TerminalView`, on the same stream, under the same harness.

**Appearance is a seam, not a given.** The engine owns the tree, the drag and the persistence; a
renderer trait owns every pixel. Ubiq writes its own skin instead of taking the library's default and
keeps `D18` — square surfaces, one coloured left edge, tokens only. The skin is also where §3's
placement policy is enforced, because whether a tab offers a drag at all is the skin's answer.

### What it costs: `D17`

`D17` says each screen area is a free function and `AppState` is the application's only `Render`. A
dock panel cannot be a function: the library requires each panel to be an entity that renders,
focuses and emits. **This proposal reverses the "one view" half of `D17` and keeps the other half.**
`D17` names its own reversal trigger — *"if a panel ever needs its own focus and key handling, that
is the point to reverse this"* — and a dock of independently focusable panels is that point arriving.

The half that stays is the one that mattered: **`AppState` remains the only owner of state.** Each
panel is a thin adapter holding an `Entity<AppState>` and a panel kind, whose render delegates to the
existing `fn render(&AppState, &mut Context<AppState>)` in `ui/terminal.rs`, `ui/explorer.rs`,
`ui/chat/` and the rest. Every one already has that exact signature, so the adapter is a `match` and
the area modules are untouched. A panel holds no state beyond what identifies it — a pane ID, a file
path, a viewer kind.

**One thing a spike has to prove first.** The adapter renders by updating `AppState` from inside its
own render, which is sound only because a child entity's render runs in the layout pass, after the
parent's render has returned. If that ordering does not hold, the fallback is to read `&AppState`
from the entity and move the area modules' `cx.listener` call sites onto the existing
`ui::handler`/`ui::indexed` bridges, which already take a plain closure — a day of work, not a
redesign, but worth knowing before the phase rather than during it.

## 5. What each area becomes

| Today | Becomes | Class | Count | What changes |
|---|---|---|---|---|
| Terminal dock body | `Terminal` panel | Free | One per pane | All panes draw at once; the pane list is the dock's tabs |
| Log console | `Logs` panel | Free | One | Stops borrowing the pane strip; `D25` is superseded |
| Explorer | `Explorer` panel | Edge | One | Moves between borders; keeps its width |
| Chat | `Chat` panel | Edge | One | Same, on the other border |
| Editor tab | `Editor` panel | Centre | One per open file | Each file gets its own `EditorState` |
| — | `Viewer` panel | Centre | One per open file | New; the companion proposal |
| Empty page | Unchanged | — | — | Still what a rail mode with no screen shows |
| Titlebar, rail, status bar | Unchanged | — | — | Chrome does not move. `D18`'s frame is not part of the dock |

What leaves the tree: `LayoutMode` and its accessor, `DockTab` and `select_dock_tab`,
`PendingFocus::Logs`, the second caller of `kit::tab_strip` — the dock's groups draw tabs now — and
the three pairs of panel size constants in `theme.rs`, which become the skin's defaults and then the
persisted layout's business.

The editor's row is the one that is more than a port: one panel per file means one `EditorState` per
file, and the buffer copy on every tab click disappears. That change, and the viewers it makes
possible, are the companion proposal's.

## 6. Focus, resize and the harness under a dock

The call order, restated. Nothing in the transport changes — no new message, no changed payload, no
new field. (Its companion adds one family, for a diagram rendered outside the UI.)

| What the user does | What happens |
|---|---|
| Clicks a terminal panel's tab | The dock displays it and gives it focus; the adapter calls `focus_pane()`, which sends `Focus` |
| Clicks a non-terminal panel | The dock focuses that panel. `AppState` clears the focused pane, so no pane receives keystrokes |
| Drags a terminal panel to an edge | The dock splits and re-parents by ID; the emulator is untouched, is laid out in its new rectangle, measures itself, and its resize callback posts `TerminalResize` |
| Drags a terminal panel into another group | Same, minus the split. If it lands as a background tab it is not laid out and not resized until it is displayed |
| Drags a divider | The two groups re-measure; every laid-out emulator in them posts its own resize |
| Zooms a panel | It fills the region; one measurement out, one on the way back |
| Closes a terminal panel's tab | `close_pane()` as today — `CloseWorkspace`, and the child is killed. `D22` is unchanged |
| Spawns a pane | `WorkspaceSpawned` builds the panel and adds it to whichever group most recently held a terminal, or to a new group in the bottom region |

Two of these are rules rather than rows. **A background tab's stream never stalls** — the emulator's
reader thread is not a render, and consumes whether or not the panel is laid out. And **focus is a
property of the dock, mirrored into `AppState`**, not the other way round: `AppState` learns which
pane is focused from the panel the dock activated, and `Focus` is sent on that transition and no
other.

## 7. Persistence

The dock serialises its whole arrangement — the tree, the axes, the sizes, which tab is displayed,
and one opaque payload per panel written by the panel itself — and rebuilds it on load through a
registry keyed by panel name. A name whose builder is missing becomes a placeholder carrying its
payload forward, so an unknown panel is preserved rather than silently deleted on the next save.

That fits the seam [`config-persistence-proposal.md`](./config-persistence-proposal.md) already
proposes, without extending it: the layout is **view state, and the host stores it as an opaque value
it never parses**, keyed by project. Ubiq writes nothing inside the project's folder. This resolves
`G13` and more than it asked for — sizes, visibility, arrangement and which files were open, in one
blob, which is exactly the row that proposal lists as "terminal layout".

Three rules the payloads need. **A panel's name is permanent**, because it is the key a saved layout
is rebuilt from. **Layout persists; harnesses do not**: a saved terminal panel is dropped on load and
the tree normalises around the gap, so a window reopens with its editors, viewers and side panels
where they were and no terminals — restoring them is `Q1`, whether sessions survive a restart at all,
and this rule is where that answer lands. And **a panel's payload is what it is looking at, not what
it drew** — a path, a kind, a layout mode, a scroll position, never a parsed scene or a rendered
diagram. What a viewer may put in its payload is its companion's rule.

## 8. Failure

| What happens | Result |
|---|---|
| A pane is dragged while its harness is writing | Output keeps arriving; the emulator redraws in the new rectangle after the move's resize |
| A pane becomes a background tab | It stops being laid out, keeps consuming its stream, and is resized when it is next displayed |
| A harness exits while its panel is a background tab | The panel's dot changes; nothing moves and nothing closes |
| An Edge panel is dragged over the centre | No drop indicator appears and the drop is refused; the panel returns |
| The last panel in a region is closed | The region collapses. The centre with nothing in it shows the empty page |
| A saved layout names a panel the build lacks | It is drawn as a placeholder carrying its payload, and survives the next save |
| A saved layout is a stale version, or holds terminal panels | The version is discarded for the default; the terminals are dropped and the rest restored |

## 9. Phases

1. **The dock, with today's contents.** The `DockArea`, Ubiq's own skin over it, the panel adapter,
   and the six existing areas as panels in the arrangement `shell.rs` builds now. Drag, split, tab
   and zoom work; nothing else changes. `LayoutMode`, `DockTab` and the hand-rolled strip leave.
2. **Panes as first-class panels.** Every pane draws at once, the placement policy is enforced, the
   console stops being a special tab, and §6's focus and resize paths are the ones in the code.
   Resolves `G6`; supersedes `D25`.
3. **Persistence.** `dump`/`load`, the panel registry, the version, and the blob in the host's view
   store. Resolves `G13`.
4. **The editor split into panels.** One `EditorState` per file; the buffer copy on tab switch goes.
   It is the companion proposal's first phase, and the seam every viewer there attaches to.

Phases 1 to 3 stand alone and are worth taking without the rest.

## 10. What this asks to be decided

Five decision rows, if this is taken:

- The window's arrangement is a dock tree the user rearranges, not a frame the code fixes. Panels
  are dragged, tabbed, split, docked and zoomed.
- The dock is `gpui-component`'s, with Ubiq's own skin over its renderer seam. Ubiq writes no drag,
  no drop indicator and no layout serialisation.
- `D17` is half reversed: a panel is a view, because the dock requires one. `AppState` stays the only
  owner of state, and panels stay adapters over the area functions that exist.
- Where a panel may sit is a property of its kind — Edge, Free or Centre — and that is what keeps the
  explorer, the agents browser and the chat on a border.
- The layout persists as the dock's own serialisation, held by the host as opaque view state. Panels
  restore; harnesses do not.

`D25` — the log console as a dock tab — is superseded rather than amended: it existed because a
fourth panel cost three constants and a titlebar switch, and under a dock it costs nothing.

One glossary change: **panel** becomes the movable unit, **pane** narrows to the terminal kind of
panel, and **region**, **group** and **split** are added. Every existing rule about panes survives
unedited, which is the test that the narrowing is safe.

Backlog rows left open: no top edge region in the component library's dock, so a top dock is a centre
split; the free-floating tiles canvas the library offers and nothing here uses; keyboard navigation
between panels, which `D17`'s reversal makes possible and no phase above builds; and whether a
restored layout should offer to respawn the harnesses it dropped, which waits on `Q1`.

## Related docs

- [`../features/workbench.md`](../features/workbench.md) — the frame this replaces, and the area table it maintains
- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — the pane rules §2 is drawn from
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and conventions the skin keeps
- [`file-viewers-proposal.md`](./file-viewers-proposal.md) — what the centre's panels draw
- [`../tech/architecture.md`](../tech/architecture.md) — the rules a moved pane may not break
- [`config-persistence-proposal.md`](./config-persistence-proposal.md) — the view store §7's layout blob goes into
- [`../tech/decisions.md`](../tech/decisions.md) — `D17`, `D18` and `D25`, which §4 and §10 touch
- [`../backlog.md`](../backlog.md) — `G6`, `G13` and `Q1`
