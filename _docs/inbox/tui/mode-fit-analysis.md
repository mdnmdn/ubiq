---
id: inbox-tui-mode-fit
title: Which modes a terminal can draw — the scorecard
kind: inbox
status: proposal
summary: "The workbench's nine rail modes and its shared surfaces scored against a monospace terminal grid — six tests a screen must pass, the verdict and the include/exclude list per mode, the two hard noes (the Teams graphs, the Sink), the one screen dropped for a superior incumbent (Git), the one the seed underrated (Tasks), and the medium-advantage rule that the TUI delegates to its own incumbents rather than porting them. Consequence: the TUI's own command surface is a palette plus one key per screen, not a rail."
read_when: you are deciding what a terminal interface draws, what it drops, or why a mode was left out
updated: 2026-09-24
depends_on: [inbox-tui, feat-workbench]
---

# Which modes a terminal can draw — the scorecard

## Purpose

The rail selects between nine modes, and a second interface renders its own chrome rather than
borrowing the window's. This document scores each of the nine, plus the shared surfaces that sit
under them, against the medium a TUI actually has: an 80–200-column grid of monospace cells. The
seed proposed three areas of the workbench and dropped a fourth; this is that decision re-run
against the workbench as built.

## The six tests

A screen survives the move to a terminal when it passes enough of these, and fails when the second
one bites:

1. **Text-shaped.** The screen is tables, lists, prose or forms. A grid of cells is a rendering
   model, not a limitation, for anything that is already text; it is a rewrite for anything drawn.
2. **No superior terminal incumbent.** Where the user's terminal already ships a strictly better
   answer — a CLI, tig, lazygit, `less`, `rg` — Ubiq should not compete.
3. **Keyboard-steerable.** Every gesture the doc names — hover, drag, right-click, double-click,
   wheel — must become a key or a leader-prefixed chord. A screen whose meaning half lives in hover
   does not survive.
4. **Host-owned facts.** What it draws comes from the host, so an unprivileged second client can
   redraw it. A screen made of the window's own arrangement can only be copied, and the copy drifts.
5. **The medium is the SSH session.** The mode has to be worth having on the machine at the far end
   of a pipe; it must not assume a workstation, a GPU, or a mouse.
6. **The medium's advantages are used, not fought.** The window had to build every capability it
   shows; the terminal already ships whole families of them — a pager, an editor, a fuzzy finder, a
   multiplexer, a git client. A mode that re-implements what the shell does first, and better, fails
   the way Git failed; a mode that leans into what a TUI does best — one thing at a time, realtime
   streams, keys, a pipe out to the rest of the shell — is the one worth having on a far machine. By
   this test the TUI delegates where it is worse and stays in-app only where its answer feeds host
   state; the delegation table lives in [tui-ecosystem.md](./tui-ecosystem.md).

Every mode below ends with its **Include** and **Exclude** lists: what a terminal interface draws,
and what it deliberately does not. The lists are exhaustive about the mode's own surface and silent
about anything no screen — window or terminal — draws.

## The modes

### Control — fit: good

Both pages — the five host readings and the usage meter — are a polled table: `ListStats` is the one
thing the interface asks the host for, on a two-second loop, and the doc's own rule is no bar and no
percentage anywhere on the page. There is no gesture beyond a tab strip. `top` is the incumbent and
it is not a substitute — this is Ubiq's own readings, from Ubiq's own coordinator. The most literal
phase-0 screen the proposal could draw, and nothing in the current tree weakens the seed's claim.

- **Include:** the two-page tab strip and both pages whole — the five readings as a label/value
  table, the usage meter as a fixed-column table, rows in the current host's ordering. The page is
  phase 0 of the release scope.
- **Exclude:** the window's drawn readings, and there are none — this screen is text that happens to
  be drawn with a GPU, which is the strongest form of the fit argument. The one thing the window
  refuses and the TUI inherits the refusal of: bars and percentages.

### Sink — fit: poor

The kitchen sink's twelve fixture pages exist to exhibit the window's drawing — the style reference,
every special viewer, the A2UI surface, the teamsim graph testbed — and it "asks the host for
nothing": it is the window's own bench. A terminal has no use for a page that shows GPUI primitives
there is no GPUI, and a fixture bench travels with the interface it exhibits. Not a release screen by
any phase. Its one transferable idea is the fixture-page habit itself: a TUI kit earns its own `--kit`
page on day one, as the window's did.

- **Include:** the habit, not the pages — one `--kit` fixture page for the TUI's own widgets, exactly
  the shapes the window's bench exhibits for its own.
- **Exclude:** all twelve fixture pages — the style reference, every special viewer, the A2UI
  surface, the on-device buffers, the script scratchpad, the teamsim testbed. They exhibit a GPUI
  nothing here draws, and a fixture bench does not travel to another interface.

### IDE — fit: good for the explorer, search and the text viewer; partial as a whole

The explorer tree with its filter and its git-letter badges, the file picker and project search are
the most terminal-native shapes in the product — the ranger/nnn/fzf idiom exactly. The text viewer
with syntax colour is `less` plus a highlighter, which is a solved field Ubiq merely joins. What does
not travel is the rest of the viewer family: Markdown and Mermaid previews, images, Excalidraw
scenes, the web-panel bridge — every viewer that is not monospace text — and editing, in the first
cut. The seed drew exactly this line and the current tree does not move it; the IDE feature doc adds
drag-to-group tabs and right-click row menus to the gesture total, all of which become keys.

- **Include:** the explorer tree with its filter and git-letter badges, the file picker, project
  search with its result list, and a read-only text viewer with syntax colour; the outline panel as a
  text list beside a document; every context-menu row as a palette entry. Editing waits for a later
  phase and a separate argument.
- **Exclude:** every viewer that is not monospace text — Markdown previews, Mermaid, diagrams,
  Excalidraw scenes, the web-panel bridge, images and the image annotation toolbar — and the
  markdown minimap. Tab drag-to-group becomes a buffer switch; hover tooltips die; the open-file
  write path is out of the first cut.

### Git — fit: good as a screen, and dropped anyway

The screen is refs, paged history with painted lanes, the three change lists and a diff — mostly
tables and prose, and `git log --graph` already paints lanes in ASCII. The gesture list is long
(right-click on path, commit, branch; cmd-click to compare) and every one of them is a key in the
incumbents. This is the strongest confirm of the seed's drop: the terminal the TUI runs in *already
ships* git, tig and lazygit, and all three are better than what Ubiq would draw in eighty columns.
The one thing Ubiq draws that the incumbents do not — the multi-repository lane engine, lanes across
every repository inside a project — is precisely the part not worth a terminal port. Keep Git out.

- **Include:** nothing. Not one screen of the mode travels.
- **Exclude:** the refs explorer, the paged commit history with its painted lanes, the conflicted,
  staged and unstaged change lists, the commit box, the diff under them, the lane engine and the repo
  strip. Delegated whole: git, tig and lazygit are installed in the same terminal this interface
  draws in, and the TUI's own git story is one help line — what the incumbents do, run them.

### Agents — fit: good

The columns are a transcript and a composer over host-owned conversations, and the transcript is a
fold the window keeps of host deltas — which is the one real sharing problem in the whole proposal
(`G342`), and the reason the seed schedules a crate move at phase 3. Everything drawn is text:
Markdown blocks, tool calls with diffs, thinking, permission prompts, the "needs you" strip, a
lifecycle glyph, context and token readouts, the delegate list. The gestures — tab drag-to-group,
hover summaries, the three-dots menu — are exactly the set that becomes leader-prefixed keys, and
the run-and-answer an agent loop is the release that justifies the rest (seed §10, unchanged). If a
developer will not talk to an agent from a terminal over SSH, nothing after it should be built.

- **Include:** one conversation at a time — a terminal has no eight columns — the transcript with its
  tool blocks and folds, the composer, the permission prompts and the pending-ask answer, the "needs
  you" strip, the lifecycle glyph, the context and token readouts, the delegate list, the roster of
  agents the host broadcasts, and the new-agent/harness picker. Switching conversations is a key, and
  the bench of agents no screen shows is a list.
- **Exclude:** the parallel columns and their tab chrome (a key, not a layout), drag-to-group,
  hover summaries, inline links, and the three-dots menu (its actions become palette entries). The
  window's column arrangement is the interface's own fact anyway, so a second interface would have
  nothing to copy but its own.

### Teams and `[Teams]` — fit: poor

Cards on a dotted ground, fenced into task containers, arranged by twelve computed algorithms, drawn
with hexagonal status marks and drag-trail grains; positions are "the interface's own fact, written
on the spot" and every arrangement is a projection of the window's canvas maths. A spatial graph is
the opposite of a monospace grid, and a graph's value *is* spatial arrangement. There is no terminal
incumbent because there is no terminal version of this; there is also no demand for one — the people
looking at the graph are looking at the window. Not in any release. `[Teams]` adds its inspector and
tasks drawer to the same canvas; same verdict.

- **Include:** nothing drawn. The only *reading* either graph offers that a terminal wants — who
  spawned whom — is the Tasks board's missions-and-delegates tree, and that is where the TUI gets it.
- **Exclude:** the canvas and its dotted ground, the twelve arrangements, the hexagonal status marks,
  the drag-trail grain, the task fences, the inspector and the tasks drawer, the span and "hide done"
  filters. The positions are the window's own facts ("the interface's own fact, written on the spot"),
  so a second interface has nothing host-owned to draw even if it wanted to.

### KB — fit: partial

A source is a list with add/edit/remove through settings-shaped forms, which is text. What the mode
renders in its centre is "the same viewer the IDE uses," so the drop line is identical to IDE's:
prose travels (the `glow`/`less` audience), a diagram or an image in the flow of the document does
not. KB is IDE, subdivided by source — the list-and-prose half is cheap once the IDE text viewer
exists, and the rendered-picture half is dropped with the rest of the viewer family. The win is a
project's knowledge base readable from the machine the host runs on, which is a real SSH story.

- **Include:** the source list with its name, kind, filter and access markers; add/edit/remove of a
  source through settings-shaped forms; the name-glob filter; and a document's prose read through the
  IDE text viewer — the glossary-and-wiki case is exactly the `glow`/`less` audience.
- **Exclude:** a diagram or an image in the flow of a document (the web-panel bridge), the
  annotation toolbar a project picture gets, and any renderer past monospace. The drop line is IDE's,
  stated once here and not restated per viewer.

### Tasks — fit: good, and the seed underrated it

A column per status, a card per task, one find field, labels as a filter, and a panel that reports
one task and edits it a field at a time — columns, cards and forms are the kanban idiom the terminal
already speaks, and the task set is the host's, so the board is data end to end. The drags (card to a
gap or column, a wheel over a lane) become keys; the one geometric thing, the plan surface's minimap,
is dropped exactly as the seed dropped it. The seed left the board out of the first three releases on
the argument that a board needs its own case; the scorecard now says that case was written before the
board was built — it is the cheapest mode after Control, and worth a phase-1 *option* rather than a
first-release commitment.

- **Include:** the columns as status lanes, the cards with their titles and labels, the find field
  and the label filter, the task panel that reports one task and edits it a field at a time, the
  missions-and-delegates list (the reading Teams draws as a graph), and the new-task form. Card
  placement and a card's summary each become a key.
- **Exclude:** drag-to-a-gap and drag-to-a-column placement (a key, not a gesture), the wheel-over-a-lane motion,
  the plan surface's minimap and its thread rail, and the plan's geometry. The board is host data end
  to end, so nothing excluded here is a host loss — it is a gesture the TUI replaces.

## The shared surfaces — not modes, but they are most of the TUI's skeleton

- **Panes — the substrate, not a port.** A pane is already a terminal grid; a TUI is the machine
  that draws grids. This is the expensive part of the whole proposal (the seed's §5) and the scorecard
  only moves it earlier or later, never off the schedule.
  - **Include:** the alacritty grid walked into a ratatui buffer, the leader key, resize into the
    harness, the redraw budget, and a pane's own multi-pane tiling behind the leader — the tmux
    answer, with split, zoom and swap.
  - **Exclude:** the window's dock — regions, tab groups, edge drops, divider drags all become
    leader keys — and the pane chrome the window draws. A pane's re-parenting stays a coordinator
    fact (`AdoptProject`), reachable by a key, never by a drag.
- **Sessions and workspaces — fit: good, by inheritance.** The model is tmux's by name: a session is
  a named grouping that owns a folder, a workspace is one running agent, attach and detach rehome the
  view. A TUI inherits the mental model whole, and its session keyboard answers are already the
  multiplexer's.
  - **Include:** the session list (`tmux ls` is the shape), attach, detach and spawn-in-session, and
    the workspace readouts the host reports. The window's own session story is the mode's.
  - **Exclude:** the cross-window adoption flows and the onboarding chrome; those are window facts,
    and a terminal's "one host, one client" needs no adoption ceremony.
- **Notifications — fit: good.** A host-owned history, level-coloured one-line rows, mute rules; the
  bell becomes a status-line glyph and a key. Trivial, and it is the remote-debugging story a build
  box needs.
  - **Include:** the history list with level colour, origin and optional link, unread accounting, and
    mute-rule editing through a form.
  - **Exclude:** the titlebar bell and its arrival flash (a status-line ink and a key instead), and
    the desktop relay — the TUI's "desktop" is the terminal it draws in.
- **Logs — fit: good.** The console is five thousand uniform one-line rows with a subsystem selector
  and a level floor — the canonical TUI surface (`tail -f`), two controls, no gestures. A terminal
  interface that cannot show logs is not fit to attach to a remote host.
  - **Include:** the ring whole, the subsystem selector, the level floor, follow and clear — the
    whole screen, because it is a filter over one-line text.
  - **Exclude:** the loudest-record tab dot (a row mark, not a screen), and nothing else — this is
    the cleanest fit in the tree.
- **The chrome — fit: good, and it collapses.** The status bar, the ⌘K navigator, project picker,
  settings forms, the file picker and the tool list are each a list, a form or a line, and together
  they are how a TUI is navigated. The ⌘K navigator is the closest thing the window has to a command
  palette — which is what a terminal interface is, structurally. The unbundling of "a mode's chat tab
  sits in a dock region" has no TUI meaning: a terminal has one region, the screen, so a conversation
  about a project *is* the Agents screen under another name.
  - **Include:** the status bar as a read-only line, the palette (built on the navigator's own
    grammar), the project list and picker, project and application settings as forms, the file picker
    as a forest, the tool list, and the chat-tab idea renamed: a project's conversation opens the
    Agents screen, no dock involved.
  - **Exclude:** the dock and all regions, the window-face and project-tint badges (colour survives
    as tokens, not as tiled squares), hover hint marks, and the sign-in modal's embedded full-screen
    terminal — in a TUI that is just a pane.

## The scorecard's consequences for the seed

- Confirms the seed's three-area scope — Control, reduced IDE, agents and conversations — against the
  nine-mode rail it never saw.
- Confirms the Git drop against a screen the seed never saw, for the same reason and a sharper one
  (lazygit).
- Names the two `no`s the seed left implicit: the Teams graphs and the Sink.
- Reopens Tasks as the one mode the seed's "first three releases" line has to explain.
- Restates the mode the seed never named: KB is IDE, subdivided.

## Related docs

- [`workbench.md`](../../features/workbench.md) — the nine modes, their gestures and which facts are the window's
- [`ratatui-tui-proposal.md`](./ratatui-tui-proposal.md) — the proposal this scorecard feeds, and its §3 area table it corrects
- [`tui-release-scope.md`](./tui-release-scope.md) — the first release these verdicts translate into
- [`stats.md`](../../features/stats.md) — the one poll a control screen makes
- [`workbench-tasks.md`](../../features/workbench-tasks.md) — the board the seed underrated