# Reorganising the library, and the registers

The rulebook is `_docs/_meta/librarian.md`. Read it before creating, moving, splitting or deleting
anything under `_docs/`. This file is the working summary.

Before a maintenance pass, read: `librarian.md`; `authoring.md`, so you know what contributors were
instructed to do and do not "fix" it; `INDEX.md`, for the catalogue and the fact-ownership
registry; `feedback.md` and `review-log.md`, so you neither repeat a rejected proposal nor redo
last week's work.

## The bookkeeper's line

> **The bookkeeper rewrites form and proposes substance.**

| May act directly | Must propose instead |
|---|---|
| Fix frontmatter | Change a normative rule, a product decision or an architectural invariant |
| Regenerate `INDEX.md` and the code map | Add a new rule |
| Move, split or merge per the placement rule and the length bands | Resolve or reprioritise a backlog item |
| Repair broken path and symbol references | Decide that a documented behaviour is wrong |
| Rewrite transient phrasing into the present tense | |
| Relocate open questions into `backlog.md` | |
| Delete `wip/` documents whose task has closed; file `inbox/` items | |

When in doubt, the material is substance. Without that line a documentation agent quietly becomes
an architect and the library stops being a trustworthy record of what was decided.

**Detect drift; do not resolve it.** Finding that an anchored file changed after a document's
`verified` date is mechanical (`just docs-drift`); judging whether the document is still true
requires knowing what the change meant. **Re-stamping `verified` on a document nobody read is the
worst available outcome** — it converts *unknown* into *confirmed* with no one having looked.

**Contradictions are recorded, not silently resolved**: both sides named, in `backlog.md` if it is
a project question, in `feedback.md` if it is a documentation one.

## Triggers

| Trigger | Scope |
|---|---|
| A task touched `crates/` | Only documents whose `code_anchors` intersect the diff: re-verify or record drift |
| `inbox/` is non-empty | Classify and file, or propose a new document |
| Weekly | Light pass: L1, L2, L4, L5, L7, L9, L10 |
| Monthly | Deep pass: the L3 drift queue plus the light pass |
| Quarterly | Full pass: L1–L10, `backlog.md` triage, and whether the taxonomy still fits |

Every pass ends with a short report, **one line appended to `review-log.md`**, and at most three
new `feedback.md` entries. Never a silent pass — "nothing to change" is a result and gets its line.

## Splitting and folding

- Target **150–400** lines, ceiling **500**. Exempt: `INDEX.md`, `backlog.md`, the glossary, the
  decision register, the code map, the transport contract, and the two `_meta/` ledgers — all read
  by lookup rather than read through.
- Over 500 → split **by reader intent**, never by topic-slicing. Pull the runbook out of the
  architecture document; never create `architecture-part-2`.
- Under 80 lines with no independent reader → fold into its parent.
- A document may be dominated by at most two of: narrative/rationale, normative rules, reference
  tables, procedures, catalogue. **A procedure never lives inside an architecture document** —
  different readers, different half-lives, different failure modes when stale.
- After any move: fix `depends_on` (ids survive a move; a *renamed* id does not), re-link from
  `INDEX.md`, update the fact-ownership registry, and run `just docs-index` then `just docs-lint`.

## Keeping the duty discoverable — check on every pass

| Channel | Carries |
|---|---|
| `AGENTS.md` | The duty in one sentence, and the pointer to `_docs/_meta/authoring.md`. Always loaded, so the only guaranteed channel — it works *because* it is short |
| `INDEX.md` | The same pointer, beside the reading paths |
| `just docs-touched` | The documents anchored to the files in your diff, firing exactly when the duty applies |

The redundancy is deliberate: the three fail independently. L10 is the mechanical half of this, and
it ranks above tidying prose — a reorganisation that quietly drops the pointer from `AGENTS.md` has
broken the library's upkeep while leaving every document intact.

## The decision register — `_docs/tech/decisions.md`

One entry per structural choice, cited as `D7` across the library and **never by row position**.
**Appending the row is part of making the decision**, not follow-up work.

```markdown
### D42 — <the choice, as a claim in the present tense>

<Two or three paragraphs: what is chosen, and the alternative it was weighed against.>

**Why:** <only where a reader would otherwise propose the alternative.>

**Cost:** <what this trade costs. Mandatory.>
```

**Every entry names its cost**, because a decision recorded without one reads as a law rather than
a trade, and the next reader cannot tell whether the trade still holds.

This is the one document that may name what a choice replaced — rationale stays true even when the
alternative is gone — which is why `tech-decisions` is L5-exempt for *used to*, *previously* and
*no longer*.

Reversal and supersession are **edits to the existing entry, never deletions**, and the numbering
never moves:

- Fully replaced: a bold **Superseded by `D42`.** opening the body, then the original reasoning
  in the past, then a `**Why it was superseded:**` paragraph. `D25` is the worked example.
- Partly replaced: **Half reversed by `D42`.** as a closing paragraph, saying which half fell,
  which half stands, and what the evidence was that the two were separable. `D17` is the example.
- A new entry that reverses an old one still gets its own number and its own cost.

## The backlog — `_docs/backlog.md`

Three tables, each row naming the documents it affects so resolving an item tells you what to
update:

```markdown
## Gaps — the tree lacks something the documentation describes
| # | Item | Affects |
| G156 | <what the tree lacks, in the present tense, naming the code> | [`wip/indexing.md`](./wip/indexing.md) |

## Open questions — a decision nobody has made
| # | Question | Affects |
| Q5 | <the question, and the options it is between> | [`tech/transport-contract.md`](./tech/transport-contract.md) |

## Deferred — decided to wait
| # | Item | Why it waits |
| D1 | Splitting the coordinator into its own process | The contract makes it cheap later |
```

- `Gnn` ids are allocated by taking the next unused number; ids are cited from other rows and from
  documents, so **never renumber**. (Duplicate ids exist in the register today — `P8` in
  `feedback.md` is the open proposal about it.)
- A gap row is where a `status: draft` document's honesty lives. Write the gap here and keep the
  document's prose unhedged.
- **An item that ships or is dropped leaves this file.** Its outcome lives in git, or in the
  decision register if it settled something structural.
- Documentation-structure proposals go to `_meta/feedback.md`. The test: does resolving it change
  what Ubiq does (backlog), or where a document lives (feedback)?

## The proposal ledger — `_docs/_meta/feedback.md`

Append-only table `id · date · target · kind · rationale · status`, `kind` one of
`split · merge · move · rename · rule-change · gap · drift`. Open rows above, a Closed table below.

- **Cap of five open entries.** When full, report the finding instead of appending — a backlog of
  untriaged proposals is noise, and the cap forces triage. (Rows filed past the cap say so and why.)
- **Rejections keep their reason permanently**, so the next bookkeeper does not raise them again.
- **Accepted proposals are executed by the bookkeeper**, then closed with what was done.
- One rationale sentence per entry. If it needs a paragraph it is substance, and belongs in a
  conversation with a human.

## The review log — `_docs/_meta/review-log.md`

One row per pass: `date | scope | checks | found | fixed | left alone`. Append-only, never a silent
pass.
