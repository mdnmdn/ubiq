# The hub, the framing, and the two ends of a remote connection

## `crates/ubiq-proto/src/bus.rs` — the switchboard

One host, many clients. A window attaches with `Hub::connect` and gets a `Client`; the host reads
every client through one `HostEnd` and answers each message **to somebody**. Attaching and
detaching are facts about the *transport* rather than things either half says, so they are
`FromClient` variants and not messages.

`hub()` opens the bus: one `HostEnd` for the process, and a `Hub` that mints a client per window.
Every channel is `flume::unbounded`. **Unbounded is the contract, not an accident** — a window
that falls behind must never stall the host's reader, because that stalls the harness.

*(`_docs/tech/architecture.md` still describes this as `pair()` opening two queues. The code is
`hub()`; the doc sentence predates the routing hub and is the thing to fix if you touch it.)*

| Type | Is |
|---|---|
| `Hub` | Cloneable, process-wide. The binary starts the host with the other end and hands this to the interface, which mints one client per window |
| `HostEnd` | One inbox for every client, plus the routing table to answer through |
| `Client` | A window's end: `send`, `from_host`, `sender`, `input`, `id` |
| `ClientId` | One attached window, for as long as it is attached. **Not a contract id** — never serialises, never persists. A detached host would take it from the connection it accepted, not from anything a client said |
| `To` | `Client(ClientId)` — the window that owns the pane or asked the question — or `Everyone` |
| `FromClient` | `Connected(id)`, `Said { client, message }`, `Gone(id)` |
| `Mailbox` | A pre-addressed host-side sink, resolved once so the hot path never takes the routing lock |
| `Outbox` | A cloneable way to speak to the host with no window in hand, carrying the client id |
| `PaneInput` / `PaneOutput` | The `Write` and blocking `Read` a pane's emulator gets **in place of a pseudo-terminal** |

### Routing

- `HostEnd::send(To::Client(id), msg)` — one window. `To::Everyone` — every attached client, each
  getting a clone. **A broadcast with nobody attached is not a reason for anything to stop.**
- `HostEnd::mailbox(to)` resolves the destination **once**, so a pane's reader thread never takes
  the lock per chunk. `Mailbox::send` answers `bool`: whether the destination is still reachable.
  **That answer is what a pane's reader thread stops on** — once the window that owned the pane
  has gone, a reader that kept draining the pseudo-terminal into nowhere would keep the harness
  alive with it.
- The host's run loop is `Coordinator::run` in `crates/ubiq-host/src/coordinator.rs`:
  `HostEnd::recv()` (or `recv_timeout`, for work of its own like a debounced preference write)
  feeding `client_here` / `dispatch` / `client_gone`.
- `Client`'s `Drop` removes it from the routing table and sends `FromClient::Gone`, so the host can
  reap the panes that window owned. **Nothing else drops**, now that the host outlives every window.

`FromClient::Said` is as wide as the contract's widest variant on purpose: `Message` is already
the one place a size trade-off is made, for the terminal chunks on the hot path, and boxing it
again here would move the cost to every dispatch site for a clippy heuristic rather than a real one.

### `PaneOutput` / `PaneInput`

The emulator wants a `Read` and a `Write`. It gets these, which are bus endpoints for **one pane
ID and never a pseudo-terminal** — that is what keeps the UI honest about a pane being an ID plus
a byte stream. `pane_output()` returns the sender (for the router) and the reader (for the
emulator). Reads block, because the emulator reads on its own thread; **dropping the matching
sender is how a pane is told its harness is done** — the read returns end of stream. An empty
chunk deliberately does not read as EOF.

### `detached()` — a client with no `Hub`

`detached()` mints the `Client` half of the contract with **no networking, no framing, nothing
that touches a socket**, plus a `Detached` handle a pump drives: `said()` is the outbound
receiver (everything `Client::send`, `Outbox` and `PaneInput` produce, as `FromClient::Said`), and
`deliver()` is the inbound sender (a message from the socket surfaces on `Client::from_host`
exactly as if a `Hub` had routed it). Detached ids come from a **separate process-wide counter**,
so the two numberings can never collide into one id meaning two different clients. `clients` is
`None`, so there is no routing table to remove itself from on drop — but the `Gone` announcement
still goes out, because whatever drives the socket still needs to know.

### The tape

`bus::tape()` is a process-wide ring of everything that crossed, for the debug viewer, modelled on
`log::Logs` and one-way for the same reason. `TAPE_CAPACITY` 500, `TAPE_MAX_JSON` 8 KiB per entry,
`TAPE_DIR_ENV` = `UBIQ_TAPE_DIR` turns on continuous untruncated capture to a JSONL file (unset —
the normal case — and nothing is ever written). `Tape::dump()` writes the ring to
`ubiq-tape-<stamp>.jsonl`, one JSON object per line, so ids and tokens are `jq`'s to pull out.

**`Tape::record` skips `TerminalOutput`, `TerminalInput` and `TerminalResize` entirely.** A tape
that serialised every terminal chunk would put a JSON encoder on the path between the
pseudo-terminal and the screen and stall the harness — the one thing the bus exists to avoid.

## `crates/ubiq-proto/src/wire.rs` — the framing

**A frame is a 4-byte big-endian length prefix followed by a MessagePack body.** This module owns
framing only: no socket, no thread, no notion of a connection.

| Item | Is |
|---|---|
| `MAX_FRAME` | 64 MiB. A prefix claiming more is refused **before the body is allocated**, so a corrupt or hostile header costs four bytes |
| `encode` / `decode` | Body only, no prefix — for callers that frame differently, and for the size tests |
| `write_frame` / `read_frame` | Prefix plus body |
| `WireError` | `Io`, `Encode`, `Decode`, `FrameTooLarge(u32)`, `Eof` |

**`Eof` is its own variant on purpose**: a peer that closed cleanly *between* frames is how a
socket pump learns the other side is done, and must never be conflated with a torn frame or a real
I/O failure. A partial prefix or a partial body is a real error, because the stream was already
inside a frame — `read_exact_or_eof` is what tells the two apart.

**MessagePack via `rmp-serde`, and self-describing is the reason, not a side effect** (`D79`).
`wire.rs` encodes with **`to_vec_named`, never the compact positional `to_vec`**:

- `ProjectSnapshot` flattens `ProjectRecord` into itself with `#[serde(flatten)]`, and flatten
  needs a map to merge into. A positional encoding has none, so `to_vec` fails to round-trip it.
- Dozens of optional fields across the message set carry `skip_serializing_if`, which needs a
  format that deserialises into a self-describing shape (`deserialize_any`).
- **A remote host and a UI built at different revisions do not have to agree on field order** to
  decode each other's frames — a fact worth having before either half can be on the far end of a
  socket. Postcard and bincode were rejected for exactly this.

The cost: field names on every frame, and **a `Vec<u8>` costs one MessagePack integer per byte
unless it carries `#[serde(with = "serde_bytes")]`**. Three fields on the hot path carry it —
`TerminalOutput.bytes`, `TerminalInput.bytes`, `WriteProjectFile.bytes`. Losing that attribute on
a future field would silently blow up its frame size and nothing but
`a_terminal_frame_is_close_to_its_payload_size_not_several_times_larger` would catch it.

## `crates/ubiq-host/src/remote.rs` — the listener

**Everything this module does is transport.** The only thing a connection needs from the bus is
`Hub::connect`, so a connection here is: attach, pump frames off the socket onto the client, pump
messages off the client onto the socket, drop the client when either side stops. **No message
family is special-cased, and none should ever be added here.**

`serve(hub, bind) -> Serving { addr, token }` binds a `TcpListener`, generates the token, and
spawns the accept loop on `ubiq-remote-listen`. The `Hub` is cloned into it; the caller keeps its
own clone for local windows — **this adds a second kind of attacher, not a different `Hub`**.

**Why `std::net::TcpListener` and not `tiny_http`** (used elsewhere in the workspace for the OAuth
redirect): `tiny_http` cannot hand back the raw socket after answering — it owns the connection
for the life of one response. This listener answers one HTTP-shaped request and then keeps the
same socket for the life of a session, reading and writing raw frames. **Do not "simplify" this to
`tiny_http` later.**

### The handshake

| Constant | Value | Why |
|---|---|---|
| `MAX_HEADER` | 8 KiB | An unauthenticated peer gets no chance to make this host buffer without bound |
| `HANDSHAKE_TIMEOUT` | 10 s | The header block is read a byte at a time from an unauthenticated socket; `MAX_HEADER` bounds the memory and nothing else bounds the time. **Cleared the moment the upgrade is written**, because a session is idle between frames by design |
| `STOP_POLL` | 200 ms | How often the writer thread wakes to check whether the reader gave up |

Routes: `GET /attach?token=<token>` with an `Upgrade: ubiq` header → `101 Switching Protocols`
(then raw frames); a bad or missing token → `401`; a plain `GET /` → `200` with *"this is a Ubiq
host. attach at /attach?token=<token>."*; anything else → `404`; an oversized header → `400`.

Two details that look like fussiness and are not:

- **`read_request` reads one byte at a time, deliberately not through a `BufReader`.** A buffered
  reader can pull well past the header block's end in one underlying read, and everything past the
  blank line **is the first wire frame** once the handshake upgrades. One syscall per byte is a
  one-time cost on a connection that then runs a whole session.
- **`constant_time_eq`** never early-returns on a length mismatch or the first differing byte. A
  length check up front, or `iter().eq()`, would let a timing side-channel narrow the token byte
  by byte.

The token is 256 random bits rendered base64url with no padding — long enough to paste into a
connection string, short enough to actually paste. `base64_url_no_pad` is spelled out locally so
this module needs no new dependency for one call site.

### The session

`pump(stream, hub)` calls `hub.connect()` and runs two threads. The `Client` is shared behind an
`Arc` because both directions need it (`send` and `from_host` both take `&self`); it drops —
telling the coordinator `FromClient::Gone` — once both threads release their half. **The writer
polls `from_host` with a timeout rather than blocking**, because a blocking `recv()` would not
notice the reader giving up: the coordinator has no reason to stop answering just because the
socket died. The reader sets `stopped` on its way out.

### Starting one

`ubiq --serve` (`0.0.0.0:7420`), `--serve=<addr>`, `--bind <addr>`, `--port <n>`; `--bind` and
`--port` each win over the matching half of a `--serve` value, and either alone also asks for a
server. **`--serve`'s value is always attached with `=`** — `argv_paths` skips a `-`-prefixed
token without knowing the next one belongs to it, so a separated value would be read as a project
path. A served run **opens no window** and takes no part in the one-application-per-root handoff.
A bad value or a bind failure exits 2 rather than leaving something that looks like it worked. The
banner (bind address, token, and a paste-ready `http://<ip>:<port>?token=<token>`) goes to
**stdout, not through `tracing`**, so no log filter can hide it.

**`--serve` binds every interface** because a single-interface default would require the operator
to know in advance which address their machine is reachable at, which is what a first `--serve`
run cannot assume (`D80`). Whoever holds the token has a terminal on that machine, and the
connection is **plaintext** — no TLS, no rotation, no expiry, no per-client identity. Usable
behind a trusted network or a tunnel the operator adds. That is `G168`.

## `crates/ubiq/src/app/remote_connect.rs` — the dialer

The mirror image of the listener's accept side: one binding, one dialing. Read the two together.
The transport half is deliberately dumb and knows no message family.

**Never on the GPUI thread.** Every step of `dial` is a blocking syscall — DNS, TCP connect, a
byte-at-a-time header read. `AppState::try_connect_remote` runs it in `cx.background_spawn` and
reads the answer back through `cx.spawn`. A modal that called this from a click handler would
freeze the window for as long as a dead address takes to time out.

`CONNECT_TIMEOUT` is 6 s — Ubiq's own short ceiling, because the platform's connect timeout can be
minutes. It bounds the TCP handshake *and* each line of the HTTP response (a firewall swallowing
the request would otherwise hang the thread forever), and is **cleared once the upgrade lands**.
`MAX_HEADER` and `STOP_POLL` mirror the listener's, and `read_capped_line` makes the same
one-byte-per-syscall trade for the same reason.

`is_typable` refuses any control character in the address or the token **before the socket is
opened**. *A connection string is pasted from somewhere else — that is the whole point of it — so
its halves are untrusted text.* A CR or LF would end the request line and let what followed be
read as further headers: request smuggling with the user's own hand on the paste.

`attach_request` builds exactly what `remote.rs::read_request` parses, and a test pins its shape
with no socket at all. `101` → attach; `401` → `ConnectFailure::TokenRejected`; anything else →
`Refused`. Then `bus::detached()` and `spawn_pump`, two threads named
`ubiq-remote-client-{reader,writer}`.

`land_remote_connect` discards a `Client` that arrives after the user cancelled or retried —
**dropping it tells the host this window is gone**, which is the right thing to say to a host
nobody asked to keep talking to. On success `attach_remote` registers the connection and:

```rust
self.bus.send_to(HostRef::Remote(host_id), Message::ListProjects);
```

**Addressed rather than resolved, because attaching a host is not choosing it** — `active` stays
where the user left it. Nothing about a remote is known until it says so, and it says nothing
unasked.

## `crates/ubiq/src/app/hosts.rs` — the window's multiplexer

`Bus` stands in for the `ubiq_proto::bus::Client` field `AppState` used to hold, and exposes the
same four methods under the same names and signatures — `send`, `sender`, `input`, and (through
`connections()`) `from_host`. That is deliberate: `crates/ubiq/src` calls `bus.send(...)` at ~130
sites and none of them may need to learn that a message might now have somewhere else to go.
**What changed is what answers a call, not how a call is written.**

`HostRef` is `Local` or `Remote(HostId)`. **The local host is always attached and a remote is
added *alongside* it**, never swapped in, so a window's terminals never all move because one
remote dropped (`D81`). `HostId` is UI-local: minted here, never in a `Message`, meaningless to
any host — **every connection remains, from a host's point of view, an ordinary single-host
session**, which is what keeps the listener free of multi-host awareness.

### Resolution — `Bus::resolve`

1. **Pane first** (`pane_id_of`) — a pane is a running harness on one specific machine; its input,
   its resize and the close that kills it do nothing anywhere else.
2. **Project second** (`Message::project_id()`) — a project has no single pane but is still hosted
   on one machine, so a git refresh or a file read must land there even with no pane open.
3. **`active` last** — a message naming neither has nothing to be resolved *from*; it can only
   mean whichever host the user is pointed at.

`pane_id_of` lives here rather than on the contract because routing is the interface's concern:
the pane family is a handful of variants, not most of the enum. It covers `TerminalOutput`,
`PaneExited`, `PaneError`, `TerminalInput`, `TerminalResize`, `Focus`, `CloseWorkspace`,
`HarnessLoginStarted`, `HarnessLoginLink`, and `WorkspaceSpawned` (via `workspace.id`).

### The rest of the surface

| Method | Does |
|---|---|
| `send` | Resolve, then send |
| `send_to(host, msg)` | Bypass resolution — browsing a remote's filesystem or listing its repositories before any project on it exists |
| `client_for(host)` | `None` for a host no longer held. **Deliberately not a fall back to local** |
| `sender()` / `input(pane)` | Must return something, so these are the one place a gone host lands on the local client — with a log line |
| `connections()` | Every inbound stream tagged with its `HostRef`; read once at construction by `boot.rs` |
| `register_remote(client, label)` | Mints the `HostId`, hands back the inbound stream — the caller still spawns its router |
| `note_pane` / `note_project` / `forget_*` | The routing maps, in `RefCell` so `send` can stay `&self` |
| `drop_remote(id)` | Forgets the host's projects, resets `active` to `Local`, **hands its panes back** |
| `projects_not_on(host)` | Every project some *other* host reported |
| `active` / `set_active` / `remotes` | The Hosts section's dropdown |

**`client_for` returning `None` is the point** (`D85`): a dropped remote's pane ids were minted by
that remote and mean nothing here, so sending its keystrokes or the close that kills it to the
local coordinator is not a harmless near-miss — it is a coordinator being asked about panes it
never created. **An undeliverable message is dropped and logged.**

**`drop_remote` hands panes back rather than forgetting them**, because closing one is the
caller's job: `close_pane` still sends a `CloseWorkspace`, and with the pane still recorded that
close resolves to the absent host and is dropped, where forgetting it first would resolve it to
`active` and ask the local host about an id it never minted.

`host_menu_rows(remotes, saved, failed)` builds the dropdown: `Local`, then live remotes, then
saved hosts with nothing live behind them. **A saved host already attached is not listed twice** —
folded into the attached row by `save_id` (and by address when ids are empty), because showing
both would let one host answer to two rows with two different fates for a pick. `HostStatus` is
`Attached` / `NotAttached` / `Failed`.

## `crates/ubiq/src/app/wire.rs` — the UI's dispatch

`route_host(host, from_host, cx)` is **one router task per connection**: drain the stream, hand
each arrival to `receive` **tagged with the `HostRef` it came from** before that method ever sees
it. Shared by `boot.rs`'s loop over `bus.connections()` and by `attach_remote`, so the tagging
lives once. When the channel disconnects — for a remote, the pump threads gave up — it runs
`disconnect_host` and marks the address failed.

`receive(host, message, cx)` tries twelve `receive_*` helpers in order — pane, project, file, git,
work, conversation, session, account, search, assist, repo, host browse. **The families are
disjoint, so each helper answers with the message back when it is none of its own** and the next
is offered it. The fall-through logs *"the window was sent a message only it may send"*.

`disconnect_host(id, cx)` is one method for both ways a host is lost — the socket ending under
`route_host`, and the Hosts section's Disconnect button. It closes every pane the host was running
**before** anything about them is forgotten, then drops the connection, which tells the far side
over `FromClient::Gone` that this window is no longer attached. It answers with the label. A
saved host then starts an unattended reconnect that re-registers under a new `HostId` and
re-asks `ListProjects`; the host still reaps panes on `Gone`, so there is nothing to reattach
those harnesses to.

### Catalogue merging

A window attached to several hosts holds **one** project list for all of them, because a remote
appearing or vanishing must not rearrange work that has nothing to do with it (`D85`).

```rust
Message::ProjectList { projects } => {
    let keep = self.bus.projects_not_on(host);   // read before cx.global_mut borrows the app
    for project in &projects { self.bus.note_project(project.record.id, host); }
    cx.global_mut::<WindowRegistry>().replace_all_except(projects, &keep);
```

**A `ProjectList` is the whole truth about the host that sent it and says nothing about any
other.** Applied as a plain replacement it would erase every other host's rows — which is how
attaching a remote used to make the local machine's projects vanish.

**`Message::HostInfo` is taken only from `HostRef::Local`.** Every host greets a new client with
it unsolicited, so a remote's would repaint the config root and — worse — re-ask for the shells,
agent types, accounts and profiles, whose answers replace those lists whole. The menus name what
can be started on *this* machine (`G188`).

## `crates/ubiq/src/state/windows.rs` — `WindowRegistry`

The one piece of workbench state that is **process-wide rather than per window**: a project is
open in at most one window, so no window can answer "where is this project open?" from its own
copy. **The catalogue is not here** — it belongs to the host, and this is a projection of it,
keyed by id in a `BTreeMap` (a ULID sorts by creation time) and idempotent by construction,
because project messages are broadcast and every window applies the same snapshot.

- `replace_all(projects)` = `replace_all_except(projects, &[])`.
- **`replace_all_except(projects, keep)`** removes the kept ids from the current map first, merges
  the incoming snapshots over them, and then reconciles every window slot: a window holding a
  project the catalogue no longer names loses it and **stays open on nothing**, because a
  catalogue arriving is not the user closing anything. Ubiq never closes a window on the user's
  behalf.
- `apply(snapshot)` for one row, `forget(id)` for a `ProjectForgotten`.

Its cost is one thing to hold onto: a catalogue answer is only as scoped as the host tagging
behind it, so **a project whose host was never recorded** — `note_project` missed at the one place
it is first announced — is either erased by the next host's answer or kept forever by every one of
them.
