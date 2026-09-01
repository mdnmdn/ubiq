---
id: inbox-find
title: Proposal — find and replace in a file
kind: proposal
status: proposal
summary: The editor's find bar, which the component library already draws and Ubiq has never claimed — the four options it owes the user, why whole word and regex have to come from upstream rather than from a second find bar beside the first, and what replace means to a buffer whose file the host owns.
read_when: you are deciding how a file is searched, what the find bar's options mean, or what a replace does to a buffer the host will be asked to write
updated: 2026-09-01
depends_on: [feat-workbench, tech-ui, inbox-omni]
---

# Proposal — find and replace in a file

**Ubiq's editor already has find and replace, and nothing in Ubiq knows it.** `⌘F` opens a panel
over the buffer with a query field, a case toggle, a match counter, previous and next, and — under
`⌘⇧F` — a replacement field with *Replace* and *Replace All*. All of it belongs to the component
library, none of it was asked for, and no affordance anywhere in the window says it is there.

So this proposes something narrower and more useful than a find bar: **claiming the one that exists**.
Four options rather than one, an entry point the user can see, a query that survives a tab switch, a
replace that cannot make edits the user is unable to save, and a hand-off to
[`omni-search-proposal.md`](./omni-search-proposal.md) when one file is not the question.

## 1. Where it stands

**The find bar is live in the tree today.** `attach_file` builds each buffer with `EditorState::new`
(`crates/ubiq/src/app.rs:2088-2096`), and that constructor sets `searchable = true` itself — the flag
defaults to `false` for every other input and the code-editor mode overrides it. The `Search` and
`Replace` actions and their `cmd-f` / `cmd-shift-f` bindings are registered by
`gpui_component::init`, which `crates/ubiq-app/src/main.rs:70` calls at startup, in the `"Input"` key
context the component sets on its own rendered element. Nothing in `crates/ubiq/` had to opt in, and
nothing in `crates/ubiq/` can tell you it happened.

**What it does.** The matcher is a single-literal `aho-corasick` automaton rebuilt on every query
change, over the buffer's rope. It tracks every match, wraps forward and backward, reports `n/m`, and
replaces the current match or all of them. The panel draws itself as an overlay whenever a search
session is open, and the match highlights are painted by the component's own element.

**What it does not do.** There is **no whole-word matching** — every substring occurrence is a match
— and **no regex**. The case option is `ascii_case_insensitive`, so it folds `a`–`z` and nothing
else: a query for `straße` and a buffer holding `STRASSE` do not meet, and neither do `i` and `İ`.
Those are the three gaps between what the tree has and what this document asks for.

**Nothing in the interface mentions it.** The editor's tab strip has no find affordance, the
titlebar's search icon is `|_, _, _| {}` (`crates/ubiq/src/ui/titlebar.rs:73`), and the `⌘K` field
says *Search files, or run a command…* which is neither this nor implemented (`G16`). A keystroke
that only works if you already guessed it is not a feature the product has.

**Each buffer has its own search session, because each buffer is its own state.** `FileBody::Text`
holds one `EditorState` per open file (`crates/ubiq/src/state/editor.rs:60-79`), which is the right
shape and has the consequence that `⌘F` in a second tab starts from an empty query.

**A truncated read is already unsavable.** `FileBody::Text` carries `truncated`, and `savable()`
(`crates/ubiq/src/state/editor.rs:162-168`) refuses the save, because writing a prefix back over a
file is data loss. The find bar does not know that, so *Replace All* in such a buffer is a pile of
edits the user cannot commit.

## 2. What this decides

Not whether to build a find bar. Whether Ubiq **owns** the one it has:

- the option set — §3;
- who does the matching, and at what cost — §4;
- the parts that are Ubiq's regardless of that answer — §5;
- and what a replace means when the file belongs to the host — §6.

## 3. The four options

**One record, four options, shared with project search.** It is `Query` in a `search` module in
`crates/ubiq-proto/src/`, defined and argued in
[`omni-search-proposal.md`](./omni-search-proposal.md) §3, and this document is its second user.

| Option | Means | In the tree |
|---|---|---|
| literal / `.*` | The text is a substring, or a regular expression | Literal only |
| `Aa` | Case-sensitive | Present, ASCII-folding only |
| `\b` | The match is bounded by word boundaries | Absent |

**They compose in one order, and it is worth writing down**: the pattern is taken literally or as a
regex, word boundaries are wrapped around whatever that produced, and case sensitivity applies to the
whole. A literal `foo` with whole word on matches `foo(` and not `food`; a regex `fo+` with whole word
on matches `fooo` standing alone and not the `fooo` inside `xfooo`.

**The find bar's options and project search's options are the same options.** Not similar — the same
record, the same order of composition, the same meaning. A user who ticks two boxes in the editor and
then asks the same question of the project must not get a different answer because two engines
disagreed about what a word boundary is. That is the entire reason `Query` lives in `ubiq-proto`
rather than in the interface.

## 4. Where whole word and regex come from

The matcher is `aho-corasick` behind a private field, and the two missing options cannot be reached
from outside the component's crate. The match ranges are private and rebuilt only from that
automaton; the search panel is a private type constructed in one internal place, with no hook to swap
it; and the highlights are painted by an element that reads those private fields directly. There is
no seam. So there are exactly two routes, and they are not close.

**Route A — extend the component, upstream.** The change is two files: the matcher's query field
becomes a mode rather than an automaton, its build and its match loop branch on that mode, the session
grows two flags, and the panel grows two toggle buttons beside the case one it already has.
Everything downstream of match production consumes `Vec<Range<usize>>` and needs no change at all —
the painting is engine-agnostic. `regex` is already a dependency of the crate that holds the matcher,
so the change adds an import and no dependency.

**Route B — a second find bar, Ubiq's own.** The public surface is real: the buffer's rope, a
`RopeExt` trait with offset/line/column conversion and a word-range helper, the selected range as a
setter, and `TextDecoration` for painting arbitrary ranges. It is also not enough. There is **no
public scroll-to-offset** — the offset-based scroll is crate-private, so *next match* would mean
computing pixel offsets from a line height by hand — and **no public replace-of-an-arbitrary-range**:
a replace is select-then-replace-the-selection, one match at a time, with undo entries to match.
Decorations paint a text style, not the boxed match the component draws, so Ubiq's find bar would
look unlike the editor it sits in. And the component's own bar would still be there on `⌘F`, unless
Ubiq turned `searchable` off and paid for everything twice.

**Route A, and this is what the convention already says.** `../tech/ui-and-design.md` puts
gpui-component first, and a private matcher is exactly the case that rule exists for. The cost is
honest and worth naming: the dependency is a git rev, so the interim is a fork at a branch and a
pull request behind it, and the pin moves when it merges. That is a URL in a manifest, not a new
class of problem — the dependency is already pinned to a commit rather than a release.

**Route A fixes the case folding too**, which Route B would have had to fix separately. Once matching
goes through `regex`, case-insensitivity is Unicode case folding rather than the ASCII range, and the
`straße` case stops being a bug nobody filed.

## 5. What Ubiq owns either way

Four things, none of which the component can decide.

**The query survives a tab switch.** Each buffer has its own session, so `⌘F` in a new tab opens
empty by default. The last query and its four options belong to the window, and opening the find bar
seeds the new buffer's session with them — the same shape as the pending-attach queue the editor
already uses, and for the same reason: setting a value needs a window and a keystroke does not always
have one. Whether the *replacement* text carries too is a real question, and the answer is no: a
replacement carried into the wrong file is how a user destroys something quietly.

**The entry point is visible.** A find affordance in the editor's tab strip, and the titlebar's dead
search icon given the one meaning it can have — which is project search, not this (project search's
§9). Ubiq owns the discoverability of a keystroke the component only owns the behaviour of.

**Replace moves off `⌘⇧F`.** The component binds it to replace-in-file in the key context a focused
editor sits in, and project search wants it for the reason every editor a user arrives from gives it:
that is what it means. So replace-in-file becomes `⌘⌥F` and `⌘⇧F` goes to the dock's search tab.
Ubiq owns which keystroke means what, even where the behaviour behind it is the component's — and a
shortcut that means two things depending on where the caret is belongs to nobody.

**Selection seeds the query.** `⌘F` with a selection searches for the selection. This is the one
convention every editor shares and the one thing users notice missing.

**The hand-off.** *Search in project* on the find bar sends the query and its options to the dock's
search tab; a result group there offers *find in this file*. One shared record makes both a single
line, and it is the reason the two proposals are written as halves of one thing.

## 6. Replace, and the file underneath

**A replace edits the buffer. It does not write the file.** The buffer is the interface's; the file is
the host's, reached only by `WriteProjectFile` with the version the read came with
(`crates/ubiq-proto/src/messages.rs:179-184`). So *Replace All* makes the tab dirty, exactly as typing
does, and nothing reaches the disk until a save. This falls out of the architecture rather than being
designed: the interface has no path to the file, which is what makes replace safe by construction.

**Three consequences, and one of them is a change to the tree.**

**A truncated buffer must refuse replace, not just refuse the save.** `savable()` already blocks the
write, so today a user can *Replace All* in a prefix of a large file and discover at `⌘S` that none of
it can be kept. The component has a public `replaceable` flag on the state and gates its own replace
UI on it, so a truncated read is built with replace off and the panel simply does not offer it. Find
still works — reading a prefix is honest, writing one is not.

**A `Binary` or `Failed` body has no find bar**, because it has no buffer. Nothing to decide; worth
stating so nobody adds a toolbar button that renders on a body that cannot hold one.

**A save after a replace can be refused, and that is correct.** `expected` is optimistic
concurrency: if an agent in a pane rewrote the file while the user was replacing in it, the write
fails and the interface says so rather than winning the race. This is the one place a user is likely
to meet that refusal, because *Replace All* is the edit most likely to follow a long pause. Two rows
in the tree stand between here and that experience being decent: the keyboard path to save does not
work at all — `⌘S` is bound in a key context no element declares and the handler is never registered
(`G51`) — and a refused write's recovery is not designed.

**Replace across files is not here.** It needs a write per file with a version each, a preview, and a
story for the file that changed underneath; it is a backlog row on the project-search proposal,
which is shaped to accept it.

## 7. Failure

| When | What happens |
|---|---|
| The query is an invalid regex | The field marks itself; the match count reads none; nothing is highlighted |
| The query matches nothing | The counter says so; previous and next do nothing |
| The buffer is a truncated read | Find works, replace is not offered |
| The body is binary or failed | No find bar |
| The buffer is reloaded under an open search | The session's query stands and the matches are recomputed |
| The tab is closed with the bar open | The session goes with the buffer, and the window's remembered query stands |
| A replace-all is saved and refused on version | The buffer keeps the edits, the tab stays dirty, and the refusal is the save's to report |

## 8. Rules this adds

**Find in file runs in the interface, over a buffer the interface already holds.** It crosses no bus,
reads no path and asks the host nothing. It is the one search that is not the host's, and the reason
is that its subject is already in memory.

**The four options are one record, shared with project search, and neither side may grow a fifth
alone.**

**A buffer that cannot be saved cannot be replaced in.** Whatever makes a save dishonest makes a bulk
edit dishonest first.

**A component behaviour Ubiq relies on is a behaviour Ubiq makes visible.** An undiscoverable feature
inherited from a dependency is not a feature the product has — and it is not documented by the
dependency's changelog either.

## 9. Phases

1. **Claim what exists.** The find affordance in the tab strip, selection-seeds-query, the window's
   remembered query seeded into a new buffer's session, and replace turned off for a truncated read.
   No dependency change, and it is most of the user-visible gap.
2. **The shared query.** `Query` in `ubiq-proto` — landing with project search's phase 1 — and the
   find bar's state expressed in it rather than in the component's two flags.
3. **Whole word and regex, upstream.** The matcher's mode, the session's flags, the panel's two
   toggles, the pull request, and the fork pin until it merges. Unicode case folding arrives with it,
   and so does the `⌘⇧F` rebinding, which is a binding the component registers and is cleanest
   changed where it is registered.
4. **The hand-off.** *Search in project* from the bar and *find in this file* from a result group.
   Waits on project search's dock tab.

Phase 1 stands alone and is worth doing whatever happens to the rest. Phase 3 is the only one with a
dependency on somebody else's repository, which is why it is not first.

## 10. What this asks to be decided

Six decision rows:

- The editor's find and replace is the component library's, and Ubiq's job is the option set, the
  entry points and the guards — not a second implementation beside it.
- Whole word and regex arrive by extending the component upstream, with a fork pin in the interim,
  rather than by building a parallel find bar over the public decoration and selection APIs.
- The four options are one record in `ubiq-proto`, shared with project search, composed in one stated
  order.
- The last query and its options belong to the window and are carried into each buffer's session; the
  replacement text is not.
- A buffer whose read was truncated is built with replace disabled, so the interface cannot invite an
  edit it will refuse to save.
- Find in file crosses no bus, and replace reaches the disk only through the save path that already
  exists, with the version the read came with.

Backlog rows this leaves open: whether a Ubiq-side binding registered after
`gpui_component::init` overrides one of the component's own, which decides whether the `⌘⇧F` move
can happen before phase 3 and which nothing here has checked against GPUI's precedence rules; the
case folding, which stays ASCII-only until phase 3 lands; a
refused write's recovery, which `⌘S` needs before *Replace All* makes it likely; the keyboard path to
save, which is `G51`; find within a selection, and preserve-case replace, both deliberately absent
from the option set; a match count that is exact in a fifty-thousand-line buffer, which the
component's rebuild-on-every-keystroke matcher decides and not Ubiq; and whether the find bar should
appear in a viewer that is not an editor at all, which waits on
[`file-viewers-proposal.md`](./file-viewers-proposal.md).

## Related docs

- [`omni-search-proposal.md`](./omni-search-proposal.md) — the other half, and where `Query` is defined
- [`file-viewers-proposal.md`](./file-viewers-proposal.md) — the per-file panels and viewers a find bar would have to live in
- [`../features/workbench.md`](../features/workbench.md) — the editor, the tab strip and the save path
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the gpui-component-first convention §4 leans on
