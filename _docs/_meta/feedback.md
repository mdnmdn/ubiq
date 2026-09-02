---
id: meta-feedback
title: Proposal ledger
kind: meta
status: current
summary: Append-only ledger of documentation changes the bookkeeper may not make unilaterally, and the resolutions they received.
read_when: you are a bookkeeper with a structural itch you may not act on yourself, or you are triaging one
updated: 2026-09-02
verified: 2026-09-02
depends_on: [meta-librarian]
review_cycle: quarterly
---

# Proposal ledger

Everything the bookkeeper may not do on its own authority lands here as one row: a split, a merge, a
move, a rename, a rule change, a gap, or drift it cannot resolve. The rules that govern this table —
the cap of five open entries, permanent rejections, and who executes an accepted proposal — are in
[`librarian.md`](./librarian.md).

Not this table: unresolved *product* questions. Those are one line in
[`../backlog.md`](../backlog.md). The test is whether resolving it changes a document's place in the
library (here) or changes what Ubiq does (there).

## Open

| id | date | target | kind | rationale | status |
|---|---|---|---|---|---|
| P1 | 2026-09-01 | `features/workbench.md` | split | The document is 480 lines and past the linter's 400-line target. It now owns two screens: the window's own shell — rail, panels, projects, explorer, editor, status bar — and the agents screen, which is a capability of its own with its own graph, inspector and drawer. Deleting the agents screen would delete those sections, which is the `features/` test | open |
| P2 | 2026-09-01 | `features/workbench.md` | split | Sharpens `P1`, which is now blocking: the tasks board landed in the same document and it is 571 lines, past the 500-line ceiling rather than the 400-line target, so `just docs-lint` fails on it. The document owns three screens now — the shell, the agents screen and the board — and the last two are one capability between them, since they are two views of one set of tasks | open |
| P3 | 2026-09-01 | `features/workbench.md` | split | Restates `P1` and `P2` against a document that is 877 lines, well past the 500-line ceiling, so `just docs-lint` fails on it. What would come out is one document over the work: the agents screen and the tasks board — the graph, its selection model, the inspector, the tasks drawer, the columns, the cards, the task panel and the form in it — leaving the shell with the rail, the panels, the projects, the explorer, the editor and the status bar. The two screens are one capability, since they are two views of one set of tasks, and deleting that capability would delete those sections, which is the `features/` test. The argument for leaving it whole: the two screens share the window's areas table, the split between what a window owns and what a project owns, and the status bar's per-mode readout, and each of those facts has exactly one owner — a split would either copy them or leave the new document reaching back into the shell's for its own sizes and its own state ownership | open |
| P4 | 2026-09-01 | `inbox/movable-panels-proposal.md` | gap | The proposal is implemented: the window's arrangement is a dock of movable panels, its phases 1 to 3 are in the tree, and the decisions it asked for are `D42` with `D17` half reversed and `D25` superseded. Its fourth phase — one panel per open file — belongs to `inbox/file-viewers-proposal.md`, which is not implemented. So the document is a record of a settled design whose facts have owners elsewhere: retire it, or reduce it to whatever its companion still needs from it. Retiring an inbox document is a move, which is not the contributor's to make | open |
| P5 | 2026-09-01 | `features/workbench.md` | split | New information for `P1`–`P3` rather than a fourth restatement: the document is now 991 lines, and the screen that took it there is the kitchen sink, which is not a project capability at all. So the cut `P3` proposes — one document over the work, taking the agents screen and the board — leaves the sink behind in the shell's document, where a page holding fixtures, a style reference and the window's only modals sits beside the rail, the projects and the explorer. Either the cut is two documents rather than one, or the sink is the piece that comes out first: deleting it would delete its sections, it shares none of the three facts `P3` names as the argument for leaving the document whole, and it depends on the shell only for the rail mode that selects it | open |
| P6 | 2026-09-02 | `features/workbench.md` | split | New information for `P1`–`P5` rather than a sixth restatement, on two counts. The document is 1398 lines, and the screen that took it there is a second screen over the work: the rail's Agents mode is a row of parallel columns for talking to the agents, and the graph those five rows call "the agents screen" is Orchestration. So the cut `P3` proposes — one document over the work — takes three screens, not two, and the phrase "the agents screen" in `P1`–`P5` names the graph rather than the screen that carries the name today: a librarian acting on those rows without reading this one would cut the wrong sections. The argument for leaving the document whole gains a fourth shared fact as well — the two screens over the agents and the board all read `ui/work.rs` for the token a state takes, which `tech/ui-and-design.md` owns | open |

## Closed

| id | date | target | kind | rationale | resolution |
|---|---|---|---|---|---|
| — | — | — | — | — | — |

## Related docs

- [`librarian.md`](./librarian.md) — what the bookkeeper may and may not do
- [`review-log.md`](./review-log.md) — what each pass actually did
