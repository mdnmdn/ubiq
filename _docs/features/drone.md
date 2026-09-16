---
id: feat-drone
title: Drones
kind: feature
status: current
summary: One small executable Ubiq places on a machine it is not running on, serving that machine's terminal, files, search and machine facts over a single duplex byte stream — attached as an ordinary host over an SSH exec channel, with three lifetimes, a per-project origin, and a hash-pinned binary the interface uploads when the remote `PATH` has none.
read_when: you are changing how Ubiq reaches a machine it is not running on — the drone binary, its handshake, its lifetime, its deployment, or the SSH profiles and surfaces behind it
updated: 2026-09-16
verified: 2026-09-16
code_anchors: [crates/ubiq-drone/src/main.rs, crates/ubiq-drone/src/lib.rs, crates/ubiq-drone/src/relay.rs, crates/ubiq-drone/src/socket.rs, crates/ubiq-drone/src/linger.rs, crates/ubiq-drone/src/state.rs, crates/ubiq-drone/src/search.rs, crates/ubiq-drone/src/scrollback.rs, crates/ubiq-host/src/carrier.rs, crates/ubiq-proto/src/carrier.rs, crates/ubiq-proto/src/drone/mod.rs, crates/ubiq-proto/src/drone/manifest.rs, crates/ubiq-proto/src/settings.rs, crates/ubiq-proto/src/projects.rs, crates/ubiq-host/src/settings.rs, crates/ubiq/src/app/ssh_connect.rs, crates/ubiq/src/app/remote_connect.rs, crates/ubiq/src/app/hosts.rs, crates/ubiq/src/app/projects.rs, crates/ubiq/src/ui/remote_connect.rs, crates/ubiq/src/ui/settings.rs, crates/ubiq/src/ui/sink/project.rs, crates/ubiq-app/src/lib.rs, _tools/drone.py]
depends_on: [tech-architecture, tech-transport, tech-structure, wip-drone]
review_cycle: monthly
---

# Drones

## Purpose

A **drone** is one small executable Ubiq places where it is not running, serving that machine's
terminal, files, search and machine facts over a single duplex byte stream. A user picks an SSH
profile and a folder, and gets panes, a file tree, an editor and a content search against the far
machine inside the same window as everything local. Nothing durable is written to the far machine
beyond the binary itself and a socket, and no credential material crosses to it.

The crate is built twice from one source. The **relay** build — the one the tree has — carries no
harness knowledge at all: it links the lean core of `crates/ubiq-host` and serves pseudo-terminals,
files, browse, search and host facts. A **runtime** build, which would link `agent-manager` to
compose and confine harness runs inside WSL or a container, has no design; it stays with the working
note in [`../wip/drone.md`](../wip/drone.md).

## Behaviour

**A drone attaches as an ordinary host on the multi-host bus** (`D116`), not as a proxy the local
coordinator routes through and not as a new message family. The interface reaches it exactly as it
reaches a `ubiq --serve` host, so host routing, the Hosts menu, project listing and disconnect carry
it unchanged. The local coordinator never learns that a drone exists.

**A drone is reached over the SSH exec channel's own stdio.** No port is forwarded and no tunnel is
built: Ubiq spawns the system `ssh` with piped stdio and speaks the framed protocol over those
pipes. Ubiq never parses `ssh` output for structure — what comes back is an exit code and bytes.

### The handshake

**The drone speaks first.** It writes a `DroneHello` carrying its version, `os`, `arch`, `triplet`,
a message-schema number and a capability set, and waits for a `DroneReady` on the bare stream —
before the hub, the relay thread or the pump exists. A refusal is logged to standard error and exits
non-zero having spawned no pseudo-terminal and touched nothing on the far machine.

**The schema number is what an old drone and a new Ubiq disagree about.** It covers exactly the
messages a drone speaks: the pane family, the file family, the host browse family, the read half of
the project family, `ListShells` and `HostInfo`. A mismatch is answered with a refusal carrying a
sentence the connect modal shows verbatim, and both sides close cleanly.

**A capability is advertised only once it has been probed** (`D135`). A relay drone says `files`,
and says `search` as well only when it found a search tool to serve it with. Naming a capability the
drone would answer with a refusal is worse than naming none, because the interface would offer the
user something that fails.

**The heartbeat is a message pair, not a frame type** (`D118`). Either end pings and both answer. A
ping is owed only after twenty seconds of complete inbound silence, so a session with a live pane
puts no extra frame on the wire, and three unanswered pings — about a minute — is gone. A peer that
misses them is torn down by the same path a dropped link takes, because a second shutdown path is
a second chance to leave a pane running.

### Connection profiles and their secrets

An **SSH connection profile** is a name, a host, a port, a user and an authentication choice:
an agent, a key file, a password, or deferral to `~/.ssh/config` by alias — where the host field is
the alias and every other field is ignored, which the form says by hiding them.

**The profile list is the interface's and the secret material is the host's** (`D124`). The list
rides the ordinary settings write. A passphrase or password crosses only as a `Secret`, into the
connector store's own `ssh` namespace keyed by the profile id, and the answer to setting or clearing
one is the whole host-layer record, because the interface draws the flag that moves with it.

**The flags are re-stamped, never believed.** Every host-layer write prunes the store to the ids the
list still names and whose authentication still takes a secret, then reads each flag back. A profile
deleted, or switched to agent authentication, leaves no material filed under an id nothing names.

**The secret reaches `ssh` without passing through the interface** (`D125`). `SSH_ASKPASS` points at
Ubiq's own binary, and the child's environment carries a profile id and a config root — references,
never material. The helper mode opens the host's store itself and writes the secret on its own
standard output, where OpenSSH reads an askpass answer from. The whole path is keychain to helper
stdout to `ssh`: never `argv`, never a file, never the bus, never the drawing process.

### Three lifetimes

A drone's lifetime is two axes over one knob, and three presets name the points worth having
(`D131`):

| Preset | What it does |
|---|---|
| **Attached** | The `ssh` session *is* the drone. No socket, no detach; it dies with the link |
| **Session** | The drone detaches and outlives a dropped link for ten minutes |
| **Managed** | The drone detaches and outlives a dropped link indefinitely |

**Persistence is a unix socket at mode `0600`, never a terminal multiplexer.** Filesystem
permissions are the whole of the access control, since no port is forwarded. A multiplexer only
*holds* the detached process where a user can read its log; the socket is what reattaching finds.
The socket path derives from a hash of the canonicalised roots, so two windows cannot start rival
drones for one project, and its containing directory is forced to `0700` before the bind.

**The linger is the drone's own timer,** because the whole point is that Ubiq may be gone. It is
re-asserted on each attach over one line the attaching process writes before the handshake — the
drone's two halves talking over a socket only they can open — and a thirty-second startup grace
covers a zero linger expiring in the gap before the first client arrives. On expiry the drone kills
its panes and exits.

**Reattaching is a new host attach, not a resumed one.** A dropped link takes the lost host's panes
with it; on the new connection the drone re-announces each live pane and replays a bounded per-pane
scrollback ring, 256 KiB and oldest first, kept only while the drone is detached. Pane ids are
minted by the drone and stay stable, so it is the same pane rather than a live-but-blank one.
Closing a *pane* kills its harness (`D22`); what a dropped link ends is the link.

### Managed drones

**The state file answers the question a socket cannot.** A socket says whether something is
listening; a JSON file written beside it says *what* — version, pid, triple, start time, roots, pane
count, linger and socket path — without connecting. It is written through a temporary file and a
rename, so a reader racing the write sees one whole version or the other, and a slow thread
refreshes it so the pane count and linger stay roughly true between attaches.

**Liveness is the socket answering, not the pid.** A pid can be recycled between one check and the
next, where a socket that accepts a connection is a drone answering on this same call. A state file
whose socket does not answer is pruned — file and socket both — as it is found, because a caller
that lists is a caller about to decide whether to start one.

**Starting adopts rather than failing.** Binding a path a live drone answers on is success with
nothing served, and a path that answers nothing is removed and rebound. That is what makes the one
shell line the interface sends — start-or-adopt, then exec an attach — safe to re-run after a
dropped link rather than a race the second caller loses.

**Three command-line verbs report and end a drone without attaching to it:** one lists every live
drone in the state directory as JSON, oldest first; one prints a single drone's state and exits
non-zero when none is there; one stops it. Stopping speaks a second verb on the socket preamble,
answered with one line, so it needs no protocol change and no second shutdown path — it raises a
flag the relay's own tick reads.

### A project that runs on a drone

A project record carries an optional **origin** — a profile, the folder on the far machine, a
lifetime preset and an optional linger. Saving one writes the record and stops; nothing dials until
the project is opened. Opening it launches or adopts the drone from the interface's side of the bus,
adopting a host that is attached for that same profile and folder.

**For a duplicate project id the local host owns the row** (`D132`). A project the local catalogue
names while a drone fills it in is the one case where a host does not own its rows wholesale, so the
bus remembers every id the local catalogue ever named and answers *local* for it whatever host
serves it. Serving still moves, which is what puts a pane on the far machine, and a later local
re-listing does not take the folder back off a live drone. The drone is given the locally-minted
project id for that folder, so a reattach is the same project rather than a second one.

**A laptop that sleeps loses its drone, never its project.** When a drone goes away, each local row
it was serving is marked unreadable with a sentence naming the drone.

### Where the binary comes from

**The remote `PATH` is tried first and the deployer is the fallback** (`D133`), keyed on the far
shell's exit 127 and on nothing else, and only when no explicit remote path was named. Deploying
eagerly would turn a working connect into a refusal. The path that worked is remembered on the saved
host, and a hand-typed path works the same way on a machine where the binary sits off the `PATH`.

**Provenance is hash-pinned, not signed.** A generated manifest carries one entry per triple —
triple, path, SHA-256 and length — for the locally cross-built binaries, and the verifier proves
only that these are the bytes the manifest tool hashed, never who built them. There is no signing
key and no release pipeline. The commands that cross-build and hash are
[`../tech/operations.md`](../tech/operations.md)'s.

**The deployer is four reported steps:** probe the machine with `uname -sm` and map it onto a
triple; resolve the manifest row and hash-pin the local binary before anything leaves this machine;
upload it by `scp` into a per-version folder under the remote user's own home; and make it runnable.
A triple the manifest has no entry for is a refusal naming the triple and the build command. Each
step is reported, so a slow first connect says which step it is on rather than looking hung.

### Search

**A drone shells out to the machine's own tool and never builds an index** (`D134`). It probes for
`rg`, then `ag`, then `grep`, once at process start, and translates a query and its filter into that
tool's own flags — argv, never a shell string — streaming the results as exactly the messages the
local search worker sends, batched the same way and under the same ceilings, so the interface needs
no branch for a drone's answer. Cancelling kills the child.

**File scope is the only scope a drone serves.** Tasks, chats and the knowledge base are the
coordinator's own state; a drone answers an empty result rather than an error, because nothing
failed — there was simply nothing there that could have answered. A machine with none of the three
tools answers a walk error naming what was tried.

### What a drone refuses

A drone answers the pane family in full, the file family except diffing, host browse, project
listing, shell listing and search. **Everything else is refused with a typed answer** through a
catch-all arm, so a message added later is refused rather than dropped. Seven messages have no error
variant to refuse with — the stats listing, the six notifications and the command-line shortcut —
and those log a warning and drop.

### The surfaces

- **The connect modal** carries a carrier pill: a socket body, or a profile chosen from the saved
  list, a folder, and a lifetime preset. A Check control sits under the profile picker.
- **The SSH profiles settings section** is a row list, a form modal that holds no typed values of
  its own, and a delete confirm. Saving mints the id and writes the list; the secret box is sent
  only when it was typed into, is blanked either way, and is never prefilled from storage.
- **The Drones settings section** lists, per saved SSH-carrier host, what is running there — roots,
  uptime, pane count, version and linger — with a per-host Refresh, a per-host Check, and a per-drone
  Stop behind a danger confirm, since stopping kills every pane the drone holds. Version skew is
  flagged on the row rather than hidden: a managed drone outlives the Ubiq that deployed it, and the
  one remedy is stop, redeploy, reattach. **A drone is never hot-swapped under live panes.**
- **The project dialog's Remote panel** edits the project's origin — runs here, or on a drone with a
  profile, a folder and a preset — and a form agreeing with the record writes nothing.

**The Check control answers a question neither a listing nor a dial can:** whether `ssh` works and
nothing answers to the drone binary, versus `ssh` never getting through at all. It chains this
build's own version flag with the drone's probe line in one remote command and reads the exit status
itself, rather than folding a missing binary into the same refusal every other remote command
returns. The dialogs these sections sit in are [`workbench.md`](./workbench.md)'s.

## Contract

Every message a drone speaks is an existing one, and
[`../tech/transport-contract.md`](../tech/transport-contract.md) owns all of them:

- **The carrier family** — `DroneHello`, `DroneReady`, `Ping`, `Pong` — the one part of the contract
  no dispatch ever sees (`D117`). The pumps at each end read, write and swallow them there. They are
  ordinary frames rather than a preamble because the stream is length-prefixed MessagePack, and a
  second encoding would need its own ceiling, truncation rule and decode errors.
- **`MESSAGE_SCHEMA`**, a `u32` beside the frame ceiling in `crates/ubiq-proto/src/wire.rs`.
- **`ProjectRecord.runs_on`**, an optional `DroneOrigin`, carried on an update as a three-state
  `DroneChange` because an `Option<Option<_>>` cannot cross the wire. It is purely additive, so the
  catalogue version does not move.
- **`SavedRemoteHost.carrier`**, its `DronePreset` and its remembered `drone_path`, in
  `crates/ubiq-proto/src/settings.rs` beside `SshProfile` and its authentication choice.
- **The search family**, answered by a drone exactly as the local worker answers it.
- **`HostInfo`**, which the relay greets each attaching client with.

## Implementation

The crate and its feature split are [`../tech/project-structure.md`](../tech/project-structure.md)'s;
the lean host build and the multi-host bus are
[`../tech/architecture.md`](../tech/architecture.md)'s.

**On the far machine.** `crates/ubiq-drone/src/main.rs` hand-rolls its argv; `relay.rs` is the run
loop that stands in for the coordinator at a fraction of its surface — the pty map with its owners
and focus, a file worker, a machine-facts sampler and a seeded catalogue held as a plain vector,
since a relay has no write half to it. Standard output is flushed on every write, because a
length-prefixed binary frame carries no newline and an unflushed answer would leave the drone
looking hung; logging goes to standard error, which the frames do not own. The config root and
workarea are a per-pid folder under the temporary directory. `socket.rs` holds the listen, attach
and stop paths and the multiplexer that only holds the process, `linger.rs` the timer, `state.rs`
the state file with its list-and-prune, `search.rs` the one-shot execs, `scrollback.rs` the ring —
which needs a sink that outlives a link, so a private hub with one client held for the life of the
process sits between the pty and the ring. `lib.rs` holds `capabilities()` and `probe_line()`.

**The session pump exists twice and its rules once.** `ubiq_host::carrier::pump()` and
`spawn_pump()` in `crates/ubiq/src/app/remote_connect.rs` are two threads, because `crates/ubiq`
does not depend on `crates/ubiq-host` and `just ui` enforces it. Duplicating the thread shape is
inherent to that boundary; duplicating the *rules* would let the two ends drift about how long
silence may last, so the handshake helpers and the heartbeat state machine sit in
`crates/ubiq-proto/src/carrier.rs` and both pumps call them. A carrier supplies one thing beyond its
two halves — a closer that makes a **blocked** read on the other half return, since dropping a write
handle does not, and without it the host never sees the peer go and never reaps the panes.

**On the interface's side.** `crates/ubiq/src/app/ssh_connect.rs` owns every reach of a far machine:
`dial_ssh()` and `dial_ssh_reporting()`, `ensure_drone()` and its reporting twin, `check_host()`,
`list_drones()` and `stop_drone()`. All of them build their argv through one builder and run off the
main thread. The child, its pipes, its stderr drain and a watchdog live in that module, and nothing
below it sees a descriptor. `crates/ubiq-proto/src/drone/` holds the manifest, the `uname` map and
the verifier — in the protocol crate rather than the host's, because the interface shells to `ssh`
itself and cannot name the host crate; the host re-exports it under the old name.

`Bus::row_owner()` in `crates/ubiq/src/app/hosts.rs` is the duplicate-row rule, read by project
listing, wholesale replacement and host drop alike; `disconnect_host()` reads the rows a lost host
served before dropping it, and marks each survivor unreadable.
`AppState::ensure_drone()` in `crates/ubiq/src/app/projects.rs` is the launch-or-adopt path for a
project with an origin, and `project_pinned_to()` is how the reconnect loop and the connect modal
find the pinned id. Reconnect dispatches on carrier: a socket host dials with its token, an
SSH carrier re-runs the dial with the saved profile, root and preset — cheap and non-destructive,
because the launch line adopts. Only the **Attached** preset has nothing to reattach to.

The askpass helper mode lives in `crates/ubiq-app/src/lib.rs`, the one crate that names both halves.
The drawing sits in `crates/ubiq/src/ui/remote_connect.rs` (the carrier pill and the per-step note),
`crates/ubiq/src/ui/settings.rs` (the SSH profiles and Drones sections) and
`crates/ubiq/src/ui/sink/project.rs` with its state in `crates/ubiq/src/state/sink.rs` (the Remote
panel). `_tools/drone.py` generates and verifies the manifest.

## Failure

- **A schema mismatch** is refused at the handshake, before a pseudo-terminal exists. The modal
  shows the drone's own sentence.
- **The remote binary is missing.** The far shell's exit 127 is what turns a dial into a deploy; a
  triple the manifest cannot serve is a refusal naming the triple and the build command.
- **`ssh` fails.** Failures reuse the connect modal's existing vocabulary: exit 255 or no status is
  unreachable with the tail of stderr, and any other non-zero is a refusal.
- **The link drops.** Attached ends; Session and Managed fall into the reconnect loop, whose fresh
  start-or-adopt line finds the same drone and replays each pane's ring.
- **The linger expires.** The drone kills its panes and exits. What the user sees is a host
  disappearing, and a Session host's reconnect loop starts a *fresh* drone with none of the old
  one's panes — recorded as a gap in [`../backlog.md`](../backlog.md) rather than solved here.
- **No search tool on the far machine.** `search` is left out of the advertised set and a query
  answers a walk error naming what was tried.
- **Diffing a file on a drone** is refused: the git capability it would sit behind is advertised by
  nothing, because cross-compiling `git2` per triple is unstarted.
- **A version skew** is flagged on the Drones row. The schema number is what actually catches a
  disagreement at the handshake.

## Related docs

- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the carrier family, the schema
  number, `runs_on`, the saved host's carrier, and the search family a drone answers
- [`../tech/architecture.md`](../tech/architecture.md) — the lean host, the multi-host bus, and the
  rule that the UI never assumes the pseudo-terminal is local
- [`../tech/project-structure.md`](../tech/project-structure.md) — the crate, its modules and the
  host's feature split
- [`../tech/operations.md`](../tech/operations.md) — the lean-build check, the cross build, the
  manifest and its verifier
- [`../tech/decisions.md`](../tech/decisions.md) — `D116`–`D118`, `D124`, `D125` and `D131`–`D139`
- [`workbench.md`](./workbench.md) — the settings dialog and the connect modal these sections sit in
- [`../wip/drone.md`](../wip/drone.md) — the drone as a tool an agent can reach: designed, unbuilt
- [`../backlog.md`](../backlog.md) — `G108`, `G149`, `G256`–`G258` and `G261`–`G268`

## Next steps

- Run the SSH paths against a real `sshd` — the argv, the askpass handshake, adoption over the wire,
  and the deployer's four steps are verified over their inputs and over nothing else.
- Produce one cross-built binary so the manifest carries an entry and the deployer can put a binary
  on a far machine.
- Say that a lifetime ran out, rather than reporting it as a link that dropped.
- Give the seven refusable-but-silent messages an error variant to be refused with.
- Advertise a `git` capability, and serve file diffing behind it.
- Design the runtime build — `agent-manager` on the far side, for WSL and containers.
