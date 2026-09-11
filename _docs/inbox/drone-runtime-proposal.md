---
id: inbox-drone-runtime
title: Proposal — the drone, a portable runtime Ubiq places on another machine
kind: proposal
status: proposal
summary: One small executable, built twice, that Ubiq puts where it is not running — a relay build that gives a remote machine's terminal, files and machine facts over SSH with no agent knowledge at all, and a runtime build that adds an embedded agent-manager to compose and confine harness runs inside WSL or a container; one duplex byte stream, one contract with a capability handshake, a signed manifest per triple, no socket by default, no state, and no reconnect.
read_when: you are deciding how a terminal, a file or a harness reaches a machine Ubiq is not running on, what "drone" means in this tree, which carrier a remote link rides, how a foreign-triple binary gets fetched and verified, or why confinement in a pane is macOS-only
updated: 2026-09-11
depends_on: [tech-architecture, tech-transport, tech-agent-manager, tech-structure, tech-decisions, feat-panes, inbox-isolation]
---

# Proposal — the drone, a portable runtime Ubiq places on another machine

Ubiq reaches only the machine that draws the window. Two quite different things want fixing, and
conflating them is how this design goes wrong:

**Remote access.** A terminal, a file tree and a machine's facts on a host across a network. No
agents, no harnesses, no composition — the user opens a shell somewhere else and edits something
there. The far machine may belong to someone else, may have other users on it, and is reached over a
network that has to be assumed hostile.

**A local boundary.** A Windows user whose toolchains, repositories, shells and agent harnesses all
live inside a WSL distribution, one kernel boundary away — and, later, the same shape for a
container. Their Claude Code is a Linux binary with a Linux config directory and a Linux credential
store, and no amount of pseudo-terminal work on the Windows side composes a run for it. Here
`crates/agent-manager` must be on the far side, because composing a run *is* reading that machine's
files and writing that machine's run directory. There is no network and no third party: it is the
user's own machine, one `wsl.exe` away.

This proposes the **drone** as one executable that serves both — **one crate, one contract, two
builds**, differing by a feature flag that decides whether `agent-manager` is linked. §2 is that
split and the reason it is a flag rather than two projects. Everything else — the carrier, the
frame, the manifest, the no-reconnect rule — is shared, because both cases are the same problem:
one duplex byte stream to a process somewhere Ubiq is not.

## 1. Where this starts

**This is a deferred idea, not a new one, and the tree names it in three places already.**
[`../backlog.md`](../backlog.md)'s Deferred table carries `D2`, *"harnesses on remote hosts"*, on
`D1`'s reasoning that the contract makes the coordinator's process boundary cheap to move later.
[`../tech/architecture.md`](../tech/architecture.md) states the shape as a design commitment:
*"a harness running on another host or in a container is structurally the same problem as a terminal
stream crossing a machine boundary … the contract is identical, because a pane was always a tagged
bidirectional byte stream plus control messages."* And
[`../tech/transport-contract.md`](../tech/transport-contract.md) names the file family's seam for it
by this word: *"the seam a remote drone slots into: a project id and a relative path do not say
which machine answered."* What is missing is everything underneath those sentences.

**Six positions this document takes up front**, because each is somewhere a reasonable design goes
the other way:

- **Shell out to the system `ssh`**, never an embedded SSH client — `D49`'s precedent, and the
  margin is wider here: host-key checking, every authentication method the user has configured,
  `~/.ssh/config` aliases and jump hosts, all already trusted and none of them re-earned. This is
  the opposite call from `D43`, which refused to shell to `git diff`, and the two agree rather than
  conflict: shell to the real thing when it hands back an exit code and bytes, own the work when it
  hands back a format that has to be understood. Nothing here asks `ssh` for structure.
- **A remote project is a field on `ProjectRecord`**, not a second kind of project, so every message
  that resolves through a project id keeps working unchanged.
- **The default carrier is a child process's own stdio**, not a forwarded port — §3. A forwarded
  port is a socket every other local user on that machine can dial; a pipe is not.
- **A drone speaks its own small contract**, not the whole of `Message` — §5.
- **No secret on `argv`, no index, and no persistent state on the far side** — §8.
- **A dropped link is a teardown, not a retry loop** — §12.

**What already exists, and is most of the transport.**
`crates/ubiq-proto/src/wire.rs` frames a message as a four-byte big-endian length and a MessagePack
body (`D79`), and it reads and writes any `Read`/`Write` while knowing nothing about sockets — which
is the whole reason a stdio carrier is nearly free. `crates/ubiq-host/src/remote.rs` already accepts
connections, checks a constant-time bearer token, upgrades an HTTP request to raw frames and hands
the socket to `bus::detached()` behind two pump threads; `crates/ubiq/src/app/remote_connect.rs` is
the mirror image, and it re-dials a lost saved host on a doubling backoff. `crates/agent-manager`
builds with `--no-default-features` and has no UI dependency at any feature level, which `just core`
checks on every change, and `crates/ubiq-host` is proof the stack composes into a windowless binary.

**What does not exist.** No `.cargo/config.toml` and no cross-compilation — `just bundle` makes one
macOS `.app`, `just bundle-win` one Windows exe, and nothing produces a headless artifact for a
third triple. Nothing fetches, verifies or caches a binary: `resolve_program` in
`crates/ubiq-host/src/agent.rs` locates what is already on `PATH` and that is the extent of it. No
message carries a schema version, because both ends have always been one build. And no code anywhere
mentions WSL.

## 2. Two postures, one crate, two artifacts

**The two use cases differ in more than a feature list — they differ in who owns the machine and
whether there is a wire at all.** That is what makes one binary with a flag right and two projects
wrong: the difference is a dependency and a threat model, not a design.

| | **Relay** — remote access | **Runtime** — WSL, and later containers |
|---|---|---|
| Whose machine | Someone else's, or a shared server | The user's own, one boundary away |
| Carrier | `ssh -T`, later a socket on a tailnet | `wsl.exe -d <distro> --`, `docker exec -i` |
| Is there a wire | Yes, and it is assumed hostile | **No.** A pipe between two processes of the same user on one machine |
| Answers | panes, files, machine facts | the same, plus composed and confined runs |
| Links `agent-manager` | No | Yes, with `isol8` behind it |
| Build | `cargo build -p ubiq-drone` | `--features agents` |
| Artifact | `ubiq-drone-<triple>` | `ubiq-drone-agents-<triple>` |

**One contract covers both, because the handshake already negotiates capability.** The `run` group
of §5 exists in the enum in both builds; the relay build simply does not advertise it in `Hello`,
and Ubiq never sends it. Nothing is conditionally compiled in `crates/ubiq-proto` — a contract that
changes shape with a feature flag is a contract with two versions, which is the thing §4's single
`schema` integer exists to avoid.

**The flag is checked the way the crate boundary already is.** `just core` exists precisely because
`agent-manager` must keep building without its frontend; the drone gets the mirror check — a recipe
that fails if `agent-manager` appears in a default build's dependency graph. Without it one
convenient `use` quietly makes the relay build the runtime build.

**And the relay is worth keeping small rather than shipping one fat drone everywhere**, because it
is the one that goes onto machines the user does not own: a build that cannot compose a run, cannot
read a credential store and links no sandbox engine is a smaller thing to audit, to trust, and to
justify to whoever administers that host. The runtime build never leaves the user's own machine, so
it can afford what the relay cannot.

**Both builds are the same guest: one process, one directory, one byte stream.** Three properties
carry the rest of the design, and every later section is one of them cashed in.

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
| **stdio, local** | `wsl.exe -d <distro> -- <path>/ubiq-drone --stdio`, `docker exec -i`, or the binary directly in development. **No network is involved at any point** | **The runtime build's only carrier**, and the first one to build |
| **stdio, over SSH** | `ssh -T <host> <path>/ubiq-drone --stdio` — the same frames, on a channel the user already authenticates | **The relay build's default** |
| **tcp+tls** | `ubiq-drone --listen 127.0.0.1:0`, the same HTTP-upgrade-and-bearer-token handshake `remote.rs` already implements | Opt-in, P4 |
| **ssh forward** | `-L`/`-R` onto the tcp carrier, for a site where an exec channel is unavailable but a forward is not | A configuration of the row above, not a third code path |
| **Tailscale** | The tcp carrier over a tailnet address, with the mesh supplying identity and encryption | Later. Nothing in the protocol changes |
| **https streaming** | Chunked request and response, or a WebSocket, for an egress-only network | Later, and the only row that would add a framing concern — see §4 |

**The stdio carrier is the cheapest and it gives the most.** No listening socket anywhere on the far
machine; authentication and confidentiality borrowed entire from a channel the user already trusts
and configured (`~/.ssh/config`, an agent, a hardware token, jump hosts); teardown for free, since
closing the child's stdin is an EOF the drone cannot ignore; and one code path across WSL, SSH,
containers and development, differing only in the argv Ubiq spawns.

**And for the runtime build it does better than secure the wire: it removes it.** `wsl.exe` and
`docker exec -i` are process spawns, not connections — same machine, same user, a pipe the kernel
owns. Nothing to encrypt, nothing to authenticate, nothing to bind, no credential in the path. Every
token, certificate and tailnet consideration in §10 belongs to the relay alone, which is the
strongest argument for keeping the postures named apart rather than designing for their union.

**One real constraint: the stream must be a pipe and never a terminal.** A tty translates `\n` into
`\r\n`, intercepts `^C`, `^Z` and `^S`, and on a Windows console treats `0x1A` as end-of-file — each
of which corrupts a binary frame silently. So `ssh` is invoked with `-T`, the drone calls `isatty`
on its own stdin at startup, and **a drone whose stdin is a terminal refuses to start**, with a named
error rather than a stream that dies on the first byte a frame happens to contain. An armoured
framing for a carrier that cannot offer a pipe is deferred, and is all the https row would need.

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
  a pane that looks like a crashed shell.
- **A keepalive from day one.** `Ping`/`Pong`, unsolicited from the Ubiq side on an idle link.
  `G189` records that the existing remote bus has none and that a half-open connection — a suspended
  laptop, an expired NAT binding — is noticed only when a write next fails, which for an idle pane
  can be never. The drone is the cheap place to get this right, and the stdio carrier's EOF does not
  cover the tcp one.

## 5. The message set

**A drone speaks `DroneMessage`, not `Message`.** A new `drone` module in `crates/ubiq-proto`,
sharing `wire.rs`'s framing, `ids.rs`'s ULID newtypes and `messages.rs`'s `Secret` — one wire crate,
two enums, and only one of them ever reaches a window.

The architecture rule that the UI and the host talk only through `messages.rs` is untouched, because
**`DroneMessage` never reaches the UI**. The coordinator is the only thing that speaks it, and it
translates: a `TerminalInput` for a pane a drone owns leaves as a `drone::Input`, and the
`drone::Output` that comes back re-enters the bus as an ordinary `TerminalOutput`. The interface is
never told a machine boundary exists — the domain rule *"the UI never assumes the pseudo-terminal is
local"* collected rather than bent.

**Why not reuse `Message`.** A drone would link the whole contract — connectors, git, assist, work,
notifications, usage rows, search — to answer "unsupported" to four-fifths of it: compile time on a
foreign triple, size against §8's budget, surface against §10's, and a version lock across families
it never touches. The argument the other way — that a parallel protocol means two ways to open a
shell and two ways to read a file — holds for the *UI-facing* contract, which is exactly the one
this leaves alone.

Five groups, and this is the whole of it:

| Group | Variants | Notes |
|---|---|---|
| **link** | `Hello`, `Ready`, `Ping`, `Pong`, `Shutdown`, `ProtocolError` | §4. `Shutdown` is a courtesy; EOF is the real one |
| **pane** | `SpawnPane`, `Input`, `Resize`, `Kill`, `Output`, `Exited`, `PaneError` | A one-to-one mirror of the pane family, with the drone minting its own pane ids and the coordinator mapping them to the `PaneId` the window holds |
| **file** | `Tree`, `Read`, `Write`, `Edit`, `Listing`, `Content`, `Written`, `FileError` | The `files/` worker's request set, confined to roots declared at `Hello` — same ceilings, same optimistic `FileVersion`, same `PathOp` vocabulary |
| **machine** | `Probe`, `MachineInfo` | What `host_meta.rs` and `shells.rs` answer locally: hostname, os, arch, triple, cpu count, memory, disk free, the shells that exist, the `PATH` that a login shell reports |
| **run** | `ListRunnable`, `Runnable`, `ComposePane`, `Composed`, `RunError` | §6, and **answered by the runtime build alone** — in the enum in both, advertised in `Hello` by one. The agent-manager surface: which harnesses, accounts and profiles exist *there*, and a spawn that is a composed run rather than a program name |

Deliberately absent, each for its own reason: **git** (`libgit2` cross-compiled per triple, for a
diff that is not what a remote shell is for), **search and index** (§8 — a guest does not hold a
tantivy index, and a shelled-out `rg` is a later capability), **watch** (a `notify` handle plus a
debounce thread is state, and the explorer can re-list), **conversation** (§6), and everything the
coordinator owns by definition — projects, sessions, notifications, connectors, the usage meter.

## 6. The runtime build composes harness runs

The obvious rule to write is *a drone never runs a harness*, on the reasoning that a composed run is
`agent-manager`'s to build and `agent-manager` has no story for a machine it is not running on.
**For remote access that is the right call and this proposal keeps it — the relay build links no
`agent-manager` at all. For a local boundary the conclusion inverts: the fix is to
put `agent-manager` on the other side, not to keep harnesses off it.**

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
That is the **runtime build's** dependency floor. The relay build's floor is far below it — serde,
the wire, a pty and not much else — which is §2's whole point and the reason §8 states two budgets
rather than one.

**What stays home: interpretation.** A structured run — Claude Code's native `stream-json` bridge
(`D95`), or an ACP adapter — is a byte stream with a parser on it. The drone composes, confines and
spawns; the bytes come back opaque, and `agent_manager::io`'s bridge runs in `crates/ubiq-host`
exactly where it runs today. So the drone gains no conversation family, no `ConvUpdate` and no
opinion about what a harness said, which keeps the whole conversation vocabulary, its permission
prompts and its tool blocks on one side of one boundary. **This is the one thing to verify before P2
is scheduled**: if `io::IoBridge` cannot be driven from a supplied reader/writer pair rather than a
child it spawned itself, the bridge moves to the drone and `DroneMessage` grows a conversation
group — a materially larger drone, and a worse answer.

## 7. isol8 on the far side: not a Landlock problem, a descriptor-ownership one

**Confinement is kernel-local: you cannot sandbox a process on a machine you are not on.** So a
confined run on the far side means `crates/agent-manager/src/isolate.rs` running there — the same
`plan()` producing the same `isol8::Spec` and `Context`, the same `HomeMode`, the same layer
selection, the same `ENV_PASS` allowlist. The runtime build needs no isolation code of its own and
gets none; it calls the library the way `compose_run` in `crates/ubiq-host/src/agent.rs` already
does.

**Landlock is not the obstacle, and it is worth being exact about what is.** isol8's Linux backend
enforces, in the kernel, and isol8's own CLI confines successfully inside WSL today. What fails is
narrower: `isolate::confined_launch` renders an applied policy into a `Launch` for *a caller that
owns its own descriptors*, and only macOS has a rendered form to give — `sandbox-exec -p <policy>`
is itself an `execve`, so the harness stays one process whatever is on its stdio. The function
returns an error everywhere else. And per `G90` the pty seam that would replace it **already exists
in the pinned isol8 revision**; what blocks the switch is an ownership shape in Rust, not a kernel
capability: `PtyChild` fuses the child with the master, so `resize` borrows it while `child` borrows
it mutably, and a pane cannot be resized on one thread while another waits on the process. Ubiq
cannot accept that, because *a resize is incomplete until the harness knows* — a pane that resizes
while its harness believes the old size is the classic corruption bug.

**So: an implementation problem, on both available routes.** Neither needs anything from the kernel
that is not already there.

**Route A — split the handle.** What `refs/isol8-pty-seam-update.md` §8 asks for: a master-only
handle, independent of the child, that resize and the reader can hold while the reaper waits.
Smallest change to isol8. Its cost lands here: on Linux a confined run's pseudo-terminal would be
isol8's, not `portable-pty`'s, so `pty/mod.rs` — in the host and in the drone — grows a backend enum
and two lifecycles to keep aligned, one per platform, for the rest of the project's life.

**Route B — an exec shim, the way macOS already does it.** A small `isol8-exec` that sets
`no_new_privs`, applies the ruleset to itself, and `execve`s the target in place. Landlock
restrictions are preserved across `execve` by design — this is exactly the shape of the sandboxer
sample the kernel itself ships — so the confined process is still one process, still owns whatever
descriptors its parent gave it, and `confined_launch` returns a `Launch` on Linux for the same
reason it does on macOS. **Nothing in `pty/mod.rs`, the host, or the drone changes at all**, and
`portable-pty` stays the single pseudo-terminal implementation on every platform. Its cost: a second
small binary to build, ship and version alongside the drone, and a rendered policy that has to reach
it out of band — an inherited fd or a file, never `argv`, since a policy is large and `argv` is
public.

**Recommendation: B**, and it is not close for this tree. Route A buys a smaller diff in isol8 and
pays for it with a permanent asymmetry in the one module where asymmetry is most expensive; route B
makes Linux look like macOS and leaves both the host and the drone untouched. Route A becomes the
right answer only if isol8's Linux policy ever needs more than Landlock — a mount or PID namespace
cannot be applied and then `execve`d in place as cleanly, because the process that unshares is not
the process that lands inside.

**What ships before either lands.** Composition, accounts, profiles, skills, MCP and the session
record all work on Linux today; only confinement does not. So the runtime build runs
`Isolation::None` there and **says so in `Hello`'s capability set**, and Ubiq tells the user the run
is unconfined rather than letting them assume otherwise. A capability that is advertised and absent
is a lie the user pays for later.

**Two smaller platform facts, so they are not discovered twice.** A container is already a boundary,
so a runtime build inside Docker can honestly report unconfined rather than nesting a sandbox in a
sandbox — and `isol8::sandbox::ensure_not_nested` already refuses that case explicitly. And a
*Windows-native* runtime build is not the answer to the Windows question: per `G90`, isol8's
AppContainer backend builds a token without enforcing the policy's path grants, so confinement there
would be confinement in name only. WSL is the Windows story, which is where this proposal started.

[`isolation-proposal.md`](./isolation-proposal.md) is where the local half of this argument lives,
and `G90` is the row that owns the limitation.

## 8. Non-invasive, stated as a budget

"Light and non-invasive" is a claim that decays unless it is written as something checkable.

**One directory.** `$XDG_CACHE_HOME/ubiq-drone/` on Linux, the platform equivalent elsewhere,
holding the binary, per-run config directories, managed homes and nothing else — no dotfile in
`$HOME`, no `PATH` edit, no shell profile line, no unit, no launch agent, no registry key.
`ubiq-drone --purge` removes it and leaves the machine as it was found.

**No daemon.** The drone exits when its carrier closes — EOF on stdin, or a closed socket — and it
exits **with its children**, by process group on Unix and job object on Windows, so a dropped link
cannot leave a confined harness running against a repository nobody is watching. An orphan drone is
the failure this design refuses to produce.

**No socket by default.** `--listen` is the only way to get one, it is off, and it binds loopback
unless told otherwise — the reverse of `D80`, deliberately: a host is started to be attached to, a
drone is a guest on a machine that did not ask for it.

**No persistent state between links.** No catalogue, no index, no database, no watch; every answer
is computed on demand. The class of tool this replaces routinely holds hundreds of megabytes of
resident index for exactly this job, and that is the cost the whole design exists to avoid — a
repository could afford it, and a drone pointed at someone's home directory for a maintenance task
could not.

**Two size targets, and a recipe that prints both.** A stripped relay under 8 MB and a runtime build
under 20 MB, with `just drone-size` naming the numbers, because a budget nobody measures is a budget
nobody keeps. That recipe is also where §2's flag check belongs, since it already builds both. What
neither may ever link: `tantivy`, `rusqlite`, `git2`, `notify`, `gpui`, or any async runtime — none
of which this stack uses today, and all of which are one convenience away.

## 9. Distribution: a manifest, a cache, and one push

**A drone is fetched, never built on demand.** Ubiq holds a repository address; the repository holds
a manifest carrying a `schema`, the `ubiq_version` it was cut for, and one entry per
`(triple, build)` — `triple`, `build` (`relay` or `agents`), `file`, `size` and `sha256`.

**Two repository kinds, one resolver.** An `https://` base URL in a release build, and a local
directory for development — the same manifest, read from a file instead of fetched. The dev kind is
how this is worked on before any release pipeline exists — which matters, because without it every
phase below waits on a cross-compilation and publishing story that does not exist yet.

**The manifest is signed and the artifact is hashed.** An Ed25519 signature over the manifest bytes,
verified against a public key compiled into Ubiq; a SHA-256 per artifact, checked after download and
again before every use, because a cache is a writable directory and a check that runs once runs at
the wrong time. **An unsigned manifest is accepted only for a local directory repository, behind a
development flag a release build does not compile.**

**The cache is Ubiq's**, at `<config root>/drones/<ubiq-version>/<triple>/<build>/ubiq-drone`. The
build belongs in the key because a relay and a runtime for one triple are two binaries with two
hashes; the version belongs there because it is what makes §2's pinning arithmetic rather than
policy — a new Ubiq fetches a new drone, and the old one is garbage rather than a compatibility
problem.

**Getting it there is one probe and, at most, one push.**

1. **Probe.** One round trip — `ssh -T <target> 'uname -sm; test -x <path> && <path>
   --fingerprint'`, or the same through `wsl.exe -d <distro> --` — answering both questions at once:
   which triple to resolve, and whether a good copy is already there. `--fingerprint` prints the
   drone's own SHA-256 and exits.
2. **Compare** it against the manifest's hash; if it matches, skip to 4.
3. **Push over the carrier that is already there.** `ssh -T <target> 'cat > <path>.tmp && chmod +x
   <path>.tmp && mv <path>.tmp <path>'`, streaming the cached bytes on stdin: no `scp`, no second
   authentication, no second channel, and a `~/.ssh/config` jump host works because `ssh` is doing
   the work. Temp-and-rename is what stops a half-written binary from ever being executable. Under
   WSL the same shape runs through `wsl.exe`, into the distro's own filesystem rather than through
   `/mnt/c`, which is slow enough to be worth naming.
4. **Launch.** The same invocation with `--stdio`, and §4's handshake.

Ubiq never executes a binary on the far side that it did not just verify or just fingerprint.

## 10. Security

The brief asks for the channel and the protocol to be "super secure". Stating the threat model is
what makes that a design rather than an adjective — and the first thing the model says is that
**there are two of them**.

**The runtime build's threat model is nearly empty, and saying so is the point.** `wsl.exe` and
`docker exec -i` are process spawns by the same user on the same machine. No wire, no listener, no
token, no certificate, no third party. What remains is the supply-chain row below — the binary has
to be the right binary — and nothing else in this table applies to it. Every other row is about the
relay build and a network.

| Threat | Answer |
|---|---|
| Someone reading or altering the link in transit | The carrier's own cryptography — SSH, TLS, a tailnet. **The drone implements no cryptography of its own**, which is the only defensible position for a binary this small |
| Another local user on the far machine talking to the drone | There is nothing to talk to: the default carrier is a pipe between two processes, and no socket is bound. With `--listen`, a 256-bit token compared in constant time, handed over stdin at startup, loopback-bound, **never on `argv`**, where `ps` would show it to every user on the box |
| A tampered or substituted drone binary | §9: signed manifest, per-artifact hash checked before use, version-pinned key, temp-and-rename so a partial write is never executable |
| A drone reaching outside what it was asked for | The file group is confined to roots declared at `Hello` and resolved with the same two-layer check `files/path.rs` already implements — textual component rejection, then canonicalise-and-contain, with a symlink-leaf refusal on writes |
| Credential material crossing the link | It does not. **Accounts on the far machine are the far machine's.** Ubiq ships a reference — an account name the drone's own `FsAccountStore` resolves — and never material, which is the existing rule and not a new one. A harness that needs a login logs in *there*, in its own pane, the way the local login flow already works |
| A secret in a log | `Secret` from `messages.rs`, whose `Debug` writes `Secret(***)` and whose only reader is a deliberately-named `expose()` (`D65`). The drone's diagnostics go to stderr and never carry one |

And the bound on a drone's authority, said out loud: **a drone can do anything the user it runs as
can do on that machine.** That is what it is for, and the control is the carrier's own
authentication — if someone can open the channel they could have run the commands anyway — which is
why §3 prefers a carrier the user already authenticates and §8 refuses to open a second door beside
it.

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
on a lost socket. A drone does not, because the two are not the same kind of peer: a host is durable
and still owns its panes, sessions and catalogue when the socket comes back, so re-dialling reaches
them; a drone owns nothing that outlives the link, so a reconnect would reach an empty process.
Reconnecting to a drone is spawning a new drone, and spelling it that way is more honest than a
retry loop that looks like recovery.

**What it costs, said plainly.** A closed laptop lid kills every remote pane and every harness under
one, including a long agent run someone was depending on. Surviving that means a drone that outlives
its carrier — durable identity, a listening endpoint or a relay, and far-side state — which is a
different product with a different security posture, and is the one thing this proposal defers
rather than decides.

## 13. What can go wrong

- **The carrier's stdin is a terminal.** Refused at startup, before a frame is written (§3); the
  likeliest cause is an `ssh` invocation that lost its `-T`.
- **Versions disagree.** Refused at `Ready` with both named, never as a pane that starts and then
  misbehaves.
- **No drone exists for that triple.** The failure names the triple the probe found and the triples
  the manifest offers — a fixable complaint rather than a dead end.
- **The hash does not match.** Refuse; do not run, and do not silently re-download and retry. A
  mismatch is a corrupt cache or something worse, and both want a person.
- **No writable cache directory on the far machine**, or a `noexec` mount under it. Reported at the
  push step, distinct from "unreachable", so the user knows to name another path.
- **isol8 cannot confine on that platform.** Not an error: a capability absent from `Hello`, and a
  run that says it is unconfined (§7).
- **The link drops mid-work.** §12. One teardown, no partial state, no orphan.

## 14. Phases

**P0 — the contract and the stdio carrier.** The `drone` module, the handshake, the keepalive, the
pane group, the `drones` link owner and the `PaneBacking` seam; relay build only, placed by absolute
path, no manifest. Both carriers land together because they differ by argv alone, so this phase ends
with a terminal in a WSL distro *and* a terminal on an SSH host.

**P1 — machine and files.** The machine group, the file group with declared roots, and the remote
project field. **This completes the relay build**, and with it the whole remote-access use case.

**P2 — runs.** The `agents` feature: `agent-manager` embedded, the run group, accounts and profiles
resolved on the far side, §2's flag check, and `Isolation::None` advertised honestly on Linux (§7).
Deliberately ahead of distribution — a developer placing a binary by hand proves it, and the WSL
user is the one waiting.

**P3 — distribution.** The manifest, signature, cache, fingerprint probe and push, for both
artifacts. This is what makes a drone something a user gets rather than something a developer
places.

**P4 — the socket carrier.** `--listen`, TLS, the token, and a tailnet address as its first real
user. Relay only; the runtime build has no use for a socket and should not grow one.

**An upstream track beside all of it: isol8's descriptor seam** (§7). It gates confinement on Linux
and nothing else. Two questions are answered before the phase that depends on them rather than
during it — §6's io bridge before P2, and §7's route before any confinement work.

## 15. What this asks to be decided

| | Question | Recommendation |
|---|---|---|
| a | One contract for the drone, or reuse `Message`? | **`DroneMessage`, a second enum in `ubiq-proto`.** The coordinator translates; the UI contract never changes (§5) |
| b | Two projects, two crates, or one crate with a flag? | **One crate, one contract, two artifacts** behind an `agents` feature, with a recipe that fails if `agent-manager` reaches a default build. The postures differ by a dependency and a threat model, not a design (§2) |
| c | Does a drone run harnesses? | **The runtime build does**; the relay build never does and links nothing that could (§6) |
| d | Which isol8 route unblocks confinement in a pane? | **Route B, the exec shim** — it makes Linux look like macOS and leaves `pty/mod.rs`, the host and the drone untouched. Route A is the smaller isol8 diff and the right answer only if the Linux policy ever needs more than Landlock (§7) |
| e | Where does the structured-io bridge run? | **On the host**, with the drone relaying opaque bytes — *conditional* on `io::IoBridge` being drivable from a supplied reader/writer pair. Verify before P2 is scheduled; if not, the bridge moves to the drone and the contract grows a conversation group (§6) |
| f | Default carrier? | **stdio.** One code path for SSH, WSL, containers and local development, and no socket on the far machine (§3) |
| g | Does a drone ever listen? | **Only when told**, loopback by default, and **relay build only** — the reverse of `D80`, for a reason (§8, §10) |
| h | Reconnect? | **No, permanently.** A drone owns nothing that would survive one (§12) |
| i | Signed manifests before any release pipeline exists? | **Yes, with a local-directory dev repository as the unsigned exception**, compiled out of release builds (§9) |
| j | Does a drone answer search? | **No**, not even shelled out, in these phases. It is a capability the contract has room for and no phase claims |
| k | Which triples ship first? | **`x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`** — WSL and the common remote host, in that order. Everything else follows the manifest |

## 16. Rules this adds

- **The drone's stdout carries frames and nothing else.** Every diagnostic goes to stderr. One stray
  write desynchronises the stream permanently.
- **A drone's stdin is a pipe, never a terminal**, and a drone that finds otherwise refuses to start.
- **A drone binds no socket unless asked**, and binds loopback when asked without an address. The
  runtime build never asks.
- **The relay build links no `agent-manager` and no `isol8`**, checked by a recipe rather than
  trusted to a reviewer. One convenient `use` is all it takes for the small artifact to stop being
  small.
- **The contract does not change shape with a feature flag.** Both builds carry every variant; what
  differs is the capability set in `Hello`.
- **A drone writes into one directory, `--purge` removes it, and it dies with its channel taking its
  children with it.**
- **Ubiq never executes a far-side binary it has not just verified or just fingerprinted.**
- **No secret on `argv`**, and no credential material over the link at all — only references the far
  side resolves.
- **`DroneMessage` never reaches the interface.** A drone fact that must be drawn is translated by
  the coordinator into the existing contract.
- **A capability is advertised only when it works on that platform.** An absent capability is a
  truthful answer; an advertised one that silently does nothing is not.

## 17. What stays out

Beyond the groups §5 leaves out — git, search, index, watch, conversation:

- **A drone that outlives its carrier.** §12 is the argument; this is the deferral it makes
  deliberately rather than by omission.
- **An https-streaming carrier.** The only row in §3 that would need a second framing.
- **A harness reaching *out* from a local pane to a remote machine** — a `remote-file` and a
  `remote-shell` tool over the in-process MCP surface `crates/ubiq-host/src/mcp/` already shapes,
  pointed at a drone. A consumer of this, and worth its own proposal after P1.
- **A Windows-native runtime build.** WSL is the Windows story, and it is a Linux triple; §7's
  AppContainer note is why a native one would confine in name only. A Windows *relay* is a separate
  question with no use case yet.
- **A second protocol for the local case.** WSL and a remote host are one carrier apart and nothing
  else; a separate transport for the local boundary would be two of everything for no gain.

## 18. Rows this proposes for the backlog

| Row |
|---|
| No cross-compilation exists: `just bundle` and `just bundle-win` produce host-native artifacts and nothing builds a headless binary for a third triple |
| Nothing in the tree fetches, verifies or caches a binary from a manifest; `resolve_program` locates what is already on `PATH` and that is all |
| No message carries a protocol or schema version — true today only because both ends are always one build |
| `G90` already owns the confinement limitation and needs widening, not duplicating: it reads as a macOS-only gap in a local pane, and the runtime build makes it the blocker on the primary Windows path. Its framing also wants correcting — the obstacle is descriptor ownership, not Landlock, and §7's exec shim is a second route it does not name |
| Whether `agent_manager::io`'s bridge can be driven from a supplied reader/writer pair rather than a child it spawned decides where the structured-io parser lives, and therefore how large a drone is |
| The existing remote bus has no keepalive (`G189`); the drone contract carries `Ping`/`Pong` from P0 and the host bus still does not |

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the rules a drone must not bend, and the
  "remote harnesses" future it cashes in
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the framing this reuses and the
  pane and file families the coordinator translates into
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — the boundary a drone carries across a
  machine, and what `--no-default-features` leaves behind
- [`isolation-proposal.md`](./isolation-proposal.md) — the local half of the confinement argument
