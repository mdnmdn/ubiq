---
id: inbox-message-queue-steering
title: Problem — a turn in flight can only be queued behind or cancelled, never steered
kind: proposal
status: proposal
summary: Everything Ubiq does with a prompt typed during a running turn happens in the interface — `Conversation::queued` holds it and the turn ending sends the front of the queue. No bridge carries a mid-turn message, because the ACP client refuses a second `session/prompt` outright while the Claude Code bridge could write one but is never asked to, leaving the harness's own queue and its `cancel_queued` interrupt unused. This is what each layer does today and what steering would cost.
read_when: you are changing how a prompt is held or sent while a turn is running, wiring mid-turn injection, or asking why Enqueue waits for the turn to end
updated: 2026-09-14
depends_on: [feat-chat, ref-acp-protocol, tech-agent-manager, tech-transport]
---

# Problem — a turn in flight can only be queued behind or cancelled

A user watching an agent go the wrong way has two moves in Ubiq: **Stop**, which throws the turn
away, or **Enqueue**, which waits politely for the wrong turn to finish. There is no third — no way
to say "keep going, but use the existing helper" and have it reach the model before the next tool
call. Every harness this matters for has a mechanism for it. None of them is wired.

This document says what the four layers actually do today, where the two bridges diverge, and what
the smallest honest version of steering would cost.

## 1. Where it stands

**The queue is interface state, and only interface state.** Nothing below `crates/ubiq` knows a
prompt was held.

| Layer | What it does with a prompt typed mid-turn |
|---|---|
| Composer | `AppState::send_or_enqueue` — `run != Working` sends; `Working` with text enqueues; `Working` with nothing does nothing, because Stop is a separate control |
| `Conversation` | `queued: Vec<QueuedMessage>` with `enqueue` / `dequeue_front` / `remove_queued`, beside `draft` and `attached` for the draft's reason |
| `AppState::wire` | On every `ConversationUpdate`, once `run == Run::Idle`, `dequeue_front()` → `send_prompt`. One entry per turn end, oldest first |
| Bus | `Message::PromptAgent { agent_id, text }` — a plain turn. There is no message that means "while you are working" |
| Coordinator | Live conversation → `Conversation::prompt`; no conversation yet → `launch_pending`, the first prompt being also the launch trigger |
| Bridge | `AgentInput::Prompt { content }` |

Two details of the interface half are deliberate and worth keeping through any change.
Attachments are composed into the text **at enqueue time**, so a queued entry is one flat string: a
queue row carrying its own tag list would need its own tag row, its own removes and its own
colouring, which is a second composer. And `send_prompt` is the only builder of `PromptAgent`,
so the one-shot preamble folds in front of whichever turn is genuinely first — the composer's or
the queue's. [`../features/chat.md`](../features/chat.md) owns both facts.

## 2. Where the two bridges diverge

**ACP (`io/acp_client.rs`) refuses.** `session/prompt` is long-lived, so the write does not block:
the id is allocated, recorded in the one-cell `shared.turn` slot *before* the line goes out, and the
reader matches the eventual response to emit `TurnEnded`. A second `AgentInput::Prompt` while that
slot is occupied is an error — *"acp runs one turn at a time: the prompt with id N is still in
flight"* — and no frame is written. That is correct for ACP v1 and not a limitation to route around:
overwriting the slot would strand the first response and its caller would wait for a `TurnEnded`
that can never come.

**Claude Code (`io/jsonl.rs`) would simply write it.** The bridge has no turn slot and no in-flight
guard; a second prompt would be written to stdin as another `{"type":"user",…}` line. The CLI
already handles that: its interrupt control request answers with `still_queued`, and
`cancel_queued: true` cancels every uuid-stamped message queued behind the turn — a queue the
harness maintains and `am` currently never puts anything in
(`crates/agent-manager/_docs/harness/claude-code.md` §"Process lifecycle"). So the capability is
there on the Claude Code path, untested, reachable only because nothing above ever sends into it.

**Codex (`io/codex.rs`)** blocks on `turn/start`'s ack only, to capture `turn.id` for
`turn/interrupt`. Same shape as ACP for our purposes: one turn named at a time.

The asymmetry is the real finding. The queue lives in the interface because **the weakest bridge
sets the behaviour for all of them**, and no layer between the composer and the bridge carries the
distinction. Both bridges synthesise the `UserMessageChunk` themselves, since neither path echoes a
live turn's prompt back — which means a steered message would also have to draw itself, and
somewhere other than the transcript's turn body.

## 3. What is missing

Nothing in the tree discovers, advertises or sends an injection. There is no `session/inject`, no
`queue` / `steer` / `interrupt_immediate` mode, and `prompt_caps` reads only content-type
capabilities off `initialize`. `session/inject` is a harness extension in any case, not ACP — the
protocol's own answer is **v2**, where a `session/prompt` response means "acknowledged" rather than
"the turn is over", the agent replays the user message where it inserted it, and it signals when it
is idle. [`../references/acp-protocol.md`](../references/acp-protocol.md) §"What v2 changes" is the
owning statement; queueing and mid-turn steering are named there as the affordances that unlocks.

So mid-turn redirection today is exactly two things: hold it in the client, or `CancelTurn` and
re-prompt.

## 4. What improving it costs

Three steps, each worth having on its own, in the order their cost rises.

**a. Say what the queue is for.** The cheapest change is the one with no wire in it: the queue is
drawn as a list of pending prompts, and a reader cannot tell that the entry will arrive *after* the
turn rather than *during* it. A line on the composer and a queue-row affordance that reads "sends
when this turn ends" costs a string and settles the expectation the rest of this document is about.

**b. A steer that the interface routes, not a new message.** A second `Message::PromptAgent` while
working already means something coherent at every layer except the ACP bridge, which refuses it
honestly. The minimum wiring is therefore not a new message but a **capability on the conversation**
— something like `Conversation::accepts_input`, which already exists for exactly this shape of
question, extended to say *accepts input while working*. The coordinator answers it from the
bridge; `send_or_enqueue` reads it and, where it is true, sends instead of enqueuing. Claude Code
gets steering; every other harness keeps today's behaviour, and no bridge has to lie.

The cost sits in the transcript, not the transport. A steered message is not the head of a turn: it
has to draw inside the running turn's body, in order, and the synthesised `UserMessageChunk` both
bridges emit is currently read as "a turn opened". `ConvUpdate` would need to distinguish the two,
which is a wire change and therefore a `Dnn`.

**c. `cancel_queued` on Stop.** Independent of the above and small: `am` writes the interrupt with
`cancel_queued` left off, "having nothing queued behind the turn it is stopping". Once (b) puts
something there, that stops being true — Stop must cancel the harness's queue too, or a stopped turn
is followed by the message the user stopped it to avoid.

Beyond these, ACP v2 is the general answer and a separate piece of work: it makes steering the
protocol's own behaviour rather than a per-harness extension, and it is also what would let two
panes observe one conversation. It should not be approached as a steering feature.

## 5. Open questions

- Does a steered message belong in the queue list until it is acknowledged, or does it go straight
  into the transcript on write, as a sent prompt does today?
- If a harness accepts mid-turn input, is Enqueue still offered at all, or does the control collapse
  back to Send with the queue reserved for harnesses that cannot take one?
- Claude Code's queue is uuid-stamped and cancellable per message. Ubiq's queue ids are a local
  counter with no relation to it. If the client's queue drains into the harness's, which one does a
  queue row's delete control address?
