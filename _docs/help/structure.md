---
id: help-structure
title: Help structure
kind: help
status: draft
summary: The information architecture of the user manual — its sections in reading order and nav order, the five page types with a skeleton and an exemplar each, page ids and order, when a page splits, how context keys and element targets are allotted, and what the index page does.
read_when: you are adding, moving or splitting a help page, or choosing its type, section, id or context key
updated: 2026-10-09
verified: 2026-10-09
code_anchors: [help/manifest.toml, _tools/helpbundle.py]
depends_on: [wip-help, wip-help-content]
---

# Help structure

This document owns the information architecture of the manual: the sections and what belongs in each, the five page types (concept, reference, guide, how-to, FAQ) and their skeletons, `nav.order`, when a page splits, how `context:` keys are allotted, and the job of the `index` page.

`_docs/help/` describes how the manual is written; `wip/help.md` keeps how the machinery works; `help/` is the manual. `features/` owns every behavioural fact, and a help page restates it in user terms without contradicting it.

The words on a page belong to [`style`](./style.md), its pictures to [`images`](./images.md), and
the record of which area has which page to [`coverage`](./coverage.md).

## 1. Sections, in reading order

The manual reads from *why* to *what* to *what went wrong*. A reader who starts at the top and
stops anywhere has learnt the most useful thing first.

| # | Section | Folder | Page type | What belongs here | Facts come from |
|---|---|---|---|---|---|
| 1 | Start | `index`, `getting-started` | — | The landing page (§7) and the first run: open a project, start one agent, see it work | `inbox/onboarding-proposal.md`, everything below |
| 2 | Philosophy | `philosophy/` | concept | What Ubiq believes and refuses, written for a user: many agents in one window; a real terminal per agent, and why the chat panel exists anyway; any harness, no model chosen for you; your machine, credentials referenced and never held; what Ubiq is not | `product/overview.md`, the root `AGENTS` domain rules |
| 3 | Concepts | `concepts/` | concept | The nouns: projects, panes and panels, workspace sessions, conversations, accounts and profiles, harnesses, and the glossary | `product/glossary.md`, `features/panes-and-terminals.md`, `features/sessions-and-workspaces.md`, `features/workbench.md` |
| 4 | Guides | `guides/` | guide | One real task end to end, done in a running build before it is written: first project and agent; two agents on one task; reviewing and committing an agent's work; the knowledge base; accounts and profiles for several clients; the Tasks board; a remote machine; querying a database | the reference pages of every area the guide crosses |
| 5 | How-to | `how-to/` | how-to | One task per page, named for the verb, harvested from the reference pages' "What you can do here" and then from the FAQ | the reference page the task lives on |
| 6 | Reference | `agents/`, `workbench/`, `git/`, `kb/`, `db/`, `tasks/`, `teams/`, `remote/`, `settings/` | reference | One page per area of the interface, bound by `context:` (§5) | the area's `features/` document |
| 7 | FAQ | `faq/` | FAQ | One file per topic — `faq/agents`, `faq/workbench`, `faq/accounts`, `faq/troubleshooting` — for the questions the rest of the manual answers somewhere a reader does not look | user-visible limits in `backlog.md`, phrased as the user meets them; `troubleshooting`; what guide authors got stuck on |
| 8 | Troubleshooting | `troubleshooting` | — | Symptom, cause, fix — one heading per symptom, worded as the user sees it | `features/logs.md`, the failure behaviour in every feature document |

`nav.order` in `help/manifest.toml` lists the folder column in this order, a folder as `name/` and
a single page by its file name:
`["index.md", "getting-started.md", "philosophy/", "concepts/", "guides/", "how-to/", "agents/", "workbench/", "git/", "kb/", "db/", "tasks/", "teams/", "remote/", "settings/", "faq/", "troubleshooting.md"]`.
A new folder joins the list in the same change that adds its first page; the packer leaves an
unlisted folder out of the nav.

### 1.1 The reference folders

The reference folders follow the rail: a project mode is a folder, and what every mode shares is
`workbench/`. An area that fits no other folder goes in `workbench/`.

| Folder | Holds a page for |
|---|---|
| `agents/` | Agents mode, the chat panel, accounts, profiles, permission modes, connectors |
| `workbench/` | The titlebar, the rail, the dock and its panels, the IDE's explorer, editor and search, logs, notifications, stats, the kitchen sink |
| `git/` | Git mode and each of its panels |
| `kb/` | The knowledge base mode and its panels |
| `db/` | DB mode and its panels |
| `tasks/` | The Tasks board |
| `teams/` | Teams mode |
| `remote/` | Drones and remote hosts |
| `settings/` | One page per settings section |

The first page of a reference folder, `order: 10`, is the mode's overview and carries its `rail.`
key; the panels follow it.

## 2. Page types

Every page is exactly one type. The type decides the folder, the skeleton and the question the page
answers; a page that answers two questions is two pages (§4).

| Type | Answers | Skeleton, in order |
|---|---|---|
| Concept | *what is it, and why* | `## What it is` — the idea in two or three paragraphs, no steps · `## Why it works this way` — the reason, in the user's terms |
| Reference | *what is on this screen* | an untitled intro paragraph: where the area sits and what it is for · `## The parts` — a table, one row per control: its icon where it has one, its label in bold, one sentence · `## What you can do here` — links to how-tos and guides, no steps inline · `## When something goes wrong` — the failures a user meets here, each with its way out |
| Guide | *walk me through a real task* | `## What you will end up with` — the outcome in one or two sentences · `## Before you start` — what must be true first · `## Steps` — a numbered list, a screenshot after any step whose result is not obvious · `## What next` — one to three links |
| How-to | *how do I do this one thing* | an untitled sentence saying when you need it · a numbered list of steps · one line saying what you see when it worked |
| FAQ | *the question people actually ask* | per question: `## <the question, as a user would ask it>` · two to six sentences that answer it · a link to the page with the full answer |

**Choosing the type.** Ask what the reader typed or pressed to arrive. F1 on a screen wants a
reference page. "How do I…" wants a how-to; "walk me through…" a guide; "what is a…" or "why
does…" a concept. A question that has a full answer elsewhere, but which readers keep asking in
their own words, is a FAQ entry pointing at it.

**No closing link heading.** A page ends with `related:` in its frontmatter, never with a
`## Related` or `## See also` heading: the panel draws `related:` as a "see also" strip at the foot
of the page, and a heading would print the same links twice. Every page still links to at least one
other page from its body.

### 2.1 Exemplars

One page of each type, to copy. Each restates facts from the `features/` document named in its
column, and every control it names exists in the shipped build. Their frontmatter is below, written
as YAML exactly as `wip/help.md` §2.1 shows; `status` is omitted because each is `current`.

| Field | Concept | Reference | Guide | How-to | FAQ |
|---|---|---|---|---|---|
| file | `concepts/panes` | `git/changes` | `guides/review-and-commit` | `how-to/hide-or-close-a-tab` | `faq/workbench` |
| `id` | `panes` | `git-changes` | `guides-review-and-commit` | `how-to-hide-or-close-a-tab` | `faq-workbench` |
| `title` | Panes and panels | The Changes panel | Review and commit what an agent changed | Hide or close a tab | Workbench questions |
| `summary` | What a panel is, what a pane is, and why every agent gets a real terminal. | The conflicted, staged and unstaged lists in Git mode, and the commit box under them. | Read each file an agent touched, stage what you keep, and commit it. | Choose whether a tab goes away with its harness still running, or ends it. | What people ask about tabs, panels and the window. |
| `keywords` | `[terminal, tab, focus, dock, split]` | `[stage, unstage, discard, commit, amend]` | `[review, diff, stage, commit]` | `[close, hide, detach]` | `[tab, panel, chat, closed]` |
| `context` | — | `[panel.ubiq.git.changes]` | — | — | — |
| `order` | `10` | `20` | `30` | `40` | `20` |
| `related` | `[workbench-panels, glossary]` | `[git-overview, git-history]` | `[git-changes]` | `[panes]` | `[how-to-hide-or-close-a-tab]` |
| source | `features/panes-and-terminals.md`, the glossary | `features/workbench-git.md` | `features/workbench-git.md` | `features/workbench.md`, Editor settings | `features/workbench.md`, Editor settings |

**Concept** body:

```markdown
## What it is

Everything in the window is a [panel](workbench-panels); a pane is the panel holding one harness.

## Why it works this way

A harness redraws the whole screen and reads every key, so only a real terminal shows it as meant.
```

**Reference** body:

```markdown
The Changes panel sits on the right of [Git mode](git-overview) and lists what differs from HEAD.

## The parts

| Part | What it does |
|---|---|
| **Conflicted**, **Staged**, **Unstaged** | One list each, with a count; click a heading to fold it |
| **+** / **-** on a row | Stages or unstages that path; on a list's heading, every path in it |

## When something goes wrong

A conflicted path has no **+** or **-**. Discarding a change asks you to confirm first.
```

The full page adds `## What you can do here`, linking the guide below.

**Guide** body:

```markdown
## What you will end up with

One commit holding the changes you accepted.

## Before you start

An agent has finished a task in a project that is a git repository.

## Steps

1. Pick **Git** on the rail, click a path under **Unstaged**, and read its diff.
2. Press **+** on each path you keep, then write a message in the commit box and commit.
```

The full page puts a screenshot after step 1 and ends with `## What next`.

**How-to** body:

```markdown
Use this when you want a tab out of the way without stopping its agent, or gone for good.

1. Right-click the tab, and pick **Hide** to keep its harness running or **Close** to end it.
2. To change what the tab's **×** does, open **Settings**, then **Editor**, then **Closing a tab**.

A hidden pane is listed in the new-pane menu's **Detached** group. [Panes](panes) explains why.
```

**FAQ** body, one question of the file:

```markdown
## Why does closing a chat tab not end the conversation?

A chat tab's **×** hides it, because the conversation belongs to Ubiq, not to the tab. To end it,
pick **Close** from the tab's right-click menu — see [Hide or close a tab](how-to-hide-or-close-a-tab).
```

## 3. Ids, files and order

| Field | Rule |
|---|---|
| file | `help/<folder>/<name>.md`, `<name>` lowercase and hyphenated: a noun for a concept or reference page, the verb phrase for a how-to or guide (`hide-or-close-a-tab`), the topic for a FAQ file |
| `id` | `<folder>-<name>` for a page in a folder; the bare name for a single page at the root. An id is forever: links, the context map and the agent's help tools all address it. Ids that predate this rule (`panes`, `sessions`, `projects`, `glossary`) stay as they are |
| moving a file | keep the id; add the old path to `redirects` |
| `order` | multiples of 10 within the folder, in the order a reader should meet the pages; a reference folder's overview is `10` |
| `status` | `planned` for a stub in the nav before its body exists; `draft` for a page whose facts have not been checked against a running build; otherwise `current`, and the field is omitted |
| `keywords` | the words a user types that the page's prose never uses — another harness's term, the git verb, the symptom |

**Adding a page** is one change: the file with full frontmatter, its folder in `nav.order` if the
folder is new, a link to it from at least one existing page, its row in [`coverage`](./coverage.md),
and `just help-check` green.

## 4. When a page splits

A page splits when any of these holds. The larger half keeps the id; the other half gets a new id
and a link from the first.

- **It answers a second question type.** Steps inside a reference page move to a how-to and leave
  a link in "What you can do here"; a reason inside a how-to moves to a concept.
- **It covers two areas a user reaches separately**, each with its own context key — two panels,
  two settings sections. One area, one page (§5).
- **It outgrows its type's length limit** in the style guide.
- **"The parts" passes about twelve rows.** The area holds a sub-area, and the sub-area becomes its
  own reference page.
- **A FAQ file passes about ten questions.** The topic splits in two, each file named for its half.

Headings are link targets with no redirect, so a split moves whole headings and keeps their words.
A split is also a coverage change: any context key that moves goes with the heading that explains
it, and both pages' rows in the coverage table say so.

## 5. Context keys and element targets

F1 asks *where am I* and resolves through the context ladder in `wip/help.md` §5 — an explicit
page, then `panel.<name>`, `view.<name>`, `rail.<name>`, then `index`. ⇧F1 asks *what is that* and
resolves to one sentence. Both rest on one rule: **an area gets a page, an element gets a
sentence.**

| Place | Gets | Bound by |
|---|---|---|
| A rail mode | its folder's overview page | `rail.<mode>` in that page's `context:` |
| A panel | a reference page | `panel.<name>` in its page's `context:` |
| A screen the navigator reaches | a reference page | `view.<name>` in its page's `context:` |
| A dialog, modal or settings section | a reference page | rung 1: the screen's `help_page` returns the page's id, and no `context:` key is involved |
| A button, field, chip or row | one sentence in the element catalogue, written with the element | the element's id; a page may name it in `targets:` |

- **One key, one page.** A key appears in exactly one page's `context:`. The packer refuses a key
  two pages both claim; the manifest's `[context]` table settles such a clash only when both pages
  genuinely need the key, and stays empty otherwise.
- **A key moves in one change**: off one page and onto the other together, since a key on both
  pages at once fails `just help-check`.
- **A page may claim several keys** when they are one area to the user — the DB mode page claims the
  mode and its three panels.
- **Only reference pages carry `context:`.** A concept, guide, how-to or FAQ is reached by a link,
  never by F1, because F1 answers *where am I* and only a reference page is about a place.
- **`targets:` is optional and many-to-many.** It lists the elements a reference page explains
  beyond their sentence. The packer validates each entry against the element catalogue; the
  interface does not read the map, so a target is never the only way to reach a page.
- **A key with no page falls through** to the next rung and ends at `index`. That is a gap in
  coverage, not a working state.

## 6. Element sentences

The sentence an element gets is help content and follows the style guide, but it is not a page.
One sentence, at most two: what the element is, then what using it does. It speaks about the kind,
never the instance — "a file in the project", not a path. An element that needs more than two
sentences is described in its area's "The parts", and that page lists it in `targets:`.

## 7. The single pages

Three pages sit outside every folder, and each has a job no folder page shares.

| Page | Job | Never |
|---|---|---|
| `index` | The map of the manual, and F1's last rung (below) | steps, a `context:` key |
| `getting-started` | The first run, from launch to one agent working in one project, in the order the first-run flow presents it; ends with a link to the first guide | a second agent, settings beyond what the first run asks |
| `troubleshooting` | One `##` per symptom, as the user sees it; under it the cause in a sentence and the fix in steps | a symptom a reference page's "When something goes wrong" answers fully — that page is linked instead |

### 7.1 The `index` page

`index` is the landing page and the last rung of the ladder, so F1 always opens something. It has
one job: a map of the manual.

- Two sentences on what Ubiq is, linking to the philosophy section for the rest.
- One line per section in §1's order: a link to the section's first page and half a sentence on
  what the reader finds there.
- One full-window screenshot, the only one outside a guide's first step.

It carries no fact found nowhere else, no steps and no `context:` key. `getting-started` is the
first page it links to.

## Related docs

- [`wip/help.md`](../wip/help.md) — how the help machinery works
- [`wip/help-content.md`](../wip/help-content.md) — the programme that writes the manual
- [`style`](./style.md) — voice, terminology, formatting and length
- [`coverage`](./coverage.md) — every area, its page and its key
