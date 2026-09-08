---
id: tech-agent-manager
title: The agent-manager boundary
kind: tech
status: draft
summary: What the embedded harness-management library owns, what Ubiq owns, how the application consumes it, and the rule that keeps the two from growing into each other.
read_when: you are about to write code that launches a harness, drives one as a conversation, names a harness config path, or touches accounts, skills or MCP servers
updated: 2026-09-08
verified: 2026-09-08
code_anchors: [crates/ubiq-host/Cargo.toml, crates/ubiq-host/src/agent.rs, crates/ubiq-host/src/conversation.rs, crates/ubiq-host/src/coordinator.rs, crates/agent-manager/src/lib.rs, crates/agent-manager/src/session.rs, crates/agent-manager/src/harness/mod.rs, crates/agent-manager/src/provision.rs, crates/agent-manager/src/spec.rs, crates/agent-manager/src/resolve.rs, crates/agent-manager/src/profile.rs, crates/agent-manager/src/isolate.rs, crates/agent-manager/src/io/mod.rs]
depends_on: [tech-structure]
review_cycle: monthly
---

# The agent-manager boundary

## What the library is

`agent-manager` wraps a running harness. Rather than executing `claude` directly, it composes a run
— skills, MCP servers, an account, initial instructions, hooks, all pulled from a catalog — into a
throwaway configuration directory, optionally inside an isolated environment, and launches the real
binary against it. The user's own `~/.claude` and its siblings are read-only for the duration.

It ships two front ends: an `am` CLI for the terminal, and a front-end-agnostic library for
embedding. Ubiq is the embedder.

Its full documentation lives with the crate, starting at `crates/agent-manager/_docs/README.md`.
**That library owns every fact about harness configuration.** This document owns only the boundary.

## The division

| Fact | Owner |
|---|---|
| Where a harness stores its config, and in what format | the library |
| How to launch a harness, and with which arguments | the library |
| What a run is composed of — skills, MCPs, account, instructions, hooks | the library |
| What a saved definition can pin, how it inherits, and where it is stored | the library |
| The permission modes a harness has (`Harness::modes`), and how `Profile.mode` reaches a policy | the library |
| Which executable this machine runs for a harness, when it is not the harness's own name | Ubiq |
| Which definition a conversation starts from, and the form that writes one | Ubiq |
| Which accounts exist and how credentials are referenced | the library |
| Session history and resume, as the *harness* understands it | the library |
| Which files are a harness's own record of a conversation, and the on-disk shape a record is written in | the library |
| Where a run's record is kept, under whose id, and when it is written | Ubiq |
| How a harness's I/O is bridged into structured events, and what those events are called | the library |
| The one translation from those events onto the bus | Ubiq |
| What a policy grants, and how the operating system enforces it | the library |
| Which policy layers a confined run stacks, and which of them are unusable | the library |
| Whether an agent is confined at all, and where its run directory lives | Ubiq |
| Which `$HOME` a confined agent runs with, and which folders it may reach beyond the policy | Ubiq |
| That a harness runs under a pseudo-terminal in a pane | Ubiq |
| Which panes exist, which is focused, how they are laid out | Ubiq |
| A session as a *user's* piece of work, with a home folder | Ubiq |
| The window, the theme, the chrome | Ubiq |

The overlapping word is **session**, and the two meanings are genuinely different. The library's
session is a harness conversation that can be resumed. Ubiq's session is a named grouping of panes
with a folder. A document that means one must say which.

## How Ubiq consumes it

The library exposes a run as a value — a `RunSpec` in `crates/agent-manager/src/spec.rs` — built
from flags, settings and catalog contents, then handed to a provisioner that materialises the
configuration directory and produces a launch. Ubiq's coordinator constructs that value
programmatically instead of parsing command-line flags, and spawns the resulting launch under a
pseudo-terminal it owns.

`crates/ubiq-host/src/agent.rs` is the whole of that consumption, and it is deliberately thin. The
agent-type list is `harness::all()` projected into `AgentTypeInfo`, each row marked with whether the
harness's own binary is on this machine or a command override is configured for it. A spawn naming
one of those ids composes a run, provisions it, and answers with what to exec. A spawn naming
anything else is a program name, which is what a shell is.

**What this machine runs for a harness is Ubiq's answer, applied where the launch is resolved
rather than by changing the library.** The library says what a harness is called and how to launch
it; `Agents::resolve_program` in `crates/ubiq-host/src/agent.rs` is the seam, already there to turn
a bare `claude` into an absolute path, that now also checks `HostSettings.agent_commands` first. An
override for the harness splits into words — quote-aware, backslashes preserved so a Windows path
survives — the first word becomes the program (looked up on the login shell's `PATH` only when it
has no path separator), and the rest is prepended to the arguments the library already composed. No
entry for a harness resolves exactly as before, and `crates/agent-manager` is unchanged either way:
which binary a harness is called is still the library's fact, an override is a fact about this
machine.

**Ubiq does not build the `RunSpec` itself — `resolve` does.** `agent.rs` calls
`agent_manager::resolve::resolve` with a `RunFlags` naming only the harness and the folder, and
overrides exactly four fields of what comes back: the configuration directory (Ubiq owns where a
run's state lives), the I/O mode (Ubiq owns which face the workspace wears), the isolation
(Ubiq's own settings own the toggle, and it applies to a conversation exactly as to a pane), and —
when that isolation is on — the permission mode, because a confined run is contained by the sandbox
rather than by the prompts and would otherwise stop on every ask the sandbox has already answered.
Ubiq names no mode to do it: `Harness::unattended_mode` is the library's own word for which of its
`modes()` means "ask nothing" (`None` for a harness that has no such mode or already asks nothing),
and an explicit mode picked for this run outranks it — a profile's `mode` does not, being a default
under the same toggle. Everything else — which account, which model, which skills and MCP servers, which config overlays — is the library's
answer, read from the profile that names them. So an account reaches a pane without `agent.rs`
learning what an account is, and a harness that grows a new composition knob needs no change here.

The stores `resolve` reads are the filesystem defaults, each rooted under Ubiq's own config root so
a development run never touches what the `am` CLI manages: `<root>/accounts`, `<root>/profiles`,
`<root>/catalog`. A missing directory is an empty store, not an error, so this resolves on a machine
that has configured nothing. The library's own settings file is deliberately **not** read — Ubiq's
settings are the settings surface, and a second file answering the same question is a second
answer — which leaves `resolve`'s precedence as flags, then the profile.

**A configured harness entry is a `Profile`.** The pair a user thinks of as "Claude Code, work
account" is `agent_manager::profile::Profile` with its `harness` and `account` set, and the agent
layer that comes later is the same type with `defaults.instructions` filled. A `Profile` also
carries a `mode` beside its `isolate` — the harness-native permission mode, which `resolve` reads
into `spec.policy.permission_mode` under a flag and above nothing, so it sits on the profile rather
than in `ProfileDefaults`: it is a policy axis, not a composition input. The profile named `default`
is what a run with no explicit selection resolves to.

**Ubiq writes the form over profiles and none of the mechanism behind them.**
`crates/ubiq-host/src/agent.rs` reads and writes them through an `FsProfileStore` rooted at
`<root>/profiles` — `profiles()` projects each into a `ProfileInfo` for the wire, skipping any that
pins no harness, and `save_profile()` folds one back into a `Profile` and calls
`FsProfileStore::save`. The store owns the on-disk shape, the id and the resolution; the host owns
only where the root is. There is no delete, because the library offers none: adding a `remove_dir_all`
here rather than a `delete` there is exactly the shape rule 1 forbids — [`../backlog.md`](../backlog.md).
The four fields the interface can set are the harness, the account, the model and the mode; the
skills, MCP servers, hooks, instructions, isolation and `extends` chain a `Profile` can carry are
still written by hand, because nothing lists the catalog on the wire.

**A workspace has two faces, and `agent.rs` composes both.** `Agents::compose` is the terminal one:
`IoModes::Passthrough`, and a launch to exec under a pseudo-terminal. `Agents::converse` is the
other: `IoModes::Structured`, and a `structured_bridge` over the harness's own JSON instead of a
launch, because a conversation's harness writes frames on a pipe rather than drawing a screen. What
differs between them beyond the mode is the run directory's name and the isolation, both below.

**A bare run with no account and no profile still reuses the login already on the machine.**
`seed_zero_config_login` in `crates/agent-manager/src/provision.rs` runs after profile resolution
finds no login named, and tries two tiers in order: first, copy the harness's own
`Harness::config_anchor().login_seed` files out of the real `$HOME` — correct for every harness
whose credential is a plain file, since that is the same file the harness itself reads. If that
copy places nothing, it falls back to `Harness::ambient_login()`, a harness's own account of its
live login when that login is **not** a `$HOME` file the first tier could ever find — Claude Code
overrides it to read the OAuth blob the macOS Keychain holds, which is where it actually keeps a
session rather than in `~/.claude/.credentials.json`. Either tier is skipped once a login has
already landed from an account home or a profile overlay, and `ambient_login`'s default is `None`,
so a harness that keeps no such out-of-band login is unaffected.

**A login the run refreshes is written back to where it was seeded from, at teardown.** An OAuth
refresh rotates the refresh token, so once a harness rewrites the copy in its run directory the
original is revoked — deleting that directory would log the user out everywhere. `Agents::archive`
calls the library's `harness::harvest_login` before either teardown path removes a run directory,
and it is the one place both `retire` and `sweep` pass through. The origin it writes to is recorded
on the run's `SessionMeta::login_home` rather than held in memory, because nothing holds the
`Composed` that long; a login that came from no directory at all (Claude Code's macOS Keychain) is
found again through `Harness::ambient_login` and stored through `Harness::adopt_login`. Only the
files the harness marks `SeedFile::credential` travel back — the identity and onboarding state a
login also seeds picks up a run's own project history, and must not reach the user's real file.

**The bridge is owned by a pump thread, and `crates/ubiq-host/src/conversation.rs` is that thread.**
`IoBridge::next_event` blocks and both its methods take `&mut self`, so whoever reads a bridge
cannot also be handed a prompt; the reader owns it and a turn reaches the harness through the
detached `AgentInputSink` the bridge hands out. Events reach the window on the same unbounded
mailbox a pseudo-terminal's reader uses, so a window behind on drawing never stalls the harness.

**A detached `AgentKill` is the second handle out of a bridge, for the same reason the first one
exists.** Asking a harness to shut down is the graceful way out and a harness that does not act on
the ask keeps the pump thread waiting; nothing else can reach the child, because the pump owns the
bridge. So
`IoBridge::killer` hands out a kill-by-pid over the process the library spawned — the library keeps
naming how a harness is started and stopped — and Ubiq's `Conversation::abort` is what uses it,
behind `AbortConversation`. The pid cannot go stale: a bridge holds its child unreaped until it is
dropped, and reaping is still the bridge's own teardown.

**Every harness the library returns a structured bridge for can hold a conversation; not every one
of those keeps its process alive across turns.** `IoSupport::structured` is the first question —
`Agents::converses` answers it, and a harness that fails it (Grok) is refused as a conversation
outright, with a message naming the reason, and left out of the chat-start menus by
`AgentTypeInfo::chat` on the wire. `IoSupport::multi_turn` is the second, narrower question, and
`Agents::multi_turn` answers it: whether one process takes a second prompt over the bridge's own
`AgentInputSink`, true for Claude Code and codex. A **one-shot** harness (opencode, Copilot) answers
`multi_turn: false` and still converses — its prompt is argv rather than a pipe write, one process
answers exactly once and exits, and that exit is a turn ending, not the conversation's. The
coordinator starts a one-shot harness's pump `quiet`, so it never announces `ConversationEnded`
on its own; when the process exits, `finish_one_shot_turn` in `crates/ubiq-host/src/coordinator.rs`
carries the transcript's sequence counter forward, keeps the harness's own session id off
`Conversation::session_id()` (populated from `AgentEvent::SessionStarted` as the pump sees it), and
puts the conversation back into `pending_conversations` rather than ending it. The next prompt
relaunches the harness with that id as `RunSpec::resume` and the new text as
`RunSpec::initial.prompt` — `ConverseOptions` in `crates/ubiq-host/src/agent.rs` carries both
through `Agents::converse`. A harness that names no session id is relaunched anyway and answers with
no memory of the turn before it, which is `G95`.

**One file knows both vocabularies.** `map_event()` in the same module is the only place that names
`agent_manager::io::AgentEvent` and `ubiq_proto::conversation::ConvUpdate` together. Both are the
Agent Client Protocol's `session/update` vocabulary — `D53` — so the translation is a rename, and
confining it to the host is what keeps the interface free of any dependency on this library. A
second mapping anywhere else is the boundary being crossed.

Four things in these files are Ubiq's rather than the library's, and all four concern ownership
rather than configuration. **A run's configuration directory belongs to whatever owns the run**: it
is `ConfigStrategy::Fixed` under Ubiq's own config root, named by the pane id or by the agent id —
both ULIDs, so neither can be read as the other's — deleted when that pane closes or that
conversation is retired, and swept at startup for whatever a killed process left. **An agent in a
pane is confined unless the host settings say otherwise** — the policy grants the project's folder
and that directory, and denies the rest of the machine. Which harnesses opt out, which layers a
confined run stacks, and which of them are unusable stays the library's: it has the layered shape
for that, and a second one here would be two places to look. See `D52`. A conversation follows the
same setting as a pane, and what the bridge still owns alone is tool approval: it answers each
request itself, because it holds its child's descriptors. That is `G92` in
[`../backlog.md`](../backlog.md).

**Which `$HOME` a confined agent runs with is Ubiq's answer, and so is every folder granted beyond
the policy.** `HostSettings.agent_home` and `HostSettings.extra_grants` reach `Agents::set_policy`
on every `SetSettings`, and become the home mode and the `extra_ro`/`extra_rw` lists of the
`IsolateOptions` handed to `agent_manager::isolate::plan`. The default is the user's real home, and
that is not a soft default: a layer's `~`-relative grant expands against the run's *effective*
home, so a replaced home aims every toolchain grant the policy carries — `~/.cargo`, `~/.npm`,
`~/.dotnet` — at a directory nothing populated, and the agent holds `cargo` on its `PATH` and
cannot build. Inheriting grants nothing extra by itself: only the paths a resolved layer names are
reachable inside the home. **A toolchain installed somewhere other than its default location is
discovered rather than configured**, which narrows what `extra_grants` is for: `compose_run` calls
`IsolateOptions::grant_toolchains_from_env()` before it applies the settings' grants, so a
`CARGO_HOME` or `GOPATH` pointing outside `~` needs no host setting, and the user's own grant —
applied last — stays the last word. Which variables that reads is the library's list, not Ubiq's. What stays per-run is the configuration, which `CLAUDE_CONFIG_DIR` and
its siblings pin to the run directory. A `~`-prefixed grant is expanded by Ubiq against
`isolate::real_home` before it is passed, because inside a layer `~` means the effective home
rather than the user's — which is exactly the confusion the default avoids.

**A run's record is written through the library and kept by Ubiq.** Two library entry points carry
it. `Harness::transcripts(config_dir)` in `crates/agent-manager/src/harness/mod.rs` answers the
files a harness wrote as its own record inside a relocated configuration directory — defaulted to
empty, which means that harness's record is not portable yet, and overridden today only by
`Claude`. That defaulted method is the reason rule 1 survives this feature: Ubiq copies a file whose
path it was told, and a `projects/<hash>/*.jsonl` literal in `crates/ubiq-host/src/` would be the
boundary crossed. `agent_manager::session::save` in `crates/agent-manager/src/session.rs` writes the
`SessionMeta` — and only `meta.json`, deliberately not the library's `session::start`, which would
leave an empty `transcript.jsonl` that `read_transcript` would report back as an empty
`AgentEvent` transcript, when Ubiq's record is the harness's own file rather than `AgentEvent` lines.

The three answers around them are Ubiq's. **Where the store lives**: `Agents::sessions_dir()` is
`<root>/sessions`, passed to every call explicitly rather than resolved through
`session::sessions_root`, so `AM_SESSIONS` cannot redirect a user's Ubiq transcripts into the store
the `am` CLI manages — with the consequence, deliberate, that `am session ls` does not list Ubiq's
runs. **Which id names a record**: the pane's or the agent's ULID, because that is what a teardown
holds and the harness's own session id never reaches this process. **When it is written**: the
metadata when a run is composed, so a run that crashes still has a record; the harness's files at
teardown, in `Agents::archive`, which every path that deletes a run directory calls first — the
startup sweep included, where the finish time is the sweep's own and the exit code stays unknown.
All of it is best effort: a record that cannot be written is never a reason to fail a spawn or a
close, and a run with no metadata is a plain shell pane rather than an error.

Confining a run in a terminal Ubiq owns is macOS-only. isol8 spawns with inherited stdio and keeps
its child handle private, so no host can hand it a pseudo-terminal; `isolate::confined_launch`
renders the policy and execs `sandbox-exec`, which macOS supports and Landlock cannot. The seam that
replaces it is specified in `refs/isol8-pty-seam-update.md`.

Everything an embedder can substitute is a trait: the catalog registry, the account store, the
secret store, profiles, templates, session history, and an in-process MCP service. Ubiq supplies its
own implementations where it wants application-specific behaviour and takes the filesystem defaults
elsewhere.

Two feature decisions follow from embedding rather than shelling out:

- **`default-features = false`.** The library's default build pulls in `clap` and `ratatui` for its
  own front ends. An application that has a window needs neither.
- **`inproc-mcp` when Ubiq exposes its own tools.** The library can host an embedder-registered MCP
  service on a loopback endpoint and inject it into the run as an ordinary remote MCP server —
  which is how a hosted agent calls back into Ubiq.

`crates/ubiq-host/Cargo.toml` declares the dependency and `crates/ubiq/Cargo.toml` does not, which
is where the edge belongs: the host owns configuration and processes, and the interface may not name
either. `just host` and `just ui` are the mechanical checks that this stayed true, and `just core`
is the check that the host only ever reaches for the library's ungated core — `cli` and `pty` are
absent from this build, so the CLI's own helpers are not available to it and the host builds its
stores itself. Letting the *user* choose a composition is on the wire: `StartConversation` carries a
profile id beside the account, and a profile's own fields are read by `resolve` under any flag the
launch passes. What a composition can still not name from the interface — the catalog's skills and
MCP servers — is tracked in [`../backlog.md`](../backlog.md).

## The rules

**1. Ubiq never names a harness configuration path.** Not `~/.claude`, not `CLAUDE_CONFIG_DIR`, not
a settings filename. If Ubiq needs one, the library grows an accessor. A path literal in
`crates/ubiq/src/` is the clearest possible sign the boundary has been crossed.

**2. Ubiq never hard-codes how to launch a harness.** Which binary, which arguments, which
environment — the library answers all three. Ubiq's `agent.rs` holds the *user-facing* agent-type
list and gets its launch facts from the library.

**3. Nothing about windows, panes or terminals goes into the library.** The library builds with no
UI dependency and must keep doing so; that property is what lets it stay embeddable by anything.

**4. New harness support is a library change.** Adding a harness means a new implementation there,
against the runtime contract already written up in `crates/agent-manager/_docs/harness/`. Ubiq gains
the harness with no change of its own — which is the whole point of the split.

**5. A fact stated in the library's documentation is linked, never copied.** Two copies of a harness
launch flag is one copy that goes stale silently.

## Rationale

**Why embed rather than shell out to `am`?** A subprocess boundary would cost a serialisation round
trip on every run, make in-process MCP impossible, and turn every error into parsed text. The
library was built to be embedded, and its core is deliberately free of terminal and CLI types.

**Why keep harness knowledge out of the application at all?** Because it is the fastest-moving,
most-copied knowledge in this domain: config locations move, flags change, and every tool that
wraps agents ends up with a stale table of them. Concentrating it in one crate with its own
documentation means the table is wrong in one place, and fixable in one place.

## Related docs

- [`project-structure.md`](./project-structure.md) — the two crates and their division of labour
- [`architecture.md`](./architecture.md) — where the coordinator sits, and what it is allowed to hold
- `crates/agent-manager/_docs/README.md` — the library's own documentation, starting point
