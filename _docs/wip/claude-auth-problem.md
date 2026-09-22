---
id: wip-claude-auth-problem
title: Claude credentials vanish from a run directory
kind: wip
status: current
summary: "A Claude Code pane loses its OAuth login after some hours. Root cause found: Claude Code stores the login in a macOS keychain item keyed by sha256 of $CLAUDE_CONFIG_DIR whenever the keychain is reachable, migrating the seeded .credentials.json into it and deleting the file, so every refresh is invisible to agent-manager and a per-run config dir makes the key per-run. The fix denies a Claude run the login keychain. This is the full record — evidence, the discarded hypotheses, the fix as landed, and what to check if it is not resolutive."
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

> **Resolved 2026-09-14 — read §16, §19, §20, §21 first.** Claude Code has two credential
> backends. Whenever `~/Library/Keychains` is reachable it keeps the login in
> `Claude Code-credentials-<sha256($CLAUDE_CONFIG_DIR)[:8]>`, migrates the seeded
> `.credentials.json` into it and **deletes the file**; only when the keychain is unreachable does
> it read and rewrite that file. A run's config dir is a fresh ULID, so the key is per-run and every
> refresh is invisible to this crate. The fix withholds the login keychain from a Claude run (§20).
> **§1-§15 are the investigation as it happened and contain conclusions later overturned** — they
> are kept because the falsified hypotheses are what stop the next reader repeating them. §21 says
> what to check if the fix does not hold.

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
| 2026-09-14 13:42 | run dir `01M2FRXP…` gone — the old pane was retired, so the snapshot row reads `MISSING` for that reason, **not** the failure under investigation. Do not read it as evidence |
| 2026-09-14 13:43 | new pane, new run dir `01M2FVN3…`, seeded **mode 600** — the §5 chmod is in the running build. Same token as before: sha256-8 `accessToken=d31af002` `refreshToken=0888341f`, `expiresAt` still 19:53. **Nothing rotated**; the Keychain origin handed out the identical login, so the 7 h clock is unchanged and the interesting window is still ~19:53 |
| 2026-09-14 13:44 | `snapshot.sh` now falls back to `/tmp/ubiq-auth-watch/target` when its argument is gone, so the detached loop follows the live pane across a retirement. `UBIQ_LOG_FILE` confirmed still in `log.rs:347`; `/tmp/ubiq-auth.log` does not exist — **the §6 watch has never been started**, the app is not running under `UBIQ_LOG_FILE` |
| 2026-09-14 15:04 | file present and **untouched** — mtime still 13:43, 509 bytes, mode 600, `accessToken=d31af002` `refreshToken=0888341f`. 1 h 20 m into the run, nothing has been rewritten |
| 2026-09-14 15:04 | **the detached snapshot loop is blind.** Every row since 13:44 says `MISSING` while the file plainly exists: the loop was `nohup`'d from the *previous* pane, so it runs under that pane's sandbox and cannot read another run's directory — the `target` fallback added at 13:44 points it somewhere it is denied. `MISSING` from that loop is now meaningless. A watch has to run **outside any pane**, from the user's own terminal |
| 2026-09-14 21:56 | **the failure, caught live.** `.credentials.json` is **gone** from run dir `01M2FVN3…`, ~2 h after its 19:53 `expiresAt`. The pane is still alive and the run directory is **intact** — `.claude.json` is still being written (`backups/.claude.json.backup.1789415787673`, 21:56), `oauthAccount` still names the user. So this is **not** a run-dir removal and **not** a `park_agent` scrub: a scrub takes the whole login seed set, and `.claude.json` survived. H4 is out |
| 2026-09-14 21:56 | the credential was never rewritten before it vanished: the last observation (15:04) still had the seed's mtime 13:43 and fingerprints `d31af002`/`0888341f`. Between 13:43 and deletion **nothing wrote the file** |
| 2026-09-14 21:56 | **the Keychain item is blanked, and it is not this run's origin.** `security find-generic-password -s 'Claude Code-credentials'` returns `accessToken:""`, `refreshToken:""`, `expiresAt:0` — `sha256[:8]=e3b0c442` is the empty string, in both fields. Its `mdat` is **2026-09-09 14:54:07 UTC**, five days old: **nothing wrote the Keychain today**. And it claims `subscriptionType:"pro"`, `rateLimitTier:"default_claude_ai"`, where the seeded run credential was `team` / `default_claude_max_5x` — a *different account* |

## 9. What the 21:56 and 20:26 captures change

Two conclusions in §3 and §4 do not survive it.

**§3.1 is wrong about the origin.** It concluded that tier A finds nothing and so "the origin is the
Keychain". But the Keychain item was already **blanked on 2026-09-09** and describes a `pro` account,
while the credential this run was seeded with was `team`/`default_claude_max_5x`. A blanked blob is
what `credentials::login_is_usable()` exists to reject, so `ambient_login` cannot have supplied it.
`~/.claude/.credentials.json` still does not exist. **The durable origin of today's token is
therefore neither tier A nor tier B, and is not yet identified** — the remaining candidate is the
`files` secret store under `~/.config/agent-manager`, i.e. an imported account rather than a
zero-config login. That store cannot be listed from inside a pane; §10 asks for it.

**H1's mechanism needs restating.** H1 said the refresh is computed and the Keychain persist is
denied by Seatbelt. The Keychain's `mdat` shows no write today, which is consistent with a *denied*
write — but equally consistent with no attempt at all. What the capture does establish is the
**shape** H1 predicts: the run-dir fingerprint **never changed**, and the file was **removed**
rather than rewritten or blanked. On the evidence so far H1 remains the standing theory, now with
the observation that the origin it would have to be reconciled against is itself unusable.

**A second failure is now in play, upstream of the first.** Even a perfect write-back has nowhere
good to go: the origin is blanked and belongs to another account. Whatever seeded this run, the
`sync_login` reconciler ranks candidates and writes the winner back — and the only usable candidate
in the system was the copy that has now been deleted. **There may no longer be a usable Claude
credential anywhere on this machine.** §10's first question is whether one survives.

## 10. What the next session must ask the user

These cannot be answered from inside a pane — the sandbox answers *Operation not permitted*. Ask
the user to run them with the `!` prefix, from outside any managed pane:

```sh
ls -laR ~/.config/agent-manager            # the files secret store — is there an account, and a usable blob
ls -lt  ~/.config/ubiq/runs/               # which run dirs exist, and which still hold a .credentials.json
grep -rl claudeAiOauth ~/.config/ubiq/runs/ 2>/dev/null
```

The question that ranks above all others: **does any usable credential still exist**, in the store,
in another live run directory, or nowhere. If nowhere, a fresh `claude login` is needed before the
watch can be run again — and that login must be captured into the store, or the next run reproduces
this from a blank origin.

The §6 log watch has still never run (`/tmp/ubiq-auth.log` does not exist), so the `isolate::plan`
line that would show whether the policy can reach the Keychain — the one line that separates a
denied write from no write at all — is still unobserved. Starting it is unchanged in priority.
| 2026-09-14 20:26 | `_docs/log-claude.png` — a `sync_logins` pass, 33 min past expiry. **3 accounts, 5 run dirs.** Account **S** is this run's: origin `dir(/Users/mdn/.config/ubiq/accounts/S)`, `.claude/.credentials.json`, 509 bytes, `expiresAt=1789408388533` — the 19:53 stamp, so S is definitively the origin of run `01M2FVN33G…`. Three run dirs held it (`01M2FVN33G…`, `01M2FW139A…`, `01M2FVJVGB…`) and **all three carried the identical fingerprints** `accessToken=19955dd6 refreshToken=68ed7240` (DefaultHasher scheme), `usable=true`, all already expired. `picked … winner=origin`, `skipped writing back to the origin, already matches the winner` |
| 2026-09-14 22:04 | origin `accounts/S/.claude/.credentials.json` read directly: **mtime 11:53**, 509 bytes, mode 600, sha256-8 `d31af002`/`0888341f` — byte-identical to what the run was seeded with, **never written since 11:53**. |

## 11. The origin, settled — and H1 confirmed in shape

`_docs/log-claude.png` answers §10's first question outright, and §9's "the origin is not yet
identified" is superseded.

**The origin is a per-account directory under `~/.config/ubiq/accounts/<account>`**, not the
Keychain and not `~/.claude`. This run belongs to account **S**, whose
`.claude/.credentials.json` is the durable copy. Other accounts exist on this machine and share the same
machinery; they are **not** part of this investigation — everything below is account S. `provision::seed_zero_config_login`
and its two tiers — §3.1 — describe the *zero-config* path, which is simply not the path Ubiq takes
here: these runs are seeded from an explicit account. **§3.1's tier analysis is correct code and
the wrong path; the Keychain finding of §9 is true but irrelevant, a stale `pro` item from
2026-09-09 that nothing in this flow reads.**

**The reconciler works, and has nothing to reconcile.** At 20:26, 33 minutes after the token
expired, the pass reads the origin and all three run directories holding it and finds **one single
fingerprint across all four copies** — `19955dd6`/`68ed7240`, matching the origin's on-disk
`d31af002`/`0888341f` in the hand scheme. It correctly picks the origin as winner and correctly
skips a no-op write. There is no bug in `sync_login`, `newest_login` or the 30 s timer: H2 and H3
are **out**. The origin's mtime is still **11:53**, ten hours untouched.

**What this leaves is H1, confirmed in shape if not yet in mechanism.** Across the whole life of
the token — seed at 13:43, expiry at 19:53, deletion by 21:56 — **no fingerprint anywhere ever
changed**. Three separate live run directories all held the same expired login and not one of them
was rewritten. A refresh was certainly attempted; it persisted nowhere Ubiq can see, and the file
was then removed rather than updated. The remaining unknown is narrow and unchanged: **whether the
persist was denied or never attempted**, which only the `isolate::plan` line answers.

One new fact narrows it further: the run's sandbox can **read** its own account origin —
`accounts/S` is readable from inside the pane, while paths outside the run's own grants answer
*Operation not permitted*. So the policy grants the account path. Whether that grant is
read-**write**, and whether the harness would even use it, is the next thing to read out of the
log — if the origin is writable from the run and still was never written, the refresh never
produced a credential at all.

## 12. The two questions, and H5

The investigation reduces to two questions. They are not independent.

### Why was it never refreshed?

Established, not theorised: **no fingerprint anywhere ever changed.** The origin's mtime is 11:53,
ten hours before the failure. The 20:26 pass read the origin and all three run directories holding
account S's login and found one identical, already-expired login in all four. So whatever the
harness did at expiry, **it persisted nothing Ubiq can see** — and the reconciler had nothing to
carry, which is why it is not at fault.

Two mechanisms remain, and only the `isolate::plan` line separates them: the persist was **denied**
by the sandbox, or **never attempted** because the refresh call itself failed. §11 leans on the
first; H5 below is a reason to take the second seriously.

### Why was it deleted?

A deletion that takes `.credentials.json` and leaves `.claude.json` and the directory intact is not
the shape of any Ubiq teardown — those take the whole seed set or the whole directory (§3.4, H4).
It is the shape of a **harness logging itself out**: Claude Code clears its credential when the
server tells it the credential is finished. The trigger for that is an `invalid_grant` on refresh —
which is what the server answers for a refresh token that has already been **spent**.

### H5 — three live runs share one single-use refresh token

**Account S's login was seeded, byte-identical, into at least three concurrent run directories**
(`01M2FVN33G…`, `01M2FW139A…`, `01M2FVJVGB…` — the 20:26 pass names all three). OAuth refresh
tokens **rotate**: the first use mints a new pair and retires the presented token. `sync_login`
already knows this — its step 4 exists precisely because "a refresh **rotates** the refresh token
and every other run is now holding a revoked one" (§3.3). But that reconciler runs **every 30 s,
after the fact**. The window between one run spending the token and the others being handed the
replacement is up to 30 s, and at expiry all three runs reach for it at once.

So: one run refreshes, spends the shared token, and — for whatever reason in §11 — **fails to
persist** the result. The other two present the now-revoked token, get `invalid_grant`, and
**delete their credential**, which is exactly what was observed. The winner's own copy is never
written either, so nothing distinguishes it in the log.

This explains both questions with one mechanism, and it explains the timing: nothing goes wrong
until the access token expires and all live runs refresh within the same minute of each other.

**What would confirm it**: run **exactly one** Claude pane on account S across an expiry. If a
single run survives its expiry and a fleet of three does not, H5 holds and the fix is about
serialising the refresh, not only about the sandbox.
**What would falsify it**: a single lone run also loses its credential at expiry — then the fault
is entirely in the persist path (§11) and the concurrency is incidental.

H1 and H5 are not exclusive. A denied persist makes H5 *certain* to fire whenever more than one run
is live, because the winner can never publish the replacement.

## 13. H1 is falsified — the sandbox blocks nothing

Read out of `isolate.rs` against the working tree:

- the **run's config dir is read-write** (`isolate.rs:541`, `read_write_grants` at `isolate.rs:934`
  puts it first);
- **`~/Library/Keychains` is granted `rw`, unconditionally, to every sandboxed run.** `plan()`
  extends `cfg.default_profiles` with `DEV_LAYERS` regardless of `cmd[0]` (`isolate.rs:569-570`),
  and `DEV_LAYERS` (`isolate.rs:297`) leads with `integrations/keychain`, marked *not optional*;
  that profile grants `~/Library/Keychains` read-write plus mach-lookup on `securityd`/`secd`/
  `trustd`. `auto_profiles` is not even what pulls it in. The contrast `login_confined()`
  (`isolate.rs:606`) sets `auto_profiles = false` and **omits** that layer on purpose — the §3.5
  doc comment describes the capture policy, and does not apply to a normal run;
- **outbound network is open** — `builtin_defaults()` includes `macos/system-runtime`, which is
  `(allow network*)` unconditionally, with a comment saying tiered network isolation is not
  implemented and a confined process can reach any host. Proxy and CA env vars are forwarded too;
- the **account origin dir is read-only** to the run (`read_only_grants`, `isolate.rs:979-982`,
  from `Source::Dir(account.home)`) — which is correct, it is not a write target.

**So there is nothing to deny.** The run could have written its own credential file, and could have
written the Keychain, and could reach the token endpoint. H1's mechanism — a Seatbelt-denied
persist — cannot be what happened.

That inverts the §11 conclusion into something sharper. The Keychain is **writable** by the run and
its `mdat` is still **2026-09-09**; the run dir is **writable** and its file was never rewritten.
A successful refresh had two permitted places to land and landed in neither. **No refresh ever
succeeded.** The question is no longer where the result went — it is why the call failed.

The grep for the watch, now that the line is known:

```sh
grep 'resolved sandboxed run policy' "$UBIQ_LOG_FILE"   # layers=, rw=, ro=, home=
grep 'account login origin resolved' "$UBIQ_LOG_FILE"   # which account dir was granted
```

## 14. H7 — the seeded refresh token was already spent

If no refresh ever succeeded all day, the simplest explanation is that the refresh token in
`accounts/S` **was already retired before the day began**.

The origin was written at **11:53** and never again. Its access token was minted with a 19:53
expiry, so it worked all afternoon — a bearer token needs no refresh to keep working. Nothing would
reveal a dead refresh token until 19:53, when every run tried to use one and got `invalid_grant`,
cleared its credential and logged itself out. That is the observed behaviour exactly, and unlike H5
it does not require the three runs to race.

It also fits the surrounding wreckage: the Keychain item was **blanked on 2026-09-09**, which is
what a Claude logout looks like. A login that was rotated or revoked outside Ubiq leaves the stored
copy stale, and a capture taken at 11:53 preserves whatever was true then, not what is true at
expiry.

**H5 and H7 make the same prediction for a fleet and differ for a lone run** — H5's single pane
survives its expiry, H7's does not. The single-pane test in §12 therefore separates them, and is
still the cheapest next move.

**A faster, destructive-but-free test exists.** Present the origin's refresh token to the token
endpoint by hand:
- `invalid_grant` ⇒ **H7 confirmed**, and nothing was lost — the token was already dead;
- success ⇒ H7 falsified, the token was live, and the new pair **must** be written into
  `accounts/S/.claude/.credentials.json` immediately or that success is itself the rotation that
  breaks the account.

It needs the user's go-ahead precisely because of the second branch.

## 15. The deletion is not Ubiq's — traced exhaustively

Every path in `ubiq-host` and `agent-manager` that can touch a run dir's credential was traced
against the working tree. **None of them produces the observed state.**

| path | what it removes | when |
|---|---|---|
| `Agents::retire` / `retire_agent` (`agent.rs:1542`, `1607`) | the **whole run dir** | after the pane's process has exited |
| `Agents::sweep` (`agent.rs:1568`) | whole dir, or `scrub_login` for a persistent run | **startup only** |
| `Agents::scrub_login` (`agent.rs:1431`) | **every** `login_seed` entry — for Claude, `.credentials.json` *and* `.claude.json` in the same loop | from `park_agent` (conversation ended) or the startup sweep |
| `Agents::park_agent` (`agent.rs:1461`) | via `scrub_login`, both files | conversation ended |
| `refresh_login` / `sync_login` / `write_credential` (`agent.rs:1319`, `harness/mod.rs:414`, `625`) | **never deletes** — stages a temp file and `rename`s, and refuses any blob failing `login_is_usable` or older by expiry | the live 30 s timer |
| `sign_out` / `delete_account` (`account.rs:375`) | files under the **account home**, never a run dir | explicit CLI |
| `run::cleanup` (`run.rs:196`), `overlay::sweep_old_runs` | whole dir, `Ephemeral` strategy only — Ubiq uses `Fixed` | not on Ubiq's path |

The single "delete files, keep the directory" path is `scrub_login`, and it removes the **whole seed
set together**: it cannot leave `.claude.json` behind. Every path that can remove the credential
alone removes the entire directory with it. And the one family that runs *while a pane is alive* —
the 30 s reconciler — has no delete in it at all.

**So the Claude Code process deleted its own credential.** That is now established by exhaustion,
not inferred from shape.

One refinement worth recording: `credentials/mod.rs:181-186` documents the *known* Claude failure
mode as rewriting the file **blanked**, not removing it — which is why `login_is_usable` exists.
What was observed at 21:56 is an **absent** file, not a hollow one (the `ls -la` shows no entry at
all, and `find` over the run dir finds nothing). **The harness took a path the tree does not yet
describe** — a full logout rather than a blanking. That distinction matters for the fix: a blanked
file is something `sync_login` already defends against; a deleted one leaves the run with nothing to
reseed from except the origin, which is exactly the stale copy under suspicion in §14.

## 16. ROOT CAUSE — Claude Code stores credentials in a keychain item keyed by the config dir

A forced `/login` at 22:21 settled it. **Claude Code 2.1.270 on macOS does not persist to
`$CLAUDE_CONFIG_DIR/.credentials.json` at all.** It writes a login-Keychain generic password whose
service name is namespaced by the config directory:

```
Claude Code-credentials-<sha256(CLAUDE_CONFIG_DIR)[:8]>
```

Verified arithmetic: this run's `CLAUDE_CONFIG_DIR` is
`/Users/mdn/.config/ubiq/runs/01M2FVN33G91HNRWF2EPQDG28P`, and
`sha256(path)[:8] = 2c922a00` — exactly the item the login wrote. No trailing slash, no
normalisation; the raw value of the variable.

**Evidence from the login.** Immediately after `/login` succeeded:

| place | state |
|---|---|
| `$CLAUDE_CONFIG_DIR/.credentials.json` | **still absent** — not recreated |
| `accounts/S/.claude/.credentials.json` | untouched, mtime still **11:53** |
| `Claude Code-credentials` (unsuffixed) | untouched, `mdat` still **2026-09-09** |
| `Claude Code-credentials-2c922a00` | **written**, `mdat` 20:21:01Z = 22:21:01 local |

The new item holds a fresh, usable login — sha256-8 `accessToken=567647b0`
`refreshToken=5d7b898a`, `expiresAt` 2026-09-15 **06:21** — and `.claude.json`'s
`profileFetchedAt` is 22:21:01.900, the same second. A successful login is therefore **completely
invisible** to every file Ubiq watches.

**The deletion is explained, and it was not a logout.** That item's `cdat` is **19:56:25Z =
21:56:25 local** — the very minute §8 recorded the file as missing. What happened at 21:56 was a
**migration**: Claude read the seeded `.credentials.json`, wrote it into its namespaced Keychain
item, and removed the file. §15 was right that no Ubiq path deleted it and right that the harness
did, but the motive was migration, not a server-driven logout. (`.last-update-result.json` shows a
native self-update from 2.1.270 attempted at 21:27 and *failed*, so the trigger was the running
binary's own migration step, not a new version.)

**The refresh is explained too.** After migration there is no file to rewrite, and before it the
binary was already keychain-first. Every refresh Claude performs lands in
`Claude Code-credentials-<hash>`, which `sync_login`, `harvest_login` and `newest_login` never read.
The reconciler is not broken — **it is watching a location the harness abandoned.**

### Why this looks like "credentials vanish after some hours"

Nothing fails while the seeded access token is valid, because a bearer token needs no refresh. At
its expiry the harness refreshes into the Keychain, Ubiq never sees the new token, and the next run
seeded from the origin gets the **stale 11:53 copy** — whose refresh token may by then have been
spent and rotated away by an earlier run. That is the reported symptom exactly.

### The 92 orphans

`security dump-keychain` lists **92** `Claude Code-credentials-*` items. Every run directory is a
fresh ULID, so every run mints a **new Keychain item that nothing ever deletes** — `retire` removes
the directory and leaves the credential behind. This is both a leak and a disclosure issue: 92 live
refresh tokens are sitting in the login Keychain.

### What this invalidates

- **H1, H5, H7 are all moot as stated.** They argue about a file the harness no longer uses.
  H5's shared-token race remains *plausible* for the origin copy across runs, but it is downstream
  of this.
- **§3.2's `login_seed`** is a one-way seed: Claude consumes `.credentials.json` and never writes it
  back. `SeedFile::credential(...)`'s write-back contract does not hold for this harness version.
- **§7's proposed fix is wrong.** Denying the Keychain would not make Claude fall back to a file —
  §3.5 already records that 2.1.218+ treats an unreachable keychain as an **error**. Granting it
  more is not the answer either; it is already granted `rw` (§13).

### The shape of the fix

Ubiq must treat the **Keychain item as the credential of record** for Claude Code on macOS, keyed
by `sha256(config_dir)[:8]`:

1. **harvest** from `Claude Code-credentials-<hash(run_dir)>` rather than from the run dir's file,
   and rank that blob in `newest_login` as it ranks a file copy today;
2. **seed** by *writing* that item before launch, instead of (or as well as) dropping
   `.credentials.json` — the file seed can stay as the migration source, but it cannot be the
   channel back;
3. **delete** the item in `retire`/`scrub_login`, which also stops the leak;
4. keep the file path for non-macOS and for older harness versions — this is a macOS-specific
   storage backend, so it belongs behind the same `Harness` seam that already owns
   `ambient_login`/`adopt_login` in `harness/claude.rs`.

This needs a `Dnn` decision row: it changes what agent-manager considers the durable location of a
Claude credential.

**Immediate cleanup available**: the 92 orphaned items should be pruned, keeping only
`2c922a00` (this live run). Not yet done — it is destructive and was not authorised.

## 17. Why a standalone install survives, and whether the key is predictable

### The key is predictable, and Ubiq knows it before launch

`sha256($CLAUDE_CONFIG_DIR)[:8]`, over the raw variable value — no trailing slash, no normalisation.
Checked against the 92 items: this run's path hashes to `2c922a00`, which is present; every other
path tested is absent, and no item matches a path we did not expect. A 32-bit exact match on the one
run known to have written is not coincidence.

**Ubiq chooses the run directory**, so it can compute the service name *before* spawning the
harness. Both directions are therefore open to it: write the item to seed a login, read it back to
harvest one. That is what makes §16's fix tractable.

Two caveats worth keeping: the derivation is inferred from a single writing run, so re-verify it
when the harness version changes; and it is an undocumented implementation detail of the harness,
which argues for §18's option over depending on it.

### The standalone install: the difference is the *stable* config dir, not keychain isolation

The proposed explanation — that a standalone install isolates keychain access and so keeps using the
file — does not survive the evidence. `~/.claude/.credentials.json` **does not exist** on this
machine, and `~/.claude` hashes to `1bc99cf3`, which is **not** among the 92. What does exist is the
unsuffixed `Claude Code-credentials` item, `cdat` **2026-02-07**, `mdat` **2026-09-09** — seven
months of continuous updates in the *keychain*. A standalone install is keychain-backed too.

So the difference is not *where* it stores. It is that a standalone install has **one fixed config
dir for its whole life**, hence **one stable keychain key**. Refresh writes the item; the next launch
reads the same item; the credential survives for months with no reconciliation at all.

Ubiq mints a **new ULID run directory per run**, so every run has a **different keychain key**. A
refresh lands in an item keyed to a directory that is deleted at teardown, and the next run is keyed
somewhere new and seeded from the stale file origin. **The credential cannot survive a run boundary
by construction.** That is the bug, stated exactly.

## 18. The two candidate fixes, assessed

**Option A — deny the keychain, stay on disk.** Not viable as stated. §3.5 already records that
Claude Code 2.1.218+ treats an unreachable keychain as an **error rather than falling back to a
file**, which is precisely why `login_confined()` exists for captures. And §16 shows the current
binary *migrates the file into the keychain and deletes it*, so the file path is not merely
unpreferred, it is actively abandoned. Pursuing this means fighting the harness every version.

**Option B — handle the keychain item.** Correct, and feasible now that the key is derivable
(§17). The work is §16's four steps: harvest from the item, seed by writing it, delete it on
retire (which also clears the 92-item leak), all behind the `harness/claude.rs` seam.

**Option C — give each account a stable config dir, which neither bullet proposed.** This is what
makes a standalone install work, and it is the smallest change: key the run directory by
**account** rather than by a per-run ULID, so a Claude run reuses one directory and therefore one
keychain item across runs. The credential then survives exactly as it does standalone, with no
harvesting, no seeding and no dependence on an undocumented hash.

Its cost is that a run directory is no longer private to a run — concurrent runs on one account
share a config tree, which is what the per-run ULID was buying. Whether that is acceptable depends
on what else in the tree must stay per-run; `.claude.json`, history and sessions would become
shared, which may be desirable or may not.

**Recommendation: C if per-account sharing is acceptable, B otherwise.** B and C are compatible —
C removes the need for B in the common case, and B still covers a run that must be isolated. A is
not recommended in either case.

## 19. The standalone install measured — §16's conclusion corrected, Option A is the fix

The user's working standalone install was measured directly. Its config dir is
`/Users/mdn/works/_dev-home/.claude-syn` (not reachable from inside a pane — *Operation not
permitted*; the credential was copied out to `refs/sample-credentials.json` to be read).

| check | result |
|---|---|
| `.credentials.json` on disk | **exists**, 509 bytes |
| `sha256(config_dir)[:8]` = `b8e483cd` | **absent** from the 92 keychain items (`…/` variant `cba978d8` absent too) |
| token freshness | `expiresAt` **2026-09-15 06:33**, `refreshTokenExpiresAt` **2026-10-11** |
| fingerprints | sha256-8 `accessToken=4a81a8f2` `refreshToken=701e19fc` |

**This install has no keychain item and a freshly refreshed file.** An 8-hour access token expiring
at 06:33 tomorrow was minted ~22:33 today, so Claude Code 2.1.270 **did rewrite
`.credentials.json` on refresh**, and has been doing so for a month.

### What was wrong in §16

§16 concluded that 2.1.270 "does not persist to `.credentials.json` at all" and that the file path
is "actively abandoned". **That is wrong as a general claim.** The harness has *two* backends and
chooses between them at runtime. What §16 observed was the choice a *Ubiq* run makes, not the only
choice available. §18's assessment of Option A inherited the error and is withdrawn; §3.5's note
that 2.1.218+ errors rather than falling back does not hold for 2.1.270, which falls back cleanly.

### What actually selects the backend

The discriminator is **whether the keychain is reachable**, and the two installs differ exactly
there:

- the standalone runs under a **relocated `HOME`** (`/Users/mdn/works/_dev-home`), so there is no
  `~/Library/Keychains` to use → it takes the **file** backend, and refreshes into the file;
- a Ubiq run inherits the **real `HOME`** (`HOME=/Users/mdn`, confirmed in-pane) and `plan()` grants
  `~/Library/Keychains` read-write unconditionally via `DEV_LAYERS` (§13) → the keychain is
  reachable, so Claude takes the **keychain** backend, migrates the seeded file into
  `Claude Code-credentials-<hash>` and deletes it (§16).

Ubiq is handing Claude the keychain and then watching the file. The harness is behaving correctly
in both installs.

### The fix: Option A, restored and now evidence-backed

**Deny a Claude run the keychain, exactly as `login_confined()` already denies it for a capture**
(`isolate.rs:606`, `auto_profiles = false`, `integrations/keychain` omitted). The run then takes the
file backend, refreshes into `$CLAUDE_CONFIG_DIR/.credentials.json`, and **every existing mechanism
works unchanged** — `harvest_login`, `sync_login`, `newest_login` and the 30 s reconciler are all
correct code that has simply had nothing to read. §7's original proposal was right, and the
standalone install is a month-long proof that it works.

Concretely: drop `integrations/keychain` from `DEV_LAYERS` for a Claude run, or relocate the run's
`HOME` so no keychain exists. The first is narrower and matches the capture path already in the
tree.

This supersedes §16's fix (harvest the keychain item) and §18's Option C (stable per-account config
dir). Both remain workable, but neither is needed: Option A leaves the credential in the one place
Ubiq already manages, keeps per-run isolation, and needs no dependence on an undocumented service
name.

**Still worth doing independently**: pruning the 92 orphaned keychain items (§16), which are a leak
regardless of which fix lands. Once Option A is in place no new ones are minted.

A `Dnn` row is still required — the change removes a capability from a sandboxed run.

## 20. The fix, as landed (2026-09-14)

`isolate::plan()` now withholds the login keychain from a Claude run, so the harness takes its file
backend and `.credentials.json` becomes the credential of record again — the thing the whole
`harvest_login` / `sync_login` / `newest_login` machinery has always assumed.

### What it does

- **`KEYCHAIN_DENIED`** (`isolate.rs`) lists the harnesses: `claude-code`, `claude-code-acp`.
  `keychain_denied()` gates on it **and on macOS**, since elsewhere there is nothing to withhold.
- **`write_keychain_override()`** generates `<state_dir>/profiles-no-keychain/integrations/keychain.toml`
  and `plan()` appends that directory to `isol8::Config::profile_paths` for a denied harness.
- The generated layer is a **replacement** for the built-in `integrations/keychain`, not an
  omission. It keeps `securityd`/`trustd` mach-lookups, `/Library/Keychains/System.keychain` and
  `/private/var/db/mds`, and sets **only `~/Library/Keychains`** to `access = "none"`, under both
  the subtree and the literal-metadata grant.
- `plan()`'s log line gained **`keychain_denied=`**, so a run says in one field which backend it
  will take.

### Why an override rather than dropping the layer

This is the part that cost the most and is worth not rediscovering. Removing
`integrations/keychain` from `DEV_LAYERS` **does not remove it from the policy**: `agents/claude-code`
lists it in its own `requires`, isol8 auto-matches that layer on `cmd[0] == claude`, and
**auto-matched layers resolve after named ones** (`select_layer_names`, isol8
`crates/isol8-core/src/profile.rs`). A deny layer named alongside `DEV_LAYERS` therefore sits
*below* the grant it is meant to beat, and paths merge highest-layer-wins. The first attempt did
exactly this and the test caught it.

A profile path replaces a same-named built-in **wherever it is pulled in**, so the override is the
one form nothing can outrank. `LayerRegistry::load` applies `profile_paths` last, and
`integrations/keychain` is the only layer in the whole profile set that grants
`~/Library/Keychains` — checked, not assumed.

It gets **its own directory** rather than the caller's profile root, because a profile path applies
to every run that names it: sharing the root would impose the override on every other harness.

### Why the TLS grants stay

`DEV_LAYERS` documents that `integrations/keychain` is not optional because rustls-based tools
(`mise`, `cargo`) validate TLS through the trust daemon. That need is satisfied by the mach-lookups
and the **system** keychain, not by the user's login keychain — so the override keeps them and a
denied run can still `cargo fetch`. `a_keychain_denied_harness_cannot_reach_the_login_keychain`
asserts both halves: the login keychain denied, `System.keychain` still reachable.

### Tests

- `a_keychain_denied_harness_cannot_reach_the_login_keychain` — launches as `claude` so the
  auto-match and its `requires` are genuinely in play, and asserts on the **merged grant**, not the
  layer list. Asserting on the layer list is the trap: the layer is *expected* to be present, and
  only its content is overridden.
- `a_harness_outside_the_deny_list_keeps_the_login_keychain` — a `codex` run keeps the layer whole,
  so the fix for one harness has not narrowed the sandbox for every other.

Both are `#[cfg(target_os = "macos")]`. `cargo test -p agent-manager`: **545 passing**, with the
same two pre-existing environment failures recorded in §5.4 and no others.

### Code annotations

The discovery is recorded where someone would hit it, not only here:

- `harness/claude.rs`, on `config_anchor()` — the two backends, what selects them, and that the
  `login_seed` is a one-way door unless the policy withholds the keychain.
- `isolate.rs`, on `KEYCHAIN_DENIED` — the same, from the policy side, with the per-run key and why
  removing a harness from the list silently moves its credential out of reach.
- `account.rs`, on `CLAUDE_KEYCHAIN_SERVICE` — that the constant is the **default-config-dir name
  only**, that a relocated run's item is namespaced, and that a blanked item under the plain name
  says nothing about whether a usable login exists elsewhere.

## 21. If this is not resolutive

The fix rests on one claim: **an unreachable login keychain makes Claude Code use
`.credentials.json` and rewrite it on refresh.** That is measured on a standalone install running
this way for a month (§19), and `login_confined`'s doc comment records the same behaviour for a
capture. But it is measured on a *relocated `$HOME`*, not on a *policy deny*, and those are not the
same thing — §3.5 already records that 2.1.218+ reports a merely-unreachable keychain as an error.
If 2.1.270 distinguishes them, this fix fails and the log will say so.

**Check these first, in order.** All of it is now in one log line and one file:

1. `grep 'resolved sandboxed run policy' "$UBIQ_LOG_FILE"` — is `keychain_denied=true` on the
   Claude run at all? If not, the harness id is not in `KEYCHAIN_DENIED` (an ACP run, or a renamed
   id) and nothing else below matters.
2. Does the pane **start**? A harness that treats the deny as an error fails at launch, loudly.
   That is the good failure: it is immediate and unambiguous, not a seven-hour one.
3. Watch `$CLAUDE_CONFIG_DIR/.credentials.json` across an expiry. **The mtime changing is the whole
   experiment.** Every observation in §8 has the same mtime as the seed; one that moves means the
   file backend is live and the fix works.
4. `security dump-keychain | grep -c 'Claude Code-credentials-'` — the count must **stop growing**.
   A new item after this landed means a run still reached the keychain.

**If the deny is rejected rather than honoured**, the next move is the one the standalone install
proves: relocate the run's `$HOME` (`HomeMode::Managed`) instead of denying a path, so there is no
`~/Library/Keychains` to reach at all. That is a larger change — the home is the caller's choice,
and `DEV_LAYERS`' `~`-relative grants expand against the effective home (see
`home_mode_inherit_sets_neither_home_field`) — which is why it was not the first attempt.

**If the deny is honoured and the credential still dies**, the fix worked and something else is
wrong. The remaining candidate is §12's H5, which this change does not address: three live runs
share one origin copy, refresh tokens are single-use, and `sync_login` redistributes the rotated
token only every 30 s. The test for it is unchanged — **one pane across an expiry survives, three
do not** — and the fix would be to serialise the refresh or narrow that window.

**Discarded, so nobody re-runs them**: H1 (sandbox denied the write — §13, everything was granted),
H2/H3 (the reconciler failed — §11, it was correct and had nothing to carry), H4 (`park_agent`
scrubbed it — §9, `.claude.json` survived), H7 (the seeded token was born spent — §16, the file was
migrated away, not rejected).

### Housekeeping this change does not do

The **92 orphaned keychain items** predate the fix and stay until removed by hand; the user is
doing that. No new ones are minted once this is in force, which is what item 4 above checks.
