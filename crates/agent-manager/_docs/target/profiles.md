# Profiles & agents

> A **profile** is a named, persistent base — an account (login), a set of
> defaults, and a default isolation policy — from which every run makes a
> **throwaway overlay**. An **agent** is a profile with its composition frozen.
> This doc defines the model, the cross-harness "cleanest solution" it rests on,
> and the phased path to get there.

## 1. Why: two config lifetimes

Everything a run needs from a harness's config splits into two halves with
**opposite lifetimes**:

| Concern | Examples | Lifetime | Scope |
|---|---|---|---|
| **Identity / base state** | credentials, `hasCompletedOnboarding`, user settings, theme | **persist** across runs | per *account* |
| **Run composition** | skills, MCP servers, hooks, instructions, model | **ephemeral**, per run | per *run* |

The current code puts composition in an ephemeral config dir and tried to get
identity by relocating `HOME` to a per-account directory. That was wrong on two
counts (see §4): it broke credential discovery *and* stripped the user's
toolchain. A **profile** fixes the split by making the ephemeral run dir an
**overlay over a persistent per-profile base**: seed identity in, layer
composition on top, throw the overlay away.

## 2. Usage tiers

Three personas in ascending order of control — and the boundary between them is
exactly **how much of `HOME` they give up**:

| Tier | Who | Typical command | Auth | HOME | Isolation |
|---|---|---|---|---|---|
| **A — casual** | just run an agent | `am claude --mcps a,b --model haiku --prompt 'hi'` | default login (real `~/.claude`), lazy-captured into the implicit `default` profile | **real** | occasional `--isolate` |
| **B — expert** | curated, repeatable setups | `am claude --profile work` (+ per-run overrides) | named profiles, possibly **multiple accounts**, `--account` overrides | **always real** | opt-in per profile/run |
| **C — hardcore** | full sandboxes | `am claude --profile ci --isolate=locked` | multiple accounts, each in its own **replaced** HOME | **replaced** (isol8), toolchain reconstructed | always |

**The central dividing line: A and B never touch `HOME`.** They rely entirely on
the seed-into-relocated-config-dir mechanism (§5), so the user's toolchain always
survives — switching accounts or profiles never costs `nvm`/`mise`/`pyenv`.
**C is the only tier that replaces `HOME`,** and does so deliberately, accepting
that the toolchain must be reconstructed inside the sandbox (§8). HOME
replacement is thus an *explicit opt-in*, never a silent side effect of picking
an account.

How each tier uses the machinery:

- **A (casual).** No profile authored. The implicit `default` profile lazily
  captures the existing login on first use (§6.1); composition is 100% per-run
  flags. `--isolate` wraps the launch (§8) without changing auth or HOME.
- **B (expert).** Authors `profiles/<name>/profile.toml` fixing an account +
  defaults; may keep several accounts and switch via `--account`/`--profile`.
  Because every account is reused by **seeding**, switching is free of any
  toolchain cost. Per-run flags still override the profile — the sweet spot the
  whole "cleanest solution" is built for.
- **C (hardcore).** Runs under isol8 with a replaced HOME per account/profile,
  seeding the login into the *sandbox* HOME. This is also the only correct home
  for Class-C harnesses like grok, which have no config lever (§5).

## 3. The hard constraint: don't touch `HOME`

Relocating `HOME` has **strong, non-obvious consequences**. A child launched
under a synthetic `HOME` loses everything anchored there:

- version/tool managers — `nvm`, `mise`, `pyenv`, `rbenv`, `asdf`, `volta`
- shell rc and PATH shims (`~/.zshrc`, `~/.local/bin`, `~/.cargo/bin`, …)
- SSH keys/config, git config, cloud credentials, language caches

All of it is reconstructable, but only *deliberately*. So the governing rule:

> **Never relocate `HOME` merely to inject config. Relocate the harness's own
> config/data dirs via its native env levers, seed captured credentials into
> those relocated dirs, and leave the real `HOME` (and the toolchain) intact.**

`HOME` relocation is reserved for **explicit, opt-in full isolation** (isol8,
§8), where reconstructing the environment is the whole point.

## 4. What was broken (and the empirical findings)

`provision()` (Claude) used to set `HOME = account.home` *and*
`CLAUDE_CONFIG_DIR = <ephemeral dir>`. Two failures:

1. **Credentials never found → perpetual onboarding.** Verified against Claude
   Code **2.1.206**: `CLAUDE_CONFIG_DIR` relocates the *entire* config —
   `.credentials.json`, `.claude.json`, `projects/`, `sessions/` all move into
   it, and `HOME` stays untouched. (The old `_docs/harness/claude-code.md` claim
   that `CLAUDE_CONFIG_DIR` moved only `.claude/` and not `~/.claude.json` is
   **stale** — corrected there now.) So the HOME-resident creds were never read;
   the empty ephemeral config dir made every run look like a first run.
2. **Toolchain stripped.** `account.home` was a bare dir — no `nvm`/`mise`/etc.

**Fix (landed for Claude):** copy `<home>/.claude/.credentials.json` →
`$CLAUDE_CONFIG_DIR/.credentials.json` and `<home>/.claude.json` →
`$CLAUDE_CONFIG_DIR/.claude.json`, and **do not** set `HOME`. Validated
end-to-end: a headless run against a fresh seeded config dir returns `AUTH_OK`
with no onboarding, real `HOME` intact.

## 5. The cleanest solution, generalized across harnesses

Harnesses differ in whether their config lever and their credential store are
the *same* dir. Three structural classes:

| Class | Harnesses | Config lever (relocatable, HOME-independent) | Credential store | Clean strategy |
|---|---|---|---|---|
| **A — unified root** | Claude Code (`CLAUDE_CONFIG_DIR`), Codex (`CODEX_HOME`) | one env var relocates config **and** creds | inside that root | point lever at ephemeral dir → **seed creds in** → keep real `HOME` |
| **B — split store** | opencode (`OPENCODE_CONFIG_DIR` for config; auth under `~/.local/share/opencode`, HOME-relative) | config only | HOME-relative *data* dir | relocate config via lever **and** relocate the data dir via its own lever (`XDG_DATA_HOME`?) → seed creds there → keep real `HOME`. If no data-dir lever exists, fall back to Class C for creds only. |
| **C — HOME-only** | grok (`~/.grok`, no `GROK_CONFIG_DIR`) | none | HOME-relative | **must** relocate `HOME` → toolchain caveat applies → prefer pairing with isol8 (§8) |

So the generalized design: **the same seed-into-relocated-dir pattern for A and
B; `HOME` relocation only for C.** Class C is precisely the case that motivates
first-class isolation — its non-isolated form is inherently lossy, and that
should be surfaced, not hidden.

### 5.1 Express it in the `Harness` trait

Replace per-harness bespoke credential logic with declarative metadata + one
generic seeding step:

```rust
/// Where a harness's config/credentials live, and how to relocate + seed them.
struct ConfigAnchor {
    /// Env vars that relocate config/data into a dir we control, keeping HOME.
    /// e.g. [("CLAUDE_CONFIG_DIR", Relocate::All)]  (Class A)
    ///      [("OPENCODE_CONFIG_DIR", Relocate::Config),
    ///       ("XDG_DATA_HOME", Relocate::Data)]      (Class B)
    ///      []                                        (Class C → relocate HOME)
    levers: Vec<(String, Relocate)>,
    /// Files that constitute a captured login, seeded generically:
    /// src is relative to `account.home`; dst is relative to the relocated dir.
    login_seed: Vec<SeedFile>,   // e.g. .claude/.credentials.json → .credentials.json
    /// True only for Class C: no lever, HOME must be relocated (toolchain caveat).
    requires_home_relocation: bool,
}
```

A single `seed_login(dir, account.home, harness.config_anchor().login_seed)`
then serves every Class A/B harness. (The Claude `seed_account_login` we landed
is the concrete first instance of this — generalize it here.)

## 6. The profile model

```
profile (persistent)                         run (ephemeral overlay)
profiles/<name>/
├── base/            ← identity per harness   materialize ─────────┐
│   └── <harness>/   (e.g. .claude/.credentials.json, .claude.json)│
├── profile.toml     ← defaults                                     ▼
│   ├── account       = "<id>"               <relocated config dir> (ephemeral)
│   ├── harness       = "claude"  (optional)  ├── (seeded identity from base/)
│   ├── mcps/skills/hooks/model/instructions  ├── mcp.json    ← composition
│   └── isolate       = false | "<policy>"    ├── skills/     ← composition
```

A run resolves as: **profile → materialize overlay (seed base + write
composition) → wrap for isolation (§8) → launch.** Consequences that fall out:

- **One profile, many compositions.** Identity/base is fixed; `--mcps postgres`
  vs `--mcps figma` only changes the overlay. Your "single profile, different
  MCPs/skills" requirement is free.
- **`account` becomes a field of the profile**, not a parallel concept. The
  bare `[defaults].account` in settings is subsumed by a default *profile*
  (§7). (Accounts stay a first-class store; a profile *references* one.)

### 6.1 Zero-config default (make "it just works" the default)

`am claude` with no `--profile` uses an implicit `default` profile that **lazily
captures your existing login on first use**: seed `profiles/default/base/claude/`
once from the real `~/.claude` (creds + `.claude.json`), persist it, reuse it
forever after. First run bootstraps; every run after is logged-in with zero
flags. Named profiles and per-run flags layer on top.

## 7. Resolution & the default question

Precedence (highest wins, replace-by-default, matching existing merge rules):

```
CLI flag  >  --profile <name>  >  [defaults].profile  >  implicit "default"
```

- **"Do we need a `default` property?"** — express it as a **default profile**
  (`am profile use <name>` → `[defaults].profile`), not a bare default account.
  One knob for "what runs by default"; the account default is just a field of
  that profile.
- Per-run flags (`--mcps`, `--model`, `--account`, `--isolate`) still override
  the profile's fields, so a profile is a *default*, never a straitjacket.

## 8. Isolation is an orthogonal axis

Profile = *what config*; isolation = *what sandbox*. `isolate.rs` already wraps
a `Launch` as a pure argv transform, so the two compose cleanly:

- A profile carries a **default** for the isolation axis (`isolate = false`, or
  `isolate = "dev.toml"` naming an isol8 policy). Per-run `--isolate[=profile]`
  overrides.
- **Class C harnesses (grok) are where the axes meet.** Their non-isolated form
  must relocate `HOME` (lossy). Under isol8 the sandbox HOME is reconstructed
  deliberately, so seed the login into that sandbox HOME and the toolchain
  caveat becomes an explicit, understood cost rather than a silent breakage.
- Even for Class A, full isolation is available: run inside an isol8 HOME and let
  `CLAUDE_CONFIG_DIR` point inside it. The seeding step is identical; only the
  `HOME` the child sees changes.

## 9. Materializing the overlay (symlink-else-copy, GC, Windows)

- **`materialize` abstraction:** identity files (creds, `.claude.json`) are
  linked back to `profiles/<name>/base/` when possible; composition files
  (mcp.json, skills) are freshly written and **owned** by the run.
- **Copy vs symlink:** credentials that the harness *rewrites in place* (OAuth
  refresh) are currently **copied** (safe, but a refreshed token is discarded at
  cleanup). Persisting refreshes back to `base/` is a profile-layer follow-up
  (copy-back on exit, or symlink with care — harnesses that replace the file via
  rename break a symlink).
- **Manifest:** record linked-vs-owned per file so cleanup can never delete a
  profile's real base by following a link.
- **Cleanup / GC:** delete the overlay on exit; a periodic sweep removes
  `runs/<id>/` older than N days (the `runs/` root already exists). Symlinks
  make this safe — removing an overlay never touches `base/`.
- **Windows:** symlinks need privilege there. `materialize` falls back to file
  copy / directory junctions; the manifest makes link-vs-copy invisible upstream.

## 10. Agents = frozen sub-profiles

An **agent** is a profile whose composition is **pinned** (fixed skills,
instructions, model, optionally a restricted policy), so `am agent reviewer`
launches a fully-specified run with no open composition flags. The model already
supports it — an agent is a profile preset with the overlay locked. Project-scoped
profiles/agents (`<project>/.agent-manager/profiles/…`) are a later layer;
global profiles come first.

## 11. Phased plan — **status: all landed**

1. **Bug fix (Claude) ✅** seed login into `CLAUDE_CONFIG_DIR`, stop clobbering
   `HOME`. Validated end-to-end (AUTH_OK).
2. **Generalize seeding ✅** `ConfigAnchor`/`SeedFile`/`seed_login` on the
   `Harness` trait; Codex (Class A, `CODEX_HOME`), opencode (Class A after the
   `XDG_DATA_HOME` probe verified — B-1 below), grok (Class C, HOME-relocation +
   seed). Each declares `config_anchor()`.
3. **Profile store + resolve ✅** `src/profile.rs` (`Profile`/`ProfileStore`/
   `FsProfileStore`, `extends` inheritance via `resolve_chain`/`flatten`);
   `--profile` flag + `[defaults].profile` with precedence `flags > profile >
   per-harness > defaults` (§7); `am profile ls|show|use|create`; zero-config
   login reuse (`seed_zero_config_login`) instead of a materialized `default`
   profile file.
4. **Materialize + GC ✅** `src/overlay.rs`: `materialize` (symlink-else-copy of
   the profile `base/<harness>/` overlay, leaf-wins, never clobbers `am`-managed
   files, Windows copy fallback) + `sweep_old_runs` (day-later GC of the runs
   root, `AM_RUNS_TTL_DAYS`, default 7). No manifest needed — only files are
   linked, so `remove_dir_all` unlinks without following into `base/`.
5. **Agents ✅** `am agent <name>` = a profile with a `harness` pin run frozen
   (no composition flags). Project-scoped profiles/agents remain a later layer.

## 12. Decisions (resolved)

- **B-1 ✅ resolved = Class A-clean.** opencode's `XDG_DATA_HOME` **does** relocate
  its data/credential tier — verified empirically against opencode 1.17.18
  (`auth list` read `$XDG_DATA_HOME/opencode/auth.json` and overrode the
  HOME-relative default). So opencode seeds like Claude/Codex and drops HOME
  relocation. Recorded in `_docs/harness/opencode.md`.
- **B-2 → copy (for now).** Credentials are *copied* into the run dir; a
  refreshed OAuth token stays in the ephemeral dir and is discarded at cleanup.
  Copy-back-on-exit persistence is a future refinement. The config *overlay*
  (non-credential) is symlinked.
- **B-3 → independent.** Accounts stay their own store; a profile *references* an
  account by id, and `--account` remains a per-run override that wins over the
  profile's account (implemented in `resolve.rs`).

## 13. Where it lives (implementation map)

- `src/harness/mod.rs` — `Relocate`/`SeedFile`/`ConfigAnchor`, `Harness::config_anchor`, generic `seed_login`.
- `src/harness/{claude,codex,opencode,grok}.rs` — per-harness `config_anchor()` + seed-not-relocate provision.
- `src/profile.rs` — profile store + `extends` inheritance.
- `src/resolve.rs` — profile selection + 4-layer `pick` + `config_bases`.
- `src/overlay.rs` — `materialize` + `sweep_old_runs`.
- `src/provision.rs` — overlay materialize + GC hook + `seed_zero_config_login`.
- `src/cli/{profile,agent}.rs` + `src/cli/mod.rs` — `am profile` / `am agent`.
- `src/settings.rs` — `[defaults].profile`.
