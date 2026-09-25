---
id: wip-planning-system
title: The planning system — what remains open
kind: wip
status: draft
summary: What is still undecided about the planning flow — missions, plans and their annotations — after all seven staged slices (T-56 through T-62) and the follow-on provenance work (T-94) shipped. The built design lives in `features/workbench-tasks.md`, `tech/transport-contract.md` and `tech/decisions.md` (`D157` through `D161`); this file is only the remainder.
read_when: you are picking up T-85, or deciding whether a mission should inherit anything from its parent, or whether `require plan` should gate anything
updated: 2026-09-24
verified: 2026-09-24
code_anchors: [crates/ubiq/src/state/document.rs, crates/ubiq-host/src/store/plan.rs, crates/ubiq-host/src/plan/mod.rs, crates/ubiq-host/src/plan/service.rs, crates/ubiq-proto/src/plan.rs]
depends_on: [feat-workbench, tech-transport, tech-decisions, wip-kb]
---

# The planning system — what remains open

The planning flow this file once proposed — a mission task type, parent/child and task-to-task
references, task attachments, a plan document with human-and-agent annotations, the plan editor,
the `ubiq-plan` MCP surface, the new-mission dialog and a `mission assistant` profile flag, and
project-scoped profiles — is built, across all seven staged slices plus the edit-provenance and
`PlanConflict` work that followed. What each piece is and how it works now lives in
[`features/workbench-tasks.md`](../features/workbench-tasks.md) (behaviour and implementation),
[`tech/transport-contract.md`](../tech/transport-contract.md) (the `Plan`, annotation and
provenance message families) and [`tech/decisions.md`](../tech/decisions.md) — `D157` (edit
provenance), `D158` (a profile's project scope), `D159` (the annotation anchor is a stable block
id), `D160` (the plan editor is native, not a web-panel tenant) and `D161` (the family is keyed by
a `DocumentHandle`, and a project file's sidecar lives beside the file).

What remains is one blocked card and two open questions.

## Open items

- **T-74 — where an annotation sidecar lives for a document inside the user's own git repository —
  is decided and built (`D161`).** A project's markdown file is annotated in place: the body is the
  file, and the sidecar is `<file>.md.annotation.json` beside it, inside the repository — a
  deliberate, narrow exception to `D30`. The handle moved into the contract as
  `ubiq_proto::plan::DocumentHandle` and grew its second variant, `File { project_id, rel_path }`,
  so the whole family is keyed by a document rather than by `(project_id, task_id)`; the block
  matcher, the orphaning rule, the conflict arbitration and the provenance layer are the plan's,
  unchanged. A plan keeps its own `<TaskId>.annotations.json` under the config root — two spellings
  for one format, kept rather than migrated.

  **The interface half is built** (T-124): `ViewLayout::Annotation` is markdown's fourth position,
  `crate::state::plan::file_document` is called from `AppState::open_file_document`, and the
  surface a tab draws there is the plan editor's own — `ui/document.rs`, with `ui/plan.rs` reduced
  to the dialog frame around it. Agent integration for a file document's annotations is still an
  explicitly later card: in plan mode they feed the agents through `ubiq-plan`, for a file they
  only sit there.
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

The plan editor's own known gaps — its decoration layers painting into a buffer the preview-only
surface no longer draws, the `/` menu that went with that buffer, and frontmatter having no block
kind of its own on the annotation surface — are `G332`, `G333` and `G341` in
[`backlog.md`](../backlog.md), not restated here.

## Related docs

- [`../features/workbench-tasks.md`](../features/workbench-tasks.md) — the tasks board, missions, the plan
  store, the plan editor and the annotation layer, built
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the `Plan`, annotation and
  provenance message families
- [`../tech/decisions.md`](../tech/decisions.md) — `D157`–`D161`, and `D30`, `D113`, `D121`,
  `D138` this design builds on
- [`kb.md`](./kb.md) — the knowledge base T-85's answer would point into
- [`../backlog.md`](../backlog.md) — `G332`, `G333`, `G341`
