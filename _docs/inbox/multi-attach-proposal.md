---
id: inbox-multi-attach
title: Proposal — many interfaces on one host
kind: proposal
status: proposal
summary: What a host owes several attached interfaces at once — the three kinds of resource it holds, the write lease that a pane needs and a conversation does not, the one geometry a pseudo-terminal has, where a joining viewer's first frame comes from when nothing is kept, and the line between one person with two screens and two people with one agent, which is where a paid tier would sit if there is one.
read_when: you are deciding what happens when two interfaces attach to one host, who may type into a shared pane, whether a conversation can have two readers, or whether collaboration is a tier
updated: 2026-09-12
depends_on: [tech-architecture, tech-transport, tech-decisions, feat-panes, feat-chat, prod-overview, inbox-tui, inbox-chat-resume]
---

# Proposal — many interfaces on one host

[The terminal interface](./ratatui-tui-proposal.md) puts a second client on a host and stops at the
first hard edge: a pane belongs to whoever spawned it, so a terminal attached to the window's host
sees the window's projects and not the window's shells. That edge is not about terminals.

**Everything the host streams, it streams to one client, and keeps none of it.** A pseudo-terminal's
bytes go to the owner's mailbox and the host holds no scrollback. A conversation's deltas go to the
owner's mailbox and the host holds no transcript — it counts a sequence number and forgets the
content. Everything the host *owns* rather than streams — the catalogue, the agent roster, the
readings — already goes to everyone.

So two interfaces on one host is not a transport question; the transport was built for it. It is
three questions about every resource: **who may write it, who may watch it, and where does a joiner's
first frame come from.** This proposes the answers, and separates the half that is one person with
two screens from the half that is two people with one agent — because only the second is a product
change, and only the second could be a tier.

## 1. What the host does today

**Facts it owns, sent to everyone.** The project catalogue broadcasts, and so does `AgentChanged`:
the roster of agents in a project reaches every attached client with no ownership at all. Nothing has
to change here, and it is the proof that fan-out is already a thing the coordinator does.

**Questions it answers, per asker.** File reads, directory listings, search, host browse: a request
carries its client and the answer goes back to it. Two interfaces asking the same question get two
answers and never collide. Nothing has to change here either.

**Streams it addresses to one owner.** `crates/ubiq-host/src/pty/mod.rs` resolves a pane's mailbox
once, at spawn, and its reader thread stops when that mailbox is gone. The coordinator keys `owners`
by pane and `conversation_owners` by agent, and a conversation's updates go to the owner it finds
there. Both are single-owner by construction, and neither keeps
history: bytes are opaque by `D6`, and the conversation pump in
`crates/ubiq-host/src/conversation.rs` carries a monotonic sequence while the content passes
straight through.

**And one arbiter already exists.** `D37` makes a save name the version it read and refuses it if the
file moved. That is the shape every rule below takes: the host decides, and a client is told no —
never two clients quietly overwriting each other.

## 2. Three kinds of resource

| Kind | What it holds | The rule | What it costs |
|---|---|---|---|
| **Broadcast** | The catalogue, the agent roster, host readings, notifications | Everyone sees it. No ownership, no lease | Nothing. It works today |
| **Per-client** | File reads and writes, trees, search, host browse, focus, layout | Each client asks and is answered; a write is arbitrated by the version it names | Nothing. `D37` is the arbiter and it is built |
| **Streamed** | A pane's bytes, a conversation's deltas | Many may watch; exactly one may write; a joiner needs a first frame | All of the work below |

The taxonomy is the proposal's spine: two of the three kinds need no design at all, and stating that
is what stops "multi-attach" from being read as a rewrite. The third kind is two resources, and they
are less alike than they look.

## 3. The lease

**A pseudo-terminal has one input stream, and two writers interleave into garbage.** The mixing
happens in the kernel, below anything the contract could arbitrate, so no protocol fixes it after the
fact. A pane therefore has a **write lease**: one client holds it, every attached client can see who,
it can be handed over or taken when the holder has gone, and it is released on disconnect.

**A conversation is not like that.** Turns serialise inside the harness, and the composer already
queues a prompt sent while a turn runs — so two clients prompting one agent is coherent, and the
transcript stays a single ordered thing. What needs arbitration there is the *ask*: a permission
prompt has one answer, and the second client to answer must be told the question is gone rather than
answering a second time. So the rule is **one answer per outstanding ask, broadcast to everyone
watching**, which is also what clears the "needs you" strip on every screen at once.

**The lease is not a permission.** For one person with two interfaces it exists so that a stray
keystroke in the window they are not looking at does not land in a shell — a safety catch, not an
authorisation. That distinction is what §7 turns on.

## 4. One pane, one geometry

A pseudo-terminal has one window size, and it is the kernel's, not a view's. Two viewers on different
terminals cannot both be right, and reflowing per viewer would need a terminal state engine in the
host, which `D6` forbids and `D1` refuses to write.

Three answers exist. The smallest attached client wins, which is what a multiplexer usually does and
which lets a phone shrink someone's editor. Every viewer gets its own reflow, which is the one that
is not available. Or **the lease holder's geometry is the pane's, and every other viewer letterboxes**
— the person typing keeps their screen, and a watcher sees a smaller rectangle inside their own.

Take the third. And the rule underneath does not bend: a resize is incomplete until the harness knows,
so a lease hand-over that changes the geometry is a resize like any other.

## 5. The joiner's first frame

Nothing is kept, so a client that attaches to a live pane sees a blank rectangle until the next byte
arrives, and one that attaches to a live conversation sees an empty transcript until the next delta.
This is the expensive part, and the two resources want different answers.

**A pane: a bounded ring, and a nudge.** The host can keep the last few tens of kilobytes per pane —
*storing is not parsing*, so `D6` survives intact, and the cost is memory and one honest privacy note:
the ring is process memory and never reaches disk. A ring alone is not a screen, because it may begin
mid-escape-sequence and because a full-screen harness paints by absolute address; so the joiner also
gets a repaint nudge — the resize round-trip every full-screen application answers by redrawing.
Together they cover the case Ubiq actually has, which is a harness on the alternate screen. They do
not cover a shell halfway down a scroll, and pretending otherwise would be the wrong promise.

**A conversation: the argument has to be had, not assumed.**
[`chat-resume-and-fork`](./chat-resume-and-fork.md) argues that Ubiq should not own a transcript
record, because the harnesses replay their own on resume. That argument is about durability across a
restart, and a live in-process delta buffer for the seconds a second interface takes to join is a
different object — but it is close enough that building one without answering that document would be
two designs for one fact. The honest first cut is **the joiner sees what happens next**: a transcript
that starts when it attaches, a line saying so, and no store at all. It is ugly, it is cheap, and it
is enough to find out whether anyone wants the feature before paying for the machinery.

## 6. Leaving is not killing

`client_gone` reaps everything a client owned. With one interface that is correct. With several it is
a bug waiting: a viewer closing its terminal must drop its lease and its watches, and nothing else.

The reader thread's rule — it stops when nobody is listening — becomes *it stops when the last viewer
has gone*, and even then `D103` already says a pane outlives its viewers until something kills it.
This is a defect that multi-attach exposes rather than creates, it is small, and it is worth fixing
whether or not the rest of this proposal is accepted.

## 7. One person, or several

Everything above works with no notion of who is attached. That is deliberate, and it is the line the
rest of the decision sits on.

**One person, several interfaces.** A window on the desk, a terminal over SSH, both the same person's.
There is no identity to model — every client is the same human — and the lease is a safety catch. This
is what the terminal interface proposal already needs, it costs the sections above and nothing more,
and it is inside the product's own boundary: one person, one machine.

**Several people, one host.** Two developers watching one agent work, a pane handed from one to the
other, a colleague looking at a run. The mechanism is identical — the same lease, the same fan-out,
the same first-frame problem — and everything that is *added* is about people: a named identity per
client instead of one bearer token that grants everything, a roster showing who is attached, per
resource authorisation rather than a first-come lease, and an audit of who typed what into whose
shell. `product/overview.md` refuses multi-user operation in as many words, so this is a scope change
taken on purpose or not at all.

The engineering consequence is the useful one: **the difference between them is a policy, not a
protocol.** One contract, one message set, one codebase; a policy object answers "may this client take
this lease", and its default answer is "yes, there is only one person here".

## 8. Whether it is a tier

Worth separating three things that get argued as one.

**What a team would pay for is not the lease.** It is the rendezvous — two machines finding each other
without a port forward, a VPN or a shared token pasted into a chat — plus identity, audit retention
and somebody to call. The lease is a Tuesday's work once the taxonomy is written down; the service is
the product.

**The licence already carries the posture.** This tree is under the Sustainable Use License, which
permits internal business use, so a team running several interfaces against one host is inside the
licence as it stands. A tier therefore cannot be a flag in this build. It is either a separately
licensed module that supplies the policy object — the shape the licence this project adopted was
designed around — or it is the service on the other end of the rendezvous. Both keep one protocol.

**And the sequencing writes itself.** Build the lease because one person with two interfaces needs it.
Keep the policy behind a seam from the first commit, so a tier stays possible without a fork. Decide
the tier when somebody asks for the second *person*, not the second interface — the two are months
apart, and only the second one changes what Ubiq is.

One thing here is not an engineering call: crossing the multi-user refusal in `product/overview.md`
is a product decision, and that document prescribes.

## 9. Order

| # | Ships | Why now |
|---|---|---|
| 0 | Leaving drops a lease and a watch, never a pane | A defect on its own terms, small, and every phase below assumes it |
| 1 | The taxonomy written down, and the write lease on panes | The single-person case, which the terminal interface needs to be usable |
| 2 | The ring and the repaint nudge | A joining viewer sees a screen rather than a blank rectangle |
| 3 | Conversations: many prompters, one answer per ask, and the transcript question settled with `inbox-chat-resume` rather than around it | The second interface becomes a place to work rather than a place to watch |
| 4 | Identity, roster, authorisation, audit | Only if the second person is real, and only after the product boundary moves |

Phases 0 and 1 are worth doing even if the terminal interface is never built: they are what makes two
windows of the same application coherent, which is a thing the window can already be.

## 10. What this would record

- **`D115` — a streamed resource has many viewers and one writer.** A pane's keyboard and a
  conversation's outstanding ask are leased; everything the host owns broadcasts, and everything it
  answers is per-asker. Cost: a lease to display, hand over and reap, and a first-frame problem for
  every joiner.
- **`G255`** — a pane's geometry belongs to its lease holder, so a viewer on a smaller terminal
  letterboxes and nothing tells it why.
- **`G256`** — the ring gives a joiner bytes and not a screen: a shell mid-scroll repaints for nobody,
  and only a full-screen harness answers the nudge.
- **`G257`** — whether a live conversation buffer may exist is unsettled between this and
  `inbox-chat-resume`, and building one without settling it makes two designs for one fact.
- **`G258`** — every attached client is the same person by assumption; there is one token, no identity
  and no audit, so the lease is a safety catch and cannot be a permission.
- **`G259`** — reaping is written for one interface: `client_gone` takes panes and conversations with
  it, which is correct with one client attached and wrong with two.

## Related docs

- [`ratatui-tui-proposal.md`](./ratatui-tui-proposal.md) — the second interface that raises all of
  this, and the host record that lets it attach
- [`architecture.md`](../tech/architecture.md) — one host, many clients, and the rule that keeps the
  halves apart
- [`panes-and-terminals.md`](../features/panes-and-terminals.md) — focus, resize, and what closing a
  pane does
- [`chat.md`](../features/chat.md) — the outstanding-ask model a second reader has to share
- [`overview.md`](../product/overview.md) — the multi-user refusal that §7 runs into
