---
id: wip-help-content
title: Writing the manual — the help content programme
kind: wip
status: draft
summary: The brief for turning the 25-page help stub into a full user manual — the goal, the authoring guide that has to exist first under `_docs/help/`, the button and screenshot images, the new philosophy, guide, how-to and FAQ sections, the coverage of every area against the id inventory, and the task list a coordinator delegates to subagents.
read_when: you are coordinating or doing any part of the help content work — writing a page, the authoring guide, an image, or a coverage row
updated: 2026-10-09
verified: 2026-10-09
code_anchors: [help/manifest.toml, _tools/helpbundle.py, _tools/icons.py, assets/icons/icons.yaml, crates/ubiq/src/state/ui_id.rs, crates/ubiq-proto/src/help.rs]
depends_on: [wip-help, wip-in-place-help, inbox-help]
---

# Writing the manual — the help content programme

The help *machinery* is built and described in [`help.md`](./help.md). What is missing is the
*manual*: `help/` holds 25 short pages (18–78 lines each), 14 of them bound to a context key, and no
written convention for how a page should read. This document is the brief for closing that gap.
It is a work document: it is deleted when the programme closes, and what survives it lives in
`_docs/help/` and in `help/`.

Everything here happens in the base repository (`ubiq/`). Commit with `git -C ubiq commit` from the
Studio root, or plain `git commit` from inside `ubiq/`.

---

## 1. Goal

A user who has never seen Ubiq can learn it from the help panel alone, and a user who has can
answer "what is this, and how do I…" from F1 without leaving the window. Concretely:

1. **Every area gets a page.** Every panel, screen, rail mode, dialog and settings page in
   [`inbox/ui-id-inventory.md`](../inbox/ui-id-inventory.md) §4 is reachable from F1, through the
   context ladder in `help.md` §5.
2. **The manual explains *why*, not only *what*.** A philosophy section says what Ubiq believes and
   refuses; guides walk through real tasks end to end; how-tos answer one question each; a FAQ
   catches what the rest misses.
3. **Pages show the interface.** Buttons appear inline as the same icon the user sees; screens
   appear as screenshots taken one way.
4. **It reads as one voice.** A written authoring guide under `_docs/help/` fixes structure, tone,
   terminology and images, and every page follows it.
5. **It stays correct.** `just help-check` passes; every page links to at least one other; every
   `context:` key exists.

Not in scope: the identity-scheme decision between [`in-place-help`](./in-place-help.md) and
[`inbox/ui-id-proposal.md`](../inbox/ui-id-proposal.md), the `uiids` tool
([`inbox/ui-id-tooling-proposal.md`](../inbox/ui-id-tooling-proposal.md)), help against a remote
host (`G298`), and a Studio help namespace. Those come after the content exists.

## 2. Reference map

Read only what your task needs; the task table in §6 names it per task.

| Document | What it gives this work |
|---|---|
| [`wip/help.md`](./help.md) | The mechanics: page frontmatter (§2.1), body rules (§2.2), link forms (§2.3), images (§2.4), the packer's checks (§3), the context ladder (§5), the panel (§6), what an agent sees (§8), how to add a page (§9) |
| [`inbox/help-proposal.md`](../inbox/help-proposal.md) | Why help is built this way; the phases and what is deferred |
| [`inbox/ui-id-inventory.md`](../inbox/ui-id-inventory.md) | §4 — every place in the interface and the code that draws it: **the coverage checklist**. §6 — the rule "an area gets a page, an element gets a sentence" |
| [`wip/in-place-help.md`](./in-place-help.md) | ⇧F1 and the balloon; the 34 `UiId`s and their one-sentence blurbs |
| [`inbox/ui-id-proposal.md`](../inbox/ui-id-proposal.md) | §5 — why element help is a `targets:` field, not a `context:` key |
| [`inbox/onboarding-proposal.md`](../inbox/onboarding-proposal.md) | First-run flow — what the getting-started page and the first guide must agree with |
| [`product/overview.md`](../product/overview.md) | The source for the philosophy section: what Ubiq is, who it is for, what it refuses to be |
| [`product/glossary.md`](../product/glossary.md) | The source for `help/concepts/glossary` and for the terminology list in the style guide |
| `features/*.md` | The contract of each capability — the *source of truth* a help page explains in user terms |
| [`_meta/authoring.md`](../_meta/authoring.md) | Frontmatter and the folder rule `_docs/help/` is an exception to (§3 below) |
| [`backlog.md`](../backlog.md) | `G296` anchors don't scroll · `G299` search is unranked · `G300` nobody has opened the panel in a running binary · `G319`/`G322` element targets are dropped |
| the `ubiq-icons` skill | The icon registry (`assets/icons/icons.yaml`, 135 SVGs) and the render loop in `_tools/icons.py` |

Which feature document feeds which help section:

| Help section | Sources in `_docs/features/` |
|---|---|
| `concepts/` | `panes-and-terminals`, `sessions-and-workspaces`, `workbench` |
| `agents/` | `workbench-agents`, `chat`, `connectors` |
| `workbench/` | `workbench`, `workbench-ide`, `workbench-sink`, `workbench-tasks`, `workbench-teams`, `logs`, `notifications`, `stats` |
| `git/` | `workbench-git` |
| `kb/` | `wip/kb.md` |
| `db/` | `workbench-db` |
| remote | `drone` |

## 3. The authoring guide — a new `_docs/help/` folder

Before more pages are written, the conventions are written. Today they are scattered across
`help.md` §2 and nobody's head; twenty writers (human or agent) will produce twenty manuals.

`_meta/authoring.md` §2 says *never create a folder*. This one is created on the owner's explicit
instruction, and it costs the plumbing below — do all of it in the same commit, or `just docs-lint`
fails.

**Plumbing.**
- `_docs/help/` documents use `kind: help` and the id prefix `help-`.
- `_tools/docs.py`: add `("help", "Help authoring")` to `GROUPS` and `"help": "help"` to
  `GROUP_BY_KIND`, so the generated catalogue in `INDEX.md` lists them. Then `just docs-index`
  (or whatever recipe regenerates the catalogue — `_tools/docs.py --help`).
- `INDEX.md` §1: a row for `help/` — *How the user manual is written: structure, voice, images*.
- `_meta/authoring.md` §4: add `help-` to the prefix list and `help` to the `kind` list.
- The `ubiq-docs` skill's "Where a document goes": one line.

**Boundary.** `_docs/help/` describes *how the manual is written*. `wip/help.md` keeps *how the
machinery works*. `help/` *is* the manual. `features/` remains the owner of every behavioural fact;
a help page restates it in user terms and never contradicts it — where they disagree, the feature
document wins and the help page is fixed.

**The four documents.**

| File | id | Owns |
|---|---|---|
| `_docs/help/structure` | `help-structure` | The information architecture: the sections and what belongs in each (§5 here, made permanent); the five page types and their skeletons (below); `nav.order`; when a page splits; how `context:` keys are allotted; the `index` page's job |
| `_docs/help/style` | `help-style` | Voice and tone; the terminology list; formatting conventions; what never appears in a page |
| `_docs/help/images` | `help-images` | Inline button icons and screenshots: how they are made, named, sized, themed and referenced (§4 here, made permanent) |
| `_docs/help/coverage` | `help-coverage` | The living table: every area from the inventory, the page that covers it, its context key, its state (`missing` / `stub` / `done`). Replaces §6's table once written |

**Page types**, to be fixed in `structure` — each with a skeleton a writer copies:

| Type | Folder | Answers | Skeleton |
|---|---|---|---|
| Concept | `concepts/`, `philosophy/` | *what is it, and why* | `## What it is` · `## Why it works this way` · `## Related` |
| Reference | `workbench/`, `agents/`, `git/`, `kb/`, `db/`, … | *what is on this screen* | one-paragraph intro · `## The parts` (each control: icon, name, one sentence) · `## What you can do here` (links to how-tos) · `## When something goes wrong` |
| Guide | `guides/` | *walk me through a real task* | `## What you will end up with` · `## Before you start` · numbered steps with screenshots · `## What next` |
| How-to | `how-to/` | *how do I do this one thing* | one sentence of when · numbered steps · one-line result · `## See also` |
| FAQ | `faq/` | *the question people actually ask* | `## <question as asked>` · two to six sentences · a link to the page with the full answer |

**Voice and tone**, to be fixed in `style` — the starting position:
- Second person, present tense, active voice. "Press F1", not "The user may press F1".
- Plain and calm; no marketing, no "simply", "just", "easy", no exclamation marks.
- Lead with the outcome, then the steps. One idea per paragraph.
- Name controls exactly as the interface labels them, in **bold**; keys as `Ctrl+K` / `⌘K`, both
  platforms where they differ.
- Use the glossary's words and only them — *pane*, *panel*, *harness*, *agent*, *project*, *rail
  mode*. The word *session* is ambiguous in Ubiq (root `AGENTS`): say *workspace session* or
  *conversation*.
- No code paths, no Rust names, no message variants, no backlog ids. A user cannot act on them.
- Describe what the shipped build does. A missing feature is absent from the manual, not promised.
- Pages are short: a reference page fits two screens of the help panel; anything longer splits.
- Start at `##` (the title is drawn by the panel); every heading is a link target, so name them for
  life.

## 4. Images

Two kinds, with different jobs.

### 4.1 Button icons, inline

A reference page names a control *and shows it*: "Press ![](../img/ui/titlebar-notifications.png)
**Notifications** to…". The icon comes from the same SVG the interface draws, so it cannot drift
from the button.

- **Source.** `assets/icons/<name>.svg`, listed in `assets/icons/icons.yaml`. Never redraw an icon
  for help; if one is missing, it is missing from the app too — that is `ubiq-icons` work.
- **Tool.** A new `export` subcommand on `_tools/icons.py` (it already rasterises through resvg, the
  same rasteriser GPUI uses) writing `help/img/ui/<name>.png` at 2× the interface size, plus a
  `just help-icons` recipe. A batched tree-wide write belongs in `_tools`, per the root rules.
  Generated files are committed, so `help-check` validates them like any image.
- **Colour.** The interface draws an icon as a mask tinted by a theme token; a PNG has one colour
  and the help panel follows the theme. Export one mid-tone tint legible on both palettes, and
  record the chosen value in `_docs/help/images`. Read the value from `crates/ubiq/src/theme.rs`;
  do not invent one.
- **Verify first.** The markdown renderer supports inline images (`ui/viewer/markdown.rs`), but no
  help page uses one yet. Before exporting 135 icons, put one icon inline in one page, run the app,
  press F1 and check it sits on the text baseline at text height. If it does not, the fallback is
  the icon on its own line in a "The parts" table cell — decide, and write the decision into
  `images`.

### 4.2 Screenshots

- PNG, 2×, dark theme unless the page is about theming (`help.md` §2.4), the default window size,
  the demo project in `_data/config`, no personal paths, account names or tokens on screen.
- Crop to the area the page is about; a full-window shot only on the `index` and a guide's first
  step.
- Named `help/img/<section>/<page-id>[-<n>].png`; alt text says what the picture shows.
- Taken by hand for now (macOS `screencapture -o -l <window-id>`). If more than ~30 are needed, a
  capture recipe is its own task, filed then.
- A screenshot rots when the screen changes: `coverage` lists the pages carrying one, so a UI
  change can find them.

## 5. The manual's new shape

Today's `nav.order` is `index, getting-started, concepts/, agents/, workbench/, git/, kb/, db/,
troubleshooting`. The target, in reading order:

| Section | Folder | New or existing | Contents |
|---|---|---|---|
| Start | `index`, `getting-started` | existing, rewrite | Landing page with a map of the manual; first run aligned with `onboarding-proposal` |
| Philosophy | `philosophy/` | **new** | Why Ubiq exists and the beliefs behind it — see §5.1 |
| Concepts | `concepts/` | existing, extend | Projects, panes, panels, workspace sessions, conversations, accounts and profiles, harnesses, the glossary |
| Guides | `guides/` | **new** | End-to-end walkthroughs — §5.2 |
| How-to | `how-to/` | **new** | One task per page — §5.3 |
| Reference | `agents/`, `workbench/`, `git/`, `kb/`, `db/`, plus `tasks/`, `teams/`, `remote/`, `settings/` | existing + new folders | One page per area, bound by `context:` |
| FAQ | `faq/` | **new** | Grouped by topic, one file per group — §5.4 |
| Troubleshooting | `troubleshooting` | existing, extend | Symptom → cause → fix |

### 5.1 Philosophy pages

Sourced from `product/overview.md` and the root `AGENTS` domain rules, written for users:

- `philosophy/why-ubiq` — many agents, one window; tmux specialised for agent harnesses.
- `philosophy/real-terminals` — why an agent gets a real terminal and not a chat box, and why the
  chat panel exists anyway.
- `philosophy/any-harness` — Claude Code, Codex, Gemini CLI, opencode, Copilot CLI side by side;
  Ubiq does not pick your model.
- `philosophy/your-machine` — local-first; accounts hold references to credentials, never the
  credentials.
- `philosophy/what-ubiq-is-not` — the refusals from `product/overview.md`.

### 5.2 Guides

Each verified by doing it in a running build before it is written:

- Your first project and your first agent.
- Two agents on one task, side by side.
- Reviewing and committing what an agent changed (Git mode).
- Giving an agent knowledge: the knowledge base.
- Several accounts and profiles for several clients.
- Planning work on the Tasks board and handing it to agents.
- Working on a remote machine (drones).
- Querying a database with an agent beside you (DB mode).

### 5.3 How-to

Harvested from the reference pages' "What you can do here" lists, then from the FAQ: switch a
harness's permission mode; resume or fork a conversation; split and move panes; hide versus close a
tab; search the project; stage a file; sign in a connector (a GitHub or GitLab identity — not an
MCP server); change theme and interface size; find the logs; use the navigator. One page each,
named for the verb. Staging is per path, not per hunk (`features/workbench-git.md`).

### 5.4 FAQ

`faq/agents`, `faq/workbench`, `faq/accounts`, `faq/troubleshooting`. Seed questions
from `backlog.md` rows that describe user-visible limits (phrased as the user would meet them, never
by id), from `troubleshooting`, and from what the guides' authors got stuck on.

## 6. Tasks

Phases run in order; tasks inside a phase run in parallel, **at most three subagents at a time**.
"Model" is a suggestion: `haiku` for mechanical work, `sonnet` for writing, `opus` for the
judgement calls.

| # | Task | Reads | Writes | Model | Needs |
|---|---|---|---|---|---|
| **Phase 0 — ground truth** |||||
| T0.1 | Run the app (`just dev` from the Studio root), press F1, open every existing page, follow every link, note what is broken (`G300`) | `help.md` §6–7 | a findings list appended to this document | sonnet | — |
| T0.2 | Inline-icon spike (§4.1 "Verify first") | `help.md` §2.4, `ui/viewer/markdown.rs` | the decision, appended here | sonnet | — |
| T0.3 | Coverage table: every area in inventory §4 × existing page × context key × `missing`/`stub`/`done` | `inbox/ui-id-inventory.md` §4, `help/**/*.md` frontmatter, `help.md` §5 | `_docs/help/coverage` draft (held until T1.1) | haiku | — |
| **Phase 1 — the authoring guide** |||||
| T1.1 | Create `_docs/help/` and its plumbing (§3) | §3 here, `_meta/authoring.md`, `_tools/docs.py`, `INDEX.md` | the four files' frontmatter, `docs.py`, `INDEX.md`, `authoring`, `ubiq-docs` skill | sonnet | — |
| T1.2 | Write `structure` and `style` from §3 and §5 here, with one exemplar page of each type | §3, §5, `help.md` §2, `product/glossary.md` | `_docs/help/structure`, `_docs/help/style` | opus | T1.1 |
| T1.3 | Write `images`; add `icons.py export` + `just help-icons`; export the icons | §4, `_tools/icons.py`, `ubiq-icons` skill, `theme.rs` | `_docs/help/images`, `_tools/icons.py`, `Justfile`, `help/img/ui/*.png`, `tech/operations.md` row | sonnet | T0.2, T1.1 |
| T1.4 | Owner review of T1.2–T1.3 | — | — | human | T1.2, T1.3 |
| **Phase 2 — skeleton** |||||
| T2.1 | New folders with `planned` stub pages (`status: planned`, full frontmatter), `nav.order` updated, `context:` keys moved to their final pages | `structure`, `coverage` | `help/**`, `help/manifest.toml` | sonnet | T1.4 |
| **Phase 3 — content** (one subagent per section; each section is one commit) |||||
| T3.1 | Philosophy | `product/overview.md`, root `AGENTS` | `help/philosophy/*` | opus | T2.1 |
| T3.2 | Concepts, rewrite + extend | `product/glossary.md`, `features/panes-and-terminals.md`, `features/sessions-and-workspaces.md` | `help/concepts/*` | sonnet | T2.1 |
| T3.3 | Reference — agents and chat | `features/workbench-agents.md`, `features/chat.md`, `features/connectors.md` | `help/agents/*` | sonnet | T2.1 |
| T3.4 | Reference — workbench, IDE, sink, logs, notifications, stats | the matching `features/*.md` | `help/workbench/*` | sonnet | T2.1 |
| T3.5 | Reference — git, kb, db | `features/workbench-git.md`, `wip/kb.md`, `features/workbench-db.md` | `help/git/*`, `help/kb/*`, `help/db/*` | sonnet | T2.1 |
| T3.6 | Reference — tasks, teams, remote, settings | `features/workbench-tasks.md`, `features/workbench-teams.md`, `features/drone.md`, inventory §4.4 | new folders | sonnet | T2.1 |
| T3.7 | Guides, each done in a running build first | §5.2, the reference pages | `help/guides/*`, screenshots | sonnet | T3.2–T3.6 |
| T3.8 | How-to and FAQ | §5.3–5.4, the reference pages, `backlog.md` | `help/how-to/*`, `help/faq/*` | sonnet | T3.2–T3.6 |
| T3.9 | Start pages and troubleshooting, rewritten last | `inbox/onboarding-proposal.md`, everything above | `index`, `getting-started`, `troubleshooting` | opus | T3.7, T3.8 |
| **Phase 4 — consistency** |||||
| T4.1 | Review pass against `style`: terminology, voice, length, links, every page linked from one other | `_docs/help/*`, `help/**` | fixes, by section, back to subagents | opus | phase 3 |
| T4.2 | `coverage` all `done`; `just help-check` and `just docs-lint` green | — | — | haiku | T4.1 |
| **Optional, independent** |||||
| T5.1 | Fix `G322`: a `targets` field on `HelpCatalog` and a lookup beside `context_page` | `in-place-help` §2.5, `crates/ubiq-proto/src/help.rs` | code + `wip/help.md` | sonnet | — |
| T5.2 | Review the 34 element blurbs in `state/ui_id.rs` against `style` | `in-place-help`, `style` | `crates/ubiq/src/state/ui_id.rs` | sonnet | T1.4 |

## 7. Delegating — the prompt every subagent gets

Paste this ahead of the task's own row:

> You are writing part of Ubiq's user manual. Work only inside `ubiq/` (the base repository).
> Read first: `_docs/wip/help-content` (this programme — your task is row `<T#>`),
> `_docs/help/style` and `_docs/help/structure` (once they exist), and only the documents
> your row names. Facts come from `_docs/features/`; if a feature document and the running app
> disagree, say so in your report instead of guessing.
> Rules: use the Edit/Write tools for every edit — no `sed -i`, no `perl -pi`, no heredoc or
> Python rewriting a file. Never invent a control, a shortcut or a behaviour. Never reference code,
> Rust names or backlog ids in a help page. Run `just help-check` (and `just docs-lint` if you
> touched `_docs/`) from the Studio root before reporting. Do not commit; do not revert changes you
> did not make. Report: files written, `help-check` result, anything you could not verify.

The coordinator reviews each section, sends corrections back to the same subagent, then commits
it — one commit per section, message `help: <section>`.

## 8. Done when

- `_docs/help/` exists with its four documents, listed in `INDEX.md`, and `docs-lint` is green.
- Every area in `coverage` is `done`, bound to a context key or reachable by a rung-1
  `help_page()`.
- Philosophy, guides, how-to and FAQ sections exist and appear in `nav.order`.
- Reference pages show their buttons inline; guides carry screenshots.
- `just help-check` is green, and someone has read the manual end to end in a running build.
- This document is deleted; what it decided lives in `_docs/help/`, and its open questions are rows
  in `backlog.md`.

## 9. Open questions

- One icon tint for both themes, or a renderer change so an inline SVG takes the text colour? T0.2
  decides; the second is a code task.
- Does `_docs/help/coverage` stay after the programme, as the screenshot-rot index, or fold into
  `structure`? Leaning: it stays.
- Studio pages (`studio/` namespace, `help.md` §9) follow the same guide but are out of this scope.

## Related docs

- [`help.md`](./help.md) — the machinery this content runs on
- [`in-place-help`](./in-place-help.md) — element help and the `UiId` catalogue
- [`inbox/ui-id-inventory.md`](../inbox/ui-id-inventory.md) — the coverage checklist
- [`inbox/help-proposal.md`](../inbox/help-proposal.md) — why help is built this way
- [`_meta/authoring.md`](../_meta/authoring.md) — the documentation rules
