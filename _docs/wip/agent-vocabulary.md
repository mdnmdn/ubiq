---
id: wip-agent-vocabulary
title: The conversation vocabulary, the chat surface and the login sandbox
kind: wip
status: draft
summary: What landed in the round that made a login reach its harness's runtime, gave a conversation its model, thinking level and mode, turned the IDE chat into editor-like tabs, and gave every conversation a lifecycle — and what of it is verified against a running binary rather than only against tests.
read_when: you are picking up this work, or you need to know which parts of it have been seen working and which have only been reasoned about
updated: 2026-09-05
verified: 2026-09-05
code_anchors: [crates/agent-manager/src/isolate.rs, crates/agent-manager/src/harness/mod.rs, crates/ubiq-host/src/coordinator.rs, crates/ubiq-host/src/store/harness.rs, crates/ubiq-host/src/shells.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/ui/chat/sidebar.rs, crates/ubiq/src/state/dock.rs]
depends_on: [wip-agent-setup, tech-agent-manager, feat-chat, feat-workbench]
review_cycle: monthly
---

# The conversation vocabulary, the chat surface and the login sandbox

This continues [`agent-setup.md`](./agent-setup.md), which owns the protocol design and the package
order behind a real conversation. **That document stays the design; this one is the state.** Where
the two disagree about what exists, this one is newer.

Four threads landed together, and they are independent enough to be read separately.

## 1. A login reaches its harness's runtime

**The bug.** Signing in worked for Claude Code and failed for Codex. Neither the keychain policy nor
the harness's `LoginPlan` was at fault. `login_confined` replaces `$HOME` with the capture
directory, and isol8 auto-grants **nothing** from the real home when the home is replaced. The only
real-home grant a login had was a hardcoded, Claude-shaped one. Claude Code is a self-contained
binary living in exactly that directory, so it worked. Codex is `#!/usr/bin/env node`, and
isol8's `confine_executable` grants the script and its enclosing npm package but never reads the
shebang and never grants the interpreter — so every read Node made outside that package was denied
under `(deny default)`.

The tempting explanation — that a normal run works because its home is right — does not hold: a run
replaces the home too. A run survives on `macos/system-runtime`'s `/opt` grant. isol8 ships
`toolchains/node` and `toolchains/runtime-managers` layers, but they declare no `executables` filter,
so they are not auto-selectable, nothing requires them, and their `~`-relative grants would land in
the capture directory anyway.

**What it now does.** `login_runtime_grants` derives the real-home read-only grants from the resolved
program: the program's own symlink chain, the shebang interpreter's chain resolved against `PATH`,
and the well-known runtime-manager roots. Every entry is existence-guarded and canonicalised —
canonicalised because isol8 renders a grant as a literal subpath while the process opens the resolved
path, so a grant still carrying `..`, or one naming `/var` where macOS means `/private/var`, matches
nothing and denies without saying so. `login_confined` also honours `IsolateOptions::extra_ro`, which
was a run-only escape hatch despite its doc comment.

`auto_profiles` stays off. The test asserting a login policy never resolves `integrations/keychain`
is the reason that policy exists and must keep passing.

**A shell probe, because no test can answer this.** `BeginHarnessLogin` carries a `probe` flag. A
probe runs the user's shell **inside the policy the harness's login would have run under** — the
plan, the confinement and the render are identical, and only the argv after `-p <policy>` is swapped,
after confinement rather than before, so the grants are still computed from the harness binary. A
probe records no account and reports no login outcome. It is how a human answers the question the
suite cannot: `command -v node` succeeding while `node -v` fails is this bug's exact signature.

## 2. A conversation's vocabulary

`Harness` gained `version()`, `discover_thinking()` and `modes()`, all defaulted so a harness with no
such concept needs no override — which is the honest answer for opencode, Copilot and Grok. Claude's
reasoning levels are scraped from `--help`; Codex's come free from the bundled-models probe already
run for `discover_models`. Modes are fixed CLI enums and are **not** probed.

Three launch-time `ConfigOption`s now reach the composer instead of one — `model`, `thinking`,
`mode` — inside the existing `ConfigOptions` update, so no new `ConvUpdate` variant was needed. All
three are launch-time because every bridge refuses `SetConfigOption`. Picking a model re-issues the
list, because a level belongs to a model and offering one the chosen model does not accept is the
lie the design exists to prevent.

The catalogue is cached on disk, keyed on harness, account and the binary's own version string
(D60). An empty answer is never cached — it is a failed probe wearing a success. A harness whose
`--version` cannot be read bypasses the cache entirely rather than writing an entry that would
outlive the upgrade meant to invalidate it.

The last model and thinking level a harness actually launched with come back preselected (D63), and
— the part that is easy to get half-right — the launch resolves the same remembered value when the
picker was never touched, so what the window shows and what the harness runs are one story.

**`PATH`, and why the picker showed only "Default" (D62).** Discovery spawns the harness by bare
command name. A desktop-launched Ubiq inherits a thin `PATH`, so those spawns failed, the failure
degraded to a lone "Default" choice, and the harness still *looked* available because availability is
probed through the login shell. The host now repairs its own `PATH` once, before any thread exists.
That one repair fixes discovery, `version()` and any bare-name spawn together.

## 3. The chat surface is editor-like

A chat tab is a view onto a conversation the host owns (D61), and the host needed no change at all
for it — no message, no lifecycle. `PanelKind::Chat` carries an id the way `Terminal` and `File`
already do, so many may exist and each survives a restart through the dock's own payload. Chat is no
longer edge-only and is closable, including the last one. The single constant composer slot became a
shared pool, and freeing a slot clears its draft, so what was typed at one conversation cannot turn
up addressed to another.

Each tab attaches to any conversation through a searchable picker. A conversation already attached to
another chat tab draws **greyed rather than hidden** — a row that vanishes reads as a conversation
that ended. That exclusivity is **per IDE chat surface only**: the workbench may show the same
conversation at the same time, because a view was never the workspace.

The lifecycle glyph, the three-dots menu and the two `+` controls share one toolbar row, icons only
with tooltips. The shared conversation view gained a `header` flag beside `footer` and `composer`, so
the agents column is unchanged and both surfaces still read one lifecycle rule rather than two.

## 4. A conversation outlives its harness

Four verbs: **Stop** interrupts a turn, **Unload** kills the harness and keeps everything else,
**Resume** starts it again under the same id, **Delete** ends it and takes the run directory.
Delete confirms first, because it is irreversible.

Resume cost almost nothing structurally: `launch_pending` used to *remove* the `PendingConversation`,
destroying the launch recipe. Keeping it and flipping the prompt dispatch from "is it pending?" to
"is it live?" gave restart with no second relaunch path.

Conversations also name themselves now (D58) — the harness's command with a counter from the second,
per project, first free name reused — and the harness list separates default from configured (D59),
so signing an account in no longer removes the ability to start that harness zero-config.

## What is verified, and what is not

The distinction matters more than usual here, because the headline fix is the least testable thing
in the round.

| Claim | How far it is verified |
|---|---|
| Workspace tests, clippy under `-D warnings`, fmt, and the crate-boundary checks | Run green |
| The login policy grants the interpreter | **Reasoned and unit-tested, never seen signing anybody in.** The probe shell exists precisely because this is the gap |
| opencode and grok logins | Unverifiable here — neither binary is installed (`G118`) |
| A successful real-harness resume, end to end over the bus | No test. No coordinator test in this tree spawns a real harness, and a hanging test would be worse than the honest gap |
| Everything visual | Not verified. No agent in this round could drive the GUI |
| `docs-lint` | Not run against a measured baseline this round |

Two failures in this round were invisible to a green build, and both are worth remembering. A
duplicated `#[test]` attribute silently stopped a test being a test — `cargo test` did not care and
`clippy -D warnings` caught it. And a crate whose only edit was a call site was never compiled with
its own tests, because verification had been scoped to the crates that owned the logic; only
`cargo test --workspace` covers `ubiq-app`.

## Next steps

- Complete a real Codex login, and use the shell probe to record what the policy actually reaches.
- Close `G120`: a resumed harness has no memory of the transcript above it. This needs the session
  record `agent-setup.md`'s P2c describes; `spec.resume` and the session id on `Started` already
  exist, so the remaining work is small but unverified under structured I/O.
- Close `G121`: `Conversation::stop` joins the pump on the coordinator's thread, so a bridge that
  will not drain freezes every window. It was true of ending before; a menu item makes it reachable.
- Fill the agent-definition half `agent-setup.md`'s P4 leaves open. `WorkAgent` already carries
  `task`, `role` and `parent`, but nothing fills them from a real run — which is why the bench menu
  groups by availability alone (`G122`) rather than by role or team.
- Let a conversation be renamed (`G119`). Naming is derived, and nothing can override it.
- Give the model picker a way to say a probe failed. The list falls back to "Default" and the reason
  reaches the log alone, which is how `PATH` hid for as long as it did.

## Related docs

- [`agent-setup.md`](./agent-setup.md) — the design and package order this continues
- [`../tech/decisions.md`](../tech/decisions.md) — D58 to D63, the decisions named above
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the conversation and account families
- [`../features/chat.md`](../features/chat.md) — the chat surface and its composer
- [`../features/workbench.md`](../features/workbench.md) — the agents screen and the settings overlay
- [`../backlog.md`](../backlog.md) — G118 to G122, the gaps named above
