---
name: ubiq-transport
description: Reference for the wire between Ubiq's two halves — the message contract in crates/ubiq-proto, the Hub and its routing, the socket framing, the ULID id types, the log sink, and the remote listener and dialer that put a UI on another machine. Use when adding, changing or removing a message, touching the bus or routing, wiring a remote host, changing the framing, adding an id type, or adding a log subsystem.
---

# The transport

Everything the two halves of Ubiq share lives in `crates/ubiq-proto`, and nothing either half
owns alone does. The interface and the host both depend on it; **neither depends on the other**.
That is a crate boundary, checked by `just ui` and `just host`, not a convention.

```
crates/ubiq-proto            crates/ubiq-host              crates/ubiq
"what may be said"           "the one host"                "the windows"
messages.rs  the enum        coordinator.rs dispatches     app/wire.rs dispatches
bus.rs       Hub/Client      remote.rs listens (TCP)       app/hosts.rs multiplexes
wire.rs      socket framing                                app/remote_connect.rs dials
ids.rs       ULID newtypes
log.rs       the sink
```

## Read first

| You are | Read |
|---|---|
| Adding, changing or removing a message | `_docs/tech/transport-contract.md` — **owns every message fact** |
| Wiring either half to the bus, or crossing the UI/host line at all | `_docs/tech/architecture.md`, then the transport contract |
| Touching the listener, the dialer, `--serve`, or a saved host | `_docs/tech/architecture.md` ("Why the split is drawn before it is needed"), `_docs/tech/operations.md` |
| Adding a log event or a subsystem | `_docs/features/logs.md` |
| Arguing with a rule here | `_docs/tech/decisions.md` — `D24`, `D27`, `D28`, `D32`, `D53`, `D79`–`D82`, `D85` |
| Wondering what is still missing | `_docs/backlog.md` — `G168` (no TLS), `G169` (saved hosts), `G188` (per-host menus), `G189` (no keepalive) |

Your change updates the documents it touched in the same commit. `just docs-touched` names them.

## The hard rules

- **The UI and the host talk only through `Message`.** No direct call, no shared handle, no
  callback that skips the enum — even though they share a process. `crates/ubiq` has no
  `crates/ubiq-host` in its dependency graph, so reaching around is a compile error (`D27`).
- **A pane is an ID plus a byte stream.** No path, no process handle, no file descriptor crosses
  into UI code. A record that crosses the bus must survive serialisation, which is the mechanical
  form of that rule — `WorkspaceInfo` carries geometry and never a writer.
- **Every message in the pane family carries a `PaneId`**, including in the single-pane case.
- **Terminal bytes are opaque**, and `bytes` is a byte sequence and never a `String`: harness
  output is not guaranteed valid UTF-8 at a message boundary.
- **One host per process** (`D28`), started by the binary before the first window. A window
  attaches with `Hub::connect` and gets a `Client`.
- **Pane replies route to the one window that owns the pane; project messages broadcast.**
  `ProjectAdded` / `ProjectChanged` / `ProjectForgotten` go to `To::Everyone` so every picker
  agrees by construction. A `ProjectList`, a `Preferences`, a `Settings` and every file, git,
  work and search reply go to `To::Client(asker)`.
- **The bus never blocks the host's reader.** Every channel is unbounded, on purpose: a window
  behind on drawing must never stall the thread draining a pseudo-terminal, because that stalls
  the harness. A full queue is a slow UI, not a stopped harness.
- **A remote connection is an ordinary client of the same `Hub`.** `remote.rs` does
  `Hub::connect()` plus two pump threads and nothing else. **No message family is special-cased
  for a remote, and none may ever be.**
- **`HostId` and `ClientId` are not contract ids.** Neither serialises; a host has no way to learn
  another host exists (`D81`).
- **Payloads are owned.** No borrowed data, no handles, nothing that fails to serialise.
- **Material crosses only in a `Secret`, and a `Secret` is never printed** (`D65`). It has no
  `Display`, no `Deref`, no `AsRef<str>`; `Debug` writes `Secret(***)`; `expose()` is the one way
  out, so every leak site is one grep. The log sink and the tape both serialise whole messages.

## Checklist — adding or changing a message

1. **Pick the family.** The transport contract's "Adding a variant" section is the decision tree;
   `reference/messages.md` has the same test in table form. Wrong family is the expensive mistake,
   because it decides who answers and who hears.
2. **Add the variant** to `crates/ubiq-proto/src/messages.rs`, inside its family's `// ── … ──`
   banner and on the right side of the direction split. Owned payload only.
3. **Add a row to the table in `_docs/tech/transport-contract.md`, in the same commit.** That
   document owns the fact; nowhere else may restate it.
4. **Handle it in the host's dispatch** — `Coordinator::dispatch` in
   `crates/ubiq-host/src/coordinator.rs`. *A message the coordinator receives but ignores is worse
   than one that does not exist.* Response-direction variants are rejected there rather than
   falling through.
5. **Handle it in the UI's dispatch** — the matching `receive_*` helper in
   `crates/ubiq/src/app/wire.rs`. The helpers are tried in order and each hands the message back
   when it is none of its own; the fall-through logs *"the window was sent a message only it may
   send"*.
6. **If it carries a `pane_id`, add it to `pane_id_of` in `crates/ubiq/src/app/hosts.rs`.** The
   catch-all arm answers "no pane", so a pane-carrying variant left out routes to whichever host
   is *active* rather than to the one that owns the pane. With one host attached nothing goes
   wrong — which is exactly why this is easy to miss.
7. **If it carries a project id, add it to `Message::project_id()`** in `messages.rs`. That is the
   second half of `Bus::resolve`. (`Suggest` has its own arm because the id sits inside
   `SuggestSubject`, not beside it.)
8. **If it makes a structural choice, append a row to `_docs/tech/decisions.md`.**
9. `just verify` (= `check clippy test host ui docs-lint`), and `just fmt`.

Adding a **record** or an **enum** that travels in a payload obliges the same table update — the
transport contract lists all of them. A new **id kind** is one `ulid_id!` invocation in `ids.rs`;
never reuse an existing kind because the shape matches.

## Gotchas that bite

- **A byte-vector field needs `#[serde(with = "serde_bytes")]`.** Without it `rmp-serde` writes one
  MessagePack integer *per byte* — roughly 4x the payload. Three fields carry it today:
  `TerminalOutput.bytes`, `TerminalInput.bytes`, `WriteProjectFile.bytes`. Only the size test in
  `wire.rs` catches a fourth that forgets (`D79`).
- **The enum is as wide as its widest variant, and the terminal chunks share it.** A payload over
  ~200 bytes gets boxed — `AgentChanged` (a `WorkAgent` is 272 bytes) and `ConversationUpdate` are
  the two that do. A `Box` serialises as what is inside it, so the wire form is unchanged.
- **The tape skips the pane hot path.** `bus::tape()` records every message as JSON for the debug
  viewer *except* `TerminalOutput` / `TerminalInput` / `TerminalResize` — a JSON encoder between
  the pseudo-terminal and the screen would stall the harness. Don't add a family to that skip list
  and don't take one off it.
- **Ordering is per pane and per agent, never global.** Two messages for the same pane arrive in
  order; across panes nothing is promised. `ConversationUpdate.seq` is per-agent, monotonic, from 1.
- **The file family and the host browse family share one worker and one queue**, so answers come
  back in the order they were asked. A pool would reorder and cost a sequence number on the wire.
- **An interface-minted id is the stale-answer discipline.** `SearchId`, `ConnectId`, `CloneId`,
  `RepoQueryId`, `SuggestId` and the conversation's `AgentId` are minted UI-side so a reply naming
  an id nobody still holds is discarded on arrival rather than drawn.
- **`Bus::client_for` has no fall back to the local client.** A message for a host that is gone is
  dropped and logged, because a dropped remote's pane ids mean nothing locally (`D85`).
- **`HostInfo` from a remote is ignored.** Only the local host's is applied, so the config root and
  the shells/agent-types/accounts/profiles menus stay the local machine's (`G188`).

## Verifying

`just verify` is what a change has to pass. `just ui` alone proves the interface still cannot see
the host crate; `just host` proves the host still cannot see a drawing crate. Framing has its own
tests inside `crates/ubiq-proto/src/wire.rs` — round-trip, flattened-snapshot, non-UTF-8 bytes,
frame size, torn frame, clean EOF, oversize prefix. `UBIQ_TAPE_DIR=<dir>` captures every non-pane
message crossing the bus as JSONL from the first message on.

## Reference files

- [`reference/messages.md`](reference/messages.md) — every family, what it asks, what answers it,
  and the invariant it carries; plus the records, the ids and the log sink.
- [`reference/hub.md`](reference/hub.md) — the `Hub`, routing, the framing, the remote listener and
  the dialer, and the window's multiplexer over several hosts.
