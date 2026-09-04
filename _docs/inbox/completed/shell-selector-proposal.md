---
id: inbox-shell-selector
title: Proposal — OS-aware shell selector on the new-pane control
kind: proposal
status: proposal
summary: Split the bottom region's "+" into a default click and a small chevron menu that lists the shells actually installed for the platform, plus a "Logs" entry — and stop opening the console panel automatically.
read_when: you are changing the new-pane control, how a pane's program is chosen, or the console panel's default visibility
updated: 2026-09-02
depends_on: [feat-panes, feat-logs, tech-transport]
---

# Proposal — OS-aware shell selector on the new-pane control

**Today the "+" on the bottom region's tab bar does one thing: it opens `$SHELL` with no args.**
`NewPane.run` (`crates/ubiq/src/ui/dock/skin.rs:57`) calls `spawn_pane(None, Vec::new(), cx)`
(`crates/ubiq/src/app/wire.rs`), which sends `SpawnWorkspace { agent_type: None, .. }`. The
coordinator resolves `None` to `shells::default_program()` — `$SHELL`, or `/bin/sh` — and that is the only program a pane has ever been
started with from the UI; `agent_type: Some(..)` exists on the message and is exercised only by
`crates/ubiq-host/tests/coordinator.rs`. There is no way, today, to open a pane running a shell other
than the user's default one.

Separately, a defect in how that default shell is spawned means most users hit broken shell
init on every new pane — see the finding below. Fixing it is a prerequisite for a selector to be
worth building: picking a different shell from a menu would just move the same bug onto a different
program if the pane spawn path itself did not run each one as its platform expects.

## Motivation

**The current spawn is not a login shell, so PATH-setting init is silently skipped.** Both
`pty::spawn` (`crates/ubiq-host/src/pty/mod.rs:49`) and the coordinator's default built the shell with
`CommandBuilder::new(program)`. In `portable-pty` 0.9.0, only `CommandBuilder::new_default_prog()`
takes the branch that prefixes argv0 with `-` (`cmdbuilder.rs:510-517` in the vendored crate source);
`new(program)` always takes the plain branch. A non-login zsh/bash never sources `.zprofile` /
`.zlogin` — where Homebrew's `shellenv` and most `pyenv`/`starship`/`nvm` PATH setup lives — so tools
that are genuinely installed are reported as `command not found` inside Ubiq's panes and work fine in
Terminal.app/iTerm. This is a correctness bug independent of the selector, filed in
[`embedded-shell-env-note.md`](./embedded-shell-env-note.md).

**There is no way to open a second shell for occasional use.** A user with `fish` as `$SHELL` who
wants one `bash` pane to debug a `.bashrc`-specific issue, or a Windows user who wants `cmd` instead
of the default PowerShell pane, has no path to that today short of typing `bash` inside the shell
that is already running — which starts a nested, not a fresh, shell.

**The console panel opens on every window whether or not anyone asked for it.**
`default_layout()` (`crates/ubiq/src/ui/dock/mod.rs:485`) builds `PanelKind::Logs` and installs it in
the bottom region unconditionally, alongside the centre, explorer and chat panels. This is documented,
current, deliberate behaviour — [`feat-logs`](../features/logs.md) states plainly: *"It is always
present and is never closed... It is drawn in every rail mode and with no project open, which is the
state in which it is most worth reaching."* This proposal's lateral request — stop auto-opening it —
**reverses that stated contract**, not just an implementation detail. It is called out here rather
than folded quietly into the plan; whoever picks this up should treat the console's default
visibility as its own decision, not a side effect of the shell-selector work, and update
`feat-logs.md`'s Behaviour section explicitly if it goes ahead. Nothing about the ring, the sink, or
the panel's own toolbar changes — only whether the panel is in the arrangement on a fresh window.

## Proposal

**1. Fix the login-shell defect, for any program the pane spawn path is given, not only the
default.** Whichever shell ends up running — the platform default or one picked from the menu below —
it should behave the way a user's ordinary terminal does. On Unix that means argv0 prefixed with `-`;
`portable-pty` already implements this, but only through `new_default_prog()`, which takes no program
name and no args, so `pty::spawn` cannot use it as-is for a named shell. The fix is either replicating
the four-line prefixing portable-pty already does (skip `CommandBuilder`'s convenience path
entirely for the shell case) or upstreaming a way to ask for a login prefix on `CommandBuilder::new`.
Windows has no equivalent: `pwsh.exe`/`cmd.exe` have no login/non-login split, so nothing changes
there. This step alone already fixes the errors in
[`embedded-shell-env-note.md`](./embedded-shell-env-note.md) even before the menu exists.

**2. The "+" keeps its current behaviour: click opens the platform default shell.** No change to
`NewPaneRun`'s trigger or what a bare click does.

**3. A small chevron (`v`) sits beside the "+", opening a menu of what else can be spawned here.**
Reuse the one dropdown mechanism the window already has —
[`crates/ubiq/src/ui/kit/menu.rs`](../../crates/ubiq/src/ui/kit/menu.rs), the same trigger-plus-
`deferred`-list `Picker`/`Action` pattern `file_tab_menu` uses off `open_file_tab_menu()`
(`crates/ubiq/src/app/editor.rs`) — rather than introducing a second menu mechanism. The rows:

- One row per shell **actually present** on the machine, OS-aware:
  - macOS / Linux candidates: `zsh`, `bash`, `fish` (and any others worth a fixed list — not an open
    text field). The row for `$SHELL`'s own basename is marked as the default, matching what the bare
    "+" click already runs.
  - Windows candidates: PowerShell (`pwsh.exe` if present, else `powershell.exe`) and `cmd.exe`.
  - A shell not found on the machine is not shown — this is a bounded fixed list which the host
    checks against, not a free-form launcher, matching the "not a new dependency, not a hypothetical
    config surface" bar this codebase holds itself to.
- A trailing `Logs` row that opens the console panel on demand (only meaningful if item 4 below is
  taken).

**4. (Under discussion — see Motivation) Stop installing the console panel in `default_layout()`.**
If this proceeds, `crates/ubiq/src/ui/dock/mod.rs:485` and its `install()` call for
`Region::Bottom` drop out of the startup path, the menu's `Logs` row becomes the panel's only way to
appear on a fresh arrangement, and `feat-logs.md`'s Behaviour section is edited to match. A window
restoring a *saved* arrangement that already has the console open is unaffected either way — this
only changes what a brand-new window starts with.

## Where a shell list has to come from

**The UI may not decide which shells exist.** `AGENTS.md`'s architecture rules are explicit: *"The
UI never assumes the pseudo-terminal is local. No path, no process handle, no file descriptor crosses
into UI code."* Checking `/opt/homebrew/bin/fish` or walking `PATH` from `crates/ubiq` would violate
that outright — it is exactly the kind of host-side fact the UI is not allowed to know for itself.

So shell discovery is `ubiq-host`'s job, answered over the bus like everything else the UI cannot
read on its own. The nearest existing shape is `Message::HostInfo`
(`crates/ubiq-proto/src/messages.rs:85`), sent once per window at attach; either that message grows
an `available_shells: Vec<ShellInfo>` field (name, program, `is_default`), or a new
request/response pair is added following `ListProjects`/`ProjectList`'s shape if the list should be
re-probed rather than sent once (e.g. a shell installed after the window opened). Which of the two is
right is an implementation call, not a design one — either keeps the rule intact; a fixed
request/response pair is likely the smaller change since `HostInfo`'s existing shape and call site
were not built to be extended with a list.

## What does not change

- `SpawnWorkspace`'s shape (`crates/ubiq-proto/src/messages.rs:64`) already carries `agent_type:
  Option<String>` and `args: Vec<String>` — picking a menu row is `spawn_pane(Some(program),
  Vec::new(), cx)`, the same call `NewPane.run` already makes with `None`. No transport change needed
  for the spawn itself, only for the shell list (see above).
- The console panel's own behaviour, ring, toolbar and focus rule — untouched by this proposal except
  for whether it is in a *fresh* window's arrangement.
- No new dependency: shell discovery is fixed candidates checked for existence, not a package that
  enumerates installed software.

## Next steps

- Decide whether item 4 (console panel's default visibility) proceeds, and if so, land it as its own
  change against `feat-logs.md`'s Behaviour section, separate from the selector.
- Decide `HostInfo` extension vs. a new request/response pair for the shell list.
- Land the login-shell fix (item 1) first — it stands alone and already fixes a real defect users hit
  today, independent of whether the menu ships.

## Related docs

- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — the new-pane control
  and `SpawnWorkspace`'s existing contract
- [`../features/logs.md`](../features/logs.md) — the console panel's current, documented "always
  present" behaviour that item 4 would reverse
- [`./embedded-shell-env-note.md`](./embedded-shell-env-note.md) — the login-shell defect this
  proposal's item 1 fixes
- [`./terminal-interaction-proposal.md`](./terminal-interaction-proposal.md) — the keyboard
  pass-through audit, unrelated to this but touching the same dock tab bar

## Status (2026-09-02)

Items 1, 2 and 3 landed: the login-shell fix in `crates/ubiq-host/src/pty/mod.rs`, shell discovery
in `crates/ubiq-host/src/shells.rs` behind `ListShells`/`ShellList`, and the chevron menu in
`crates/ubiq/src/ui/new_pane_menu.rs`. The behaviour is
[`../features/panes-and-terminals.md`](../features/panes-and-terminals.md)'s and the trade is `D49`.
Item 4 landed too, asked for directly: the console is not in a fresh window's arrangement, it closes
like any other panel, and the menu's `Logs` row is what puts it on screen. What the proposal did not
foresee is that the control it adds is drawn on the pane region's own strip, so an empty region has
to stay a legal arrangement and the switch that opens one starts a pane in it — `D50`.
