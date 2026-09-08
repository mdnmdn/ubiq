# Frontmatter, the lint rules, and the tooling

Everything here is verified against `_tools/docs.py` and the `docs-*` recipes in `justfile`.

## What counts as a document

`_tools/docs.py` treats **every `*.md` under `_docs/` as a document**, except anything under
`_docs/design/` (assets) and the ignored directories (`.git`, `target`, `refs`, `node_modules`,
`.venv`, `__pycache__`, `.serena`). There is no opt-out flag: a Markdown file dropped anywhere else
under `_docs/` is linted, must carry a unique `id`, and must be linked from `INDEX.md`.

## The frontmatter schema

A YAML block delimited by `---` on the first line and a later `---`. Anything else — a missing
block, an unclosed one, YAML that does not parse, a top-level value that is not a mapping — is an
L1 failure.

| Field | Mechanically required | Read by | Notes |
|---|---|---|---|
| `id` | **Yes** (L1: present, unique) | L1, drift, catalogue exemptions, `depends_on` | Prefix mirrors the folder. Immutable — other documents cite it. Several ids are hard-coded in the linter's exemption sets, so renaming one silently changes which checks apply |
| `title` | No | catalogue | Falls back to the first `#` heading, then the filename |
| `kind` | No | catalogue grouping **only for root-level documents** (`GROUP_BY_KIND`: product/feature/tech/meta) | A document inside a folder is grouped by its folder. `backlog.md` sits under **Tech** because it is `kind: tech` |
| `status` | No | L5 | L5 runs only on `status: current`. `draft`, `superseded` and `proposal` are never checked for banned phrasing |
| `summary` | No | catalogue (`—` when absent) | |
| `read_when` | No | nothing mechanical; INDEX §4 is assembled by hand from it | A document without one is invisible to an agent that does not already know it exists |
| `updated` | No, except `wip/` | L7 | A `wip/` document with no parsable `updated`, or one older than 30 days, fails L7 |
| `verified` | No | L3 (`docs-drift`), catalogue column | Compared against the last commit date of each `code_anchors` file |
| `code_anchors` | No | L2 (each file must exist), L3, `docs-touched`, the code map's inversion | A list of repo-relative paths. A bare string is accepted and wrapped |
| `depends_on` | No | L1 (each must resolve), `docs-graph` | Ids, not paths |
| `review_cycle` | No | **nothing** | Declarative intent only |
| `external_paths` | No | L2 | Any truthy value switches off the backticked-path existence check for that document. Used by no document in the tree today |

Dates may be YAML dates or ISO strings; anything else parses as absent.

## The lint rules

`just docs-lint [PATHS]` → `uv run _tools/docs.py lint [PATHS]`.

**L10 and L1 always run over the whole library**, whatever paths you pass. L2, L4, L5, L7 and L9
run over the selected documents only; with no paths, over all of them. If the paths you give match
no document, it prints a warning and lints everything.

**Exit code 1 if any finding has severity `fail`.** Warnings never fail the run. `just verify` =
`check clippy test host ui docs-lint`, so a `fail` anywhere in `_docs/` is a red gate.

| id | Severity | Triggered by | Fix |
|---|---|---|---|
| **L1** | fail | Frontmatter missing/unclosed/unparsable/not a mapping; no `id`; an `id` another document already uses; a `depends_on` entry matching no document's `id` | Fix the block. For a duplicate id, the *second* document alphabetically is reported |
| **L2** | fail | A `code_anchors` path that does not exist; a backticked token that looks like a repo file and is not there; a backticked `symbol()` found nowhere in the source; a Markdown link whose target does not resolve relative to the document | Fix the reference, or record drift if the code moved |
| **L4** | fail / warn | **fail:** over 500 lines; fenced code over 15% of the document; any single fence over 20 lines. **warn:** 401–500 lines; under 80 lines | Split (via `feedback.md`, not yourself), fold, or replace the fence with a file-plus-symbol pointer |
| **L5** | fail | A banned phrase in prose, in a `status: current` document outside `wip/`, `inbox/` and `_meta/` | Rewrite in the present tense |
| **L7** | fail | A document not linked from `INDEX.md`; a `wip/` document with no usable `updated` or last updated over 30 days ago; a file in `inbox/` over 14 days old | List it in `INDEX.md` (run `just docs-index`), close out the `wip/` note, or file the `inbox/` item |
| **L9** | fail | The same link target appearing more than 3 times in one document | Drop the surplus; keep the `## Related docs` entry |
| **L10** | fail | `AGENTS.md` not mentioning `INDEX.md`, not pointing at `_docs/_meta/authoring.md`, or not matching `/same\s+commit/i`; `INDEX.md` not mentioning `authoring.md` | **Restore before anything else** — the library's upkeep is broken while this is red, even if every document is intact |

Not scripted, and left to judgement: **L3** (drift — `just docs-drift` queues it), **L6** (a fact
stated outside its owning document), **L8** (an `Open questions` or `Known gap` heading inside a
`current` document).

### L2, precisely

- A backticked token is a **repo path claim** only when it matches
  `^[\w./-]+\.(rs|toml|md|json|yaml|yml|py|sh|css|html|js|png)$` **and** starts with `crates/` or
  `_tools/`, **and** does not contain `refs/` (read-only reference checkouts of other projects are
  not claims about this tree).
- A backticked bare `something.md` is resolved as a **document**: first as a repo-relative path,
  then by searching for any file under `_docs/` ending in `/something.md`. Documents in `_meta/`
  are exempt, because the ledgers name documents that have since been deleted and resolving those
  names would force the history to be falsified.
- A backticked `name()` must appear as an identifier somewhere under `crates/ubiq-proto/src`,
  `crates/ubiq-host/src`, `crates/ubiq/src`, `crates/ubiq-app/src` or `crates/agent-manager/src`.
  It is a grep over identifiers, not a resolver — a same-named local variable satisfies it, and a
  method the tree genuinely lost is what it catches.
- Markdown links are resolved relative to the containing document. `http://`, `https://`, `#…` and
  `mailto:` are skipped; a `#fragment` is stripped before resolving.
- Each distinct reference is reported once per document.

### L5's banned list

`now · already · no longer · used to · previously · currently · not yet · will be · has been added
· was changed`

Matched case-insensitively on word boundaries, with any whitespace between the words. **Inline code
spans are blanked before matching**, so `` `Date.now()` `` in prose is safe. `tech-decisions` is
exempt from `used to`, `previously` and `no longer` — a decision may say what it replaced.

### L4's exemptions

Read-by-lookup documents skip both the length band and the fence caps: `index`, `backlog`,
`prod-glossary`, `tech-decisions`, `tech-code-map`, `tech-transport`, plus the append-only ledgers
`meta-feedback` and `meta-review-log`. `tech-diagrams` is exempt from the fence caps only — its
value is the verbatim sample — and still bound by the length band. L9's exemptions are `index`,
`backlog` and `tech-code-map`: those documents *are* routing tables, so repetition is the point.

## The tooling

| Command | Does | Exit |
|---|---|---|
| `just docs-lint [PATHS]` | The checks above. `uv run _tools/docs.py lint --json` gives machine-readable findings | 1 on any `fail` |
| `just docs-index` | Rewrites the generated blocks **in place**: `catalogue` in `INDEX.md`, and `tree` + `anchors` in `tech/code-map.md`. Prints a diffstat for what it changed | 0 |
| `just docs-check` | The same, without writing | 1 if a block is out of date |
| `just docs-drift` | L3: every anchored document whose `code_anchors` files were committed **after** its `verified` date, most stale first | 0 always — it detects the *possibility* of drift only |
| `just docs-touched [PATHS]` | With no paths, `git diff --name-only HEAD` minus `_docs/`. Prints changed file → documents anchoring it, then the files no document anchors | 0 always |
| `just docs-graph` | The `depends_on` graph: roots, cycles, isolated documents, and hubs with more than 5 inbound edges | 0 |

### What the generated blocks contain

- **`INDEX.md` `catalogue`** — one table per group (Product, Features, Tech, Meta, Work in
  progress, Unclassified), each row `[title](link) | summary | verified`. `INDEX.md` itself and
  everything in `inbox/` are excluded, which is why an `inbox/` document must still be linked from
  INDEX's prose or tables to satisfy L7.
- **`code-map.md` `tree`** — the source tree of Ubiq's four crates (`agent-manager` draws its own).
  Regeneration **re-reads the existing block** for its hand-written per-entry descriptions and its
  entry order, both of which are unrecoverable from the filesystem, so an unchanged tree produces
  no diff. Keep the two-space-separated description column when you edit it by hand.
- **`code-map.md` `anchors`** — every `code_anchors` entry inverted to file → documents, plus an
  **Unanchored** list of source files no document names, restricted to Ubiq's own crates.

The markers are `<!-- generated:begin NAME -->` / `<!-- generated:end NAME -->`. Delete one and the
action prints `skipped … no markers` rather than failing.
