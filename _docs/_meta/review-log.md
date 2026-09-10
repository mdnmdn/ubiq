---
id: meta-review-log
title: Review log
kind: meta
status: current
summary: Append-only record of what each documentation maintenance pass checked, fixed and left alone.
read_when: you are starting a maintenance pass and need to know what the last one did
updated: 2026-09-10
verified: 2026-09-10
depends_on: [meta-librarian]
review_cycle: quarterly
---

# Review log

One row per pass. Never a silent pass: "nothing to change" is a result and gets its line. The
reporting rule is in [`librarian.md`](./librarian.md).

| date | scope | checks | found | fixed | left alone |
|---|---|---|---|---|---|
| 2026-08-31 | Full rebuild of the library | L1–L10 | The whole of `_docs/` described a Tauri + xterm.js frontend the tree no longer contains | Rebuilt the library against the GPUI workspace: `INDEX.md`, `product/`, `features/`, `tech/`, `_meta/`; ported `_tools/docs.py`; rewrote `AGENTS.md` and `README.md` | `design/` — captured wireframes and prototypes are evidence and stay verbatim |
| 2026-09-10 | Full pass, on request | L1–L10 | 225 lint failures and 6 of 20 anchored documents in the drift queue. Two structural causes carried most of it: L7 demanded an `INDEX.md` listing from every `inbox/` document, which the index generator excludes by design, and the earlier moves into `inbox/completed/` and `inbox/backlog/` left 73 relative links pointing one level too high | Down to 65 failures, with L1, L5, L7, L9 and L10 clean. Corrected the L7 check to match its own rule; repaired the 73 links and 15 other broken paths and symbols; rewrote 20 transient phrasings; trimmed the two documents over the link-repeat cap; wrote frontmatter for five documents that had none; repointed `excalidraw.py` to `.claude/`; resolved the drift queue to 1 — three unambiguous corrections (`features/logs.md`'s seven subsystems, `wip/clone-a-project.md`'s Escape rule, `wip/indexing.md`'s schema number) and two honest re-stamps; executed `P4`, `P7`, `P10`, `P12` and `P14` and closed them | The 54 remaining L2 failures are `inbox/` proposals citing symbols the implementation renamed — correcting them would rewrite the record of what was proposed, so the check wants the exemption `design/` has. The 11 L4 failures are the four documents `P1`–`P15` already propose splitting, plus seven `inbox/` proposals. `tech/diagram-format.md` stays queued: its format claims are confirmed, but `just diagram` names a file that has moved and `tech/project-structure.md` says tooling lives in `_tools/` |

## Related docs

- [`librarian.md`](./librarian.md) — the checklist each pass runs, and the reporting rule
- [`feedback.md`](./feedback.md) — the proposals a pass raised but may not execute
