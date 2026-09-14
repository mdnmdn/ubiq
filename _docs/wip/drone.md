---
id: wip-drone
title: The drone, phase by phase
kind: wip
status: current
summary: The running state of the drone's nine phases — one small executable Ubiq places on another machine to serve its terminal, files and machine facts over a single duplex byte stream. Phases 1 to 6 are built; the interface dials one over ssh and its panes survive a dropped link, though nothing is yet proven against a real ssh. Adoption, per-project configuration and deployment are not built. This document is the ledger, updated as each phase lands.
read_when: you are picking up the drone work and need to know what is built, what is next, and which decisions are already settled
updated: 2026-09-14
verified: 2026-09-14
code_anchors: [crates/ubiq-drone/src/main.rs, crates/ubiq-drone/src/lib.rs, crates/ubiq-drone/src/relay.rs, crates/ubiq-drone/src/carrier.rs, crates/ubiq-drone/tests/session.rs, crates/ubiq-drone/tests/handshake.rs, crates/ubiq-host/src/carrier.rs, crates/ubiq-host/src/lib.rs, crates/ubiq-host/Cargo.toml, crates/ubiq-proto/src/carrier.rs, crates/ubiq-proto/src/wire.rs, crates/ubiq-proto/src/settings.rs, crates/ubiq-host/src/connectors/store.rs, crates/ubiq-host/src/settings.rs, crates/ubiq/src/app/remote_connect.rs, crates/ubiq/src/app/settings.rs, crates/ubiq/src/ui/settings.rs, crates/ubiq/src/app/ssh_connect.rs, crates/ubiq/src/ui/remote_connect.rs, crates/ubiq/src/state/remote.rs, crates/ubiq-app/src/lib.rs, crates/ubiq-drone/src/linger.rs, crates/ubiq-drone/src/scrollback.rs, crates/ubiq-drone/src/socket.rs, crates/ubiq-drone/tests/detach.rs]
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
| 4 | SSH connection profiles and their secrets | **Built** |
| 5 | The interface attaches over SSH | **Built** |
| 6 | Detach and linger | **Built** |
| 7 | Managed drones | Next |
| 8 | Per-project configuration | Not started |
| 9 | Provenance, then git and search | Not started |

Phase 5 is the first a user can reach; everything before it is drivable only by hand and by test.
Nothing is proven against a real `ssh` yet — see the gaps.

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

### 4 — SSH connection profiles and their secrets *(built)*

Built before the drone needs it, because three things want it and none of them is the drone alone:
the drone's carrier, a remote `ubiq-host` reached over an ssh tunnel, and `G149`.

**`SshProfile { id, name, host, port, user, auth }` in `HostSettings.ssh_profiles`**, where `auth`
is `Agent | KeyFile { path, has_passphrase } | Password { has_password } | ConfigAlias` — the last
deferring wholly to `~/.ssh/config`, its host field an alias and every other field ignored, which
the form says by hiding them. A path is a reference and rides the record; a passphrase never does.
`HOST_SETTINGS_SCHEMA` goes to 16 on `ai_providers`' footing rather than `tools`': an older build
dropping these rows strands their passphrases under ids nothing on disk names, and a profile is
re-typable where a stranded secret is not.

**The list is the interface's and the material is the host's** (`D124`). The profiles ride
`SetSettings` whole, in the ownership class `remote_hosts` is in, because nothing writes one
unattended. The passphrase or password crosses only in a `Secret` — `SetSshSecret` and
`ClearSshSecret` (`D65`) — into a fourth namespace of the connector `Store`, `harness: "ssh"` and
`name: <profile id>`, beside `connector:`, `connector-app` and `ai-provider`, inheriting its
`usable()` probe. Both answer with the whole host-layer record rather than an acknowledgement,
because the `has_*` flag moves with the secret and the interface draws that flag.

**The flags are re-stamped, never believed.** Every host-layer write prunes the store to the ids the
list still names and whose auth still takes a secret, then reads each flag back — closing the hole
the ownership split opens, a profile deleted or switched to `Agent` leaving its material filed under
an id nothing names. The rule is a pure `stale_ssh_secrets` in `settings.rs` and the coordinator
applies it with the `Store` the connector modules already share, so `settings.rs` gains no keychain
dependency and the rule is testable where there is no secret service. The cost is a keychain read
per profile per write; the record is rebroadcast only when a flag moved. A refusal names the wrong
thing — no such profile, or an auth method that holds none — *before* asking whether the keychain
is usable, so it reads the same on a machine that has none. The drone refuses both on the host
settings layer rather than joining the seven it can only log and drop: that set exists because
those have no error variant, and these have one.

The interface grows an **SSH profiles** section, following `ai_providers` function for function — a
row list, a form modal holding no typed values of its own, and a delete confirm. A row states the
method and whether a secret is filed, and offers Forget secret only when one is. Saving mints the id
UI-side, writes the list through the ordinary `SetSettings` path, and sends `SetSshSecret` only if
the box was typed into; the box is blanked either way and a secret is never sent back to be
prefilled. The nav entry borrows `IconName::Network` rather than a drawn icon.

**What is designed and not built is the delivery.** OpenSSH reads a password from `/dev/tty`, not
stdin, so a shelled-out `ssh` cannot be fed one; the answer is an askpass helper — `SSH_ASKPASS`
pointing at Ubiq's own binary with `SSH_ASKPASS_REQUIRE=force` and no controlling terminal, which
keeps the material out of `argv`, out of any file, and off the bus outbound. Nothing dials `ssh`
yet, so there is no process to set those variables on and no test that could exercise it. It lands
with phase 5, which is what gives it a caller. `G258`.

### 5 — The interface attaches over SSH *(built)*

The first phase a user can reach.

**`crates/ubiq/src/app/ssh_connect.rs`** spawns the system `ssh` with piped stdio, runs
`ubiq_proto::carrier::welcome` on the bare pipes — Ubiq's half of the handshake, which until now
only tests called — and hands the two halves to `spawn_pump`. That pump became generic over
`R: Read`/`W: Write` with its closer a `Box<dyn FnOnce() + Send>` instead of a `TcpStream`: a pipe
pair is not one handle that both reads and writes, and each carrier spells "unblock the reader"
differently. Ubiq shells to `ssh` and never parses its output for structure — what comes back is an
exit code and bytes, which is what separates this from `D43`. The child, its pipes, its stderr drain
and a 45-second watchdog live in that module and nothing below it sees a descriptor.

The argv is built from the profile: `-T`, a connect timeout, `-p` and `-l` when they are the user's,
and for `ConfigAlias` none of them — the point of that variant is that `~/.ssh/config` governs.
`BatchMode=yes` only when the profile has no secret filed, because batch mode would refuse the
askpass helper. The remote command is `ubiq-drone --stdio`, a bare name resolved by the remote
`PATH`, plus `--root` when the saved root is not empty. There is no path setting yet: phase 9 is
what deploys a drone and caches where it put it, so there is nothing for one to point at.

**Failures reuse the existing vocabulary**, so the modal needed no new kinds. A handshake refusal is
`Refused` carrying the drone's sentence verbatim — which names both schema numbers — and is the
first time a refusal is shown to anyone rather than logged and exited. Exit 255 or no status is
`Unreachable` with the tail of stderr, ssh's own class; any other non-zero is `Refused`, which is
where `ubiq-drone: not found` lands.

**The secret reaches `ssh` without passing through the interface** (`D125`). `SSH_ASKPASS` points at
Ubiq's own binary with `SSH_ASKPASS_REQUIRE=force`, and the child's environment carries a profile id
and a config root — references, not material. The helper mode is in `ubiq-app`, the one crate that
names both halves, so it opens the host's store itself and writes the secret on its own standard
output, where OpenSSH reads an askpass answer from. The material's whole path is keychain to helper
stdout to `ssh`: never `argv`, never a file, never the bus, never the drawing process. That closes
the question phase 4 left open, and it closes it better than the message that was the alternative.

The connect modal grows a mode pill: a socket body as before, or a profile chosen from
`ssh_profiles` and a folder. `SavedRemoteHost` grew `carrier` for it, at `HOST_SETTINGS_SCHEMA` 17.

`HostEntry` and `host_menu_rows` needed nothing, as the design said — but the *pick* path did.
`reconnect_saved_host` filled an address and started a socket dial unconditionally, and
`remote_socket_lost` would have started a TCP reconnect loop for a dropped drone. Both are now
carrier-aware. A drone is a host (`D116`); what was not carrier-agnostic was the dialling, not the
listing.

### 6 — Detach and linger *(built)*

`--listen [<sock>]` binds a unix socket at mode `0600` and detaches; `--attach [<sock>]` is a pure
byte relay between its own stdio and that socket, so reattaching is a fresh
`ssh <target> <drone> --attach <sock>`. Filesystem permissions are the whole of the access control —
no port is forwarded, which is why the shelved proposal's exec-time keypair defends nothing. The
path is derived from a hash of the canonicalised roots under `$XDG_RUNTIME_DIR`, falling back to the
cache directory, so two windows cannot start rival drones for one project. **The containing
directory is forced to `0700` first**, because that is what closes the window between `bind` and the
socket's own chmod. A path that still accepts a connection is refused; a dead one is removed.

**Not `tmux attach`:** tmux is a terminal emulator and would mangle binary frames. A multiplexer only
holds the detached process where a user can read its log — `tmux new -d`, else `screen -dmS`, else
`setsid`, else a bare spawn that says it will die with the login. The persistence is the socket,
never the multiplexer. `--listen` is the launcher and `--foreground` is what the held process runs.

`--linger` is the single knob: `0` exits when the last client detaches, `N` (default 10 minutes)
survives a dropped link, `never` is a managed drone. It is the *drone's* timer, because the whole
point is that Ubiq may be gone, and it is **re-asserted on each attach** — over one ASCII line the
attaching process writes before the handshake, which is the drone's own two halves talking over a
socket only they can open, and so not a protocol change. A **30-second startup grace** covers the
case the knob alone gets wrong: `--linger 0` would otherwise exit in the gap before the first client
arrives. After the first client the linger rules exactly. On expiry the drone kills its panes and
exits, which is destructive and has to be visible in the connect UI and on the host row — it is not
yet.

This phase is why `D118` exists: without the heartbeat a detached drone never observes its client
leaving, never starts its countdown, and lives forever by accident.

A detached drone holds live pseudo-terminals, their ids, the owners map and a **bounded per-pane
scrollback ring** (256 KiB, oldest first) — and nothing else: no catalogue on disk, no index, no
credentials. The ring needed a sink that outlives a link: `Pty::forward_output` stops when its sink
reports the client gone, which is right for a closed window and wrong for a dropped one, so a
private hub with one client held for the life of the process sits between them. Rings are kept only
when the drone is detached.

**Reattach is a new host attach, not a resumed one.** `Bus::drop_remote` keeps today's behaviour of
taking a lost host's panes with it, and on the new connection the drone re-announces each live pane
with `WorkspaceSpawned` and replays its ring as `TerminalOutput`. Pane ids are minted by the drone
and stay stable, so it is the same pane; without the ring the user would get a live-but-blank
terminal until the next output. `D22` still holds: closing a *pane* kills its harness, and what a
dropped link ends is the link.

Four tests in `crates/ubiq-drone/tests/detach.rs` drive the real binary over the real socket: a
detached drone holds its panes across a dropped link and the reattached pane is proven live by
writing a file; the ring is bounded and keeps the newest; `--linger 0` takes the process and its
socket with the last client; an attach's re-asserted linger overrides a `never` the launch set.

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
cargo test -p ubiq-drone       # 14: unit (5), detach (4), handshake (3), session (2)
cargo test -p ubiq-host --test settings --test connectors   # the ssh namespace and its reconcile
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
- **`D124` — an SSH profile is the interface's record and its secret is the host's**, which is what
  obliges the host to prune and re-stamp on every write.
- **`D125` — the askpass helper reads the keychain itself**, so a password reaches `ssh` without
  ever entering the drawing process.
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
- **Nothing has been run against a real `ssh`.** There is no ssh client and no `sshd` in the
  sandbox this was built in, so the ssh path — the argv, the askpass handshake, the drone found on
  the far `PATH` — is verified by unit tests over its *inputs* and by nothing over its behaviour.
  That is the single largest untested surface in the drone, and phases 4, 5 and 6 all rest on it.
  `G258` is the fixture that would close the askpass half.
- **A drone that lingers out is invisible.** Expiry kills its panes and exits, which is destructive
  and must show in the connect UI and on the host row; nothing draws it. A user whose laptop slept
  past the linger finds the panes gone with no account of why.
- **`ubiq-drone` is a bare name on the remote `PATH`.** Nothing deploys one and nothing says where
  it is, so phase 5 works only where a drone has been installed by hand. Phase 9 is the deployer;
  until then `ubiq-drone: not found` is the common first failure, and it is reported as a refusal
  with ssh's own words.
- A refusal is a sentence on the wire and a non-zero exit; **no modal shows it**, because there is
  no connect flow for a drone to fail in yet. Also phase 5.
- `capabilities` is advertised and nothing reads it. The interface has no reason to branch on one
  until it can attach a drone at all.
- The runtime build — `agent-manager` on the far side, for WSL and containers — is designed nowhere.

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the lean host, and the rule that the UI never assumes the pseudo-terminal is local
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the framing a carrier honours, the file family's seam, and [`../tech/project-structure.md`](../tech/project-structure.md) for the crate's feature split
- [`../backlog.md`](../backlog.md) — `G108`, `G149`, `G188`, `G189`, `G256`, `G257`, `G258`; and `D116`–`D118` and `D124` in [`../tech/decisions.md`](../tech/decisions.md)
