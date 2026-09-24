---
id: inbox-tui-overview
title: The TUI proposal set — how to read it
kind: inbox
status: proposal
summary: "The map of the terminal-interface proposal documents under `inbox/tui/` — the two seeds a branch brought in (a ratatui front end, and many interfaces on one host), re-homed here and renumbered against the current register, and the three new documents that score the workbench's nine modes against a terminal, lay out a first-release scope, and map which widget crate each screen buys. Also carries the delta: what the current tree settled since the seeds were written."
read_when: you are deciding whether Ubiq gets a second front end, or you are looking for where the terminal-interface proposal work lives and what it assumes
updated: 2026-09-24
depends_on: [inbox-tui, inbox-multi-attach]
---

# The TUI proposal set — how to read it

## Purpose

This folder is home to the proposal that Ubiq gets a second interface: a terminal one, drawn with
ratatui, that attaches to a host as an ordinary client — when the desktop is not the machine the
hosts run on, which for anyone over SSH is most of the time. Two of the documents came in from the
branch `claude/ratatui-tui-proposal-vy3x3c`; this folder exists to give that work a current-tree
reading before anyone decides on it, and to hold the three documents the reading produced.

## The set

| Document | What it is | Read it when |
|---|---|---|
| [`ratatui-tui-proposal.md`](./ratatui-tui-proposal.md) | The seed: a second interface as a second client, what it covers and drops, a pane inside a pane, the host record, the three build shapes, the phases | Deciding whether a second front end is worth it at all |
| [`multi-attach-proposal.md`](./multi-attach-proposal.md) | The seed with it: what a host owes two attached interfaces — the resource taxonomy, the write lease, one geometry, a joiner's first frame, and the line between one person with two screens and two people with one agent | Deciding what happens when two interfaces join one host |
| [`mode-fit-analysis.md`](./mode-fit-analysis.md) | New: the workbench's nine modes scored against a terminal grid, and the two hard `no`s it cannot draw | Deciding what the TUI draws and what it drops |
| [`tui-release-scope.md`](./tui-release-scope.md) | New: a concrete first-release scope — screens, keyboard model, kept-out list and phases — updated to the current workbench | Planning the build, or arguing its scope |
| [`tui-ecosystem.md`](./tui-ecosystem.md) | New: the crate map — which ratatui widget each screen adopts, adapts, or writes | Choosing a widget for a screen, or arguing the buy-versus-write line |

## Where the seeds came from, and what was changed

Both seeds were authored on `origin/claude/ratatui-tui-proposal-vy3x3c` against a September-12 tree
and are placed here with three edits only:

1. **Relative links to sibling inbox documents gained a `../`.** The two seeds moved down one level,
   and every `./detached-panes-proposal.md`, `./chat-resume-and-fork.md` and
   `./backlog/remote-drone-proposal.md` would have broken dead.
2. **The proposed register rows were renumbered.** The seeds propose `D113`–`D115` and `G250`–`G259`,
   all occupied in the current register by unrelated decisions and gaps, so they now read `D164`–
   `D166` and `G340`–`G349`. The reservation, held across the set:
   | Number | Proposed for |
   |---|---|
   | `D164` | A second interface is a client, not a mode |
   | `D165` | A host advertises itself in its config root; a front end attaches before it starts one |
   | `D166` | A streamed resource has many viewers and one writer |
   | `G340`–`G344` | The terminal interface's five gaps (seed §12) |
   | `G345`–`G349` | The multi-attach's five gaps (seed §10) |
3. **`updated` restamped; `depends_on` verified against current ids.**

No seed prose was rewritten. Every path the seeds describe was re-verified to exist on 2026-09-24 —
`crates/ubiq-proto/src/bus.rs` still mints a client per attachment and `bus::detached` still exists,
`crates/ubiq-proto/src/wire.rs` still frames length-prefixed MessagePack, the coordinator still keys
its maps by `ClientId` and reaps on `client_gone`, the conversation fold is still window-side state
in `crates/ubiq/src/state/conversation.rs`, and ratatui 0.30 still resolves through the harness
library's TUI stub in `crates/agent-manager/src/tui.rs`. What the seeds propose as *unbuilt* is
still unbuilt, which is the point of a proposal.

## What the current tree settled since the seeds

- **The windowless build already exists.** `D162` made the GPUI interface a cargo feature `ui`,
  default-on; `--no-default-features` builds a console-only Ubiq — same host, same `--serve` — and
  `just headless` asserts no GPUI crate is in its tree. The seed's §7 "three shapes" is half-built:
  the *no-window* half is done and has nothing to run, and the terminal interface is what would run
  there. A `tui` feature and a second binary target slot onto what exists; the seed's talk of adding
  the first feature is now history.
- **The workbench grew to nine modes.** The seed was written against four areas of the window. The
  rail now selects between `Control` and `Sink` (APP) and `IDE`, `Git`, `Agents`, `Teams`, `[Teams]`,
  `KB` and `Tasks` (PROJECT). The seed's area table is history; [`mode-fit-analysis.md`](./mode-fit-analysis.md)
  is the current reading, and [`tui-release-scope.md`](./tui-release-scope.md) the current phases.
- **The host record the seed proposes does not exist.** A served run binds `0.0.0.0:7420` by default,
  prints a bearer token to stdout and keeps nothing on disk (`G168`); the window persists its own
  saved remotes with the token in the keychain. "Attach, or start" is still the unbuilt half, and the
  seed's §8 is the proposal for it.
- **The floor of visible facts is wider than the seed promised.** The plan, quota, notification,
  knowledge-base, project and session families broadcast or answer per asker — none stream — so a
  second client sees more than "the catalogue, the agent roster, the readings" before any host change.

## Reading order

1. [`mode-fit-analysis.md`](./mode-fit-analysis.md) — which modes fit, the question this folder exists to answer
2. [`ratatui-tui-proposal.md`](./ratatui-tui-proposal.md) — the proposal itself
3. [`multi-attach-proposal.md`](./multi-attach-proposal.md) — the edge the proposal stops at
4. [`tui-release-scope.md`](./tui-release-scope.md) — what gets built, and in what order
5. [`tui-ecosystem.md`](./tui-ecosystem.md) — what each screen is built from

## Related docs

- [`transport-contract.md`](../../tech/transport-contract.md) — the message families a second client speaks
- [`workbench.md`](../../features/workbench.md) — the nine modes the analysis scores
- [`decisions.md`](../../tech/decisions.md) — the register the reservations above land in