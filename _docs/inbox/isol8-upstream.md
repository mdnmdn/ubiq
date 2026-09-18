---
id: inbox-isol8-upstream
title: isol8 — what this tree owes upstream
kind: note
status: proposal
summary: Six findings against isol8 v0.4.0 that belong in mdnmdn/isol8 rather than here — sockets denied outright on Windows because the file hook has no device exemption, a denial log that is named but never written, a path-policy tie-break that silently drops a widening grant, the missing ConPTY seam that the confine re-invocation exists to work around, a confined shell that hangs the moment it forks, and a Windows spec that inherits a hundred macOS-shaped grants — each with the evidence that produced it and what closing it would change in Ubiq.
read_when: you are raising an issue against isol8, reading `isolate::confine_entrypoint` and wondering why it exists, or deciding whether to move the isol8 pin
updated: 2026-09-18
depends_on: [tech-agent-manager, backlog]
---

# isol8 — what this tree owes upstream

Windows pane confinement was built against **isol8 v0.4.0**, pinned in
`crates/agent-manager/Cargo.toml` at `rev = 14a87bd50e8062442859fd4ee85cdd94f99b5de5`. That
revision is upstream `main` and the `v0.4.0` tag; no branch or pull request in the repository
carries ConPTY work. **Nothing here asks for a pin move, and nothing here blocks Ubiq.** Windows
confinement works today against v0.4.0 exactly as pinned — see `tech/agent-manager.md`. These are
the six things found while building it that are isol8's to fix, collected so they can be raised
on their own schedule.

## 1. The Windows file hook denies every socket, not just unlisted paths

`nt_create_file_detour` (`crates/isol8-winhook/src/winhook.rs:581-600`) is the hook's
`NtCreateFile` detour, and it hands the object name straight to `check_path` with no exemption for
anything that is not a file. Winsock opens a socket by opening the `\Device\Afd` device;
`is_absolute_windows` (same file) returns true for a leading backslash, so the object name stays
`\Device\Afd\Endpoint`, matches no grant a filesystem policy could ever name, and
`PathPolicy::allows` denies it exactly as it would deny an unrelated directory.

**What it cost.** `claude auth login` confined on Windows exited immediately with `Login failed:
Failed to start OAuth callback server: Failed to start server. Is port 0 in use?` — the OAuth
callback server could not open a socket at all. The same login unconfined, against the same
relocated home, opened the server and the browser every time. A confined process on Windows could
not use the network under any policy, full stop, and nothing in the policy or the grants said so
until the detour itself was read.

**What Ubiq did about it.** `WINDOWS_DEVICE_RW` (`crates/agent-manager/src/isolate.rs`) grants
`\Device\Afd` and `\Device\Nsi` read-write to every confined login and every confined run, naming
the two devices Winsock and name resolution open. It is a workaround grafted onto a path policy
that was never meant to carry it, not a fix: granting a device path says nothing about which host
or port a socket may reach, because nothing in `isol8::Spec` or `PathPolicy` can see a socket's
destination. Tracked as `G281`, and the more serious finding of the four here — the missing denial
log below only slows down diagnosing a policy; this one denied a whole capability silently.

**What would close it.** Either the hook exempts device opens from path checking and isol8 grows a
real network policy — hosts or ports it can allow or deny, the way a filesystem grant does paths —
or it documents plainly that Windows confinement has no network story and every embedder must grant
the devices itself, as this one now does.

## 2. The denial log is named but never written

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

Tracked here as `G289`.

## 3. A tie in the path policy silently drops the widening grant

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

## 4. There is no ConPTY seam, and Ubiq's confine re-invocation is the shape of its absence

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

## 5. A confined shell hangs the moment it forks, and no path grant reaches it

`crates/agent-manager/examples/confined_shell_probe.rs` runs a battery of shells under a confine
payload the interface actually wrote, swapping only `ConfinePayload::cmd`. Measured on Windows 11
against isol8 rev `14a87bd5`: `cmd /c echo`, `powershell -Command` (5.1) and `bash -c 'echo ok'`
(git bash, no fork) all pass confined, exactly as unconfined. `bash -lc 'echo ok'` (a login shell)
and `bash -c 'echo a | cat >/dev/null'` (a pipeline, which forks) both **hang** confined and pass
unconfined. `bash -c` alone passes only because bash `exec`s its single command without forking —
the first case that actually forks is the first case that hangs.

**This is not a missing path grant.** A run granted `C:\` read-write, `\Device` and `\??` on top of
the pane's own policy hangs identically. Narrower probes walked the failure forward instead:
granting `\Device\Null` read-write moves the error from `/dev/null: Permission denied` to
`/usr/bin/cat: Permission denied` — a different denial, not a pass — and nothing tried makes the
pipeline complete.

**The mechanism is the cause.** isol8's Windows backend hooks `CreateProcessInternalW`, forces
`CREATE_SUSPENDED` on every child it did not itself create suspended, injects its hook DLL with
`CreateRemoteThread(LoadLibraryW)`, and waits `INFINITE` on that thread with no timeout — at every
process-creation hop, not only the first. msys2 emulates POSIX `fork()` by cloning bash's own
address space into a suspended child and requires the parent's and child's memory layouts to match
at the moment it resumes; a DLL injected into that child between the clone and the resume is
exactly the kind of change that breaks the assumption. `node -e "...execSync('bash -c echo')..."`
spawning bash passes confined, so the second hop and the injection are not broken in general — the
incompatibility is specific to msys2's own `fork` emulation, not to Windows process creation or to
this hook's presence alone.

**What it costs Ubiq.** A confined pane on Windows has no working POSIX shell: `bash -lc` and any
pipeline hang rather than fail, so Claude Code's `Bash` tool run inside an isolated Windows pane
never returns. The only workaround today is turning `isolate_agents` off for the whole machine —
there is no per-pane escape. Tracked as `G301`.

**What would close it.** Either the hook stops injecting into a process it already suspended before
the child's own address-space setup completes — which needs isol8 to know msys2's `fork` shape
specifically — or it exempts a process the caller marks as its own emulated fork, which is not a
distinction Windows process creation makes on its own. Neither is a small change; this is raised so
it is tracked rather than because a fix is obvious.

## 6. A Windows confine spec inherits a hundred macOS-shaped grants

A rendered Windows confine policy carries 102 grants shaped for macOS: `C:\Users\<user>\
Library/Keychains`, `/private/var/db/mds`, `/Library/Developer/CommandLineTools/...` and their
siblings, none of which name anything on the host they were rendered for.

**Cause.** isol8's own profile files — `profiles/integrations/keychain.toml` and
`profiles/shared/agent-common.toml` — ship their `filter = { os = ["macos"] }` line commented out,
each followed by a note that the filter is not parsed yet. isol8 treats an absent filter as
"matches every host", so `apply_layer_filter` does not zero these layers on Windows the way it
zeros every other platform-scoped one. `crates/agent-manager/src/isolate.rs`'s own doc comment on
`DEV_LAYERS` asserts the opposite — "a layer whose `filter.os` does not match this host is kept as
an empty shell rather than dropped" — and that assumption is exactly what fails for these two
files: the filter is not merely non-matching, it is not there to be read at all.

**What it costs.** Nothing functional — Ubiq's naming logic is correct, and a grant for a path that
does not exist on the host is inert. It is cosmetic: `isol8 --show-policies` on a confined Windows
run reads longer and stranger than the policy that is actually enforced, which is exactly the kind
of noise that makes a real denial harder to spot by eye.

**A related but harmless rendering bug.** isol8's `home::expand_tilde` builds a grant's path with
`Path::join` over the raw TOML tail rather than splitting it on `/`, so a `~/Library/Keychains`
grant renders on Windows with mixed separators — `C:\Users\x\Library/Keychains` — rather than a
clean Windows path. Harmless in practice, because the matcher normalises `/` to `\` before
comparing, but one more sign the two commented-out filters were never exercised against a Windows
render.

**What would close it.** Uncomment the two `filter.os` lines once isol8 parses the field, or filter
these two layers out by name on a non-macOS host the way every other platform-scoped layer already
is. Tracked as `G302`.

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
- [`backlog.md`](../backlog.md) — `G281`, `G289`, `G290`, `G292`, `G301`, `G302`
