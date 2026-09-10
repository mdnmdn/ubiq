---
id: inbox-render-velocity
title: Problem — every long list in the interface is built whole, every frame
kind: proposal
status: proposal
summary: One `impl Render for AppState` draws the whole window, and every wire message notifies it unconditionally — so a streamed token rebuilds every visible list at once. Nine lists build one element per row per frame with no virtualization (chat transcript under forty blocks, diff body up to ten thousand rows, commit history, refs, working tree, outline), three of them re-filter or re-lowercase their whole backing vector per frame, and the plain editor path clones the entire file to a `String` on every render for a value it never reads. The interface already holds both virtual lists it needs — `gpui::uniform_list` for fixed rows, `gpui_component::v_virtual_list` for variable ones — and neither is a new dependency.
read_when: a list, transcript or diff is slow to scroll, you are adding a screen that draws one row per item, or you are asking why a streaming agent makes an unrelated panel stutter
updated: 2026-09-09
depends_on: [feat-chat, feat-workbench, tech-ui, tech-version-control, tech-architecture]
---

# Problem — every long list in the interface is built whole, every frame

Ubiq's slowness under a long list is not one bug in one screen. It is one architectural fact and a
set of consequences that follow from it mechanically:

> **The window is a single render entity, and every wire message marks it dirty.**
> `impl Render for AppState` (`crates/ubiq/src/app/shell.rs:809`) is the only `Render` in the crate
> that draws anything real — the rest are drag ghosts. `Message::ConversationUpdate` calls
> `cx.notify()` unconditionally (`crates/ubiq/src/app/wire.rs:1107`), once per streamed delta, with
> no coalescing and no test for whether that agent is on screen.

So a token arriving in a background agent's transcript rebuilds the dock, the explorer, the git
panel, the open diff and the editor tab. Every list that builds one element per row does that work
again, at the harness's token rate. The lists are the multiplier; the notify path is the clock.

## 1. What is actually slow, per area

Every row below was read out of the tree, not inferred.

### The chat transcript

| Fact | Where | Cost |
|---|---|---|
| Windowing exists and is well built — off-screen blocks become a measured height-only placeholder | `state/conversation.rs:1126`, `ui/conversation/mod.rs:492` | O(visible) above the floor |
| …but it is gated behind forty blocks. Below that, *every block is built every frame*, as its own comment says | `state/conversation.rs:972` (`WINDOW_MIN = 40`) | O(n) per frame, and n < 40 is the common case |
| `visible_blocks()` scans and allocates a fresh `Vec` over every block, per transcript, per frame | `ui/conversation/mod.rs:433` | O(n) allocation per frame |
| `has_subagent()` scans all blocks, once per `Delegate` tool block — and delegates are deliberately never folded | `state/conversation.rs:413`, `ui/conversation/mod.rs:591` | O(n²) per frame on a delegating run |
| The streaming block's whole body is cloned into a `SharedString` each time it is built; the tail body grows every token | `ui/conversation/mod.rs:580` | O(len) clone per token |
| Nothing is virtualized with a real list — the transcript hand-rolls its own windowing | `ui/conversation/mod.rs` | see §3 |

Subagents are **not** a multiplier: `visible_blocks()` filters by `viewing_subagent()`, so one
transcript is drawn at a time and the switcher is a cheap summary. What subagents cost is the
`has_subagent` scan above, and the fact that a delegate's own stream notifies the root window while
its transcript is not even on screen.

### The editor and the preview

The buffer itself is fine and needs no work: it is `gpui_component`'s editor over a `ropey::Rope`,
with its own `Element` that lays out only `calculate_visible_range`, and a tree-sitter highlighter
that takes an edit delta and answers `styles(range)`. A twenty-thousand-line file costs O(viewport)
per frame there.

What costs is the code around it:

| Fact | Where | Cost |
|---|---|---|
| `let source = state.read(cx).value().to_string()` clones the **entire file** at the top of `drawn()`, before the branch — and the `Editor` branch, the general case, never reads it | `ui/viewer/mod.rs:112` | O(file) clone per frame, per open code tab |
| The markdown path re-scans the whole document for fences and clones the body again, unconditionally, before `TextView`'s own equality guard can short-circuit the parse | `ui/viewer/markdown.rs:65,78,235` | 2×O(file) per frame |
| The outline builds up to `MAX_DEFS = 2000` elements with `.children(rows)` | `ui/outline.rs:264` | O(defs) per frame (data itself is debounced) |

The outline's second tree-sitter parse (`ui/outline.rs:166`) is already marked as `ponytail:` debt
and is debounced and bounded; it is not part of this problem.

### The git view

| Fact | Where | Cost |
|---|---|---|
| The diff body builds every hunk header and every row into a `div` chain, up to the host's `MAX_ROWS = 10_000` | `ui/viewer/diff.rs:36`, `crates/ubiq-host/src/files/diff.rs:49` | O(rows) per frame — the worst single number in the interface |
| Commit history builds every visible commit row | `ui/git/history.rs:44` | O(page) per frame |
| `visible_commits()` allocates a fresh `to_lowercase()` for the summary, author and short id of **every** commit, every frame, whether or not a filter is typed | `state/git.rs:373` | 3n allocations per frame |
| `lanes()` maxes over all commits every frame just to size the gutter | `state/git.rs:389` | O(n) per frame |
| Refs re-filter the whole vector once per section — five passes a frame | `state/git.rs:353`, `ui/git/refs.rs:36` | 5×O(n) per frame |
| The working tree re-filters `entries` for conflicted, staged and unstaged, plus a fourth pass for `staged_count` | `state/git.rs:237`, `ui/git/changes.rs:59,240` | 4×O(n) per frame |
| History pagination is wired one way and dead: `GitView` writes `log_cursor` and `log_done` and nothing ever reads them to ask for page two, so history stops at a hundred commits | `state/git.rs:300`, `app/git.rs:44` | correctness, not speed |

The host side is already right: lanes are assigned once per page in
`crates/ubiq-host/src/git/graph.rs`, the diff is parsed once and capped, and the log message
already carries a cursor. Nothing in this proposal moves work across the bus.

## 2. The shape of the fix

Three things, cheapest first, and each is worth landing alone.

**Phase 0 — stop doing work nobody asked for.** No new abstraction, no new element, a handful of
lines each: move the `source` clone inside the branches that read it; precompute the lowercase
haystack once on `CommitRow` where `commit_rows()` already formats the date; group refs and
working-tree entries once when the reply lands instead of per section per frame; cache `lanes()` and
the staged counts beside them; replace `has_subagent`'s scan with a set built the way
`tool_block_index` already is. This is the whole of §1's "per frame" column that is pure waste, and
it is a day's work with no design in it.

**Phase 1 — virtualize the lists.** Both tools are already in the tree at the pinned revision, and
adding neither is a dependency change:

- **`gpui::uniform_list`** for fixed-height rows. The precedent is `ui/logs.rs:161`, which is
  exactly this shape: a captured `Arc` of rows, a `range` closure, `.track_scroll(&handle)`.
  Diff rows are already `px(ROW_TEXT + 6.)` tall (`ui/viewer/diff.rs:216`), commit, ref, change and
  outline rows likewise — so `uniform_list` fits all five with no measurement.
- **`gpui_component::{v_virtual_list, VirtualListScrollHandle}`** for variable-height rows — the
  `VirtualList` documented as gpui-kit's, vendored here under `gpui-component` at rev `df1d07b` and
  re-exported from its `ui` crate. It takes an `Rc<Vec<Size<Pixels>>>` of per-item sizes and an
  `Entity<V>`, which is what a chat transcript needs and `uniform_list` cannot express.

The diff body is where this lands first: flatten `hunks` into one `Vec<Row>` of header and line
rows once when `FileDiff` arrives, hand `uniform_list` its length, and ten thousand elements a frame
become thirty. The transcript is the second: `v_virtual_list` fed by the heights `TranscriptScroll`
already measures replaces the hand-rolled `Built`/placeholder machinery *and* removes the
forty-block floor, which is the only reason a short conversation is slow at all.

**Phase 2 — make the clock quieter.** Even a fully virtualized window still rebuilds every panel on
every token while the root is one entity. Two steps, in order:

1. **Cheap and immediate:** in `wire.rs`, skip `cx.notify()` for a `ConversationUpdate` whose
   `agent_id` is not visible in any dock panel — the record is updated either way, and an off-screen
   delegate stops driving frames. A coalescing window on the visible case (one notify per frame
   rather than one per delta) is the same edit.
2. **The real answer, and a decision:** give each open conversation its own `Entity`, so a token
   notifies one transcript instead of the window. This is a structural change to `AppState` and
   belongs in the decision register with what it costs, not in a phase-0 patch.

## 3. What this proposal does not do

- **No change to the wire.** Every message family stays as it is; the diff cap, the log cursor and
  the host's lane assignment are already the right shape.
- **No terminal work.** Panes are `gpui-terminal`'s and out of scope.
- **No editor engine work.** `gpui_component`'s editor already virtualizes and highlights by range;
  the fix there is deleting a clone, not writing a renderer.
- **No new dependency,** and no new abstraction over the two list elements. A shared row helper is
  worth extracting only after the third call site wants the same one.

## Next steps

- Phase 0 is implemented, per area: the buffer is cloned only in the branches that read it, the
  markdown fence scan is cached behind a length-and-hash fingerprint, `GitView` computes each
  commit's search haystack and the lane count on the reply and groups refs and the working tree in
  one pass, and `Conversation` holds a block index that makes `visible_blocks()` a shared `Arc` and
  `has_subagent()` a set lookup, with the streaming tail rendered once rather than per frame.
  `features/workbench.md` owns those facts.
- The history's pagination has a caller: a `Load more commits` row at the foot of the list sends
  `ProjectGitLog { cursor: log_cursor, .. }` through `AppState::load_more_git_log`, so history goes
  as far as it is read. `G129` and `G210` are closed with it, and
  [`tech/version-control.md`](../../tech/version-control.md) states the paged walk end to end.
- Phase 1 is implemented: `uniform_list` for the diff body, the commit history, the three change
  lists and the outline; `gpui_component::v_virtual_list` for the transcript, whose `WINDOW_MIN`
  floor, `Built` rows and placeholder machinery are deleted. Two shortcuts it left are `G221` (the
  diff's assumed monospace advance) and `G222` (the flatten memoised where it is read).
  Refs are deliberately not virtualized — `G220` says why, and it is a trap worth reading before
  reaching for one `uniform_list` per section.
- The rule is in [`tech/ui-and-design.md`](../../tech/ui-and-design.md), with the consequence beside
  it: a uniform row cannot grow, so a long diff line is scrolled to rather than wrapped.
- Phase 2 step 1 is implemented, and the coalescing is the frame's rather than a timer's:
  `Conversation::notify_due()` is true for the first delta of a burst and `drawn()` opens the next
  window. Step 2 — one `Entity` per conversation — is not, and is `G219` rather than a decision
  row: nothing has been chosen to record, and where the split stops is the open question.
- Filing this document is `P14` in `_meta/feedback.md`.
