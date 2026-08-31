---
id: inbox-session-naming
title: Proposal — what to call a working area
kind: proposal
status: proposal
summary: The word "session" is spent twice and describes neither thing well — the tests a replacement has to pass, the candidates against them, the cascade that frees "workspace", the standalone fallback "lane", and the blast radius of doing it.
read_when: you are naming the level between a project and an agent, or deciding whether to take the rename
updated: 2026-08-31
depends_on: [feat-sessions, prod-glossary, tech-transport]
---

# Proposal — what to call a working area

One narrow question, split out of [`agent-graph-proposal.md`](./agent-graph-proposal.md) because it
is decidable on its own and because it gets more expensive every week it is not: what is the level
between a project and an agent called?

## 1. What is wrong with "session"

Three things, and the first is already written down as a rule.

**It is spent twice.** `AGENTS.md` carries a domain rule whose entire content is that the word means
two things — Ubiq's named grouping of panes, and the harness library's resumable conversation — and
that a document using it must say which. [`../backlog.md`](../backlog.md) Q4 exists because of the
same collision. A vocabulary that needs a disambiguation rule has already failed; the rule is a
patch over a naming bug.

**It implies transience.** A session is the thing that ends when you log out. This thing is durable,
named, colour-coded, remembered across restarts, and has a goal — the opposite end of the spectrum
from a login session. Users read the word and expect it to be cheap and disposable, which is exactly
the wrong instinct about a worktree with a branch and a week of work in it.

**It describes neither of its two useful properties.** The thing has a folder and it has a goal.
"Session" says nothing about either, so every sentence that uses it has to add both.

Note what is *not* wrong with it: tmux calls its top-level grouping a session, and Ubiq is tmux for
harnesses, so a tmux user reads it correctly on the first try. That is the one real argument for
keeping the word, and it is worth putting on the scale.

## 2. The tests

A candidate has to pass all three. Most do not.

**1 — The collision test.** Is the word already spent by git (`branch`, `worktree`, `tree`,
`stash`), by Rust and the operating system (`thread`, `stream`, `process`, `task`, `channel`,
`context`), by the LLM domain (`context`, `session`, `agent`, `run`, `prompt`), by `agent-manager`
(`session`, `run`, `profile`, `account`, `catalog`), or by Ubiq's own interface (`project`, `pane`,
`workspace`, `workbench`, `rail`, `activity`, `dock`, `panel`, `tab`)?

**2 — The sentence test.** All three have to read naturally, because all three will be said daily:

> "a ___ per feature" · "which ___ is that agent in?" · "close the ___, and its agents keep running"

**3 — The header test.** Does the plural work as a column header, a rail label and a menu group,
with no explanation attached?

And an unwritten fourth: it should be a word the user already owns. Nothing here is worth teaching.

## 3. The candidates

| Term | Reads as | 1 | 2 | 3 | Verdict |
|---|---|:-:|:-:|:-:|---|
| **workspace** | the area you work in | ✗ | ✓ | ✓ | The best fit by meaning, and blocked only by Ubiq's own use of it. §4 unblocks it |
| **lane** | one parallel track of work | ✓ | ✓ | ✓ | The best fit that needs nothing else to move |
| **bay** | a work bay; one job at a time | ✓ | ✓ | ~ | Same virtues as lane, more physical, less familiar as software vocabulary |
| **track** | a line of work | ~ | ✓ | ✓ | Fine, except "tracking" is already overloaded by version control and by status |
| **effort** | a body of work with a goal | ✓ | ✓ | ~ | Accurate and abstract. Says nothing about a folder, which is half the point |
| **desk** | where your current work sits | ✓ | ~ | ~ | Charming; implies one person, and this thing holds a crew |
| **room** | where the agents are | ✓ | ~ | ✓ | Good for the crew, silent about the folder and the branch |
| **site** | where building happens | ~ | ✓ | ~ | "Site" is a website to most readers, and that reading is hard to shake |
| **mission**, **campaign**, **front** | a goal and a team | ✓ | ~ | ✗ | Overblown for "a folder with a branch". Ubiq is not a war room |
| **branch**, **worktree**, **tree** | git's own words | ✗ | — | — | Vetoed. An area is not always either, and git owns them outright |
| **thread**, **stream**, **process** | | ✗ | — | — | Vetoed. All three are spent in a Rust codebase full of all three |
| **run**, **job** | one execution | ✗ | — | — | Vetoed. `agent-manager` calls a launched harness a run |
| **activity** | | ✗ | — | — | Vetoed. The rail is the activity rail |
| **context**, **space** | | ✗ | — | — | Vetoed by the LLM sense, and by meaning nothing in particular |
| **session** | keep it | ✗ | ✓ | ✓ | Fails test 1 against the library. Passes everything else, and tmux says it |

## 4. The recommendation is a cascade, not a swap

`workspace` is the word a user reaches for unprompted — VS Code has taught a generation that a
workspace is a folder you work in — and the only thing blocking it is that Ubiq spends it on
something else: **one running agent**.

That use is already on its way out. [`agent-graph-proposal.md`](./agent-graph-proposal.md) says an
agent and a workspace are one record, and that document says *agent* wherever identity is meant. The
glossary's own definition — "one running instance of one agent inside a session" — is a definition
of an agent with extra words. So:

| Today | After |
|---|---|
| session — a named grouping with a folder | **workspace** |
| workspace — one running agent | **agent** |

Two renames, one sweep, and every sentence gets shorter. "Which workspace is that agent in?" is the
question users are already asking in those words.

**The argument for doing it now is the argument project-handling makes about ids.** That proposal
already plans a sweep that touches every id site in the contract — `Uuid` to `Ulid` behind newtypes
— and a rename that rides along with it costs the diff it is already paying for. Done separately,
later, it is a second sweep over the same files, and by then there are persisted TOML keys, a
message log, and possibly a socket protocol with a version on it.

**If the cascade is too much, take `lane`.** It is the only candidate that needs nothing else to
move: `session` becomes `lane`, `workspace` stays where it is, and the collision with the library
disappears the same day. It is short, it collides with nothing, it pluralises cleanly in a header,
and "a lane per feature" is how people already describe the thing.

The ranking, then: **workspace** if the cascade is taken, **lane** if it is not, **bay** if `lane`
reads too much like a racing metaphor for the room.

## 5. What it costs

Honest blast radius, so the decision is made with it in view.

**The contract.** `ListSessions`, `CreateSession`, `AttachToSession`, `DetachFromSession`,
`SessionList`, `SessionCreated`, `SessionAttached`, `SpawnWorkspace`, `WorkspaceSpawned`,
`SessionInfo`, `WorkspaceInfo`, and `session_id` on almost every payload. Most of these are
documented and unimplemented — [`../backlog.md`](../backlog.md) G19 says only three of the family
exist in code — so the rename is largely a documentation edit today and will not be next year.

**The code.** The orchestrator's session table, the agent registry, `state/`, and every test that
names one.

**The documents.** [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md)
is a *filename* with `id: feat-sessions` in it, and ids never change once other documents cite them.
That makes this a librarian's move rather than a contributor's — the one part of this that cannot be
done by the person doing the rename, under `_meta/authoring.md`'s rules.

**Nothing crosses into `agent-manager`.** The library keeps calling its resumable conversation a
session, because that is the harness's own word and the crate is not ours to rename. The whole
collision is resolved on Ubiq's side, which is the cheapest place it could possibly be resolved.

**What does not get renamed.** `task` stays. It collides with `tokio::task` and with the tool a
harness spawns subagents through, but it is the domain's actual word, the rail mode is already
called Tasks, and no user-facing sentence about a task is ambiguous. A collision that never confuses
a reader is not worth a sweep.

## 6. What this asks to be decided

- The word `session` leaves Ubiq's vocabulary. Which replacement wins is the second question; that
  it goes is the first.
- Preferred: the cascade — `session` → `workspace`, `workspace` → `agent` — taken in the same sweep
  as the contract's id change, because that sweep is already touching every site.
- Fallback if the cascade is refused: `lane`, which needs nothing else to move.
- `agent-manager` keeps `session` for the resumable conversation, and the rule in `AGENTS.md` that
  exists to disambiguate the two is deleted rather than reworded.
- The document rename that follows is a librarian pass, filed to `_meta/feedback.md`, not something
  the renaming change makes on its own.

## Related docs

- [`agent-graph-proposal.md`](./agent-graph-proposal.md) — the model this word sits in the middle of
- [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — the document that would be renamed
- [`../product/glossary.md`](../product/glossary.md) — where both definitions live today
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the message names this touches
- [`project-handling-proposal.md`](./project-handling-proposal.md) — the id sweep this would ride along with
- [`../backlog.md`](../backlog.md) — Q4, the collision, and G19, why the contract rename is cheap today
