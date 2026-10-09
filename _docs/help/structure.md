---
id: help-structure
title: Help structure
kind: help
status: draft
summary: The information architecture of the user manual — its sections, the five page types and their skeletons, nav order, when a page splits, how context keys are allotted and what the index page does.
read_when: you are adding, moving or splitting a help page, or choosing its type, section or context key
updated: 2026-10-09
verified: 2026-10-09
code_anchors: [help/manifest.toml, _tools/helpbundle.py]
depends_on: [wip-help, wip-help-content]
---

# Help structure

This document owns the information architecture of the manual: the sections and what belongs in each, the five page types (concept, reference, guide, how-to, FAQ) and their skeletons, `nav.order`, when a page splits, how `context:` keys are allotted, and the job of the `index` page.

`_docs/help/` describes how the manual is written; `wip/help.md` keeps how the machinery works; `help/` is the manual. `features/` owns every behavioural fact, and a help page restates it in user terms without contradicting it.

## Contents

Written by task T1.2 of [`wip/help-content.md`](../wip/help-content.md).

## Related docs

- [`wip/help.md`](../wip/help.md) — how the help machinery works
- [`wip/help-content.md`](../wip/help-content.md) — the programme that writes the manual
