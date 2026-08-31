---
id: meta-review-log
title: Review log
kind: meta
status: current
summary: Append-only record of what each documentation maintenance pass checked, fixed and left alone.
read_when: you are starting a maintenance pass and need to know what the last one did
updated: 2026-08-31
verified: 2026-08-31
depends_on: [meta-librarian]
review_cycle: quarterly
---

# Review log

One row per pass. Never a silent pass: "nothing to change" is a result and gets its line. The
reporting rule is in [`librarian.md`](./librarian.md).

| date | scope | checks | found | fixed | left alone |
|---|---|---|---|---|---|
| 2026-08-31 | Full rebuild of the library | L1–L10 | The whole of `_docs/` described a Tauri + xterm.js frontend the tree no longer contains | Rebuilt the library against the GPUI workspace: `INDEX.md`, `product/`, `features/`, `tech/`, `_meta/`; ported `_tools/docs.py`; rewrote `AGENTS.md` and `README.md` | `design/` — captured wireframes and prototypes are evidence and stay verbatim |

## Related docs

- [`librarian.md`](./librarian.md) — the checklist each pass runs, and the reporting rule
- [`feedback.md`](./feedback.md) — the proposals a pass raised but may not execute
