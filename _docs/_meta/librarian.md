---
id: meta-librarian
title: Librarian rulebook
kind: meta
status: current
summary: How `_docs/` is organized, why it is organized that way, and how a bookkeeper agent keeps it that way.
read_when: you are creating, moving, splitting or deleting a document, or running a maintenance pass
updated: 2026-08-31
verified: 2026-08-31
review_cycle: quarterly
---

# Librarian

This is the rulebook for the documentation itself, not for the product. Read it before creating,
moving, splitting or deleting anything under `_docs/`. Two audiences: a human deciding where a
document belongs, and a **bookkeeper agent** running a maintenance pass (§8). If you are here because
your code change touched a document, you want [`authoring.md`](./authoring.md) instead — it is short,
and it is the whole of what a non-librarian owes the library.

The library serves agents that arrive with no memory of previous work. Every rule below exists to
make one thing cheap: *finding the two or three documents a task needs, and trusting them.* A
document that is wrong is worse than no document, because it is trusted.

**Before a maintenance pass, read:** this file; `authoring.md`, so you know what contributors were
instructed to do and do not "fix" it; `../INDEX.md`, for the catalogue and the fact-ownership
registry; `feedback.md` and `review-log.md`, so you neither repeat a rejected proposal nor redo last
week's work.

---

## 1. The library

Folders encode **kind of knowledge**. Frontmatter encodes **stability**. Never mix the two — a
`drafts/` folder would only duplicate `status: draft`.

| Path | Holds | Written for |
|---|---|---|
| `INDEX.md` | The map: catalogue, fact-ownership registry, reading paths | Every agent, first |
| `backlog.md` | Every open question, known gap and deferred item, project-wide | Whoever plans next work |
| `product/` | Why Ubiq exists, in user terms. **No code references.** | Product decisions |
| `features/` | One document per user-visible capability: contract on top, implementation below | Building or changing a feature |
| `tech/` | Cross-cutting models, rules, conventions and procedures | Any implementation task |
| `design/` | Wireframes, prototypes and captured artifacts. **Assets, not documents** | Anyone building a screen |
| `wip/` | The current task's working notes. Deleted when the task closes | The task in flight |
| `inbox/` | Raw unprocessed input: transcripts, dumps, pasted drafts | Nobody yet — to be filed |
| `_meta/` | How this library works. The underscore means *not project knowledge* | The bookkeeper |

`wip/` and `inbox/` are the only folders whose documents may talk about time — before/after states,
version deltas. Everywhere else, see §5. `design/` is excluded from every check in §8.3: a captured
prototype is evidence, and editing it to satisfy a linter destroys what makes it evidence.

## 2. Where a document goes

The placement rule, which settles the product-versus-technical question without a third category:

> **If deleting the capability from Ubiq would delete the document, it belongs in `features/`.
> If the document would survive, it belongs in `tech/`.**

Applied:

- Pane focus and layout behaviour → its feature document. Remove panes, the behaviour goes too.
- The transport contract every message obeys → `tech/`. It outlives any one feature.
- The session and workspace model → its feature document. The rule that the UI never touches a
  PTY → `tech/`.
- The theme token set → `tech/`. A screen's composition → its feature document.

Then, in order:

1. Is it unprocessed input? → `inbox/`, and stop.
2. Is it notes for the task in flight? → `wip/`, and stop.
3. Is it about the documentation? → `_meta/`.
4. Is it something unresolved? → an entry in `backlog.md`, not a new document.
5. Is it an image, a wireframe or a captured prototype? → `design/`, and link it from `tech/`.
6. Product-only, no code? → `product/`.
7. Otherwise apply the placement rule.

**Prefer extending an existing document to adding one.** A new file needs a reader who would not have
found the material where it lives today. Say so in the frontmatter `summary` if it is not obvious.

## 3. Document anatomy

Fixed section order, so an agent can skim to the part it needs and stop reading.

### 3.1 Feature document

Contract first, implementation second. The seam between *Behaviour* and *Implementation* is where
the document stops prescribing and starts describing — the line the drift check (**L3**) and the
mismatch rule in §8.1 both turn on.

```markdown
## Purpose            2–4 sentences. The user-visible outcome.
## Behaviour          The contract: rules, invariants, edge cases, what is forbidden.
## Contract           Which message variants and state it touches — names only, linked.
## Implementation     Modules and functions in call order: ui → state → orchestrator → pty.
## Failure            Harness death, PTY close, spawn failure.
## Related docs       One list. The only place a link may repeat.
## Next steps         Optional, ≤8 bullets, no dates. The only forward-looking section allowed.
```

*Behaviour* must be checkable by someone who cannot read Rust. *Implementation* must be enough for
someone who has never opened the repository to know which files to open.

### 3.2 Technical document

```markdown
## <what it is>       One paragraph: the shape of the thing.
## <rules or model>   The substance — normative rules, or the model, or the reference tables.
## <rationale>        Why it is this way, where a reader would otherwise propose the alternative.
## Related docs
```

Rationale earns its place only where it prevents someone from "fixing" the design. Do not explain
what nobody would question.

### 3.3 Product document

Purpose, then scope, then non-goals. No file paths, no function names, no code. If a product
document needs one, that sentence belongs in a feature document instead.

## 4. Balance

### 4.1 Length

Target **150–400 lines**. Ceiling **500**.

- Over 500 → split **by reader intent**, never by topic-slicing. Pull the runbook out of the
  architecture document; do not create `architecture-part-2`.
- Under 80 lines with no independent reader → fold it into its parent.

Exempt from both bounds: `INDEX.md`, `backlog.md`, the glossary, the decision register, the code map,
the transport contract, and the two append-only ledgers in `_meta/`. All of these are read by lookup
rather than read through.

### 4.2 Content type

Five types: **narrative/rationale · normative rules · reference tables · procedures · catalogue**.

A document may be dominated by at most **two**. In particular, **a procedure never lives inside an
architecture document** — a setup runbook and a layering rule have different readers, different
half-lives, and different failure modes when stale.

### 4.3 Code in documents

- Fenced code ≤ **15%** of a document's lines; each fence ≤ **20 lines**.
- Beyond that, name the file and the symbol — `crates/ubiq/src/app/wire.rs`, `spawn_pane()` — and drop
  the block.
- **Never cite line numbers.** They rot within one commit. File plus symbol is stable and greppable.
- Fences are for what cannot be pointed at: a message shape, a directory tree, a shell invocation.

## 5. Timelessness

Documents in `product/`, `features/` and `tech/` are written in the present tense and describe only
what holds today. A reader must never have to reconstruct a timeline to know the current state.

**Banned phrasings** (`just docs-lint` greps for these):

```
now  already  no longer  used to  previously  currently  not yet
will be  has been added  was changed
```

Status banners announcing that a document disagrees with another document are banned outright. That
condition is a `wip/` note or a `backlog.md` entry, never a header. The honest way to say a design
runs ahead of the code is `status: draft` in the frontmatter — a machine-readable fact, not prose a
reader has to date.

History has exactly three homes:

- **git** — what changed and when.
- **`tech/decisions.md`** — why this and not the alternative. A decision may name what it replaced,
  because rationale stays true even when the thing it replaced is gone.
- **`review-log.md`** — what the bookkeeper did.

Forward-looking content is allowed in two places: a document's final `## Next steps` (capped, no
dates, no promises), and `backlog.md`.

## 6. Frontmatter

Every document under `_docs/` carries it, and the field list with its per-field meaning lives in
[`authoring.md`](./authoring.md) — where contributors need it, and stated once. This section holds
only what the field list cannot: why the fields exist, and how to judge a bad one.

`id` prefixes mirror the folder: `prod-`, `feat-`, `tech-`, `meta-`, `wip-`. An `id` is never changed
once written, since `depends_on` cites ids precisely so a retitled or relocated document does not
break the graph. Renaming a file is free; renaming an id is not.

**`read_when` is the field that gets a document opened,** and the one worth rejecting when it is
lazy. `summary` says what a document contains; `read_when` names the situation that should send
someone to it, phrased as the task at hand and not as the subject matter — "you are adding a message
variant", not "message conventions". `INDEX.md`'s reading paths are assembled from these lines, so a
document without one is effectively invisible to an agent that does not already know it exists.

**`updated` versus `verified`** carries the weight. Git records when a file was touched; it cannot
record when someone last confirmed the document still matches reality. `verified` plus `code_anchors`
answers the only question that matters for maintenance: *has any anchored file changed since this
document was last confirmed?* That turns upkeep from a judgement call into a queue.

`code_anchors` names the files a reader would open next — the two or three that carry the document's
claims, not every file it mentions. Omit for `product/` and `_meta/`.

## 7. Cross-references

- **One fact, one owner.** Each class of fact — message variants, theme tokens, commands,
  decisions — is stated in exactly one document. Everyone else links. The ownership registry lives
  in `INDEX.md`.
- **Derived content is generated or linked, never copied.** Directory trees and document listings
  are produced by `just docs-index` between markers, or replaced by a pointer. Hand-maintained
  copies drift silently and are the main way a library rots.
- **A link target appears at most three times per document,** one of them in `## Related docs`.
- **Cite across documents by title, not by section number.** Section numbers are useful *within* a
  document and break the moment one is reorganized.
- **Cite code by path and symbol.** Never a line number (§4.3).

`crates/agent-manager/` is a separate library with its own `_docs/` and its own `AGENTS.md`. This
library never restates what that one owns — it links, and the boundary between the two is stated once
in `tech/agent-manager.md`.

## 8. The bookkeeper

### 8.1 Powers and limits

> **The bookkeeper rewrites form and proposes substance.**

**May act directly:** fix frontmatter; regenerate `INDEX.md` and the code map; move, split or merge
documents per §2 and §4; repair broken path and symbol references; rewrite transient phrasing into
the present tense; relocate open questions into `backlog.md`; delete `wip/` documents whose task has
closed; file `inbox/` items.

**Must not act, must propose instead:** changing a normative rule, a product decision or an
architectural invariant; adding a new rule; resolving or reprioritising a backlog item; deciding
that a documented behaviour is wrong.

Without that line a documentation agent quietly becomes an architect, and the library stops being
trustworthy as a record of what was decided. When in doubt, the material is substance.

**On a mismatch, apply the direction rule** — some documents describe the code and yield to it,
others prescribe and outrank it. The contributor's guide owns that rule and states it in full.

The bookkeeper's limit is worth naming: **it detects drift but must not resolve it.** Finding that an
anchored file changed after a document's `verified` date is mechanical; judging whether the document
is still true requires knowing what the change meant, which the bookkeeper does not. Re-stamping
`verified` on a document nobody actually checked is the worst available outcome: it converts
*unknown* into *confirmed* with no one having looked.

**Contradictions are recorded, not silently resolved.** Where two documents disagree, write the
disagreement down with both sides named — in `backlog.md` if it is a project question, in
`feedback.md` if it is a documentation one.

### 8.2 Triggers

| Trigger | Scope |
|---|---|
| A task touched `crates/` | Only documents whose `code_anchors` intersect the diff: re-verify or record drift |
| `inbox/` is non-empty | Classify and file, or propose a new document |
| Weekly | Light pass: checks L1, L2, L4, L5, L7, L9, L10 |
| Monthly | Deep pass: the drift queue (L3) plus the light pass |
| Quarterly | Full pass: L1–L10, `backlog.md` triage, and whether the taxonomy still fits |
| On request | As asked |

### 8.3 The revision checklist

`just docs-lint` runs every mechanical check below. Cite these ids in reports and feedback entries.

| id | Check | Failure action |
|---|---|---|
| **L1** | Frontmatter parses; `id` unique; `depends_on` resolves | Fix |
| **L2** | Every referenced file path, symbol and link exists | Fix the reference, or record drift if the code moved |
| **L3** | No `code_anchors` file changed after `verified` (`just docs-drift`) | Read the diff; fix only what is unambiguous, otherwise queue it |
| **L4** | Length and code-density bands respected (§4) | Split, fold, or propose |
| **L5** | No banned phrasing in a `status: current` document (§5) | Rewrite in the present tense |
| **L6** | No fact stated outside its owning document (§7) | Replace with a link |
| **L7** | No orphans: unlisted in `INDEX.md`; `wip/` past its task; `inbox/` older than 14 days | File, list, or delete |
| **L8** | No `Open questions` or `Known gap` heading in a `current` document | Move the content to `backlog.md` |
| **L9** | No link target appears more than three times within one document | Drop the surplus; keep the `## Related docs` entry |
| **L10** | The update duty and the `authoring.md` pointer are present in `AGENTS.md` and `INDEX.md` | Restore them before anything else (§8.5) |

L1, L2, L4, L5, L7, L9 and L10 are mechanical and scripted. L3, L6 and L8 need judgement about
meaning and stay with a capable model.

### 8.4 Reporting

Every pass ends with:

1. A short report: what was checked, what was fixed, what was found and left alone.
2. One line appended to `review-log.md` — date, scope, checks run, counts.
3. At most **three** new entries in `feedback.md`.

Never a silent pass. "Nothing to change" is a result and gets its log line.

### 8.5 Keeping the duty discoverable

**A duty nobody is pointed at does not exist.** Most documents here are updated by agents whose job
is code and who will never open this file. They comply only if something they already read tells them
to, at the moment it applies. Maintaining those channels is the bookkeeper's job, and it ranks above
tidying prose:

| Channel | Carries | Why it matters |
|---|---|---|
| `AGENTS.md` | The duty in one sentence, and a pointer to `authoring.md` | Always loaded, so the only guaranteed channel. It works *because* it is short — let it grow into a catalogue and the pointer drowns |
| `INDEX.md` | The same pointer, beside the reading paths | Read at the start of a task, by an agent that may skim the preamble |
| `just docs-touched` | The documents anchored to the files in your diff | Fires exactly when the duty applies, and depends on nobody having read anything |

Redundancy across these three is deliberate: they fail independently. **Check L10 on every pass** — a
reorganization that quietly drops the pointer from `AGENTS.md` has broken the library's upkeep while
leaving every document intact.

## 9. Proposing a change

`feedback.md` is the bookkeeper's channel for everything it may not do itself (§8.1). Append-only
table: `id · date · target · kind · rationale · status`, where *kind* is one of
`split · merge · move · rename · rule-change · gap · drift`.

Three rules make it survive agents with no memory of previous passes:

- **Cap at five open entries.** When full, report the finding instead of appending — a backlog of
  proposals nobody triages is noise, and the cap forces triage.
- **Rejections keep their reason, permanently.** A declined proposal stays in the table with its
  one-line resolution, so the next bookkeeper does not raise it again.
- **Accepted proposals are executed by the bookkeeper,** then closed with what was done.

One rationale sentence per entry. If it needs a paragraph, it is substance and belongs in a
conversation with a human, not in a ledger.

---

## Related docs

- [`authoring.md`](./authoring.md) — the duty every contributor owes this library
- [`feedback.md`](./feedback.md) — the open proposal ledger
- [`review-log.md`](./review-log.md) — what past passes did
- [`../INDEX.md`](../INDEX.md) — the catalogue, the fact-ownership registry, and the reading paths
