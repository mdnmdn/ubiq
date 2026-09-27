# Profiles & agents

> A **profile** is a named, persistent base — an account (login), a set of
> defaults, and a default isolation policy — from which every run makes a
> **throwaway overlay**. An **agent** is a profile with its composition frozen.
> This doc defines the model, the cross-harness "cleanest solution" it rests on,
> and the phased path to get there. A login lives in a profile's own config home
> (§6.2, `D193`); nothing in `am` captures, seeds or roams one. §4 and §5 record
> the seeding design that preceded it.

## 1. Why: two config lifetimes

Everything a run needs from a harness's config splits into two halves with
**opposite lifetimes**:

| Concern | Examples | Lifetime | Scope |
|---|---|---|---|
| **Identity / base state** | credentials | **persist** across runs | per *account* |
| **Run composition** | skills, MCP servers, hooks, instructions, model | **ephemeral**, per run | per *run* |
| **Preference defaults** | theme, TUI mode, first-run opt-ins | **persist**, editable | per *harness*, global (§14) |

`hasCompletedOnboarding` and the per-project trust dialog look like identity
but aren't stored anywhere persistent at all — they're forced fresh on every
ephemeral run instead (§14.2), since they're wizard-completion flags, not
state a user would ever want to differ per account.

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
| **A — casual** | just run an agent | `am claude --mcps a,b --model haiku --prompt 'hi'` | the harness's own login, in its own config (`Native`, §6.2) | **real** | occasional `--isolate` |
| **B — expert** | curated, repeatable setups | `am claude --profile work` (+ per-run overrides) | named profiles, possibly **multiple accounts**, `--account` overrides | **always real** | opt-in per profile/run |
| **C — hardcore** | full sandboxes | `am claude --profile ci --isolate=locked` | multiple accounts, each in its own **replaced** HOME | **replaced** on request (`home = "ephemeral"` / `"managed"`), toolchain reconstructed | always |

**The central dividing line: A and B never touch `HOME`.** They rely entirely on
the harness's config-dir levers (§5), so the user's toolchain always
survives — switching accounts or profiles never costs `nvm`/`mise`/`pyenv`.
**C is the only tier that replaces `HOME`,** and does so deliberately, accepting
that the toolchain must be reconstructed inside the sandbox (§8). HOME
replacement is thus an *explicit opt-in*, never a silent side effect of picking
an account.

How each tier uses the machinery:

- **A (casual).** No profile authored. The run uses the harness's own config and
  login in place (§6.1); composition is 100% per-run flags. `--isolate` wraps the launch (§8) without changing auth or HOME.
- **B (expert).** Authors `profiles/<name>/profile.toml` fixing an account +
  defaults; may keep several accounts and switch via `--account`/`--profile`.
  Because each profile's login lives in its own config home, switching is free
  of any toolchain cost. Per-run flags still override the profile — the sweet spot the
  whole "cleanest solution" is built for.
- **C (hardcore).** Runs under isol8 with a replaced HOME per account/profile. This is also the only correct home
  for Class-C harnesses like grok, which have no config lever (§5).

## 3. The hard constraint: don't touch `HOME`

Relocating `HOME` has **strong, non-obvious consequences**. A child launched
under a synthetic `HOME` loses everything anchored there:

- version/tool managers — `nvm`, `mise`, `pyenv`, `rbenv`, `asdf`, `volta`
- shell rc and PATH shims (`~/.zshrc`, `~/.local/bin`, `~/.cargo/bin`, …)
- SSH keys/config, git config, cloud credentials, language caches

All of it is reconstructable, but only *deliberately*. So the governing rule:

> **Never relocate `HOME` merely to inject config. Relocate the harness's own
> config/data dirs via its native env levers — a profile's own config home — and
> leave the real `HOME` (and the toolchain) intact.**

`HOME` relocation is reserved for the **explicit `home` mode** a confined run may
ask for (isol8, §8), where reconstructing the environment is the whole point. A
confined run that asks for nothing keeps the real `HOME`, because the toolchain
grants its layers carry are `~`-relative and a replaced home aims them at an
empty directory — [`cli.md`](./cli.md) §"Settings file + flag merge" owns that rule.

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

Seeding was replaced by a login kept in each profile's own home (`D193`, §6.2).

## 5. The cleanest solution, generalized across harnesses

Harnesses differ in whether their config lever and their credential store are
the *same* dir. Three structural classes:

| Class | Harnesses | Config lever (relocatable, HOME-independent) | Credential store | Clean strategy |
|---|---|---|---|---|
| **A — unified root** | Claude Code (`CLAUDE_CONFIG_DIR`), Codex (`CODEX_HOME`), Copilot CLI (`COPILOT_HOME`) | one env var relocates config **and** creds | inside that root | point lever at ephemeral dir → **seed creds in** → keep real `HOME` |
| **B — split store** | opencode (`OPENCODE_CONFIG_DIR` for config; auth under `~/.local/share/opencode`, HOME-relative) | config only | HOME-relative *data* dir | relocate config via lever **and** relocate the data dir via its own lever (`XDG_DATA_HOME`?) → seed creds there → keep real `HOME`. If no data-dir lever exists, fall back to Class C for creds only. |
| **C — HOME-only** | grok (`~/.grok`, no `GROK_CONFIG_DIR`) | none | HOME-relative | **must** relocate `HOME` → toolchain caveat applies → prefer pairing with isol8 (§8) |

Copilot CLI is a cautionary tale for this classification: an initial pass
assumed Class C (no config lever documented), until `COPILOT_HOME` was
actually tested against the installed binary and found to relocate the whole
tree — Class A, no `HOME` relocation needed at all. **Verify against the real
binary before classifying a harness**, not just its doc — see
`_docs/harness/copilot.md`'s and `src/harness/copilot.rs`'s correction notes.

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
    /// True only for Class C: no lever, HOME must be relocated (toolchain caveat).
    requires_home_relocation: bool,
}
```

The login itself is no part of it: it lives in a profile's own home (`D193`, §6.2).

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
│   ├── mode          = "<permission mode>"   ├── skills/     ← composition
│   └── isolate       = false | "<policy>"
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

`am claude` with no `--profile` runs from the harness's own default config and
login in place (§6.2, `ConfigStrategy::Native`); named profiles and per-run flags
layer on top.

### 6.2 The per-profile home (`D193`)

A profile also owns one **persistent harness config home** per harness,
`profiles/<name>/home/<harness>/` beside `base/` — `ProfileStore::home(id,
harness)` names it (`FsProfileStore::home_dir` computes it; the default and a
store with no filesystem answer `None`; `ScopedProfileStore` answers from the
root that holds the profile). The login lands in it one of two ways: the
profile's first terminal run shows the harness's own login screen, or an
explicit sign-in runs `Harness::login_home` (Claude: `CLAUDE_CONFIG_DIR=<home>
claude auth login`; the CLI's `am profile login`). Either way the harness owns
the login and its refresh from then on: nothing is captured, seeded or read
back.

`provision::prepare_home(harness, home, templates)` makes a home ready for
either: it creates it and applies the §14.1 templates **once in its life**,
tracked by a `.am-home` marker written after them under a lock on
`.am-home.am-lock`. A home signed in before its first run still gets them, a
preference its user later changes stays changed, and two first runs racing on
one home apply them once. Templates only fill missing keys, so a login already
there is kept.

A run from it carries `ConfigStrategy::Home { home, scratch }`. `provision()`
calls `prepare_home`, then hands a harness whose `Harness::shares_home()` is
true to `Harness::provision_home(spec, Some(home), scratch)`, which writes
every per-run file into `scratch` and passes it by flag, so concurrent runs of
one profile share the home read-write and never see each other's MCP servers,
settings or skills (Claude's composition: `_docs/harness/claude-code.md`).
Beyond `prepare_home`, the home takes only §14.2's fix-ups. No login is seeded,
`Provisioned::dir` is the scratch and
`Provisioned::home` the home. `ephemeral` only ever removes `dir`: `provision`
leaves it false, and a caller that made the scratch for one run (the CLI) sets
it — no cleanup reaches the home, which lives under the profiles root, where
`sweep_old_runs` never looks. A harness that cannot share a home
(no built-in one) is provisioned into
`scratch` as under `ConfigStrategy::Fixed`. The profile overlay (§9) is not
applied under a shared home.

A run with **no profile** carries `ConfigStrategy::Native { scratch }`: the
harness's own default config, in place. A sharing harness gets
`provision_home(spec, None, scratch)` — for Claude, no `CLAUDE_CONFIG_DIR` in
the launch, and an inherited one is left alone, since a user who exported it
made it their default — and nothing is written outside `scratch`: no
templates, no onboarding or trust entry, no login. Print mode skips Claude's
trust dialog; a terminal run shows Claude's own. API-key, auth-token and helper
accounts apply, by env and `--settings`. A harness that cannot share a
home runs a `Native` spec in `scratch` as under `ConfigStrategy::Fixed`.

A **confined** `Home` or `Native` run is granted the home it runs from
read-write — `IsolateOptions::grant_config_home` adds the profile's home, or
under `Native` the harness's `Harness::default_homes()` (Claude: `~/.claude`,
or `$CLAUDE_CONFIG_DIR`, plus `~/.claude.json` when that is unset; Codex
`$CODEX_HOME` or `~/.codex`; Copilot `$COPILOT_HOME` or `~/.copilot`; opencode
its XDG data and config dirs; grok `~/.grok`). On macOS it also keeps the whole
`integrations/keychain` layer: the item Claude keys to the home is the login
itself.

`Harness::session_transcripts(config_home, cwd, session_id)` names ONE
session's files in a shared or native home, where `transcripts` would answer
every run's (Claude: `projects/<slug(cwd)>/<id>.jsonl` plus the `<id>/` dir
beside it; `None` for the home is `$CLAUDE_CONFIG_DIR`, else `~/.claude`).
Default `None`: the harness cannot name one session.

## 7. Resolution & the default question

Precedence (highest wins, replace-by-default, matching existing merge rules):

```
CLI flag  >  --profile <name>  >  [defaults].profile  >  implicit "default"
```

- **"Do we need a `default` property?"** — express it as a **default profile**
  (`am profile use <name>` → `[defaults].profile`), not a bare default account.
  One knob for "what runs by default"; the account default is just a field of
  that profile.
- Per-run flags (`--mcps`, `--model`, `--account`, `--permission-mode`,
  `--isolate`) still override the profile's fields, so a profile is a *default*,
  never a straitjacket.
- **`mode`** is the harness-native permission mode (e.g. `mode = "plan"`), a
  policy axis like `isolate` rather than a composition default — it lands on
  `spec.policy.permission_mode`, under `--permission-mode` and above nothing
  else.

## 8. Isolation is an orthogonal axis

Profile = *what config*; isolation = *what sandbox*. `isolate.rs` resolves a
`Launch` into an isol8 policy (a `Spec` + `Context`) in-process rather than
wrapping argv itself; a caller that owns a pseudo-terminal turns that policy
back into a `Launch` via `confined_launch`, a pane stopgap that execs
`sandbox-exec` on macOS, on Windows re-invokes the running binary under
`isolate::CONFINE_ARG` instead (isol8's Windows backend spawns with no
console-creation flag, so the re-invoked child attaches to the pseudo-terminal
it itself runs in — `confine_entrypoint()` is the embedder's hook, and no
second binary ships), and errors honestly on Linux
(`refs/isol8-pty-seam-update.md` tracks the seam that would replace both
stopgaps).

**`plan` names the layer stack itself, because isol8's own selection cannot reach
the ones a build needs.** isol8 auto-selects a layer from `cmd[0]` alone, and only
its `agents/*` layers declare an `executables` filter — so no `toolchains/*` layer is
ever selected, since a build tool is a *child* of the confined command and never the
command. `DEV_LAYERS` (`isolate.rs`) is therefore an explicit list of 16:
`integrations/{keychain,macos-gui,git}` and `toolchains/{runtime-managers,
apple-toolchain-core,rust,node,python,go,java,dotnet,bun,deno,ruby,php,perl,elixir}`. Two
of them are not about toolchains at all: `integrations/keychain`, because rustls
tools (mise, cargo) validate TLS through the trust daemon rather than a CA file, and
`integrations/macos-gui`, because a TUI harness calling `CGSEventSourceForID`
deadlocks on a WindowServer mutex when the lookup is denied.

**One of those sixteen names is not isol8's.** isol8 (v0.4.0) ships no .NET layer,
so `plan` *writes* `toolchains/dotnet` — `DOTNET_LAYER_BODY` in `isolate.rs` — into
`<state_dir>/profiles-dotnet/toolchains/dotnet.toml` and hands that directory to
`profile_paths` ahead of the caller's own profile root, so a `toolchains/dotnet`
written under that root still wins. It grants the CLI home (`~/.dotnet`, whose
first-use sentinel every `dotnet` command writes before doing anything else),
NuGet's config, package and cache roots, the template engine, `dev-certs` and
`user-secrets` stores, and `/etc/dotnet`'s install-location marker — plus a
`filter = { os = ["windows"] }` policy over the `%USERPROFILE%`, `%APPDATA%` and
`%LOCALAPPDATA%` spellings of the same, a per-user SDK install and MSBuild's node
state. The SDK tree itself needs nothing: `base` grants `/usr`,
`macos/system-runtime` grants `/opt`, `windows/system-runtime` grants
`%PROGRAMFILES%`.

This is not a duplicate of `DEV_RW_HOME_ROOTS` below but its other half. Those
grants are joined absolutely against the **real** home, and nothing grants a
replacement home wholesale, so under an `Ephemeral` or `Managed` home a run was
denied `$HOME/.dotnet` while `~/.dotnet` in the real home stayed writable — the
SDK then reports that it cannot determine a home directory rather than a denial,
and the reaction it invites is pointing `DOTNET_CLI_HOME` at the repository. A
layer's `~` expands against the *effective* home, which is the case the const
cannot reach. Delete the generated layer the day isol8 ships one: the name in
`DEV_LAYERS` keeps working and starts resolving to the built-in.

**`BROKEN_LAYERS` records the 10 that must never be named.** Nine carry `[macos] raw`
SBPL naming a symbol isol8's macOS backend never defines: `home-literal` / `home-subpath`
(xcode, kubectl, chromium-headless, chromium-full), both families at once (ssh, 1password,
container-runtime-default-deny), or the bare variable `HOME_DIR` inside `string-append`
(docker, ssh-agent-default-deny). The rendered policy opens `(version 1)` and
`(deny default)` with no `(define …)` prelude at all, so each of those is an unbound
variable that fails the **whole** policy — nothing starts, however unrelated the layer is
to the run. `integrations/shell-init` is the tenth: not broken, but a bad default.
`integrations/ssh` is excluded twice over:
beyond the macro bug it denies `~/.ssh` as a subpath, and Seatbelt is
last-match-wins with denies rendered after allows, so it swallows the
`~/.ssh/known_hosts` read `integrations/git` grants.

**Docker therefore cannot be enabled by naming `integrations/docker`.** A run that needs
it grants the daemon socket and `~/.docker` by hand, the same hand-replication a tuned
setup already does for xcode and chromium — and the shipped layer's own header calls
socket access "High-risk", since a container daemon socket is a route out of the sandbox.

**Four gaps are filled by hand.** `DEV_RW_HOME_ROOTS` grants `~/.dotnet`, `~/.nuget`,
`~/.templateengine`, `~/.aspnet`, `~/.microsoft` and `~/.local/share/NuGet`
read-write **in the real home**, the generated `toolchains/dotnet` layer above
covering the effective one; the list is deliberately
**not** existence-filtered, since `~/.dotnet` does not exist until the first
`dotnet` run creates it and the grant is what makes that creation legal. And
`ENV_PASS` widens isol8's deny-by-default env (`HOME PATH SHELL TMPDIR USER
LOGNAME PWD`) to the terminal (`TERM_PROGRAM`, `TERM_PROGRAM_VERSION`), the locale
(`LANG`, `LC_ALL`, `LC_CTYPE`), proxy in both cases, CA trust, and the toolchain
relocation roots (`CARGO_HOME`, `GOROOT`, `JAVA_HOME`, …). `GIT_*` and `XDG_*` are
left out on purpose: an inherited `GIT_DIR` would retarget the agent's own git at
the wrong repository, and a relocated `XDG_*` points every lookup away from the
`~/.config` paths the layers grant.

And `IsolateOptions::grant_toolchains_from_env()` covers the relocated install. The shipped
`toolchains/*` layers grant a toolchain's **default** location (`~/.cargo`, `~/.rustup`,
`~/go`), so on a machine whose install lives elsewhere the policy names the wrong directory
and `cargo`, `rustc` and `go` fail while `npm`, `node`, `dotnet`, `python` and `mise` work —
forwarding `CARGO_HOME` tells a tool where to look and grants nothing. The method walks
`TOOLCHAIN_ROOT_VARS`, 17 variable names each paired with whether a build writes there, and
appends every absolute value it reads to `extra_ro` — `GOROOT`, `JAVA_HOME`, `SDKMAN_DIR`,
SDK trees a build reads and never writes — or to `extra_rw`: the caches, registries and
install roots `CARGO_HOME`, `RUSTUP_HOME`, `GOPATH`, `GOMODCACHE`, `GOCACHE`, `PYENV_ROOT`,
`MISE_DATA_DIR`, `NPM_CONFIG_PREFIX`, `PNPM_HOME`, `VOLTA_HOME`, `BUN_INSTALL`,
`DOTNET_ROOT`, `NUGET_PACKAGES`, `PUB_CACHE`. Three rules carry the reasons. A relative or
empty value is skipped, because isol8 resolves a grant against no working directory, so
honouring one would grant a different tree rather than fail. The paths are not
existence-filtered, for `DEV_RW_HOME_ROOTS`'s reason: a root the tool creates on first use
needs the grant in order to be created, and a grant on an absent path is inert. And **the
environment read is the caller's, not `plan`'s** — `plan` must answer identically for a
given `IsolateOptions`, the front-end-agnostic invariant this module keeps — so a host calls
it where it decides to consult its own environment: `compose_run` in
`crates/ubiq-host/src/agent.rs` and `isolate_options` in `crates/agent-manager/src/cli/run.rs`,
both one line, both **before** the caller's own extra grants so an explicit grant is the last
word. `TOOLCHAIN_ROOT_VARS` is the relocation half of `ENV_PASS` and the two stay in step;
the test `every_toolchain_root_var_is_also_passed_through` is what makes that hold. The
private `grant_toolchains(lookup)` the method delegates to takes the lookup as an argument
because a test that mutated the process environment would race the rest of the binary.

And on macOS, `APPLE_SDK_RO_ROOTS` and `APPLE_RW_HOME_ROOTS` fill the fourth: isol8's
`integrations/xcode` layer is in `BROKEN_LAYERS` — its `[macos] raw` block calls
`home-literal`, which the macOS backend never defines, and that fails the whole policy the
same way the ten layers in the "gaps" list above do — while
`toolchains/apple-toolchain-core` grants `/Library/Developer/CommandLineTools/usr/bin` as a
*literal* and names clang, git, ld and forty more binaries one by one without naming
`swift`, `swiftc` or the twenty `swift-*` tools beside them. So a confined
`swift --version` was denied on a machine whose toolchain is complete. The
two consts replicate the reference profile's hand-written grants: `APPLE_SDK_RO_ROOTS`
read-only over `/Library/Developer/CommandLineTools`, `/Library/Developer/Toolchains`,
`/Applications/Xcode.app`, `/Applications/Xcode-beta.app` and the Xcode preferences plist;
`APPLE_RW_HOME_ROOTS` read-write over `~/Library/Developer/{Xcode,CoreSimulator,
XCTestDevices,CoreDevice}` and the Xcode/SwiftPM caches under `~/Library/Caches` and
`~/Library`. Both are macOS-only (Windows has no Xcode-shaped toolchain to grant) and, like `DEV_RW_HOME_ROOTS`, not existence-filtered —
a cache directory Xcode creates on first use needs the grant to be created. Two things paths
cannot express stay open: a simulator or device run also needs `com.apple.CoreSimulator*`
mach lookups and `user-preference-read/write` on `com.apple.dt.Xcode`, neither of which any
`isol8::Spec` field carries; and `swift build` fails regardless, because SwiftPM shells out
to `sandbox-exec` itself and Seatbelt cannot nest — only `--disable-sandbox` gets a confined
`swift build` running.

Either way the two axes compose cleanly:

- A profile carries a **default** for the isolation axis (`isolate = false`, or
  `isolate = "dev.toml"` naming an isol8 policy). Per-run `--isolate[=profile]`
  overrides.
- **Class C harnesses (grok) are where the axes meet.** Their non-isolated form
  must relocate `HOME` (lossy). Under isol8 the sandbox HOME is reconstructed
  deliberately, so the login lives in that sandbox HOME and the toolchain
  caveat becomes an explicit, understood cost rather than a silent breakage.
- Even for Class A, full isolation is available: run inside an isol8 HOME and let
  `CLAUDE_CONFIG_DIR` point inside it; only the `HOME` the child sees changes.

## 9. Materializing the overlay (symlink-else-copy, GC, Windows)

- **`materialize` abstraction:** identity files (creds, `.claude.json`) are
  linked back to `profiles/<name>/base/` when possible; composition files
  (mcp.json, skills) are freshly written and **owned** by the run.
- **No credentials:** a login is never materialized into a run; it lives in the
  profile's own home (`D193`, §6.2).
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
   `XDG_DATA_HOME` probe verified — B-1 below), Copilot CLI (Class A after the
   `COPILOT_HOME` lever was verified — an initial pass assumed Class C from
   the doc alone), grok (Class C, HOME-relocation + seed — the one harness so
   far with no config lever at all). Each declares `config_anchor()`.
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
- **B-2 → no copy (`D193`).** Credentials are not copied anywhere: the login
  stays in the profile's own home. The config *overlay* (non-credential) is
  symlinked.
- **B-3 → independent.** Accounts stay their own store; a profile *references* an
  account by id, and `--account` remains a per-run override that wins over the
  profile's account (implemented in `resolve.rs`).

## 13. Where it lives (implementation map)

- `src/harness/mod.rs` — `Relocate`/`ConfigAnchor`, `Harness::config_anchor`; `TemplateFile`, `Harness::templates`, generic `apply_templates` (§14); `Harness::post_seed` (§14).
- `src/harness/{claude,codex,opencode,grok,copilot}.rs` — per-harness `config_anchor()` + provision.
- `src/harness/claude.rs` — `templates()` (theme/tui/Claude-in-Chrome defaults) + `post_seed()` (onboarding/trust-dialog fix-ups); see §14.
- `src/profile.rs` — profile store + `extends` inheritance + `ProfileStore::home` (§6.2).
- `src/harness/mod.rs` — `Harness::shares_home` / `provision_home` / `login_home` / `session_transcripts` (§6.2).
- `src/resolve.rs` — profile selection (`effective_profile`) + 4-layer `pick` + `config_bases`.
- `src/overlay.rs` — `materialize` + `sweep_old_runs`.
- `src/provision.rs` — overlay materialize + GC hook + `apply_templates` + `post_seed` (§14); `prepare_home` and the `Home` / `Native` path (§6.2).
- `src/cli/{profile,agent}.rs` + `src/cli/mod.rs` — `am profile` / `am agent`.
- `src/settings.rs` — `[defaults].profile`.

## 14. Preference templates and structural post-seed fix-ups

Beyond identity (§1's "persist per account") and composition (§1's "ephemeral
per run"), a harness has a **third kind of state**: cosmetic/first-run
preferences (theme, TUI mode, "enable this integration?" opt-ins) that are
neither secret nor run-specific, but that the harness's own onboarding wizard
normally sets interactively — a wizard an ephemeral, always-first-run
`CLAUDE_CONFIG_DIR` never gets to go through. Left unset, every single `am`
run would hit that wizard. Two different fixes apply, split by whether the
value is a genuine user preference or a correctness requirement:

### 14.1 Templates — user-editable preferences

`Harness::templates() -> Vec<TemplateFile>` declares JSON files (relative to
the harness's relocated config root, e.g. `settings.json`, `.claude.json`)
that should be **merged in from an editable template store** rather than
hardcoded in Rust. Each `TemplateFile { name, default }` pairs a destination
filename with a `fn() -> serde_json::Value` used only to seed the template the
first time it's read.

`crate::harness::apply_templates(dir, harness_id, templates)` (called
generically from `provision::provision()`, after the overlay, for every
harness — most have an empty `templates()` and it's a no-op):

1. Resolves the template store root (`~/.config/agent-manager/templates` by
   default, overridable via `AM_TEMPLATES` — same pattern as
   `AM_ACCOUNTS`/`account::resolve_accounts_root`).
2. For each declared file, if `templates/<harness-id>/<name>` doesn't exist
   yet, creates it from `default()` — so there's always a real, editable file
   on disk, not a value hidden in Rust that a user would have to read source
   to discover.
3. Shallow-merges that template's keys into the run's `dir/<name>`,
   **gap-filling only**: any key the run itself already generated (policy,
   `apiKeyHelper`, hooks, …) wins; the template supplies
   only keys nothing else set.

Editing `~/.config/agent-manager/templates/claude-code/settings.json` (e.g.
changing `"theme": "dark"` to `"light"`) takes effect on the very next run —
no rebuild, no per-account duplication. Because the store is keyed by
`harness-id`, adding template defaults for another harness (codex, opencode,
grok) is purely `impl Harness::templates()` for that harness — no core
change, matching every other harness-extension seam in this codebase.

Claude's current templates: `settings.json` (`theme`, `tui`) and
`.claude.json` (`claudeInChromeDefaultEnabled`).

### 14.2 `post_seed` — structural fix-ups, never template-overridable

`Harness::post_seed(spec, dir)` (default no-op; also called generically from
`provision::provision()`, last in the pipeline) is for values that look
similar but are **not** preferences — they're correctness requirements of
`am`'s always-ephemeral-config model that must always be forced, never left
to a user-editable file a stray edit could break:

- `hasCompletedOnboarding: true` — a login made non-interactively (`claude auth
  login`, `am profile login`) never runs the wizard that normally sets this, so a
  fully-authenticated config would otherwise still show the onboarding UI.
- `projects[spec.cwd].hasTrustDialogAccepted: true` — Claude Code gates a
  per-project trust dialog on this, keyed by the exact cwd string; a fresh
  `CLAUDE_CONFIG_DIR` has no record of any cwd, so every run would otherwise
  hit that dialog too.

Under a shared home (§6.2) both are added **only when missing**, as a
read-modify-write of `<home>/.claude.json` under an exclusive lock on
`.claude.json.am-lock`, written beside the file and renamed onto it: the file
holds the login's identity and every other run's projects, so it is never
replaced by a fresh document, and a value already there is the harness's own.

Rule of thumb for future additions: if a value is something a user might
reasonably want to change, it's a **template** (§14.1). If unsetting it would
just reintroduce a wizard/dialog `am`'s ephemeral model makes unavoidable
otherwise, it belongs in **`post_seed`**.
