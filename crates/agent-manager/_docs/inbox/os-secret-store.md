# Proposal: harness-scoped credentials + pluggable storage

> **Status: inbox proposal** (not accepted / not implemented).  
> **Date:** 2026-07-19  
> **Audience:** implementers / coding agents — design **and** handoff brief.  
> **Single source of truth:** this file only. Do **not** pre-document the
> design in `cli.md`, `am-as-library.md`, harness docs, `open-points.md`,
> etc. Those describe **shipped** behavior; update them **after**
> implementation (§17).

> **Context:** macOS Claude login lives in system Keychain; `am` seeds **files**
> into ephemeral config dirs. Stopgap: `am account import --write` extracts
> into `accounts/default/`. This proposal defines the full model: **harness-
> scoped** credentials (same name, e.g. `default`, per harness), **pluggable
> storage engines** selected in the main toml, a **lib-mode `SecretStore`
> trait** (CLI is one consumer), and expanded **`am account`** subcommands
> (`dump`, `delete`, `renew`, `rename`).

### Document map

| Section | Content |
|---------|---------|
| §§1–15 | Design (problem, model, CLI, engines, security) |
| **§16 Implementation brief** | **Start here to code** — files, phases, APIs, tests, DoD |
| §17 | Which **shipped** docs to update **after** code lands (not before) |

### Background docs (current behavior only — not the proposal)

Read these for how `am` works **today**. The proposal itself is only this file.

| Doc | Why |
|-----|-----|
| [am-as-library.md](../am-as-library.md) | Store traits, `Source`, lib vs `cli` features, no clap in core |
| [architecture.md](../architecture.md) | Pipeline `resolve → RunSpec → provision → run`; invariants |
| [profiles.md](../profiles.md) | Accounts vs profiles; Class A/B/C anchors; no HOME override |
| [cli.md](../cli.md) | `am account` surface; settings discovery; env vars |
| [overview.md](../overview.md) | Product goals; account references-not-secrets |
| [open-points.md](../open-points.md) §2 | Login-capture follow-ups (metadata, copy-back) |
| [harness/claude-code.md](../harness/claude-code.md) | Keychain service name; seed paths; import stopgap |
| [harness/codex.md](../harness/codex.md), [opencode.md](../harness/opencode.md), [copilot.md](../harness/copilot.md), [grok.md](../harness/grok.md) | Per-harness `login_seed` / capture |
| [harness/structure.md](../harness/structure.md) | Required harness doc sections |
| [AGENTS.md](../../AGENTS.md) | Crate layout, build/test commands |
| [test-plan.md](../test-plan.md) § account tests | Manual cases to extend |

### Related code (today)

| Path | Role |
|------|------|
| `src/account.rs` | `Account`, `AccountStore`, `FsAccountStore`, Keychain extract helpers (`read_claude_keychain_credentials`, `import_default_claude_from_keychain`, `DEFAULT_ACCOUNT_ID`) |
| `src/cli/account.rs` | `am account` clap: `ls`/`use`/`import`/`login`; Keychain import CLI |
| `src/harness/mod.rs` | `Harness`, `ConfigAnchor`, `SeedFile`, `seed_login`, `LoginPlan` |
| `src/harness/claude.rs` | `login_seed`, `login()`, `provision` account seed, Keychain docs |
| `src/harness/{codex,opencode,copilot,grok}.rs` | Same patterns per harness |
| `src/resolve.rs` | `account_id` → `Account` → `spec.account_login` via `login_source` |
| `src/spec.rs` | `RunSpec.account`, `RunSpec.account_login: Option<Source>` |
| `src/source.rs` | `Source::Dir` / `Source::Files` — provision content seam |
| `src/provision.rs` | `seed_zero_config_login`; calls `harness.provision` + overlay + `post_seed` |
| `src/settings.rs` | `Settings`, `HarnessDefaults`, `default_config_dir`, `global_config_write_path` |
| `src/lib.rs` | Module exports; `#![forbid(unsafe_code)]`; core must build `--no-default-features` |
| `Cargo.toml` | Features: `cli`, `pty`, `frontend`, `inproc-mcp` — **no new feature required** for `SecretStore` (core) |

---

## 1. Problem

| Layer | Today |
|-------|--------|
| Claude Code (macOS) | OAuth in system Keychain; on-disk stub often empty |
| `am` account index | Reference-only; **one** `home` per account id, shared across harnesses |
| Capture layout | Per-harness *files under one home* (works by path coincidence) |
| Storage | Plain files only; no engine selection in config |
| CLI | `ls` / `use` / `import` / `login` — no dump / delete / renew / rename of stored secrets |

Pain:

1. Relocated harness config does not see system Keychain.
2. Account id is not explicitly **(harness, name)** — multi-harness “default”
   works only because file paths don’t collide under one home.
3. No first-class storage engine choice (private keychain vs plain files vs OS).
4. Lib embedders cannot plug a DB/vault without forking file layout assumptions.
5. No lifecycle ops: inspect (dump), delete, renew, rename credentials.

---

## 2. Goals

1. **Credential identity is `(harness_id, name)`** — e.g. `(claude-code, default)`
   and `(codex, default)` are **independent** entries that may share the same
   human name.
2. **Pluggable `SecretStore` trait** in **core** (no `cli` feature) so lib-mode
   embedders inject their own backend; CLI builds the engine from settings.
3. **Storage engine chosen in main toml** (`config.toml` / `am.toml`), e.g.
   private keychain vs plain files (OS store as optional wrap / tier).
4. **Private keychain under config dir** preferred when possible; plain files
   always available.
5. **CLI surface** for lifecycle: `dump`, `delete`, `renew`, `rename` (plus
   existing `ls` / `use` / `import` / `login`).
6. **`renew` via harness trait** — default: run harness with these credentials,
   run a harness-defined refresh command, capture updated blobs back into the
   store.
7. Account index stays free of secret **values**; bodies live only in
   `SecretStore`.
8. Seed into ephemeral dirs unchanged in spirit (`login_seed` / `Source`).

## 3. Non-goals

- Replacing harness-native stores for interactive use outside `am`.
- Cloud password managers in v1.
- Dumping secrets to logs / CI transcripts by default (dump is explicit + tty-aware).
- Changing the ephemeral run-dir model.

---

## 4. Credential identity: harness-scoped names

### 4.1 Key

```text
CredentialId {
  harness: HarnessId,   // "claude-code", "codex", "opencode", …
  name: String,         // "default", "work", "ci", …
}
```

Storage key (backend-agnostic string form):

```text
am.cred.<harness_id>.<name>.<blob>
# e.g.
am.cred.claude-code.default.credentials
am.cred.claude-code.default.claude-json
am.cred.codex.default.auth
```

Multiple harnesses **may** use the same `name` (`default`); they never share
storage slots.

### 4.2 Index vs body

| Piece | Holds |
|-------|--------|
| Account / credential **index** entry | `harness`, `name`, optional env refs, `captured` metadata (plan tier, email redacted), timestamps — **no tokens** |
| `SecretStore` | Blob bytes for that `(harness, name)` |

Today’s single `Account.home` layout becomes either:

- **legacy:** one home dir with harness-relative paths (status quo), or  
- **target:** index row + `SecretStore` blobs keyed by `(harness, name)`.

`resolve` for `am claude --account default` loads
`CredentialId { harness: claude-code, name: default }` (harness from the run,
name from `--account` / `[defaults].account` / profile).

**Default name:** ambient import still uses name **`default`** for the active
harness being imported (or import all known harnesses into their own
`default` slots).

### 4.3 Listing

```text
am account ls
# harness       name      engine    meta
# claude-code   default   keychain  subscription=pro
# codex         default   keychain  …
# claude-code   work      files     …
```

---

## 5. Storage engines + main toml config

### 5.1 Engines

| Engine id | Description |
|-----------|-------------|
| `keychain` | **Private keychain** under `am` config dir (`$AM_CONFIG/keychain/` or `~/.config/agent-manager/keychain/`). Preferred default when the dir is writable. Optionally DEK wrapped in OS Keychain / CredMan / Secret Service. |
| `files` | Plain files under accounts root (mode `0600`) — today’s layout / portable / CI. |
| `os` (optional tier) | System Keychain / Windows Credential Manager / Linux Secret Service — primarily for **vault master key**, or optional mirror; not the preferred bulk store for every blob (ACL / naming / UX). |

### 5.2 Settings (main toml)

```toml
# ~/.config/agent-manager/config.toml  (or project am.toml)

[credentials]
# Which SecretStore implementation the CLI constructs for account ops + seed.
# Lib embedders ignore this and pass their own &dyn SecretStore.
engine = "keychain"          # "keychain" | "files"  (default: "keychain" if possible, else "files")

# Optional overrides
# keychain_dir = "/custom/path"   # else $AM_KEYCHAIN or <config-dir>/keychain
# files_root   = "…"              # else $AM_ACCOUNTS / default accounts root

[defaults]
account = "default"          # credential *name* (harness comes from the run)

[harness.claude]
account = "work"             # name only; resolves as (claude-code, work)
```

Resolution for the CLI engine:

```text
1. [credentials].engine if set and constructible
2. else try private keychain (config dir writable) → engine keychain
3. else files
```

Env override (optional): `AM_CREDENTIALS_ENGINE=keychain|files`.

### 5.3 Cascade inside an engine family

For `engine = "keychain"`:

```text
get: private vault → legacy plain files (migration) → error
set: private vault (create if needed); if vault unlock fails → error or files if policy allows
```

For `engine = "files"`: only the accounts tree (current behavior).

---

## 6. `SecretStore` trait (core / lib)

**Lives in core** (`src/account` or `src/credentials`) — compiled with
`--no-default-features`. **Not** gated on `cli` / `pty`. See
[am-as-library.md](../am-as-library.md) § storage extension points.

```rust
/// Stable id for one harness-scoped credential set.
pub struct CredentialId {
    pub harness: String,
    pub name: String,
}

/// One named blob inside a credential set (maps to a seed path).
pub struct CredentialBlob {
    pub name: String,           // e.g. "credentials", "auth", "claude-json"
    pub rel_path: PathBuf,      // e.g. ".claude/.credentials.json" (login_seed.src shape)
    pub bytes: Vec<u8>,
}

pub trait SecretStore: Send + Sync {
    fn list(&self) -> Result<Vec<CredentialMeta>>;
    fn get(&self, id: &CredentialId) -> Result<Option<Vec<CredentialBlob>>>;
    fn set(&self, id: &CredentialId, blobs: &[CredentialBlob]) -> Result<()>;
    fn delete(&self, id: &CredentialId) -> Result<()>;
    fn rename(&self, from: &CredentialId, to_name: &str) -> Result<()>;
    // rename keeps harness fixed; only the name component changes.
}

pub struct CredentialMeta {
    pub id: CredentialId,
    pub engine: String,                 // "keychain" | "files" | "memory" | …
    pub captured: BTreeMap<String, String>, // non-secret
}
```

**Built-in impls (core):**

| Impl | Feature needs |
|------|----------------|
| `FileSecretStore` | none (always) |
| `PrivateKeychainStore` | none (or thin optional crypto dep) |
| `OsSecretStore` | none (subprocess) or optional |
| `MemorySecretStore` | none — tests + embedders |
| `CascadeSecretStore` | composes the above |

**CLI** (`feature = "cli"`): reads `[credentials].engine`, builds
`Box<dyn SecretStore>`, passes it into account commands and into resolve /
provision wiring.

**Lib embedder:**

```toml
agent-manager = { path = "...", default-features = false }
```

```rust
let secrets: Arc<dyn SecretStore> = Arc::new(MyDbSecretStore::connect(...)?);
// pass into resolve / AccountStore adapter / provision seed path
```

`AccountStore` may **delegate** login bodies to `SecretStore`:

```rust
fn login_source(&self, harness: &str, name: &str) -> Result<Option<Source>> {
    let id = CredentialId { harness: harness.into(), name: name.into() };
    Ok(self.secrets.get(&id)?.map(|blobs| {
        Source::Files(blobs.into_iter().map(|b| (b.rel_path, b.bytes)).collect())
    }))
}
```

Filesystem `AccountStore` remains valid for **index** only; blobs move to
`SecretStore`. Embedders can implement either a combined store or two traits.

---

## 7. Harness trait: renew (+ optional export shape)

Credential **renewal** is harness-specific (OAuth refresh, re-login, device
code, …). Core defines hooks on `Harness`; defaults are safe no-ops or a
documented generic path.

```rust
pub trait Harness {
    // … existing methods …

    /// Relative blob paths this harness expects for a captured login
    /// (same list as config_anchor().login_seed today).
    fn credential_blobs(&self) -> &[SeedFile]; // or reuse config_anchor().login_seed

    /// How to renew stored credentials for this harness.
    /// Default: spawn the harness with `creds` seeded into a temp config
    /// dir, run [`Self::credential_renew_command`], then re-read the seed
    /// paths and return updated blobs.
    fn renew_credentials(
        &self,
        creds: &[CredentialBlob],
    ) -> Result<Vec<CredentialBlob>> {
        default_renew_via_launch(self, creds)
    }

    /// Argv fragment for the default renew path (e.g. Claude:
    /// `["auth", "status"]` is NOT enough — prefer a real refresh if any;
    /// or re-run a short headless probe that forces token refresh).
    /// Default: empty → `renew_credentials` returns "not implemented".
    fn credential_renew_command(&self) -> Option<Vec<String>> {
        None
    }
}
```

**Default renew algorithm** (`default_renew_via_launch`):

1. Create a temp / ephemeral config dir.  
2. `seed_login` from `creds` using `config_anchor().login_seed`.  
3. Launch harness with normal relocation env (`CLAUDE_CONFIG_DIR`, …) and
   `credential_renew_command()` argv (or harness-specific interactive
   re-login if required).  
4. Wait for exit (or bounded success).  
5. Read updated files from the temp dir at `login_seed` destinations.  
6. Return new `CredentialBlob`s for `SecretStore::set`.

Harnesses that cannot refresh headless override `renew_credentials` to
open an interactive login (same as `login()`) or bail with a clear error.

---

## 8. CLI: `am account` subcommands

> CLI only (`feature = "cli"`). All ops take **name** and **`--harness`**
> (default: infer from context where safe; for global ops require explicit
> harness, or accept `--all-harnesses` for `ls`).

### 8.1 Surface

```text
am account ls [--harness <id>]
am account use <name>                    # sets [defaults].account = name (name only)
am account import [--write] [--harness <id>]
am account login <name> --harness <id>

am account dump <name> --harness <id> [--json] [--path <blob>]
am account delete <name> --harness <id> [--yes]
am account renew <name> --harness <id>
am account rename <old> <new> --harness <id>
```

| Subcommand | Behavior |
|------------|----------|
| **`dump`** | Print credential blobs for `(harness, name)` to stdout. Default: redacted summary (metadata + token lengths / expiry). `--json` full blobs **only** if stdout is a TTY or `AM_ALLOW_SECRET_DUMP=1` / `--show-secrets` (document footgun). Prefer printing paths + “present/absent” for scripts without secrets. |
| **`delete`** | `SecretStore::delete` + remove index row for that `(harness, name)`. Refuse to delete without `--yes` if it is the configured default name. |
| **`renew`** | Load blobs → `Harness::renew_credentials` → `SecretStore::set`. Print short status (renewed / unchanged / failed). |
| **`rename`** | `SecretStore::rename` within the **same harness** (`old` → `new` name). Update index; if `[defaults].account == old`, update to `new` (optional prompt). Does **not** move credentials across harnesses. |

Existing:

| Subcommand | Notes under this model |
|------------|-------------------------|
| **`ls`** | List `(harness, name)` pairs + engine + non-secret meta. |
| **`use <name>`** | Sets default **name** only (harness-agnostic default name for all harnesses, or document per-harness defaults via `[harness.<id>].account`). |
| **`import`** | Source = harness-native (Claude system Keychain, …) → `set(CredentialId{harness, name: "default"}, …)`. |
| **`login`** | Interactive capture → `set` for `(harness, name)`. |

### 8.2 Examples

```bash
am account import --write --harness claude
# → (claude-code, default) in configured engine

am account ls
am account dump default --harness claude          # redacted
am account dump default --harness claude --show-secrets

am account renew default --harness claude
am account rename default personal --harness claude
am account use personal

am account delete personal --harness codex --yes
```

### 8.3 Separation from lib

| Concern | Where |
|---------|--------|
| `SecretStore`, `CredentialId`, renew helpers, engines | **core** |
| clap parsing, TTY checks for dump, pretty tables | **`cli` feature** only |
| Embedder UI for dump/delete | caller’s code; same trait methods |

---

## 9. Private keychain (config folder)

Unchanged intent from earlier revision:

- Path: `~/.config/agent-manager/keychain/` or `$AM_KEYCHAIN` / `$AM_CONFIG/keychain/`.
- Relocates with config; isol8-friendly; no collision with Claude’s system service.
- DEK in OS store when possible; else sealed vault / 0600.
- Selected when `[credentials].engine = "keychain"`.

---

## 10. Provision / resolve wiring

```text
flags.account / profile / [harness].account / [defaults].account
       → name: "default"
run harness id
       → CredentialId { harness, name }
SecretStore::get
       → Source::Files(blobs)
RunSpec.account_login = that Source
provision seed_login → ephemeral dir
```

Zero-config (no account name): optional fallback still seeds from real HOME /
system Keychain **or** auto-uses name `default` if present for that harness.

---

## 11. Migration

1. **Now:** import → files under `accounts/default/` (shared home layout).  
2. **Phase A:** `SecretStore` + `FileSecretStore`; CLI engine `files`; identity becomes `(harness, name)` in index.  
3. **Phase B:** `PrivateKeychainStore` + `[credentials].engine`; import writes there.  
4. **Phase C:** CLI `dump` / `delete` / `renew` / `rename`.  
5. **Phase D:** `Harness::renew_credentials` per harness (Claude first).  
6. **Phase E:** DEK-in-OS; drop long-lived plaintext where possible.  

Migrate legacy `accounts/<name>/` multi-harness homes by splitting into
per-`(harness, name)` blobs on first read.

---

## 12. Security

| Topic | Rule |
|-------|------|
| `dump` | Redacted by default; full secrets require explicit flag + TTY or env allowlist |
| Index toml | Never tokens |
| Logs | Never blob bodies |
| `delete` | Confirm if name is defaults.account |
| Renew temp dir | GC after success/failure; same hygiene as run dirs |

---

## 13. Acceptance criteria

- [ ] Two harnesses can each store credentials named `default` without clobbering.
- [ ] `[credentials].engine = "files" | "keychain"` selects CLI backend; lib ignores and uses injected `SecretStore`.
- [ ] `SecretStore` usable with `default-features = false` ([am-as-library.md](../am-as-library.md)).
- [ ] `am account dump|delete|renew|rename` implemented on CLI feature only.
- [ ] `renew` calls harness trait; default path seeds temp dir → run command → re-read blobs → `set`.
- [ ] Bare `am claude` with `[defaults].account = "default"` seeds from `(claude-code, default)`.
- [ ] Unit tests: MemorySecretStore, rename isolation across harnesses, engine selection parsing.

---

## 14. Decision needed

1. Is `[defaults].account` a **name only** (same default name for every harness) or should defaults be per-harness only via `[harness.<id>].account`?  
   **Proposal:** name-only global default + per-harness override (status quo shape).  
2. Vault format for private keychain (single sealed map vs SQLite).  
3. Full `dump --show-secrets` allowed in non-TTY with env allow, or never?  
4. Cross-harness rename (copy name from claude default → codex default)? **Out of scope** — use import/login per harness.

---

## 15. Related (quick list)

- [am-as-library.md](../am-as-library.md) — store traits, lib vs CLI features  
- [cli.md](../cli.md) — command surface  
- [profiles.md](../profiles.md) — accounts / defaults  
- [architecture.md](../architecture.md) — pipeline invariants  
- [open-points.md](../open-points.md) §2 — login-capture follow-ups  
- `_docs/harness/claude-code.md` — Keychain import stopgap  
- `src/account.rs` — Keychain extract helpers already landed  

---

## 16. Implementation brief (for coding agents)

> **Implement in order.** Do not skip “core builds with `--no-default-features`”.  
> Do not put clap or terminal types in `src/credentials/` or `src/account` secret APIs.  
> Do not log secret bodies. Do not write into Claude’s system Keychain service
> `Claude Code-credentials` (read/import only).

### 16.1 Hard constraints (from AGENTS / architecture)

| Rule | Source |
|------|--------|
| `#![forbid(unsafe_code)]` | `src/lib.rs` |
| Core builds: `cargo build -p agent-manager --no-default-features` | `AGENTS.md`, `am-as-library.md` |
| Full checks: `cargo test -p agent-manager < /dev/null`, `cargo clippy -p agent-manager --all-features -- -D warnings` | `AGENTS.md` |
| User real harness config read-only during run | architecture invariants |
| Accounts index never stores token **values** | `account.rs` module docs |
| Seed credentials with **copy** not symlink | `harness::seed_login` |
| `RunSpec` self-contained after resolve (no store calls in provision) | `am-as-library.md` §2 |
| Relocate via harness levers (`CLAUDE_CONFIG_DIR`, …), not HOME (Class A) | `profiles.md` §5 |

### 16.2 Suggested module layout

```text
src/
├── credentials/              # NEW — core (always compiled)
│   ├── mod.rs                # SecretStore trait, CredentialId, CredentialBlob, CredentialMeta
│   ├── file.rs               # FileSecretStore (accounts tree / per-harness dirs)
│   ├── keychain.rs           # PrivateKeychainStore under config dir
│   ├── memory.rs             # MemorySecretStore (tests + embedders)
│   ├── cascade.rs            # optional: private → files fallback on get
│   └── settings.rs           # parse engine id; build_store(settings) — keep free of clap
├── account.rs                # KEEP index; delegate login_source to SecretStore when wired
├── harness/mod.rs            # add renew_credentials defaults
├── harness/claude.rs         # renew command / override if needed
├── resolve.rs                # resolve (harness, account name) → SecretStore::get → account_login
├── settings.rs               # Settings.credentials: CredentialsSettings
├── cli/account.rs            # dump/delete/renew/rename + wire engine
└── lib.rs                    # pub mod credentials;
```

Alternative: keep types under `src/account/secrets.rs` if you want fewer top-level modules — either is fine; **trait must be public and core**.

### 16.3 Types to add (canonical)

```rust
// credentials/mod.rs (or account/secrets.rs)

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialId {
    pub harness: String,  // Harness::id() e.g. "claude-code" — NOT alias "claude"
    pub name: String,     // "default", "work", …
}

#[derive(Debug, Clone)]
pub struct CredentialBlob {
    /// Stable blob id within the set (e.g. "credentials", "claude-json", "auth").
    pub name: String,
    /// Path relative to account home / seed root — MUST match SeedFile.src
    /// (e.g. ".claude/.credentials.json", ".claude.json", "auth.json").
    pub rel_path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CredentialMeta {
    pub id: CredentialId,
    pub engine: String,
    pub captured: BTreeMap<String, String>,
}

pub trait SecretStore: Send + Sync {
    fn list(&self) -> Result<Vec<CredentialMeta>>;
    fn get(&self, id: &CredentialId) -> Result<Option<Vec<CredentialBlob>>>;
    fn set(&self, id: &CredentialId, blobs: &[CredentialBlob]) -> Result<()>;
    fn delete(&self, id: &CredentialId) -> Result<()>;
    /// Rename within the same harness only (`id.harness` fixed).
    fn rename(&self, from: &CredentialId, to_name: &str) -> Result<()>;
}

/// Map ConfigAnchor::login_seed + a home Source into blobs (and reverse).
pub fn blobs_from_seed(source: &Source, seed: &[SeedFile]) -> Result<Vec<CredentialBlob>>;
pub fn source_from_blobs(blobs: &[CredentialBlob]) -> Source; // Source::Files
```

**Harness id rule:** always store `Harness::id()` (`claude-code`), resolve CLI
aliases (`claude`) via `harness::resolve` before building `CredentialId`.

### 16.4 Settings schema

Extend `src/settings.rs`:

```rust
// On Settings:
pub credentials: CredentialsSettings,

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CredentialsSettings {
    /// "keychain" | "files". None = auto (keychain if writable, else files).
    pub engine: Option<String>,
    pub keychain_dir: Option<String>,
    pub files_root: Option<String>,
}
```

Example toml:

```toml
[credentials]
engine = "keychain"   # or "files"

[defaults]
account = "default"
```

Env: `AM_CREDENTIALS_ENGINE`, `AM_KEYCHAIN` (dir), existing `AM_ACCOUNTS`.

Builder used by CLI only:

```rust
// credentials/settings.rs or cli helper
pub fn build_secret_store(settings: &Settings) -> Result<Box<dyn SecretStore>>;
```

### 16.5 Wire into resolve / provision

**Today** (`src/resolve.rs` ~321–382):

```text
account_id → accounts.account(id) → accounts.login_source(id) → spec.account_login
```

**Target:**

```text
name = account_id (still a name string from flags/settings)
harness = RunSpec.harness / flags.harness  (must be known at resolve time — already is)
id = CredentialId { harness, name }
secrets.get(&id) → Option<blobs> → Source::Files → spec.account_login
// keep Account for env/helper/api_key_env injection if present
```

**Compatibility:** if `SecretStore::get` returns `None`, fall back to
`AccountStore::login_source(name)` / `Account.home` (legacy file homes from
`am account login` / current Keychain import under `accounts/default/`).

**Provision:** unchanged consumption — harnesses already seed from
`spec.account_login` + `account.home` (see `claude.rs` provision account block).
Prefer populating `account_login` so Class A harnesses never need HOME override.

**Zero-config** (`provision::seed_zero_config_login`): after SecretStore lands,
prefer `get(CredentialId { harness, name: "default" })` before copying real `$HOME`.

### 16.6 Harness trait additions

File: `src/harness/mod.rs` on `trait Harness`:

```rust
fn renew_credentials(&self, creds: &[CredentialBlob]) -> Result<Vec<CredentialBlob>>;
fn credential_renew_command(&self) -> Option<Vec<String>>; // default None
```

Default impl of `renew_credentials`:

1. `tempfile` or `new_run_dir`-like temp under `AM_RUNS`  
2. `seed_login(dir, &source_from_blobs(creds), &self.config_anchor().login_seed)`  
3. Build `Launch` env like `provision` would for relocation only (minimal —
   may call a new `Harness::launch_for_credential_ops(dir)` helper to avoid
   full provision of skills/MCP)  
4. Run `command()` + `credential_renew_command()?` with `std::process::Command`
   (not necessarily PTY)  
5. Re-read seed dest paths from `dir`  
6. Return blobs  

**Claude (phase D):** start with re-import from system Keychain
(`read_claude_keychain_credentials`) as `renew` if headless refresh is
impossible; document in harness doc. Better later: force token refresh path.

**Per-harness `login_seed` reference (dst = what ephemeral needs):**

| Harness | id | SeedFile src → typical store blob |
|---------|-----|-----------------------------------|
| Claude | `claude-code` | `.claude/.credentials.json`, `.claude.json` — see `Claude::config_anchor` |
| Codex | `codex` | `auth.json` — `Codex::config_anchor` |
| opencode | `opencode` | see `Opencode::config_anchor` login_seed |
| copilot | `copilot` | `config.json` |
| grok | `grok` | `.grok/auth.json` (or current anchor) |

Read each `config_anchor()` in code — do not hardcode from this table alone.

### 16.7 CLI changes (`src/cli/account.rs`)

Extend `AccountCommand` enum:

```text
Dump { name, harness, json, show_secrets, path }
Delete { name, harness, yes }
Renew { name, harness }
Rename { old, new, harness }
```

- Resolve harness key with `harness::resolve` → use `.id()`.  
- Construct `SecretStore` via `build_secret_store(&settings)`.  
- `dump`: redacted unless `--show-secrets` and (TTY or `AM_ALLOW_SECRET_DUMP=1`).  
- `renew`: `store.get` → `harness.renew_credentials` → `store.set`.  
- `import --write`: write via `SecretStore::set` for
  `CredentialId { harness: claude-code, name: default }` (keep Keychain read
  helpers in `account.rs` or move to `credentials/import_claude.rs`).  
- `login`: after capture, `set` blobs from home files (in addition to or instead
  of relying only on `home` path).  
- `ls`: list from `SecretStore::list` (+ legacy homes).  

Load settings for engine: reuse discovery from `settings::load` / same path as
`cli/run.rs` (grep how run loads settings).

### 16.8 File layout on disk

**Engine `files`:**

```text
$AM_ACCOUNTS/   # or default_config_dir()/accounts
  <name>/
    <harness_id>/          # NEW split (legacy: files mixed under <name>/)
      .claude/.credentials.json
      .claude.json
    # OR flat legacy: detect config_anchor paths under <name>/
  default.toml             # index only if still used
```

**Engine `keychain` (private vault):**

```text
$AM_CONFIG/keychain/     # default_config_dir()/keychain or AM_KEYCHAIN
  store.json             # v1 acceptable: map key→base64, mode 0600
  # later: encrypted store + OS-wrapped DEK
```

Logical key: `am.cred.<harness>.<name>.<blob_name>`.

**v1 vault format (recommended for first ship):** single JSON object
`{ "version": 1, "entries": { "<logical_key>": "<base64>" } }`, file mode
`0600`. Encryption can be Phase E without changing the trait.

### 16.9 Phased PR plan (ship incrementally)

| Phase | Deliverable | Definition of done |
|-------|-------------|--------------------|
| **A** | `credentials` module + `SecretStore` + `MemorySecretStore` + `FileSecretStore` | unit tests; `--no-default-features` build; no CLI yet |
| **B** | `Settings.credentials` + `build_secret_store` | parse engine from toml; unit test |
| **C** | resolve/provision: get `(harness,name)` → `account_login` | `am claude --account default` seeds from FileSecretStore; legacy home fallback works |
| **D** | Migrate Keychain **import** to `SecretStore::set` for `(claude-code, default)` | import --write populates store; bare am claude works when defaults.account=default |
| **E** | CLI `dump` / `delete` / `rename` | clap + tests (dump redaction) |
| **F** | `Harness::renew_credentials` default + CLI `renew` | Claude path at least re-import or documented no-op |
| **G** | `PrivateKeychainStore` + engine `keychain` | default engine auto; isol8/config relocate note in docs |
| **H** | Docs: cli.md, am-as-library.md (un-proposal), harness capture sections, open-points | checklist §13 green |

### 16.10 Tests to write

| Test | Where |
|------|--------|
| Memory set/get/delete/rename; rename does not cross harness | `credentials/memory.rs` |
| FileSecretStore round-trip for two harnesses both named `default` | `credentials/file.rs` |
| `source_from_blobs` / `blobs_from_seed` match Claude seed paths | unit |
| Settings parse `[credentials] engine` | `settings` tests |
| resolve prefers SecretStore over empty home | resolve test with MemorySecretStore |
| dump redaction never prints accessToken without flag | cli test or pure redactor unit |
| `cargo build -p agent-manager --no-default-features` | CI / local |
| Existing account/login tests still pass | `cli/account.rs`, harness login tests |

### 16.11 Manual verification (after D+)

```bash
# real GUI terminal (Keychain access)
cargo run -p agent-manager -- account import --write --harness claude
cargo run -p agent-manager -- account ls
cargo run -p agent-manager -- claude -- auth status
# expect loggedIn: true

cargo run -p agent-manager -- account dump default --harness claude
cargo run -p agent-manager -- account renew default --harness claude
cargo run -p agent-manager -- account rename default personal --harness claude
cargo run -p agent-manager -- account use personal
```

### 16.12 Explicit non-goals for implementer (do not expand scope)

- Cloud vaults, 1Password, Bitwarden  
- Overwriting Claude system Keychain  
- Cross-harness rename  
- Full interactive TUI for credentials  
- Changing PTY runner or io bridges  
- MCP-as-skill / isolation work  

### 16.13 Current stopgap to preserve until D

Keep working:

- `account::read_claude_keychain_credentials`  
- `account::import_default_claude_from_keychain`  
- `cli/account.rs` Keychain import + `set_defaults_account`  
- File layout `accounts/default/.claude/.credentials.json`  

Phase D should **call into** these readers and **write** through `SecretStore`,
then optionally keep writing files for dual-read during migration.

### 16.14 Definition of done (full feature)

- [ ] All of §13 acceptance criteria  
- [ ] Phases A–G complete (H docs)  
- [ ] `cargo test -p agent-manager < /dev/null` green  
- [ ] `cargo build -p agent-manager --no-default-features` green  
- [ ] `cargo clippy -p agent-manager --all-features -- -D warnings` green  
- [ ] Manual Keychain import + `am claude -- auth status` on macOS  
- [ ] `am-as-library.md` SecretStore section marked implemented (not “proposed”)  
- [ ] open-points §2 updated or closed for storage-related items  

---

## 17. Doc updates **after** implementation only

Do **not** edit these while the feature is still proposal-only. After code
ships, update them to match reality (and mark this inbox doc Implemented or
move a short summary into `target/` if desired):

| File | Post-implementation update |
|------|----------------------------|
| This inbox doc | Status → **Implemented** + date, or archive |
| [am-as-library.md](../am-as-library.md) | Document `SecretStore` as a real extension point |
| [cli.md](../cli.md) | Ship `dump`/`delete`/`renew`/`rename`; `[credentials]` engine |
| [open-points.md](../open-points.md) | Close/adjust login-capture items; note copy-back via renew if done |
| [profiles.md](../profiles.md) | Harness-scoped credential names if behavior changes |
| Each `_docs/harness/*.md` | Capture/renew notes only if harness behavior changed |
| [test-plan.md](../test-plan.md) | New account lifecycle cases |
| [AGENTS.md](../../AGENTS.md) | Module tree if `src/credentials/` added |
| [architecture.md](../architecture.md) | Only if pipeline/store story changes |

---

## 18. Agent prompt (copy-paste)

```text
Implement harness-scoped credentials + SecretStore per:
  crates/agent-manager/_docs/inbox/os-secret-store.md
especially §16 Implementation brief.

The proposal lives ONLY in that inbox file — do not document the design
in cli.md / am-as-library.md / harness docs until code has shipped; then
update those per §17.

Constraints:
- Core only for SecretStore (default-features = false must build)
- CredentialId = (harness_id, name); "default" allowed per harness
- [credentials].engine in settings: keychain | files
- CLI: am account dump|delete|renew|rename --harness <h>
- renew via Harness::renew_credentials (default seed+run+re-read)
- Do not put secrets in account index toml
- Do not write Claude system Keychain; import read-only is OK
- Preserve existing import/login until dual-write migration works
- Follow phases A→G; ship FileSecretStore before private keychain
- Run: cargo test -p agent-manager < /dev/null
      cargo build -p agent-manager --no-default-features
```
