---
id: feat-workbench-git
title: Git mode — refs, history and changes
kind: feature
status: draft
summary: The rail's Git mode — the refs explorer of branches, remotes, tags, stashes and submodules with the repositories a project holds above them, the paged commit history with its painted lanes, the conflicted, staged and unstaged change lists with the commit box, the diff under them, and the strip of icon actions and the HEAD pill.
read_when: you are changing the Git screen — its refs, history, change lists, commit box or diff — or the strip above it
updated: 2026-10-05
verified: 2026-10-05
code_anchors: [crates/ubiq/src/state/git.rs, crates/ubiq/src/app/git.rs, crates/ubiq/src/ui/git/mod.rs, crates/ubiq/src/ui/git/refs.rs, crates/ubiq/src/ui/git/history.rs, crates/ubiq/src/ui/git/changes.rs, crates/ubiq/src/ui/git/diff.rs, crates/ubiq/src/ui/viewer/diff.rs, crates/ubiq/src/ui/ribbon.rs, crates/ubiq/tests/git.rs]
depends_on: [feat-workbench, tech-ui, tech-version-control]
review_cycle: monthly
---

# Git mode — refs, history and changes

## Purpose

Git is the rail mode that shows what version control knows about a project, whole: the refs on
one side, the uncommitted work on the other, and the commit history between them. It exists so
that reading a repository does not mean leaving the window the agents are working in. How a
repository is actually read — discovery, the worker, the lane engine — belongs to
[Version control](../tech/version-control.md); this document is the screen.

## Behaviour

**The Git screen is what version control knows, whole.** The refs explorer (branches, remotes, tags,
stashes, submodules) in the left region, the uncommitted changes and the commit box in the right,
the commit list in the centre. A first visit opens the left and right regions onto those panels and
puts the history in the centre; the bottom pane region stays shut. A saved arrangement from before
the refs and changes panels existed is not a first visit, so it does not have them injected into it:
the region it left open and empty is closed instead, the same rule `D156` gives every mode's side
furniture — Tasks and Agents included — rather than one of Git's own, and the titlebar's side-panel
switch, the same one the IDE uses for the explorer, is what brings that side's furniture back. The comparison is a panel of its own and comes forward when a changed path is picked. It is the same facts the explorer's badges and the status bar's branch carry, at
the size they can be read at: the tree answers "is this file changed" and this screen answers
"what has this repository been doing". A red `experimental` ribbon sits on the screen's top-left
corner for as long as the rail is on Git.

**The strip over the panels is icon buttons and the HEAD pill.** Every button is an icon with a
tooltip. `Checkout` checks out whatever the refs panel has selected — faint with nothing selected,
same as with no repository. Fetch all, pull and push write, including an `ssh` remote
(`git@host:path`). Branch raises a
popover with a name field, and Enter or `Create` makes that branch at HEAD and checks it out. Stash
shelves the uncommitted changes, untracked files with them. Undo takes the last commit back into the
index and is refused on a first commit. All three refuse while a merge or rebase is in progress, and
the refusal reads as the screen's error. With no repository the buttons but refresh draw faint and
take no click, and keep their tooltips. The pill beside them is the selected repository's branch,
tracking counts and any in-progress operation. A refresh asks the host again. What is typed into the commit box is kept with the project, until the commit it
was written for succeeds: the working-tree reply that follows empties the box and drops the amend
flag, rather than leaving a message on screen that is now history.

**A write that could lose something asks first.** Discarding a change, a hard reset, deleting a ref
and reverting a file to a commit each raise a confirm dialog (`GitConfirm`, `state/git.rs`) before
they send anything; accepting is what sends the write, forced where the question was about forcing
one — a delete that is answered deletes an unmerged branch rather than being refused a second time.
Reverting a file is in that list because what it overwrites was never committed: it is in no
history to be read back out of. A merge, cherry-picking, reverting a commit and a soft or mixed
reset all write straight away — those are commits on top of what was already there.

**A checkout is attempted unforced and only then asks.** Checking out a branch or a tag sends
`force: false` whatever the working tree holds, because a modification that does not collide comes
across the switch, the same way `git checkout` carries it. The confirm is raised from the host's
*refusal* instead: the UI holds the rev of the checkout it is waiting on (`checkout_in_flight`) and
a `GitError` naming a checkout conflict turns into the dialog, whose accept is the only thing that
sends `force: true`. Any other failure of that checkout — an unresolvable rev, say — reads as the
screen's error, so a forced checkout is never offered over a failure that was not about
overwriting work.

**A project holding more than one repository shows one at a time.** The refs panel opens with a
Repositories section listing the project's own repository and each nested one it manages; a project
with a single repository has no such section. Each row carries a dot when the repository has pending
changes, a `!` when it has conflicts, and the current branch's `↑ahead ↓behind`. Picking a row makes
it the screen's repository: the refs, the history, the change lists, the commit box's target and
every write are that repository's only, never mixed with another's, and what belonged to the one
before — its selection, its diff, its history cursor — is dropped. The choice is kept per project
for the life of the window and is not saved.

**The uncommitted row is the top row of the history, and it is there only while something is
uncommitted.** With no changes the row is not drawn; otherwise it reads `Uncommitted changes (N)`.
It is selected the same way a commit is and is what the screen opens on, and a click on it reveals
the Changes panel — opening its region if that is shut, adding the panel and focusing its tab.
Picking a commit points the panel beside it at that commit instead. The panel says what the log said and no more — a commit's own file list
needs a log the git family does not carry.

**The change lists are conflicted, staged and unstaged, each a collapsible section with its own
counter.** A path with an index change is in Staged; a path with a worktree change is in Unstaged —
a path changed both ways appears in both, once each. Conflicted draws first and only when it has
something to say; Staged and Unstaged stay on screen even at zero, so the split itself is always
visible. A click on a section's heading, drawn the full width of the panel, shuts or reopens it; the count
keeps reading while it is shut. `+` on the right of a row stages that path and `-` unstages it,
right-justified the way the section heading's own `+` (Unstaged, stage all) and `-` (Staged, unstage
all) are. A conflicted row has neither. A row takes the colour the explorer paints the same path
in, so the two never disagree. Rows run the panel's full width with the `+`/`-` at its right edge,
and hovering one names the path relative to the project.

**A search field, full width above the lists, narrows the panel to paths that match, as it is typed.** It
filters conflicted, staged and unstaged together and the range comparison's own file list the same
way, case-insensitively over the path; a path a filter drops keeps its stage, its `+`/`-` and its
section, so a search never changes what a click on a surviving row does. The same field appears over
both views the panel draws — the working tree and the two-commit range — because a search over
changed paths is one idea wherever the panel is showing them.

**Picking a changed path compares it, and the comparison is the host's.** The pane under the
history draws the hunks `DiffProjectFile` answers with, through the same renderer a diff tab uses.
A staged row is compared against the index and an unstaged or conflicted row against HEAD, because
the file family offers those two and no index-against-HEAD. The pane shuts rather than the history
shrinking, and switches between unified and side by side.

**Cmd/ctrl-click a second commit to compare it against the first.** The panel beside the history
lists the paths that differ between the two — oldest to newest, no `+` / `-`, because a range is a
read — and picking one asks `ProjectGitChanged` for the diff between those two revisions rather
than against the working tree. A plain click on any row, including the uncommitted one, drops the
pair and returns to that row's own view.

**A right-click on a changed path, a commit or a branch raises that row's own menu**, the same
`kit::context_menu` every other right-click on the window draws with, and every row in it does
something. A changed path offers stage, unstage, discard (behind the confirm), opening the file,
copying its path, and — once the history panel has a real commit selected — restoring the path from
that commit, labelled with its short id (`Revert to 4f2a9c1`), behind the confirm; dead with
nothing selected there. A
commit offers copying its SHA, cherry-picking it, reverting it, and resetting `HEAD` to it soft,
mixed or hard, the last behind the confirm. A local branch or a tag offers checkout, merge (dead
for the branch already checked out, and dead on every row while `HEAD` is detached — there is no
branch to merge onto) and delete (behind the confirm), plus copying its name; a
remote-tracking branch offers checkout and the copy only — merge and delete stay dead, because
neither means anything against a ref this screen does not own the way it owns a local branch or a
tag. A stash or a submodule row offers only the copy — checkout too stays dead, because neither
names a rev this screen checks out.

**Whatever the screen is waiting on is named, not just spun.** `GitView::in_progress()` reads
whichever of a write, a history page or a two-commit comparison is still in flight and the toolbar
prints its word — `Staging…`, `Committing…`, `Loading history…` — beside the changed-paths count,
so a click that has not answered yet is a state the user can read rather than a guess.

**A diff's rows are one height, so a long line is scrolled to rather than wrapped.** The body is a
virtual list over the hunks flattened into one row per header, line or side-by-side pair, which is
what makes a ten-thousand-row comparison cost a viewport; a uniform row cannot grow, so the list is
as wide as its widest line and a line past the pane's edge is reached by scrolling sideways. A
minified file reads as one very long row rather than as a wall of wrapped text.

**The history and the refs are real.** The branch list, the tags and the stashes come from one
`ProjectGitRefs` reply; the remotes and the submodules ride with the overview. Local branches and
remotes draw as a tree split on `/`, so `feature/things` nests under `feature` and
`origin/feature/things` nests under `origin`. **Local branches** is the one section open when the
sidebar first draws; remotes, tags, stashes and submodules start shut, because a branch list is
what the sidebar is opened for and the other four are long enough to push it off screen. At the top
level of the local and the remote tree, the current branch sorts first, then `main`, `master`,
`develop` and `dev` in that order, then everything else alphabetically — only a branch with no
folder of its own is pinned this way, so `main` inside a folder stays where its folder puts it. A
single click on a local or a remote-tracking branch row jumps the history to that branch's tip
without filtering it: the tip's row is selected and scrolled to, under whatever filter is set, and
only when that row is not drawn (hidden by the search or Mine, or older than the loaded pages) are
the filters cleared and the log paged until it is, the way `GitView::jump_ref` waits for it —
revealing the commit list if it was put away; a double-click checks it out (the toolbar's own
confirm rule applies). A tag, stash or submodule row keeps its own click, which scrolls the list to
the commit it points at. A search field above the sidebar's sections narrows every one of
them — local branches, remotes, tags, stashes and submodules — to names that match, as it is typed;
a section a search leaves empty is not drawn, and searching opens every folder the tree would
otherwise keep shut, so a match is never left behind a twisty. The commit log pages
in from `ProjectGitLog`, oldest page first and newest commit within it, and pages as far as it is
read: the view stores `log_cursor` and `log_done` (`state/git.rs`), and the list itself asks for the
next page as the reader's scroll comes within reach of the bottom of what has loaded, before they
reach the edge. The foot of the list carries a `Load more commits` row as a fallback and a status
line: it sends the same request by hand for the rare frame nothing has scrolled near the end yet,
and reads `Loading more…` while a page — scrolled or clicked for — is in flight. The history's search
matches a commit's message and SHA, a searchable picker walks one branch instead of HEAD, and
`my commits` keeps only what the signed-in user wrote; every filter clears together. A commit's
lane is real: the host allocates it from actual parent ids, and the interface derives from the
same `lane`/`merges`/`parents` the bus already carries which lanes are live above and below each
row and which converge at it — the joins that turn into elbows and the births that open a new
column. A merge's extra parents claim their own lanes for the lines drawn behind it rather than
the one hollow dot a parent count could offer. The graph is painted as one canvas per row: a
straight line where the host says a lane lives, an elbow where a lane ends at a dot or a merge's
extra-parent lane is born, and a hollow ring for a merge's dot. The rows read as a table: the
gutter holds the graph, a Message label takes the flexible column, and Author, When and SHA are
fixed on the right, each named by a faint header row once the history exists and each aligned with it
on every row, the uncommitted one included; a long message truncates rather than pushing the columns. Lane state carries
across pages, keyed by the cursor and filters the next one arrives with, so a
branch does not visually collapse and reopen at a page boundary.

## Contract

**The git family is what the Git screen speaks:** `ProjectGit` and `RefreshProjectGit` for the
overview and the working tree, `ProjectGitRefs` for the sidebar, `ProjectGitLog` for the history,
page by page, and `WriteProjectGit` for stage, unstage, commit, fetch, pull, push, branch, stash,
undo, checkout, merge, delete-ref, discard, restore-file, cherry-pick, revert-commit and reset.
Each carries a `repo`, empty for the project's own, and the screen discards a reply whose
`repo` is not the one it shows. `GitError` answers a refused write the way it already answers a
refused read — `last_error` holds it under the commit box regardless of which op sent it. The full
family is [`../tech/transport-contract.md`](../tech/transport-contract.md).

## Implementation

`state/git.rs` is the Git screen's view of one project's repository, held on `OpenProject` beside
the graph's and the board's: which sidebar sections are shut, which ref and which commit are
selected, what is typed in the search and the commit box, which changed path the diff is about and
what it is compared against. It holds no working-tree records — `group_changes()` takes the
`GitEntry` pairs the host sent, which `OpenProject::git_entries` keeps whole beside the projection
the explorer got, and buckets them into `ChangeGroups`' conflicted, modified and untracked in one
pass, each path once — and `settle()` drops a selection whose path has gone clean.
`grouped_refs()` does the same for the sidebar's five sections. **What a frame reads, a reply
 computes:** `set_commits()` and `extend_commits()` build each commit's search haystack, the graph's
 per-row cells and the lane count as they land, so `history()`, `visible_commits()` and `lanes()`
 are reads rather than a scan over the page, and the count of staged paths is passed to the commit
 box rather than derived again there. `history()` pairs the rows a filter leaves with the
 `GraphCell` the loaded walk gave each — a search hides rows, it does not relayout the graph. The
 graph's cells are a projection of the host's lane data, not an interface topology.
`Side::base()` is where a list's comparison base is decided, and `RefRow` and `CommitRow` are
built from the host's answers by `ref_rows()` and `commit_rows()` in `state/git.rs`. Its four
widths and the graph's lane pitch are constants there, the way the board's and the columns' are
theirs. `last_error` holds a `GitError::Failed` reason until the next working-tree reply.

`ui/git/` draws it, one file per panel: `refs.rs` the left region's sections and their rows — the
 file list's own row chrome, so a ref reads the way a path does — `history.rs` the search, the
 graph's painted lanes, the column header and the commits, `changes.rs` the right region's three lists, the `+` / `-` on each path and the
commit box, `diff.rs` the comparison under the history, which hands the hunks to
`ui/viewer/diff.rs` rather than drawing them again. `refs.rs`'s `repositories()` draws the
Repositories section above the five ref sections, from `state/git.rs`'s `repo_entries()`; it is
absent when that returns nothing, and capped in height so a project of many clones scrolls it.
`git::toolbar()` is the strip itself,
painted by `ui/shell.rs` above the dock while the rail is on Git; it carries the HEAD pill, the
icon buttons — each a `kit::icon_button_tip`, an icon button that names itself in a tooltip and
draws faint and inert when disabled — checkout / fetch all / pull / push / branch / stash / undo, the
working-tree count and a refresh. `branch_button()` owns the `Branch` popover (`MenuId::GitNewBranch`):
a text field and a `Create` button, submitted by `submit_git_branch()`. `ribbon::experimental()` is
the red `experimental` band in that column's top-left corner — `kit::ribbon` at `TopLeft`, drawn
by `ui/shell.rs` while the rail is on Git. `AppState::write_git()` sends `WriteProjectGit`
against `git_repo_of()`'s repository; commit, fetch, pull, push, branch, stash, undo, checkout,
merge, delete-ref, cherry-pick, revert-commit and reset also ask for refs and a fresh log page,
while discard and restore-file — path-scoped, not history-moving — do not.
`AppState::checkout_git_ref()` is the one path every checkout affordance calls — the toolbar
button (`checkout_selected_git_ref()`, whatever the refs panel has selected), a double-click on a
branch row, and the ref menu's `Checkout` — so the rule lives once: it always writes `force:
false` and parks the rev on `GitView::checkout_in_flight`. `Message::GitError` is where the
question is asked — a `Failed` reason that `checkout_conflict()` recognises, with a rev parked,
raises `GitView::ask_confirm(GitConfirm::Checkout { .. })` and is not drawn as the screen's error;
anything else clears the slot and is. The confirm's accept is the only sender of `force: true`.
`AppState::pick_git_menu_action()` is every other menu row's own writer, one match arm per
`GitAction` — `Discard`, `RevertToCommit`, `Reset`'s hard mode and `DeleteRef` route through
`ask_confirm` the same way, and `DeleteRef`'s accept writes `force: true`; `Open` calls
`select_file()`, the same path the explorer's own `Open` takes, and is not a write. `GitView::confirm` and `AppState::confirm_git_action()`/`cancel_git_confirm()` are the
dialog's state and its two answers; `ui/shell.rs::git_confirm()` paints it with `kit::confirm_modal`
from the window root — a rung of the shared overlay stack, `Layer::GitConfirm` (`T-270`), rather
than painted from the toolbar the way `branch_button()`'s own popover is. Escape dismisses it on
the same footing as the pane's and the conversation's close confirms; `crates/ubiq/tests/dismiss.rs`
asserts its place in the peel order.
`OpenProject::git_repo` is the selected repository, set by `select_git_repo()`, which calls
`GitView::reset_repo()` and asks for that repository's overview, refs and first log page;
`git_overview()` is the selected repository's overview, where `open.git` stays the project's own for
the status bar and the explorer. `GitView`'s change lists are built from the selected repository's
paths only (the merged map holds every repository's), so a stage never lands in another index. `PanelKind::GitRefs` / `GitChanges` / `GitHistory` / `GitDiff` are the
four dock panels; they are drawn only in Git mode with a project, and `ModeLayout::default_for(Git)`
opens the left and right regions onto the first two. `AppState::queue_git_furniture()` puts them in
their home regions on a first visit, and `toggle_region()` fills an emptied left with the refs and
an emptied right with the changes. The history and the three change lists are `uniform_list`s over
their row counts; the sidebar's refs are the one list still built whole (`G220`). `ui/viewer/diff.rs`
flattens the hunks into one `Rows` of header, line and pair rows with the widest line's character
count beside them, memoised per comparison rather than rebuilt per frame (`G222`), and hands
`uniform_list` the length. The status bar's branch wording is `ui/status_bar.rs`'s
`operation_label()` and `capped()`, shared so the strip and the screen cannot say different things
about one repository.

`AppState` holds the screen's text entities — `git_search`, `git_branch_query`, `git_ref_query`,
`git_change_query` and `git_message` — mirroring each into the project's view through
subscriptions, with `sync_git_fields()` filling them back on the frame after a project swings in,
on the explorer filter's rule. The commit box is the one field mirrored while it is still focused,
and only to empty it: a landing working-tree reply that follows a commit clears `git_message` and
`amend` even with the caret sitting in the box. `state/git.rs`'s `ref_tree()` is what sorts a ref
section's top level — the current branch, then `TRUNK_BRANCHES`, then alphabetical — and what a
non-empty `ref_search` does to a section's shut folders: search bypasses `shut_folders` outright
rather than reading it. A first visit to Git reveals the history panel in the centre, with the left
and right regions open — refs on **Local branches** alone, the sidebar's other four sections shut —
and the bottom shut — `collapse_empty_regions()` leaves both regions' edges unarmed while
`queue_git_furniture()`'s refs and changes panels are still in the pending-panel queue, so neither
is put away before `settle_panels()` drains it. There is no per-frame refill behind that: Git's
furniture is queued exactly once, by `queue_git_furniture()` on entering the mode with no saved
blob, the same way `queue_kb_furniture()` and `queue_mode_furniture()` seed the knowledge base's,
the board's and the agents screen's own side panel (`D156`). A restored blob that predates the refs
or the changes panel is not a first visit and gets no injection: the region it left open and empty
is closed by `collapse_empty_regions()` instead, and the titlebar's side-panel switch — answered by
`mode_side_furniture()`, shared with `toggle_region()` — is what puts Git's furniture there on a
click. `select_git_ref()` reveals that panel if it was
hidden; `jump_to_git_ref()` selects the commit the ref points at and scrolls the list to it, clearing the
filters only when the row is not visible under them (`settle_git_jump()` finishes it after the log
page that brings the commit in).
`select_git_path()` reveals the diff panel. The IDE's right region stays shut on a first visit,
whatever `settle_persistent_chat()` attaches behind it. `git_view()`, `git_view_mut()` and
`git_entries()` are the accessors; `select_git_path()` is the one mutator that sends anything, and
it sends `DiffProjectFile` only when the selection actually moved. `refresh_git()` asks for the
overview and the working tree together. `ProjectFileDiffed` feeds whichever of the screen and a diff
tab was waiting on that path and base. The whole of it is tested without a frame in
`crates/ubiq/tests/git.rs`.

## Failure

| What happens | Result |
|---|---|
| The Git screen is opened on a folder that is not a repository | The toolbar says so, the lists are empty and nothing is drawn as clean. No error to dismiss |
| A repository is open and the working tree has nothing to say | The panel says there is nothing to commit, which is the answer clean gives and unread does not |
| A changed path is picked and the host has not answered yet | The pane says it is reading. The last comparison is never left under the new name |
| The path the diff pane is about goes clean | The selection and the comparison go with it, and the pane asks for another path |
| A commit is selected | The panel reports what the log said and states that a commit's file list needs a message the git family does not carry |
| A repository exists and cannot be read | Badges clear rather than freeze at the last good answer. A corrupt object database is the case this exists for |
| A working-tree reply is older than one already held | Discarded, so a slow walk cannot repaint what was true earlier |
| A harness exits | The pane stays, and the window asks the host to refresh that project's version control |

## Related docs

- [`workbench.md`](./workbench.md) — the rail, the dock and the panels this mode is drawn inside
- [`../tech/version-control.md`](../tech/version-control.md) — how a repository is read, and the lane engine behind the graph
- [`workbench-ide.md`](./workbench-ide.md) — the explorer's git badges, and the diff renderer both screens hand hunks to
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the git family on the wire
- [`../backlog.md`](../backlog.md) — what this mode still lacks
