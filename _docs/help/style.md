---
id: help-style
title: Help style
kind: help
status: draft
summary: Voice, tone, terminology, formatting conventions, length limits and the list of what never appears in a help page.
read_when: you are writing or reviewing the words of a help page, or the one-sentence help of an element
updated: 2026-10-09
verified: 2026-10-09
code_anchors: [help/manifest.toml, _tools/helpbundle.py]
depends_on: [wip-help, wip-help-content, prod-glossary]
---

# Help style

This document owns the voice and tone of the manual, the terminology list, the formatting conventions for controls, keys and headings, and what never appears in a page.

`features/` owns every behavioural fact; where a help page and a feature document disagree, the feature document wins and the help page is fixed.

Which page a thing goes on, and its skeleton, belong to [`structure`](./structure.md); icons and
screenshots to [`images`](./images.md). These rules hold for every page and for the one-sentence
help an element shows under ⇧F1.

## 1. Voice and tone

The manual talks to one person who is using Ubiq while reading it. It is plain, calm and exact.

| Rule | Write | Not |
|---|---|---|
| Second person, present tense, active voice | Press **F1** to open the page for the screen you are on. | The user may press F1, which will open the relevant page. |
| Lead with the outcome, then the steps | To keep an agent running out of sight, right-click its tab and pick **Hide**. | Right-click the tab. A menu appears. Pick Hide. The agent keeps running. |
| One idea per paragraph | Two short paragraphs: what the panel shows, then what you do with it. | One paragraph that explains the panel, a setting and a shortcut. |
| No marketing, no reassurance | The Changes panel lists every path that differs from HEAD. | The powerful Changes panel makes reviewing a breeze! |
| No "simply", "just", "easy", "obviously", no exclamation marks | Press **+** on the row. | Just press + and you're done! |
| State, do not hedge | **Close** ends the harness. | **Close** should usually end the harness. |
| Describe the shipped build | (a missing feature is absent from the page) | This arrives in a later version. |
| Say why when a choice is surprising, once | A chat tab hides rather than closes, because the conversation outlives the tab. | A chat tab hides rather than closes. (and the reader closes it three times) |
| Name the user's goal, not the machinery | The harness redraws at the new size. | Ubiq sends the new geometry to the pseudo-terminal. |

**The interface is "Ubiq", never "we" or "the app".** A sentence whose subject is Ubiq says what
Ubiq does — "Ubiq keeps the arrangement when you come back" — not what it tries or wants.

**A failure is written as the way out, never as blame.** "Open a project first" — not "you forgot
to open a project".

## 2. Terminology

Use these words, with these meanings, and no others. The words in the third column belong in a
page's `keywords:` when a user is likely to type them, never in its prose.

| Write | Meaning | Not |
|---|---|---|
| **harness** | the agent program itself: Claude Code, Codex, Gemini CLI, opencode, GitHub Copilot CLI | CLI, tool, bot, model |
| **agent** | one harness running in a project, doing work for you | workspace, instance, process, AI |
| **pane** | the terminal panel an agent or a shell runs in | window, console, view |
| **panel** | anything in the window with a tab that you can move, split or close | widget, view, pane (for a panel that is not a terminal) |
| **tab** | the handle of a panel | page |
| **window** | the operating-system window Ubiq draws in | screen, app |
| **region** | the centre, left, right or bottom of the window | sidebar, drawer, area |
| **rail** · **rail mode** | the column of destinations on the left · one of them, by its label: **IDE**, **Git**, **Agents**, **Tasks** | activity bar, tab, view |
| **project** | a folder Ubiq has opened, and everything it remembers about it | repo (unless it is about git), workspace, folder |
| **workspace session** | Ubiq's named grouping of agents with a home folder | session, workspace |
| **conversation** | one harness's exchange with you that can be resumed or forked | session, thread, chat history |
| **account** | a named identity a harness runs under; it points at credentials and never holds them | login, credentials, user |
| **profile** | a named base configuration a run starts from | preset, template |
| **permission mode** | how much a harness may do without asking, by the harness's own label | permission level, safety mode |
| **connector** | a signed-in identity at a code host or workspace provider — GitHub, GitLab, Atlassian | integration, MCP server |
| **MCP server** | a tool provider a harness can call | connector, plugin |
| **skill** | packaged instructions given to a harness for a run | prompt, plugin |
| **subagent** | an agent another agent started | child agent, worker |
| **focus** | which panel receives your keystrokes; exactly one has it | active, selected |
| **drone** · **remote host** | Ubiq's helper on another machine · that machine, as Ubiq connects to it | server, agent |
| **help panel** | this manual, inside the window | docs, help window |

**Session is never written alone.** In Ubiq it names two things: Ubiq's grouping of agents, and the
resumable conversation a harness keeps. Write *workspace session* for the first and *conversation*
for the second, every time, even when the context seems to settle it.

**Label first, then the word.** Where the interface labels a control, the page uses its label
exactly — **Hide**, **Close**, **Settings** — even when the terminology list has a general word for
what it does.

## 3. Formatting

| What | Convention | Example |
|---|---|---|
| A control's label | bold, spelt and capitalised exactly as on screen | **Unstaged**, **New terminal** |
| A control with no label | its tooltip in bold, then its icon inline as [`images`](./images.md) says | the **Settings** button |
| A path through menus or settings | the labels in bold, joined by "then" | open **Settings**, then **Editor** |
| A key or chord | bold; macOS first, then Windows and Linux, joined by " / " only when they differ; modifiers in the platform's own order and glyphs | **⌘K** / **Ctrl+K** · **F1** · **⇧F1** |
| Named keys | the word, capitalised | **Enter**, **Esc**, **Tab**, **Space** |
| What the user types, a command, a file name the user sees | inline code | type `exit` |
| Headings | from `##`, never `#` — the panel draws the title; `###` only inside a long reference page; sentence case; no numbering | `## When something goes wrong` |
| Heading words | named to last, because every heading is a link target and renaming one breaks links | "Staging a path", not "New: staging" |
| Lists | numbered for steps in order, bulleted for choices and links, prose for anything with a "because" | — |
| Tables | for parts, settings and comparisons of three or more rows; one sentence per cell | — |
| Emphasis | italics for a term at its definition, nowhere else | a *pane* is… |

**Links.** Link another page by its id, with the id as the link target and the page's name as the
text; a heading in it by `<id>#<slug>`. Never link by path, which breaks when the file moves. `ubiq://./<view>` opens a place in
the window and is used where doing beats reading about it; it does nothing with no project open, so
the sentence around it never depends on it. External links go only to a harness's or a provider's
own documentation. Link the first mention of a term on a page, not every one.

**Placeholders** are in angle brackets inside inline code — `<project>` — and say what goes there.

## 4. Length

The help panel is narrow; a page that does not fit is not read. Lines count the body, frontmatter
excluded.

| Type | Limit | Over it |
|---|---|---|
| Concept | 60 lines | split into two concepts, or move the reason to the philosophy section |
| Reference | 80 lines — two screens of the help panel | split per [`structure`](./structure.md) §4 |
| Guide | 120 lines, at most ten steps | split into part one and part two, each its own guide |
| How-to | 25 lines, at most seven steps | it is a guide |
| FAQ entry | two to six sentences | the answer belongs on a page; the entry links to it |
| Paragraph | five lines | split it |
| `summary` | one sentence, at most 140 characters, saying what the page answers | — |
| Element sentence | one sentence, at most two | the element gets a row in its area's "The parts" |

## 5. What never appears

A user cannot act on any of these, and each one rots the moment the code moves.

- **Code paths and file names of Ubiq itself** — no `crates/…`, no source file.
- **Rust names** — no types, functions, enum variants, panel kind names or setting keys as the code
  spells them. Write the setting's label.
- **Message names and anything about the wire** — the host, the coordinator, the bus, a
  pseudo-terminal, unless a page on remote hosts needs the idea, and then in the user's words.
- **Backlog, decision or task ids** (`Gnn`, `Dnn`, `Tn.n`), and links into `_docs/`.
- **Promises** — no "coming soon", "planned", "in a later version", and no description of a
  feature the shipped build does not have. A known limit is stated as it is met: "Search results
  are not ranked", not as a plan to fix it.
- **Dates and version history** — no "new in", no "changed in", no account of how a screen
looked before. The `since` field carries
  the version a page describes.
- **Personal data** — no real account names, tokens, home folder paths or machine names, in text
  or in screenshots.
- **Internal names for things the user sees differently** — the user sees **Agents**, not a mode
  id; a tab, not a panel kind.

## 6. Reviewing a page

A page is ready when each of these holds; the review pass reads the whole manual against them.

| Check | Against |
|---|---|
| Every fact matches its `features/` document and the running build | the source column in [`structure`](./structure.md) §1 |
| Every control named exists, under that label | the running build |
| Every word in §2's "Not" column is absent from the prose | §2 |
| No line of §5 is broken | §5 |
| The page fits its length | §4 |
| It links to at least one other page, and one page links to it | `just help-check` |

## Related docs

- [`wip/help.md`](../wip/help.md) — how the help machinery works
- [`wip/help-content.md`](../wip/help-content.md) — the programme that writes the manual
- [`product/glossary.md`](../product/glossary.md) — the terms the terminology list is drawn from
