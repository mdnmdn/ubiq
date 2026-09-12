---
id: wip-drone
title: The drone, phase by phase
kind: wip
status: current
summary: The running state of the drone's nine phases — one small executable Ubiq places on another machine to serve its terminal, files and machine facts over a single duplex byte stream. Phases 1 to 3 are built and verified; the SSH connection store, the interface, the lifecycle and the deployment are not. This document is the ledger, updated as each phase lands.
read_when: you are picking up the drone work and need to know what is built, what is next, and which decisions are already settled
updated: 2026-09-12
verified: 2026-09-12
code_anchors: [crates/ubiq-drone/src/main.rs, crates/ubiq-drone/src/lib.rs, crates/ubiq-drone/src/relay.rs, crates/ubiq-drone/src/carrier.rs, crates/ubiq-drone/tests/session.rs, crates/ubiq-drone/tests/handshake.rs, crates/ubiq-host/src/carrier.rs, crates/ubiq-host/src/lib.rs, crates/ubiq-host/Cargo.toml, crates/ubiq-proto/src/carrier.rs, crates/ubiq-proto/src/wire.rs, crates/ubiq/src/app/remote_connect.rs]
depends_on: [tech-architecture, tech-transport, tech-structure, inbox-drone-runtime]
---

# The drone, phase by phase

A **drone** is one small executable Ubiq places where it is not running, serving that machine's
terminal, files and machine facts over a single duplex byte stream. It is built twice from one
crate: a **relay** build with no agent knowledge at all, and — later, and not designed here — a
**runtime** build that links `agent-manager` to compose and confine harness runs inside WSL or a
container.

The design is [`../inbox/drone-proposal-v2.md`](../inbox/drone-proposal-v2.md), whose §1 is unfinished,
and the shelved [`../inbox/backlog/remote-drone-proposal.md`](../inbox/backlog/remote-drone-proposal.md) it keeps most of.
**This document is self-contained: every phase below carries the detail needed to build it.**

## The shape

A drone attaches to a window as **an ordinary host on the existing multi-host bus** (`D81`) — not a
proxy the local coordinator routes through, and not a new message family. The interface reaches it
exactly as it reaches a `ubiq --serve` host, which is what lets `Bus::route_host`,
`projects_not_on` and `drop_remote` carry it unchanged.

This departs from the shelved proposal's §3, which put a `remote: Option<RemoteOrigin>` field on
`ProjectRecord` and routed that project's messages through the local coordinator.

## Phases

| # | Phase | State |
|---|---|---|
| 1 | The lean host build | **Built** |
| 2 | `crates/ubiq-drone` and the stdio carrier | **Built** |
| 3 | Handshake, schema version, heartbeat | **Built** |
| 4 | SSH connection profiles and their secrets | Next |
| 5 | The interface attaches over SSH | Not started |
| 6 | Detach and linger | Not started |
| 7 | Managed drones | Not started |
| 8 | Per-project configuration | Not started |
| 9 | Provenance, then git and search | Not started |

Phase 5 is the first a user can reach; everything before it is drivable only by hand and by test.

### 1 — The lean host build *(built)*

`crates/ubiq-host` gates five features — `git`, `index`, `harness`, `listener`, `desktop` — with
`full` all five and `default = ["full"]`, so an ordinary build is unchanged. `coordinator` needs
`full` and is never compiled into a drone: gating a 5480-line struct would scatter `cfg` through the
largest file in the tree for nothing. What a drone links instead is the self-contained modules under
it, which already take a `Job` and a `Mailbox`.

`links` is deliberately **not** gated: it is a byte scanner with no dependency of its own, so gating
it would fork `pty`'s read loop to save nothing.

`just relay` builds the lean configuration and fails if any gated crate reaches its tree. It is part
of `just verify`. Measured: 205 dependency-tree lines against the full build's 614.

An integration test naming a gated module carries `required-features` in `Cargo.toml`; two collector
tests in `tests/projects.rs` are gated inline, so the other 27 still cover `projects` leanly.

### 2 — `crates/ubiq-drone` and the stdio carrier *(built)*

**`ubiq_host::carrier`** holds the session pump, moved out of `remote.rs` because that module is the
TCP listener and is gated behind `listener`. The pump needs neither a socket nor TLS. In its place
`remote.rs` keeps a `SocketCloser`, and the `closer: TcpStream` parameter became a `Closer` trait.

`Closer` is the one thing a carrier must supply beyond its two halves: the writer calls it on its
way out and it must make a **blocked** read on the other half return. Dropping the writer's handle
does not — a socket sends no EOF while the reader holds its clone — and without it the host never
sees `Gone` and never reaps the panes. `NoCloser` is sound for the ssh case only because the dead
`ssh` that failed the write has already closed the stdin the reader is sitting on.

**`crates/ubiq-drone`** is a binary plus a thin lib. `--stdio` (the default) serves one session on
standard input and output; `--root` is repeatable and seeds the in-memory catalogue; `--probe`
prints `os=… arch=… triplet=…` for a future deployer; `--version` names the build a handshake will
check. Argv is hand-rolled, following `ubiq-app`'s `serve_bind`.

`relay.rs` is ~600 lines against the coordinator's 5480. It holds the pty map with its owners and
focus, a `files::Files` worker, a `host_meta` sampler and the seeded catalogue, and greets each
attaching client with `HostInfo` exactly as `Coordinator::client_here` does.

**It answers** the pane family in full, the file family except `Diff`, `BrowseHostDir`,
`ListProjects` and `ListShells`. **It refuses everything else with a typed answer** through a
catch-all arm, so a message added later is refused rather than dropped — conversation, account,
quota, connector, repository, git, work, search, assist, web-asset and catalogue-write families each
answer their own error variant. Seven messages have no error variant to refuse with (`ListStats`,
the six notification messages and `CliShortcut`); those log a warning and drop, and are the only
silences.

**Standard output is flushed on every write.** `wire::write_frame` does not flush, and `io::Stdout`
buffers by *line* — a length-prefixed binary frame carries no newline, so an unflushed answer would
sit in the buffer and the drone would appear to hang.

The catalogue is a plain `Vec<ProjectRecord>` rather than `MemoryProjectStore`: a relay drone has no
write half to the catalogue, so nothing would ever call `upsert`. The config root and workarea are
`$TMPDIR/ubiq-drone-<pid>`, created lazily because `host_meta` needs a path to sample free space
against; nothing durable is written to the far machine. Logging goes to stderr, never stdout, which
the frames own.

Two tests drive a real session over a loopback pair and the real framing. One spawns a shell, echoes
through it, resizes to 132×43 and confirms with `stty size` — the domain rule that a resize is
incomplete until the harness knows — then asserts a refused family answers rather than going silent.
The other has a pane touch a marker file in a loop, ends the stream, and asserts the marker stops
reappearing: the orphaned-pty path, proved against the process rather than against a message.

### 3 — Handshake, schema version, heartbeat *(built)*

**Four variants, one new family.** `DroneHello`, `DroneReady`, `Ping` and `Pong` are the transport
contract's *carrier family*, the only messages no dispatch ever sees: the pumps at each end read and
write them and swallow them there, on the same standing `remote.rs`'s HTTP upgrade has. They are
ordinary frames rather than a preamble because the stream is already length-prefixed MessagePack —
a second encoding on it would need its own ceiling, its own truncation rule and its own decode
errors, all to save two variants on an enum that carries a hundred and fifty.

**The rules live in `ubiq_proto::carrier`, the threads do not.** There are two pumps and there
always will be: `ubiq_host::carrier::pump` and `spawn_pump` in `crates/ubiq/src/app/remote_connect.rs`,
because `crates/ubiq` does not depend on `crates/ubiq-host` and `just ui` enforces it. Duplicating
the thread shape is inherent to that boundary; duplicating the *rules* would let the two ends drift
about how long silence is allowed to last, so the handshake helpers and the `Heartbeat` state
machine are in the protocol crate and both pumps call them.

**`MESSAGE_SCHEMA` is a `u32` beside `MAX_FRAME`, starting at 1.** Its doc names exactly what a
drone speaks — the pane family, the file family, the host browse family, the read half of the
project family, `ListShells` and `HostInfo` — because those are the only messages an old drone and a
new Ubiq can disagree about; every other family a drone refuses outright, so their shape cannot
matter. A mismatch is answered with `accepted: false` and a sentence a modal shows, and both sides
close cleanly. Nothing enforces the bump mechanically, which is stated in the doc comment rather
than implied: it is a discipline, and what it buys when honoured is a sentence at connect time
instead of a decode failure mid-session.

**The handshake happens before the drone has anything to clean up.** `ubiq-drone` writes its hello
and waits for the ready on the bare stream, ahead of the hub, the relay thread and the pump, so a
refusal is logged to standard error and exits non-zero having spawned no pseudo-terminal and touched
nothing on the far machine. By the time a relay exists it is too late to have not started one. A
relay build advertises `capabilities` of `files` and nothing else — a string set, so the list grows
without a schema bump, and naming a capability it would only refuse would have the interface offer
the user something that fails.

**The heartbeat is a message pair, not a frame type.** A frame type below the message would change
`[u32 length][msgpack]` for every build that already exists; two variants cost nothing by
comparison, which is what settles `G189`'s open question. Either end pings and both answer, because
either end can be the one that goes quiet. A ping is owed only after 20 seconds of complete inbound
silence — so a session with a live pane in it puts no extra frame on the wire, and a dead stream
under a busy pane is still found by the next failed write — and three unanswered pings, about a
minute, is gone. Twenty seconds sits inside the shortest NAT idle timeouts that are common in
practice, which makes it a keepalive as well as a detector; a minute of tolerance survives a machine
paging back in without leaving a dead session lingering past the point the user has noticed. A peer
that misses them is torn down by the path a dropped socket already takes, because a second shutdown
path is a second chance to leave a pane running.

The writer thread's existing `STOP_POLL` poll of `from_host` became a `flume::Selector` over that
receiver and a small channel of pongs the reader owes, so the reader never touches the write half
and a pong is not delayed by a poll interval. `is_gone` is checked at the top of each iteration
rather than only on timeout: an outbound direction that is busy while the inbound one is dead never
reaches the timeout arm.

Three tests in `crates/ubiq-drone/tests/handshake.rs` drive it over a real loopback pair and read
raw frames rather than attaching a client, because what is under test is which frames crossed and in
which order. A matching schema completes and serves; a mismatched one is refused with the reason
carried verbatim and the stream then ends with no `HostInfo`, which is the proof that no relay and
therefore no pane ever existed; a `Ping` comes back as a `Pong` with the same nonce and the session
behind it is undisturbed.

Registered as `D117` (the family's exception) and `D118` (the ping's numbers).

Closes `G189`, and opens `G256` and `G257` in its place. This phase was first written up as closing
`G109` too, which is wrong and is worth recording so nobody repeats it — those
row numbers came from the shelved proposal, which *proposed* `G107`–`G111` and never had them
allocated; the real `G109` is the search filter's missing interface, and nothing here touches it.

### 4 — SSH connection profiles and their secrets *(next)*

Built before the drone needs it, because three things want it and none of them is the drone alone:
the drone's carrier, a remote `ubiq-host` reached over an ssh tunnel, and `G149` — "an ssh clone
needs a credential callback, an agent socket and a key the host has no story for".

`SshProfile { id, name, host, port, user, auth }` in a new `HostSettings.ssh_profiles`, mutated by
the interface and riding `SetSettings` whole — the ownership class `remote_hosts` is in, not the
host-owned class `connections` is in, because nothing writes a profile unattended. `auth` is
`Agent | KeyFile { path, has_passphrase } | Password { has_password } | ConfigAlias`, the last
deferring wholly to the user's `~/.ssh/config`. A path is a reference and rides the record; a
passphrase never does, and `has_*` is derived from the secret store rather than stored, as
`OauthApp.has_secret` is.

The material goes in a fourth namespace of `crates/ubiq-host/src/connectors/store.rs`'s existing
`Store` — `harness: "ssh"`, `name: <profile id>` — beside `connector:`, `connector-app` and
`ai-provider`, inheriting its `usable()` probe. It crosses inbound in a `Secret` and only in a
`Secret` (`D65`).

Delivery is the sharp part: OpenSSH reads passwords from `/dev/tty`, not stdin, so a shelled-out
`ssh` cannot be fed one. An askpass helper is the answer — `SSH_ASKPASS` pointing at Ubiq's own
binary with `SSH_ASKPASS_REQUIRE=force` and no controlling terminal — which keeps the material out
of `argv`, out of any file, and off the bus outbound entirely.

### 5 — The interface attaches over SSH *(not started)*

The first phase a user can reach. Depends on phase 4 for the profile it dials with.

A new `ssh_connect` module under `crates/ubiq/src/app/` spawns `ssh` with piped stdio and hands the child's
standard input and output to `bus::detached()` behind the shape `spawn_pump` in
`crates/ubiq/src/app/remote_connect.rs` already uses. It calls `ubiq_proto::carrier::welcome` —
Ubiq's half of the handshake, which until this phase only tests call — before the pump starts, and
maps a non-zero exit plus the child's standard error onto the existing `ConnectFailure` vocabulary
so the modal needs no new failure kinds. Ubiq shells to the system `ssh` and never parses its output
for structure: what comes back is an exit code and bytes, which is what separates this from `D43`.

The connect modal grows an SSH mode: `editing_body` in `crates/ubiq/src/ui/remote_connect.rs`, with
the state on `RemoteConnectState` in `crates/ubiq/src/state/remote.rs` and setters following
`set_remote_scheme` / `set_remote_trust` in `crates/ubiq/src/app/remote_connect.rs`. It collects a
profile and a remote root instead of an address and a token. `SavedRemoteHost` in
`crates/ubiq-proto/src/settings.rs` grows a carrier discriminant beside `RemoteScheme`, which is a
`HOST_SETTINGS_SCHEMA` bump on the protocol that file documents.

`HostEntry` and `host_menu_rows` in `crates/ubiq/src/app/hosts.rs` need nothing: a drone is a host
(`D116`), so the Hosts section, the picker and Disconnect carry it already. Architecture rule 2
holds — the child process handle lives in `ssh_connect.rs` beside the `TcpStream` that
`architecture.md` names as the sanctioned exception, and no descriptor reaches a drawing module.

Verify by attaching to a drone over `ssh localhost`, opening a pane in it, and confirming the Hosts
section lists it and Disconnect tears it down.

### 6 — Detach and linger *(not started)*

`--listen <sock>` binds a unix socket at mode `0600` under `$XDG_RUNTIME_DIR`, falling back to the
cache directory, and detaches; `--attach <sock>` is a pure byte relay between its own stdio and that
socket, so reattaching is a fresh `ssh <target> <drone> --attach <sock>`. Filesystem permissions are
the whole of the access control — no port is forwarded, which is why the shelved proposal's
exec-time keypair defends nothing. The socket path is derived from a hash of the root so two windows
cannot start rival drones for one project.

**Not `tmux attach`:** tmux is a terminal emulator and would mangle binary frames. A multiplexer is
used only to hold the detached process where a user can read its log — `tmux new -d -s ubiq-drone
'<drone> --listen <sock>'` when tmux or screen is present, `setsid` otherwise. The persistence is the
socket, never the multiplexer.

`--linger` is the single knob: `0` exits when the last client detaches, `N` (default 10 minutes)
survives a dropped link, `never` is a managed drone. It is the *drone's* timer, not Ubiq's, because
the whole point is that Ubiq may be gone; it is passed at launch and re-asserted on each attach, so
changing it needs no restart. On expiry the drone kills its panes and exits, which is destructive
and must be visible in the connect UI and on the host row.

This phase is why `D118` exists: without the heartbeat a detached drone never observes its client
leaving, never starts its countdown, and lives forever by accident.

A detached drone holds live pseudo-terminals, their ids, the owners map, and a bounded per-pane
scrollback ring (256 KiB, oldest first) — and nothing else: no catalogue on disk, no index, no
credentials. **Reattach is a new host attach, not a resumed one.** `Bus::drop_remote` keeps today's
behaviour of taking a lost host's panes with it, and on the new connection the drone re-announces
each live pane with `WorkspaceSpawned` and replays its ring as `TerminalOutput`. Pane ids are minted
by the drone and stay stable, so it is the same pane; without the ring the user would get a
live-but-blank terminal until the next output. `D22` still holds: closing a *pane* kills its
harness, and what a dropped link ends is the link.

### 7 — Managed drones *(not started)*

A state file beside the socket — pid, version, triple, started-at, root, pane count — so a restarted
Ubiq, or a second one, discovers and adopts a running drone instead of starting a rival. A pid that
is not alive means stale: remove the socket and the file and start fresh. `--list` prints running
drones as JSON, `--status` one, `--stop <sock>` closes its panes and exits.

A Drones panel in application settings lists what is running per configured target with uptime,
panes, version and linger, and a Stop. Version skew is expected rather than exceptional, because a
managed drone outlives the Ubiq that deployed it: `MESSAGE_SCHEMA` catches it at the handshake and
the remedy is one action — stop, redeploy, reattach. **A drone is never hot-swapped under live
panes.**

### 8 — Per-project configuration *(not started)*

`runs_on: Option<DroneOrigin>` on `ProjectRecord` in `crates/ubiq-proto/src/projects.rs`, where
`DroneOrigin { profile, root, preset, linger_secs }`. This field is pre-authorised:
`_docs/inbox/completed/project-handling-proposal.md` reserved, by name, "the field a remote drone
would need to say *where* the folder is". (Its neighbouring exclusion — a per-project default
harness, which is `agent-manager`'s — does not apply.)

It rides `UpdateProject`, and because it is genuinely three-state — say nothing, clear it, set it —
it copies the `IndexChange` pattern in the same file rather than nesting `Option<Option<_>>`.
`ProjectStore` is the right trait: the host acts on this, which rules out `PreferenceStore` (an
opaque blob the host must not read), and there is no per-project file to justify `TaskStore`.

Opening a project that names a drone launches or adopts it and attaches it as a host, passing the
locally-minted `ProjectId` in argv so the drone announces that root under the *same* id — one row in
the list, not two, its host attribution changing as the drone comes and goes.

**This is the one phase with real design risk, and `D116` is where it comes from.**
`Bus::projects_not_on`, `WindowRegistry::replace_all_except` and `Bus::drop_remote` all assume a host
owns its rows wholesale. A locally-owned record that a remote host *fills in* is a new case, and the
local record must survive the drone being down — drawn from `ProjectHealth::Unreadable` with the
reason — or the user loses a configured project every time their laptop sleeps. Budget for this
being most of the phase.

The interface gains a fifth `ProjectNav` arm, `Remote`, in `crates/ubiq/src/ui/sink/project.rs`
(the project-settings dialog lives in the kitchen-sink module, not a `project_settings.rs`) with the
variant in `crates/ubiq/src/state/sink.rs`. Its nav enablement rule — in `Form::Live` only General is
enabled, plus Tools when editing — has to learn the new arm.

### 9 — Provenance, then git and search *(not started)*

`ssh <target> uname -sm` probes the triple; a cache under Ubiq's config root keyed by version and
triple resolves the binary; `scp` uploads it to a throwaway path under the remote user's cache
directory; `ssh <target> <path> --stdio --root <root>` launches it. The first three are skipped when
the cached remote path reports the right `--version`.

A new `just drone-build` cross-builds per triple, and a new `drone.py` under `_tools/` hashes the
results into a generated `manifest` module under `crates/ubiq-host/src/drone/` — the shape
`_tools/webassets.py` uses for the
vendor mirror, sha256 per entry in generated Rust with a `verify` mode for drift. That is
**hash-pinned, not signed**: there is no signing key and no release pipeline (`G108`), and the docs
must say so rather than imply provenance the tree lacks. What ships without a pipeline is a locally
cross-built binary, a `--drone-path`, and the cache; what stays blocked is fetching a drone Ubiq did
not build.

Then `DiffProjectFile` behind the `git` capability — cross-compiling `git2` per triple is real work
and should not gate the rest — and `SearchProject` shelling to whatever `rg`, `ag` or `find` the
remote `PATH` has, advertised as the `search` capability. **A drone never builds an index:** it is a
guest on someone else's machine, and the resident cost an index implies is the thing this design
exists to avoid. That is a decision about shape, not a deferral.

## Working on this

```
cargo build -p ubiq-drone      # the drone
cargo test -p ubiq-drone       # 5 tests: session (2), handshake (3)
just relay                     # the lean host builds and stays lean — part of `just verify`
just host / just ui            # the crate-boundary guards
just verify                    # everything a change has to pass, the three above included
just docs-touched              # which documents your diff obliges you to update
```

**Four failures in this tree are pre-existing and are not yours.** Confirm against a clean tree
before chasing any of them:

- `crates/ubiq-proto/tests/work.rs` does not compile — `TaskRecord` is missing `key`/`kind`/`labels`
  and `UpdateTask` has no `shape`. Someone else's in-flight work; `AGENTS.md` says do not revert
  what you did not write.
- `foundation-models` fails its build script under a sandbox: it shells to `swiftc`, which cannot
  run. This breaks `cargo build -p ubiq-app`, `just check` and so `just verify`.
- `browse::the_default_request_answers_with_a_real_absolute_path` fails with
  `Denied("Operation not permitted")` — a sandbox refusing to list the home directory.
- `coordinator::opening_a_project_starts_its_filesystem_watch` times out after 20 seconds when run
  alone — a sandbox refusing filesystem-watch events.

The order that keeps this document true: code, then the register and the backlog, then this ledger
last. Written the other way round it records intentions rather than facts.

## Decisions already settled

Three are in the decision register, where they are argued and their costs stated; the rest hold for
phases that have not landed, and move there as each one does.

- **`D116` — a drone attaches as an ordinary host**, not as a project's remote origin, so the
  coordinator never learns one exists. Its cost is the shape of phase 8: a machine that is not
  reachable has no rows in the catalogue rather than unreachable ones.
- **`D117` — the carrier family is the one part of the contract no dispatch ever sees.**
- **`D118` — the heartbeat pings only silence**, 20 seconds and three misses.
- **The carrier is the ssh exec channel's stdio by default** — no forwarded port, which is why the
  shelved proposal's exec-time keypair defends nothing and is not built.
- **Persistence, when it lands, is a unix socket at mode `0600`**, never `tmux attach`: tmux is a
  terminal emulator and would mangle binary frames. A multiplexer only holds the detached process.
- **Lifecycle is two axes over one knob** — whether the process detaches, and `--linger` (`0`, `N`,
  `never`) for how long it waits with no client. Attached, Session and Managed are presets.
- **An ssh secret reaches `ssh` through an askpass helper**, never `argv`, a file, or the bus:
  OpenSSH reads passwords from `/dev/tty`, not stdin, so a shelled-out `ssh` cannot simply be fed one.
- **Provenance is hash-pinned, not signed.** There is no signing key and no release pipeline
  (`G108`); saying "signed" would imply provenance the tree does not have.
- **A drone never builds an index**, and a relay drone never runs a harness.

## Known gaps

- The refusal set has seven messages with no error variant to answer with; they log and drop, and
  `search` is refused though the lean host carries the worker — a scope call, cheap to revisit.
- Nothing verifies a `MESSAGE_SCHEMA` bump mechanically. The doc comment says so; no test compares
  the message set against a recorded shape, and a breaking change that nobody bumps for is found
  as a decode failure mid-session, which is the failure the number exists to replace. `G256`.
- **The heartbeat breaks a peer built before it.** `Ping` is an ordinary variant on a tagged enum,
  so a build that predates it cannot decode the frame: its read loop fails and the session ends,
  where before it would have run. The handshake closes this for a drone and closes nothing for
  `ubiq --serve`, which states no version at all — and `wire.rs` says a remote host and UI may be
  different builds, which this is the first change to make untrue. `G257`.
- **Nothing in the interface dials a drone yet**, so `ubiq_proto::carrier::welcome` — Ubiq's half of
  the handshake — is called only by tests. Phase 5 is what puts it on a real `ssh` channel, and
  until then the only proven pairing is a drone against a test's far end.
- A refusal is a sentence on the wire and a non-zero exit; **no modal shows it**, because there is
  no connect flow for a drone to fail in yet. Also phase 5.
- `capabilities` is advertised and nothing reads it. The interface has no reason to branch on one
  until it can attach a drone at all.
- The runtime build — `agent-manager` on the far side, for WSL and containers — is designed nowhere.

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the lean host, and the rule that the UI never assumes the pseudo-terminal is local
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the framing a carrier honours, the file family's seam, and [`../tech/project-structure.md`](../tech/project-structure.md) for the crate's feature split
- [`../backlog.md`](../backlog.md) — `G108`, `G149`, `G188`, `G189`, `G256`, `G257`; and `D116`–`D118` in [`../tech/decisions.md`](../tech/decisions.md)
