---
id: wip-drone
title: The drone as a tool an agent can reach
kind: wip
status: current
summary: The working note for the drone's tenth phase — the drone as a fifth MCP server, with `drone_*` tools over the bus, capability-gated, a new `shell` message pair and a per-root read-only mode. Designed, and none of it written. Carries the gaps the nine built phases left open too, chief among them that nothing has been run against a real `ssh`, `scp` or `sshd`.
read_when: you are picking up the drone work — building phase 10, or closing one of the gaps the built phases left
updated: 2026-09-16
verified: 2026-09-25
code_anchors: [crates/ubiq-host/src/mcp/server.rs, crates/ubiq-host/src/mcp/catalogue.rs, crates/ubiq-host/src/mcp/tools.rs, crates/ubiq-host/src/mcp/registry.rs, crates/ubiq-host/Cargo.toml, crates/ubiq-proto/src/carrier.rs, crates/ubiq-drone/src/lib.rs, crates/ubiq-proto/src/messages.rs]
depends_on: [feat-drone, tech-transport, tech-architecture, inbox-drone-runtime]
---

# The drone as a tool an agent can reach

Phases 1 to 9 are built, and what they built is [`../features/drone.md`](../features/drone.md): the
lean host and the crate, the handshake and heartbeat, SSH profiles and their secrets, attaching over
`ssh`, detach-and-linger, managed drones, per-project origins, provenance and search. This note is
what is left — the tenth phase, designed and unwritten, and the gaps the nine left behind.

The design this work follows is [`../inbox/drone-proposal-v2.md`](../inbox/drone-proposal-v2.md),
whose §1 is unfinished, and the shelved
[`../inbox/backlog/remote-drone-proposal.md`](../inbox/backlog/remote-drone-proposal.md) it keeps
most of.

## Phase 10 — the drone as a tool the agent can reach

**Designed, not built.** Every built phase serves the *interface* — a tree, a file, a search hit
land in a window because a person clicked something. Phase 10 gives the *agent inside a pane* the
same reach, so the harness working on a local project chooses where a file lands rather than being
confined to the folder it was launched in. Stage one is local-project-plus-drone: the harness keeps
its own native file tools on the local folder and gains a second filesystem and shell it reaches
through MCP. A project served wholly by a drone, with no local folder beside it, is a later question
and is not designed here.

**It lands beside a catalogue that exists, not beside a second one.**
`crates/ubiq-host/src/mcp/` is Ubiq's own MCP server — not agent-manager's `inproc-mcp`, which
`crates/ubiq-host/Cargo.toml` takes `agent-manager` with `default-features = false` precisely to
leave off. One process-wide loopback listener in `mcp/server.rs`, a static table of servers and JSON
schemas in `mcp/catalogue.rs`'s `SERVERS`, dispatch in `mcp/tools.rs`'s `call`, injected per agent
as a plain `McpRef::Inline` HTTP server at `/mcps/<key>/<name>`, and offered per profile through the
settings checklist `Message::Mcps` feeds. A drone server is one more `ServerSpec` beside `test`,
`project-info`, `manage-ubiq-tasks`, `use-task` and `ubiq-kb`, and one more arm in `tools::call`'s
match.

**The drone MCP is a façade over the bus, not a new subsystem.** `D116` makes a drone an ordinary
host on the multi-host bus; this is one more client of it. The tool set maps onto messages that
exist: `info` onto the handshake's `DroneIdentity` plus `HostInfo` plus the drone's state record,
`ls` onto `ProjectTree`, `read` onto `ReadProjectFile`, `write` onto `WriteProjectFile`, `delete`
and `move` onto `EditProjectPath`'s `PathOp`, `search` onto `SearchProject`. Two things are
genuinely new.

**The drone MCP sees roots, not a filesystem** (`D136`). Every file-family message carries a
`ProjectId` and a `rel_path` rather than an absolute one, so a tool reading an absolute path does
not fit this shape and is not made to. A drone's roots are exactly the base folders its `info` tool
reports; confinement above a root is the file worker's existing path safety rather than a second
check invented for a model, and read-only is a property of a root rather than of a machine, because
one drone can serve a project meant for editing and a second meant only for reading logs. The cost
is real: an agent cannot touch anything outside a root, which is the point and also a refusal a user
will meet the first time they ask for something a level up.

**`shell` is a new message, never a scraped pseudo-terminal** (`D137`). A drone serves terminal
panes, and a pane is a byte stream with no exit code and no completion signal — the domain rule that
terminal bytes stay opaque exists for exactly this reason. A tool call needs stdout, stderr and an
exit code, and scraping a pane's screen for them is the thing that rule forbids, not a shortcut
around it. So: a `RunCommand`/`CommandFinished` pair, answered by an ordinary process on the host
side and gated behind a `shell` capability. It is not drone-specific — the local host answers it
exactly as a drone does — which is what lets `drone_copy` between a local project and a remote one
fall out as one tool composing two hosts' `read` and `write` rather than a transfer protocol
invented for the occasion.

**Tool calls round-trip the bus, and that is the exception this phase buys** (`D138`).
`mcp/tools.rs`'s own header states the rule the rest of the server holds to: most tools are answered
on the listener's own thread from the facts the URL resolved to, and none of them ask the
coordinator a question, because a harness waiting on that thread is a harness a busy coordinator can
stall. Every drone tool is a round trip by definition — there is no local fact to answer from, only
a question the far end has to answer. So the drone server holds a pending-call table with a timeout
and drives it off the bus as a client of its own, correlated by the echo keys the file family
carries for exactly this reason: `ProjectTreeListing` and `ProjectFileContents` echo their
`rel_path` so a late answer lands on the right row, and this is a second reader of that same design,
not a new one. Search is the one family that does not answer in a single message, so the table
carries an accumulating case as well: `drone_search` collects `SearchMatches` until `SearchFinished`
or `SearchError` ends it and answers once with what it collected, discards `SearchProgress` for want
of anywhere to stream it, times out over the whole search rather than between messages, and caps the
collection with a `limit` whose default the tool description states — sending `CancelSearch` on
reaching it, rather than letting the drone walk the rest of a tree whose matches are already being
thrown away.

**The tool list is built per run from advertised capabilities** (`D139`).
`crates/ubiq-drone/src/lib.rs`'s `capabilities()` argues this for the interface: naming a capability
the drone would only refuse is worse than naming none, because an optimistic list puts something in
front of a user — or, here, a model — that answers with a refusal every time. So no `drone_search`
without the `search` capability and no `drone_shell` without `shell`. This is the first real reader
of `DroneIdentity::has` outside a test — `G265` has stood open for exactly that reason, and this
phase is what closes it.

**Every tool takes an optional remote, and the single-remote user never types it.** Omitted, it
resolves in order: the project's `runs_on` drone first, since `DroneOrigin` sits on `ProjectRecord`;
then the only attached drone, if there is exactly one; then the drone flagged default in settings.
`AgentFacts` in `mcp/registry.rs` carries the project, so the first rule costs nothing beyond a
lookup on hand.

**Naming stays out of Ubiq's own vocabulary.** The tools are `drone_*`, a namespace nothing else in
the catalogue will collide with, but every description is written in ordinary remote-machine
language — "the remote machine", "its files" — and explains none of Ubiq's internals, because a
model has no prior for the word "drone" and every prior for "a remote machine."

**Safety has two gates and this phase adds neither a third nor a duplicate.** The harness's own
permission mode prompts before an MCP tool call, so there is nothing to invent at that layer. The
hard boundary sits at the drone: a read-only root refuses `WriteProjectFile` and `EditProjectPath`
at the drone itself, never only in the MCP layer sitting in front of it, because that layer is a
client like any other on the bus and a client-side check is a courtesy, not a permission. That needs
a per-root mode on the drone, surfaced in the drone's state record and echoed back by `info`.

**One loose end is a gap, not solved here.** `WriteProjectFile` carries `expected` for optimistic
concurrency and refuses an absent one on a file that exists rather than forcing an overwrite.
Threading a content hash through a model's tool call is awkward — a hash is not a value a model
tracks between turns — and the likely shape is a plain create call plus an explicit overwrite flag
on write, but which is undecided (`G268`).

## Working on this

The recipes are [`../tech/operations.md`](../tech/operations.md)'s — `just relay` for the lean host,
`just drone-build` and the two manifest recipes for the cross build and its pins, `just host` and
`just ui` for the crate-boundary guards, and `just verify` for the gate a change has to pass.
`cargo test -p ubiq-drone` runs 33 tests: 14 unit, and integration tests over managed drones,
detach, search, the handshake, a session and root ids.

The order that keeps a note like this true: code, then the decision register and the backlog, then
the feature document, then this note last. Written the other way round it records intentions rather
than facts.

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

## Known gaps

- **Nothing has been run against a real `ssh`, `scp` or `sshd`.** There is no ssh client and no
  `sshd` in the sandbox this was built in, so every ssh-side path — the argv, the askpass handshake,
  the deployer's four steps, adoption over the wire, the remote list and stop verbs — is verified by
  unit tests over its *inputs* and by nothing over its behaviour. That is the single largest
  untested surface in the drone, and phases 4 through 9 all rest on it. `G258` covers the askpass
  half, `G261` the deployer, `G262` adoption over ssh. The Check action is the smallest thing in the
  interface that exercises the real `ssh` argv and askpass path end to end, on one click against a
  real host — the cheapest way to start closing this, not a closing of it: it runs in the same
  sandbox as everything else here, and nobody has pressed it against a real `sshd` either.
- **The manifest is empty.** There is no cross toolchain here, so `just drone-build` has never run
  and the deployer refuses every triple with the sentence naming it. That refusal is the designed
  behaviour, and it means the deploy path has never produced a binary on a far machine. `G261`.
- **A drone that lingers out is announced only as a link that dropped.** Phases 7 and 8 closed half
  of this. Expiry itself says nothing — the panes go, and a `Session` host's reconnect loop silently
  starts a *new* drone, so a user whose laptop slept past the linger finds empty panes with no
  account of why. `G263`.
- **Nothing verifies a `MESSAGE_SCHEMA` bump mechanically** (`G256`), and **the heartbeat breaks a
  peer built before it** (`G257`). Both untouched. The number stands at 1 and is right to: `runs_on`
  is additive and `#[serde(default)]`, so an old drone omitting it decodes as the local project
  every such record meant, and `crates/ubiq-proto/src/wire.rs`'s own paragraph exempts exactly that
  case. Bumping would refuse every deployed drone for a change neither end can misread.
- **`DiffProjectFile` is refused**, and the `git` capability it would sit behind is advertised by
  nothing: cross-compiling `git2` per triple is unstarted. `G264`.
- The refusal set has seven messages with no error variant to answer with; they log and drop.
- The runtime build — `agent-manager` on the far side, for WSL and containers — is designed nowhere.
- **Phase 10 is designed and none of it is written.** `RunCommand`/`CommandFinished` does not exist
  (`G266`), a drone's root carries no read-only mode for `WriteProjectFile` and `EditProjectPath` to
  refuse against (`G267`), and whether `drone_write` creates only or also takes an explicit
  overwrite is undecided (`G268`).

## Three decisions that hold without a register row

Nothing about them is a trade anyone would reverse:

- **The carrier is the ssh exec channel's stdio**, with no forwarded port — which is why the shelved
  proposal's exec-time keypair defends nothing and is not built.
- **Persistence is a unix socket at mode `0600`**, never `tmux attach`: tmux is a terminal emulator
  and would mangle binary frames. A multiplexer only holds the detached process.
- **Provenance is hash-pinned, not signed.** There is no signing key and no release pipeline
  (`G108`); saying "signed" would imply provenance the tree does not have.

The fourteen that do have rows are `D116`–`D118`, `D124`, `D125` and `D131`–`D139`, argued with
their costs in [`../tech/decisions.md`](../tech/decisions.md).

## Related docs

- [`../features/drone.md`](../features/drone.md) — everything the nine built phases produced
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the carrier family, the file
  family's echo keys, and the search family a drone answers
- [`../backlog.md`](../backlog.md) — `G108`, `G149`, `G256`–`G258` and `G261`–`G268`
