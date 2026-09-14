---
id: wip-claude-auth-problem
title: Claude credentials vanish from a run directory
kind: wip
status: current
summary: A Claude Code pane loses its OAuth login after some hours. This is the standing record of the investigation — the machine evidence, the full credential path through agent-manager and the host, the hypotheses and what would falsify each, the instrumentation added to settle it, and the log of what has been observed so far.
read_when: you are working on Claude account credentials, the per-run config directory, the isol8 policy a run gets, or the token write-back
updated: 2026-09-14
verified: 2026-09-14
code_anchors: [crates/agent-manager/src/harness/mod.rs, crates/agent-manager/src/harness/claude.rs, crates/agent-manager/src/provision.rs, crates/agent-manager/src/isolate.rs, crates/agent-manager/src/run.rs, crates/agent-manager/src/overlay.rs, crates/agent-manager/src/account.rs, crates/agent-manager/src/credentials/mod.rs, crates/ubiq-host/src/agent.rs, crates/ubiq-host/src/coordinator.rs, crates/ubiq-proto/src/log.rs]
depends_on: [tech-agent-manager, feat-logs]
---

# Claude credentials vanish from a run directory

A long-lived Claude Code pane stops working after some hours, and
`$CLAUDE_CONFIG_DIR/.credentials.json` is gone. **This document is written to be read cold.**
Everything needed to resume the investigation is here; nothing depends on remembering a
conversation. Append to the watch log at the end, do not rewrite the sections above it.

## 1. The symptom, as reported

- A Claude Code pane launched by Ubiq works, then after some hours can no longer refresh its access
  token.
- `$CLAUDE_CONFIG_DIR/.credentials.json` is missing when it is looked for.
- The **first theory** was that isolation seeds the file read-only, so the harness cannot rewrite it
  on refresh, and a fix was attempted along those lines — make it writable and write it back to the
  store. No proof was ever gathered for that theory.
- The **standing theory**, as of 2026-09-14, is that the write is blocked by **isol8** — the
  sandbox policy — not by a file mode. See §5.

## 2. Machine evidence — 2026-09-14 13:06

Read off a live ubiq-managed Claude pane (the session that opened this investigation).

- `CLAUDE_CONFIG_DIR=/Users/mdn/.config/ubiq/runs/01M2FRXP7Q51RK322EKP5Z8V4Y` — a **per-run**
  directory keyed by a ULID, not the user's `~/.claude`. The host owns it
  (`Agents::run_dir_for`, `crates/ubiq-host/src/agent.rs`), and it is passed to agent-manager as
  `ConfigStrategy::Fixed`.
- `~/.claude/.credentials.json` does **not** exist on this machine. The user's own Claude session
  lives in the **macOS login Keychain**; the run directory holds a copy seeded out of it.
- The seeded copy was `-rw-r--r--` (mode **644**), owned by the user, on a writable directory.
  **It is writable.** The read-only-file theory does not hold for this build. (The 644 is still
  wrong — every other credential write in the crate forces 0600 — and has been corrected as part of
  the instrumentation work; it was never the cause.)
- Token window, read out of the live file (values elided):

  | field | value |
  |---|---|
  | `claudeAiOauth.expiresAt` | `1789408388533` → 2026-09-14 **19:53:08** |
  | `claudeAiOauth.refreshTokenExpiresAt` | `1791966676533` → 2026-10-14 10:31:16 |
  | `subscriptionType` | `team` |
  | `rateLimitTier` | `default_claude_max_5x` |

  The run directory was created at **12:55**. The access token therefore expires **~7 h after the
  pane started**, which is exactly the reported "after some hours". The *refresh* token is a month
  out, so nothing is expiring except the access token — the failure is in the **refresh**, not in
  the credential's lifetime.

## 3. What the code actually does

Traced 2026-09-14 against the working tree at `43bdb41`.

### 3.1 Where the durable credential lives

`crates/agent-manager/src/credentials/` has four `SecretStore` backends; the default engine is
**`files`** (`credentials/mod.rs`, `resolve_engine()`), with **no macOS-specific override**. But
that store backs `am account import` and friends — it is *not* what a live run reads.

For a zero-config run (no explicit account, which is this case) the durable origin is found by
`provision::seed_zero_config_login()` (`crates/agent-manager/src/provision.rs:197-249`) in two
tiers:

- **tier A** — copy `~/.claude/.credentials.json` out of the real `$HOME`;
- **tier B** — `Harness::ambient_login()`, which for Claude reads the **macOS login Keychain**
  (`harness/claude.rs:580-587`, via `account::read_claude_keychain_credentials`).

On this machine tier A finds nothing, so **the origin is the Keychain**. That matters: the
write-back path for a Keychain origin is `Harness::adopt_login` (`claude.rs:593-598`), which shells
out to `security` to write the Keychain item — *not* a plain file write.

### 3.2 How the run directory is seeded

Claude Code is Class A: `CLAUDE_CONFIG_DIR` relocates the whole config tree
(`Claude::config_anchor()`, `harness/claude.rs:146-155`), whose `login_seed` is

```rust
SeedFile::credential(".claude/.credentials.json", ".credentials.json"),
SeedFile::new(".claude.json", ".claude.json"),
```

`harness::seed_login()` (`harness/mod.rs:319-333`) **copies** — never links — because the harness
rewrites the file in place. Only the `credential: true` entry is ever written back.

### 3.3 The write-back, which already exists

`harness::sync_login()` (`harness/mod.rs:392-450`) is the reconciler:

1. read the origin's blob and every live run dir's copy (with mtime);
2. rank them in `newest_login()` (`mod.rs:479-523`) — by claimed `expiresAt` when anything claims
   one, else by mtime — filtering out **blanked** blobs via `credentials::login_is_usable()`;
3. write the winner back to the origin (`write_credential` for a directory origin,
   `adopt_login` for a Keychain origin);
4. write the winner into every *other* live run dir holding an older copy, because a refresh
   **rotates** the refresh token and every other run is now holding a revoked one.

It is driven from two places:

- **agent-manager's own** `run::cleanup()` (`run.rs:196-203`), at teardown only — and its own doc
  admits a token rotated mid-run is lost if the process is killed;
- **the host**, on a timer: `Coordinator::sync_logins_due` (`crates/ubiq-host/src/coordinator.rs:3169-3181`,
  `LOGIN_SYNC_EVERY = 30s`) → `Agents::sync_logins()` (`crates/ubiq-host/src/agent.rs:1227-1255`).
  There is also `Agents::refresh_login(key)` (`agent.rs:1063`), which harvests before re-seeding a
  run.

**So "no write-back exists" is not the bug.** A 30s reconciler is in place.

### 3.4 What removes a run directory

Every path is event-driven, none is a timer racing a live pane (verified):

| path | trigger |
|---|---|
| `Agents::retire(pane)` (`agent.rs:1434`) | `Coordinator::pane_gone` |
| `Agents::retire_agent` / `park_agent` (`agent.rs:1358`, `1482`) | `Coordinator::end_conversation`, via `client_gone` or `reap_conversations` (which filters on `conversation.ended()`) |
| `Agents::sweep()` (`agent.rs:1453`) | **once**, at host startup, before any pane exists |
| `run::cleanup()` (`run.rs:200`) | agent-manager's own CLI runs, ephemeral config only — not Ubiq's `Fixed` strategy |
| `overlay::sweep_old_runs()` (`overlay.rs:41-88`) | opportunistically at `new_run_dir()`, `AM_RUNS_TTL_DAYS` default 7 |

`park_agent` is notable: it keeps the run directory but **scrubs the login files**.

### 3.5 The sandbox policy a run gets

`isolate::plan()` (`crates/agent-manager/src/isolate.rs:526`) builds the isol8 `Spec`:

- `read_write_grants()` (`isolate.rs:922`) grants the **run's config dir** and its **cwd**
  read-write, plus developer roots. So the run dir *is* writable under the policy as written.
- `auto_profiles = true`, so isol8 matches the harness's own layer on `cmd[0]`.
- `~/Library/Keychains` is **not** in the read-write grants.

Contrast `isolate::login_confined()` (`isolate.rs:~583`), the **login capture** policy, whose
doc comment is the single most important piece of prior knowledge in this file:

> A login is confined to keep it away from the *operating system's keychain*, so that it writes the
> plaintext credential a capture can collect. […] For Claude Code 2.1.218+ a relocated `$HOME` alone
> leaves the keychain merely *unreachable* — no `~/Library/Keychains` — which that version reports
> as an **error rather than falling back to a file**.

That is the shape of the bug, stated in the tree already, for a different code path.

## 4. Hypotheses, and what would settle each

**H1 — the refresh never reaches the file, because Claude persists to the Keychain.**
On macOS, Claude Code prefers the login Keychain. A sandboxed run auto-selects the harness layer,
which may grant Keychain *read* but cannot grant a write to `~/Library/Keychains` (not in
`read_write_grants`). So the refresh is computed, the persist is denied by Seatbelt, and Claude
treats the credential as unusable — blanking or removing `.credentials.json`.
*Falsified if*: the run dir's `.credentials.json` fingerprint changes at the refresh moment.
*Confirmed if*: the fingerprint never changes and the file is blanked or removed instead.

**H2 — the rotated refresh token is written to the run dir but lost.**
If the copy *is* rewritten and `sync_login` fails or never runs, the origin keeps the pre-rotation
refresh token; the next run is seeded a retired one, refresh answers `invalid_grant`, and Claude
deletes the file.
*Falsified if*: the 30s `sync_logins` log shows the origin being updated with the new fingerprint.

**H3 — `adopt_login` fails silently against the Keychain.** The origin here is the Keychain, so the
write-back is a `security` invocation, not a file write. Under the host's own process (unconfined)
it should work, but its failure was previously only a `warn!` with no digest.
*Falsified if*: the log shows a successful Keychain write with the new fingerprint.

**H4 — `park_agent` scrubs the login of a pane the user still considers alive.** Reaching it needs
`conversation.ended()`, so it should not fire under a live pane — but it is the one path that
deletes credentials *without* deleting the directory, which matches "the file disappeared, the
directory did not".
*Falsified if*: no `park_agent`/scrub line appears near the failure.

H1 is the standing theory and matches the user's reading that the block is isol8, not permissions.

## 5. Instrumentation added (2026-09-14)

The point of this pass: **every action on a token is now a log line**, so the failure moment can be
reconstructed from a log file hours later rather than reproduced.

### 5.1 The digest

`credentials::login_digest(bytes) -> String` (`crates/agent-manager/src/credentials/mod.rs`) is the
only thing that may be logged about a credential. It prints byte length, the `login_is_usable` flag,
every `*expire*` field, `valid_for=NhNm` computed against now, and an **8-hex fingerprint per token
field** (`DefaultHasher`, a fingerprint not a digest).

**The fingerprint is the instrument.** A refresh rotates both tokens; two digests naming different
fingerprints are two different logins. That is the only way to see a rotation that happened
somewhere and was then lost. No credential bytes or token values are ever logged.

### 5.2 Where the lines are

- `harness/mod.rs` — `seed_login` (what was seeded, with digest; also now chmods 0600),
  `sync_login` (origin digest, every held run-dir copy with digest and mtime, which one won,
  whether a write-back happened), `newest_login` (the ranking inputs), `write_credential`,
  `harvest_login`.
- `provision.rs` — the run dir created, which seeding tier was taken (real `$HOME` vs ambient
  Keychain), the account origin resolved.
- `run.rs` — `cleanup()` no longer swallows the harvest result; the ephemeral-dir removal is logged.
- `overlay.rs` — every run dir the TTL sweep removes.
- `isolate.rs` — `plan()` logs the layer set actually in force, the read-write grants and the home
  mode. **This is the line that shows whether the policy can reach the Keychain.**
- `harness/claude.rs` — `ambient_login`, `read_ambient_keychain_login`, `adopt_login`: whether the
  Keychain read/write succeeded, with digests.
- `account.rs` — the `security` subcommand outcomes, and every file `sign_out`/`delete_account`
  removes.
- `crates/ubiq-host/src/agent.rs` — each 30s `sync_logins` pass, `refresh_login`, `compose_run`
  (the digest the run *starts* with), and every `retire`/`park`/`sweep` that removes or scrubs a
  run directory, with the reason.

### 5.3 Making the log survive

Logs previously died with the process (in-memory ring + stderr; `crates/ubiq-proto/src/log.rs`).
An opt-in **file sink** was added: set **`UBIQ_LOG_FILE=/path/to/ubiq.log`** and `install()` appends
the formatted log there as well. Append-only, unrotated, no secrets. `ubiq_host::agent` was also
reclassified from `Subsystem::Ui` to `Subsystem::Harness`, where it belongs.

### 5.4 State of the change

Uncommitted in the worktree as of 2026-09-14. `cargo check` is clean for `agent-manager`,
`ubiq-host` and `ubiq-proto`; `agent-manager`'s suite is 494 passing with two failures that predate
this work and are environment artifacts (`provision::tests::ephemeral_strategy_creates_a_fresh_dir_under_state`
cannot write to the real `~/.config/agent-manager` from inside a pane, and
`io::jsonl::tests::live_two_turn_structured_session_when_claude_available` needs a live binary).

`just verify` does **not** pass whole, for reasons outside this change: someone else's git-history
work is mid-flight in the same worktree (`crates/ubiq-host/tests/git_history.rs`, `tests/diff.rs`),
and `just docs-lint` has 70-odd pre-existing failures across the library. Neither is this change's
to fix. `_docs/tech/agent-manager.md` is 494 lines against a 400-line target — it was already over
before the paragraph added here.

Also of note: `crates/ubiq-proto/src/log.rs` was reverted on disk once mid-edit by another process
in this shared worktree, and the edits were reapplied. Check that `UBIQ_LOG_FILE` is still in
`log.rs` before trusting a run.

## 6. How to run the watch

```sh
UBIQ_LOG_FILE=/tmp/ubiq-auth.log RUST_LOG=agent_manager=debug,ubiq_host=debug just dev
```

Start a Claude pane, note the `expiresAt` it launches with (the `compose_run` line prints the
digest), and leave it. The interesting window is the **hour around `expiresAt`** — 7 h in on a
fresh login.

What to read, in order:

1. the `isolate::plan` line — does the policy name a keychain layer, and is the run dir in the
   read-write grants;
2. the `compose_run` digest — the fingerprint the run started with;
3. the 30s `sync_logins` lines — does any fingerprint ever change;
4. anything at the failure moment: a blanked blob (`usable=false`), a removal, a scrub, a `security`
   failure.

Useful one-liners:

```sh
grep -E 'accessToken=|refreshToken=' /tmp/ubiq-auth.log   # every fingerprint, in order
grep -E 'usable=false|removing|scrub|security' /tmp/ubiq-auth.log
```

To watch the file directly alongside the log:

```sh
while :; do ls -l@ "$CLAUDE_CONFIG_DIR/.credentials.json"; sleep 300; done
```

## 7. If H1 is confirmed — the shape of the fix

Not implemented; recorded so the next pass does not re-derive it.

Either **deny the Keychain for a run the way `login_confined` denies it for a capture**, so Claude
takes its file-fallback path and rewrites the run dir's copy (which `sync_login` already harvests
every 30s) — the more consistent option, since it makes a run behave like a capture; or **grant
`~/Library/Keychains` read-write** so the harness can persist where it wants to, which widens the
sandbox and leaves two sources of truth. The first is preferred.

Either way it is a change in `crates/agent-manager/src/isolate.rs`, and it needs a `Dnn` decision
row because it changes what a sandboxed run may reach.

## 7a. A later session cannot look at this run's directory

Each ubiq-managed Claude pane gets **its own** `runs/<ULID>` directory, and it is gone or stale by
the time a fresh session reads this. So `01M2FRXP7Q51RK322EKP5Z8V4Y` above is evidence, not a
place to go look. A later session:

- finds **its own** directory with `echo $CLAUDE_CONFIG_DIR`, which is a *different* pane and a
  *different* token — it says nothing about the one that failed;
- cannot list `~/.config/ubiq/runs/` from inside a pane (the sandbox answers *Operation not
  permitted*); ask the user to run `! ls -lt ~/.config/ubiq/runs/` if the directory listing is
  needed.

This is why the evidence lives in this document and in `UBIQ_LOG_FILE`, and why the watch log below
records **fingerprints and mtimes rather than paths**. A fingerprint recorded here can be compared
against one printed hours later in any session; a run directory cannot.

### The scratch folder

`/tmp/ubiq-auth-watch/` (mode 700), set up 2026-09-14 13:22, outlives the pane:

| file | what it is |
|---|---|
| `README.md` | the baseline and the run dir being watched, for a session that has neither |
| `snapshot.sh [file]` | appends one redacted row; defaults to `$CLAUDE_CONFIG_DIR`, takes a path when run from another session |
| `snapshots.tsv` | the rows: `when state mode bytes mtime access_sha8 refresh_sha8 expiresAt refreshTokenExpiresAt` |
| `watch.log` | stdout of the background loop |

**No token material is written there** — only sha256-8 fingerprints and metadata. `state` is the
answer at a glance: `ok`, `BLANKED` (a failed refresh blanked the tokens), `MISSING` (the file went
away), `UNREADABLE`.

A detached loop snapshots every 5 minutes. It is `nohup`'d, so it survives this session, but it is
**not** a launch agent and will not survive a reboot or a killed process tree — a fresh session
should check `tail /tmp/ubiq-auth-watch/snapshots.tsv` and restart it if the rows stopped:

```sh
nohup sh -c 'while :; do /tmp/ubiq-auth-watch/snapshot.sh <path-to-credentials.json>; sleep 300; done' \
  >> /tmp/ubiq-auth-watch/watch.log 2>&1 &
```

## 8. Watch log

Append one row per observation. Keep the digest fingerprints — they are the evidence.

**Two fingerprint schemes are in play, and they do not compare.** The rows below taken by hand use
`sha256(token)[:8]`; `credentials::login_digest` in the code uses Rust's `DefaultHasher`. Compare
hand-taken rows to hand-taken rows, and log lines to log lines. To read a file by hand in the same
scheme as the rows here:

```sh
python3 -c "
import json,hashlib,os,sys
d=json.load(open(sys.argv[1]))['claudeAiOauth']
print({k:hashlib.sha256(d[k].encode()).hexdigest()[:8] for k in ('accessToken','refreshToken')}, d['expiresAt'])
" "$CLAUDE_CONFIG_DIR/.credentials.json"
```

| when | what happened |
|---|---|
| 2026-09-14 12:55 | run dir `01M2FRXP…` created, credentials seeded from the Keychain, mode 644, `expiresAt` 19:53 |
| 2026-09-14 13:06 | evidence in §2 collected; read-only-file theory ruled out |
| 2026-09-14 13:16 | file still present, still mode 644, **mtime still 12:55** — untouched since the seed. sha256-8 `accessToken=d31af002` `refreshToken=0888341f`. Baseline for the rotation check: if these change, the harness did write; if they never change and the login fails at 19:53, H1 holds |
| 2026-09-14 13:22 | `/tmp/ubiq-auth-watch/` created, 5-minute snapshot loop started against this pane's file; still `ok`, unchanged |
| 2026-09-14 ~13:30 | instrumentation of §5 landed; `UBIQ_LOG_FILE` watch not yet started |
