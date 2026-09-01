---
id: meta-feedback
title: Proposal ledger
kind: meta
status: current
summary: Append-only ledger of documentation changes the bookkeeper may not make unilaterally, and the resolutions they received.
read_when: you are a bookkeeper with a structural itch you may not act on yourself, or you are triaging one
updated: 2026-09-01
verified: 2026-08-31
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

## Closed

| id | date | target | kind | rationale | resolution |
|---|---|---|---|---|---|
| — | — | — | — | — | — |

## Related docs

- [`librarian.md`](./librarian.md) — what the bookkeeper may and may not do
- [`review-log.md`](./review-log.md) — what each pass actually did
