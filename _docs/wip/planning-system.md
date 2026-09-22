---
id: wip-planning-system
title: The planning system — what remains open
kind: wip
status: draft
summary: What is still undecided about the planning flow — missions, plans and their annotations — after all seven staged slices (T-56 through T-62) and the follow-on provenance work (T-94) shipped. The built design lives in `features/workbench.md`, `tech/transport-contract.md` and `tech/decisions.md` (`D157` through `D160`); this file is only the remainder.
read_when: you are picking up T-74 or T-85, or deciding whether a mission should inherit anything from its parent, or whether `require plan` should gate anything
updated: 2026-09-22
code_anchors: [crates/ubiq/src/state/document.rs, crates/ubiq-host/src/store/plan.rs]
depends_on: [feat-workbench, tech-transport, tech-decisions, wip-kb]
---

# The planning system — what remains open

The planning flow this file once proposed — a mission task type, parent/child and task-to-task
references, task attachments, a plan document with human-and-agent annotations, the plan editor,
the `ubiq-plan` MCP surface, the new-mission dialog and a `mission assistant` profile flag, and
project-scoped profiles — is built, across all seven staged slices plus the edit-provenance and
`PlanConflict` work that followed. What each piece is and how it works now lives in
[`features/workbench.md`](../features/workbench.md) (behaviour and implementation),
[`tech/transport-contract.md`](../tech/transport-contract.md) (the `Plan`, annotation and
provenance message families) and [`tech/decisions.md`](../tech/decisions.md) — `D157` (edit
provenance), `D158` (a profile's project scope), `D159` (the annotation anchor is a stable block
id) and `D160` (the plan editor is native, not a web-panel tenant).

What remains is two blocked cards and two open questions.

## Open items

- **T-74 — where an annotation sidecar lives for a plan-shaped document inside the user's own git
  repository.** The built annotation layer keeps its sidecar in the config root, beside the plan
  itself: `<TaskId>.annotations.json` next to `<TaskId>.md`, both under
  `<config root>/projects/<ProjectId>/plans/`, which is safe because a plan lives there too and
  never in the user's tree. `crate::state::document::DocumentHandle`
  (`crates/ubiq/src/state/document.rs`) has exactly one variant, `Plan`, for this reason: a second
  variant that annotates an arbitrary project `.md` file needs its own answer for where the
  matching sidecar goes, and writing Ubiq's bookkeeping into a repository the user did not ask to
  have annotated is a decision nobody has taken. This is what blocks the second `DocumentHandle`
  variant — the plan editor is presently the only implementation of the annotated-document surface.
  Undecided.
- **T-85 — what a knowledge-base attachment becomes in a chat prompt.** Three candidates: a
  resolved temp file the prompt names by path, an inlined excerpt of the document's text folded
  into the prompt directly, or a reference the `ubiq-kb` MCP server resolves when the agent asks
  for it. Chat attachments never cross the bus today — `Conversation::attached` is interface state
  folded into `PromptAgent { text }` at send — and picking from the knowledge base is not wired
  into the composer's file picker at all yet, which is a KB-aware picker for whichever answer this
  becomes. Undecided.
- **Does `require plan` do anything mechanical**, or is it only a reminder? The new-mission dialog
  built it as a reminder — one more sentence in the assistant's opening turn
  (`state/new_mission.rs::mission_briefing()`) — deliberately, on the reading that a gate is a
  separate decision. It could instead hold a mission out of `Done` until a plan exists.
- **Does a child inherit anything from its parent** — labels, colour, session, assignee? Nothing
  does today; `parent: Option<TaskId>` carries no propagation.

The plan editor's own known gap — its provenance underlines drifting from the text while the
buffer is dirty — is `G332` in [`backlog.md`](../backlog.md), not restated here.

## Related docs

- [`../features/workbench.md`](../features/workbench.md) — the tasks board, missions, the plan
  store, the plan editor and the annotation layer, built
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the `Plan`, annotation and
  provenance message families
- [`../tech/decisions.md`](../tech/decisions.md) — `D157`–`D160`, and `D30`, `D113`, `D121`,
  `D138` this design builds on
- [`kb.md`](./kb.md) — the knowledge base T-85's answer would point into
- [`../backlog.md`](../backlog.md) — `G332`
