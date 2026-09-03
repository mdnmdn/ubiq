---
id: inbox-drone
title: Proposal — the drone, shell and file access to a remote host
kind: proposal
status: proposal
summary: A small process Ubiq injects into a remote host over SSH, so a project's shell and file access can come from another machine through the same pane and file messages a local project already speaks — the version-and-identity handshake that fences it, how the binary gets there, and what stays out of scope.
read_when: you are deciding how a project's files or shell reach a machine Ubiq is not running on, what "drone" means in this tree, or which host messages a remote transport answers
updated: 2026-09-03
depends_on: [tech-architecture, tech-transport, tech-agent-manager, tech-structure]
---

# Proposal — the drone, shell and file access to a remote host

Ubiq hosts every harness on the machine it runs on. A project's folder, its shell, its git
repository — all of it is read by `crates/ubiq-host` reaching directly into the local filesystem and
spawning local pseudo-terminals. There is no notion anywhere in the tree of a project whose files and
shell live somewhere else.

This proposes the **drone**: a small binary Ubiq deploys onto a remote host over SSH, which then does
for that host what the coordinator's `pty/` and `files/` modules already do for this one — spawns
PTYs, reads and writes files, walks a tree — and answers for it over a connection back to the Ubiq
that put it there.

## 1. This is not a new idea in this tree — it is a deferred one

Three places already name exactly this, unprompted by this document.

**The backlog has been carrying it as a deferred item.** `_docs/backlog.md`'s Deferred table: "D2 |
Harnesses on remote hosts | Same reason, one step further out" — "the same reason" being D1's, that
the transport contract makes the coordinator's process boundary cheap to move later because neither
half speaks anything but the contract. This proposal is what stops it waiting.

**The architecture document already describes the shape.** From
[`../tech/architecture.md`](../tech/architecture.md), "Why the split is drawn before it is needed":
*"A harness running on another host or in a container is structurally the same problem as a terminal
stream crossing a machine boundary. The coordinator stops assuming the pseudo-terminal is local; the
per-pane stream arrives over a network transport. The contract is identical, because a pane was
always a tagged bidirectional byte stream plus control messages."* That is this proposal's §3,
written before it, at the level of a design commitment rather than a hint.

**The transport contract already names the file family's seam for it.** From
[`../tech/transport-contract.md`](../tech/transport-contract.md), in the file family: *"This is the
file-level form of the rule that the UI never assumes the pseudo-terminal is local, and it is the
seam a remote drone slots into: a project id and a relative path do not say which machine
answered."* The word "drone" is already in the tree, naming the same thing this document designs.

**What is missing is everything underneath those sentences.** No SSH client dependency exists in any
manifest. No message in `crates/ubiq-proto/src/messages.rs` carries a protocol version — the bus has
never needed one, because both ends are always compiled from the same commit in the same process,
which stops being true the moment a drone can be built from a different one. `just bundle`
(`Justfile:26`) assembles one `.app` for the machine running it and nothing cross-compiles or
publishes a headless artifact. `_tools/` has no download step of any kind. This proposal is the
design for closing that gap, not a new direction.

## 2. What a drone is, and is not

**A drone is a coordinator's `pty/` and `files/` modules, cross-compiled to run alone on someone
else's machine, answering the same two message families over a socket instead of a function call.**
It renders nothing, has no opinion about layout, and carries no harness knowledge: exec'ing a program
by name is a shell starting a shell, the same rule `agent-manager.md` states for the local coordinator
— *"A spawn naming anything else is a program name, which is what a shell is."* **A drone never runs a
harness, and that is permanent rather than a v1 gap:** a composed run — skills, MCP servers, an
account — is `agent-manager`'s to build, and `agent-manager` has no story for a machine it is not
running on, and gains none here. What a future version adds is not a harness on the far end of a drone
connection, but the opposite direction — a harness Ubiq already runs locally, handed a `remote-file`
and a `remote-shell` tool that call back into a project's drone. That is the MCP-connector future in
§12, and it is the only way this design ever lets a harness touch a remote host.

**A drone is not a second host process, and it does not get its own message family.** §3 is the whole
of why: the pane and file families already carry everything a remote shell or a remote file read
needs, because neither one says which machine answered. Building a parallel "remote" protocol would
mean two ways to open a shell and two ways to read a file, which is the outcome
[`../tech/transport-contract.md`](../tech/transport-contract.md) and `D3` exist to prevent.

## 3. Reusing the contract, and the one thing it does not yet carry

**A remote project is a `ProjectRecord` with a `remote` field, not a second kind of project.** Every
message in the pane, file, session and git families already resolves through a project id and a
relative path; none of them needs to change. What changes is entirely inside the coordinator: where
`SpawnWorkspace` today calls `pty::spawn` and `ReadProjectFile` today calls into `files/mod.rs`, a
project carrying a `RemoteOrigin` (host, user, remote root) routes the same request over that
project's drone connection instead. The interface draws a remote project exactly as it draws a local
one — the same explorer, the same tabs, the same terminal panes — because nothing about `AppState` or
`crates/ubiq/src/ui/` needed to learn that a machine boundary exists. That is rule 2 of
[`../tech/architecture.md`](../tech/architecture.md) cashed in rather than bent: *"No path, no process
handle, no file descriptor crosses into UI code."* A drone is the thing on the other end of a stream
the UI already only ever holds by ID.

**The one thing the contract does not carry, and now must: a version.** Every message today assumes
both ends were built from the same tree, which the in-memory bus makes true by construction. A drone
is compiled once, at some Ubiq version, and kept running for as long as its SSH session lives — it
can easily be older or newer than the Ubiq that reconnects to it after an update. So the very first
thing on a drone connection, before any pane or file message, is a handshake:

```
DroneHello   drone → Ubiq   { drone_version, os, arch, message_schema, capabilities[], auth_proof }
DroneReady   Ubiq → drone   { ubiq_version, message_schema, accepted: bool }
```

`message_schema` is a single integer bumped whenever a variant in `messages.rs` that a drone must
speak changes shape — not the whole enum's version, since most of it (work, conversation, search) a
drone never touches. A mismatch is refused before a single pane or file message crosses, with a
message the status bar can show, rather than a `PaneError` that looks like a crashed shell.
`capabilities` is where "does this drone have git" and "which search tool did it find on the remote
host" travel — see §8 and §12 — so a coordinator never has to ask and a drone never has to be built
two ways.

This handshake is new wire surface, but it is one pair of messages, not a family: it exists once per
connection, is answered by the coordinator alone, and never reaches the UI.

## 4. Identity: an ephemeral key at launch, not a key baked into the binary

The prompt that seeded this proposal asked for a key or certificate patched into the executable's own
payload before upload, so a drone could only ever speak to the Ubiq that deployed it. That is real
security work worth having, but it is not what v1 needs, for a reason worth stating plainly: **the
SSH session that puts the drone on the host is already an authenticated, encrypted channel**, and
handing the drone a fresh secret over that channel costs a great deal less than patching a
per-deployment certificate into a cross-compiled binary before every upload.

**What v1 does instead.** Ubiq mints an ephemeral asymmetric keypair per deployment, holds the private
half, and passes the public half to the drone over the SSH `exec` channel's **standard input** — never
as a command-line argument, because `argv` is visible to every other local user on the remote host
through `ps`, and a secret in `argv` is a secret in a place this document would otherwise have to
defend. The drone holds it in memory only, never writes it to disk, and every message after
`DroneHello` on that connection is authenticated against it — concretely, the handshake upgrades the
socket to a channel keyed by that pair (TLS with the pinned key, or a Noise handshake; the choice is
implementation, not design) before the pane and file families are allowed to cross it.

**Why authentication is still needed at all, given SSH already encrypted the pipe.** Because SSH
secures the *tunnel*, not who may use the socket at either end of it. §6 below rides the drone's
connection over an SSH port forward, and a forwarded local port is a socket anything else running on
that machine can dial. Without the handshake, any local process on the remote host that connects to
the forwarded port before the real drone does would be answered as if it were the drone; with it, a
connection that cannot prove it holds the key Ubiq handed this deployment is refused before it can
send a `TerminalInput` or a `WriteProjectFile`.

**The binary-embedded certificate becomes worth its cost for exactly one case v1 does not need to
solve: a drone that must go on answering after the SSH session that launched it has closed.** An
exec-time secret dies with that session. A drone meant to be long-running or reachable
intermittently — reconnecting over §6's future relay path rather than a live SSH tunnel — needs an
identity that survives past the moment it was deployed, which is what a payload-embedded key buys and
an exec-time one cannot. Filed as `G111` in §14, deliberately deferred rather than dropped.

## 5. Getting the binary onto the host

**Ubiq shells out to the system's own `ssh` and `scp`, rather than embedding an SSH client.**
`D49` already sets this precedent for the local coordinator — a shell pane execs the real `zsh` or
`bash` on the machine rather than Ubiq reimplementing shell startup — and the same trade holds here
with a wider margin: `ssh` and `scp` on the user's machine already carry host-key verification,
every authentication method the user has configured (keys, an agent, a certificate, a hardware
token), `~/.ssh/config` aliases and jump hosts, for free and already trusted by the user. A Rust SSH
crate would have to re-earn every one of those, and get them wrong in ways a user cannot fix from
Ubiq's settings screen.

**This is the opposite call from `D43`, and the difference is worth naming rather than leaving as a
tension.** `D43` rejected shelling to `git diff` because the host would then own a text format's every
corner case — a missing, old or oddly-configured `git` changes what the parser sees, silently. Nothing
here asks `ssh`/`scp` for structure: deploying a drone wants an exit code, a copied file and a byte
stream to relay, never a format to parse. The two decisions agree rather than conflict — shell to the
real thing when what it hands back is exit-code-and-bytes; own the work in the host when what it hands
back needs to be understood.

**The flow, once a user names a host and a remote path:**

1. **Probe.** `ssh <target> uname -sm` — one round trip — tells the coordinator which artifact to
   fetch: `linux-x86_64` covers the common case, `linux-arm64` the next most common.
2. **Resolve the binary.** A drone build for that OS/arch, checked in this order: a copy Ubiq's own
   bundle already carries for the platforms it ships (once §12's packaging work exists); else a cache
   under Ubiq's config root, keyed by version and target; else a fetch from this Ubiq build's own
   GitHub release, named `ubiq-drone-<os>-<arch>` and pinned to the running Ubiq's own version tag —
   pinned because `DroneHello`'s `message_schema` is only useful if the two ends were never going to
   agree by accident.
3. **Upload.** `scp` the binary to a throwaway path under the remote user's own cache directory —
   the remote-host analogue of the throwaway configuration directory `agent-manager` already mints
   locally per run.
4. **Launch.** `ssh -R <remote_port>:localhost:<local_port> <target> '<path>/ubiq-drone'`, writing
   the public half of §4's keypair to that command's stdin once the process starts. `-R` is what
   makes the drone the one that dials — see §6 — and is why the exec line needs no address or port
   baked into it beyond what SSH itself is already forwarding.
5. **Handshake.** The drone connects to its own `localhost:<remote_port>`, which `sshd` tunnels back
   to the `<local_port>` Ubiq is listening on; `DroneHello`/`DroneReady` run; the project attaches.

Steps 1–3 are skipped on every deployment after the first to the same host and version — the drone's
own throwaway path is checked before a fresh upload, the way a cached run directory already is
locally.

## 6. The transport: riding the tunnel first, a connection-independent path later

**v1 rides the SSH session, both directions supported as a fallback pair, because sites differ in
which one their `sshd` allows.** `-R` (remote forward — §5's default) makes the *drone* the one that
opens the socket, which means it takes no address to reach and works from behind whatever NAT put the
remote host there in the first place; some `sshd` configurations disable `AllowTcpForwarding` for
just this direction. `-L` (local forward) is the fallback: Ubiq forwards a local port to a port the
drone itself binds on the remote host's loopback interface, which needs no `GatewayPorts` setting and
is the more commonly allowed direction. Ubiq tries `-R` first and falls back to `-L` on a refused
forward, which is one flag on the same `ssh` invocation and no second code path — both still ride one
SSH session, and both still terminate when that session closes, which is the honest v1 boundary: a
drone's connection is exactly as durable as the tunnel that carries it, the mirror of `D22`'s "closing
a pane kills its harness" applied to a machine rather than a process.

**What this defers rather than builds.** A drone that must go on answering after the launching SSH
session ends needs a transport that does not depend on that session at all — a direct outbound
connection to an address Ubiq (or a relay) holds open, authenticated by §4's future embedded
identity rather than an exec-time key. That is real infrastructure — a listener with a stable address,
or a relay service, neither of which exists today — and is exactly the class of thing the "future
features" list this proposal was handed already named: HTTPS, a custom relay, a mesh network over
WireGuard or Tailscale. Naming it here is what keeps §4's embedded-key deferral and this deferral
consistent with each other: they are the same future feature seen from two sides, filed together as
`G111`.

## 7. Shell access

A remote pane is `SpawnWorkspace` naming a remote project, exactly as today, with `agent_type`
resolving to a program name on the *remote* host's `PATH` rather than the local one's — `ListShells`
against a remote project asks the drone rather than the local `shells::is_shell()` probe, and answers
with what exists there. From that point the pane family needs nothing new: `TerminalOutput`,
`TerminalInput`, `TerminalResize`, `Focus`, `PaneExited` and `PaneError` all already carry a `pane_id`
and opaque bytes, and the coordinator's job is unchanged — read a stream, write a stream, propagate a
resize — it is only reading and writing a drone's forwarded socket instead of a local pseudo-terminal.
The drone's own `pty/`-equivalent owns the actual remote PTY and answers those five messages in the
same shape the local coordinator already produces them in, which is what makes this section nearly
empty: the design work is §3 and §4, and this is what falls out of them for free.

## 8. File access, and why a drone never holds an index

`ProjectTree`, `ReadProjectFile`, `WriteProjectFile` proxy the same way — a remote project's drone
walks and reads on demand, one directory at a time, on exactly the discipline `D35` already chose for
the local host: no eager walk, no persistent index, a `node_modules` costing one row and no recursion.
**Correctness needs nothing more than what `D35` already proves works for a large local repository**,
so a remote project behaves like any other project the moment §3 and §4 are in place, and P2 ships
with no index of any kind.

**A drone holding a warm index is the wrong shape to reach for at all, not merely one to defer.** The
whole premise of injecting a small binary over SSH rather than a full remote-development agent is that
the drone stays a guest on someone else's machine — the class of tool this replaces routinely holds
several hundred megabytes to a few gigabytes of resident index for exactly this job, which is the cost
this design exists to avoid. A remote project scoped to a repository can afford it; a remote project
scoped to a user's home directory for a maintenance task — the other real use of shell-and-file access
to a host — cannot, and a design that only behaves well in the narrow case is a design that surprises
someone the day they point it at the wide one.

**So the `search` capability, when it lands, shells out rather than indexing.** A drone that advertises
`search` in `DroneHello.capabilities` probed the remote `PATH` once at startup for `rg`, then `ag`,
then falls back to `find` piped through `grep` — the same "shell to the real thing already on the
machine" call `D49` makes for a login shell, applied a second time in this document. `SearchProject`
against a remote project runs that probed command and streams rows back the same shape a local search
answers in; nothing is held between requests, and closing the last pane against that project leaves
the drone holding no more state than it held before the search ran. The search family (`SearchProject`)
is otherwise **out of scope for this proposal** — landing the capability is P4's, in §11 — and a remote
project with no search-capable tool on its `PATH` simply does not offer that toolbar action, the same
way an unavailable `AgentTypeInfo` is offered and not pickable rather than pretending to exist.

**A persistent, host-side file index stays out of scope entirely**, not deferred to a later phase — see
§12.

`DiffProjectFile` against a remote project's git repository is deferred to its own phase (§11, P3)
rather than shipped with the rest of file access, because `D43` already settled — for the local
host — that shelling out to `git` is the wrong way to get a diff, and cross-compiling `libgit2` for
every drone target is real work that should not gate shell and file access on landing first.

## 9. What can go wrong

- **The SSH session drops mid-work.** Every pane on that project answers `PaneError`, every in-flight
  file request answers `ProjectFileError`, and the project's health (the same `ProjectHealth` a
  missing local folder already reports) turns `Unreadable` with the reason. Nothing auto-reconnects
  in v1 — the user re-attaches, which re-runs §5 from the cached-binary step.
- **A stale drone answers with the wrong `message_schema`.** Refused at `DroneReady`, before any pane
  or file message, with a status line naming the mismatch rather than a pane that looks like it
  started and then silently misbehaves.
- **Something else answers the forwarded port before the real drone connects.** Refused by §4's
  handshake — an unauthenticated peer never gets past `DroneHello`.
- **The remote host has no `ssh`-forwardable path at all** (both `-R` and `-L` refused by policy).
  Reported as a specific failure at step 4 of §5, distinct from "host unreachable," so the user knows
  what to ask their administrator for rather than guessing.
- **The remote root the user named does not exist, or is not a directory.** The same `ProjectHealth`
  probe that already runs for a local `AddProject` runs through the drone instead — no new failure
  vocabulary needed.

## 10. What this asks to be decided

| | Question | Recommendation |
|---|---|---|
| a | SSH client: shell to the system `ssh`/`scp`, or a Rust SSH crate embedded in `ubiq-host`? | **Shell out**, `D49`'s precedent — every auth method, host-key check and `~/.ssh/config` alias for free. |
| b | Does a remote project get a new record type, or a field on `ProjectRecord`? | **A field.** `remote: Option<RemoteOrigin>` on the existing record; every message that resolves a project by id keeps working unchanged. |
| c | Does a remote project persist in the catalogue the same as a local one? | **Yes** — same `projects.toml`, `D29`'s discipline, nothing project-shaped needs a second store. |
| d | Where does the drone binary come from before Ubiq itself has a release pipeline? | **Genuinely blocked** — `just bundle` produces one unsigned local `.app` and nothing cross-compiles yet. Filed as `G108`, and this proposal's P1 cannot ship ahead of it. |
| e | Auto-reconnect when the tunnel drops? | **Not in v1.** A dropped drone behaves like a dropped local process; reconnecting is a gesture, not a background retry loop. |
| f | Does `-R` or `-L` try first? | **`-R` first**, falling back to `-L` on a refused forward — one flag, one code path, covers both common `sshd` policies. |

## 11. Phases

**P1 — shell access.** The `RemoteOrigin` field, shelling to `ssh`/`scp`, §4's exec-time keypair,
`DroneHello`/`DroneReady`, and the pane family proxied over a drone connection. No file family yet.
This is blocked on `G108` — a cross-compiled, versioned drone artifact has to exist before step 2 of
§5 has anything to fetch.

**P2 — file access.** `ProjectTree`, `ReadProjectFile`, `WriteProjectFile` proxied, walked on demand,
no index. Closes the "file access" half of the original ask.

**P3 — git.** `DiffProjectFile` against a remote repository, with `libgit2` cross-compiled per drone
target on `D43`'s discipline, advertised as a `DroneHello` capability rather than assumed present.

**P4 — hardening.** The binary-embedded identity of §4, the connection-independent transport of §6,
and the `search` capability of §8 — probing the remote `PATH` for `rg`/`ag`/`find` and shelling to it,
never building an index.

## 12. Explicitly out of scope

Named by the request that seeded this proposal, and kept out on purpose rather than forgotten:

- **An MCP/tool connector giving a local, `agent-manager`-composed harness a `remote-file` and a
  `remote-shell` tool.** This is a *consumer* of what P1–P3 build, not a drone feature — it is the
  in-process MCP surface `crates/ubiq-host/src/mcp_server.rs` already carries the shape for
  (`agent-manager.md`'s `inproc-mcp`), pointed at a project's drone instead of its local files. The
  harness itself never leaves the local pane; only the two tools reach across. Worth its own proposal
  once P2 has shipped.
- **Windows remote hosts.** `D3` in `_docs/backlog.md`'s Deferred table already says the same of Ubiq
  itself: "the pseudo-terminal layer is cross-platform; nothing has been run there." A drone target
  is a cross-compilation problem before it is a design one.
- **Transports beyond SSH** — HTTPS, a custom relay, a mesh network over WireGuard or Tailscale. §6
  names the one case that needs them (a drone outliving its launching session) and files it as
  `G111` rather than designing three transports nothing yet needs.
- **A persistent, host-side file index.** §8 argues this is the wrong shape for a guest process, not
  merely a later one: search shells to whatever the remote host already has rather than building and
  holding state of its own, on every project this design is pointed at, narrow or as wide as a home
  directory.

## 13. Rules this adds

- **A remote project is a field, not a second kind of project.** Every message family keeps resolving
  through `project_id` and `rel_path`; only the coordinator's dispatch learns a project can answer
  from elsewhere.
- **No path, no process handle, no file descriptor named by a remote host crosses into UI code**,
  the same rule architecture rule 2 already states — a drone's connection is held by the coordinator
  alone.
- **A drone connection is authenticated before it is trusted**, and versioned before it is used.
  `DroneHello`/`DroneReady` runs on every connection, ahead of the first pane or file message.
- **A secret is never passed as an argument.** Anything a drone must hold — today's exec-time key,
  and any future embedded one — travels on stdin or inside the binary's own payload, never on `argv`.

## 14. Rows this proposes for the backlog

| Id | Row |
|---|---|
| G107 | No SSH/remote transport exists; a project's shell and file access assume the same machine as `crates/ubiq-host` — `pty/` and `files/` reach the local filesystem directly |
| G108 | No cross-compiled drone artifact and no release pipeline producing per-arch binaries; `just bundle` builds one unsigned `.app` for the machine running it and nothing distributes a headless binary |
| G109 | The message set carries no protocol or schema version — every message assumes both ends were compiled from the same commit, true today only because the bus is in-process |
| G110 | `ubiq-host` has no SSH client and no path that shells to the system `ssh`/`scp`, so nothing in the tree can reach a second machine at all |
| G111 | A drone's identity is exec-time only, and its transport dies with the SSH session that launched it — no binary-embedded identity and no connection-independent transport exist for a drone meant to outlive its deployment |

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — rule 2, and the "remote harnesses" future this proposal cashes in
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the pane and file families this reuses, and the framing rules a socket transport must honour
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — why a drone carries no harness knowledge, and where the future MCP connector's precedent lives
- [`../tech/decisions.md`](../tech/decisions.md) — `D49` (shelling to a real shell), `D43` (why not shell to `git`), `D22` (closing ends a harness)
- [`../backlog.md`](../backlog.md) — `D2` in the Deferred table, which this proposal is what stops deferring
