---
id: inbox-ui-id-tooling
title: Proposal — uiids, a lint and search tool for UI element ids
kind: proposal
status: proposal
summary: A Python tool in `_tools/` that answers which UI ids exist, which are declared but never drawn, which are drawn but never declared, which have help content, and which help pages point at ids that are gone — built on the catalogue as the single source of truth with a deliberately narrow grep over Rust as the auditor, because a regex over Rust cannot see an id built at runtime and should not pretend to; six subcommands, a human table and a JSON mode, two `just` recipes, and a place in `just verify` beside `help-check`.
read_when: you are building or changing the UI id lint, adding an id to the catalogue, or deciding how id validation is split against the help packer
updated: 2026-09-20
depends_on: [inbox-ui-id, wip-help, tech-operations, tech-ui]
---

# Proposal — uiids, the lint for element identity

[`ui-id-proposal.md`](./ui-id-proposal.md) makes a catalogue of UI element ids a published contract.
A contract with no checker is a comment, and the checking is a batched, tree-wide operation over
several hundred rows across two languages — which puts it in `_tools/` as a script, not in the Rust
build. This specifies that script, `uiids.py`. It does not implement it.

The tool's value is concentrated in two answers — **what is drawn but undeclared**, and **what
coverage an area has** — and the design below is arranged so those two are exact and the rest are
allowed to be approximate.

---

## 1. The questions

| Question | Subcommand | Exactness |
|---|---|---|
| Which ids exist? | `list` | exact — the catalogue is the answer |
| Which are declared but never drawn? | `unused` | approximate, over-reports |
| Which are drawn but never declared? | `undeclared` | exact, within §2's rule |
| Which have help content? | `coverage --help-only` | exact |
| Which help `targets:` name an id that is gone? | `check` | exact |
| Duplicates, collisions, alias chains | `check` | exact |
| Naming-convention violations | `check` | exact |
| Coverage by area | `coverage` | exact denominator, honest numerator |
| What is this id, and where is it drawn? | `show <id>` | exact plus a grep |

The asymmetry between `unused` and `undeclared` is the whole design, and §2 is why.

## 2. Where the facts come from

Three sources, of very different quality.

**The catalogue — `ui-targets.toml`.** Authoritative, trivially parsed, one row per id with its
kind, label, blurb, optional help page and optional alias. Everything the tool says about what
*should* exist comes from here and nowhere else.

**Help content — the page frontmatter.** `targets:` lists, read with the same frontmatter reader
`helpbundle.py` uses. No second parser.

**Rust — `crates/ubiq/src/`.** This is the hard one, and the honest answer is that a script cannot
extract ids from Rust reliably. Three ways to try:

| Approach | Gets | Misses |
|---|---|---|
| Regex over `.rs` for the literal marking call | every id written as a string literal | ids built with `format!`, passed through a variable or a `const`, derived from an enum's slug in a loop, or produced by a macro |
| A real parse — `syn` or tree-sitter | the same set, with fewer false positives from comments and strings | exactly the same dynamic cases, because they are semantic and not syntactic |
| Catalogue as sole truth — never scan Rust | a clean, fast cross-check of declarations against help | any drift between the catalogue and what the code actually draws, which is the drift that matters |

None of the three is good enough alone, and the second buys almost nothing over the first: the cases
a regex misses are misses because the id does not exist until runtime, and a parser does not change
that. Adding a Rust toolchain to a Python script to gain robustness against comments is a bad trade.

**The recommendation is the third approach as the model, with the first as its auditor — made
reliable by constraining the code rather than by improving the regex.** The marking API takes a
**string literal only**; an id with a dynamic part goes through the instance form, whose path
argument is still a literal and whose selector is not part of the id (`ui-id-proposal.md` §2). So
every declared path in the tree is greppable *by construction*, and the tool fails the build on a
marking call whose argument is not a literal, naming the file and telling the author to use the
instance form. That is what makes `undeclared` exact: not a clever pattern, a narrow one plus a rule
the code obeys.

What stays approximate is `unused`. An id can be declared, drawn only through a path the grep cannot
see, and still be reported unused — so `unused` is a **report, never a failure**, and `check` does
not consider it. A row can also be suppressed with `used_dynamically = true`, which is a claim the
author makes and the tool believes.

**Optional, phase two: `--from-runtime`.** The inspector's per-frame registry can dump the union of
what it saw over a session to JSON, and the tool can read it for coverage. This is sampling, not
proof — a screen never visited contributes nothing — so it feeds `coverage` and never `check`, and
never runs in CI.

## 3. What `check` enforces

Eight rules, all exact, all failures:

- Every declared id is well formed: dot-separated lowercase segments matching `[a-z0-9-]+`, two to
  five of them, first segment from the closed root set in the catalogue's header.
- No id is declared twice, and no id collides with an alias of another.
- An alias chain is one hop. Two is a failure, and the fix is to collapse it.
- Every id's parent path is a declared id with `kind = "area"`. This is what makes the hierarchy real
  rather than a convention about dots.
- `ext.` is reserved: the base catalogue declares nothing under it.
- Every row has a `label` and a `blurb`. An id with no sentence is an id the inspector cannot show.
- Every `help` page id on a row resolves, and every `targets:` entry on a help page resolves to a
  declared id or to a live alias.
- Every marking call in `crates/ubiq/src/` takes a string literal, and its id is declared.

Deliberately not failures: an unused id, an area with low coverage, an id with no help page.

## 4. Interface

```
uv run _tools/uiids.py <subcommand> [--json] [--root PREFIX]

  check       everything in §3; exit 1 on any finding
  list        declared ids, filtered by --root and --kind
  find QUERY  substring search over id, label and blurb
  show ID     one row in full, plus where it is drawn and which pages name it
  unused      declared, not found in the tree — a report
  undeclared  drawn, not declared — the adoption worklist
  coverage    per area: declared, drawn, blurbed, with a help page
```

Human table by default, through `rich`, matching how `docs.py` presents a lint report; `--json` for
anything reading it. `--root` takes an id prefix so an edition, or one area under work, can be
checked alone — the same shape the catalogue's edition list needs.

Exit codes follow `docs.py`: **0** clean, **1** findings, **2** the tool could not run — a malformed
catalogue, a missing source tree. The distinction matters because `verify` should fail loudly on
either but a caller scripting `undeclared` needs to tell "nothing to report" from "nothing read".

Two recipes:

| Recipe | Does |
|---|---|
| `just ui-ids` | the `coverage` summary, the day-to-day view |
| `just ui-ids-check` | `check`, and joins `just verify` |

`ui-ids-check` **belongs in `verify`**, with the same escape hatch `help-check` has: it passes
trivially when `ui-targets.toml` is absent, so a tree part-way through adoption still builds. It
needs no compilation and no bundle, so it costs a second.

## 5. The split against `helpbundle.py`

The two overlap in one place — `helpbundle.py` already validates that every `context` key is claimed
exactly once and names a real panel, view or rail mode, reading those out of the Rust source to do
it. That is the same *kind* of work. It should still stay where it is, and `uiids.py` should
neither extend it nor call it.

**Recommended split: the packer stays mechanical, the linter owns semantics.**

- `helpbundle.py` keeps `context` exactly as it is, and learns `targets:` only as another list to
  invert into the bundle's derived catalogue. Inverting a list needs no knowledge of what a valid id
  is, so the packer never reads `ui-targets.toml`. It runs on every `dev` and every `build` and must
  stay fast and free of extra inputs.
- `uiids.py` owns the catalogue and is the only thing that decides whether an id is real — including
  whether a page's `targets:` entry resolves. It runs in `verify` only.
- The one genuinely shared piece is the frontmatter reader, which moves into a small
  `frontmatter.py` in `_tools/` that both import. Two YAML front-matter parsers in one repository is
  how the two drift.

The alternative — folding id validation into `helpbundle.py` — makes help packing depend on the
interface catalogue, which means a tree with no help content still needs one, and a change to the id
grammar rebuilds the help bundle. The dependency runs the wrong way: help is one consumer of
identity, not its owner.

## 6. Failure modes, and what it must not do

**It over-reports `unused`.** Every id reached through a helper, a loop or a const shows up until
someone marks the row. Present that list as a worklist, never as an error, or it gets suppressed
wholesale and stops meaning anything.

**It cannot tell reachable from drawn.** An id written into a branch no user can enter counts as
used. Proving reachability needs the running app, which is what `--from-runtime` samples and does not
prove.

**It picks up test and fixture literals.** Scanning `crates/ubiq/src/` includes inline test modules.
This is the right answer anyway: an id a test names is an id, and requiring it to be declared is
correct.

**It must not rewrite Rust.** No autofix touches a `.rs` file. `--fix` may do exactly one thing:
append a stub row to the catalogue for each undeclared id, with `label` and `blurb` left empty so
`check` still fails until a human writes them. That makes adoption a batch operation, which is what
puts it in `_tools/`, without letting a script invent meanings.

**It must not become a second `docs.py`.** No prose rules, no length rules, no opinion about help
page content beyond whether a `targets:` entry resolves.

**Nothing in `crates/` ever reads its output.** It is a build-time checker. The runtime reads the
catalogue directly; if the tool vanished the application would be unchanged, and that is the test of
whether the split in §5 is honest.

## 7. Build order

1. `check` over the catalogue alone — grammar, duplicates, alias chains, the area-parent rule. Useful
   before a single element is marked.
2. The Rust grep, `undeclared`, and the literal-argument rule. This is the adoption engine.
3. `list`, `find`, `show`, `coverage`. Authoring comfort.
4. Help cross-checks, once `targets:` exists on pages.
5. `--from-runtime`, if coverage turns out to be a question anyone actually asks.

Phase 1 and 2 are what earns the recipe a place in `verify`.

---

## Related docs

- [`ui-id-proposal.md`](./ui-id-proposal.md) — the catalogue, the grammar and the stability contract
- [`../wip/help.md`](../wip/help.md) — the packer, its validation, and the derived catalogue
- [`../tech/operations.md`](../tech/operations.md) — where the recipes are listed
- [`help-proposal.md`](./help-proposal.md) — why help validation lives outside the Rust build
