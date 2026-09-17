---
id: inbox-isol8-upstream
title: isol8 — what this tree owes upstream
kind: note
status: proposal
summary: Three findings against isol8 v0.4.0 that belong in mdnmdn/isol8 rather than here — a denial log that is named but never written, a path-policy tie-break that silently drops a widening grant, and the missing ConPTY seam that the confine re-invocation exists to work around — each with the evidence that produced it and what closing it would change in Ubiq.
read_when: you are raising an issue against isol8, reading `isolate::confine_entrypoint` and wondering why it exists, or deciding whether to move the isol8 pin
updated: 2026-09-17
depends_on: [tech-agent-manager, backlog]
---

# isol8 — what this tree owes upstream

Windows pane confinement was built against **isol8 v0.4.0**, pinned in
`crates/agent-manager/Cargo.toml` at `rev = 14a87bd50e8062442859fd4ee85cdd94f99b5de5`. That
revision is upstream `main` and the `v0.4.0` tag; no branch or pull request in the repository
carries ConPTY work. **Nothing here asks for a pin move, and nothing here blocks Ubiq.** Windows
confinement works today against v0.4.0 exactly as pinned — see `tech/agent-manager.md`. These are
the three things found while building it that are isol8's to fix, collected so they can be raised
on their own schedule.

## 1. The denial log is named but never written

`isol8::backends::analyze_denial_log_path(pid)` (`crates/isol8-core/src/backends/windows.rs:122`)
returns the path of an NDJSON log of denied paths, and its comment says the hook DLL "may append"
to it. `crates/isol8-winhook` contains no writer for that file. The detours
(`crates/isol8-winhook/src/winhook.rs:582-654`) deny through `allowed`/`check_path` at
`winhook.rs:544-563` and record nothing.

**What it cost.** A confined harness that dies against its own policy leaves no trace of which path
it was refused. Finding the cause of one such death here meant bisecting the policy by hand across
a dozen runs — grant by grant, then variable by variable — to arrive at a fact one log line would
have stated. The alternative on Windows today is Process Monitor.

**What would close it.** Either the hook writes the log the API promises, or
`analyze_denial_log_path` goes and the docs stop naming it. A dead accessor is worse than no
accessor, because it reads like a diagnostic path that exists.

Tracked here as `G280`.

## 2. A tie in the path policy silently drops the widening grant

`PathPolicy::effective_access` (`crates/isol8-path-policy/src/lib.rs:78-91`) picks the grant with
the longest normalized path, and on a tie keeps the one it already has — the comparison is `spec >
len`, not `>=`. So adding a grant whose path is textually equal to an existing one, in order to
widen it (`metadata` to `rw`, say), does nothing at all, and says nothing about having done
nothing.

**Why it matters to a caller.** Composing a policy by appending grants is the obvious thing to do
with the API, and it is right in every case except the one where the caller most needs it: raising
the access on a path a layer already mentions. Doing that correctly requires removing the earlier
grant first, which the type does not hint at.

**What would close it.** Either last-writer-wins on a tie, or the widest access wins on a tie, or
the API refuses a duplicate path loudly. Any of the three is predictable; the current behaviour is
the only one that is not.

## 3. There is no ConPTY seam, and Ubiq's confine re-invocation is the shape of its absence

isol8's pty seam is `cfg(unix)` at every public point: `Backend::spawn_with_stdio`
(`crates/isol8-core/src/backends/mod.rs:63`), `Sandbox::spawn_pty`, `Sandbox::spawn_with_stdio`,
`open_pty`, `SandboxStdio`. `SandboxChild::windows()` is `pub(crate)`. A host that has already
opened a pseudoconsole therefore has no way to hand it to isol8, and nothing to exec either — the
macOS answer, rendering a policy that `sandbox-exec` applies in place, has no Windows equivalent.

Ubiq works around this rather than waiting for it. isol8's Windows backend calls `CreateProcessW`
with no console-creation flag (`backends/windows.rs:96-109`: no `CREATE_NEW_CONSOLE`, no
`DETACHED_PROCESS`, no `CREATE_NO_WINDOW`, a zeroed `STARTUPINFOW` with no `STARTF_USESTDHANDLES`),
so a process it spawns attaches to whatever console its caller already has. `confined_launch` puts
the running binary back in the pane, re-invoked under `isolate::CONFINE_ARG` against a
`ConfinePayload`, and letting isol8 create the harness from in there lands it on the host's own
ConPTY, at the host's own size, as a real console — the Chrome `--type=renderer` multi-call
pattern rather than a second shipped executable. Measured: `size=100x30 redirected=False`, exit 0,
against a pseudoconsole opened at exactly that size.

**What would close it.** A Windows `spawn_with_stdio` taking either explicit handles
(`STARTF_USESTDHANDLES` with `bInheritHandles`) or a pseudoconsole
(`STARTUPINFOEX` plus `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`). `refs/isol8-pty-seam-update.md`, in
isol8's own tree, specifies the unix seam and says ConPTY is separate work.

**What Ubiq would delete.** The `isolate::CONFINE_ARG`/`confine_entrypoint` re-invocation, the
`ConfinePayload` contract and its file under `<state_dir>/confine/`, and the `win32job` dependency
carried only for the confined process's job object. `confined_launch` would render a launch on
Windows the way it does on macOS, and one process per confined pane would go away.

## The trap worth reporting even without a fix

isol8 grants the **resolving** process's working directory read-write (`overrides_layer`, from
`Context::cwd`), and a process it confines inherits its parent's working directory. An embedder
that resolves a policy in one process and applies it in another — which is what any out-of-process
shim does — therefore has to make the applying process enter that same directory first, or the
confined child runs somewhere its own policy denies and dies at startup. `ConfinePayload::cwd`
carries it for exactly this reason.

This is defensible behaviour, not a bug: an ambient grant has to come from somewhere. But it is
undocumented, invisible, and fatal, and combined with the missing denial log above it produces a
failure with no diagnosis path at all. A sentence in isol8's embedding guide would be enough.

## Related docs

- [`tech/agent-manager.md`](../tech/agent-manager.md) — how `confined_launch` and the shim work here
- [`backlog.md`](../backlog.md) — `G280`, `G281`, `G283`, `G285`
