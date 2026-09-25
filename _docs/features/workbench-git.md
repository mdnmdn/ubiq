---
id: feat-workbench-git
title: Git mode — refs, history and changes
kind: feature
status: draft
summary: The rail's Git mode — the refs explorer of branches, remotes, tags, stashes and submodules, the paged commit history with its painted lanes, the conflicted, staged and unstaged change lists with the commit box, the diff under them, and the strip that names the repository and what HEAD is doing.
read_when: you are changing the Git screen — its refs, history, change lists, commit box or diff — or the strip above it
updated: 2026-09-25
verified: 2026-09-25
code_anchors: [crates/ubiq/src/state/git.rs, crates/ubiq/src/app/git.rs, crates/ubiq/src/ui/git/mod.rs, crates/ubiq/src/ui/git/refs.rs, crates/ubiq/src/ui/git/history.rs, crates/ubiq/src/ui/git/changes.rs, crates/ubiq/src/ui/git/diff.rs, crates/ubiq/src/ui/git/repo_selector.rs, crates/ubiq/src/ui/viewer/diff.rs, crates/ubiq/src/ui/ribbon.rs, crates/ubiq/tests/git.rs]
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

**The strip over the panels names the repository and what HEAD is doing.** The repository selector
lists the project's repositories — the root, and a submodule or a nested one where the project has
one — and the pill beside it is the branch, the tracking counts and any in-progress operation. Every
git message is keyed by the project alone, so picking a row in the popover only closes it; the
screen still shows the root repository (`G270`). Fetch all, pull and push write,
including an `ssh` remote (`git@host:path`); branch, stash and undo take no click. A refresh asks
the host again. What is typed into the commit box is kept with the project, until the commit it
was written for succeeds: the working-tree reply that follows empties the box and drops the amend
flag, rather than leaving a message on screen that is now history.

**The uncommitted row is the top row of the history.** What is not committed yet is selected the
same way a commit is and is what the screen opens on; picking a commit points the panel beside it
at that commit instead. The panel says what the log said and no more — a commit's own file list
needs a log the git family does not carry.

**The change lists are conflicted, staged and unstaged, each a collapsible section with its own
counter.** A path with an index change is in Staged; a path with a worktree change is in Unstaged —
a path changed both ways appears in both, once each. Conflicted draws first and only when it has
something to say; Staged and Unstaged stay on screen even at zero, so the split itself is always
visible. A click on a section's heading, drawn the full width of the panel, shuts or reopens it; the count
keeps reading while it is shut. `+` on the right of a row stages that path and `-` unstages it,
right-justified the way the section heading's own `+` (Unstaged, stage all) and `-` (Staged, unstage
all) are. A conflicted row has neither. A row takes the colour the explorer paints the same path
in, so the two never disagree.

**A search field above the lists narrows the panel to paths that match, as it is typed.** It
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
`kit::context_menu` every other right-click on the window draws with. A changed path offers stage
and unstage today; a commit offers copying its SHA; a branch offers copying its name. The rest of
each menu is drawn and disabled — checkout, cherry-pick, discard and the like have no operation
behind them yet — so the vocabulary is visible before the write is.

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
click on a branch reveals the commit list if it was put away; a double-click scrolls the list to the
commit that branch points at. A search field above the sidebar's sections narrows every one of
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
fixed on the right, each named by a faint header row once the history exists. Lane state carries
across pages, keyed by the cursor and filters the next one arrives with, so a
branch does not visually collapse and reopen at a page boundary.

## Contract

**The git family is what the Git screen speaks:** `ProjectGit` and `RefreshProjectGit` for the
overview and the working tree, `ProjectGitRefs` for the sidebar, `ProjectGitLog` for the history,
page by page, and `WriteProjectGit` for stage, unstage, commit, fetch, pull and push. The full
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
`ui/viewer/diff.rs` rather than drawing them again, and `repo_selector.rs` the chrome-strip control
that lists a project's repositories when it has more than one, though selecting a row only closes
the popover (`G270`). `git::toolbar()` is the strip itself,
painted by `ui/shell.rs` above the dock while the rail is on Git; it carries the selector, the HEAD
pill, fetch all / pull / push, the working-tree count and a refresh. `ribbon::experimental()` is
the red `experimental` band in that column's top-left corner — `kit::ribbon` at `TopLeft`, drawn
by `ui/shell.rs` while the rail is on Git. Branch, stash and undo stay inert. `AppState::write_git()` sends `WriteProjectGit`; commit, fetch, pull and push also ask for
refs and a fresh log page. `PanelKind::GitRefs` / `GitChanges` / `GitHistory` / `GitDiff` are the
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
hidden; `jump_to_git_ref()` selects the commit the ref points at and scrolls the list to it.
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
