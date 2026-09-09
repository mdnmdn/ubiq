---
name: ubiq-docs
description: Reference for Ubiq's documentation library (_docs/) — the same-commit update duty, where a document goes, the frontmatter schema, the docs-* tooling and every lint rule, and the rules for reorganising the library, the decision register and the backlog. Use when finishing any code change, writing or updating a document, adding a Dnn decision or a Gnn backlog row, running or fixing `just docs-lint`, or moving/splitting anything under _docs/.
---

# The Ubiq documentation library

`_docs/` exists so an agent that arrives with no memory can find the two or three documents its
task needs, **and trust them**. A wrong document is worse than none, because it is believed.

The owning documents are `_docs/_meta/authoring.md` (the contributor's duty — self-sufficient) and
`_docs/_meta/librarian.md` (structure and maintenance). This skill is the working reference over
both; where they disagree with it, they win.

## After a code change — the whole workflow

AGENTS.md states the duty: **your change updates the documents it touched, in the same commit.**
Not follow-up work.

1. **`just docs-touched`** — with no arguments it reads `git diff --name-only HEAD` (skipping
   `_docs/` itself) and prints two things: a table of changed files against the documents whose
   `code_anchors` name them, and a list of changed files **anchored by no document** — the
   missing-document case. Pass paths to ask about files you have not committed against.
2. **If nothing turned up**, check `_docs/INDEX.md`: its catalogue says which document owns which
   capability, §3 says which owns which class of fact, §4 says which to read for your task. Then
   grep `_docs/` for the symbol or file you touched. Still nothing, and the change is
   user-visible? See "The one document you may create" below.
3. **Work out what each named document owes** — the table under "When you owe an update".
4. **Edit it in place.** Match the document you are in: its section order, heading depth, table
   style and level of detail are already correct. Bump `updated`. Bump `verified` to today if you
   read its claims against the code and they held; fix what broke first if they did not.
5. **`just docs-lint`** (or `just docs-lint _docs/features/foo.md` while iterating). Exit 1 means a
   `fail`-severity finding; warnings do not fail. Every rule and its trigger is in
   [`reference/frontmatter-and-lint.md`](reference/frontmatter-and-lint.md).
6. **`just docs-index`** if you added a document, or changed a `title`, `summary`, `verified` or
   `code_anchors` — it regenerates INDEX.md's catalogue and `tech/code-map.md`'s tree and
   file→document inversion. `just docs-check` fails instead of writing. `just verify` runs
   `docs-lint` but **not** `docs-check`, so a stale catalogue lands silently.

### When you owe an update

| If your change… | Then… |
|---|---|
| Alters behaviour a user can see | That feature document's *Behaviour* **and** *Implementation* |
| Adds, renames or moves a module or symbol a document names | Fix the references — grep the old name across `_docs/` |
| Adds, removes or reshapes a transport message | `tech/transport-contract.md` |
| Changes a build, test or run command | `tech/operations.md` |
| Adds or changes a theme token or a UI convention | `tech/ui-and-design.md` |
| Makes a structural choice someone might later reverse | A `Dnn` row in `tech/decisions.md` — what, why, what it costs |
| Leaves something unresolved | One `Gnn` line in `backlog.md`, naming the document it affects |
| Touched a file in some document's `code_anchors` | Re-verify that document and stamp `verified` |

**Not your job:** restating what the code plainly says. Name the module and the function and stop —
Rust `//!` headers carry that layer and cannot drift.

**On a mismatch, decide which side is authoritative before editing either.** A *Behaviour*
section, any rule under `tech/`, a decision row and anything in `product/` **prescribe**: if the
code disagrees, the code is wrong — fix it or file it, never edit the document to match, which
turns a bug into a documented feature. An *Implementation* section, the code map and an anchor list
**describe**: there the document yields.

## Two verbs, and the one exception

You have exactly two: **amend** an existing document, and **append** a row or bullet to one.

| You may | You may not |
|---|---|
| Edit any section of an existing document | Move, split, merge or rename anything |
| Append a row to a register, or a line to `backlog.md` | Reorganise sections, or change a document's shape |
| Add a `## Next steps` bullet (≤8, no dates) | Invent a rule, or change one that exists |
| Re-stamp `verified` | Create a document, except the one case below |
| Drop raw material into `inbox/` | Leave a `TODO` or an `Open questions` heading behind |

Content is yours; **structure belongs to the librarian** — arranging the library needs a view of
all of it. Anything you may not do goes to one of three places, no permission needed: raw
undistilled material → `inbox/`; a structural itch (this should be split, those two contradict
each other) → one row in `_meta/feedback.md`, stating both sides rather than picking a winner;
unresolved substance → one line in `backlog.md`.

**The one document you may create:** you shipped a user-visible capability nothing covers, and
*deleting that capability from Ubiq would delete the document*. Then write it under `features/`,
now, by copying the nearest sibling and replacing its content — that gives you the right
frontmatter and the right section order. Anything you are unsure about is a "no". Never create a
folder. Never a second document for a capability that already has one.

## Where a document goes

Folders encode **kind of knowledge**; frontmatter encodes **stability**. Never mix them — a
`drafts/` folder would only duplicate `status: draft`.

| Path | Holds |
|---|---|
| `INDEX.md` | The map: catalogue, fact-ownership registry, reading paths |
| `backlog.md` | Every open question, known gap and deferred item, project-wide |
| `product/` | Why Ubiq exists, in user terms. **No code references** |
| `features/` | One document per user-visible capability: contract on top, implementation below |
| `tech/` | Cross-cutting models, rules, conventions and procedures |
| `references/` | Specifications of protocols Ubiq speaks but does not own. Exempt from the length and fence caps |
| `design/` | Wireframes, prototypes, captured artifacts. **Assets, not documents** — excluded from every lint check, because editing evidence to satisfy a linter destroys what makes it evidence |
| `wip/` | The current task's working notes. Deleted when the task closes |
| `inbox/` | Raw unprocessed input, waiting to be filed |
| `_meta/` | How this library works. The underscore means *not project knowledge* |

> **The placement rule: if deleting the capability from Ubiq would delete the document, it belongs
> in `features/`. If the document would survive, it belongs in `tech/`.**

Pane focus behaviour → `features/`. The transport contract every message obeys → `tech/`. The
session model → `features/`. The rule that the UI never touches a PTY → `tech/`. In order:
unprocessed input → `inbox/`; notes for the task in flight → `wip/`; about the documentation →
`_meta/`; unresolved → a `backlog.md` row, not a document; an image or prototype → `design/`,
linked from `tech/`; product-only, no code → `product/`; otherwise the placement rule.

**Prefer extending an existing document to adding one.**

## The conventions

1. **Present tense, current state**, everywhere outside `wip/` and `inbox/`. Never "now", "already",
   "no longer", "used to", "previously", "currently", "not yet", "will be", "has been added", "was
   changed" — `just docs-lint` greps for exactly these. A reader must never reconstruct a timeline;
   history is git's, the decision register's and `review-log.md`'s.
2. **One fact, one owner.** Each class of fact is stated in exactly one document and everyone else
   links. `INDEX.md` §3 is the registry; a copy that is not the registered owner is the one to
   replace with a link.
3. **`status: draft` means the design is settled and the code is behind it.** Much of Ubiq is
   designed ahead of implementation. Draft is not licence to hedge — the gaps are `Gnn` rows in
   `backlog.md`, never "not yet" in the prose and never a status banner in a heading.
4. **Cite code as file plus symbol** — `crates/ubiq/src/app/wire.rs`, `spawn_pane()`. Never a line
   number; they rot within one commit.
5. **Keep code out.** ≤20 lines per fence, ≤15% of the document. Beyond that, name the file.
6. **A link target appears at most three times per document**, one of them in `## Related docs`.
   Cite other documents by title, never by section number.
7. **Length: 150–400 lines target, 500 ceiling.** Past 500, make your edit anyway and file a split
   in `feedback.md` — do not split it yourself.
8. **`crates/agent-manager/` has its own `_docs/` and its own `AGENTS.md`**, and owns every fact
   about harness configuration. This library links there; `tech/agent-manager.md` states the
   boundary once.

Section order is fixed. Feature: `Purpose · Behaviour · Contract · Implementation · Failure ·
Related docs · Next steps` — *Behaviour* checkable by someone who cannot read Rust, *Implementation*
enough for someone who has never opened the repo to know which files to open. Tech: what it is ·
the rules or model · rationale (only where it stops someone "fixing" the design) · Related docs.
Product: purpose · scope · non-goals, with no path, symbol or code.

## Frontmatter

Every document under `_docs/` carries it (`design/` is not documents). The block is already at the
top when you amend: change `updated`, change `verified` if you checked, add any file you made the
document depend on to `code_anchors`, and leave the rest alone.

| Field | Required | Means | How to fill it |
|---|---|---|---|
| `id` | **Yes** — lint fails without it, and on a duplicate | The stable handle `depends_on` cites | Prefix mirrors the folder: `prod-` `feat-` `tech-` `meta-` `wip-` `inbox-`. **Never changed once written** — renaming a file is free, renaming an id breaks the graph |
| `title` | By convention | The catalogue's link text | Falls back to the first H1, then the filename |
| `kind` | By convention | `product` \| `feature` \| `tech` \| `meta` \| `wip`; `inbox/` uses `proposal` | Only mechanical role: groups a root-level document in the catalogue |
| `status` | By convention | `current` \| `draft` \| `superseded`; `inbox/` uses `proposal` | Only `current` is checked for banned phrasing |
| `summary` | By convention | The document's catalogue row | One line, says what is inside |
| `read_when` | By convention | What gets the document opened; feeds INDEX §4 | Phrase as the reader's **task**: "you are adding a message variant", not "message conventions" |
| `updated` | By convention (`wip/`: effectively yes) | You changed the content | ISO date. A `wip/` document with none, or older than 30 days, fails L7 |
| `verified` | By convention | Someone read this against the code and it held | ISO date, stamped only by whoever actually looked. Re-stamping unread turns *unknown* into *confirmed* — the worst outcome available |
| `code_anchors` | Where code exists | The files a reader opens next — the two or three carrying the claims, not everything mentioned | Repo-relative paths; each must exist. Drives `docs-touched`, `docs-drift` and the code map. Omit for `product/` and `_meta/` |
| `depends_on` | Optional | The document graph | **Ids, not paths.** Every one must resolve to a known id or lint fails |
| `review_cycle` | Optional | `quarterly`, or `monthly` where the area moves fast | Declarative — no tooling reads it |

Full field-by-field detail, the undocumented `external_paths` escape hatch, and every lint rule are
in [`reference/frontmatter-and-lint.md`](reference/frontmatter-and-lint.md).

## Registers

- **`tech/decisions.md`** — one `Dnn` entry per structural choice, cited as `D7` and never by row
  position. **Appending the row is part of making the decision.** Every entry names its cost.
  Superseding is an edit to the old entry, not a deletion. This is the one document that may name
  what a choice replaced.
- **`backlog.md`** — `Gnn` gaps, `Qn` open questions, `Dn` deferred items, each row naming the
  documents it affects. A `TODO`, an `Open questions` heading or a "not decided yet" aside belongs
  here instead. An item that ships or is dropped **leaves the file**.
- **`_meta/feedback.md`** — `Pn` proposals the bookkeeper may not execute itself.
- **`_meta/review-log.md`** — one row per maintenance pass. Never a silent pass.

Formats, examples and the reorganisation rules are in
[`reference/librarian.md`](reference/librarian.md).

## Reference files

- [`reference/frontmatter-and-lint.md`](reference/frontmatter-and-lint.md) — the schema field by
  field, every L-check with exactly what triggers it and what a failure means, and what each
  `_tools/docs.py` action does.
- [`reference/librarian.md`](reference/librarian.md) — reorganising the library, what the
  bookkeeper may and may not do, and the decision / backlog / feedback / review-log formats.
