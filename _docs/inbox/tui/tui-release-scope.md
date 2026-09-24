---
id: inbox-tui-release
title: The terminal interface — first release, key map, phases
kind: inbox
status: proposal
summary: What a first terminal-interface release actually ships, against the workbench as built — a command palette and one key per screen instead of a rail, the screens in four releases, the kept-out list with a line of reasoning each, the keyboard model inside and outside a pane, and the decision and gap rows the whole set would leave in the register.
read_when: you are planning the terminal interface's phases, arguing its scope, or choosing what a phase cuts
updated: 2026-09-24
depends_on: [inbox-tui, inbox-tui-mode-fit, inbox-multi-attach]
---

# The terminal interface — first release, key map, phases

## Purpose

The [mode-fit analysis](./mode-fit-analysis.md) decides what a terminal can draw; this document
decides what a first release should draw, in what order, with what keyboard, and what it would leave
in the register. It supersedes the seed's §3 and §10 tables, which were written against a
four-area workbench and a tree without the `ui` feature.

## The command surface

A TUI has no rail; it has one screen at a time and a way to get to the next. The window's ⌘K
navigator is the shape that generalizes, so the terminal interface is:

- **a command palette** — fuzzy-prefixed, every action discoverable, the navigator's own grammar;
- **one key per screen** — `Ctrl-1`..`Ctrl-9` already jump the rail in the window, and the TUI maps
  the same digits to its screens in the same order (PROJECT group first, APP above it), so what a
  user learned in one interface is not unlearned in the other;
- **`?` for help** in the palette, the seed's `just tui` check beside it.

Everything outside a pane is free territory for keys. Everything inside a pane belongs to the
harness: the seed's §5 rule is unchanged — a harness consumes nearly every chord, so pane-land is
reached through a leader key, and `D45`'s closed set of intercepted gestures becomes a closed set of
leader-prefixed ones.

## The screens, in four releases

| # | Ships | The screen draws | Proves |
|---|---|---|---|
| 0 | Attach, Control, the palette | The host record (seed §8) — read, dial, or own — and the two Control pages as tables; the palette with the screen keys; one status line showing which host and which interface owns it | The client, the record, the frame loop, the token. About a week |
| 1 | Projects, explorer, search, viewer | The project list, the explorer tree with filter and git-letter badges, the file picker, project search results, a read-only text viewer with syntax colour | The file families under a terminal's keyboard model, and the first real layout |
| 1 (option) | Tasks, read | Columns, cards, the find field and the filter; the field-at-a-time panel | That a board the seed cut is actually text end to end — cheap enough to trial while phase 1 runs |
| 2 | Agents and conversations | The transcript with tool blocks and folds, the composer, permission prompts and the "needs you" strip, the lifecycle glyph, context and token readouts, the delegate list, the pending-ask answer | That an agent can be run *and answered* from a terminal — the release the whole proposal is judged by |
| 3 | Panes | The alacritty grid walked into a ratatui buffer, the leader key, resize into the harness, the redraw budget | The expensive one — nesting, at the frame rate a harness needs |
| 4 | Remote polish | Reconnect, TLS by default on a dialled host, the terminal-only build in CI | That the SSH case is a product rather than a demo |

What changed against the seed's phases: **phase 0** is smaller — the record lives in the seed §8 and
`--no-default-features` already builds a console-only binary (D162), so the window half "reads the
record and attaches" is a boot change to one crate rather than a new invention. **Phase 1** gains the
Tasks option from the scorecard, and loses nothing. **Phase 2** is unchanged and stays the judgement
point: if a developer will not run an agent from a terminal over SSH, phases 3 and 4 should never be
built and the crate should be deleted rather than carried.

## The kept-out list, one line each

- **Git** — the terminal ships git, tig and lazygit; the multi-repo lane engine is the only part
  Ubiq draws that they do not, and it is the least worth a port.
- **Teams, `[Teams]`** — a spatial canvas is the opposite of a monospace grid, and the positions are
  the window's own facts.
- **Sink** — a bench for the window's own drawing.
- **IDE editing, Markdown, Mermaid, images, Excalidraw** — a second editor and a second renderer
  before phase 2's judgement would be exactly the over-build this proposal warns against. When editing
  does return, the delegation line in [`tui-ecosystem.md`](./tui-ecosystem.md) names `$EDITOR` as the
  answer, not an edit buffer.
- **Orchestration, connectors, capture** — the seed's line holds: each needs its own argument, and
  the plan surface's minimap is graphical.

Each line is a reversal someone might later want, and each one has a home in
[`mode-fit-analysis.md`](./mode-fit-analysis.md) with the tests that produced it.

## Sharing with the window

The seed's rule stands: **share nothing on the first two phases; extract exactly one module on the
third.** Phases 0–2 duplicate small state — a tree, a table, a query — and answer to the host for
everything else, which the current transport already lets a second client do for the plan, quota,
notification, knowledge-base, project and session families (§2 of the seed's account, verified in
`transport-contract.md`). Phase 3 moves `crates/ubiq/src/state/conversation.rs` into a crate both
interfaces depend on — the move the analysis calls `G342` — and the window's tests come with it.
Anything else is shared once a second consumer has proved the shape.

The one new sharing fact the current tree adds: `crates/agent-manager` already carries a ratatui TUI
stub behind its own `tui` feature. It is a stub and it is not this interface, but it is the crate
that owns harness launch — so the first TUI screen that touches a harness should check whether it is
borrowing the stub's key loop or the window's harness story before deciding where a key belongs.

## Colour and theme

The seed's §9 is adopted whole: the same token names against a different value table, a true-colour
value per token where the terminal reports it and an indexed fallback where it does not, and the
theme file becoming the second file in the tree allowed to name a colour. `D10`'s rule is a decision
row change, and it is part of `D164` below rather than a separate one.

## What this set would leave in the register

Accepted together, the four documents propose two decisions, one host-side rule, and ten gaps, all
numbered against the current register:

- **`D164` — a second interface is a client, not a mode.** Its own crate, its own token table, its
  own binary target; the shared layer is state or nothing (`D162`'s feature split extended to a
  second interface).
- **`D165` — a host advertises itself in its config root, and a front end attaches before it starts
  one.** Owning a coordinator means binding loopback and writing a record; finding a live record
  means attaching to it, whichever interface wrote it.
- **`D166` — a streamed resource has many viewers and one writer.** A pane's keyboard and a
  conversation's outstanding ask are leased; everything the host owns broadcasts, everything it
  answers is per-asker.
- **`G340`–`G344`** carry the seed's five terminal-interface gaps (streams route to one owner;
  nothing enforces one host per config root; the conversation fold does not travel; the dialled
  transport is token-only; the handoff socket and the record answer one question twice).
- **`G345`–`G349`** carry the seed's five multi-attach gaps (geometry on the lease holder; the ring
  is bytes, not a screen; a live conversation buffer is unsettled with `inbox-chat-resume`; every
  client is the same person by assumption; `client_gone` reaps as if one client existed).

The reservation itself lives in [`tui-overview.md`](./tui-overview.md), so the number block is
stated once.

## Related docs

- [`mode-fit-analysis.md`](./mode-fit-analysis.md) — the scorecard these screens come from
- [`tui-ecosystem.md`](./tui-ecosystem.md) — what each screen above is built from
- [`ratatui-tui-proposal.md`](./ratatui-tui-proposal.md) — the proposal, whose §10 phases this table renews
- [`multi-attach-proposal.md`](./multi-attach-proposal.md) — what the release runs into at phase 3, when a second interface spawns panes beside the window's
- [`transport-contract.md`](../../tech/transport-contract.md) — the message families the screens draw
- [`decisions.md`](../../tech/decisions.md) — the register the rows above name