---
id: meta-authoring
title: Writing and updating docs
kind: meta
status: current
summary: What every agent and human owes this documentation when they change code — and the small set of edits they may make.
read_when: your change touched code, behaviour or a contract and you are not running a maintenance pass
updated: 2026-08-31
verified: 2026-08-31
depends_on: [meta-librarian]
review_cycle: quarterly
---

# Writing and updating docs

For everyone who is **not** running a documentation maintenance pass: implementer agents, review
agents, and humans.

**This document is self-sufficient.** Everything needed to discharge the duty is here — the fields,
the placement rule, the one case where a document may be created. Reading
[`librarian.md`](./librarian.md) is not required; it covers structure and maintenance passes.

If you read nothing else:

1. Your change updates the documents it touched, **in the same commit**.
2. Edit in place. Do not move, split, rename or reorganize anything.
3. Match the document you are editing — its shape is already correct.
4. Present tense, no line numbers, no `TODO`.
5. Anything you cannot place goes to `inbox/`, `backlog.md` or `_meta/feedback.md` (§7).

---

## 0. Which documents did your change touch?

Three ways, cheapest first:

- **`just docs-touched`** — with no arguments it reads your working diff; with paths, it names the
  documents that list those files in `code_anchors`.
- **`INDEX.md`** — its catalogue says which document owns which capability, and its fact-ownership
  registry says which owns which kind of fact (message variants, theme tokens, commands).
- **grep** — search `_docs/` for the file or symbol you touched. If a document names it, that
  document is yours to check.

If nothing turns up and your change is user-visible, that is the missing-document case (§2).

## 1. Two verbs

You have exactly two: **amend** an existing document, and **append** a row or bullet to one.

| You may | You may not |
|---|---|
| Edit any section of an existing document | Move, split, merge or rename anything |
| Append a row to a register or a line to `backlog.md` | Reorganize sections or change a document's shape |
| Add a `## Next steps` bullet | Invent a rule, or change one that exists |
| Re-stamp `verified` (§6) | Create a document, except in the one case in §2 |
| Drop raw material into `inbox/` | Leave a `TODO` or an `Open questions` heading behind |

Content is yours; **structure belongs to the librarian**. That split lets you contribute correctly
without learning the whole scheme: you are trusted with what you know — what the system does — and
not with decisions about how the library is arranged, which need a view of all of it.

## 2. The one document you may create

If you shipped a user-visible capability that no document covers, **write its document now**. You
are the only one who knows it, and a librarian filing it later can only guess. This is the single
exception, and it is narrow:

> Ask: *would deleting this capability from Ubiq delete the document?* If yes, it is a feature
> document and you may create it under `features/`. If no — it is a rule, a convention, a model, a
> procedure — **you may not**: file it under §7 instead.

Anything you are unsure about is a "no". Two further limits: never create a folder, and never create
a second document for a capability that already has one — extend the existing one.

To write it, **copy the nearest sibling in `features/`** and replace its content. That gives you the
right frontmatter and the right section order for free. The order is fixed, contract before code:

```markdown
## Purpose            2–4 sentences. The user-visible outcome.
## Behaviour          The contract: rules, invariants, edge cases, what is forbidden.
## Contract           Which message variants and state it touches — names only, linked.
## Implementation     Modules and functions in call order: ui → state → orchestrator → pty.
## Failure            What happens when the harness dies, the PTY closes, or a spawn fails.
## Related docs
## Next steps         Optional, ≤8 bullets, no dates.
```

## 3. When you owe an update

Documentation is part of the change, not follow-up work. Same commit, every time.

| If your change… | Then… |
|---|---|
| Alters behaviour a user can see | Update that feature document's *Behaviour* **and** *Implementation* |
| Adds, renames or moves a module or symbol a document names | Fix those references — grep the old name across `_docs/` |
| Adds, removes or reshapes a transport message | Update the transport contract document |
| Changes a build, test or run command | Update the operations document |
| Adds or changes a theme token or a UI convention | Update the UI document |
| Makes a structural choice a reasonable person might later reverse | Append a row to the decision register: what, why, what it costs |
| Leaves something unresolved | One line in `backlog.md`, naming the document it affects |
| Touched a file listed in some document's `code_anchors` | Verify that document (§6) |

One thing that is **not** your job: documenting what the code plainly says. Name the module and the
function and stop. Rust module headers (`//!`) carry that layer, and they cannot drift, because the
diff that changes the logic is the diff that changes the header.

## 4. Frontmatter

Every document carries it. When amending, the block is already at the top of the file — change
`updated`, change `verified` if §6 applies, and add any file you made the document depend on to
`code_anchors`. Leave the rest alone; `id` in particular never changes, because other documents
cite it.

When creating (§2), the full set, which you get for free by copying a sibling:

```yaml
---
id: feat-panes                 # stable forever. Prefix mirrors the folder: prod- feat- tech- meta- wip-
title: Panes and terminals
kind: feature                  # product | feature | tech | meta | wip | inbox
status: current                # current | draft | superseded
summary: One line. This is the document's entry in INDEX.md.
read_when: you are changing pane layout, focus or resize
updated: 2026-08-31            # you changed the content
verified: 2026-08-31           # you checked it against the code and it held
code_anchors: [crates/ubiq/src/app.rs]   # the files a reader would open next
depends_on: [tech-architecture]          # ids, not paths
review_cycle: quarterly        # monthly if this area changes often
---
```

`read_when` is what gets a document opened, so phrase it as the reader's task — "you are adding a
message variant", not "message conventions". `summary` says what is inside; `read_when` says when to
care.

`status: draft` earns its own note. Much of Ubiq is designed ahead of the code, and a draft document
describes a contract that the tree does not implement end to end. Draft is not a licence to be
vague — it is a promise that the design is settled and the wiring is not.

## 5. How to comply without learning the rules

**Match the document you are editing.** Its section order, heading depth, table style and level of
detail are already correct — imitate them. That covers most of the rulebook by construction.

Beyond imitation, six constraints:

1. **Present tense, current state.** Describe what holds today. Never "now", "no longer", "used to",
   "not yet" — a reader must not have to reconstruct a timeline. What changed is git's job.
2. **Cite code as file plus symbol** — `crates/ubiq/src/app.rs`, `spawn_pane()`. Never a line number.
3. **Keep code out.** Under twenty lines per fence, and only where prose genuinely cannot carry it.
   Otherwise point at the file.
4. **State a fact once, in the document that owns it.** If it is written elsewhere, link instead of
   copying. The fact-ownership registry is in `INDEX.md`.
5. **Link a document at most three times,** one of them in `## Related docs`.
6. **Update the frontmatter** as §4 sets out.

If your edit pushes a document past ~500 lines, stop, make the edit anyway, and file a split under
§7. Do not split it yourself.

## 6. Verification

`verified` means: *someone read this document against the code and it held.* It is not "recently
edited" — that is `updated`.

Three tiers, in the order that matters:

**Per change — mandatory, and where trust actually comes from.** You touched code that a document
anchors. Read that document's claims, then either re-stamp `verified` to today, or fix what broke
and stamp it. You are the cheapest possible verifier: you are already in the relevant code, and you
are the only one who knows what you just changed.

**Periodically — the bookkeeper's safety net.** `just docs-drift` compares each document's
`code_anchors` against git and queues anything whose anchors moved after its `verified` date. This
catches what tier one missed. It is a net, not the mechanism.

**At discretion — when you rely on a document.** If you read a document to do a task and it held up,
stamp it. If it did not, that is the most valuable thing you will find all day.

**When a document and the code disagree, decide which one is authoritative before editing either.**
A *Behaviour* section, a rule under `tech/`, a decision row and anything in `product/` **prescribe**:
if the code disagrees, the code is wrong. Fix the code, or file it as a defect — silently editing the
document to match would turn a bug into a documented feature. An *Implementation* section, a code map
or an anchor list **describes**: there the document yields, and you fix the document.

## 7. When you cannot comply

Three places, and no permission needed for any of them:

- **Raw material you have not distilled** — a transcript, a dump, a half-formed draft → drop it in
  `inbox/`. Unprocessed input in `inbox/` is fine; unprocessed input filed as a document is not.
- **A structural itch** — this document should be split, that one should not exist, two of them
  contradict each other → one row in `feedback.md`. State both sides of a contradiction rather than
  picking a winner.
- **Unresolved substance** — an open question, a known gap, a deferred decision → one line in
  `backlog.md`. Never a heading inside a document.

Humans have one power beyond this: changing the rules themselves, in `_meta/`. Everything else in
this document applies to humans and agents alike.

---

## Related docs

- [`librarian.md`](./librarian.md) — the full rulebook, for maintenance passes and structural changes
- [`feedback.md`](./feedback.md) — where a structural proposal goes
- [`../INDEX.md`](../INDEX.md) — the catalogue, the fact-ownership registry, and which document to read when
