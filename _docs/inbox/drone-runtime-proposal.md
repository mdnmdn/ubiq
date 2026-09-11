---
id: inbox-drone-runtime
title: Proposal — the drone, a portable runtime Ubiq places on another machine
kind: proposal
status: proposal
summary: A small single-file executable Ubiq puts on a machine it is not running on — a Windows user's WSL distro first — which spawns pseudo-terminals, answers for files, describes its machine and composes confined harness runs through an embedded agent-manager, over one duplex byte stream carried by whatever already reaches that machine; it binds no socket, keeps no state, is fetched from a signed manifest and cached by triple, and dies with its channel because it has nothing worth resuming.
read_when: you are deciding how a terminal, a file or a harness reaches a machine Ubiq is not running on, what "drone" means in this tree, which carrier a remote link rides, or how a foreign-triple binary gets fetched and verified
updated: 2026-09-11
depends_on: [tech-architecture, tech-transport, tech-agent-manager, tech-structure, tech-decisions, feat-panes, inbox-isolation]
---

# Proposal — the drone, a portable runtime Ubiq places on another machine

Ubiq runs every harness on the machine that draws the window. That is wrong for the user this
proposal is written for: someone on Windows whose toolchains, repositories, shells and agent
harnesses all live inside a WSL distribution, on the other side of a kernel boundary, reachable only
by starting a process there. Their Claude Code is a Linux binary with a Linux config directory and a
Linux keychain, and no amount of pseudo-terminal work on the Windows side composes a run for it.

This proposes the **drone**: a single executable, built for that machine's target triple, fetched
from a manifest and cached, placed on the far side by whatever channel already reaches it, and
spoken to over one duplex byte stream. It spawns pseudo-terminals, answers for files, describes its
machine, and — this is the part that is new — **composes and confines harness runs there**, because
`crates/agent-manager` is the only thing that knows how, and a run can only be composed on the
machine whose filesystem it names.

There is a shelved proposal of the same name. §1 says what it settled, what this keeps, and the
three things the brief behind this document overturns.

## 1. Where this starts

[`backlog/remote-drone-proposal.md`](./backlog/remote-drone-proposal.md) designed a drone as *"a
coordinator's `pty/` and `files/` modules, cross-compiled to run alone on someone else's machine"*,
reached over an SSH port forward, reusing the existing pane and file message families unchanged. It
is filed under `backlog/` and it is right about most things. **This document keeps its skeleton and
overturns three of its load-bearing claims.**

| The shelved proposal says | This proposal says | Why |
|---|---|---|
| *"A drone never runs a harness, and that is permanent"* | A drone composes, confines and spawns harness runs through an embedded `agent-manager` | §6. The WSL case is unreachable otherwise — the harness, its config directory and its credentials are all on the far side |
| A drone reuses `Message` and gets no contract of its own | A drone speaks `DroneMessage`, a second and much smaller enum in the same wire crate | §5. The coordinator translates, so the UI contract is untouched and the drone never links four-fifths of a protocol it cannot answer |
| The transport is an SSH port forward, `-R` first, `-L` as fallback | The default carrier is the carrier process's own stdio; a listening socket is opt-in and a forward is one carrier among several | §3. A forwarded port is a socket every other local user on that machine can dial. A pipe is not |

Its other calls stand and are not re-argued here: shell out to the system `ssh` rather than embed an
SSH client (`D49`'s precedent); a remote project is a field on `ProjectRecord`, not a second kind of
project; no secret ever travels on `argv`; no index and no persistent state on the far side; a
dropped link is a teardown, not a background retry loop.

**What already exists, and is more than the shelved proposal assumed.**
`crates/ubiq-proto/src/wire.rs` frames a message as a four-byte big-endian length and a MessagePack
body, with `MAX_FRAME` at 64 MiB, `rmp_serde::to_vec_named` for a self-describing encoding, and
`serde_bytes` on every byte field so a terminal chunk costs its own length on the wire and not four
times it (`D79`). It reads and writes any `Read`/`Write` and knows nothing about sockets — which is
the whole reason a stdio carrier is nearly free. `crates/ubiq-host/src/remote.rs` already accepts
connections, checks a constant-time bearer token, upgrades an HTTP request to raw frames and hands
the socket to `bus::detached()` behind two pump threads; `crates/ubiq/src/app/remote_connect.rs` is
the mirror image, and it re-dials a lost saved host on a doubling backoff.
`crates/agent-manager` builds with `--no-default-features` and has no UI dependency at any feature
level, which `just core` checks on every change. `crates/ubiq-host` is proof that the whole stack
composes into a windowless binary.

**What does not exist.** No `.cargo/config.toml` and no cross-compilation: `just bundle` makes one
macOS `.app` and `just bundle-win` one Windows exe, and nothing produces a headless artifact for a
third triple. Nothing in the tree fetches, verifies or caches a binary — `resolve_program` in
`crates/ubiq-host/src/agent.rs` locates something already on `PATH` and that is the extent of it. No
message carries a schema version, because both ends have always been the same build. And no code
anywhere mentions WSL.

## 2. What a drone is

**A drone is a guest: one process, one directory, one byte stream, and nothing else on the far
machine.** Three properties carry the whole design, and every later section is one of them cashed in.

**It is pinned.** A drone artifact is keyed by the version of the Ubiq that placed it. The two ends
are never expected to negotiate their way to agreement across versions — a mismatch is refused at
the handshake, not accommodated, because the manifest that produced the binary was addressed by the
running Ubiq's own version in the first place. That is what makes a single `schema` integer enough.

**It is mortal.** A drone's life is exactly its channel's life. There is no reconnect, no session
id, no resume, no replay buffer, no acknowledgement and no state on disk that outlives the link.
§12 is the argument that this is a feature and what it costs.

**It is quiet.** It binds no socket unless told to, writes into exactly one cache directory, starts
no service, edits no profile, holds no index, and takes its children with it when it goes. §8 states
that as a budget rather than an aspiration.

What it is not: a second Ubiq host. It has no catalogue, no project store, no git worker, no
full-text index, no connector, no notification history and no usage meter. Those are the
coordinator's, they stay on the machine the user is sitting at, and §17 says so as a rule.

## 3. The carrier and the contract are different things

**The drone protocol needs one duplex byte stream. Everything else is a carrier, and carriers are
interchangeable.** Keeping those two apart is what lets the SSH question, the HTTPS question and the
Tailscale question be scheduling decisions rather than protocol decisions.

| Carrier | How | Status |
|---|---|---|
| **stdio** | Ubiq spawns a child and speaks frames over its stdin/stdout: `ssh -T host <path>/ubiq-drone --stdio`, `wsl.exe -d <distro> -- <path>/ubiq-drone --stdio`, `docker exec -i`, or the binary directly for local development | **The default, and the only one P0–P3 need** |
| **tcp+tls** | `ubiq-drone --listen 127.0.0.1:0`, the same HTTP-upgrade-and-bearer-token handshake `remote.rs` already implements | Opt-in, P4 |
| **ssh forward** | `-L`/`-R` onto the tcp carrier, for a site where an exec channel is unavailable but a forward is not | A configuration of the row above, not a third code path |
| **Tailscale** | The tcp carrier over a tailnet address, with the mesh supplying identity and encryption | Later. Nothing in the protocol changes |
| **https streaming** | Chunked request and response, or a WebSocket, for an egress-only network | Later, and the only row that would add a framing concern — see §4 |

**The stdio carrier is the interesting one, and it is the cheapest.** It gives, for free: no
listening socket anywhere on the far machine; authentication borrowed entire from a channel the user
already trusts and already configured (`~/.ssh/config`, an agent, a hardware token, jump hosts);
confidentiality from the same; teardown for free, because closing the child's stdin is an EOF the
drone cannot ignore; and one code path that covers the WSL case, the SSH case, the container case
and local development, differing only in the argv Ubiq spawns.

**It carries one real constraint: the stream must be a pipe and never a terminal.** A tty translates
`\n` into `\r\n`, intercepts `^C`, `^Z` and `^S`, and on a Windows console treats `0x1A` as
end-of-file — all of which corrupt a binary frame silently. So: `ssh` is invoked with `-T`, the
drone calls `isatty` on its own stdin at startup, and **a drone whose stdin is a terminal refuses to
start** with a named error rather than producing a stream that fails on the first byte a frame
happens to contain. An armoured framing for a carrier that cannot offer a pipe is deferred, and is
the only thing the https row would need.

**Stderr is the drone's only voice outside the protocol.** Diagnostics, panics and the refusal above
all go there; stdout carries frames and nothing else, ever. That is a rule, because one stray
`println!` desynchronises the stream for good.

## 4. The frame

**Reuse `wire.rs` unchanged.** A four-byte big-endian length and a MessagePack body is already
binary-efficient, already self-describing, already reads and writes an arbitrary `Read`/`Write`, and
already has the `serde_bytes` treatment that keeps a terminal chunk from quadrupling. There is no
argument for a second framing in this tree, and `D79` is the decision that settled the format.

Three adjustments the drone contract makes on top of it:

- **A lower ceiling.** `MAX_FRAME` is 64 MiB because a local socket can afford to be generous. A
  drone's largest legitimate payload is a file read, which the file worker caps at 2 MiB. The drone
  reads frames with an 8 MiB ceiling and treats anything larger as a protocol error, because the
  first purpose of a length prefix on a hostile stream is to bound an allocation.
- **A handshake before anything else.** `Hello` from the drone carries its version, `schema`, triple,
  and the capability set it actually answers; `Ready` from Ubiq carries the same and an accept flag.
  A mismatch is refused here, with a message the status bar can show, rather than surfacing later as
  a pane that looks like a crashed shell. This is the shelved proposal's `DroneHello`/`DroneReady`,
  kept intact.
- **A keepalive from day one.** `Ping`/`Pong`, unsolicited from the Ubiq side on an idle link.
  `G189` records that the existing remote bus has none and that a half-open connection — a suspended
  laptop, an expired NAT binding — is noticed only when a write next fails, which for an idle pane
  can be never. The drone is the cheap place to get this right, and the stdio carrier's EOF does not
  cover the tcp one.

## 5. The message set

**A drone speaks `DroneMessage`, not `Message`.** A new `drone` module in `crates/ubiq-proto`,
sharing `wire.rs`'s framing, `ids.rs`'s ULID newtypes and `messages.rs`'s `Secret` — one wire crate,
two enums, and only one of them ever reaches a window.

The architecture rule that the UI and the host talk only through `messages.rs` is untouched by this,
because **`DroneMessage` never reaches the UI**. The coordinator is the only thing that speaks it,
and it translates: a `TerminalInput` arriving from a window for a pane that a drone owns leaves as a
`drone::Input`, and the `drone::Output` that comes back re-enters the bus as an ordinary
`TerminalOutput`. The interface is not told that a machine boundary exists, which is the domain rule
*"the UI never assumes the pseudo-terminal is local"* collected rather than bent.

**Why not reuse `Message`.** Because a drone would then link the whole contract — connectors, git,
assist, work, notifications, usage rows, search — and answer "unsupported" to four-fifths of it.
That is compile time on a foreign triple, binary size against §8's budget, surface area against
§10's, and a version lock across families a drone never touches. The shelved proposal's argument for
reuse was that a parallel protocol means two ways to open a shell; that argument holds for the
*UI-facing* contract, which is exactly the one this leaves alone.

Five groups, and this is the whole of it:

| Group | Variants | Notes |
|---|---|---|
| **link** | `Hello`, `Ready`, `Ping`, `Pong`, `Shutdown`, `ProtocolError` | §4. `Shutdown` is a courtesy; EOF is the real one |
| **pane** | `SpawnPane`, `Input`, `Resize`, `Kill`, `Output`, `Exited`, `PaneError` | A one-to-one mirror of the pane family, with the drone minting its own pane ids and the coordinator mapping them to the `PaneId` the window holds |
| **file** | `Tree`, `Read`, `Write`, `Edit`, `Listing`, `Content`, `Written`, `FileError` | The `files/` worker's request set, confined to roots declared at `Hello` — same ceilings, same optimistic `FileVersion`, same `PathOp` vocabulary |
| **machine** | `Probe`, `MachineInfo` | What `host_meta.rs` and `shells.rs` answer locally: hostname, os, arch, triple, cpu count, memory, disk free, the shells that exist, the `PATH` that a login shell reports |
| **run** | `ListRunnable`, `Runnable`, `ComposePane`, `Composed`, `RunError` | §6. The agent-manager surface: which harnesses, accounts and profiles exist *there*, and a spawn that is a composed, confined run rather than a program name |

Deliberately absent, each for its own reason: **git** (`libgit2` cross-compiled per triple, for a
diff the shelved proposal already deferred), **search and index** (§8 — a guest does not hold a
tantivy index, and a shelled-out `rg` is a later capability), **watch** (a `notify` handle plus a
debounce thread is state, and the explorer can re-list), **conversation** (§6), and everything the
coordinator owns by definition — projects, sessions, notifications, connectors, the usage meter.

## 6. The reversal: a drone composes harness runs

The shelved proposal wrote *"a drone never runs a harness, and that is permanent"*, on the reasoning
that a composed run is `agent-manager`'s to build and `agent-manager` has no story for a machine it
is not running on. **The second half of that is true and the conclusion drawn from it is backwards:
the fix is to put `agent-manager` on the other machine, not to keep harnesses off it.**

**The WSL case is the argument.** A Windows user's Claude Code is installed in the distro. Its config
directory is a Linux path. Its credentials are in a Linux keychain or a Linux file. Its skills, its
MCP servers, its profiles and its accounts are all resolved from a Linux registry root. A drone that
only relays a pseudo-terminal could `exec claude` there and get a working pane by accident — and
lose account selection, profile-driven MCP and skill sets, permission modes, the ephemeral config
directory, the session record, and the confinement policy, every one of which is composed by
`resolve` and `provision` from files that exist only on the far side. There is no version of "the
composition happens here and the process runs there" that works, because composition *is* reading
that machine's files and writing that machine's run directory.

**The cost is honest and it is the drone's largest.** `agent-manager` with `--no-default-features`
keeps `spec`, `resolve`, `registry`, `account`, `profile`, `harness`, `provision`, `session`,
`source`, `config`, `settings`, `isolate` and the neutral `io` model, and drops `clap`, `ratatui`,
`crossterm`, `keyring` and its own optional `portable-pty`. `isol8` is core and comes regardless.
That is the drone's dependency floor, and it is the reason §8 states a size budget instead of
assuming one.

**What stays home: interpretation.** A structured run — Claude Code's native `stream-json` bridge
(`D95`), or an ACP adapter — is a byte stream with a parser on it. The drone composes the run,
confines it and spawns it; the *bytes* come back over the link as opaque, and
`agent_manager::io`'s bridge runs in `crates/ubiq-host` exactly where it runs today. So the drone
gains no conversation family, no `ConvUpdate`, and no opinion about what a harness said — which
keeps the entire conversation vocabulary, its permission prompts and its tool blocks on one side of
one boundary. **This is the one thing to verify before P3 is scheduled**: if `io::IoBridge` cannot be
driven from a supplied reader/writer pair rather than a child it spawned itself, the alternative is
the bridge running on the drone and a conversation group added to `DroneMessage` — a materially
larger drone, and a worse answer. §15 files it as the question it is.

## 7. isol8 on the far side, and the thing that blocks it

**Confinement is kernel-local: you cannot sandbox a process on a machine you are not on.** So a
confined run on the far side means `crates/agent-manager/src/isolate.rs` running there — the same
`plan()` producing the same `isol8::Spec` and `Context`, the same `HomeMode`, the same layer
selection, the same `ENV_PASS` allowlist. The drone needs no isolation code of its own and gets none;
it calls the library the way `compose_run` in `crates/ubiq-host/src/agent.rs` already does.

**One upstream limitation decides what P3 can actually deliver, and it lands squarely on the WSL
case.** `isolate::confined_launch` renders an applied policy as a `Launch` for a caller that owns its
own descriptors — which is what a pseudo-terminal is. On macOS that works, because `sandbox-exec -p`
is itself an exec. **On every other platform it returns an error**: Landlock is applied in-process
between fork and exec by isol8's own spawn path, and no rendered form of it exists to hand anyone.
A WSL drone is a Linux drone. So:

- A drone on macOS or a macOS remote host can confine a pty-owning run today.
- A drone in WSL, or on any Linux host, **cannot** — until isol8 grows the pty seam its own notes
  already call for.
- Until then a Linux drone runs `Isolation::None` and **says so in `Hello`'s capability set**, so
  Ubiq can tell the user the run is unconfined rather than letting them assume otherwise. A
  capability that is advertised and absent is a lie the user pays for later.

This is not a reason to delay the drone. It is a reason P3 ships composition, accounts, profiles,
skills, MCP and the session record — all of which work — and ships confinement on Linux when the
upstream seam exists. [`isolation-proposal.md`](./isolation-proposal.md) is where the local half of
this argument lives.

## 8. Non-invasive, stated as a budget

"Light and non-invasive" is a claim that decays unless it is written as something checkable.

**One directory.** `$XDG_CACHE_HOME/ubiq-drone/` on Linux, the platform equivalent elsewhere,
holding the binary, per-run config directories, managed homes and nothing else. No dotfile in
`$HOME`, no `PATH` edit, no shell profile line, no systemd unit, no launch agent, no registry key.
`ubiq-drone --purge` removes that directory and leaves the machine as it was found.

**No daemon.** The drone exits when its carrier closes: EOF on stdin under the stdio carrier, a
closed socket under the tcp one. It exits **with its children** — a process group kill on Unix, a job
object on Windows — so a dropped link cannot leave a confined harness running against a repository
nobody is watching. An orphan drone is the failure this design refuses to produce.

**No socket by default.** `--listen` is the only way to get one, it is off, and when on it binds
loopback unless told otherwise. That is the reverse of `D80`, which binds the host's listener to
every interface, and it is a deliberate reversal: a host is something a user starts for the purpose
of being attached to, a drone is a guest on a machine that did not ask for it.

**No persistent state between links.** No catalogue, no index, no database, no watch. Every answer is
computed on demand from the filesystem. The shelved proposal's argument for this is the right one and
survives intact: the class of tool this replaces routinely holds hundreds of megabytes of resident
index, and that is precisely the cost being avoided.

**A size target, and a recipe that prints it.** A stripped release drone under 15 MB, with `just
drone-size` naming the number, because a budget nobody measures is a budget nobody keeps. What it
must never link: `tantivy`, `rusqlite`, `git2`, `notify`, `gpui`, or any async runtime — none of
which this stack uses anywhere today, and all of which are one convenience away.

## 9. Distribution: a manifest, a cache, and one push

**A drone is fetched, never built on demand.** Ubiq holds a repository address; the repository holds
a manifest; the manifest names one artifact per target triple.

```json
{
  "schema": 1,
  "ubiq_version": "0.4.2",
  "drones": [
    { "triple": "x86_64-unknown-linux-gnu", "file": "ubiq-drone-x86_64-linux",
      "size": 11829456, "sha256": "…" }
  ]
}
```

**Two repository kinds, one resolver.** An `https://` base URL in a release build, and a local
directory for development — the same manifest, read from a file instead of fetched. The dev kind is
how this is worked on before any release pipeline exists, and it is the answer to the shelved
proposal's `G108`, which blocked its P1 outright.

**The manifest is signed and the artifact is hashed.** An Ed25519 signature over the manifest bytes,
verified against a public key compiled into Ubiq; a SHA-256 per artifact, verified after download and
again before every use, because a cache is a writable directory and a check that runs once is a check
that runs at the wrong time. **An unsigned manifest is accepted only for a local directory
repository, and only behind an explicit development flag that a release build does not compile.**

**The cache is Ubiq's, keyed by what identifies a drone.**
`<config root>/drones/<ubiq-version>/<triple>/ubiq-drone`. Version in the key is what makes §2's
pinning arithmetic rather than policy: a new Ubiq fetches a new drone, and the old one is garbage
rather than a compatibility problem.

**Getting it there is one probe and, at most, one push.**

1. **Probe.** One round trip: `ssh -T <target> 'uname -sm; test -x <path> && <path> --fingerprint'`
   — or the same through `wsl.exe -d <distro> --`. That answers both questions at once: which triple
   to resolve, and whether a good copy is already in place. `--fingerprint` prints the drone's own
   SHA-256 and exits.
2. **Compare.** If the fingerprint equals the manifest's hash for that triple, skip to 4.
3. **Push over the carrier that is already there.** `ssh -T <target> 'cat > <path>.tmp && chmod +x
   <path>.tmp && mv <path>.tmp <path>'`, streaming the cached bytes on stdin. No `scp`, no second
   authentication, no second channel, and a jump host configured in `~/.ssh/config` works because
   `ssh` is doing the work. The temp-and-rename is what stops a half-written binary from ever being
   executable. Under WSL the same shape runs through `wsl.exe`, writing into the distro's own
   filesystem rather than through `/mnt/c`, which is slow enough to be worth naming.
4. **Launch.** The same carrier invocation with `--stdio`, and §4's handshake.

Ubiq never executes a binary on the far side that it did not just verify or just fingerprint.

## 10. Security

The brief asks for the channel and the protocol to be "super secure". Stating the threat model is
what makes that a design rather than an adjective.

| Threat | Answer |
|---|---|
| Someone reading or altering the link in transit | The carrier's own cryptography — SSH, TLS, a tailnet. **The drone implements no cryptography of its own**, which is the only defensible position for a binary this small |
| Another local user on the far machine talking to the drone | There is nothing to talk to: the default carrier is a pipe between two processes, and no socket is bound. With `--listen`, a 256-bit token compared in constant time, handed over stdin at startup, loopback-bound, **never on `argv`**, where `ps` would show it to every user on the box |
| A tampered or substituted drone binary | §9: signed manifest, per-artifact hash checked before use, version-pinned key, temp-and-rename so a partial write is never executable |
| A drone reaching outside what it was asked for | The file group is confined to roots declared at `Hello` and resolved with the same two-layer check `files/path.rs` already implements — textual component rejection, then canonicalise-and-contain, with a symlink-leaf refusal on writes |
| Credential material crossing the link | It does not. **Accounts on the far machine are the far machine's.** Ubiq ships a reference — an account name the drone's own `FsAccountStore` resolves — and never material, which is the existing rule and not a new one. A harness that needs a login logs in *there*, in its own pane, the way the local login flow already works |
| A secret in a log | `Secret` from `messages.rs`, whose `Debug` writes `Secret(***)` and whose only reader is a deliberately-named `expose()` (`D65`). The drone's diagnostics go to stderr and never carry one |

The bound on a drone's authority is worth saying out loud rather than leaving implicit: **a drone can
do anything the user it runs as can do on that machine.** That is what it is for. The control is the
carrier's authentication — if someone can open the channel, they could have run the commands anyway —
which is exactly why §3 prefers a carrier the user already authenticates and §8 refuses to open a
second door beside it.

## 11. The seam in the coordinator

The whole change is behind one map. `Coordinator` holds `panes: HashMap<PaneId, Pty>`; that becomes
`HashMap<PaneId, PaneBacking>` with `Local(Pty)` and `Drone(DroneId, RemotePaneId)`. Every pane arm
already funnels through `owns(client, pane_id)` and then that lookup, so `TerminalInput`,
`TerminalResize`, `Focus` and `CloseWorkspace` change shape once each and nowhere else. The branch
point is `spawn_workspace`, where a project carrying a drone reference composes over the link instead
of calling `pty::spawn`.

A new `drones` module in `crates/ubiq-host` owns the links: the carrier child process, one reader thread
per drone turning `drone::Output` into `Message::TerminalOutput` on the client's mailbox — the same
shape `watch/` already uses to reach a mailbox without a coordinator relay — and one writer. The file
group routes the same way, inside `files/`'s request dispatch, on the project's location.

**`crates/ubiq` changes by one picker and one status line.** It never learns that a pane is remote,
because it never knew a pane was local.

## 12. No reconnect, and everything it buys

**A drone's link is lost, the drone is gone.** No resume, and this is a decision, not a gap.

What it removes from the design: session identifiers, a resumption handshake, a replay buffer,
sequence numbers, acknowledgements, idempotent request ids, reconciliation of two views of a pane
set, and any state on the far side that has to survive anything. A protocol without resumption is
*small*, and small is the property this drone is optimising for above all others.

What happens when a link drops: every pane it backed answers `PaneExited`, every in-flight file
request answers an error, the project's health turns `Unreadable` with a reason, and the far-side
process exits on EOF and takes its children with it. That is the same teardown `Bus::drop_remote`
and `D85` already define for a lost host — *"losing a host takes its panes and projects with it"* —
applied one level further out, and the interface already implements it.

**Where this differs from the remote host deliberately.** The interface *does* re-dial a saved host
on a lost socket, with a doubling backoff. A drone does not, because the two are not the same kind
of peer: a host is a durable process that owns panes, sessions and a catalogue which are all still
there when the socket comes back, and re-dialling reaches them. A drone owns nothing that outlives
the link, so a reconnect would reach an empty process and there would be nothing to re-attach to.
Reconnecting to a drone is spawning a new drone, and spelling it that way is more honest than a
retry loop that looks like recovery.

**What it costs, said plainly.** A closed laptop lid kills every remote pane and every harness
running under one, including a long agent run someone was depending on. Making that survivable means
a drone that outlives its carrier — a durable identity, a listening endpoint or a relay, and state on
the far side — which is a different product with a different security posture, and is the one thing
this proposal defers rather than decides.

## 13. What can go wrong

- **The carrier's stdin is a terminal.** Refused at startup with a named error, before a frame is
  written (§3). The likeliest cause is an `ssh` invocation that lost its `-T`.
- **Versions disagree.** Refused at `Ready` with the two versions named, never as a pane that starts
  and then misbehaves.
- **No drone exists for that triple.** The manifest is the answer: the failure names the triple the
  probe found and the triples the manifest offers, which is a fixable complaint rather than a dead
  end.
- **The hash does not match.** Refuse, do not run, do not silently re-download and retry — a mismatch
  is either a corrupt cache or something worse, and both want a person.
- **The far machine has no writable cache directory**, or a `noexec` mount under it. Reported at the
  push step, distinct from "unreachable", so the user knows to name a different path.
- **isol8 cannot confine on that platform.** Not an error: a capability absent from `Hello`, and a
  run that says it is unconfined (§7).
- **The link drops mid-work.** §12. One teardown, no partial state, no orphan.

## 14. Phases

**P0 — the contract and the stdio carrier.** `drone.rs`, the handshake, the keepalive, the pane
group, the `drones.rs` link owner and the `PaneBacking` seam. The drone binary built locally and
named by an absolute path; no manifest, no fetching. A terminal in a WSL distro, which is the whole
point, is reachable at the end of this phase.

**P1 — machine and files.** The machine group, the file group with declared roots, and the remote
project field. The explorer and the editor work against a drone project.

**P2 — distribution.** The manifest, the signature, the cache, the fingerprint probe and the push.
This is what makes a drone something a user gets rather than something a developer places, and it is
the phase the shelved proposal could not start.

**P3 — runs.** `agent-manager` embedded, the run group, accounts and profiles resolved on the far
side, `isol8` where the platform allows it (§7), and the io bridge question of §6 answered before
this phase is scheduled rather than during it.

**P4 — the socket carrier.** `--listen`, TLS, the token, and a tailnet address as its first real
user.

## 15. What this asks to be decided

| | Question | Recommendation |
|---|---|---|
| a | One contract for the drone, or reuse `Message`? | **`DroneMessage`, a second enum in `ubiq-proto`.** The coordinator translates; the UI contract never changes (§5) |
| b | Does a drone run harnesses? | **Yes**, and this reverses the shelved proposal. The WSL case has no other answer (§6) |
| c | Where does the structured-io bridge run? | **On the host**, with the drone relaying opaque bytes — *conditional* on `io::IoBridge` being drivable from a supplied reader/writer pair. Verify before P3 is scheduled; if not, the bridge moves to the drone and the contract grows a conversation group (§6) |
| d | Default carrier? | **stdio.** One code path for SSH, WSL, containers and local development, and no socket on the far machine (§3) |
| e | Does a drone ever listen? | **Only when told**, loopback by default — the reverse of `D80`, for a reason (§8, §10) |
| f | Reconnect? | **No, permanently.** A drone owns nothing that would survive one (§12) |
| g | Signed manifests before any release pipeline exists? | **Yes, with a local-directory dev repository as the unsigned exception**, compiled out of release builds (§9) |
| h | Does a drone answer search? | **No**, not even shelled out, in these phases. It is a capability the contract has room for and no phase claims |
| i | Which triples ship first? | **`x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`** — WSL and the common remote host, in that order. Everything else follows the manifest |

## 16. Rules this adds

- **The drone's stdout carries frames and nothing else.** Every diagnostic goes to stderr. One stray
  write desynchronises the stream permanently.
- **A drone's stdin is a pipe, never a terminal**, and a drone that finds otherwise refuses to start.
- **A drone binds no socket unless asked**, and binds loopback when asked without an address.
- **A drone writes into one directory and `--purge` removes it.** Nothing else on the far machine is
  touched, ever.
- **A drone dies with its channel, and takes its children with it.**
- **Ubiq never executes a far-side binary it has not just verified or just fingerprinted.**
- **No secret on `argv`** — kept from the shelved proposal, and now also: no credential material over
  the link at all, only references the far side resolves.
- **`DroneMessage` never reaches the interface.** If a drone fact needs to be drawn, the coordinator
  translates it into the existing contract.
- **A capability is advertised only when it works on that platform.** An absent capability is a
  truthful answer; an advertised one that silently does nothing is not.

## 17. What stays out

- **Git on the drone.** `libgit2` per triple, for a diff the shelved proposal already deferred.
- **A search index on the drone.** The wrong shape for a guest, not merely a later phase.
- **A filesystem watch on the drone.** A handle and a debounce thread are state; the explorer
  re-lists instead.
- **A drone that outlives its carrier.** §12 is the whole argument, and this is the deferral it makes
  deliberately rather than by omission.
- **An https-streaming carrier.** The only row in §3 that would need a second framing, and nothing
  yet needs it.
- **A harness reaching *out* from a local pane to a remote machine.** That is the MCP-connector idea
  the shelved proposal named — a `remote-file` and a `remote-shell` tool pointed at a drone — and it
  is a consumer of this, worth its own proposal after P1.
- **Windows as a drone target.** WSL is the Windows story here, and it is a Linux triple.

## 18. Rows this proposes for the backlog

| Row |
|---|
| No cross-compilation exists: `just bundle` and `just bundle-win` produce host-native artifacts and nothing builds a headless binary for a third triple |
| Nothing in the tree fetches, verifies or caches a binary from a manifest; `resolve_program` locates what is already on `PATH` and that is all |
| No message carries a protocol or schema version — true today only because both ends are always one build |
| `isolate::confined_launch` returns an error on every platform but macOS, so a Linux or WSL drone cannot confine a run that owns its own pseudo-terminal until isol8 grows the pty seam its own notes call for |
| Whether `agent_manager::io`'s bridge can be driven from a supplied reader/writer pair rather than a child it spawned decides where the structured-io parser lives, and therefore how large a drone is |
| The existing remote bus has no keepalive (`G189`); the drone contract carries `Ping`/`Pong` from P0 and the host bus still does not |

## Related docs

- [`backlog/remote-drone-proposal.md`](./backlog/remote-drone-proposal.md) — the shelved design this
  supersedes in three places and keeps everywhere else
- [`../tech/architecture.md`](../tech/architecture.md) — the rules a drone must not bend, and the
  "remote harnesses" future it cashes in
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the framing this reuses and the
  pane and file families the coordinator translates into
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — the boundary a drone carries across a
  machine, and what `--no-default-features` leaves behind
- [`isolation-proposal.md`](./isolation-proposal.md) — the local half of the confinement argument
