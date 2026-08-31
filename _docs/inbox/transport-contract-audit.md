---
id: inbox-transport-audit
title: Audit — the transport contract's file family
kind: proposal
status: proposal
summary: The transport contract had drifted from `messages.rs` by seven undocumented variants and eleven documented-but-absent ones; what was wrong, what was applied, what is verified clean now, and the anchoring mistake that let it happen unnoticed.
read_when: you are wondering why a code change was never reflected in a document, or you are deciding what `code_anchors` a new module needs
updated: 2026-09-01
depends_on: [tech-transport]
---

# Audit — the transport contract's file family

**The edits in §3 have all landed.** This is kept as a record rather than a task, for two reasons:
the diagnosis in §2 outlives the fix and applies to every module added from here on, and §5 is still
open.

`_docs/tech/transport-contract.md` says it **owns every message fact** — "Variant names, payload
fields, direction and response behaviour are stated here and linked from everywhere else." For the
duration of the file family's existence, it did not.

## 1. The drift, as found

`crates/ubiq-proto/src/messages.rs` defines **32 variants**. The document accounted for 28, seven of
which did not exist, and missed seven that did.

| Class | Count | What |
|---|---|---|
| In the code, absent from the document | 7 | The whole file family |
| In the document, absent from the code | 11 | Most of the session family |
| In both, described wrongly | 2 | `SpawnWorkspace`'s payload, `WorkspaceInfo`'s fields |
| Records in the code, absent from the document | 6 | `DirEntry`, `DirListing`, `FileVersion`, `FileContents`, `EntryKind`, `FileError` |

Only the first and third classes were errors. **The second was not**, and this is worth stating
plainly because the tempting fix was to delete it: `status: draft` means the design is settled and
the code is behind it, and `G19` is where that gap is registered. Deleting the session family's
table would have lost a settled design to make a lint pass. Only `G19`'s own wording was wrong — it
said those variants "exist in the contract", when they exist in the *document* and never in
`messages.rs`.

## 2. The root cause — the part that generalises

`crates/ubiq-proto/src/files.rs` was named in **no document's `code_anchors`**. So the commit that
added the file family was told it owed no document an update, and that was true given what the
frontmatter said:

```
$ just docs-touched crates/ubiq-proto/src/files.rs
Anchored by no document — the missing-document case:
  crates/ubiq-proto/src/files.rs
```

Nobody skipped the duty in `_meta/authoring.md`. The mechanism that reports the duty had a hole in
it, and reported honestly through the hole. **A new module is not covered by the document that
should own it until that document's `code_anchors` says so**, and adding the anchor is part of
adding the module, not follow-up work — exactly the way appending a decision row is part of making
the decision.

The anchor is now in place, so this particular drift cannot recur:

```
$ just docs-touched crates/ubiq-proto/src/files.rs
crates/ubiq-proto/src/files.rs → _docs/tech/transport-contract.md
```

## 3. What was applied

Six edits, all landed and verified against the current file.

| # | Edit | Where |
|---|---|---|
| 1 | `## The file family` — seven variants, the `rel_path` discipline, the bounding rules, the `expected` write guard, the truncated-read rule, per-path errors | New section, after the project family |
| 2 | `SpawnWorkspace` corrected — `project_id` is not optional, `folder` is `rel_path`, and the pre-spawn `ProjectError` refusal stated | Session family |
| 3 | The records table — `WorkspaceInfo` given `project_id` and `rel_path`, the six file records added, `FileError` moved out to the enum prose, count now nine | Payload records |
| 4 | `EntryKind` defined, with `Other` drawn-and-refused and `size` present only for regular files | Payload records |
| 5 | The file family's ordering guarantee — one worker, one queue, replies in ask order | Framing |
| 6 | `Adding a variant` step 1 rerouted for four families; it previously sent every file message to the project family | Adding a variant |

Frontmatter gained `crates/ubiq-proto/src/files.rs` and a fourth family in its `summary`; `updated`
and `verified` were re-stamped. `G19` was reworded to "exist in the transport contract document and
in no code".

## 4. Verified clean

A full comparison of the document against `messages.rs` as it now stands:

- **All 32 variants are documented**, with payload fields matching field-for-field across the file
  family.
- **All nine records and all four enums** — `ProjectHealth`, `Scope`, `EntryKind`, `FileError` — are
  described.
- **The only variants in the document and not the code are the eleven `G19` registers**, which is
  the intended state for a `draft` document.

## 5. Still open

`crates/ubiq-host/src/files/mod.rs` and `crates/ubiq-host/src/files/path.rs` are anchored by no
document — the same hole as §2, one crate over:

```
$ just docs-touched crates/ubiq-host/src/files/mod.rs crates/ubiq-host/src/files/path.rs
Anchored by no document — the missing-document case:
  crates/ubiq-host/src/files/mod.rs
  crates/ubiq-host/src/files/path.rs
```

These are the host's side, not the contract's, so they do not belong in
`transport-contract.md`'s anchors — putting them there would make every host-side change ask for a
contract review it does not need. They belong to whichever document ends up owning the explorer's
implementation, which today is `features/workbench.md`.

`path.rs` is the sharper half: its own header calls it **the security boundary**, and a boundary
described only in a source comment is one nothing obliges a reviewer to look at. Suggested row for
`backlog.md`:

```
| Gnn | The host's `files/` module is anchored by no document, so a change to it — including
`path.rs`, the containment boundary every file request resolves through — obliges no documentation
review | [`features/workbench.md`](./features/workbench.md) |
```

The link in that row is written relative to `backlog.md`, which is where it goes — not to this file.

## 6. A note on where working notes go

The first draft of this audit was written to `_docs/wip/`, which is where the library says the
current task's working notes belong. It was swept while the task was still open — reasonably, since
the edits it described had landed. The lesson is not that the sweep was wrong; it is that `wip/` is
not a safe home for anything whose value survives the task. Findings that outlive the task go here,
to `inbox/`, or to `backlog.md`. That is what §7 of `_meta/authoring.md` already says, and this is
one more case of it.

## Related docs

- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the document this audited
- [`../_meta/authoring.md`](../_meta/authoring.md) — the duty, and the `code_anchors` that report it
- [`../backlog.md`](../backlog.md) — `G19`, and where §5's row lands
